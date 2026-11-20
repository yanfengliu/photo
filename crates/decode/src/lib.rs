//! File decoding: raster and camera-RAW files in, linear `SourceImage` plus
//! metadata out. The only crate that touches image codecs for reading.
//!
//! Three entry points, all taking a path and all dispatching on its
//! extension ([`file_kind`]):
//!
//! - [`read_metadata`] reads headers only: size, capture time, camera,
//!   exposure. Fast enough to run over a whole folder at import.
//! - [`embedded_preview`] returns a small sRGB picture as quickly as the
//!   file allows, for something to show while the real decode runs.
//! - [`decode`] returns the full-resolution pixels in linear light.
//!
//! Raster files go through the `image` crate and are linearised with the
//! transfer function of their colour space. RAW files go through rawler and
//! come back camera-native and un-white-balanced, with the colour matrix and
//! as-shot gains a renderer needs.
//!
//! Every failure is a [`DecodeError`]; no input, however damaged, panics.
//!
//! See `docs/work/12_lightroom-workflow-restart/design.md`, §6.1.

mod error;
mod exif;
mod icc;
mod linear;
mod orient;
mod preview;
mod raster;
mod raw;
mod tiff_meta;

use std::path::Path;

use photo_core::image::{Rgba8Image, SourceImage};
use photo_core::meta::{FileKind, PhotoMetadata};

pub use error::DecodeError;

/// Extensions of the raster formats this crate reads, lower case, without
/// the dot.
pub const RASTER_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "tif", "tiff", "webp", "bmp", "gif"];

/// Extensions of the camera RAW formats this crate reads, lower case,
/// without the dot: exactly the list rawler declares support for (a test
/// keeps the two in step).
pub const RAW_EXTENSIONS: &[&str] = &[
    "ari", "arw", "cr2", "cr3", "crm", "crw", "dcr", "dcs", "dng", "erf", "iiq", "kdc", "mef",
    "mos", "mrw", "nef", "nrw", "orf", "ori", "pef", "raf", "raw", "rw2", "rwl", "srw", "3fr",
    "fff", "x3f", "qtk",
];

/// The largest image, in pixels, that [`decode`] and [`embedded_preview`]
/// will hold in memory. Larger files fail with [`DecodeError::TooLarge`]
/// before anything is allocated for them.
pub const MAX_PIXELS: u64 = 500_000_000;

/// A fully decoded file.
#[derive(Clone, Debug, PartialEq)]
pub struct Decoded {
    /// The pixels: linear light, 16 bits, upright. Raster files carry the
    /// colour space they were encoded in; RAW files are camera-native.
    pub image: SourceImage,
    /// What the file says about itself. Its dimensions are those of `image`.
    pub metadata: PhotoMetadata,
}

/// Whether `path` names a raster file or a camera RAW file, judged by its
/// extension alone (case-insensitive). `None` for anything this crate does
/// not read. The file is not opened and need not exist.
pub fn file_kind(path: impl AsRef<Path>) -> Option<FileKind> {
    let extension = path.as_ref().extension()?.to_str()?.to_ascii_lowercase();
    if RASTER_EXTENSIONS.contains(&extension.as_str()) {
        Some(FileKind::Raster)
    } else if RAW_EXTENSIONS.contains(&extension.as_str()) {
        Some(FileKind::Raw)
    } else {
        None
    }
}

/// Whether [`file_kind`] recognises `path`.
pub fn is_supported(path: impl AsRef<Path>) -> bool {
    file_kind(path).is_some()
}

/// Reads a file's metadata without decoding its pixels.
///
/// `width` and `height` are those of the upright picture, after the file's
/// own orientation (and for RAW files the recommended crop) is applied:
/// exactly what [`decode`] will produce. All other fields are `None` when
/// the file does not state them.
///
/// Raster files have their EXIF read with kamadak-exif. RAW files use
/// rawler's reading of the file, completed from the file's EXIF directories
/// where rawler has no field for a value.
///
/// This reads headers, not image data, so a file whose pixels are damaged
/// can still yield metadata.
pub fn read_metadata(path: impl AsRef<Path>) -> Result<PhotoMetadata, DecodeError> {
    let path = path.as_ref();
    error::guard(|| match kind_of(path)? {
        FileKind::Raster => raster::read_metadata(path),
        FileKind::Raw => raw::read_metadata(path),
    })
}

