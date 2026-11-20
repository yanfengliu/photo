//! Identifiers and the descriptive attributes of a photo: its rating, flag
//! and colour label, and the metadata read from the file.

use serde::{Deserialize, Serialize};

use crate::time::DateTime;

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub i64);

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

id_type!(
    /// A photo in the catalog. A virtual copy has its own `PhotoId`.
    PhotoId
);
id_type!(
    /// A folder on disk that the catalog knows about.
    FolderId
);
id_type!(
    /// A collection, smart collection or collection set.
    CollectionId
);
id_type!(
    /// A keyword tag.
    KeywordId
);
id_type!(
    /// One import run, used for "Previous Import".
    ImportId
);
id_type!(
    /// A saved develop preset.
    PresetId
);
id_type!(
    /// A named snapshot of a photo's develop settings.
    SnapshotId
);
id_type!(
    /// One step in a photo's develop history.
    HistoryId
);

/// A star rating from 0 (unrated) to 5.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Rating(u8);

impl Rating {
    pub const MAX: u8 = 5;
    pub const UNRATED: Self = Self(0);

    /// Clamps to the valid range.
    pub fn new(stars: u8) -> Self {
        Self(stars.min(Self::MAX))
    }

    pub fn stars(self) -> u8 {
        self.0
    }

    pub fn increased(self) -> Self {
        Self::new(self.0 + 1)
    }

    pub fn decreased(self) -> Self {
        Self(self.0.saturating_sub(1))
    }
}

/// The pick / reject flag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Flag {
    Rejected,
    #[default]
    Unflagged,
    Picked,
}

impl Flag {
    /// Stable integer form used in the catalog and in XMP: -1, 0, 1.
    pub fn to_i8(self) -> i8 {
        match self {
            Self::Rejected => -1,
            Self::Unflagged => 0,
            Self::Picked => 1,
        }
    }

    pub fn from_i8(value: i8) -> Self {
        match value {
            v if v < 0 => Self::Rejected,
            0 => Self::Unflagged,
            _ => Self::Picked,
        }
    }
}

/// A colour label.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorLabel {
    #[default]
    None,
    Red,
    Yellow,
    Green,
    Blue,
    Purple,
}

impl ColorLabel {
    pub const ALL: [Self; 6] = [
        Self::None,
        Self::Red,
        Self::Yellow,
        Self::Green,
        Self::Blue,
        Self::Purple,
    ];

    /// The label's name as written to XMP (`xmp:Label`); empty for `None`.
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Red => "Red",
            Self::Yellow => "Yellow",
            Self::Green => "Green",
            Self::Blue => "Blue",
            Self::Purple => "Purple",
        }
    }

    /// Case-insensitive inverse of [`Self::name`]; unknown text is `None`.
    pub fn from_name(name: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|label| label.name().eq_ignore_ascii_case(name.trim()))
            .unwrap_or(Self::None)
    }
}

/// Whether a file holds rendered pixels or sensor data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    /// JPEG, PNG, TIFF and friends: a finished rendering.
    Raster,
    /// Camera RAW: scene-referred sensor data.
    Raw,
}

/// Metadata read from a file. Every field is optional because every file is
/// allowed to be missing it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PhotoMetadata {
    /// Pixel dimensions after applying the file's own orientation.
    pub width: u32,
    pub height: u32,
    pub capture_time: Option<DateTime>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens: Option<String>,
    pub focal_length_mm: Option<f32>,
    pub focal_length_35mm: Option<f32>,
    /// f-number, e.g. 2.8.
    pub aperture: Option<f32>,
    /// Exposure time in seconds, e.g. 0.004 for 1/250.
    pub shutter_seconds: Option<f32>,
    pub iso: Option<u32>,
    pub exposure_bias_ev: Option<f32>,
    pub flash_fired: Option<bool>,
    pub gps: Option<GpsPosition>,
    pub artist: Option<String>,
    pub copyright: Option<String>,
}

impl PhotoMetadata {
    /// "Make Model" without repeating the make when the model already starts
    /// with it (many cameras write "SONY" and "SONY ILCE-7RM5").
    pub fn camera_name(&self) -> Option<String> {
        match (self.camera_make.as_deref(), self.camera_model.as_deref()) {
            (Some(make), Some(model)) => {
                let repeats = model
                    .to_ascii_lowercase()
                    .starts_with(&make.to_ascii_lowercase());
                Some(if repeats {
                    model.to_string()
                } else {
                    format!("{make} {model}")
                })
            }
            (None, Some(model)) => Some(model.to_string()),
            (Some(make), None) => Some(make.to_string()),
            (None, None) => None,
        }
    }
}

/// A geographic position in decimal degrees (north and east positive).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GpsPosition {
    pub latitude: f64,
    pub longitude: f64,
    pub altitude_m: Option<f32>,
}

/// Format an exposure time the way photographers read it: `1/250`, `0.4"`,
/// `30"`.
pub fn format_shutter(seconds: f32) -> String {
    if !(seconds.is_finite() && seconds > 0.0) {
        return String::new();
    }
    if seconds >= 0.3 {
        let rounded = (seconds * 10.0).round() / 10.0;
        if (rounded - rounded.round()).abs() < 0.05 {
            format!("{}\"", rounded.round() as u32)
        } else {
            format!("{rounded:.1}\"")
        }
    } else {
        format!("1/{}", (1.0 / seconds).round() as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rating_clamps_and_steps_within_zero_to_five() {
        assert_eq!(Rating::new(9).stars(), 5);
        assert_eq!(Rating::new(5).increased().stars(), 5);
        assert_eq!(Rating::UNRATED.decreased().stars(), 0);
        assert_eq!(Rating::new(2).increased().stars(), 3);
    }

    #[test]
    fn flag_integer_form_round_trips() {
        for flag in [Flag::Rejected, Flag::Unflagged, Flag::Picked] {
            assert_eq!(Flag::from_i8(flag.to_i8()), flag);
        }
    }

    #[test]
    fn label_names_round_trip_case_insensitively() {
        for label in ColorLabel::ALL {
            assert_eq!(ColorLabel::from_name(label.name()), label);
        }
        assert_eq!(ColorLabel::from_name(" green "), ColorLabel::Green);
        assert_eq!(ColorLabel::from_name("Review"), ColorLabel::None);
    }

    #[test]
    fn camera_name_does_not_repeat_the_make() {
        let mut meta = PhotoMetadata {
            camera_make: Some("SONY".into()),
            camera_model: Some("ILCE-7RM5".into()),
            ..Default::default()
        };
        assert_eq!(meta.camera_name().as_deref(), Some("SONY ILCE-7RM5"));
        meta.camera_model = Some("Sony ILCE-7RM5".into());
        assert_eq!(meta.camera_name().as_deref(), Some("Sony ILCE-7RM5"));
    }

    #[test]
    fn shutter_formats_like_a_camera_display() {
        assert_eq!(format_shutter(1.0 / 250.0), "1/250");
        assert_eq!(format_shutter(0.4), "0.4\"");
        assert_eq!(format_shutter(30.0), "30\"");
        assert_eq!(format_shutter(0.0), "");
    }
}
