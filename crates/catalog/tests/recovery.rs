//! Catalogs whose last writer went down in the middle of a write.
//!
//! A catalog normally keeps a write-ahead log, but where the file system
//! cannot support one (some network shares) SQLite falls back to a rollback
//! journal. A write that is interrupted then leaves a "hot" journal beside a
//! half-written file, and the file cannot even be read until a connection
//! that may write has replayed that journal.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use photo_catalog::{Catalog, CatalogError};
use rusqlite::{Connection, OpenFlags};

fn journal(file: &Path) -> PathBuf {
    PathBuf::from(format!("{}-journal", file.display()))
}

fn assert_unchanged(path: &Path, before: &[u8]) {
    assert!(
        fs::read(path).unwrap() == before,
        "{} was modified",
        path.display()
    );
}

/// Copy `source` as it looks in the middle of a large write that never
/// finished, and return the copy's path. The write is done by a plain SQLite
/// connection using a rollback journal, in a table called `settings`.
fn interrupted_copy(source: &Path, name: &str) -> PathBuf {
    let writer = Connection::open(source).unwrap();
    let mode: String = writer
        .pragma_update_and_check(None, "journal_mode", "DELETE", |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "delete");
    // Without syncing, the journal is valid from its first byte; and a tiny
    // cache forces half-finished pages out into the database file itself.
    writer.pragma_update(None, "synchronous", "OFF").unwrap();
    writer.pragma_update(None, "cache_size", 2).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    writer
        .execute("UPDATE settings SET value = 'torn'", [])
        .unwrap();
    let filler = "x".repeat(900);
    for index in 0..400 {
        writer
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)",
                (format!("bulk-{index}"), &filler),
            )
            .unwrap();
    }

    let copy = source.with_file_name(name);
    fs::copy(source, &copy).unwrap();
    fs::copy(journal(source), journal(&copy)).unwrap();
    writer.execute_batch("ROLLBACK").unwrap();

    // Prove the copy really is in that state: half-written and unreadable
    // for a connection that may not write.
    assert!(fs::read(&copy).unwrap() != fs::read(source).unwrap());
    let reader = Connection::open_with_flags(&copy, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let refused = reader
        .query_row("SELECT COUNT(*) FROM settings", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap_err();
    assert_eq!(
        refused.sqlite_error().map(|error| error.extended_code),
        Some(rusqlite::ffi::SQLITE_READONLY_ROLLBACK)
    );
    copy
}

#[test]
fn an_interrupted_write_is_rolled_back_when_the_catalog_is_opened() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.photocat");
    {
        let mut catalog = Catalog::open(&path).unwrap();
        catalog.set_setting("kept", "yes").unwrap();
        let run = catalog.begin_import().unwrap();
        catalog
            .insert_photos(
                run,
                &[common::raster("/p/a.jpg"), common::raster("/p/b.jpg")],
            )
            .unwrap();
    }
    let crashed = interrupted_copy(&path, "crashed.photocat");

    let catalog = Catalog::open(&crashed).unwrap();

    // Back to the last completed change: nothing of the torn write remains.
    assert_eq!(catalog.setting("kept").unwrap().as_deref(), Some("yes"));
    assert_eq!(catalog.setting("bulk-0").unwrap(), None);
    assert_eq!(catalog.stats().unwrap().photos, 2);
    assert_eq!(catalog.check_integrity().unwrap(), Vec::<String>::new());
    assert!(
        !journal(&crashed).exists(),
        "the journal was replayed and removed"
    );
}

#[test]
fn an_interrupted_write_in_someone_elses_database_is_left_exactly_as_it_is() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("theirs.db");
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT);
             INSERT INTO settings VALUES ('precious', 'data');",
        )
        .unwrap();
    }
    let crashed = interrupted_copy(&path, "theirs-crashed.db");
    let (file, log) = (
        fs::read(&crashed).unwrap(),
        fs::read(journal(&crashed)).unwrap(),
    );

    assert!(matches!(
        Catalog::open(&crashed),
        Err(CatalogError::NotACatalog)
    ));

    // Not recovered, not touched: that is for its own application to do.
    assert_unchanged(&crashed, &file);
    assert_unchanged(&journal(&crashed), &log);
}

#[test]
fn an_interrupted_write_in_a_newer_catalog_is_left_exactly_as_it_is() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.photocat");
    drop(Catalog::open(&path).unwrap());
    {
        let newer_build = Connection::open(&path).unwrap();
        newer_build
            .pragma_update(None, "user_version", Catalog::SCHEMA_VERSION + 1)
            .unwrap();
    }
    let crashed = interrupted_copy(&path, "newer-crashed.photocat");
    let (file, log) = (
        fs::read(&crashed).unwrap(),
        fs::read(journal(&crashed)).unwrap(),
    );

    assert!(matches!(
        Catalog::open(&crashed),
        Err(CatalogError::NewerSchema { .. })
    ));
    assert_unchanged(&crashed, &file);
    assert_unchanged(&journal(&crashed), &log);
}
