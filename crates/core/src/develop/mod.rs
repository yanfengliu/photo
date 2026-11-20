//! Parametric develop settings: the complete, serializable description of how
//! a photo is rendered. The original file is never modified; this is the edit.
//!
//! Slider ranges follow the conventions photographers already know from other
//! raw developers. What each number *does* to pixels is defined by the
//! `photo-color` and `photo-develop` crates, not here.

mod adjust;
mod basic;
mod color;
mod curve;
mod describe;
mod detail;
mod geometry;
mod groups;
mod local;

use serde::{Deserialize, Serialize};

use crate::meta::FileKind;

pub use adjust::{Adj, AdjSpec, KELVIN_MAX, KELVIN_MIN, RAW_TINT_LIMIT};
pub use basic::{Presence, Tone, WbPreset, WhiteBalance};
pub use color::{Calibration, ColorBand, ColorGrading, ColorMixer, GradeWheel};
pub use curve::{CurvePoint, ParametricCurve, PointCurve, ToneCurve};
pub use describe::describe_change;
pub use detail::{Detail, Effects, Grain, NoiseReduction, Sharpening, Vignette};
pub use geometry::{Crop, CropConstraint, CropRect, LensCorrections, Orientation, Transform};
pub use groups::{GroupSet, SettingsGroup};
pub use local::{BrushStroke, LocalAdjustments, Mask, MaskComponent, MaskShape, Spot, SpotMode};

/// Version of the rendering math. Stored with every photo so that a future
/// change to the pipeline can keep old edits looking the way they did.
pub const PROCESS_VERSION: u32 = 1;

/// Colour or black-and-white rendering.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Treatment {
    #[default]
    Color,
    BlackAndWhite,
}

/// The base rendering applied to camera RAW data before any slider: how
/// scene light is mapped to a displayable picture. Raster files already carry
/// their rendering, so the profile does not affect them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    /// A balanced, camera-like rendering.
    #[default]
    Standard,
    /// Lower contrast and saturation: a flatter starting point.
    Neutral,
    /// Higher contrast and saturation.
    Vivid,
    /// No tone curve at all: scene-linear data shown as it is.
    Linear,
}

impl Profile {
    pub const ALL: [Self; 4] = [Self::Standard, Self::Neutral, Self::Vivid, Self::Linear];

    pub fn label(self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::Neutral => "Neutral",
            Self::Vivid => "Vivid",
            Self::Linear => "Linear",
        }
    }
}

/// Everything that determines how a photo is rendered.
///
/// `Default` is the neutral state: rendering a raster file with it reproduces
/// the file. Use [`DevelopSettings::default_for`] for a newly imported photo.
///
/// Serialized as JSON in the catalog. Every field has a default and unknown
/// fields are ignored, so settings written by older and newer versions load.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DevelopSettings {
    pub process_version: u32,
    pub treatment: Treatment,
    pub profile: Profile,
    pub white_balance: WhiteBalance,
    pub tone: Tone,
    pub presence: Presence,
    pub tone_curve: ToneCurve,
    pub color_mixer: ColorMixer,
    pub color_grading: ColorGrading,
    pub detail: Detail,
    pub lens: LensCorrections,
    pub transform: Transform,
    pub effects: Effects,
    pub calibration: Calibration,
    pub orientation: Orientation,
    pub crop: Crop,
    pub spots: Vec<Spot>,
    pub masks: Vec<Mask>,
}

impl Default for DevelopSettings {
    fn default() -> Self {
        Self {
            process_version: PROCESS_VERSION,
            treatment: Treatment::default(),
            profile: Profile::default(),
            white_balance: WhiteBalance::default(),
            tone: Tone::default(),
            presence: Presence::default(),
            tone_curve: ToneCurve::default(),
            color_mixer: ColorMixer::default(),
            color_grading: ColorGrading::default(),
            detail: Detail::default(),
            lens: LensCorrections::default(),
            transform: Transform::default(),
            effects: Effects::default(),
            calibration: Calibration::default(),
            orientation: Orientation::default(),
            crop: Crop::default(),
            spots: Vec::new(),
            masks: Vec::new(),
        }
    }
}

impl DevelopSettings {
    /// Default sharpening amount for camera RAW (demosaiced data is soft).
    pub const RAW_SHARPEN_AMOUNT: f32 = 40.0;
    /// Default colour noise reduction for camera RAW.
    pub const RAW_COLOR_NOISE_REDUCTION: f32 = 25.0;

    /// The settings a newly imported photo of this kind starts with.
    pub fn default_for(kind: FileKind) -> Self {
        let mut settings = Self::default();
        if kind == FileKind::Raw {
            settings.detail.sharpening.amount = Self::RAW_SHARPEN_AMOUNT;
            settings.detail.noise_reduction.color = Self::RAW_COLOR_NOISE_REDUCTION;
        }
        settings
    }

    /// True when these are exactly the starting settings for `kind`.
    pub fn is_default_for(&self, kind: FileKind) -> bool {
        *self == Self::default_for(kind)
    }

