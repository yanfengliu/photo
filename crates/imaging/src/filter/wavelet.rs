//! The "a trous" (with holes) wavelet transform and soft thresholding.
//!
//! Starck and Murtagh's isotropic undecimated wavelet transform (Starck,
//! Murtagh and Bijaoui, *Image Processing and Data Analysis: The Multiscale
//! Approach*, 1998, ch. 1; Holschneider et al., 1989, for the algorithm):
//! smooth with the B3-spline kernel `[1, 4, 6, 4, 1] / 16`, doubling the
//! spacing of its taps at every level instead of shrinking the image, and
//! keep the difference between successive smoothings as that level's detail.
//! The image is the coarsest smoothing plus the sum of all details, exactly.
//!
//! Denoising shrinks each detail coefficient toward zero by a threshold
//! (soft thresholding: Donoho, "De-noising by soft-thresholding", IEEE
//! Trans. Information Theory 1995). White noise of standard deviation
//! `sigma` leaves `0.889 sigma`, `0.200 sigma`, `0.086 sigma`, `0.041 sigma`
//! in the first four 2D bands, which is what thresholds are scaled against.

use crate::buffer::Plane;

/// B3-spline scaling kernel.
const KERNEL: [f32; 5] = [1.0 / 16.0, 4.0 / 16.0, 6.0 / 16.0, 4.0 / 16.0, 1.0 / 16.0];

/// Levels beyond this are ignored: the taps would be 4096 pixels apart.
pub const MAX_WAVELET_LEVELS: usize = 12;

fn smooth_line(source: &[f32], out: &mut [f32], spacing: usize) {
    let len = source.len();
    let last = len as isize - 1;
    let reach = 2 * spacing;
    let tap = |x: usize, k: usize| {
        let index = x as isize + (k as isize - 2) * spacing as isize;
        source[index.clamp(0, last) as usize]
    };
    let weighted = |taps: [f32; 5]| {
        KERNEL[0] * taps[0]
            + KERNEL[1] * taps[1]
            + KERNEL[2] * taps[2]
            + KERNEL[3] * taps[3]
            + KERNEL[4] * taps[4]
    };
    let interior = reach.min(len)..len.saturating_sub(reach).max(reach.min(len));
    for x in (0..interior.start).chain(interior.end..len) {
        out[x] = weighted([tap(x, 0), tap(x, 1), tap(x, 2), tap(x, 3), tap(x, 4)]);
    }
    if interior.is_empty() {
        return;
    }
    let shifted = |k: usize| {
        &source[interior.start + k * spacing - reach..interior.end + k * spacing - reach]
    };
    let (a, b, c, d, e) = (shifted(0), shifted(1), shifted(2), shifted(3), shifted(4));
    let samples = out[interior.clone()]
        .iter_mut()
        .zip(a)
        .zip(b)
        .zip(c)
        .zip(d)
        .zip(e);
    for (((((value, &a), &b), &c), &d), &e) in samples {
        *value = weighted([a, b, c, d, e]);
    }
}

/// Smooth `source` into `out` with taps `spacing` pixels apart, using
/// `scratch` for the row pass. Edges are replicated.
fn smooth(source: &Plane, scratch: &mut Plane, out: &mut Plane, spacing: usize) {
    scratch.par_rows_mut(|y, row| smooth_line(source.row(y), row, spacing));
    let last = source.height() as isize - 1;
    let scratch = &*scratch;
    out.par_rows_mut(|y, row| {
        let line = |k: isize| {
            scratch.row((y as isize + (k - 2) * spacing as isize).clamp(0, last) as usize)
        };
        let (a, b, c, d, e) = (line(0), line(1), line(2), line(3), line(4));
        let samples = row.iter_mut().zip(a).zip(b).zip(c).zip(d).zip(e);
        for (((((value, &a), &b), &c), &d), &e) in samples {
            *value = KERNEL[0] * a + KERNEL[1] * b + KERNEL[2] * c + KERNEL[3] * d + KERNEL[4] * e;
        }
    });
}

/// Shrink `value` toward zero by `threshold`: values within the threshold
/// become 0, larger ones lose exactly `threshold` of their magnitude.
/// Continuous, odd, and never changes a sign. A threshold of 0 or less
/// returns `value` unchanged.
#[inline]
pub fn soft_threshold(value: f32, threshold: f32) -> f32 {
    let threshold = threshold.max(0.0);
    if value > threshold {
        value - threshold
    } else if value < -threshold {
        value + threshold
    } else if threshold > 0.0 {
        0.0
    } else {
        value
    }
}

/// Farthest pixel that can influence an output pixel after `levels` levels
/// of [`wavelet_decompose`] or [`wavelet_denoise`]: `2 * (2^levels - 1)`.
pub fn wavelet_support(levels: usize) -> usize {
    2 * ((1usize << levels.min(MAX_WAVELET_LEVELS)) - 1)
}

/// A plane split into detail bands and a smooth residual.
///
/// Level `n` (from 0, the finest) holds structure about `2^n` to `2^(n+1)`
/// pixels across. The bands and the residual sum back to the original plane
/// to within floating-point rounding.
#[derive(Clone, Debug, PartialEq)]
pub struct WaveletDecomposition {
    details: Vec<Plane>,
    residual: Plane,
}

impl WaveletDecomposition {
    /// The detail bands, finest first. Each has zero mean over smooth areas.
    pub fn details(&self) -> &[Plane] {
        &self.details
    }

    /// The detail bands, finest first, for editing in place (scaling a band
    /// sharpens or softens structure of that size).
    pub fn details_mut(&mut self) -> &mut [Plane] {
        &mut self.details
    }

