//! The table of scalar develop adjustments.
//!
//! This is the single source of truth for every slider's key, label, range
//! and step. The UI builds sliders from it, history labels are formatted from
//! it, and XMP and the test harness address settings through its keys.

use super::color::ColorBand;
use super::groups::SettingsGroup;
use super::DevelopSettings;
use crate::meta::FileKind;

/// The static description of one adjustment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AdjSpec {
    /// Stable identifier (`snake_case`), unique across all adjustments.
    pub key: &'static str,
    /// Text shown next to the slider.
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
    /// Smallest increment a slider or arrow key applies.
    pub step: f32,
    /// Decimal places shown.
    pub decimals: u8,
    pub group: SettingsGroup,
}

impl AdjSpec {
    pub fn clamp(&self, value: f32) -> f32 {
        if value.is_finite() {
            value.clamp(self.min, self.max)
        } else {
            self.min.max(0.0).min(self.max)
        }
    }

    /// The value as shown beside a slider: signed when the range is signed.
    pub fn format(&self, value: f32) -> String {
        let decimals = usize::from(self.decimals);
        let shown = if value == 0.0 { 0.0 } else { value };
        if self.min < 0.0 && shown > 0.0 {
            format!("+{shown:.decimals$}")
        } else {
            format!("{shown:.decimals$}")
        }
    }
}

macro_rules! scalar_adjustments {
    ($(
        $variant:ident: $key:literal, $label:literal, $min:literal ..= $max:literal,
        step $step:literal, decimals $decimals:literal, $group:ident, $($field:ident).+;
    )*) => {
        /// One scalar develop adjustment.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Adj {
            $($variant,)*
            /// Colour mixer hue shift for one band.
            MixHue(ColorBand),
            /// Colour mixer saturation for one band.
            MixSaturation(ColorBand),
            /// Colour mixer luminance for one band.
            MixLuminance(ColorBand),
            /// Black-and-white mix weight for one band.
            MixBw(ColorBand),
        }

        impl Adj {
            /// Every adjustment that is not per-band.
            pub const SCALARS: &'static [Adj] = &[$(Adj::$variant,)*];

            /// The description of this adjustment for raster sources. Use
            /// [`Adj::spec_for`] when the source kind is known.
            pub fn spec(self) -> AdjSpec {
                match self {
                    $(Adj::$variant => AdjSpec {
                        key: $key,
                        label: $label,
                        min: $min,
                        max: $max,
                        step: $step,
                        decimals: $decimals,
                        group: SettingsGroup::$group,
                    },)*
                    Adj::MixHue(band) => band_spec(&HUE_KEYS, band),
                    Adj::MixSaturation(band) => band_spec(&SATURATION_KEYS, band),
                    Adj::MixLuminance(band) => band_spec(&LUMINANCE_KEYS, band),
                    Adj::MixBw(band) => band_spec(&BW_KEYS, band),
                }
            }

            /// Read this adjustment's value.
            pub fn get(self, settings: &DevelopSettings) -> f32 {
                match self {
                    $(Adj::$variant => settings.$($field).+,)*
                    Adj::MixHue(band) => settings.color_mixer.hue[band.index()],
                    Adj::MixSaturation(band) => settings.color_mixer.saturation[band.index()],
                    Adj::MixLuminance(band) => settings.color_mixer.luminance[band.index()],
                    Adj::MixBw(band) => settings.color_mixer.bw_mix[band.index()],
                }
            }

            /// Write this adjustment's value as given. Callers clamp with
            /// [`AdjSpec::clamp`]; setting temperature or tint does not by
            /// itself change the white balance preset.
            pub fn set(self, settings: &mut DevelopSettings, value: f32) {
                match self {
                    $(Adj::$variant => settings.$($field).+ = value,)*
                    Adj::MixHue(band) => settings.color_mixer.hue[band.index()] = value,
                    Adj::MixSaturation(band) => settings.color_mixer.saturation[band.index()] = value,
                    Adj::MixLuminance(band) => settings.color_mixer.luminance[band.index()] = value,
                    Adj::MixBw(band) => settings.color_mixer.bw_mix[band.index()] = value,
                }
            }
        }
    };
}

