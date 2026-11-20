//! Opening, creating and refusing catalog files.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use photo_catalog::{Catalog, CatalogError};
use rusqlite::Connection;

fn catalog_path(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("library.photocat")
}

/// Byte-for-byte comparison that does not print the whole file on failure.
fn assert_unchanged(path: &Path, before: &[u8]) {
    assert!(
        fs::read(path).unwrap() == before,
        "{} was modified",
        path.display()
    );
}

/// Stand-in for a future build: stamp the file with a schema this build has
/// never heard of.
fn stamp_version(path: &Path, version: u32) {
    let conn = Connection::open(path).unwrap();
    conn.pragma_update(None, "user_version", version).unwrap();
}

#[test]
fn opening_a_missing_file_creates_a_catalog_at_the_current_schema() {
    let dir = tempfile::tempdir().unwrap();
    let path = catalog_path(&dir);
    assert!(!path.exists());

    let catalog = Catalog::open(&path).unwrap();
    assert!(path.exists());
    assert_eq!(catalog.schema_version().unwrap(), Catalog::SCHEMA_VERSION);
    assert_ne!(
        catalog.schema_version().unwrap(),
        0,
        "a blank file is version 0"
    );
    assert_eq!(catalog.check_integrity().unwrap(), Vec::<String>::new());
}

#[test]
fn an_in_memory_catalog_is_complete() {
    let catalog = common::memory();
    assert_eq!(catalog.schema_version().unwrap(), Catalog::SCHEMA_VERSION);
    assert_eq!(catalog.check_integrity().unwrap(), Vec::<String>::new());
}

#[test]
fn an_empty_file_is_initialised_as_a_catalog() {
    let dir = tempfile::tempdir().unwrap();
    let path = catalog_path(&dir);
    fs::write(&path, b"").unwrap();

    let catalog = Catalog::open(&path).unwrap();
    assert_eq!(catalog.schema_version().unwrap(), Catalog::SCHEMA_VERSION);
}

#[test]
fn a_file_backed_catalog_uses_a_write_ahead_log() {
    let dir = tempfile::tempdir().unwrap();
    let path = catalog_path(&dir);
    let _catalog = Catalog::open(&path).unwrap();

    // The journal mode is a property of the file, so any connection sees it.
    let other = Connection::open(&path).unwrap();
    let mode: String = other
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
}

#[test]
fn reopening_an_existing_catalog_keeps_what_was_written() {
    let dir = tempfile::tempdir().unwrap();
    let path = catalog_path(&dir);
    {
        let mut catalog = Catalog::open(&path).unwrap();
        catalog.set_setting("grid.thumbnail_size", "180").unwrap();
    }
    let catalog = Catalog::open(&path).unwrap();
    assert_eq!(
        catalog.setting("grid.thumbnail_size").unwrap().as_deref(),
        Some("180")
    );
    assert_eq!(catalog.schema_version().unwrap(), Catalog::SCHEMA_VERSION);
}

#[test]
fn a_catalog_from_a_newer_schema_is_refused_and_its_bytes_are_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let path = catalog_path(&dir);
    {
        let mut catalog = Catalog::open(&path).unwrap();
        catalog.set_setting("kept", "yes").unwrap();
    }
    let newer = Catalog::SCHEMA_VERSION + 41;
    stamp_version(&path, newer);
    let before = fs::read(&path).unwrap();

    match Catalog::open(&path) {
        Err(CatalogError::NewerSchema { found, supported }) => {
            assert_eq!(found, newer);
            assert_eq!(supported, Catalog::SCHEMA_VERSION);
        }
        other => panic!("expected NewerSchema, got {:?}", other.map(|_| "a catalog")),
    }
    assert_unchanged(&path, &before);

    // And it stays refused: the failed attempt did not downgrade the stamp.
    assert!(matches!(
        Catalog::open(&path),
        Err(CatalogError::NewerSchema { .. })
    ));
    assert_unchanged(&path, &before);
}

