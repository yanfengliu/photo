//! What an export does: the choices of the export dialog (DESIGN §8.4),
//! stored as an export preset.
//!
//! Every type here is plain data with a `Default` and serde support; every
//! struct reads missing fields as their defaults, so a preset saved by an
//! older build still loads. Values from outside (a preset file, the dialog)
//! are not trusted: [`crate::export`] refuses a size or quality it cannot
//! honour with [`crate::ExportError::Unsupported`] and clamps the rest.

use std::path::PathBuf;

use photo_core::colorimetry::RgbSpace;
use serde::{Deserialize, Serialize};

/// Everything one export preset decides.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportSettings {
    /// Where files go and what happens when a name is taken.
    pub location: Location,
    /// How files are named.
    pub naming: Naming,
    /// The file format and its options.
    pub format: Format,
    /// The colour space the pixels are encoded in. Every file carries a
    /// matching ICC profile, whatever the format and metadata settings.
    pub color_space: RgbSpace,
    /// The size of the exported picture.
    pub sizing: Sizing,
    /// Sharpening for the screen, applied after sizing.
    pub sharpening: OutputSharpening,
    /// Which metadata is written into the file.
    pub metadata: MetadataSettings,
    /// A text watermark, or `None` for none.
    pub watermark: Option<Watermark>,
}

/// Where exported files are written.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Location {
    /// The folder to write into.
    pub folder: Folder,
    /// A folder created inside [`Self::folder`] when it does not exist, or
    /// empty for none. `/` and `\` separate nested folders; characters a
    /// file name cannot hold are replaced and `.`/`..` components dropped,
    /// so it never leads outside the chosen folder.
    pub subfolder: String,
    /// What to do when the planned file name is already taken.
    pub existing: ExistingFile,
}

/// The folder an export goes into.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Folder {
    /// The folder that holds the original file.
    #[default]
    SameAsOriginal,
    /// A folder chosen by the user.
    Specific(PathBuf),
}

/// What happens when a file with the planned name exists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExistingFile {
    /// Write under the first free name with a numeric suffix: `name-2.jpg`,
    /// `name-3.jpg`, and so on.
    #[default]
    UniqueName,
    /// Replace the existing file. The original photo is never replaced:
    /// asking to is [`crate::ExportError::WouldOverwriteSource`].
    Overwrite,
    /// Leave the existing file alone and write nothing.
    Skip,
}

/// How exported files are named.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Naming {
    /// The file name without its extension, with tokens in braces:
    ///
    /// | Token | Becomes |
    /// | --- | --- |
    /// | `{name}` | the original file name without its extension |
    /// | `{text}` | [`Self::custom_text`] |
    /// | `{seq}` | the item's sequence number |
    /// | `{seq:N}` | the sequence number zero-padded to `N` digits (1 to 9) |
    /// | `{date}` | the capture date as `YYYY-MM-DD`, empty when unknown |
    ///
    /// Anything else, unknown tokens included, is kept as written. The
    /// result has the characters no file name may hold (`/ \ : * ? " < > |`
    /// and control characters) replaced by `_`, leading and trailing dots
    /// and spaces removed and Windows device names (`CON`, `NUL`, ...)
    /// suffixed with `_`; when nothing is left it falls back to the original
    /// file name.
    pub template: String,
    /// The text the `{text}` token stands for.
    pub custom_text: String,
    /// Whether the extension is written in lower or upper case.
    pub extension_case: ExtensionCase,
}

impl Default for Naming {
    /// The original file name.
    fn default() -> Self {
        Self {
            template: "{name}".into(),
            custom_text: String::new(),
            extension_case: ExtensionCase::default(),
        }
    }
}

/// The case of the file extension.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionCase {
    /// `.jpg`, `.png`, `.tif`.
    #[default]
    Lower,
    /// `.JPG`, `.PNG`, `.TIF`.
    Upper,
}

/// The file format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Format {
    /// Baseline JPEG, 8 bits, no chroma subsampling. `quality` runs from 0
    /// to 100; 0 and 1 both give the encoder's lowest quality and values
    /// above 100 are read as 100.
    Jpeg {
        /// 0 (smallest file) to 100 (best picture).
        quality: u8,
    },
    /// Lossless PNG.
    Png {
        /// Bits per sample.
        depth: BitDepth,
    },
    /// Uncompressed baseline TIFF.
    Tiff {
        /// Bits per sample.
        depth: BitDepth,
    },
}

impl Default for Format {
    /// JPEG at quality 90.
    fn default() -> Self {
        Self::Jpeg { quality: 90 }
    }
}

impl Format {
    /// The file extension, lower case, without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Jpeg { .. } => "jpg",
            Self::Png { .. } => "png",
            Self::Tiff { .. } => "tif",
        }
    }

    /// Bits per sample the file is written with: always 8 for JPEG.
    pub fn bits(self) -> u8 {
        match self {
            Self::Jpeg { .. } => 8,
            Self::Png { depth } | Self::Tiff { depth } => depth.bits(),
        }
    }
}

/// Bits per sample of a PNG or TIFF.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BitDepth {
    /// 8 bits per sample.
    #[default]
    Eight,
    /// 16 bits per sample: the pipeline's full export precision.
    Sixteen,
}

impl BitDepth {
    /// 8 or 16.
    pub fn bits(self) -> u8 {
        match self {
            Self::Eight => 8,
            Self::Sixteen => 16,
        }
    }
}

/// The size of the exported picture, relative to the full-resolution
/// output that the photo's crop and orientation give (see
/// [`photo_develop::output_size`]). The aspect ratio is always kept.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sizing {
    /// How the size is chosen.
    pub resize: Resize,
    /// Never make the picture larger than its full-resolution output.
    pub dont_enlarge: bool,
}

