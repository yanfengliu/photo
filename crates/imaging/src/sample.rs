//! Point sampling of a [`SourceImage`]: the kernels behind
//! [`SourcePyramid::sample`](crate::SourcePyramid::sample) and
//! [`resample`](crate::resample()).
//!
//! Coordinates are in pixels of the image being sampled, with pixel centres
//! at `+0.5`. Everything clamps to the edge: a tap outside the image reads
//! the nearest edge pixel. Results are `0..1` (`u16` code / 65535).

use photo_core::image::SourceImage;

const WHITE: f32 = SourceImage::WHITE as f32;
const CHANNELS: usize = SourceImage::CHANNELS;

/// Footprints up to this are sampled with the cubic alone. The margin keeps
/// an exact 1:1 or half-size view on the interpolating kernel after its scale
/// has been through `f32`.
const CUBIC_UNTIL: f64 = 1.001;
/// Footprints from this up are sampled with the triangle alone.
const TRIANGLE_FROM: f64 = 1.5;
/// Widest triangle the four taps per axis can hold, in pixels.
const MAX_TRIANGLE_RADIUS: f64 = 2.0;

/// How the four taps per axis of [`sample4`] are weighted: a blend of two
/// kernels, chosen by how many pixels of the sampled image one output pixel
/// covers (the footprint).
///
/// - The **cubic** is Catmull-Rom (Catmull and Rom, 1974; the `B = 0,
///   C = 1/2` member of the Mitchell-Netravali family). It interpolates: a
///   sample at a pixel centre returns that pixel exactly. Its result is
///   clamped to the range of the four nearest pixels, which removes the
///   overshoot its negative lobes would otherwise leave next to hard edges.
/// - The **triangle** is as wide as the footprint: an area-aware filter for
///   shrinking by less than the next pyramid level, where interpolating
///   would point-sample and alias.
///
/// Between a footprint of 1 and 1.5 the two are mixed, so the kernel is a
/// continuous function of the footprint: a view that is one percent smaller,
/// or a touch of lens distortion, softens the picture by a hair instead of
/// switching every pixel from one kernel to the other.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Kernel {
    /// Share of the triangle in the mix: 0 is the cubic alone, 1 the
    /// triangle alone.
    triangle: f32,
    /// Half-width of the triangle, in pixels, 1 to 2.
    radius: f32,
}

impl Kernel {
    /// The cubic alone.
    #[cfg(test)]
    pub(crate) const CUBIC: Self = Self {
        triangle: 0.0,
        radius: 1.0,
    };

    /// The kernel for an output pixel that covers `footprint` pixels of the
    /// sampled image along each axis.
    pub(crate) fn for_footprint(footprint: f64) -> Self {
        let over = ((footprint - CUBIC_UNTIL) / (TRIANGLE_FROM - CUBIC_UNTIL)).clamp(0.0, 1.0);
        // NaN compares false and lands on the cubic, like any footprint
        // that needs no triangle.
        if over > 0.0 {
            Self {
                triangle: (over * over * (3.0 - 2.0 * over)) as f32,
                radius: footprint.clamp(1.0, MAX_TRIANGLE_RADIUS) as f32,
            }
        } else {
            Self {
                triangle: 0.0,
                radius: 1.0,
            }
        }
    }

    /// True when a sample at a pixel centre is that pixel, exactly.
    fn interpolates(&self) -> bool {
        self.triangle == 0.0
    }
}

/// Split a coordinate into the index of the pixel whose centre is at or just
/// before it and the fraction of the way to the next centre. Non-finite
/// coordinates are treated as 0 so a bad position can never produce NaN.
#[inline]
fn split(coordinate: f64) -> (i64, f32) {
    let shifted = if coordinate.is_finite() {
        (coordinate - 0.5).clamp(-4.0e9, 4.0e9)
    } else {
        -0.5
    };
    let floor = shifted.floor();
    (floor as i64, (shifted - floor) as f32)
}

#[inline]
fn clamp_index(index: i64, len: usize) -> usize {
    index.clamp(0, len as i64 - 1) as usize
}

#[inline]
fn row(image: &SourceImage, y: usize) -> &[u16] {
    let stride = image.width as usize * CHANNELS;
    &image.pixels[y * stride..(y + 1) * stride]
}

