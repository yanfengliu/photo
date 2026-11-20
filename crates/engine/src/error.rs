//! The engine's typed errors.

use std::io;
use std::path::PathBuf;

use photo_catalog::CatalogError;

/// Everything an engine call can fail with.
///
/// A failed call leaves the catalog as it was: every catalog write is one
/// transaction, and a command that fails is not put on the undo stack.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EngineError {
    /// The catalog refused or failed the operation (see [`CatalogError`]).
    #[error(transparent)]
    Catalog(#[from] CatalogError),

    /// The previews folder could not be created or used.
    #[error("cannot use the previews folder {}: {source}", .path.display())]
    PreviewsFolder {
        /// The folder named in the engine's configuration.
        path: PathBuf,
        /// What went wrong.
        #[source]
        source: io::Error,
    },

    /// The background worker threads could not be started.
    #[error("cannot start the background workers: {0}")]
    Workers(#[source] io::Error),

    /// A folder pattern for copying imports by date is not usable.
    #[error("invalid folder pattern {pattern:?}: {reason}")]
    InvalidPattern {
        /// The pattern as given.
        pattern: String,
        /// Why it was refused.
        reason: &'static str,
    },

    /// The change was refused, for the reason given (the catalog is
    /// write-protected, or the photo's settings were saved by a newer
    /// version). Nothing was changed.
    #[error("{0}")]
    NotEditable(String),

    /// No photo is open in Develop.
    #[error("no photo is open in Develop")]
    NoDevelopSession,

    /// The photo open in Develop is still loading; try again once it is
    /// decoded.
    #[error("the photo is still loading")]
    StillLoading,

    /// The open photo has no mask with this index.
    #[error("there is no mask {0}")]
    NoSuchMask(usize),

    /// There is no active photo to take settings from.
    #[error("no photo is active")]
    NoActivePhoto,

    /// Nothing has been copied to paste.
    #[error("no settings have been copied")]
    NothingToPaste,

    /// There is no previous photo to take settings from.
    #[error("there is no previous photo")]
    NoPreviousPhoto,

    /// An export was asked for with no photo to export.
    #[error("no photos to export")]
    NothingToExport,

    /// A name was empty once trimmed.
    #[error("a name cannot be empty")]
    EmptyName,

    /// The saved export presets could not be read, or these settings could
    /// not be stored so that they read back. Nothing was changed.
    #[error("the export presets cannot be stored or read: {0}")]
    ExportPresets(String),

    /// A photo could not be rendered, for this reason (its file is missing
    /// or cannot be decoded).
    #[error("the photo could not be rendered: {0}")]
    Render(String),
}
