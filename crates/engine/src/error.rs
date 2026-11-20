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

    /// The photo is not in the current list, so the operation that needs
    /// its place in the list cannot run.
    #[error("photo {0} is not in the current list")]
    NotInList(photo_core::meta::PhotoId),
}