#[inline]
fn texel(row: &[u16], x: usize) -> [f32; 3] {
    let i = x * CHANNELS;
    [row[i] as f32, row[i + 1] as f32, row[i + 2] as f32]
}

/// Bilinear sample: support is the 2 x 2 pixels around the position.
#[inline]
pub(crate) fn bilinear(image: &SourceImage, x: f64, y: f64) -> [f32; 3] {
    let (width, height) = (image.width as usize, image.height as usize);
    let (ix, tx) = split(x);
    let (iy, ty) = split(y);
    let (x0, x1) = (clamp_index(ix, width), clamp_index(ix + 1, width));
    let top = row(image, clamp_index(iy, height));
    let bottom = row(image, clamp_index(iy + 1, height));
    let (a, b, c, d) = (
        texel(top, x0),
        texel(top, x1),
        texel(bottom, x0),
        texel(bottom, x1),
    );
    let mut out = [0.0; 3];
    for channel in 0..3 {
        let upper = a[channel] + (b[channel] - a[channel]) * tx;
        let lower = c[channel] + (d[channel] - c[channel]) * tx;
        out[channel] = (upper + (lower - upper) * ty) / WHITE;
    }
    out
}

/// Weights of the taps at offsets -1, 0, +1, +2 from the pixel at or before
/// the position, for a fraction `t` in `0..=1` past that pixel's centre.
#[inline]
fn weights(t: f32, kernel: Kernel) -> [f32; 4] {
    let cubic = || {
        [
            ((-0.5 * t + 1.0) * t - 0.5) * t,
            (1.5 * t - 2.5) * t * t + 1.0,
            ((-1.5 * t + 2.0) * t + 0.5) * t,
            (0.5 * t - 0.5) * t * t,
        ]
    };
    let triangle = || {
        let inverse = 1.0 / kernel.radius;
        let raw =
            [t + 1.0, t, 1.0 - t, 2.0 - t].map(|distance| (1.0 - distance * inverse).max(0.0));
        let normalize = 1.0 / (raw[0] + raw[1] + raw[2] + raw[3]);
        raw.map(|w| w * normalize)
    };
    if kernel.triangle <= 0.0 {
        cubic()
    } else if kernel.triangle >= 1.0 {
        triangle()
    } else {
        let (sharp, soft) = (cubic(), triangle());
        std::array::from_fn(|i| sharp[i] + (soft[i] - sharp[i]) * kernel.triangle)
    }
}

/// Samples in four pixels.
const TAP_SAMPLES: usize = 4 * CHANNELS;

/// The four pixels of `line` from column `ix - 1` to `ix + 2`, as twelve
/// interleaved samples, with the edge replicated.
#[inline]
fn taps(line: &[u16], ix: i64, width: usize, samples: &mut [u16; TAP_SAMPLES]) {
    if ix >= 1 && ix + 2 < width as i64 {
        let start = (ix - 1) as usize * CHANNELS;
        samples.copy_from_slice(&line[start..start + TAP_SAMPLES]);
    } else {
        for (i, pixel) in samples.chunks_exact_mut(CHANNELS).enumerate() {
            let from = clamp_index(ix - 1 + i as i64, width) * CHANNELS;
            pixel.copy_from_slice(&line[from..from + CHANNELS]);
        }
    }
}