    /// Force every scalar adjustment into its valid range for `kind`.
    /// Settings arriving from outside (XMP, presets, a paste from a photo of
    /// another kind) pass through this before they are stored.
    ///
    /// A custom white balance cannot be carried between RAW and raster
    /// sources because the units differ, so out-of-range temperature falls
    /// back to as-shot rather than being clamped to a meaningless value.
    pub fn sanitize_for(&mut self, kind: FileKind) {
        if self.white_balance.preset != WbPreset::AsShot {
            let spec = Adj::Temperature.spec_for(kind);
            let temp = self.white_balance.temp;
            if !temp.is_finite() || temp < spec.min || temp > spec.max {
                self.white_balance = WhiteBalance::default();
            }
        }
        for adj in Adj::all() {
            if adj == Adj::Temperature && self.white_balance.preset == WbPreset::AsShot {
                continue;
            }
            let value = adj.get(self);
            let clamped = adj.spec_for(kind).clamp(value);
            if clamped != value {
                adj.set(self, clamped);
            }
        }
        self.orientation = Orientation::new(self.orientation.quarter_turns, self.orientation.flip_horizontal);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_survive_a_json_round_trip() {
        let mut settings = DevelopSettings::default_for(FileKind::Raw);
        settings.tone.exposure = 0.35;
        settings.tone_curve.rgb = PointCurve::new([
            CurvePoint::new(0.0, 0.05),
            CurvePoint::new(0.5, 0.6),
            CurvePoint::new(1.0, 1.0),
        ]);
        settings.color_mixer.hue[ColorBand::Blue.index()] = -20.0;
        settings.crop.rect = Some(CropRect {
            cx: 0.4,
            cy: 0.5,
            width: 0.6,
            height: 0.7,
        });
        settings.orientation = Orientation::new(1, true);
        settings.spots.push(Spot {
            mode: SpotMode::Heal,
            target: crate::geom::Point::new(0.25, 0.75),
            source: crate::geom::Point::new(0.3, 0.7),
            radius: 0.02,
            feather: 50.0,
            opacity: 100.0,
        });
        settings.masks.push(Mask::from_shape(
            MaskShape::Linear {
                start: crate::geom::Point::new(0.5, 0.0),
                end: crate::geom::Point::new(0.5, 0.4),
            },
            1,
        ));
        let json = serde_json::to_string(&settings).unwrap();
        assert_eq!(serde_json::from_str::<DevelopSettings>(&json).unwrap(), settings);
    }

    #[test]
    fn missing_fields_take_their_defaults() {
        let settings: DevelopSettings = serde_json::from_str(r#"{"tone":{"exposure":1.5}}"#).unwrap();
        assert_eq!(settings.tone.exposure, 1.5);
        assert_eq!(settings.tone.contrast, 0.0);
        assert_eq!(settings.effects.vignette.midpoint, 50.0);
        assert_eq!(settings.process_version, PROCESS_VERSION);
    }

    #[test]
    fn unknown_fields_from_a_newer_version_are_ignored() {
        let json = r#"{"tone":{"exposure":1.0,"future_slider":3},"future_panel":{"a":1}}"#;
        let settings: DevelopSettings = serde_json::from_str(json).unwrap();
        assert_eq!(settings.tone.exposure, 1.0);
    }

    #[test]
    fn raw_photos_start_with_capture_sharpening_and_colour_denoise() {
        let raw = DevelopSettings::default_for(FileKind::Raw);
        assert_eq!(raw.detail.sharpening.amount, DevelopSettings::RAW_SHARPEN_AMOUNT);
        assert_eq!(raw.detail.noise_reduction.color, DevelopSettings::RAW_COLOR_NOISE_REDUCTION);
        assert!(raw.is_default_for(FileKind::Raw));
        assert!(!raw.is_default_for(FileKind::Raster));
        assert_eq!(DevelopSettings::default_for(FileKind::Raster), DevelopSettings::default());
    }

    #[test]
    fn sanitize_clamps_out_of_range_values() {
        let mut settings = DevelopSettings::default();
        settings.tone.exposure = 40.0;
        settings.presence.clarity = -500.0;
        settings.detail.sharpening.radius = f32::NAN;
        settings.orientation.quarter_turns = 7;
        settings.sanitize_for(FileKind::Raster);
        assert_eq!(settings.tone.exposure, 5.0);
        assert_eq!(settings.presence.clarity, -100.0);
        assert_eq!(settings.detail.sharpening.radius, 0.5);
        assert_eq!(settings.orientation.quarter_turns, 3);
    }

    #[test]
    fn sanitize_drops_a_white_balance_in_the_wrong_units() {
        let kelvin = DevelopSettings {
            white_balance: WhiteBalance {
                preset: WbPreset::Custom,
                temp: 5600.0,
                tint: 12.0,
            },
            ..Default::default()
        };
        let mut for_raw = kelvin.clone();
        for_raw.sanitize_for(FileKind::Raw);
        assert_eq!(for_raw.white_balance, kelvin.white_balance);

        let mut for_raster = kelvin.clone();
        for_raster.sanitize_for(FileKind::Raster);
        assert_eq!(for_raster.white_balance, WhiteBalance::default());

        let mut as_shot = DevelopSettings::default();
        as_shot.sanitize_for(FileKind::Raw);
        assert_eq!(as_shot, DevelopSettings::default());
    }
}
