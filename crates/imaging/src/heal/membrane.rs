//! The tone-matching field of a healing spot.
//!
//! Around the spot, just outside it, the image is sampled in small windows
//! on a ring: once around the target and once around the source. The ratio
//! of the two means is what the copied patch has to be multiplied by at
//! that point of the rim to match its new surroundings. Inside the spot the
//! factor is the harmonic function with those boundary values, given on a
//! disc by the Poisson integral: each interior point takes a weighted mean
//! of the boundary values with weights `1 / |p - b|^2` (the Poisson kernel
//! with its common factor `1 - |p|^2` cancelled by normalization).

use std::f64::consts::TAU;

use photo_core::image::SourceImage;

use super::{CHANNELS, Stamp};

/// Added to both means before taking their ratio, in `u16` codes: 1/256 of
/// white. Near black the correction then tends to an offset rather than an
/// ever larger gain on noise.
const FLOOR: f32 = 256.0;
/// The gain is limited to this factor either way.
const MAX_GAIN: f32 = 16.0;
/// Half-width of the sampling windows, as a fraction of the spot radius.
const WINDOW_FRACTION: f64 = 0.15;
const MIN_SAMPLES: f64 = 8.0;
const MAX_SAMPLES: f64 = 48.0;
/// Most pixels read along each side of a window. Every pixel of a window up
/// to this size is read, which covers spots up to about 420 px in radius.
/// A larger window is divided into this many cells a side and one pixel is
/// read from each, at a position inside its cell that a hash of the cell
/// picks. That keeps a spot's cost from growing with its size, and unlike
/// a regular sub-grid it cannot fall in step with a fine periodic texture
/// and read only its light or only its dark pixels.
const MAX_WINDOW_SIDE: i64 = 128;

/// Where in its cell, `0..stride`, the sample of cell `(column, row)` is
/// read along `axis` (0 or 1). A fixed hash, so the mean is the same every
/// time.
fn jitter(column: i64, row: i64, axis: u32, stride: i64) -> i64 {
    let mut hash = (column as u32).wrapping_mul(0x9E37_79B1)
        ^ (row as u32).wrapping_mul(0x85EB_CA77)
        ^ axis.wrapping_mul(0xC2B2_AE3D);
    hash ^= hash >> 16;
    hash = hash.wrapping_mul(0x7FEB_352D);
    hash ^= hash >> 15;
    i64::from(hash) % stride
}

/// Mean of the `(2 * half + 1)` square of pixels around `(x, y)`, with the
/// image's edge replicated. Windows wider than [`MAX_WINDOW_SIDE`] pixels
/// are estimated from one pixel per cell, as described there.
fn window_mean(image: &SourceImage, x: i64, y: i64, half: i64) -> [f32; 3] {
    let (width, height) = (i64::from(image.width), i64::from(image.height));
    let side = 2 * half + 1;
    let stride = (side + MAX_WINDOW_SIDE - 1) / MAX_WINDOW_SIDE;
    // Cells of `stride` pixels, centred on (x, y); with a stride of 1 they
    // are exactly the window's pixels.
    let cells = (side + stride - 1) / stride;
    let first = |centre: i64| centre - (cells * stride - 1) / 2;
    let (left, top) = (first(x), first(y));
    let mut sum = [0u64; 3];
    for cell_row in 0..cells {
        for cell_column in 0..cells {
            let column = left + cell_column * stride + jitter(cell_column, cell_row, 0, stride);
            let row = top + cell_row * stride + jitter(cell_column, cell_row, 1, stride);
            let index = (row.clamp(0, height - 1) as usize * image.width as usize
                + column.clamp(0, width - 1) as usize)
                * CHANNELS;
            for (total, &value) in sum.iter_mut().zip(&image.pixels[index..index + CHANNELS]) {
                *total += u64::from(value);
            }
        }
    }
    let count = (cells * cells) as f32;
    sum.map(|total| total as f32 / count)
}

/// The per-channel gain that carries a source patch to the tone of its
/// target, as boundary samples around a spot.
pub(super) struct Membrane {
    /// Radius of the sampling ring, in pixels.
    ring: f64,
    /// Unit vectors from the spot's centre to each sample.
    directions: Vec<(f32, f32)>,
    /// Target mean over source mean at each sample.
    gains: Vec<[f32; 3]>,
}