/// Four-tap separable sample: support is the 4 x 4 pixels around the
/// position (two on each side of it).
///
/// Every sum is `w0 * a + w1 * b + w2 * c + w3 * d` in that order, so with
/// weights `(0, 1, 0, 0)` on both axes the result is the pixel itself,
/// without rounding.
#[inline]
pub(crate) fn sample4(image: &SourceImage, x: f64, y: f64, kernel: Kernel) -> [f32; 3] {
    let (width, height) = (image.width as usize, image.height as usize);
    let (ix, tx) = split(x);
    let (iy, ty) = split(y);
    if tx == 0.0 && ty == 0.0 && kernel.interpolates() {
        // On a pixel centre the cubic's weights are (0, 1, 0, 0): the result
        // is that pixel. Unrotated views at 1:1 are all of this kind, so it
        // is worth not computing sixteen taps to find that out.
        let pixel = texel(row(image, clamp_index(iy, height)), clamp_index(ix, width));
        return pixel.map(|value| value / WHITE);
    }
    let wx = weights(tx, kernel);
    let wy = weights(ty, kernel);
    let mut lines = [[0u16; TAP_SAMPLES]; 4];
    if ix >= 1 && iy >= 1 && ix + 2 < width as i64 && iy + 2 < height as i64 {
        // The whole block is inside the image: one range check for all of it.
        let stride = width * CHANNELS;
        let first = (iy - 1) as usize * stride + (ix - 1) as usize * CHANNELS;
        let block = &image.pixels[first..first + 3 * stride + TAP_SAMPLES];
        for (j, samples) in lines.iter_mut().enumerate() {
            samples.copy_from_slice(&block[j * stride..j * stride + TAP_SAMPLES]);
        }
    } else {
        for (j, samples) in lines.iter_mut().enumerate() {
            taps(
                row(image, clamp_index(iy - 1 + j as i64, height)),
                ix,
                width,
                samples,
            );
        }
    }

    // Down the four rows first: twelve independent sums over contiguous
    // samples, which the compiler turns into vector code.
    let mut column = [0.0f32; TAP_SAMPLES];
    for (k, value) in column.iter_mut().enumerate() {
        *value = wy[0] * f32::from(lines[0][k])
            + wy[1] * f32::from(lines[1][k])
            + wy[2] * f32::from(lines[2][k])
            + wy[3] * f32::from(lines[3][k]);
    }
    // Then across the four pixels.
    let mut out: [f32; 3] = std::array::from_fn(|channel| {
        wx[0] * column[channel]
            + wx[1] * column[CHANNELS + channel]
            + wx[2] * column[2 * CHANNELS + channel]
            + wx[3] * column[3 * CHANNELS + channel]
    });

    if kernel.triangle < 1.0 {
        // The cubic has negative lobes: keep its result inside the range of
        // the four pixels around the position (the middle two of the middle
        // two rows). As the triangle takes over, which legitimately averages
        // more than those four, the allowed range widens to all sixteen.
        for (channel, value) in out.iter_mut().enumerate() {
            let near = [
                lines[1][CHANNELS + channel],
                lines[1][2 * CHANNELS + channel],
                lines[2][CHANNELS + channel],
                lines[2][2 * CHANNELS + channel],
            ];
            let mut low = f32::from(near[0].min(near[1]).min(near[2]).min(near[3]));
            let mut high = f32::from(near[0].max(near[1]).max(near[2]).max(near[3]));
            if kernel.triangle > 0.0 {
                let all = lines.iter().flat_map(|line| line.iter().skip(channel).step_by(CHANNELS));
                let (lowest, highest) = all.fold((u16::MAX, 0), |(l, h), &v| (l.min(v), h.max(v)));
                low += (f32::from(lowest) - low) * kernel.triangle;
                high += (f32::from(highest) - high) * kernel.triangle;
            }
            *value = value.clamp(low, high);
        }
    }
    // True divisions, so an exact pixel is exactly `code / 65535`; four at
    // once because a divider handles a full vector as fast as one lane.
    let scaled = [out[0], out[1], out[2], 0.0].map(|value| value / WHITE);
    [scaled[0], scaled[1], scaled[2]]
}

#[cfg(test)]
mod tests {
    use photo_core::colorimetry::RgbSpace;
    use photo_core::image::SourceColor;

    use super::*;

    fn ramp() -> SourceImage {
        let pixels = (0..6u16 * 5)
            .flat_map(|i| [i * 1000, i * 500, 65535 - i * 100])
            .collect();
        SourceImage::new(6, 5, pixels, SourceColor::Rgb(RgbSpace::Srgb)).unwrap()
    }

