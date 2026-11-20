//! The Basic panel: white balance, tone and presence.

use serde::{Deserialize, Serialize};

/// How the white balance was chosen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WbPreset {
    /// Use the white balance the camera recorded (or, for a raster file, the
    /// file as it is). `temp` and `tint` are ignored.
    #[default]
    AsShot,
    /// Computed from the image when chosen; the result is stored in `temp`
    /// and `tint`.
    Auto,
    Daylight,
    Cloudy,
    Shade,
    Tungsten,
    Fluorescent,
    Flash,
    /// The user set `temp` and `tint` directly.
    Custom,
}

impl WbPreset {
    pub const ALL: [Self; 9] = [
        Self::AsShot,
        Self::Auto,
        Self::Daylight,
        Self::Cloudy,
        Self::Shade,
        Self::Tungsten,
        Self::Fluorescent,
        Self::Flash,
        Self::Custom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::AsShot => "As Shot",
            Self::Auto => "Auto",
            Self::Daylight => "Daylight",
            Self::Cloudy => "Cloudy",
            Self::Shade => "Shade",
            Self::Tungsten => "Tungsten",
            Self::Fluorescent => "Fluorescent",
            Self::Flash => "Flash",
            Self::Custom => "Custom",
        }
    }

    /// The fixed (kelvin, tint) of an illuminant preset; `None` for the
    /// presets that are not a fixed illuminant. Only meaningful for camera
    /// RAW sources, where temperature is absolute.
    pub fn illuminant(self) -> Option<(f32, f32)> {
        match self {
            Self::Daylight => Some((5500.0, 10.0)),
            Self::Cloudy => Some((6500.0, 10.0)),
            Self::Shade => Some((7500.0, 10.0)),
            Self::Tungsten => Some((2850.0, 0.0)),
            Self::Fluorescent => Some((3800.0, 21.0)),
            Self::Flash => Some((5500.0, 0.0)),
            Self::AsShot | Self::Auto | Self::Custom => None,
        }
    }
}

/// White balance.
///
/// The units of `temp` and `tint` depend on the source. For camera RAW they
/// are absolute: `temp` in kelvin (2000 to 50000) and `tint` on the DNG scale
/// (-150 to 150, positive is magenta). For raster files they are relative
/// offsets from the file as it is (-100 to 100; positive `temp` is warmer,
/// positive `tint` is more magenta).
///
/// When `preset` is [`WbPreset::AsShot`] the two numbers are ignored.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WhiteBalance {
    pub preset: WbPreset,
    pub temp: f32,
    pub tint: f32,
}

/// Global tone controls. `exposure` is in stops (-5 to 5); the rest run from
/// -100 to 100.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tone {
    pub exposure: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
}

/// Presence controls, each -100 to 100.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Presence {
    pub texture: f32,
    pub clarity: f32,
    pub dehaze: f32,
    pub vibrance: f32,
    pub saturation: f32,
}
