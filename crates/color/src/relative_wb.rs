//! Relative white balance in working space (DESIGN §5.3), used for raster
//! sources and for the temp/tint offsets of local masks.
//!
//! A raster file is already a rendering, so its white balance is an offset
//! from "as it is": `temp` and `tint` name the light source the picture is
//! assumed to have been balanced for, measured from D65, and the picture is
//! adapted from that light source back to D65 with the Bradford transform
//! (Lam 1985; the linear form used by ICC v4). The light source is located
//! with the DNG temperature model of [`crate::temperature`].
//!
//! Bradford is a von Kries scaling in a sharpened cone space, so in working
//! RGB it is `P⁻¹ · diag(gains) · P` with a fixed `P`. The gains are what
//! varies with temp and tint, which is what lets a mask change them per
//! pixel without building a matrix.

use std::sync::LazyLock;

use photo_core::colorimetry::{
    Chromaticity, IDENTITY, Mat3, RgbSpace, WORKING_SPACE, mat3_apply, rgb_to_rgb,
};
use photo_core::develop::{WbPreset, WhiteBalance};

use crate::math::{
    IDENTITY_D, Mat3d, apply_d, diag_d, invert_d, mul_d, narrow, non_negative, widen,
};
use crate::temperature::{temp_tint_to_xy, xy_to_temp_tint};

// ---- Tunable constants ----------------------------------------------------

/// The range of the raster temperature and tint sliders (both ±100).
pub const RASTER_TEMP_LIMIT: f32 = 100.0;
/// Mired moved per unit of `temp`: ±100 is ∓90 mired around D65.
const MIRED_PER_UNIT: f64 = 0.9;
/// DNG tint units per unit of `tint`: ±100 is ∓0.033 Δuv (= 99 / 3000).
const TINT_PER_UNIT: f64 = 0.99;
/// The implied light source is kept between 2000 K and 40000 K and within
/// ±150 DNG tint units however many mask offsets are stacked on top of the
/// global setting.
const MIRED_RANGE: (f64, f64) = (25.0, 500.0);
const TINT_RANGE: f64 = 150.0;
/// Bounds on a single cone gain.
const GAIN_RANGE: (f64, f64) = (0.05, 20.0);

// ---------------------------------------------------------------------------

/// The Bradford cone response matrix (XYZ to sharpened LMS).
const BRADFORD: Mat3d = [
    [0.8951, 0.2664, -0.1614],
    [-0.7502, 1.7135, 0.0367],
    [0.0389, -0.0685, 1.0296],
];

/// Everything that does not depend on the slider values.
struct Reference {
    /// D65 as the temperature model reads it.
    mired: f64,
    tint: f64,
    /// `D65 - temp_tint_to_xy(D65 as read)`: removes the model's round-trip
    /// residual so that zero offsets land on D65 exactly.
    residual: (f64, f64),
    /// Cone response to D65 white.
    white: [f64; 3],
    to_cone: Mat3d,
    from_cone: Mat3d,
    to_cone_f32: Mat3,
    from_cone_f32: Mat3,
    working_to_xyz: Mat3d,
}

static REFERENCE: LazyLock<Reference> = LazyLock::new(|| {
    let (kelvin, tint) = xy_to_temp_tint(Chromaticity::D65);
    let back = temp_tint_to_xy(kelvin, tint);
    let working_to_xyz = widen(&WORKING_SPACE.to_xyz());
    let to_cone = mul_d(&BRADFORD, &working_to_xyz);
    let from_cone = invert_d(&to_cone).unwrap_or(IDENTITY_D);
    Reference {
        mired: 1.0e6 / kelvin,
        tint,
        residual: (Chromaticity::D65.x - back.x, Chromaticity::D65.y - back.y),
        white: apply_d(&BRADFORD, Chromaticity::D65.to_xyz()),
        to_cone,
        from_cone,
        to_cone_f32: narrow(&to_cone),
        from_cone_f32: narrow(&from_cone),
        working_to_xyz,
    }
});

fn finite_or_zero(value: f32) -> f64 {
    if value.is_finite() {
        f64::from(value)
    } else {
        0.0
    }
}

/// The light source implied by relative `temp` and `tint`: D65 at (0, 0),
/// bluer for positive `temp` (so the correction warms the picture), greener
/// for positive `tint` (so the correction is magenta).
pub fn relative_illuminant(temp: f32, tint: f32) -> Chromaticity {
    let reference = &*REFERENCE;
    let mired = (reference.mired - MIRED_PER_UNIT * finite_or_zero(temp))
        .clamp(MIRED_RANGE.0, MIRED_RANGE.1);
    let dng_tint =
        (reference.tint + TINT_PER_UNIT * finite_or_zero(tint)).clamp(-TINT_RANGE, TINT_RANGE);
    let xy = temp_tint_to_xy(1.0e6 / mired, dng_tint);
    Chromaticity::new(xy.x + reference.residual.0, xy.y + reference.residual.1)
}

