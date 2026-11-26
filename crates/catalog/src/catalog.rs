//! Opening a catalog and the plumbing every other module builds on.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use photo_core::meta::CollectionId;
use rusqlite::config::DbConfig;
use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior};

use crate::error::{CatalogError, Result};
use crate::identify::{identify_file, not_a_catalog};
use crate::migrate::{self, MIGRATIONS};
use crate::text;

/// Prepared statements kept ready for reuse. The catalog runs a few dozen
/// distinct statements over and over; none should be compiled twice.
const STATEMENT_CACHE_SIZE: usize = 128;

/// A user's library: every photo record, its organisation and its edits, in
/// one SQLite file.
///
/// A `Catalog` is used from one thread. Reads take `&self`; every mutation
/// takes `&mut self`, runs inside a single transaction and is durable once
/// the call returns `Ok`. An `Err` means nothing was changed.
#[derive(Debug)]
pub struct Catalog {
    pub(crate) conn: Connection,
    pub(crate) quick_collection: CollectionId,
    clock: Clock,
    read_only: bool,
}

impl Catalog {
    /// The schema version this build reads and writes.
    pub const SCHEMA_VERSION: u32 = migrate::latest(MIGRATIONS);

    /// How long a call waits for another connection that has the catalog
    /// locked, unless [`Catalog::set_busy_timeout`] says otherwise.
    pub const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

    /// Open the catalog at `path`, creating it if the file is missing or
    /// empty and upgrading it if it was written by an older schema.
    ///
    /// The file is identified without writing to it first. A catalog written
    /// by a newer schema fails with [`CatalogError::NewerSchema`], and a file
    /// that is not a Photo catalog with [`CatalogError::NotACatalog`]; in
    /// both cases the file's bytes are left exactly as they were.
    ///
    /// A catalog that was being written when the application or the machine
    /// went down is recovered here, to the state of its last completed
    /// change.
    ///
    /// A write-protected file opens for reading only (see
    /// [`Catalog::is_read_only`]), unless it would first have to be set up
    /// or upgraded, which fails with [`CatalogError::ReadOnly`].
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = plain_path(path.as_ref());
        let has_content = std::fs::metadata(&path).is_ok_and(|file| file.len() > 0);
        if has_content {
            identify_file(&path, Self::SCHEMA_VERSION)?;
        }
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        Self::initialise(Connection::open_with_flags(&path, flags)?)
    }

    /// Open a private catalog that lives in memory and vanishes when dropped.
    pub fn open_in_memory() -> Result<Self> {
        Self::initialise(Connection::open_in_memory()?)
    }

    /// True if the catalog's file is write-protected, so that every change
    /// fails with [`CatalogError::ReadOnly`] (and changes nothing); reading
    /// works as usual.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// How long a call waits for another connection that has the catalog
    /// locked before it fails ([`CatalogError::is_catalog_wide`] is then
    /// true of its error). A catalog waits [`Catalog::BUSY_TIMEOUT`] until
    /// told otherwise; with zero it does not wait at all.
    pub fn set_busy_timeout(&self, timeout: Duration) -> Result<()> {
        Ok(self.conn.busy_timeout(timeout)?)
    }

    /// The schema version stored in the file (`PRAGMA user_version`). After a
    /// successful open this equals [`Catalog::SCHEMA_VERSION`].
    pub fn schema_version(&self) -> Result<u32> {
        let version: i64 = self
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))?;
        Ok(crate::codec::to_u32(version))
    }

    /// Run SQLite's own consistency checks and return the problems found; an
    /// empty list means the file is structurally sound and no row refers to a
    /// row that does not exist.
    pub fn check_integrity(&self) -> Result<Vec<String>> {
        let mut problems = Vec::new();
        self.conn.pragma_query(None, "integrity_check", |row| {
            let line: String = row.get(0)?;
            if line != "ok" {
                problems.push(line);
            }
            Ok(())
        })?;
        self.conn.pragma_query(None, "foreign_key_check", |row| {
            let table: String = row.get(0)?;
            let rowid: Option<i64> = row.get(1)?;
            let parent: String = row.get(2)?;
            problems.push(format!(
                "{table} row {rowid:?} refers to a missing {parent} row"
            ));
            Ok(())
        })?;
        Ok(problems)
    }

    fn initialise(mut conn: Connection) -> Result<Self> {
        conn.busy_timeout(Self::BUSY_TIMEOUT)?;
        // Checked again on the connection that will write, before the first
        // statement that could change anything (switching the journal mode
        // rewrites the file header).
        let found = migrate::probe(&conn, Self::SCHEMA_VERSION).map_err(not_a_catalog)?;
        // SQLite opens a write-protected file for reading only. Such a file
        // can be read as it is, but not set up or upgraded.
        let read_only = conn.is_readonly(rusqlite::MAIN_DB)?;
        if read_only && found != migrate::Probe::Catalog(Self::SCHEMA_VERSION) {
            return Err(CatalogError::ReadOnly);
        }
        if !read_only {
            let mode: String =
                conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))?;
            if !matches!(mode.as_str(), "wal" | "memory") {
                log::warn!("catalog could not switch to a write-ahead log; journal mode is {mode}");
            }
        }
        // A commit is on disk before the call that made it returns. On macOS
        // an ordinary fsync only reaches the drive's cache, so ask for the
        // real thing (these two are no-ops everywhere else); without it a
        // power cut could lose acknowledged changes or, during a checkpoint,
        // tear the file.
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.pragma_update(None, "fullfsync", true)?;
        conn.pragma_update(None, "checkpoint_fullfsync", true)?;
        conn.pragma_update(None, "foreign_keys", true)?;
        // A catalog file may come from anywhere. Do not let SQL (ours or a
        // schema object smuggled into the file) corrupt the database on
        // purpose or call functions that are not marked harmless.
        conn.pragma_update(None, "trusted_schema", false)?;
        conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
        conn.set_prepared_statement_cache_capacity(STATEMENT_CACHE_SIZE);
        text::register_functions(&conn)?;

        migrate::migrate(&mut conn, MIGRATIONS).map_err(not_a_catalog)?;

        let enforced: bool = conn.pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
        if !enforced {
            return Err(CatalogError::ForeignKeysUnavailable);
        }
        let quick: i64 =
            conn.query_row("SELECT id FROM collections WHERE kind = 3", [], |row| {
                row.get(0)
            })?;
        let newest: i64 = conn.query_row(
            "SELECT COALESCE(MAX(edited_at), 0) FROM photos",
            [],
            |row| row.get(0),
        )?;
        Ok(Self {
            conn,
            quick_collection: CollectionId(quick),
            clock: Clock { last: newest },
            read_only,
        })
    }

    /// Run `work` inside one immediate transaction, committing if it returns
    /// `Ok` and rolling back otherwise. `work` also receives the time stamp
    /// of this write in microseconds; stamps strictly increase from one write
    /// to the next, whatever the system clock does. Fails with
    /// [`CatalogError::ReadOnly`], before anything runs, on a write-protected
    /// catalog.
    pub(crate) fn write<T>(
        &mut self,
        work: impl FnOnce(&Transaction<'_>, i64) -> Result<T>,
    ) -> Result<T> {
        if self.read_only {
            return Err(CatalogError::ReadOnly);
        }
        let now = self.clock.tick();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let value = work(&tx, now)?;
        tx.commit()?;
        Ok(value)
    }
}

