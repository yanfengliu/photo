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
mod local_adjust;

#[cfg(test)]
mod format_tests;
#[cfg(test)]
mod tests;

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
pub use local_adjust::LocalAdj;

/// Version of the rendering math. Stored with every photo so that a future
/// change to the pipeline can keep old edits looking the way they did.
pub const PROCESS_VERSION: u32 = 1;

/// Version of the JSON shape [`DevelopSettings`] serialises to: its field
/// names, its nesting and the names of its enum values. Wherever settings are
/// stored (the catalog, sidecars, presets), this number is stored beside
/// them, and a build refuses to overwrite settings written in a newer format,
/// because it would silently drop what it does not understand.
///
/// Bump it whenever the shape changes. The test
/// `the_settings_json_shape_is_pinned_to_its_format` fails when the shape
/// changes, and its message says to bump this and re-pin.
pub const SETTINGS_FORMAT: u32 = 1;

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

    /// Force every number into its valid range for `kind`: each scalar
    /// adjustment, each mask's local adjustments and shapes, each healing
    /// spot, and the crop. Settings arriving from outside (XMP, presets, a
    /// paste from a photo of another kind) pass through this before they are
    /// stored, and nothing comes out of it infinite or NaN.
    ///
    /// A custom white balance cannot be carried between RAW and raster
    /// sources because the units differ, so out-of-range temperature falls
    /// back to as-shot rather than being clamped to a meaningless value. A
    /// spot, gradient or crop rectangle whose position is not finite is
    /// dropped, as is a brush stroke with no finite point.
    pub fn sanitize_for(&mut self, kind: FileKind) {
        let angle = self.crop.angle;
        self.crop.rect = self.crop.rect.and_then(|rect| rect.sanitized(angle));
        self.crop.constraint = self.crop.constraint.sanitized();
        for mask in &mut self.masks {
            mask.components = std::mem::take(&mut mask.components)
                .into_iter()
                .filter_map(|component| {
                    component.shape.sanitized().map(|shape| MaskComponent {
                        shape,
                        subtract: component.subtract,
                    })
                })
                .collect();
            if !(0.0..=100.0).contains(&mask.amount) {
                mask.amount = if mask.amount < 0.0 { 0.0 } else { 100.0 };
            }
            for adj in LocalAdj::ALL {
                let value = adj.get(&mask.adjust);
                let clamped = adj.spec().clamp(value);
                if clamped != value {
                    adj.set(&mut mask.adjust, clamped);
                }
            }
        }
        self.spots = self
            .spots
            .iter()
            .filter_map(|spot| spot.sanitized())
            .collect();
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
        self.orientation = Orientation::new(
            self.orientation.quarter_turns,
            self.orientation.flip_horizontal,
        );
    }
}