impl Membrane {
    /// Sample `image` (as it is before the spot is applied) around the
    /// target and around the source of `stamp`.
    pub(super) fn new(image: &SourceImage, stamp: &Stamp) -> Self {
        let half = (WINDOW_FRACTION * stamp.radius).round().max(1.0);
        // Far enough out that no pixel of any window lies inside the spot,
        // where the blemish being removed would tint the estimate: a
        // window's nearest corner is `half` pixels away along both axes,
        // plus up to one more for rounding its centre to a pixel.
        let ring = stamp.radius + (half + 1.0) * std::f64::consts::SQRT_2 + 0.5;
        let count = (TAU * ring / (2.0 * half + 1.0))
            .ceil()
            .clamp(MIN_SAMPLES, MAX_SAMPLES) as usize;
        let mut directions = Vec::with_capacity(count);
        let mut gains = Vec::with_capacity(count);
        for index in 0..count {
            let (sin, cos) = (TAU * index as f64 / count as f64).sin_cos();
            let x = (stamp.centre.0 + ring * cos).floor() as i64;
            let y = (stamp.centre.1 + ring * sin).floor() as i64;
            let target = window_mean(image, x, y, half as i64);
            let source = window_mean(image, x + stamp.offset.0, y + stamp.offset.1, half as i64);
            let mut gain = [1.0f32; 3];
            for channel in 0..CHANNELS {
                gain[channel] = ((target[channel] + FLOOR) / (source[channel] + FLOOR))
                    .clamp(1.0 / MAX_GAIN, MAX_GAIN);
            }
            directions.push((cos as f32, sin as f32));
            gains.push(gain);
        }
        Self {
            ring,
            directions,
            gains,
        }
    }

