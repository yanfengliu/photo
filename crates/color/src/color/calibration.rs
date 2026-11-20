//! Calibration: move the working primaries and tint the shadows.
//!
//! The three primary controls build a matrix whose columns are the altered
//! primaries, rescaled so that white stays white. This is the construction
//! of an RGB-to-RGB matrix from new primaries and a fixed white point, done
//! directly in working RGB.

use photo_core::colorimetry::{IDENTITY, Mat3, WORKING_LUMA, luminance, mat3_apply};
use photo_core::develop::Calibration;

use crate::math::{
    Mat3d, apply_d, invert_d, narrow, non_negative, normalize_rows, slider_unit, toward_luminance,
};

// ---- Tunable constants ----------------------------------------------------

/// Hue ±100 mixes this much of the neighbouring primary into a primary:
/// positive turns red toward orange, green toward aqua, blue toward purple.
const HUE_RANGE: f64 = 0.25;
/// Saturation ±100 moves a primary this far from (or toward) its own
/// luminance, as a fraction of its distance from grey.
const SATURATION_RANGE: f64 = 0.5;
/// Shadows tint ±100 lowers (magenta) or raises (green) the green channel by
/// this fraction in the deepest shadows; red and blue move the other way by
/// the amount that keeps a neutral's luminance unchanged.
const TINT_RANGE: f32 = 0.15;
/// The tint fades out as `(1 - sqrt(Y) / TINT_REACH)^2` and is gone above
/// `Y = TINT_REACH^2` (0.36).
const TINT_REACH: f32 = 0.6;

// ---------------------------------------------------------------------------

/// The matrix on linear working RGB that the three primary hue and
/// saturation pairs describe. Every row sums to one, so neutrals are
/// unchanged; with all six sliders at zero it is exactly the identity. The
/// result can have negative channels for saturated colours.
pub fn calibration_matrix(calibration: &Calibration) -> Mat3 {
    let controls = [
        (calibration.red_hue, calibration.red_saturation),
        (calibration.green_hue, calibration.green_saturation),
        (calibration.blue_hue, calibration.blue_saturation),
    ];
    if controls
        .iter()
        .all(|(hue, saturation)| *hue == 0.0 && *saturation == 0.0)
    {
        return IDENTITY;
    }
    let luma = WORKING_LUMA.map(f64::from);
    // Column `c` is where primary `c` goes.
    let mut columns = [[0.0f64; 3]; 3];
    for (channel, (hue, saturation)) in controls.into_iter().enumerate() {
        let mut primary = [0.0; 3];
        primary[channel] = 1.0;
        let turn = slider_unit(hue) * HUE_RANGE;
        let neighbour = if turn >= 0.0 {
            (channel + 1) % 3
        } else {
            (channel + 2) % 3
        };
        primary[neighbour] = turn.abs();
        let y: f64 = primary.iter().zip(luma).map(|(p, l)| p * l).sum();
        let stretch = 1.0 + slider_unit(saturation) * SATURATION_RANGE;
        columns[channel] = primary.map(|p| y + (p - y) * stretch);
    }
    let mut matrix: Mat3d = [[0.0; 3]; 3];
    for (row, values) in matrix.iter_mut().enumerate() {
        for (column, value) in values.iter_mut().enumerate() {
            *value = columns[column][row];
        }
    }
    // Scale each primary so that the three still add up to white.
    let Some(inverse) = invert_d(&matrix) else {
        return IDENTITY;
    };
    let scale = apply_d(&inverse, [1.0; 3]);
    if scale.iter().any(|k| !k.is_finite() || *k <= 0.0) {
        return IDENTITY;
    }
    for values in matrix.iter_mut() {
        for (value, k) in values.iter_mut().zip(scale) {
            *value *= k;
        }
    }
    normalize_rows(&mut matrix);
    narrow(&matrix)
}

/// Calibration compiled for per-pixel use.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct CalibrationOp {
    matrix: Option<Mat3>,
    /// Per-channel gain change at full tint weight; zero when unused.
    tint: [f32; 3],
}

impl CalibrationOp {
    /// `None` when every calibration slider is zero.
    pub(super) fn new(calibration: &Calibration) -> Option<Self> {
        let matrix = calibration_matrix(calibration);
        let amount = slider_unit(calibration.shadows_tint) as f32 * TINT_RANGE;
        if matrix == IDENTITY && amount == 0.0 {
            return None;
        }
        // Positive is magenta: green down, red and blue up, balanced so that
        // the luminance of a neutral does not change.
        let others = amount * WORKING_LUMA[1] / (WORKING_LUMA[0] + WORKING_LUMA[2]);
        Some(Self {
            matrix: (matrix != IDENTITY).then_some(matrix),
            tint: [others, -amount, others],
        })
    }

    /// Apply to a non-negative linear working RGB value; the result is
    /// non-negative.
    #[inline(always)]
    pub(super) fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let mut rgb = match &self.matrix {
            Some(matrix) => toward_luminance(mat3_apply(matrix, rgb), &WORKING_LUMA),
            None => rgb,
        };
        if self.tint[1] != 0.0 {
            let depth = 1.0 - luminance(rgb).max(0.0).sqrt() / TINT_REACH;
            if depth > 0.0 {
                let weight = depth * depth;
                rgb = non_negative([
                    rgb[0] * (1.0 + self.tint[0] * weight),
                    rgb[1] * (1.0 + self.tint[1] * weight),
                    rgb[2] * (1.0 + self.tint[2] * weight),
                ]);
            }
        }
        rgb
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_extreme_gives_a_usable_matrix_that_keeps_white() {
        for mask in 0..64u32 {
            let pick = |bit: u32| {
                if mask & (1 << bit) != 0 {
                    100.0
                } else {
                    -100.0
                }
            };
            let calibration = Calibration {
                shadows_tint: 0.0,
                red_hue: pick(0),
                red_saturation: pick(1),
                green_hue: pick(2),
                green_saturation: pick(3),
                blue_hue: pick(4),
                blue_saturation: pick(5),
            };
            let matrix = calibration_matrix(&calibration);
            assert_ne!(
                matrix, IDENTITY,
                "combination {mask:06b} fell back to the identity"
            );
            for row in matrix {
                assert!((row.iter().sum::<f32>() - 1.0).abs() < 1e-6);
                assert!(row.iter().all(|v| v.abs() < 3.0), "{matrix:?}");
            }
        }
    }

    #[test]
    fn a_primary_goes_where_its_sliders_point() {
        let orange_red = calibration_matrix(&Calibration {
            red_hue: 100.0,
            ..Default::default()
        });
        let red = mat3_apply(&orange_red, [1.0, 0.0, 0.0]);
        assert!(red[1] > 0.2 && red[2].abs() < 1e-6, "{red:?}");
        let op = CalibrationOp::new(&Calibration {
            shadows_tint: 50.0,
            ..Default::default()
        })
        .unwrap();
        assert!(op.matrix.is_none());
        assert!(CalibrationOp::new(&Calibration::default()).is_none());
    }
}