/// Decodes a file to full-resolution linear pixels.
///
/// The image is interleaved 16-bit RGB with 65535 as 1.0, turned upright
/// according to the file's orientation.
///
/// **Raster files** are decoded at their full bit depth. Grey is copied to
/// all three channels and transparency is flattened onto white. Integer
/// samples are linearised with the transfer function of the file's colour
/// space, which is sRGB unless an embedded ICC profile has the primaries of
/// one of `photo_core`'s other spaces; floating-point samples are taken as
/// linear already and clamped to 0..=1. The result is
/// `SourceColor::Rgb(space)`.
///
/// **RAW files** are scaled between their black and white levels,
/// demosaiced, cropped to the active and recommended areas and clamped to
/// 0..=1. They are *not* white balanced: the result is
/// `SourceColor::Camera` with the colour matrix calibrated nearest D65, the
/// as-shot gains (green at 1.0, the camera's daylight balance if the file
/// has none) and the DNG baseline exposure.
///
/// A truncated file is an error, including a JPEG cut off after its
/// headers, which a lenient decoder would otherwise return half grey.
pub fn decode(path: impl AsRef<Path>) -> Result<Decoded, DecodeError> {
    let path = path.as_ref();
    error::guard(|| match kind_of(path)? {
        FileKind::Raster => raster::decode(path),
        FileKind::Raw => raw::decode(path),
    })
}

/// A small upright sRGB picture of the file, obtained as cheaply as the
/// format allows, with its longer edge at most `max_edge` pixels (a
/// `max_edge` of 0 is treated as 1). The picture is never enlarged.
///
/// For RAW files this is the JPEG the camera embedded, never a develop of
/// the sensor data; `Ok(None)` means the file carries none. For raster
/// files it is the image itself scaled down, and always present.
///
/// The result is opaque RGBA8 in sRGB. Raster files in another colour space
/// are converted; embedded camera JPEGs are taken to be sRGB.
pub fn embedded_preview(
    path: impl AsRef<Path>,
    max_edge: u32,
) -> Result<Option<Rgba8Image>, DecodeError> {
    let path = path.as_ref();
    error::guard(|| match kind_of(path)? {
        FileKind::Raster => raster::preview(path, max_edge).map(Some),
        FileKind::Raw => raw::embedded_preview(path, max_edge),
    })
}

fn kind_of(path: &Path) -> Result<FileKind, DecodeError> {
    file_kind(path).ok_or_else(|| {
        let reason = match path.extension() {
            Some(extension) => format!("files of type .{}", extension.to_string_lossy()),
            None => "files without an extension".to_string(),
        };
        DecodeError::Unsupported(reason)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_raw_extension_list_is_exactly_what_rawler_supports() {
        let mut ours: Vec<String> = RAW_EXTENSIONS.iter().map(|e| e.to_string()).collect();
        let mut theirs: Vec<String> = rawler::decoders::supported_extensions()
            .iter()
            .map(|e| e.to_ascii_lowercase())
            .collect();
        ours.sort();
        theirs.sort();
        assert_eq!(ours, theirs);
    }

    #[test]
    fn a_name_can_only_be_one_kind_of_file() {
        for extension in RASTER_EXTENSIONS {
            assert!(!RAW_EXTENSIONS.contains(extension), "{extension}");
            assert_eq!(file_kind(format!("x.{extension}")), Some(FileKind::Raster));
            assert_eq!(
                file_kind(format!("X.{}", extension.to_uppercase())),
                Some(FileKind::Raster)
            );
        }
        for extension in RAW_EXTENSIONS {
            assert_eq!(file_kind(format!("x.{extension}")), Some(FileKind::Raw));
            assert_eq!(
                file_kind(format!("X.{}", extension.to_uppercase())),
                Some(FileKind::Raw)
            );
        }
    }

    #[test]
    fn refusals_name_the_extension() {
        let error = kind_of(Path::new("notes.txt")).unwrap_err().to_string();
        assert!(error.contains(".txt"), "{error}");
        let error = kind_of(Path::new("README")).unwrap_err().to_string();
        assert!(error.contains("without an extension"), "{error}");
    }
}
