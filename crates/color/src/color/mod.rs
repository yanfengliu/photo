//! The colour stage (DESIGN §5.6, pipeline stage 7): calibration, the colour
//! mixer or the black-and-white mix, vibrance, saturation and colour
//! grading.
//!
//! Calibration works on linear working RGB. Everything else is done in
//! Oklab, with one conversion each way per pixel.

mod calibration;
mod grading;
mod mixer;

use photo_core::colorimetry::WORKING_LUMA;
use photo_core::develop::{DevelopSettings, Treatment};

use crate::math::{non_negative, slider_unit, toward_luminance};
use crate::oklab::{HueLut, oklab_to_working, working_to_oklab};

pub use calibration::calibration_matrix;
pub use grading::grading_weights;
pub use mixer::{band_centre_degrees, band_weights};

// ---- Tunable constants ----------------------------------------------------

/// Vibrance scales chroma by `1 + v * K² / (C² + K²)` with `K =
/// VIBRANCE_KNEE` (times the skin-protection weight): a colour whose Oklab
/// chroma equals the knee gets half the change a near-neutral gets. For `v`
/// in [-1, 1] the resulting chroma is still increasing in the original
/// chroma (its slope is at least 0.875 for a boost and 0 for a cut, reached
/// only at grey).
const VIBRANCE_KNEE: f32 = 0.12;
/// Vibrance is held back around skin tones: by `SKIN_PROTECTION` at an
/// Oklab hue of 55 degrees, fading as `cos⁸` of the hue distance (half
/// strength about 24 degrees away, nothing beyond 90).
const SKIN_PROTECTION: f32 = 0.6;
/// The unit `(a, b)` direction of that hue: (cos 55°, sin 55°). To move the
/// protected hue, change both numbers (a test pins them to the angle).
const SKIN_DIRECTION: (f32, f32) = (0.573_576_45, 0.819_152_05);
/// Saturation (global plus local) is clamped to this many slider units, so
/// stacked masks cannot scale chroma without bound.
const SATURATION_LIMIT: f32 = 300.0;
/// Added to squared chroma before dividing by it, so that a neutral has a
/// defined (and irrelevant) hue.
const CHROMA_FLOOR: f32 = 1.0e-12;

// ---------------------------------------------------------------------------

/// Stage 7 compiled for one set of settings: display-linear working RGB in,
/// display-linear working RGB out.
#[derive(Clone, Debug)]
pub struct ColorStage {
    calibration: Option<calibration::CalibrationOp>,
    /// The colour mixer or the black-and-white mix by hue; `None` when the
    /// one that applies to the treatment is neutral.
    hue: Option<HueLut<3>>,
    black_and_white: bool,
    /// Vibrance as a fraction, zero in black and white (where it could not
    /// do anything).
    vibrance: f32,
    saturation: f32,
    grading: Option<grading::Grading>,
}

impl ColorStage {
    /// Compile the treatment, colour mixer, vibrance, saturation, colour
    /// grading and calibration settings.
    pub fn new(settings: &DevelopSettings) -> Self {
        let black_and_white = settings.treatment == Treatment::BlackAndWhite;
        let mixer = &settings.color_mixer;
        let hue = if black_and_white {
            mixer
                .bw_mix
                .iter()
                .any(|v| *v != 0.0)
                .then(|| mixer::black_and_white_table(mixer))
        } else {
            (!mixer.hsl_is_identity()).then(|| mixer::colour_table(mixer))
        };
        let saturation = settings.presence.saturation;
        Self {
            calibration: calibration::CalibrationOp::new(&settings.calibration),
            hue,
            black_and_white,
            vibrance: if black_and_white {
                0.0
            } else {
                slider_unit(settings.presence.vibrance) as f32
            },
            saturation: if saturation.is_finite() {
                saturation
            } else {
                0.0
            },
            grading: grading::Grading::new(&settings.color_grading),
        }
    }

    /// True when [`Self::apply`] returns every non-negative input bit for
    /// bit, so the stage can be skipped.
    pub fn is_identity(&self) -> bool {
        self.calibration.is_none() && !self.needs_oklab(self.saturation)
    }

    /// True when anything after calibration is active for this saturation.
    #[inline(always)]
    fn needs_oklab(&self, saturation: f32) -> bool {
        self.black_and_white
            || self.hue.is_some()
            || self.vibrance != 0.0
            || saturation != 0.0
            || self.grading.is_some()
    }