impl Default for Sizing {
    /// Full resolution, never enlarged.
    fn default() -> Self {
        Self {
            resize: Resize::Original,
            dont_enlarge: true,
        }
    }
}

/// How the export size is chosen. Sides are rounded to whole pixels and are
/// at least one pixel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum Resize {
    /// The full-resolution output, unscaled.
    #[default]
    Original,
    /// The longer side is `pixels` long.
    LongEdge {
        /// Length of the longer side in pixels, at least 1.
        pixels: u32,
    },
    /// The shorter side is `pixels` long.
    ShortEdge {
        /// Length of the shorter side in pixels, at least 1.
        pixels: u32,
    },
    /// As large as fits inside `width` x `height`.
    Fit {
        /// Largest width in pixels, at least 1.
        width: u32,
        /// Largest height in pixels, at least 1.
        height: u32,
    },
    /// About `megapixels` million pixels in all.
    Megapixels {
        /// Millions of pixels; positive and finite.
        megapixels: f32,
    },
    /// The full-resolution output scaled by `percent` / 100.
    Percent {
        /// Scale in percent; positive and finite.
        percent: f32,
    },
}

/// Sharpening applied for viewing on a screen, after the picture has been
/// sized: an unsharp mask on luminance with a sub-pixel radius.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputSharpening {
    /// No output sharpening.
    #[default]
    Off,
    /// A light touch.
    Low,
    /// The usual amount for screen viewing.
    Standard,
    /// Strong sharpening.
    High,
}

/// Which metadata goes into the file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct MetadataSettings {
    /// How much is written.
    pub include: MetadataMode,
    /// Leave out where the photo was taken (the EXIF GPS directory).
    pub strip_location: bool,
}

/// How much metadata is written. Nothing here affects the ICC profile,
/// which every file carries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataMode {
    /// Camera, exposure, capture time, location, creator and copyright in
    /// EXIF; title, caption, creator, copyright, keywords and rating in XMP.
    #[default]
    All,
    /// Only the creator and the copyright notice (and the EXIF orientation,
    /// which describes the pixels rather than the photo).
    CopyrightAndContact,
    /// No EXIF and no XMP at all.
    None,
}

/// A line of text drawn over the picture.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Watermark {
    /// The text, on one line. Printable ASCII is drawn as it is; `©`
    /// becomes `(c)`, other characters become `?`. Empty text draws
    /// nothing.
    pub text: String,
    /// Where the text sits.
    pub anchor: Anchor,
    /// Height of the capital letters as a percentage of the picture's
    /// shorter side, so the watermark scales with the export size. Text too
    /// wide for the picture is made smaller until it fits.
    pub size: f32,
    /// 0 (invisible) to 100 (opaque).
    pub opacity: f32,
    /// The text colour as 8-bit sRGB, converted to the export's colour
    /// space.
    pub color: [u8; 3],
}

impl Default for Watermark {
    /// No text yet; white, 60% opaque, bottom right, 3% of the short side.
    fn default() -> Self {
        Self {
            text: String::new(),
            anchor: Anchor::default(),
            size: 3.0,
            opacity: 60.0,
            color: [255, 255, 255],
        }
    }
}

/// One of nine positions in the picture, each inset from the edges it is
/// near by 2.5% of the shorter side.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    /// Top left corner.
    TopLeft,
    /// Middle of the top edge.
    Top,
    /// Top right corner.
    TopRight,
    /// Middle of the left edge.
    Left,
    /// The centre of the picture.
    Center,
    /// Middle of the right edge.
    Right,
    /// Bottom left corner.
    BottomLeft,
    /// Middle of the bottom edge.
    Bottom,
    /// Bottom right corner.
    #[default]
    BottomRight,
}

impl Anchor {
    /// All nine anchors, row by row from the top left.
    pub const ALL: [Self; 9] = [
        Self::TopLeft,
        Self::Top,
        Self::TopRight,
        Self::Left,
        Self::Center,
        Self::Right,
        Self::BottomLeft,
        Self::Bottom,
        Self::BottomRight,
    ];

    /// Horizontal and vertical placement as fractions: 0 is the left or top
    /// edge, 0.5 the middle, 1 the right or bottom edge.
    pub fn fractions(self) -> (f32, f32) {
        let index = Self::ALL
            .iter()
            .position(|&anchor| anchor == self)
            .unwrap_or(0);
        ((index % 3) as f32 / 2.0, (index / 3) as f32 / 2.0)
    }
}

#[cfg(test)]
mod tests {
    use serde::Serialize;
    use serde::de::DeserializeOwned;

    use super::*;

    /// Presets are stored with serde; no serde format is a dependency of
    /// this crate, so this pins the trait bounds rather than a round trip.
    #[test]
    fn every_setting_can_be_stored_as_a_preset() {
        fn storable<T: Serialize + DeserializeOwned + Default + Clone + PartialEq>() {}
        storable::<ExportSettings>();
        storable::<Location>();
        storable::<Naming>();
        storable::<Format>();
        storable::<Sizing>();
        storable::<MetadataSettings>();
        storable::<Watermark>();
    }

    #[test]
    fn anchors_span_the_picture_row_by_row() {
        assert_eq!(Anchor::TopLeft.fractions(), (0.0, 0.0));
        assert_eq!(Anchor::Center.fractions(), (0.5, 0.5));
        assert_eq!(Anchor::Right.fractions(), (1.0, 0.5));
        assert_eq!(Anchor::Bottom.fractions(), (0.5, 1.0));
        assert_eq!(Anchor::BottomRight.fractions(), (1.0, 1.0));
    }
}
