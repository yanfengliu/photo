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
    /// Other files this export must never replace, whatever name they are
    /// reached by when the export finishes: they are compared by file
    /// identity, not by path, with the file the export would replace. A
    /// path that leads to no file when the export starts is looked at
    /// again when the file is placed. Replacing one of them is refused
    /// with [`ExportError::WouldOverwriteProtected`], and a path that leads
    /// to a file that cannot be opened refuses the overwrite with
    /// [`ExportError::CannotIdentify`] (as the original does). Only the
    /// overwrite policy replaces anything, so only it reads this.
    pub protected: &'a [PathBuf],
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

/// Why an export failed. A failed export leaves no file of its own behind,
/// neither at the planned name nor under a temporary one, except a copy
/// that no identity can be read to recognise; the crate documentation says
/// so, and what becomes of a file it had moved aside.
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
    /// The file the export would replace is the original photo itself,
    /// which an export never replaces. The path is where the original is:
    /// the planned destination or, when another program took that name
    /// while the file was being identified, the visible name beside it
    /// that the original was given instead (or, in the few cases the crate
    /// documentation lists, the hidden name it could not be moved back
    /// from).
    #[error("{0} is the original photo; an export never replaces it")]
    WouldOverwriteSource(PathBuf),
    /// The file the export would replace is one of the item's
    /// [protected](ExportItem::protected) files. The path is where that
    /// file is, as for [`Self::WouldOverwriteSource`].
    #[error("{0} is protected; this export never replaces it")]
    WouldOverwriteProtected(PathBuf),
    /// The original or one of the item's [protected](ExportItem::protected)
    /// files is there, but cannot be opened to read its identity (for want
    /// of permission, say), so nothing could tell it from the file the
    /// overwrite policy would replace. The export is refused: before
    /// anything is rendered when that is so as it starts, and otherwise as
    /// it comes to place its file. A path that leads to no file at all is
    /// no reason to refuse.
    #[error("cannot read the identity of {path}, a file this export must never replace: {source}")]
    CannotIdentify {
        /// The original or the protected file.
        path: PathBuf,
        /// Why its identity cannot be read.
        source: io::Error,
    },
    /// The settings ask for something this crate cannot write, such as a
    /// size of zero pixels or a JPEG wider than 65535 pixels.
    #[error("cannot export: {0}")]
    Unsupported(String),
    /// The export failed with `error`, and files that had the export's
    /// name, which it had moved out of the way, could not get that name
    /// back. None is lost: each is whole at one of `kept`, a visible name
    /// beside the export's (`name-kept.ext`, `name-kept-2.ext` and so on)
    /// or, when even that rename fails, the hidden name it was moved to.
    /// The original and the protected files are not among them: a refusal
    /// gives their path itself.
    #[error("{error}; what had that name could not get it back and is now {}", list(.kept))]
    Kept {
        /// Why the export failed.
        error: Box<ExportError>,
        /// Where those files are now.
        kept: Vec<PathBuf>,
    },
}

impl ExportError {
    /// An I/O failure concerning `path`.
    pub(crate) fn io(path: &Path, source: io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }

    /// This failure, with where the files that could not get the export's
    /// name back now are (see [`Self::Kept`]); itself when there are none.
    pub(crate) fn with_kept(self, kept: Vec<PathBuf>) -> Self {
        if kept.is_empty() {
            self
        } else {
            Self::Kept {
                error: Box::new(self),
                kept,
            }
        }
    }
}

/// Paths for a message, one after another.
fn list(paths: &[PathBuf]) -> String {
    let shown: Vec<String> = paths
        .iter()
        .map(|path| path.display().to_string())
        .collect();
    shown.join(", ")
}
