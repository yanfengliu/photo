//! Small numeric helpers shared by the colour operators: `f64` 3x3 matrices
//! for deriving transforms once, and branch-light `[f32; 3]` helpers for the
//! per-pixel paths.

use photo_core::colorimetry::Mat3;

/// Row-major 3x3 matrix in `f64`, used while deriving matrices so that
/// products and inverses do not accumulate `f32` rounding.
pub(crate) type Mat3d = [[f64; 3]; 3];

pub(crate) const IDENTITY_D: Mat3d = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

pub(crate) fn widen(m: &Mat3) -> Mat3d {
    m.map(|row| row.map(f64::from))
}

pub(crate) fn narrow(m: &Mat3d) -> Mat3 {
    m.map(|row| row.map(|v| v as f32))
}

pub(crate) fn apply_d(m: &Mat3d, v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

pub(crate) fn mul_d(a: &Mat3d, b: &Mat3d) -> Mat3d {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, value) in row.iter_mut().enumerate() {
            *value = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

pub(crate) fn diag_d(v: [f64; 3]) -> Mat3d {
    [[v[0], 0.0, 0.0], [0.0, v[1], 0.0], [0.0, 0.0, v[2]]]
}

/// Inverse by the adjugate, or `None` when the matrix is singular to working
/// precision.
pub(crate) fn invert_d(m: &Mat3d) -> Option<Mat3d> {
    let cofactor =
        |r0: usize, r1: usize, c0: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
    let adjugate = [
        [
            cofactor(1, 2, 1, 2),
            -cofactor(0, 2, 1, 2),
            cofactor(0, 1, 1, 2),
        ],
        [
            -cofactor(1, 2, 0, 2),
            cofactor(0, 2, 0, 2),
            -cofactor(0, 1, 0, 2),
        ],
        [
            cofactor(1, 2, 0, 1),
            -cofactor(0, 2, 0, 1),
            cofactor(0, 1, 0, 1),
        ],
    ];
    let det = m[0][0] * adjugate[0][0] + m[0][1] * adjugate[1][0] + m[0][2] * adjugate[2][0];
    let scale = m.iter().flatten().fold(0.0f64, |acc, v| acc.max(v.abs()));
    if !det.is_finite() || det.abs() <= scale.powi(3) * 1e-12 {
        return None;
    }
    Some(adjugate.map(|row| row.map(|v| v / det)))
}

/// Scale every row so it sums to exactly one (in `f64`), which makes the
/// matrix map equal channels to equal channels: neutrals stay neutral.
pub(crate) fn normalize_rows(m: &mut Mat3d) {
    for row in m.iter_mut() {
        let sum: f64 = row.iter().sum();
        if sum.abs() > 1e-12 {
            for value in row.iter_mut() {
                *value /= sum;
            }
        }
    }
}

#[inline(always)]
pub(crate) fn max3(v: [f32; 3]) -> f32 {
    v[0].max(v[1]).max(v[2])
}

#[inline(always)]
pub(crate) fn min3(v: [f32; 3]) -> f32 {
    v[0].min(v[1]).min(v[2])
}

/// Clamp each channel at zero. `f32::max` ignores NaN, so this also turns a
/// NaN channel into zero. Non-negative input is returned bit for bit.
#[inline(always)]
pub(crate) fn non_negative(v: [f32; 3]) -> [f32; 3] {
    [v[0].max(0.0), v[1].max(0.0), v[2].max(0.0)]
}

#[inline(always)]
pub(crate) fn scale3(v: [f32; 3], k: f32) -> [f32; 3] {
    [v[0] * k, v[1] * k, v[2] * k]
}

#[inline(always)]
pub(crate) fn dot3(a: &[f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Hermite smoothstep of an already normalized parameter: 0 below 0, 1 above
/// 1, C1 in between with a maximum slope of 1.5.
#[inline(always)]
pub(crate) fn smoothstep(x: f32) -> f32 {
    let t = x.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// [`smoothstep`] in `f64`, for curves evaluated while building tables.
#[inline]
pub(crate) fn smoothstep_d(x: f64) -> f64 {
    let t = x.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A ±100 slider value as a fraction of its range, clamped to ±1. NaN and
/// infinities count as zero.
pub(crate) fn slider_unit(slider: f32) -> f64 {
    if slider.is_finite() {
        (f64::from(slider) / 100.0).clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

/// Bring a colour with a negative channel back into gamut by desaturating it
/// toward its own luminance until the lowest channel is zero. Hue and
/// luminance are kept, which a per-channel clamp would shift. Colours with no
/// negative channel are returned unchanged; a colour whose luminance is not
/// positive becomes black.
#[inline]
pub(crate) fn toward_luminance(rgb: [f32; 3], luma: &[f32; 3]) -> [f32; 3] {
    let lo = min3(rgb);
    if lo >= 0.0 {
        return rgb;
    }
    let y = dot3(luma, rgb);
    if y <= 0.0 || y.is_nan() || lo.is_nan() {
        return [0.0; 3];
    }
    let t = y / (y - lo);
    non_negative([
        y + (rgb[0] - y) * t,
        y + (rgb[1] - y) * t,
        y + (rgb[2] - y) * t,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invert_round_trips_and_rejects_singular_matrices() {
        let m = [[2.0, 0.5, 0.1], [0.2, 1.5, 0.3], [0.0, 0.4, 1.2]];
        let product = mul_d(&m, &invert_d(&m).unwrap());
        for (i, row) in product.iter().enumerate() {
            for (j, value) in row.iter().enumerate() {
                let expected = if i == j { 1.0 } else { 0.0 };
                assert!((value - expected).abs() < 1e-12);
            }
        }
        assert!(invert_d(&[[1.0, 2.0, 3.0], [2.0, 4.0, 6.0], [0.0, 1.0, 0.0]]).is_none());
    }

    #[test]
    fn toward_luminance_removes_the_negative_channel_and_keeps_luminance() {
        let luma = [0.25, 0.65, 0.10];
        let fixed = toward_luminance([0.8, 0.4, -0.2], &luma);
        assert!(min3(fixed) >= 0.0 && min3(fixed) < 1e-6);
        let before = dot3(&luma, [0.8, 0.4, -0.2]);
        assert!((dot3(&luma, fixed) - before).abs() < 1e-6);
        assert_eq!(toward_luminance([0.1, 0.2, 0.3], &luma), [0.1, 0.2, 0.3]);
        assert_eq!(toward_luminance([-1.0, 0.0, 0.0], &luma), [0.0; 3]);
    }

    #[test]
    fn non_negative_swallows_nan() {
        assert_eq!(non_negative([f32::NAN, -1.0, 2.0]), [0.0, 0.0, 2.0]);
    }
}
