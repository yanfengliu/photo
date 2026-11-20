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
/// switching every pixel from one kernel to the other. That holds within
/// one image; [`resample`](crate::resample()) still steps from one pyramid
/// level to the next.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Kernel {
    /// Share of the triangle in the mix: 0 is the cubic alone, 1 the
    /// triangle alone.
    triangle: f32,
    /// One over the half-width of the triangle in pixels (1 to 2), worked
    /// out once rather than for every sample.
    inverse_radius: f32,
}

impl Kernel {
    /// The cubic alone.
    const CUBIC: Self = Self {
        triangle: 0.0,
        inverse_radius: 1.0,
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
                inverse_radius: 1.0 / footprint.clamp(1.0, MAX_TRIANGLE_RADIUS) as f32,
            }
        } else {
            Self::CUBIC
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
        let inverse = kernel.inverse_radius;
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
        // Ranges are taken across the rows sample by sample first, in plain
        // loops over arrays the compiler can do in vector registers.
        let (mut near_low, mut near_high) = (lines[1], lines[1]);
        for k in 0..TAP_SAMPLES {
            near_low[k] = near_low[k].min(lines[2][k]);
            near_high[k] = near_high[k].max(lines[2][k]);
        }
        let (mut all_low, mut all_high) = (near_low, near_high);
        if kernel.triangle > 0.0 {
            for line in [&lines[0], &lines[3]] {
                for k in 0..TAP_SAMPLES {
                    all_low[k] = all_low[k].min(line[k]);
                    all_high[k] = all_high[k].max(line[k]);
                }
            }
        }
        for (channel, value) in out.iter_mut().enumerate() {
            let (a, b, c, d) = (
                channel,
                CHANNELS + channel,
                2 * CHANNELS + channel,
                3 * CHANNELS + channel,
            );
            let mut low = f32::from(near_low[b].min(near_low[c]));
            let mut high = f32::from(near_high[b].max(near_high[c]));
            if kernel.triangle > 0.0 {
                let lowest = all_low[a].min(all_low[b]).min(all_low[c]).min(all_low[d]);
                let highest = all_high[a]
                    .max(all_high[b])
                    .max(all_high[c])
                    .max(all_high[d]);
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
mod tests;