    /// A scrambled image: every pixel unrelated to its neighbours.
    fn scrambled() -> SourceImage {
        let pixels = (0..12u32 * 9 * 3)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u16)
            .collect();
        SourceImage::new(12, 9, pixels, SourceColor::Rgb(RgbSpace::Srgb)).unwrap()
    }

    /// The pure triangle of a given half-width.
    fn triangle(radius: f64) -> Kernel {
        let kernel = Kernel::for_footprint(radius);
        assert_eq!(kernel.triangle, 1.0, "{radius} is still in the blend");
        kernel
    }

    /// `sample4` by its definition, reading every tap through a clamp.
    fn reference(image: &SourceImage, x: f64, y: f64, kernel: Kernel) -> [f32; 3] {
        let (ix, tx) = split(x);
        let (iy, ty) = split(y);
        let (wx, wy) = (weights(tx, kernel), weights(ty, kernel));
        let tap = |i: i64, j: i64, channel: usize| {
            let column = clamp_index(ix - 1 + i, image.width as usize) as u32;
            let line = clamp_index(iy - 1 + j, image.height as usize) as u32;
            f32::from(image.pixel(column, line)[channel])
        };
        let range = |taps: &[f32]| {
            taps.iter()
                .fold((f32::INFINITY, 0.0f32), |(l, h), &v| (l.min(v), h.max(v)))
        };
        std::array::from_fn(|channel| {
            let down = |i: i64| {
                wy[0] * tap(i, 0, channel)
                    + wy[1] * tap(i, 1, channel)
                    + wy[2] * tap(i, 2, channel)
                    + wy[3] * tap(i, 3, channel)
            };
            let value = wx[0] * down(0) + wx[1] * down(1) + wx[2] * down(2) + wx[3] * down(3);
            if kernel.triangle >= 1.0 {
                return value / WHITE;
            }
            let near = [(1, 1), (2, 1), (1, 2), (2, 2)].map(|(i, j)| tap(i, j, channel));
            let all: Vec<f32> = (0..16).map(|k| tap(k % 4, k / 4, channel)).collect();
            let ((mut low, mut high), (lowest, highest)) = (range(&near), range(&all));
            if kernel.triangle > 0.0 {
                low += (lowest - low) * kernel.triangle;
                high += (highest - high) * kernel.triangle;
            }
            value.clamp(low, high) / WHITE
        })
    }

    #[test]
    fn the_fast_paths_agree_with_the_definition_everywhere() {
        for image in [ramp(), scrambled()] {
            for kernel in [Kernel::CUBIC, Kernel::for_footprint(1.2), triangle(1.6)] {
                // A grid of positions from well outside the image, across
                // both borders and the interior: off the pixel grid, and
                // then exactly on pixel centres, which take a shortcut.
                for (pitch_x, pitch_y, offset) in
                    [(0.23, 0.19, 0.0), (1.0, 1.0, 0.5), (1.0, 0.25, 0.5)]
                {
                    for step_y in -12..60 {
                        for step_x in -12..70 {
                            let (x, y) = (
                                step_x as f64 * pitch_x + offset,
                                step_y as f64 * pitch_y + offset,
                            );
                            assert_eq!(
                                sample4(&image, x, y, kernel),
                                reference(&image, x, y, kernel),
                                "{kernel:?} at ({x}, {y})"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn weights_sum_to_one_and_the_cubic_interpolates() {
        for footprint in [0.3, 1.0, 1.1, 1.3, 1.5, 2.0, 7.0] {
            let kernel = Kernel::for_footprint(footprint);
            for step in 0..=16 {
                let w = weights(step as f32 / 16.0, kernel);
                assert!(
                    (w.iter().sum::<f32>() - 1.0).abs() < 1e-6,
                    "{kernel:?} {step}"
                );
            }
        }
        assert_eq!(weights(0.0, Kernel::CUBIC), [0.0, 1.0, 0.0, 0.0]);
        assert_eq!(weights(1.0, Kernel::CUBIC), [0.0, 0.0, 1.0, 0.0]);
    }

    #[test]
    fn the_kernel_follows_the_footprint_without_a_jump() {
        // Magnifying, 1:1 and an exact size that has been through f32 all
        // get the cubic alone.
        for footprint in [0.25, 1.0, 1.0005] {
            assert_eq!(Kernel::for_footprint(footprint), Kernel::CUBIC, "{footprint}");
        }
        assert!(Kernel::CUBIC.interpolates());
        // From there the triangle fades in; it is alone from 1.5 and as wide
        // as the footprint, up to the two pixels four taps can hold.
        let share = |footprint: f64| Kernel::for_footprint(footprint).triangle;
        assert!(share(1.0025) > 0.0 && share(1.0025) < 1e-4);
        assert!(share(1.1) < share(1.25) && share(1.25) < share(1.4));
        assert_eq!((share(1.5), share(1.9), share(40.0)), (1.0, 1.0, 1.0));
        assert_eq!(Kernel::for_footprint(1.7).radius, 1.7);
        assert_eq!(Kernel::for_footprint(40.0).radius, 2.0);
        assert!(!Kernel::for_footprint(1.0025).interpolates());
        // Neighbouring footprints give neighbouring weights, all the way.
        let mut previous = weights(0.3, Kernel::for_footprint(0.9));
        for step in 0..=1200 {
            let current = weights(0.3, Kernel::for_footprint(0.9 + step as f64 * 0.001));
            for (a, b) in previous.iter().zip(current) {
                assert!((a - b).abs() < 3e-3, "step {step}: {previous:?} -> {current:?}");
            }
            previous = current;
        }
    }

    #[test]
    fn a_triangle_never_has_negative_weights_and_is_symmetric() {
        for step in 0..=16 {
            let t = step as f32 / 16.0;
            let forward = weights(t, triangle(1.5));
            let mut mirrored = weights(1.0 - t, triangle(1.5));
            mirrored.reverse();
            for (a, b) in forward.iter().zip(mirrored) {
                assert!(*a >= 0.0 && (a - b).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn a_mix_never_leaves_the_range_of_its_sixteen_pixels() {
        let image = scrambled();
        for footprint in [1.0, 1.05, 1.2, 1.45, 1.8] {
            let kernel = Kernel::for_footprint(footprint);
            for step in 0..400 {
                let (x, y) = (2.0 + step as f64 * 0.019, 2.0 + step as f64 * 0.011);
                let got = sample4(&image, x, y, kernel);
                let (ix, iy) = (split(x).0, split(y).0);
                for channel in 0..3 {
                    let taps = (0..16).map(|k| {
                        let pixel = image.pixel((ix - 1 + k % 4) as u32, (iy - 1 + k / 4) as u32);
                        f32::from(pixel[channel]) / WHITE
                    });
                    let (low, high) = taps.fold((1.0f32, 0.0f32), |(l, h), v| (l.min(v), h.max(v)));
                    let value = got[channel];
                    assert!(
                        value >= low - 1e-6 && value <= high + 1e-6,
                        "{footprint} at ({x}, {y})"
                    );
                }
            }
        }
    }

    #[test]
    fn both_samplers_return_the_pixel_at_its_centre() {
        let image = ramp();
        for (x, y) in [(0u32, 0u32), (3, 2), (5, 4)] {
            let expected = image.pixel(x, y).map(|v| v as f32 / WHITE);
            let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
            assert_eq!(bilinear(&image, fx, fy), expected);
            assert_eq!(sample4(&image, fx, fy, Kernel::CUBIC), expected);
        }
    }

    #[test]
    fn the_cubic_reproduces_a_linear_ramp_between_centres() {
        let image = ramp();
        // Red rises by 1000 per pixel along a row, away from the edges.
        let value = sample4(&image, 2.75, 2.5, Kernel::CUBIC)[0] * WHITE;
        let left = image.pixel(1, 2)[0] as f32;
        assert!((value - (left + 1250.0)).abs() < 0.01, "{value}");
    }

    #[test]
    fn positions_outside_the_image_and_non_finite_ones_stay_finite() {
        let image = ramp();
        let corner = image.pixel(5, 4).map(|v| v as f32 / WHITE);
        for kernel in [Kernel::CUBIC, Kernel::for_footprint(1.2), triangle(1.7)] {
            let far = sample4(&image, 1.0e12, 1.0e12, kernel);
            for channel in 0..3 {
                assert!((far[channel] - corner[channel]).abs() < 1e-6);
            }
            assert!(
                sample4(&image, f64::NAN, f64::INFINITY, kernel)
                    .iter()
                    .all(|v| v.is_finite())
            );
        }
        assert_eq!(bilinear(&image, 1.0e12, 1.0e12), corner);
        assert!(
            bilinear(&image, f64::NAN, 0.0)
                .iter()
                .all(|v| v.is_finite())
        );
    }
}
