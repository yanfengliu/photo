//! What one export is given and what it reports.

use std::io;
use std::path::{Path, PathBuf};

use photo_core::develop::DevelopSettings;
use photo_core::meta::{PhotoMetadata, Rating};
use photo_decode::DecodeError;
use photo_develop::Source;

/// One photo to export.
#[derive(Clone, Debug)]
pub struct ExportItem<'a> {
    /// The original file. It is read (when [`Self::source`] is `None`) and
    /// never written; its folder and name feed the naming template.
    pub source_path: &'a Path,
    /// The decoded original, when the caller already has it; `None` makes
    /// the export decode [`Self::source_path`] itself.
    pub source: Option<&'a Source>,
    /// The edit to render.
    pub settings: &'a DevelopSettings,
    /// What the original file says about itself: camera, exposure, capture
    /// time, location, and the artist and copyright used when
    /// [`Self::attributes`] has none.
    pub metadata: &'a PhotoMetadata,
    /// The catalog's descriptive fields for the photo.
    pub attributes: ExportAttributes,
    /// This item's number in the export run, for the `{seq}` token.
    pub sequence: u32,
}

/// The catalog's descriptive fields for a photo, written as metadata. A
/// field that is `None` or blank is not written.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExportAttributes {
    /// Written as XMP `dc:title`.
    pub title: Option<String>,
    /// Written as XMP `dc:description`.
    pub caption: Option<String>,
    /// Written as XMP `dc:creator` and EXIF `Artist`; when blank, the
    /// original's own artist is used.
    pub creator: Option<String>,
    /// Written as XMP `dc:rights` and EXIF `Copyright`; when blank, the
    /// original's own copyright notice is used.
    pub copyright: Option<String>,
    /// Written as XMP `dc:subject`, in this order; blank keywords are
    /// skipped.
    pub keywords: Vec<String>,
    /// Written as XMP `xmp:Rating` when the photo is rated.
    pub rating: Rating,
}

/// What an export did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExportOutcome {
    /// The file was written, complete, at `path`.
    Written {
        /// Where the file is.
        path: PathBuf,
        /// Width of the picture in pixels.
        width: u32,
        /// Height of the picture in pixels.
        height: u32,
    },
    /// A file named `path` already existed and the policy was
    /// [`ExistingFile::Skip`](crate::ExistingFile::Skip): nothing was
    /// written or changed.
    Skipped {
        /// The existing file that was left alone.
        path: PathBuf,
    },
}

impl ExportOutcome {
    /// The file written, or the existing file that was skipped.
    pub fn path(&self) -> &Path {
        match self {
            Self::Written { path, .. } | Self::Skipped { path } => path,
        }
    }
}

/// Why an export failed. A failed export leaves no file behind, neither at
/// the planned name nor under a temporary one.
#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    /// The original could not be decoded.
    #[error("cannot read the original: {0}")]
    Decode(#[from] DecodeError),
    /// The cancel token was cancelled before the file was complete.
    #[error("the export was cancelled")]
    Cancelled,
    /// Reading or writing a file failed.
    #[error("cannot write {path}: {source}")]
    Io {
        /// The file or folder the failure concerns.
        path: PathBuf,
        /// What went wrong.
        source: io::Error,
    },
    /// The planned file is the original photo itself, which an export never
    /// replaces.
    #[error("{0} is the original photo; an export never replaces it")]
    WouldOverwriteSource(PathBuf),
    /// The settings ask for something this crate cannot write, such as a
    /// size of zero pixels or a JPEG wider than 65535 pixels.
    #[error("cannot export: {0}")]
    Unsupported(String),
}

impl ExportError {
    /// An I/O failure concerning `path`.
    pub(crate) fn io(path: &Path, source: io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}