fn cone_gains(temp: f32, tint: f32) -> [f64; 3] {
    let reference = &*REFERENCE;
    let source = apply_d(&BRADFORD, relative_illuminant(temp, tint).to_xyz());
    let mut gains = [1.0; 3];
    for ((gain, white), source) in gains.iter_mut().zip(reference.white).zip(source) {
        if source.is_finite() && source > 1e-9 {
            *gain = (white / source).clamp(GAIN_RANGE.0, GAIN_RANGE.1);
        }
    }
    gains
}

/// The Bradford cone gains that adapt the light source implied by `temp` and
/// `tint` to D65. Exactly `[1, 1, 1]` at (0, 0). Apply with [`adapt`].
pub fn relative_wb_gains(temp: f32, tint: f32) -> [f32; 3] {
    if temp == 0.0 && tint == 0.0 {
        return [1.0; 3];
    }
    cone_gains(temp, tint).map(|g| g as f32)
}

/// The same adaptation as one 3x3 matrix on linear working RGB. Exactly the
/// identity at (0, 0). The matrix can return slightly negative channels for
/// saturated colours at tungsten-range settings; clamp its result at zero
/// (as [`adapt`] does).
pub fn relative_wb_matrix(temp: f32, tint: f32) -> Mat3 {
    if temp == 0.0 && tint == 0.0 {
        return IDENTITY;
    }
    let reference = &*REFERENCE;
    let scaled = mul_d(&diag_d(cone_gains(temp, tint)), &reference.to_cone);
    narrow(&mul_d(&reference.from_cone, &scaled))
}

/// Apply cone gains from [`relative_wb_gains`] to a linear working RGB
/// value. Gains of exactly `[1, 1, 1]` return the input unchanged (bit for
/// bit when it is non-negative). The result is clamped at zero: adapting a
/// saturated colour from a tungsten-range light source can push a channel
/// negative, which would break every later luminance-based stage.
#[inline]
pub fn adapt(rgb: [f32; 3], gains: [f32; 3]) -> [f32; 3] {
    if gains == [1.0; 3] {
        return non_negative(rgb);
    }
    let reference = &*REFERENCE;
    let cone = mat3_apply(&reference.to_cone_f32, rgb);
    non_negative(mat3_apply(
        &reference.from_cone_f32,
        [cone[0] * gains[0], cone[1] * gains[1], cone[2] * gains[2]],
    ))
}

/// White-balance picker for raster sources: the relative white balance that
/// makes `sample` (linear RGB in the file's own colour space) neutral.
/// `None` when the sample has a non-positive or non-finite channel. Both
/// numbers are clamped to ±[`RASTER_TEMP_LIMIT`], so a strongly coloured
/// sample is only neutralised as far as the sliders reach.
pub fn pick_raster_white_balance(space: RgbSpace, sample: [f32; 3]) -> Option<WhiteBalance> {
    if sample.iter().any(|v| !v.is_finite() || *v <= 0.0) {
        return None;
    }
    let reference = &*REFERENCE;
    let working = mat3_apply(&rgb_to_rgb(space, WORKING_SPACE), sample);
    let xyz = apply_d(&reference.working_to_xyz, working.map(f64::from));
    let sum = xyz[0] + xyz[1] + xyz[2];
    if !(sum.is_finite() && sum > 0.0) {
        return None;
    }
    // The sample is what the assumed light source looks like in the file, so
    // its own chromaticity is that light source.
    let xy = Chromaticity::new(
        xyz[0] / sum - reference.residual.0,
        xyz[1] / sum - reference.residual.1,
    );
    let (kelvin, dng_tint) = xy_to_temp_tint(xy);
    let limit = f64::from(RASTER_TEMP_LIMIT);
    let temp = ((reference.mired - 1.0e6 / kelvin) / MIRED_PER_UNIT).clamp(-limit, limit);
    let tint = ((dng_tint - reference.tint) / TINT_PER_UNIT).clamp(-limit, limit);
    Some(WhiteBalance {
        preset: WbPreset::Custom,
        temp: temp as f32,
        tint: tint as f32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_offsets_imply_d65() {
        let xy = relative_illuminant(0.0, 0.0);
        assert!((xy.x - Chromaticity::D65.x).abs() < 1e-12);
        assert!((xy.y - Chromaticity::D65.y).abs() < 1e-12);
        for gain in cone_gains(0.0, 0.0) {
            assert!((gain - 1.0).abs() < 1e-12);
        }
    }

    #[test]
    fn full_scale_temperature_moves_ninety_mired() {
        let (cool, _) = xy_to_temp_tint(relative_illuminant(100.0, 0.0));
        let (warm, _) = xy_to_temp_tint(relative_illuminant(-100.0, 0.0));
        let reference = REFERENCE.mired;
        assert!((reference - 1.0e6 / cool - 90.0).abs() < 0.5);
        assert!((1.0e6 / warm - reference - 90.0).abs() < 0.5);
    }

    #[test]
    fn the_cone_matrix_sends_working_white_to_the_d65_cone_response() {
        let white = apply_d(&REFERENCE.to_cone, [1.0; 3]);
        for (a, b) in white.iter().zip(REFERENCE.white) {
            assert!((a - b).abs() < 1e-6);
        }
    }
}