scalar_adjustments! {
    Temperature: "temperature", "Temp", -100.0..=100.0, step 1.0, decimals 0, WhiteBalance, white_balance.temp;
    Tint: "tint", "Tint", -100.0..=100.0, step 1.0, decimals 0, WhiteBalance, white_balance.tint;

    Exposure: "exposure", "Exposure", -5.0..=5.0, step 0.01, decimals 2, Tone, tone.exposure;
    Contrast: "contrast", "Contrast", -100.0..=100.0, step 1.0, decimals 0, Tone, tone.contrast;
    Highlights: "highlights", "Highlights", -100.0..=100.0, step 1.0, decimals 0, Tone, tone.highlights;
    Shadows: "shadows", "Shadows", -100.0..=100.0, step 1.0, decimals 0, Tone, tone.shadows;
    Whites: "whites", "Whites", -100.0..=100.0, step 1.0, decimals 0, Tone, tone.whites;
    Blacks: "blacks", "Blacks", -100.0..=100.0, step 1.0, decimals 0, Tone, tone.blacks;

    Texture: "texture", "Texture", -100.0..=100.0, step 1.0, decimals 0, Presence, presence.texture;
    Clarity: "clarity", "Clarity", -100.0..=100.0, step 1.0, decimals 0, Presence, presence.clarity;
    Dehaze: "dehaze", "Dehaze", -100.0..=100.0, step 1.0, decimals 0, Presence, presence.dehaze;
    Vibrance: "vibrance", "Vibrance", -100.0..=100.0, step 1.0, decimals 0, Presence, presence.vibrance;
    Saturation: "saturation", "Saturation", -100.0..=100.0, step 1.0, decimals 0, Presence, presence.saturation;

    CurveHighlights: "curve_highlights", "Highlights", -100.0..=100.0, step 1.0, decimals 0, ToneCurve, tone_curve.parametric.highlights;
    CurveLights: "curve_lights", "Lights", -100.0..=100.0, step 1.0, decimals 0, ToneCurve, tone_curve.parametric.lights;
    CurveDarks: "curve_darks", "Darks", -100.0..=100.0, step 1.0, decimals 0, ToneCurve, tone_curve.parametric.darks;
    CurveShadows: "curve_shadows", "Shadows", -100.0..=100.0, step 1.0, decimals 0, ToneCurve, tone_curve.parametric.shadows;
    CurveSplitShadows: "curve_split_shadows", "Shadow Split", 0.0..=100.0, step 1.0, decimals 0, ToneCurve, tone_curve.parametric.split_shadows;
    CurveSplitMidtones: "curve_split_midtones", "Midtone Split", 0.0..=100.0, step 1.0, decimals 0, ToneCurve, tone_curve.parametric.split_midtones;
    CurveSplitHighlights: "curve_split_highlights", "Highlight Split", 0.0..=100.0, step 1.0, decimals 0, ToneCurve, tone_curve.parametric.split_highlights;

    GradeShadowsHue: "grade_shadows_hue", "Hue", 0.0..=360.0, step 1.0, decimals 0, ColorGrading, color_grading.shadows.hue;
    GradeShadowsSaturation: "grade_shadows_saturation", "Saturation", 0.0..=100.0, step 1.0, decimals 0, ColorGrading, color_grading.shadows.saturation;
    GradeShadowsLuminance: "grade_shadows_luminance", "Luminance", -100.0..=100.0, step 1.0, decimals 0, ColorGrading, color_grading.shadows.luminance;
    GradeMidtonesHue: "grade_midtones_hue", "Hue", 0.0..=360.0, step 1.0, decimals 0, ColorGrading, color_grading.midtones.hue;
    GradeMidtonesSaturation: "grade_midtones_saturation", "Saturation", 0.0..=100.0, step 1.0, decimals 0, ColorGrading, color_grading.midtones.saturation;
    GradeMidtonesLuminance: "grade_midtones_luminance", "Luminance", -100.0..=100.0, step 1.0, decimals 0, ColorGrading, color_grading.midtones.luminance;
    GradeHighlightsHue: "grade_highlights_hue", "Hue", 0.0..=360.0, step 1.0, decimals 0, ColorGrading, color_grading.highlights.hue;
    GradeHighlightsSaturation: "grade_highlights_saturation", "Saturation", 0.0..=100.0, step 1.0, decimals 0, ColorGrading, color_grading.highlights.saturation;
    GradeHighlightsLuminance: "grade_highlights_luminance", "Luminance", -100.0..=100.0, step 1.0, decimals 0, ColorGrading, color_grading.highlights.luminance;
    GradeGlobalHue: "grade_global_hue", "Hue", 0.0..=360.0, step 1.0, decimals 0, ColorGrading, color_grading.global.hue;
    GradeGlobalSaturation: "grade_global_saturation", "Saturation", 0.0..=100.0, step 1.0, decimals 0, ColorGrading, color_grading.global.saturation;
    GradeGlobalLuminance: "grade_global_luminance", "Luminance", -100.0..=100.0, step 1.0, decimals 0, ColorGrading, color_grading.global.luminance;
    GradeBlending: "grade_blending", "Blending", 0.0..=100.0, step 1.0, decimals 0, ColorGrading, color_grading.blending;
    GradeBalance: "grade_balance", "Balance", -100.0..=100.0, step 1.0, decimals 0, ColorGrading, color_grading.balance;

    SharpenAmount: "sharpen_amount", "Amount", 0.0..=150.0, step 1.0, decimals 0, Sharpening, detail.sharpening.amount;
    SharpenRadius: "sharpen_radius", "Radius", 0.5..=3.0, step 0.1, decimals 1, Sharpening, detail.sharpening.radius;
    SharpenDetail: "sharpen_detail", "Detail", 0.0..=100.0, step 1.0, decimals 0, Sharpening, detail.sharpening.detail;
    SharpenMasking: "sharpen_masking", "Masking", 0.0..=100.0, step 1.0, decimals 0, Sharpening, detail.sharpening.masking;

    NoiseLuminance: "noise_luminance", "Luminance", 0.0..=100.0, step 1.0, decimals 0, NoiseReduction, detail.noise_reduction.luminance;
    NoiseLuminanceDetail: "noise_luminance_detail", "Detail", 0.0..=100.0, step 1.0, decimals 0, NoiseReduction, detail.noise_reduction.luminance_detail;
    NoiseLuminanceContrast: "noise_luminance_contrast", "Contrast", 0.0..=100.0, step 1.0, decimals 0, NoiseReduction, detail.noise_reduction.luminance_contrast;
    NoiseColor: "noise_color", "Color", 0.0..=100.0, step 1.0, decimals 0, NoiseReduction, detail.noise_reduction.color;
    NoiseColorDetail: "noise_color_detail", "Detail", 0.0..=100.0, step 1.0, decimals 0, NoiseReduction, detail.noise_reduction.color_detail;
    NoiseColorSmoothness: "noise_color_smoothness", "Smoothness", 0.0..=100.0, step 1.0, decimals 0, NoiseReduction, detail.noise_reduction.color_smoothness;

    LensDistortion: "lens_distortion", "Distortion", -100.0..=100.0, step 1.0, decimals 0, LensCorrections, lens.distortion;
    LensVignetting: "lens_vignetting", "Vignetting", -100.0..=100.0, step 1.0, decimals 0, LensCorrections, lens.vignetting;
    LensVignettingMidpoint: "lens_vignetting_midpoint", "Midpoint", 0.0..=100.0, step 1.0, decimals 0, LensCorrections, lens.vignetting_midpoint;

    TransformVertical: "transform_vertical", "Vertical", -100.0..=100.0, step 1.0, decimals 0, Transform, transform.vertical;
    TransformHorizontal: "transform_horizontal", "Horizontal", -100.0..=100.0, step 1.0, decimals 0, Transform, transform.horizontal;
    TransformRotate: "transform_rotate", "Rotate", -10.0..=10.0, step 0.1, decimals 1, Transform, transform.rotate;
    TransformAspect: "transform_aspect", "Aspect", -100.0..=100.0, step 1.0, decimals 0, Transform, transform.aspect;
    TransformScale: "transform_scale", "Scale", 50.0..=150.0, step 1.0, decimals 0, Transform, transform.scale;
    TransformXOffset: "transform_x_offset", "X Offset", -100.0..=100.0, step 0.1, decimals 1, Transform, transform.x_offset;
    TransformYOffset: "transform_y_offset", "Y Offset", -100.0..=100.0, step 0.1, decimals 1, Transform, transform.y_offset;

    VignetteAmount: "vignette_amount", "Amount", -100.0..=100.0, step 1.0, decimals 0, Vignette, effects.vignette.amount;
    VignetteMidpoint: "vignette_midpoint", "Midpoint", 0.0..=100.0, step 1.0, decimals 0, Vignette, effects.vignette.midpoint;
    VignetteRoundness: "vignette_roundness", "Roundness", -100.0..=100.0, step 1.0, decimals 0, Vignette, effects.vignette.roundness;
    VignetteFeather: "vignette_feather", "Feather", 0.0..=100.0, step 1.0, decimals 0, Vignette, effects.vignette.feather;
    VignetteHighlights: "vignette_highlights", "Highlights", 0.0..=100.0, step 1.0, decimals 0, Vignette, effects.vignette.highlights;

    GrainAmount: "grain_amount", "Amount", 0.0..=100.0, step 1.0, decimals 0, Grain, effects.grain.amount;
    GrainSize: "grain_size", "Size", 0.0..=100.0, step 1.0, decimals 0, Grain, effects.grain.size;
    GrainRoughness: "grain_roughness", "Roughness", 0.0..=100.0, step 1.0, decimals 0, Grain, effects.grain.roughness;

    CalibrationShadowsTint: "calibration_shadows_tint", "Shadows Tint", -100.0..=100.0, step 1.0, decimals 0, Calibration, calibration.shadows_tint;
    CalibrationRedHue: "calibration_red_hue", "Red Hue", -100.0..=100.0, step 1.0, decimals 0, Calibration, calibration.red_hue;
    CalibrationRedSaturation: "calibration_red_saturation", "Red Saturation", -100.0..=100.0, step 1.0, decimals 0, Calibration, calibration.red_saturation;
    CalibrationGreenHue: "calibration_green_hue", "Green Hue", -100.0..=100.0, step 1.0, decimals 0, Calibration, calibration.green_hue;
    CalibrationGreenSaturation: "calibration_green_saturation", "Green Saturation", -100.0..=100.0, step 1.0, decimals 0, Calibration, calibration.green_saturation;
    CalibrationBlueHue: "calibration_blue_hue", "Blue Hue", -100.0..=100.0, step 1.0, decimals 0, Calibration, calibration.blue_hue;
    CalibrationBlueSaturation: "calibration_blue_saturation", "Blue Saturation", -100.0..=100.0, step 1.0, decimals 0, Calibration, calibration.blue_saturation;

    CropAngle: "crop_angle", "Angle", -45.0..=45.0, step 0.01, decimals 2, Crop, crop.angle;
}