    /// Apply the stage to one pixel. The result is finite and non-negative;
    /// colours pushed outside the working gamut are desaturated toward
    /// their own luminance. Neutral input stays neutral unless grading or
    /// the calibration shadow tint is in use, saturation -100 gives exactly
    /// equal channels, and so does black and white without grading.
    #[inline]
    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        self.render(rgb, self.saturation)
    }

    /// Apply the stage with `saturation_delta` slider units added to the
    /// global Saturation (the alpha-weighted sum of mask saturations for
    /// this pixel). A delta of zero is exactly [`Self::apply`].
    #[inline]
    pub fn apply_with_saturation(&self, rgb: [f32; 3], saturation_delta: f32) -> [f32; 3] {
        if saturation_delta == 0.0 || !saturation_delta.is_finite() {
            return self.render(rgb, self.saturation);
        }
        self.render(rgb, self.saturation + saturation_delta)
    }

    #[inline(always)]
    fn render(&self, rgb: [f32; 3], saturation: f32) -> [f32; 3] {
        let mut rgb = non_negative(rgb);
        if let Some(calibration) = &self.calibration {
            rgb = calibration.apply(rgb);
        }
        if !self.needs_oklab(saturation) {
            return rgb;
        }

        let [mut lightness, mut a, mut b] = working_to_oklab(rgb);
        if let Some(table) = &self.hue {
            // The mixer: lightness by hue (fading out toward grey), then the
            // hue turn and saturation as one rotation-and-scale of (a, b).
            const KNEE2: f32 = mixer::CHROMA_KNEE * mixer::CHROMA_KNEE;
            let chroma2 = a * a + b * b;
            let [turn_c, turn_s, lighten] = table.eval(a, b);
            lightness *= 1.0 + lighten * chroma2 / (chroma2 + KNEE2);
            (a, b) = (a * turn_c - b * turn_s, a * turn_s + b * turn_c);
        }
        if self.black_and_white {
            (a, b) = (0.0, 0.0);
        } else {
            let mut scale =
                (1.0 + saturation.clamp(-SATURATION_LIMIT, SATURATION_LIMIT) / 100.0).max(0.0);
            if self.vibrance != 0.0 {
                scale *= self.vibrance_scale(a, b);
            }
            (a, b) = (a * scale, b * scale);
        }
        let mut lab = [lightness, a, b];
        if let Some(grading) = &self.grading {
            lab = grading.apply(lab);
        }
        if lab[1] == 0.0 && lab[2] == 0.0 {
            // A neutral result is built directly so its channels are equal
            // to the bit, which a matrix round trip would not guarantee.
            let grey = lab[0].max(0.0);
            return [grey * grey * grey; 3];
        }
        toward_luminance(oklab_to_working(lab), &WORKING_LUMA)
    }

    /// The factor vibrance applies to the chroma of `(a, b)`: never
    /// negative, one for a fully saturated colour's limit, `1 + vibrance`
    /// for a near-neutral away from skin hues.
    #[inline(always)]
    fn vibrance_scale(&self, a: f32, b: f32) -> f32 {
        const KNEE2: f32 = VIBRANCE_KNEE * VIBRANCE_KNEE;
        let chroma2 = a * a + b * b;
        let (floored, over_knee) = (chroma2 + CHROMA_FLOOR, chroma2 + KNEE2);
        // One division serves both the hue distance and the chroma falloff.
        let shared = 1.0 / (floored * over_knee);
        let toward_skin = (a * SKIN_DIRECTION.0 + b * SKIN_DIRECTION.1).max(0.0);
        let cos2 = toward_skin * toward_skin * over_knee * shared;
        let cos4 = cos2 * cos2;
        let protect = 1.0 - SKIN_PROTECTION * cos4 * cos4;
        1.0 + self.vibrance * protect * KNEE2 * floored * shared
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hue_table_is_only_built_for_an_active_mixer() {
        let mut settings = DevelopSettings::default();
        settings.presence.saturation = 30.0;
        settings.presence.vibrance = 10.0;
        assert!(ColorStage::new(&settings).hue.is_none());
        settings.color_mixer.luminance[4] = 5.0;
        assert!(ColorStage::new(&settings).hue.is_some());
        settings.treatment = Treatment::BlackAndWhite;
        let stage = ColorStage::new(&settings);
        assert!(
            stage.hue.is_none() && stage.vibrance == 0.0,
            "the colour controls mean nothing in black and white"
        );
        settings.color_mixer.bw_mix[2] = 5.0;
        assert!(ColorStage::new(&settings).hue.is_some());
    }

    #[test]
    fn calibration_alone_skips_the_oklab_round_trip() {
        let mut settings = DevelopSettings::default();
        settings.calibration.shadows_tint = 40.0;
        let stage = ColorStage::new(&settings);
        // Above the tint's reach nothing at all happens to the pixel.
        assert_eq!(stage.apply([0.9, 0.5, 0.7]), [0.9, 0.5, 0.7]);
        assert!(!stage.is_identity());
    }

    #[test]
    fn vibrance_is_held_back_only_around_skin_hues() {
        const SKIN_HUE_DEGREES: f32 = 55.0;
        let (sin, cos) = SKIN_HUE_DEGREES.to_radians().sin_cos();
        assert!((SKIN_DIRECTION.0 - cos).abs() < 1e-6 && (SKIN_DIRECTION.1 - sin).abs() < 1e-6);
        let mut settings = DevelopSettings::default();
        settings.presence.vibrance = 100.0;
        let stage = ColorStage::new(&settings);
        // A weak colour at a given hue: the chroma falloff is negligible.
        let scale = |degrees: f32| {
            let (sin, cos) = degrees.to_radians().sin_cos();
            stage.vibrance_scale(1e-3 * cos, 1e-3 * sin)
        };
        assert!((scale(SKIN_HUE_DEGREES) - (2.0 - SKIN_PROTECTION)).abs() < 1e-3);
        assert!(scale(SKIN_HUE_DEGREES + 24.0) > scale(SKIN_HUE_DEGREES) + 0.25);
        for away in [150.0, 235.0, 325.0] {
            assert!(
                (scale(away) - 2.0).abs() < 1e-3,
                "hue {away}: {}",
                scale(away)
            );
        }
        // Smooth across the edge of the protected half-plane.
        assert!((scale(SKIN_HUE_DEGREES + 89.9) - scale(SKIN_HUE_DEGREES + 90.1)).abs() < 1e-4);
        // A neutral has no hue; the scale stays finite and ordinary.
        assert!((stage.vibrance_scale(0.0, 0.0) - 2.0).abs() < 1e-6);
        // Saturated colours are boosted less, and a full cut never inverts.
        assert!(stage.vibrance_scale(-0.3, 0.0) < 1.2);
        settings.presence.vibrance = -100.0;
        let cut = ColorStage::new(&settings);
        assert!(cut.vibrance_scale(-1e-4, 0.0) >= 0.0 && cut.vibrance_scale(-1e-4, 0.0) < 1e-3);
    }
}