/// A clock that never repeats or goes backwards, so "edited later" is always
/// a larger number even if the system clock is adjusted.
#[derive(Debug)]
struct Clock {
    last: i64,
}

impl Clock {
    fn tick(&mut self) -> i64 {
        let wall = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|elapsed| i64::try_from(elapsed.as_micros()).ok())
            .unwrap_or(0);
        self.last = wall.max(self.last.saturating_add(1));
        self.last
    }
}

/// SQLite gives special meaning to names such as `:memory:` and `file:...`.
/// Anchoring a relative path to `.` makes every path an ordinary file path.
fn plain_path(path: &Path) -> PathBuf {
    if path.is_relative() {
        Path::new(".").join(path)
    } else {
        path.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_strictly_increases_even_when_wall_time_is_behind() {
        let far_future = i64::MAX / 2;
        let mut clock = Clock { last: far_future };
        assert_eq!(clock.tick(), far_future + 1);
        assert_eq!(clock.tick(), far_future + 2);

        let mut clock = Clock { last: 0 };
        let first = clock.tick();
        assert!(first > 1_600_000_000_000_000, "wall time in microseconds");
        assert!(clock.tick() > first);
    }

    #[test]
    fn a_file_backed_catalog_runs_with_its_safety_settings_on() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = Catalog::open(dir.path().join("library.photocat")).unwrap();
        let pragma = |name: &str| -> String {
            catalog
                .conn
                .pragma_query_value(None, name, |row| row.get::<_, rusqlite::types::Value>(0))
                .map(|value| match value {
                    rusqlite::types::Value::Integer(number) => number.to_string(),
                    rusqlite::types::Value::Text(text) => text,
                    other => format!("{other:?}"),
                })
                .unwrap()
        };
        assert_eq!(pragma("journal_mode"), "wal");
        assert_eq!(
            pragma("synchronous"),
            "2",
            "FULL: a commit is on disk before the call returns"
        );
        assert_eq!(pragma("fullfsync"), "1");
        assert_eq!(pragma("checkpoint_fullfsync"), "1");
        assert_eq!(pragma("foreign_keys"), "1");
        assert_eq!(
            pragma("application_id"),
            migrate::APPLICATION_ID.to_string()
        );
        assert_eq!(pragma("user_version"), Catalog::SCHEMA_VERSION.to_string());
        assert_eq!(pragma("trusted_schema"), "0");
        assert!(
            catalog
                .conn
                .db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE)
                .unwrap()
        );
        // Defensive mode refuses SQL whose only use is to damage the file.
        assert!(
            catalog
                .conn
                .pragma_update(None, "writable_schema", true)
                .is_err()
                || pragma("writable_schema") == "0"
        );
    }

    #[test]
    fn the_clock_resumes_after_the_newest_edit_stored_in_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.photocat");
        // An edit stamped in the far future, as if the system clock had been
        // wrong (or has since been set back).
        let far_future = i64::MAX / 2;
        {
            let mut catalog = Catalog::open(&path).unwrap();
            let run = catalog.begin_import().unwrap();
            let photo = crate::NewPhoto::new("/clock/a.jpg", photo_core::meta::FileKind::Raster);
            catalog.insert_photo(run, &photo).unwrap();
            catalog
                .conn
                .execute("UPDATE photos SET edited_at = ?1", [far_future])
                .unwrap();
        }
        let mut catalog = Catalog::open(&path).unwrap();
        assert_eq!(catalog.write(|_, now| Ok(now)).unwrap(), far_future + 1);
        assert_eq!(catalog.write(|_, now| Ok(now)).unwrap(), far_future + 2);
    }

    #[test]
    fn relative_paths_are_anchored_so_sqlite_cannot_reinterpret_them() {
        assert_eq!(plain_path(Path::new(":memory:")), Path::new("./:memory:"));
        assert_eq!(
            plain_path(Path::new("file:x?mode=memory")),
            Path::new("./file:x?mode=memory")
        );
        let dir = tempfile::tempdir().unwrap();
        let absolute = dir.path().join("library.photocat");
        assert_eq!(plain_path(&absolute), absolute);
    }

    #[test]
    fn the_integrity_check_reports_a_row_that_refers_to_nothing() {
        let mut catalog = Catalog::open_in_memory().unwrap();
        let run = catalog.begin_import().unwrap();
        let photo = crate::NewPhoto::new("/check/a.jpg", photo_core::meta::FileKind::Raster);
        catalog.insert_photo(run, &photo).unwrap();
        assert!(catalog.check_integrity().unwrap().is_empty());

        // Something only a damaged or tampered file can contain: a history
        // step for a photo that does not exist.
        catalog
            .conn
            .pragma_update(None, "foreign_keys", false)
            .unwrap();
        catalog
            .conn
            .execute(
                "INSERT INTO history (photo_id, label, settings, settings_format, created_at)
                 VALUES (9999, 'x', '{}', 1, 0)",
                [],
            )
            .unwrap();
        catalog
            .conn
            .pragma_update(None, "foreign_keys", true)
            .unwrap();

        let problems = catalog.check_integrity().unwrap();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("history") && problems[0].contains("photos"));
    }

    #[test]
    fn a_time_stamp_at_the_end_of_time_does_not_overflow_the_clock() {
        let mut clock = Clock { last: i64::MAX };
        assert_eq!(clock.tick(), i64::MAX);
    }

    #[test]
    fn a_catalog_waits_for_a_lock_as_long_as_it_is_told_to() {
        let catalog = Catalog::open_in_memory().unwrap();
        // What SQLite itself says it will wait, in milliseconds.
        let waits = || -> i64 {
            catalog
                .conn
                .pragma_query_value(None, "busy_timeout", |row| row.get(0))
                .unwrap()
        };
        assert_eq!(waits(), 5000, "until told otherwise");
        catalog
            .set_busy_timeout(Duration::from_millis(250))
            .unwrap();
        assert_eq!(waits(), 250);
        catalog.set_busy_timeout(Duration::ZERO).unwrap();
        assert_eq!(waits(), 0, "not at all");
    }
}
