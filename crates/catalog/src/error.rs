//! The catalog's typed errors.

use std::fmt;
use std::path::PathBuf;

use photo_core::meta::{CollectionId, FolderId, PhotoId};

use crate::collections::CollectionKind;

/// Result alias used by every catalog operation.
pub type Result<T> = std::result::Result<T, CatalogError>;

/// The kind of record an id refers to, for [`CatalogError::NotFound`],
/// [`CatalogError::AlreadyExists`] and [`CatalogError::Cycle`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Entity {
    /// A photo or virtual copy.
    Photo,
    /// A folder.
    Folder,
    /// A collection of any kind.
    Collection,
    /// A keyword.
    Keyword,
    /// An import run.
    Import,
    /// A develop preset.
    Preset,
    /// A snapshot of a photo's develop settings.
    Snapshot,
    /// A step in a photo's develop history.
    HistoryStep,
}

impl fmt::Display for Entity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Photo => "photo",
            Self::Folder => "folder",
            Self::Collection => "collection",
            Self::Keyword => "keyword",
            Self::Import => "import",
            Self::Preset => "preset",
            Self::Snapshot => "snapshot",
            Self::HistoryStep => "history step",
        })
    }
}

/// Everything that can go wrong in the catalog.
///
/// A failed operation never leaves a partial change behind: every mutation
/// runs in a transaction that is rolled back when it returns an error.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CatalogError {
    /// The file was written by a newer schema. It has not been modified.
    #[error(
        "the catalog was written by a newer version (schema {found}; this build reads up to {supported})"
    )]
    NewerSchema {
        /// The schema version stored in the file.
        found: u32,
        /// The newest schema version this build understands.
        supported: u32,
    },

    /// The file is some other database, or not a database at all. It has not
    /// been modified.
    #[error("the file is not a Photo catalog")]
    NotACatalog,

    /// Paths are stored as text, so a path that is not UTF-8 cannot be
    /// catalogued.
    #[error("path is not valid UTF-8: {}", .0.display())]
    NonUtf8Path(PathBuf),

    /// The path does not name a file inside a directory.
    #[error("path does not name a file inside a directory: {0:?}")]
    InvalidPath(String),

    /// An id that does not (or no longer) exist was used.
    #[error("{entity} {id} does not exist")]
    NotFound {
        /// What kind of record was looked for.
        entity: Entity,
        /// The id that was not found.
        id: i64,
    },

    /// A restore found a record with this id in the catalog already: what it
    /// would put back has been put back before. Nothing was changed.
    #[error("{entity} {id} is already in the catalog")]
    AlreadyExists {
        /// What kind of record it is.
        entity: Entity,
        /// The id that is already in use.
        id: i64,
    },

    /// A photo with this exact path is already catalogued.
    #[error("a photo at {0} is already in the catalog")]
    DuplicatePath(String),

    /// A sibling (or, for presets, a preset in the same group) already uses
    /// this name; names compare without regard to ASCII case.
    #[error("the name {0:?} is already in use")]
    NameTaken(String),

    /// Names must contain something other than white space.
    #[error("a name must not be empty")]
    EmptyName,

    /// The collection is not of a kind that supports the operation, for
    /// example adding photos to a collection set.
    #[error("collection {id} is a {kind}, which does not support this operation")]
    CollectionKind {
        /// The collection the operation was attempted on.
        id: CollectionId,
        /// What that collection is.
        kind: CollectionKind,
    },

    /// A new order for a collection must list exactly its members, each
    /// once.
    #[error("the new order does not list exactly the photos in collection {0}")]
    OrderMismatch(CollectionId),

    /// Moving the item there would make it its own ancestor.
    #[error("{entity} {id} cannot be moved inside itself")]
    Cycle {
        /// Which hierarchy the move was attempted in.
        entity: Entity,
        /// The item that was to be moved.
        id: i64,
    },

    /// The folder still has photos, so it cannot be removed.
    #[error("folder {0} still contains photos")]
    FolderNotEmpty(FolderId),

    /// A smart-collection rule combines a field, operator and value that do
    /// not belong together. The text names the rule and the problem.
    #[error("invalid smart collection rule: {0}")]
    InvalidRule(String),

    /// The value cannot be written in a form that can be read back (for
    /// example develop settings holding a NaN). Nothing was stored.
    #[error("{what} cannot be stored")]
    Unstorable {
        /// What was being stored.
        what: &'static str,
        /// Why it cannot be written or read back.
        #[source]
        source: serde_json::Error,
    },

    /// Stored settings or rules were written in a newer format than this
    /// build reads. Reading them would drop what it does not know, so they
    /// are not read, and any change that would replace them is refused;
    /// nothing was changed.
    #[error(
        "stored {what} were written by a newer version (format {found}; this build reads up to {supported})"
    )]
    NewerFormat {
        /// What was stored.
        what: &'static str,
        /// The format it was stored in.
        found: u32,
        /// The newest format this build reads.
        supported: u32,
    },

    /// Stored JSON could not be read back. The stored text is left alone, so
    /// it is never replaced by defaults behind the user's back.
    #[error("stored {what} is unreadable")]
    Corrupt {
        /// What was being read.
        what: &'static str,
        /// Why it could not be read.
        #[source]
        source: serde_json::Error,
    },

    /// The database connection does not enforce foreign keys, so the catalog
    /// refuses to run without its integrity guarantees.
    #[error("the database does not enforce foreign keys")]
    ForeignKeysUnavailable,

    /// A schema upgrade would have left rows that refer to rows that no
    /// longer exist. The upgrade was rolled back and the file is unchanged.
    #[error("upgrading the catalog would leave {table} rows without their {parent} rows")]
    BrokenReferences {
        /// The table whose rows would have been orphaned.
        table: String,
        /// The table they refer to.
        parent: String,
    },

    /// A develop change was undone or redone out of order: the photo's
    /// develop state is not the state that change left (for undo) or found
    /// (for redo). Nothing was changed.
    #[error("the develop state of photo {0} has changed since this edit")]
    StaleChange(PhotoId),

    /// The catalog file (or the volume it is on) is write-protected, so the
    /// catalog can be read but not changed; nothing was changed. Opening
    /// fails with this when the file would first have to be set up or
    /// upgraded.
    #[error("the catalog file is write-protected, so it cannot be changed")]
    ReadOnly,

    /// The catalog file could not be read from disk.
    #[error("the catalog file could not be read")]
    Io(#[source] std::io::Error),

    /// The deadline given with [`Catalog::set_deadline`] had passed, so the
    /// change was not tried; nothing was changed.
    ///
    /// [`Catalog::set_deadline`]: crate::Catalog::set_deadline
    #[error("the time given for changing the catalog had run out, so the change was not tried")]
    OutOfTime,

    /// Any other failure reported by SQLite.
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

impl CatalogError {
    pub(crate) fn not_found(entity: Entity, id: i64) -> Self {
        Self::NotFound { entity, id }
    }

    /// True when the catalog as a whole could not be used, whatever the
    /// call asked for: another connection has it locked (see
    /// [`Catalog::set_busy_timeout`]), the time given for changes has run
    /// out ([`CatalogError::OutOfTime`]), its file is write-protected,
    /// damaged or could not be read or written, or there was no room for
    /// the change. The same call with other values would fail the same way,
    /// so nothing is gained by trying the items of a refused batch one by
    /// one.
    ///
    /// [`Catalog::set_busy_timeout`]: crate::Catalog::set_busy_timeout
    pub fn is_catalog_wide(&self) -> bool {
        use rusqlite::ErrorCode as Code;
        match self {
            Self::ReadOnly | Self::Io(_) | Self::OutOfTime => true,
            Self::Sqlite(rusqlite::Error::SqliteFailure(failure, _)) => matches!(
                failure.code,
                Code::DatabaseBusy
                    | Code::DatabaseLocked
                    | Code::ReadOnly
                    | Code::PermissionDenied
                    | Code::SystemIoFailure
                    | Code::DatabaseCorrupt
                    | Code::NotADatabase
                    | Code::DiskFull
                    | Code::CannotOpen
                    | Code::FileLockingProtocolFailed
                    | Code::NoLargeFileSupport
                    | Code::OutOfMemory
            ),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::ffi;

    use super::*;

    fn sqlite(code: std::ffi::c_int) -> CatalogError {
        CatalogError::Sqlite(rusqlite::Error::SqliteFailure(ffi::Error::new(code), None))
    }

    #[test]
    fn failures_of_the_file_or_its_lock_are_catalog_wide_and_refused_values_are_not() {
        let whole = [
            ffi::SQLITE_BUSY,
            ffi::SQLITE_LOCKED,
            ffi::SQLITE_READONLY,
            ffi::SQLITE_IOERR,
            ffi::SQLITE_IOERR_FSYNC,
            ffi::SQLITE_CORRUPT,
            ffi::SQLITE_FULL,
            ffi::SQLITE_CANTOPEN,
        ];
        for code in whole {
            assert!(sqlite(code).is_catalog_wide(), "code {code}");
        }
        assert!(CatalogError::ReadOnly.is_catalog_wide());
        assert!(CatalogError::Io(std::io::Error::other("unplugged")).is_catalog_wide());
        assert!(CatalogError::OutOfTime.is_catalog_wide());

        let of_one_record = [
            ffi::SQLITE_CONSTRAINT,
            ffi::SQLITE_CONSTRAINT_UNIQUE,
            ffi::SQLITE_TOOBIG,
            ffi::SQLITE_MISMATCH,
            ffi::SQLITE_RANGE,
        ];
        for code in of_one_record {
            assert!(!sqlite(code).is_catalog_wide(), "code {code}");
        }
        assert!(!CatalogError::DuplicatePath("/r/a.jpg".to_string()).is_catalog_wide());
        assert!(!CatalogError::EmptyName.is_catalog_wide());
        assert!(!CatalogError::Sqlite(rusqlite::Error::QueryReturnedNoRows).is_catalog_wide());
    }
}