#[test]
fn a_newer_catalog_with_changes_still_in_its_log_is_refused_without_touching_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = catalog_path(&dir);
    drop(Catalog::open(&path).unwrap());

    // A "newer build" bumps the version, and the bump lives only in the
    // write-ahead log, not yet in the main file.
    let newer_build = Connection::open(&path).unwrap();
    newer_build
        .pragma_update(None, "wal_autocheckpoint", 0)
        .unwrap();
    newer_build
        .pragma_update(None, "user_version", Catalog::SCHEMA_VERSION + 1)
        .unwrap();

    // Copying the file and its log at this moment is what that build
    // crashing looks like: a catalog whose log was never folded back in.
    let crashed = dir.path().join("crashed.photocat");
    let log = |file: &Path| PathBuf::from(format!("{}-wal", file.display()));
    fs::copy(&path, &crashed).unwrap();
    fs::copy(log(&path), log(&crashed)).unwrap();
    let main_before = fs::read(&crashed).unwrap();
    let log_before = fs::read(log(&crashed)).unwrap();
    assert!(!log_before.is_empty());

    assert!(matches!(
        Catalog::open(&crashed),
        Err(CatalogError::NewerSchema { .. })
    ));
    // The pending log was not folded into the file, nor was it touched.
    assert_unchanged(&crashed, &main_before);
    assert_unchanged(&log(&crashed), &log_before);

    // The live catalog, with the newer build still attached, is refused too.
    let before = fs::read(&path).unwrap();
    assert!(matches!(
        Catalog::open(&path),
        Err(CatalogError::NewerSchema { .. })
    ));
    assert_unchanged(&path, &before);
}

#[test]
fn another_applications_database_is_refused_and_its_bytes_are_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("someone-elses.db");
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE theirs (id INTEGER PRIMARY KEY, note TEXT); INSERT INTO theirs (note) VALUES ('precious');")
            .unwrap();
    }
    let before = fs::read(&path).unwrap();

    assert!(matches!(
        Catalog::open(&path),
        Err(CatalogError::NotACatalog)
    ));
    assert_unchanged(&path, &before);
}

#[test]
fn a_file_that_is_not_a_database_is_refused_and_its_bytes_are_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("holiday.jpg");
    let before =
        b"\xFF\xD8\xFF\xE0 definitely not sqlite, but long enough to look like a file".repeat(64);
    fs::write(&path, &before).unwrap();

    assert!(matches!(
        Catalog::open(&path),
        Err(CatalogError::NotACatalog)
    ));
    assert_unchanged(&path, &before);
}

#[test]
fn a_catalog_cannot_be_created_inside_a_directory_that_does_not_exist() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing").join("library.photocat");
    assert!(matches!(Catalog::open(&path), Err(CatalogError::Sqlite(_))));
    assert!(!path.exists());
}

#[test]
fn settings_are_a_string_key_value_store() {
    let mut catalog = common::memory();
    assert_eq!(catalog.setting("ui.module").unwrap(), None);

    catalog.set_setting("ui.module", "library").unwrap();
    catalog.set_setting("ui.module", "develop").unwrap();
    catalog.set_setting("ui.empty", "").unwrap();
    assert_eq!(
        catalog.setting("ui.module").unwrap().as_deref(),
        Some("develop")
    );
    assert_eq!(catalog.setting("ui.empty").unwrap().as_deref(), Some(""));

    assert!(catalog.remove_setting("ui.module").unwrap());
    assert!(!catalog.remove_setting("ui.module").unwrap());
    assert_eq!(catalog.setting("ui.module").unwrap(), None);
}

#[test]
fn import_runs_are_listed_newest_first_with_their_photo_counts() {
    let mut catalog = common::memory();
    assert!(catalog.latest_import().unwrap().is_none());
    assert!(catalog.imports().unwrap().is_empty());

    let first = catalog.begin_import().unwrap();
    catalog
        .insert_photos(
            first,
            &[common::raster("/p/a.jpg"), common::raster("/p/b.jpg")],
        )
        .unwrap();
    let second = catalog.begin_import().unwrap();
    catalog
        .insert_photo(second, &common::raster("/p/c.jpg"))
        .unwrap();

    let runs = catalog.imports().unwrap();
    assert_eq!(
        runs.iter().map(|run| run.id).collect::<Vec<_>>(),
        [second, first]
    );
    assert_eq!(
        runs.iter().map(|run| run.photo_count).collect::<Vec<_>>(),
        [1, 2]
    );
    assert!(runs[0].started_at >= runs[1].started_at);
    assert!(
        runs[1].started_at > 1_600_000_000,
        "time stamps are Unix seconds"
    );
    assert_eq!(catalog.latest_import().unwrap().unwrap().id, second);

    // A run that adds nothing is listed, but is not the latest import.
    let empty = catalog.begin_import().unwrap();
    let runs = catalog.imports().unwrap();
    assert_eq!(
        runs.iter()
            .map(|run| (run.id, run.photo_count))
            .collect::<Vec<_>>(),
        [(empty, 0), (second, 1), (first, 2)]
    );
    assert_eq!(catalog.latest_import().unwrap().unwrap().id, second);
}