    /// `source` corrected for a pixel `(dx, dy)` pixels from the spot's
    /// centre. May exceed the `u16` range; the caller clamps.
    #[inline]
    pub(super) fn matched(&self, source: [f32; 3], dx: f64, dy: f64) -> [f32; 3] {
        let (x, y) = ((dx / self.ring) as f32, (dy / self.ring) as f32);
        let mut total = 0.0f32;
        let mut gain = [0.0f32; 3];
        for (&(bx, by), sample) in self.directions.iter().zip(&self.gains) {
            let (ex, ey) = (x - bx, y - by);
            // Pixels of the spot are strictly inside the ring, so this never
            // divides by zero; the floor is for safety only.
            let weight = 1.0 / (ex * ex + ey * ey).max(1e-6);
            total += weight;
            for channel in 0..CHANNELS {
                gain[channel] += weight * sample[channel];
            }
        }
        let mut out = [0.0f32; 3];
        for channel in 0..CHANNELS {
            out[channel] = (source[channel] + FLOOR) * (gain[channel] / total) - FLOOR;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use photo_core::colorimetry::RgbSpace;
    use photo_core::develop::SpotMode;
    use photo_core::image::SourceColor;

    use super::*;

    fn stamp(offset: (i64, i64)) -> Stamp {
        Stamp {
            mode: SpotMode::Heal,
            centre: (50.0, 40.0),
            radius: 10.0,
            inner: 10.0,
            outer: 10.0,
            offset,
            opacity: 1.0,
        }
    }

    fn image(f: impl Fn(usize, usize) -> u16) -> SourceImage {
        let pixels = (0..80 * 100)
            .flat_map(|i| [f(i % 100, i / 100); 3])
            .collect();
        SourceImage::new(100, 80, pixels, SourceColor::Rgb(RgbSpace::Srgb)).unwrap()
    }

    #[test]
    fn a_window_mean_replicates_the_edge() {
        let ramp = image(|x, _| x as u16 * 10);
        assert_eq!(window_mean(&ramp, 50, 40, 2), [500.0; 3]);
        // Columns -2..=2 read 0, 0, 0, 10, 20.
        assert_eq!(window_mean(&ramp, 0, 0, 2), [6.0; 3]);
    }

    #[test]
    fn windows_up_to_the_limit_are_read_in_full() {
        // A ramp's mean over a symmetric window is its centre value.
        let ramp = image(|x, _| x as u16 * 10);
        assert_eq!(window_mean(&ramp, 50, 40, 15), [500.0; 3]);
        assert_eq!(window_mean(&ramp, 50, 40, 40), [500.0; 3], "81 wide");
        let flat = image(|_, _| 777);
        assert_eq!(window_mean(&flat, 10, 10, 5000), [777.0; 3]);
    }

    #[test]
    fn a_wider_window_reads_every_phase_of_a_fine_texture() {
        // A one-pixel checker in a window read one pixel per cell of 3 x 3
        // (half 191): a regular sub-grid would see only one of the two
        // values; one pixel per cell at a hashed position sees both about
        // equally, and the same ones every time.
        let pixels = (0..400u32 * 400)
            .flat_map(|i| {
                let light = (i % 400 + i / 400) % 2 == 0;
                [if light { 30_000 } else { 10_000 }; 3]
            })
            .collect();
        let checker = SourceImage::new(400, 400, pixels, SourceColor::Rgb(RgbSpace::Srgb)).unwrap();
        let mean = window_mean(&checker, 200, 200, 191);
        assert!((mean[0] / 20_000.0 - 1.0).abs() < 0.03, "{mean:?}");
        assert_eq!(mean, window_mean(&checker, 200, 200, 191));
        // The cells are centred: a ramp still averages to about its middle.
        let pixels = (0..1000u32 * 10)
            .flat_map(|i| [(i % 1000) as u16 * 50; 3])
            .collect();
        let ramp = SourceImage::new(1000, 10, pixels, SourceColor::Rgb(RgbSpace::Srgb)).unwrap();
        let mean = window_mean(&ramp, 500, 5, 300);
        assert!((mean[0] / 25_000.0 - 1.0).abs() < 0.01, "{mean:?}");
    }

    #[test]
    fn no_window_of_the_ring_reaches_into_the_spot() {
        for radius in [3.0, 10.0, 40.0, 250.0] {
            let spot = Stamp {
                radius,
                inner: radius,
                outer: radius + 0.5,
                ..stamp((25, 10))
            };
            let half = (WINDOW_FRACTION * radius).round().max(1.0) as i64;
            let membrane = Membrane::new(&image(|_, _| 100), &spot);
            // Rebuild each window as `Membrane::new` places it, and find the
            // pixel of any window closest to the spot's centre.
            let mut closest = f64::INFINITY;
            for &(cos, sin) in &membrane.directions {
                let x = (spot.centre.0 + membrane.ring * f64::from(cos)).floor() as i64;
                let y = (spot.centre.1 + membrane.ring * f64::from(sin)).floor() as i64;
                for row in y - half..=y + half {
                    for column in x - half..=x + half {
                        let (px, py) = (column as f64 + 0.5, row as f64 + 0.5);
                        closest = closest.min((px - spot.centre.0).hypot(py - spot.centre.1));
                    }
                }
            }
            assert!(
                closest >= spot.outer,
                "radius {radius}: a window pixel at {closest}"
            );
        }
    }

    #[test]
    fn identical_surroundings_need_no_correction() {
        let flat = image(|_, _| 20_000);
        let membrane = Membrane::new(&flat, &stamp((25, 10)));
        assert!(membrane.gains.iter().all(|gain| *gain == [1.0; 3]));
        assert_eq!(membrane.matched([12_345.0; 3], 3.0, -4.0), [12_345.0; 3]);
    }

    #[test]
    fn a_uniform_mismatch_is_one_gain_everywhere() {
        // Left half at 10 000, right half at 30 000; the source is 40 px right.
        let halves = image(|x, _| if x < 70 { 10_000 } else { 30_000 });
        let membrane = Membrane::new(&halves, &stamp((40, 0)));
        let expected = (10_000.0 + FLOOR) / (30_000.0 + FLOOR);
        for (dx, dy) in [(0.0, 0.0), (7.0, 0.0), (-5.0, 8.0)] {
            let matched = membrane.matched([30_000.0; 3], dx, dy);
            assert!((matched[0] - 10_000.0).abs() < 1.0, "{matched:?}");
            assert!((matched[0] + FLOOR - (30_000.0 + FLOOR) * expected).abs() < 1.0);
        }
    }

    #[test]
    fn the_field_follows_the_nearest_part_of_the_rim() {
        // The source ring is uniformly bright; the target ring is dark on
        // the left and bright on the right, so the gain must rise to the right.
        let scene = image(|x, y| if y >= 60 || x >= 50 { 40_000 } else { 10_000 });
        let membrane = Membrane::new(&scene, &stamp((0, 35)));
        let left = membrane.matched([40_000.0; 3], -9.0, 0.0)[0];
        let middle = membrane.matched([40_000.0; 3], 0.0, 0.0)[0];
        let right = membrane.matched([40_000.0; 3], 9.0, 0.0)[0];
        assert!(left < middle && middle < right, "{left} {middle} {right}");
        assert!(left < 16_000.0 && right > 34_000.0);
    }
}