    /// What is left after every detail band is removed: the plane smoothed
    /// at the coarsest scale.
    pub fn residual(&self) -> &Plane {
        &self.residual
    }

    /// Apply [`soft_threshold`] to every coefficient of detail band `n` with
    /// `thresholds[n]`. Bands without a threshold are left alone, as are
    /// thresholds without a band.
    pub fn soft_threshold(&mut self, thresholds: &[f32]) {
        for (detail, &threshold) in self.details.iter_mut().zip(thresholds) {
            if threshold > 0.0 {
                detail.par_rows_mut(|_, row| {
                    for value in row {
                        *value = soft_threshold(*value, threshold);
                    }
                });
            }
        }
    }

    /// Sum the bands and the residual back into a plane. Without edits this
    /// is the plane that was decomposed, to within rounding (a few `1e-7`
    /// for data in `0..1`).
    pub fn reconstruct(&self) -> Plane {
        let mut out = self.residual.clone();
        for detail in self.details.iter().rev() {
            out.par_rows_mut(|y, row| {
                for (value, &d) in row.iter_mut().zip(detail.row(y)) {
                    *value += d;
                }
            });
        }
        out
    }
}

/// Decompose `input` into `levels` detail bands and a residual with the a
/// trous B3-spline transform. `levels` is capped at
/// [`MAX_WAVELET_LEVELS`]; 0 gives no bands and the input as the residual.
///
/// Every band has the size of the input (nothing is decimated), so memory
/// is `levels + 1` planes; use [`wavelet_denoise`] when only the thresholded
/// result is wanted. Edges are replicated. Support: [`wavelet_support`].
pub fn wavelet_decompose(input: &Plane, levels: usize) -> WaveletDecomposition {
    let levels = levels.min(MAX_WAVELET_LEVELS);
    let (width, height) = (input.width(), input.height());
    let mut scratch = Plane::new(width, height);
    let mut current = input.clone();
    let mut details = Vec::with_capacity(levels);
    for level in 0..levels {
        let mut smoothed = Plane::new(width, height);
        smooth(&current, &mut scratch, &mut smoothed, 1 << level);
        current.par_rows_mut(|y, row| {
            for (value, &s) in row.iter_mut().zip(smoothed.row(y)) {
                *value -= s;
            }
        });
        details.push(std::mem::replace(&mut current, smoothed));
    }
    WaveletDecomposition {
        details,
        residual: current,
    }
}

/// Decompose, soft-threshold and reconstruct in one go, in four planes of
/// memory however many bands there are: band `n` is shrunk by
/// `thresholds[n]`, and there are as many bands as thresholds (capped at
/// [`MAX_WAVELET_LEVELS`]).
///
/// The same result as [`wavelet_decompose`] followed by
/// [`WaveletDecomposition::soft_threshold`] and
/// [`WaveletDecomposition::reconstruct`], to within rounding. All-zero
/// thresholds return the input, to within rounding. Thresholds are in the
/// units of the plane, so apply this to a variance-stabilized encoding if
/// noise is to be treated alike in shadows and highlights.
///
/// Edges are replicated. Support: [`wavelet_support`]`(thresholds.len())`.
pub fn wavelet_denoise(input: &Plane, thresholds: &[f32]) -> Plane {
    let (width, height) = (input.width(), input.height());
    let mut scratch = Plane::new(width, height);
    let mut smoothed = Plane::new(width, height);
    let mut current = input.clone();
    let mut out = Plane::new(width, height);
    for (level, &threshold) in thresholds.iter().take(MAX_WAVELET_LEVELS).enumerate() {
        smooth(&current, &mut scratch, &mut smoothed, 1 << level);
        out.par_rows_mut(|y, row| {
            let bands = row.iter_mut().zip(current.row(y)).zip(smoothed.row(y));
            for ((value, &fine), &coarse) in bands {
                *value += soft_threshold(fine - coarse, threshold);
            }
        });
        std::mem::swap(&mut current, &mut smoothed);
    }
    out.par_rows_mut(|y, row| {
        for (value, &residual) in row.iter_mut().zip(current.row(y)) {
            *value += residual;
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_smoothing_kernel_has_holes() {
        let mut impulse = [0.0f32; 17];
        impulse[8] = 16.0;
        let mut out = [0.0f32; 17];
        smooth_line(&impulse, &mut out, 1);
        assert_eq!(out[6..11], [1.0, 4.0, 6.0, 4.0, 1.0]);
        smooth_line(&impulse, &mut out, 2);
        assert_eq!(out[4..13], [1.0, 0.0, 4.0, 0.0, 6.0, 0.0, 4.0, 0.0, 1.0]);
        smooth_line(&impulse, &mut out, 4);
        assert_eq!(
            (out[0], out[4], out[8], out[12], out[16]),
            (1.0, 4.0, 6.0, 4.0, 1.0)
        );
    }

    #[test]
    fn a_line_shorter_than_the_kernel_is_all_edge() {
        let mut out = [0.0f32; 3];
        smooth_line(&[1.0, 1.0, 1.0], &mut out, 8);
        assert_eq!(out, [1.0, 1.0, 1.0]);
        smooth_line(&[0.0, 16.0, 0.0], &mut out, 1);
        assert_eq!(out, [4.0, 6.0, 4.0]);
    }

    #[test]
    fn support_doubles_with_every_level() {
        assert_eq!(wavelet_support(0), 0);
        assert_eq!(wavelet_support(1), 2);
        assert_eq!(wavelet_support(3), 14);
        assert_eq!(wavelet_support(99), wavelet_support(MAX_WAVELET_LEVELS));
    }
}
