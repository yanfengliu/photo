//! Deciding what a file is before anything is allowed to write to it.
//!
//! `Catalog::open` promises that a file it refuses (another application's
//! database, a catalog from a newer schema, something that is no database)
//! is left byte for byte as it was. So an existing file is first looked at
//! through a read-only connection, which cannot change the file even when
//! it is closed: unlike a writing connection, it never folds a write-ahead
//! log back into the main file.

use std::fs::File;
use std::io::{ErrorKind, Read};
use std::path::Path;

use rusqlite::{Connection, OpenFlags, ffi};

use crate::error::{CatalogError, Result};
use crate::migrate::{self, APPLICATION_ID};

/// The fixed text every SQLite database file begins with.
const MAGIC: &[u8; 16] = b"SQLite format 3\0";
/// Where the header of a database file keeps `PRAGMA user_version` and
/// `PRAGMA application_id`, as big-endian 32-bit integers.
const USER_VERSION_AT: usize = 60;
const APPLICATION_ID_AT: usize = 68;
const HEADER_LEN: usize = 100;

/// Fail unless the existing file at `path` is a catalog this build may open
/// (or a blank database it may initialise). Never writes to the file.
pub(crate) fn identify_file(path: &Path, supported: u32) -> Result<()> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let reader = Connection::open_with_flags(path, flags).map_err(translate)?;
    match migrate::probe(&reader, supported) {
        Ok(_) => Ok(()),
        // The file was being written, without a write-ahead log, when its
        // writer went down. Only a connection that may write can replay the
        // journal, and until that is done SQLite will not read the file. The
        // header is enough to tell whose file it is, so that a catalog of
        // ours goes on to be recovered and anything else stays untouched.
        Err(CatalogError::Sqlite(error))
            if extended_code(&error) == Some(ffi::SQLITE_READONLY_ROLLBACK) =>
        {
            let mut header = [0_u8; HEADER_LEN];
            match File::open(path).and_then(|mut file| file.read_exact(&mut header)) {
                Ok(()) => identify_header(&header, supported),
                Err(error) if error.kind() == ErrorKind::UnexpectedEof => {
                    Err(CatalogError::NotACatalog)
                }
                Err(error) => Err(CatalogError::Io(error)),
            }
        }
        Err(error) => Err(not_a_catalog(error)),
    }
}

/// What the first hundred bytes of a database file say about it.
fn identify_header(header: &[u8; HEADER_LEN], supported: u32) -> Result<()> {
    let number = |at: usize| {
        i32::from_be_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
    };
    if &header[..MAGIC.len()] != MAGIC || number(APPLICATION_ID_AT) != APPLICATION_ID {
        return Err(CatalogError::NotACatalog);
    }
    let found = u32::try_from(number(USER_VERSION_AT)).map_err(|_| CatalogError::NotACatalog)?;
    if found > supported {
        return Err(CatalogError::NewerSchema { found, supported });
    }
    Ok(())
}

/// Report SQLite's "file is not a database" as the typed error for it.
pub(crate) fn not_a_catalog(error: CatalogError) -> CatalogError {
    match error {
        CatalogError::Sqlite(error) => translate(error),
        other => other,
    }
}

fn translate(error: rusqlite::Error) -> CatalogError {
    // The low byte of an extended result code is the primary code.
    if extended_code(&error).map(|code| code & 0xff) == Some(ffi::SQLITE_NOTADB) {
        CatalogError::NotACatalog
    } else {
        CatalogError::Sqlite(error)
    }
}

/// SQLite's extended result code, wherever rusqlite put it.
fn extended_code(error: &rusqlite::Error) -> Option<i32> {
    match error {
        rusqlite::Error::SqliteFailure(error, _) | rusqlite::Error::SqlInputError { error, .. } => {
            Some(error.extended_code)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(application_id: i32, user_version: i32) -> [u8; HEADER_LEN] {
        let mut header = [0_u8; HEADER_LEN];
        header[..MAGIC.len()].copy_from_slice(MAGIC);
        header[USER_VERSION_AT..USER_VERSION_AT + 4].copy_from_slice(&user_version.to_be_bytes());
        header[APPLICATION_ID_AT..APPLICATION_ID_AT + 4]
            .copy_from_slice(&application_id.to_be_bytes());
        header
    }

    #[test]
    fn a_header_is_read_the_way_sqlite_writes_it() {
        // Let SQLite write a real header and read it back by offset.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("probe.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.pragma_update(None, "application_id", APPLICATION_ID)
                .unwrap();
            conn.pragma_update(None, "user_version", 7).unwrap();
        }
        let bytes = std::fs::read(&path).unwrap();
        let written: [u8; HEADER_LEN] = bytes[..HEADER_LEN].try_into().unwrap();
        let expected = header(APPLICATION_ID, 7);
        for field in [
            0..MAGIC.len(),
            USER_VERSION_AT..USER_VERSION_AT + 4,
            APPLICATION_ID_AT..APPLICATION_ID_AT + 4,
        ] {
            assert_eq!(written[field.clone()], expected[field]);
        }
        assert!(identify_header(&written, 7).is_ok());
        assert!(matches!(
            identify_header(&written, 6),
            Err(CatalogError::NewerSchema {
                found: 7,
                supported: 6
            })
        ));
    }

    #[test]
    fn only_a_catalog_of_a_known_version_passes() {
        assert!(identify_header(&header(APPLICATION_ID, 0), 1).is_ok());
        assert!(identify_header(&header(APPLICATION_ID, 1), 1).is_ok());
        assert!(matches!(
            identify_header(&header(APPLICATION_ID, 2), 1),
            Err(CatalogError::NewerSchema {
                found: 2,
                supported: 1
            })
        ));
        for foreign in [
            header(0, 0),
            header(0, 1),
            header(APPLICATION_ID + 1, 1),
            header(APPLICATION_ID, -1),
        ] {
            assert!(matches!(
                identify_header(&foreign, 1),
                Err(CatalogError::NotACatalog)
            ));
        }
        let mut not_sqlite = header(APPLICATION_ID, 1);
        not_sqlite[0] = b'X';
        assert!(matches!(
            identify_header(&not_sqlite, 1),
            Err(CatalogError::NotACatalog)
        ));
    }
}
