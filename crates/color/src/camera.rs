//! Input transform for camera-native sources (DESIGN §5.3): white-balance
//! gains, highlight reconstruction and the camera-to-working matrix.
//!
//! The colour model is the single-illuminant case of the DNG specification's
//! "Mapping Camera Color Space to CIE XYZ Space": the file's D65
//! `ColorMatrix` `A` maps XYZ to camera-native RGB, so a light source of
//! chromaticity `xy` is recorded as `A · XYZ(xy)`, and the gains that
//! neutralise it are the reciprocals of that response (DNG SDK
//! `dng_color_spec::NeutralToXY` without the two-illuminant iteration).

use photo_core::colorimetry::{Chromaticity, IDENTITY, Mat3, mat3_apply, xyz_d65_to_working};
use photo_core::develop::{KELVIN_MAX, KELVIN_MIN, RAW_TINT_LIMIT, WbPreset, WhiteBalance};
use photo_core::image::CameraColor;

use crate::math::{
    Mat3d, apply_d, invert_d, max3, mul_d, narrow, non_negative, normalize_rows, smoothstep, widen,
};
use crate::temperature::{temp_tint_to_xy, xy_to_temp_tint};

// ---- Tunable constants ----------------------------------------------------

/// Highlight reconstruction starts when the largest un-balanced channel
/// passes this fraction of the sensor's clipping level...
const RECONSTRUCT_START: f32 = 0.90;
/// ...and is complete at the clipping level itself.
const RECONSTRUCT_END: f32 = 1.0;
/// White-balance gains are kept inside this range so a degenerate matrix or
/// an absurd illuminant cannot produce an unbounded multiplier.
const GAIN_RANGE: (f32, f32) = (0.02, 64.0);
/// Temperature and tint reported when the camera's matrix is unusable.
const FALLBACK_TEMP_TINT: (f32, f32) = (6500.0, 0.0);

// ---------------------------------------------------------------------------

/// The camera matrix and its inverse in `f64`, or `None` when the matrix is
/// singular or not finite.
fn model(camera: &CameraColor) -> Option<(Mat3d, Mat3d)> {
    let matrix = widen(&camera.xyz_to_camera);
    if matrix.iter().flatten().any(|v| !v.is_finite()) {
        return None;
    }
    invert_d(&matrix).map(|inverse| (matrix, inverse))
}

fn usable_gains(gains: [f32; 3]) -> [f32; 3] {
    gains.map(|g| {
        if g.is_finite() && g > 0.0 {
            g.clamp(GAIN_RANGE.0, GAIN_RANGE.1)
        } else {
            1.0
        }
    })
}

fn chromaticity_of(xyz: [f64; 3]) -> Option<Chromaticity> {
    let sum = xyz[0] + xyz[1] + xyz[2];
    let xy = Chromaticity::new(xyz[0] / sum, xyz[1] / sum);
    (sum.is_finite() && sum > 0.0 && xy.x > 0.0 && xy.y > 0.0 && xy.x + xy.y < 1.0).then_some(xy)
}

/// The chromaticity of the light source that the camera records as
/// `neutral` (camera-native RGB of something grey), or `None` when the
/// sample has a non-positive or non-finite channel or lies outside what the
/// camera's matrix can explain.
pub fn camera_neutral_to_xy(camera: &CameraColor, neutral: [f32; 3]) -> Option<Chromaticity> {
    if neutral.iter().any(|v| !v.is_finite() || *v <= 0.0) {
        return None;
    }
    let (_, inverse) = model(camera)?;
    chromaticity_of(apply_d(&inverse, neutral.map(f64::from)))
}

/// White-balance multipliers, green normalized to 1.0, that neutralise a
/// light source of the given temperature (kelvin) and tint (DNG scale):
/// `g = 1 / (A · XYZ(xy))`.
///
/// Temperature and tint are clamped to the slider ranges. When the numbers
/// are not finite, the temperature is not positive (a setting whose numbers
/// were never filled in), or the camera's matrix cannot be used, the file's
/// as-shot gains are returned instead.
pub fn camera_gains_for_temp_tint(camera: &CameraColor, kelvin: f32, tint: f32) -> [f32; 3] {
    let fallback = usable_gains(camera.as_shot_gains);
    if !kelvin.is_finite() || kelvin <= 0.0 || !tint.is_finite() {
        return fallback;
    }
    let Some((matrix, _)) = model(camera) else {
        return fallback;
    };
    let xy = temp_tint_to_xy(
        f64::from(kelvin.clamp(KELVIN_MIN, KELVIN_MAX)),
        f64::from(tint.clamp(-RAW_TINT_LIMIT, RAW_TINT_LIMIT)),
    );
    let response = apply_d(&matrix, xy.to_xyz());
    if response.iter().any(|v| !v.is_finite() || *v <= 0.0) {
        return fallback;
    }
    usable_gains([
        (response[1] / response[0]) as f32,
        1.0,
        (response[1] / response[2]) as f32,
    ])
}

/// The white-balance multipliers for a camera source: the file's own gains
/// for [`WbPreset::AsShot`] (its `temp` and `tint` are ignored), otherwise
/// the gains for the setting's temperature and tint.
pub fn camera_gains(camera: &CameraColor, white_balance: &WhiteBalance) -> [f32; 3] {
    match white_balance.preset {
        WbPreset::AsShot => usable_gains(camera.as_shot_gains),
        _ => camera_gains_for_temp_tint(camera, white_balance.temp, white_balance.tint),
    }
}

