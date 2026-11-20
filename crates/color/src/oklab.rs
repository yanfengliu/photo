//! Oklab (Björn Ottosson, "A perceptual color space for image processing",
//! 2020) to and from linear working RGB, plus the hue parameterisation the
//! hue-selective operators index their tables with.
//!
//! The RGB-to-LMS matrix is Ottosson's XYZ-to-LMS matrix `M1` composed with
//! the working space's RGB-to-XYZ matrix, with each row then scaled to sum
//! to one, exactly as his published sRGB matrix is. Equal RGB channels
//! therefore give equal LMS and `a = b = 0`.

use std::sync::LazyLock;

use photo_core::colorimetry::{IDENTITY, Mat3, WORKING_SPACE, mat3_apply};

use crate::math::{Mat3d, invert_d, mul_d, narrow, normalize_rows, widen};

/// Ottosson's `M1`: XYZ (D65) to cone-like LMS.
const XYZ_TO_LMS: Mat3d = [
    [0.818_933_010_1, 0.361_866_742_4, -0.128_859_713_7],
    [0.032_984_543_6, 0.929_311_871_5, 0.036_145_638_7],
    [0.048_200_301_8, 0.264_366_269_1, 0.633_851_707_0],
];

struct Matrices {
    to_lms: Mat3,
    from_lms: Mat3,
}

static MATRICES: LazyLock<Matrices> = LazyLock::new(|| {
    let mut to_lms = mul_d(&XYZ_TO_LMS, &widen(&WORKING_SPACE.to_xyz()));
    normalize_rows(&mut to_lms);
    match invert_d(&to_lms) {
        Some(from_lms) => Matrices {
            to_lms: narrow(&to_lms),
            from_lms: narrow(&from_lms),
        },
        None => Matrices {
            to_lms: IDENTITY,
            from_lms: IDENTITY,
        },
    }
});

/// Cube root for the forward transform, accurate to about one unit in the
/// last place and several times cheaper than the library call it replaces
/// (three of these are the largest single cost of the colour stage).
///
/// An estimate of `x^(-1/3)` is read off the bit pattern (the exponent
/// divided by three, as in the well-known reciprocal square root trick) and
/// refined by two division-free Newton steps `r <- r (4 - x r^3) / 3`; the
/// cube root is `x r^2`, polished by one Newton step on `y^3 = x`.
///
/// Zero, negative, NaN and sub-`1e-30` inputs give zero: cone signals of a
/// working-space colour are never negative.
#[inline(always)]
fn cbrt(value: f32) -> f32 {
    // Straight-line code with one select at the end, so that the three
    // calls per pixel can be vectorized together.
    // A NaN passes through the clamp and is discarded by the select below.
    let x = value.clamp(1.0e-30, 1.0e30);
    let third = x * (1.0 / 3.0);
    let mut r = f32::from_bits(0x54a2_fa8c - x.to_bits() / 3);
    r *= 4.0 / 3.0 - third * (r * r * r);
    r *= 4.0 / 3.0 - third * (r * r * r);
    let r2 = r * r;
    let y = x * r2;
    let root = y - (y * y * y - x) * (r2 * (1.0 / 3.0));
    if value > 1.0e-30 { root } else { 0.0 }
}

/// Linear working RGB to Oklab `[L, a, b]`. `L` is 1 for working white, and
/// `a` and `b` are zero (to rounding, a few 1e-7) for neutral input.
// The coefficients are kept as published, digit for digit.
#[allow(clippy::excessive_precision)]
#[inline]
pub fn working_to_oklab(rgb: [f32; 3]) -> [f32; 3] {
    let [l, m, s] = mat3_apply(&MATRICES.to_lms, rgb).map(cbrt);
    // Ottosson's `M2`. The second and third rows sum to zero, so they are
    // written as differences: equal cone signals give exactly zero chroma.
    [
        0.210_454_255_3 * l + 0.793_617_785_0 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * (l - m) + 0.450_593_709_9 * (s - m),
        0.025_904_037_1 * (l - s) + 0.782_771_766_2 * (m - s),
    ]
}

/// Oklab `[L, a, b]` back to linear working RGB. Inverse of
/// [`working_to_oklab`] to `f32` precision. Colours outside the working
/// gamut come back with a negative channel.
// The coefficients are kept as published, digit for digit.
#[allow(clippy::excessive_precision)]
#[inline]
pub fn oklab_to_working(lab: [f32; 3]) -> [f32; 3] {
    let l = lab[0] + 0.396_337_777_4 * lab[1] + 0.215_803_757_3 * lab[2];
    let m = lab[0] - 0.105_561_345_8 * lab[1] - 0.063_854_172_8 * lab[2];
    let s = lab[0] - 0.089_484_177_5 * lab[1] - 1.291_485_548_0 * lab[2];
    mat3_apply(&MATRICES.from_lms, [l * l * l, m * m * m, s * s * s])
}

/// The Oklab hue angle of `(a, b)` in degrees, 0 to 360. Zero for a neutral.
pub fn hue_degrees(a: f32, b: f32) -> f32 {
    if a == 0.0 && b == 0.0 {
        return 0.0;
    }
    let degrees = b.atan2(a).to_degrees();
    if degrees < 0.0 {
        degrees + 360.0
    } else {
        degrees
    }
}

