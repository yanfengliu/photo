//! Deterministic fixtures shared by the contract tests.

use photo_core::colorimetry::RgbSpace;
use photo_core::image::{SourceColor, SourceImage};
use photo_imaging::{Plane, RgbImage};

/// SplitMix64: a tiny, well-mixed generator so fixtures never depend on a
/// third-party crate or on the platform.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Roughly normal with zero mean and unit variance (sum of twelve uniforms).
    pub fn normal(&mut self) -> f32 {
        (0..12).map(|_| self.unit()).sum::<f32>() - 6.0
    }
}

/// White noise in `[0, 1)`: the worst case for anything that interpolates.
pub fn noise_plane(width: usize, height: usize, seed: u64) -> Plane {
    let mut rng = Rng::new(seed);
    let data = (0..width * height).map(|_| rng.unit()).collect();
    Plane::from_vec(width, height, data).unwrap()
}

fn source(width: u32, height: u32, pixels: Vec<u16>) -> SourceImage {
    SourceImage::new(width, height, pixels, SourceColor::Rgb(RgbSpace::Srgb)).unwrap()
}

/// A source whose every sample is independent noise.
pub fn noise_source(width: u32, height: u32, seed: u64) -> SourceImage {
    let mut rng = Rng::new(seed);
    let pixels = (0..width as usize * height as usize * 3)
        .map(|_| (rng.next_u64() >> 48) as u16)
        .collect();
    source(width, height, pixels)
}

/// A source made of a few low-frequency waves per channel: smooth enough that
/// interpolation error is small, varied enough that a misplaced pixel shows.
pub fn smooth_source(width: u32, height: u32, seed: u64) -> SourceImage {
    let mut rng = Rng::new(seed);
    let waves: Vec<[f32; 5]> = (0..9)
        .map(|_| {
            [
                rng.unit() * 0.05,
                rng.unit() * 0.05,
                rng.unit() * 6.3,
                rng.unit() * 6.3,
                0.5 + rng.unit(),
            ]
        })
        .collect();
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 3);
    for y in 0..height {
        for x in 0..width {
            for channel in 0..3 {
                let value: f32 = waves[channel * 3..channel * 3 + 3]
                    .iter()
                    .map(|w| {
                        w[4] * ((x as f32 * w[0] + w[2]).sin() + (y as f32 * w[1] + w[3]).cos())
                    })
                    .sum();
                let unit = (0.5 + value / 12.0).clamp(0.0, 1.0);
                pixels.push((unit * 65535.0).round() as u16);
            }
        }
    }
    source(width, height, pixels)
}

/// A source built from a function of the pixel position returning `0..1`.
pub fn source_from_fn(width: u32, height: u32, f: impl Fn(u32, u32) -> [f32; 3]) -> SourceImage {
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 3);
    for y in 0..height {
        for x in 0..width {
            for value in f(x, y) {
                pixels.push((value.clamp(0.0, 1.0) * 65535.0).round() as u16);
            }
        }
    }
    source(width, height, pixels)
}

/// The exact `f32` a source sample converts to.
pub fn unit(sample: u16) -> f32 {
    sample as f32 / 65535.0
}

pub fn max_abs_diff(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "buffers differ in length");
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max)
}

pub fn mean_abs_diff(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "buffers differ in length");
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f32>() / a.len().max(1) as f32
}

pub fn mean(values: &[f32]) -> f32 {
    values.iter().sum::<f32>() / values.len().max(1) as f32
}

pub fn std_dev(values: &[f32]) -> f32 {
    let m = mean(values);
    (values.iter().map(|v| (v - m) * (v - m)).sum::<f32>() / values.len().max(1) as f32).sqrt()
}

/// Flat `f32` view of an RGB image, for the diff helpers above.
pub fn flat(image: &RgbImage) -> &[f32] {
    image.as_slice()
}

/// A vertical step from 0.2 to 0.8 at `width / 2` with additive noise: the
/// standard subject for "smooth the flats, keep the edge".
pub fn noisy_step(width: usize, height: usize, noise: f32, seed: u64) -> Plane {
    let mut rng = Rng::new(seed);
    let data = (0..width * height)
        .map(|i| {
            let level = if i % width < width / 2 { 0.2 } else { 0.8 };
            level + noise * rng.normal()
        })
        .collect();
    Plane::from_vec(width, height, data).unwrap()
}

/// Standard deviation of a [`noisy_step`] plane at least `margin` columns
/// away from the step and from the borders, measured against the step's own
/// levels so the edge itself does not count as noise.
pub fn flat_noise(plane: &Plane, margin: usize) -> f32 {
    let half = plane.width() / 2;
    let mut residual = Vec::new();
    for y in margin..plane.height() - margin {
        for x in (margin..half - margin).chain(half + margin..plane.width() - margin) {
            let level = if x < half { 0.2 } else { 0.8 };
            residual.push(plane.get(x, y) - level);
        }
    }
    std_dev(&residual)
}

/// Mean jump between the two columns either side of a [`noisy_step`].
pub fn edge_height(plane: &Plane) -> f32 {
    let half = plane.width() / 2;
    let column = |x: usize| {
        mean(
            &(0..plane.height())
                .map(|y| plane.get(x, y))
                .collect::<Vec<_>>(),
        )
    };
    column(half) - column(half - 1)
}

/// A one-thread pool and a many-thread pool, for the R4 determinism checks.
pub fn pools() -> (rayon::ThreadPool, rayon::ThreadPool) {
    let single = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let many = rayon::ThreadPoolBuilder::new()
        .num_threads(7)
        .build()
        .unwrap();
    (single, many)
}