/// The as-shot white balance expressed as (kelvin, tint), for display and as
/// the starting point when the user leaves "As Shot". Clamped to the slider
/// ranges; feeding the result back through [`camera_gains_for_temp_tint`]
/// reproduces the as-shot gains to a fraction of a percent whenever no clamp
/// applied.
pub fn camera_as_shot_temp_tint(camera: &CameraColor) -> (f32, f32) {
    let neutral = usable_gains(camera.as_shot_gains).map(|g| 1.0 / g);
    camera_neutral_to_xy(camera, neutral).map_or(FALLBACK_TEMP_TINT, slider_temp_tint)
}

fn slider_temp_tint(xy: Chromaticity) -> (f32, f32) {
    let (kelvin, tint) = xy_to_temp_tint(xy);
    (
        (kelvin as f32).clamp(KELVIN_MIN, KELVIN_MAX),
        (tint as f32).clamp(-RAW_TINT_LIMIT, RAW_TINT_LIMIT),
    )
}

/// The matrix from white-balanced camera RGB to linear working RGB:
/// `M = XYZ→working · A'⁻¹` with `A' = diag(1 / (A · XYZ(D65))) · A`.
///
/// It expects white-balanced input and maps equal channels to equal channels
/// (every row sums to one), so balanced white `(1, 1, 1)` becomes working
/// white. A singular camera matrix yields the identity.
pub fn camera_to_working(camera: &CameraColor) -> Mat3 {
    let Some((matrix, inverse)) = model(camera) else {
        return IDENTITY;
    };
    let white = apply_d(&matrix, Chromaticity::D65.to_xyz());
    if white.iter().any(|v| !v.is_finite() || v.abs() < 1e-9) {
        return IDENTITY;
    }
    // A'⁻¹ = A⁻¹ · diag(white).
    let mut balanced_to_xyz = inverse;
    for row in balanced_to_xyz.iter_mut() {
        for (value, w) in row.iter_mut().zip(white) {
            *value *= w;
        }
    }
    let mut to_working = mul_d(&widen(&xyz_d65_to_working()), &balanced_to_xyz);
    normalize_rows(&mut to_working);
    narrow(&to_working)
}

/// Highlight reconstruction (DESIGN §5.3). A sensor clips each channel at
/// 1.0 before white balance, so after the gains a blown pixel is tinted.
/// With `c` the largest channel of `raw` (before gains), the balanced pixel
/// is blended toward its own largest channel, which is neutral, by
/// `smoothstep(0.90, 1.0, c)`.
///
/// Pixels with `c` at or below 0.90 are returned unchanged. Channels are only
/// ever raised and values above 1.0 are kept: that headroom is what the
/// Highlights slider recovers.
#[inline]
pub fn reconstruct_highlights(raw: [f32; 3], balanced: [f32; 3]) -> [f32; 3] {
    let clip = max3(raw);
    if clip <= RECONSTRUCT_START {
        return balanced;
    }
    let top = max3(balanced);
    if clip >= RECONSTRUCT_END {
        return [top; 3];
    }
    let t = smoothstep((clip - RECONSTRUCT_START) / (RECONSTRUCT_END - RECONSTRUCT_START));
    [
        balanced[0] + (top - balanced[0]) * t,
        balanced[1] + (top - balanced[1]) * t,
        balanced[2] + (top - balanced[2]) * t,
    ]
}

/// White-balance picker for camera sources: the custom white balance that
/// makes `sample` (camera-native, not yet white balanced) neutral. `None`
/// when the sample has a non-positive or non-finite channel. The result is
/// clamped to the slider ranges, so a sample far from any plausible light
/// source is only neutralised as far as the sliders reach.
pub fn pick_camera_white_balance(camera: &CameraColor, sample: [f32; 3]) -> Option<WhiteBalance> {
    let (temp, tint) = slider_temp_tint(camera_neutral_to_xy(camera, sample)?);
    Some(WhiteBalance {
        preset: WbPreset::Custom,
        temp,
        tint,
    })
}

/// The compiled input transform for one camera source and white balance:
/// gains, highlight reconstruction, camera-to-working matrix.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraInput {
    gains: [f32; 3],
    to_working: Mat3,
}

impl CameraInput {
    /// Resolve the gains for `white_balance` and derive the matrix.
    pub fn new(camera: &CameraColor, white_balance: &WhiteBalance) -> Self {
        Self {
            gains: camera_gains(camera, white_balance),
            to_working: camera_to_working(camera),
        }
    }

    /// The white-balance multipliers in use (green is 1.0).
    pub fn gains(&self) -> [f32; 3] {
        self.gains
    }

    /// Camera-native RGB to white-balanced linear working RGB. The result is
    /// never negative; values above 1.0 are kept.
    #[inline]
    pub fn apply(&self, raw: [f32; 3]) -> [f32; 3] {
        let raw = non_negative(raw);
        let balanced = [
            raw[0] * self.gains[0],
            raw[1] * self.gains[1],
            raw[2] * self.gains[2],
        ];
        non_negative(mat3_apply(
            &self.to_working,
            reconstruct_highlights(raw, balanced),
        ))
    }
}