/// Cells of a [`HueLut`] around the full circle (a multiple of four).
const HUE_CELLS: usize = 360;
const CELLS_PER_QUADRANT: f32 = (HUE_CELLS / 4) as f32;

/// The "diamond angle" of `(a, b)`: a value in `[0, 4)` that increases
/// monotonically with the true angle (one unit per quadrant) and costs one
/// division instead of an `atan2`. C1 around the whole circle.
#[inline(always)]
fn pseudo_angle(a: f32, b: f32) -> f32 {
    let sum = a.abs() + b.abs();
    if sum <= 0.0 || sum.is_nan() {
        return 0.0;
    }
    let p = b / sum;
    if a < 0.0 {
        2.0 - p
    } else if b < 0.0 {
        4.0 + p
    } else {
        p
    }
}

/// The true angle in radians of pseudo-angle `q`.
fn pseudo_to_radians(q: f64) -> f64 {
    let quadrant = q.floor();
    let f = q - quadrant;
    quadrant * std::f64::consts::FRAC_PI_2 + f.atan2(1.0 - f)
}

/// A periodic table of `K` values per hue, read with linear interpolation.
/// It is built from a function of the true hue angle but indexed by the
/// pseudo-angle, so a lookup needs no trigonometry.
#[derive(Clone)]
pub(crate) struct HueLut<const K: usize> {
    entries: Box<[[f32; K]]>,
}

/// Prints the table's shape, not its samples.
impl<const K: usize> std::fmt::Debug for HueLut<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HueLut")
            .field("cells", &HUE_CELLS)
            .field("values_per_hue", &K)
            .finish()
    }
}

impl<const K: usize> HueLut<K> {
    /// Sample `f(hue in radians, 0 to 2π)` once per cell.
    pub(crate) fn build(f: impl Fn(f64) -> [f32; K]) -> Self {
        let entries = (0..=HUE_CELLS)
            .map(|i| {
                f(pseudo_to_radians(
                    4.0 * (i % HUE_CELLS) as f64 / HUE_CELLS as f64,
                ))
            })
            .collect();
        Self { entries }
    }

    /// The table's values for the hue of `(a, b)`.
    #[inline(always)]
    pub(crate) fn eval(&self, a: f32, b: f32) -> [f32; K] {
        let p = pseudo_angle(a, b) * CELLS_PER_QUADRANT;
        let i = (p as usize).min(HUE_CELLS - 1);
        let f = p - i as f32;
        let (lo, hi) = (&self.entries[i], &self.entries[i + 1]);
        let mut out = [0.0; K];
        for ((value, lo), hi) in out.iter_mut().zip(lo).zip(hi) {
            *value = lo + (hi - lo) * f;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pseudo_angle_is_monotonic_and_inverts_to_the_true_angle() {
        let mut previous = -1.0;
        for step in 0..3600 {
            let angle = (step as f64 * 0.1).to_radians();
            let q = pseudo_angle(angle.cos() as f32, angle.sin() as f32);
            assert!((0.0..4.0).contains(&q), "{q}");
            assert!(
                q > previous,
                "not increasing at {} degrees",
                step as f64 * 0.1
            );
            previous = q;
            assert!((pseudo_to_radians(f64::from(q)) - angle).abs() < 1e-5);
        }
        assert_eq!(pseudo_angle(0.0, 0.0), 0.0);
        assert_eq!(pseudo_angle(f32::NAN, 1.0), 0.0);
    }

    #[test]
    fn a_hue_table_reproduces_a_smooth_function_of_hue() {
        let table = HueLut::<2>::build(|hue| [hue.cos() as f32, (2.0 * hue).sin() as f32]);
        for step in 0..720 {
            let angle = (step as f32 * 0.5).to_radians();
            let [c, s] = table.eval(0.3 * angle.cos(), 0.3 * angle.sin());
            assert!((c - angle.cos()).abs() < 3e-4 && (s - (2.0 * angle).sin()).abs() < 1e-3);
        }
    }

    #[test]
    fn the_fast_cube_root_matches_the_exact_one() {
        let mut worst = 0.0f64;
        for step in 0..=24_000 {
            // Twenty-four decades, a thousand samples each.
            let x = 10.0f64.powf(-12.0 + f64::from(step) / 1000.0) as f32;
            let error = (f64::from(cbrt(x)) / f64::from(x).cbrt() - 1.0).abs();
            worst = worst.max(error);
        }
        assert!(worst < 1.5e-7, "worst relative error {worst:e}");
        assert_eq!(cbrt(1.0), 1.0);
        assert_eq!(cbrt(8.0), 2.0);
        assert_eq!(cbrt(0.0), 0.0);
        assert_eq!(cbrt(-1.0), 0.0);
        assert_eq!(cbrt(f32::NAN), 0.0);
        assert!((cbrt(1.0e-29) - 1.0e-29f32.cbrt()).abs() < 1e-16);
        assert!(cbrt(f32::MAX).is_finite() && cbrt(f32::INFINITY).is_finite());
    }

    #[test]
    fn working_white_is_lightness_one() {
        let lab = working_to_oklab([1.0; 3]);
        assert!((lab[0] - 1.0).abs() < 1e-6 && lab[1].abs() < 1e-6 && lab[2].abs() < 1e-6);
        assert_eq!(hue_degrees(0.0, 0.0), 0.0);
        assert!((hue_degrees(0.0, -1.0) - 270.0).abs() < 1e-4);
    }
}