const HUE_KEYS: [&str; ColorBand::COUNT] = [
    "hue_red",
    "hue_orange",
    "hue_yellow",
    "hue_green",
    "hue_aqua",
    "hue_blue",
    "hue_purple",
    "hue_magenta",
];
const SATURATION_KEYS: [&str; ColorBand::COUNT] = [
    "saturation_red",
    "saturation_orange",
    "saturation_yellow",
    "saturation_green",
    "saturation_aqua",
    "saturation_blue",
    "saturation_purple",
    "saturation_magenta",
];
const LUMINANCE_KEYS: [&str; ColorBand::COUNT] = [
    "luminance_red",
    "luminance_orange",
    "luminance_yellow",
    "luminance_green",
    "luminance_aqua",
    "luminance_blue",
    "luminance_purple",
    "luminance_magenta",
];
const BW_KEYS: [&str; ColorBand::COUNT] = [
    "bw_red",
    "bw_orange",
    "bw_yellow",
    "bw_green",
    "bw_aqua",
    "bw_blue",
    "bw_purple",
    "bw_magenta",
];

fn band_spec(keys: &[&'static str; ColorBand::COUNT], band: ColorBand) -> AdjSpec {
    AdjSpec {
        key: keys[band.index()],
        label: band.label(),
        min: -100.0,
        max: 100.0,
        step: 1.0,
        decimals: 0,
        group: SettingsGroup::ColorMixer,
    }
}

/// Kelvin range of the temperature slider for camera RAW sources.
pub const KELVIN_MIN: f32 = 2000.0;
pub const KELVIN_MAX: f32 = 50_000.0;
/// Tint range (DNG scale) for camera RAW sources.
pub const RAW_TINT_LIMIT: f32 = 150.0;

impl Adj {
    /// Every adjustment: the scalars, then the four per-band families.
    pub fn all() -> impl Iterator<Item = Adj> {
        let families: [fn(ColorBand) -> Adj; 4] = [
            Adj::MixHue,
            Adj::MixSaturation,
            Adj::MixLuminance,
            Adj::MixBw,
        ];
        Self::SCALARS.iter().copied().chain(
            families
                .into_iter()
                .flat_map(|family| ColorBand::ALL.into_iter().map(family)),
        )
    }

    /// The description of this adjustment for a source of the given kind.
    /// Only white balance differs: camera RAW temperature is in kelvin.
    pub fn spec_for(self, kind: FileKind) -> AdjSpec {
        let spec = self.spec();
        match (self, kind) {
            (Adj::Temperature, FileKind::Raw) => AdjSpec {
                min: KELVIN_MIN,
                max: KELVIN_MAX,
                step: 50.0,
                ..spec
            },
            (Adj::Tint, FileKind::Raw) => AdjSpec {
                min: -RAW_TINT_LIMIT,
                max: RAW_TINT_LIMIT,
                ..spec
            },
            _ => spec,
        }
    }

    /// The value a freshly imported photo of this kind starts with.
    pub fn default_for(self, kind: FileKind) -> f32 {
        self.get(&DevelopSettings::default_for(kind))
    }

    /// Look an adjustment up by its [`AdjSpec::key`].
    pub fn from_key(key: &str) -> Option<Adj> {
        Self::all().find(|adj| adj.spec().key == key)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn keys_are_unique_and_resolve_back_to_their_adjustment() {
        let mut seen = HashSet::new();
        for adj in Adj::all() {
            let key = adj.spec().key;
            assert!(seen.insert(key), "duplicate key {key}");
            assert_eq!(Adj::from_key(key), Some(adj));
        }
        assert_eq!(seen.len(), Adj::SCALARS.len() + 4 * ColorBand::COUNT);
        assert_eq!(Adj::from_key("no_such_adjustment"), None);
    }

    #[test]
    fn every_default_lies_within_its_range_for_both_source_kinds() {
        for kind in [FileKind::Raster, FileKind::Raw] {
            for adj in Adj::all() {
                // As-shot white balance leaves temp/tint unset (zero), which
                // is outside the kelvin range by design.
                if kind == FileKind::Raw && adj == Adj::Temperature {
                    continue;
                }
                let spec = adj.spec_for(kind);
                let default = adj.default_for(kind);
                assert!(spec.min < spec.max, "{adj:?}");
                assert!(spec.step > 0.0, "{adj:?}");
                assert!(
                    (spec.min..=spec.max).contains(&default),
                    "{adj:?} default {default} outside {}..={}",
                    spec.min,
                    spec.max
                );
            }
        }
    }

    #[test]
    fn set_then_get_round_trips_without_touching_other_adjustments() {
        let all: Vec<Adj> = Adj::all().collect();
        for (index, adj) in all.iter().enumerate() {
            let mut settings = DevelopSettings::default();
            let before: Vec<f32> = all.iter().map(|other| other.get(&settings)).collect();
            let spec = adj.spec();
            let value = spec.min + (spec.max - spec.min) * 0.75;
            adj.set(&mut settings, value);
            for (other_index, other) in all.iter().enumerate() {
                let expected = if other_index == index { value } else { before[other_index] };
                assert_eq!(other.get(&settings), expected, "{adj:?} affected {other:?}");
            }
        }
    }

    #[test]
    fn raw_sources_measure_temperature_in_kelvin() {
        let raw = Adj::Temperature.spec_for(FileKind::Raw);
        assert_eq!((raw.min, raw.max), (KELVIN_MIN, KELVIN_MAX));
        let raster = Adj::Temperature.spec_for(FileKind::Raster);
        assert_eq!((raster.min, raster.max), (-100.0, 100.0));
        assert_eq!(Adj::Exposure.spec_for(FileKind::Raw), Adj::Exposure.spec());
    }

    #[test]
    fn values_format_with_a_sign_only_on_signed_ranges() {
        assert_eq!(Adj::Exposure.spec().format(0.35), "+0.35");
        assert_eq!(Adj::Contrast.spec().format(-12.0), "-12");
        assert_eq!(Adj::Contrast.spec().format(0.0), "0");
        assert_eq!(Adj::Contrast.spec().format(-0.0), "0");
        assert_eq!(Adj::SharpenAmount.spec().format(40.0), "40");
        assert_eq!(Adj::Temperature.spec_for(FileKind::Raw).format(5600.0), "5600");
    }

    #[test]
    fn clamp_keeps_values_in_range_and_tames_non_finite_input() {
        let spec = Adj::Exposure.spec();
        assert_eq!(spec.clamp(9.0), 5.0);
        assert_eq!(spec.clamp(-9.0), -5.0);
        assert_eq!(spec.clamp(f32::NAN), 0.0);
        assert_eq!(Adj::SharpenRadius.spec().clamp(f32::INFINITY), 0.5);
    }
}
