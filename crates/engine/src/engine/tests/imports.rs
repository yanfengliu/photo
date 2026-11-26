//! Copy imports whose results are brought in one at a time, by hand, so
//! that a finished copy is waiting for its record at a chosen moment: when
//! the engine closes, and when the catalog cannot be written.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use photo_catalog::Catalog;
use tempfile::TempDir;

use super::{
    Engine, PATIENCE, engine_with, hold_worker, jpegs, logged, next_event, release_at_shutdown,
};
use crate::import::{CopyLayout, ImportMode, ImportRequest};

fn copying(files: Vec<PathBuf>, destination: &Path) -> ImportRequest {
    ImportRequest {
        files,
        mode: ImportMode::Copy {
            destination: destination.to_path_buf(),
            layout: CopyLayout::OneFolder,
        },
        skip_suspected_duplicates: false,
        keywords: Vec::new(),
        read_sidecars: false,
    }
}

fn catalog_path(dir: &TempDir) -> PathBuf {
    dir.path().join("catalog").join("library.photocat")
}

/// A copy import of two files on one worker, stopped where the first
/// file's copy is finished and waits for its record, and the second file's
/// copy is queued behind a worker that is busy until the engine shuts it
/// down.
struct Halfway {
    engine: Engine,
    catalog_path: PathBuf,
    /// The finished copy that has no record yet.
    copy: PathBuf,
    _dirs: [TempDir; 3],
}

fn halfway_through_a_copy_import() -> Halfway {
    let (files_dir, files) = jpegs(2, 96, 64);
    let (dir, mut engine, _) = engine_with(&[], 1);
    let destination = tempfile::tempdir().unwrap();
    let task = engine
        .start_import(copying(files, destination.path()))
        .unwrap();
    // The one worker takes its jobs in the order they were queued: the
    // reading of the first file, the reading of the second, and then the
    // copy of the first, which is queued when its reading is brought in.
    let first_read = next_event(&engine);
    engine.handle(first_read);
    let second_read = next_event(&engine);
    let first_copied = next_event(&engine);
    engine.handle(first_copied);
    // From here the worker is busy with something else, so the copy of the
    // second file stays queued.
    let release = hold_worker(&mut engine);
    release_at_shutdown(&mut engine, release);
    engine.handle(second_read);

    let copy = destination.path().join("IMG_0000.jpg");
    assert!(copy.is_file(), "the first file is copied");
    assert!(!destination.path().join("IMG_0001.jpg").exists());
    assert!(
        engine.catalog.find_by_path(&copy).unwrap().is_none(),
        "and waits for its record"
    );
    assert!(engine.import_summary(task).unwrap().imported.is_empty());
    Halfway {
        engine,
        catalog_path: catalog_path(&dir),
        copy,
        _dirs: [files_dir, dir, destination],
    }
}

#[test]
fn closing_the_app_during_a_copy_import_leaves_no_file_without_a_record() {
    let Halfway {
        engine,
        catalog_path,
        copy,
        _dirs,
    } = halfway_through_a_copy_import();
    drop(engine);

    let catalog = Catalog::open(catalog_path).unwrap();
    assert!(
        catalog.find_by_path(&copy).unwrap().is_some(),
        "{} was copied and has no record",
        copy.display()
    );
    let folder = copy.parent().unwrap();
    assert_eq!(
        std::fs::read_dir(folder).unwrap().count(),
        1,
        "the copy that was only queued was never made"
    );
}

#[test]
fn closing_does_not_wait_out_a_catalog_that_another_connection_has_locked() {
    let Halfway {
        mut engine,
        catalog_path,
        copy,
        _dirs,
    } = halfway_through_a_copy_import();
    // Another connection is in the middle of a change, and stays there.
    let other = rusqlite::Connection::open(&catalog_path).unwrap();
    other.execute_batch("BEGIN IMMEDIATE").unwrap();
    engine.closing_patience = Duration::ZERO;
    let started = Instant::now();
    let lines = logged(|| drop(engine));
    let took = started.elapsed();
    // Every wait for a locked catalog is five seconds unless closing
    // shortens it; told to have no patience, closing has none to wait out.
    assert!(
        took < Catalog::BUSY_TIMEOUT - Duration::from_secs(1),
        "closing took {took:?}"
    );

    other.execute_batch("ROLLBACK").unwrap();
    drop(other);
    let catalog = Catalog::open(catalog_path).unwrap();
    assert!(catalog.find_by_path(&copy).unwrap().is_none());
    assert!(
        copy.is_file(),
        "what could not be recorded is left where it is"
    );
    let name = copy.display().to_string();
    assert!(
        lines.iter().any(|line| line.contains(&name)),
        "and named in the log: {lines:?}"
    );
}

#[test]
fn a_batch_the_catalog_cannot_take_is_reported_and_its_copies_are_removed() {
    let (_files_dir, files) = jpegs(2, 96, 64);
    let (dir, mut engine, _) = engine_with(&[], 1);
    let destination = tempfile::tempdir().unwrap();
    let task = engine
        .start_import(copying(files, destination.path()))
        .unwrap();
    // Both files are read and copied, and the copies' results wait.
    for _ in 0..2 {
        let read = next_event(&engine);
        engine.handle(read);
    }
    let copied = [next_event(&engine), next_event(&engine)];
    let copies = ["IMG_0000.jpg", "IMG_0001.jpg"].map(|name| destination.path().join(name));
    assert!(copies.iter().all(|copy| copy.is_file()));
    // Another connection is in the middle of a change, and stays there.
    // The engine's catalog is told not to wait for it, as it would for
    // five seconds.
    let other = rusqlite::Connection::open(catalog_path(&dir)).unwrap();
    other.execute_batch("BEGIN IMMEDIATE").unwrap();
    engine.catalog.set_busy_timeout(Duration::ZERO).unwrap();
    for event in copied {
        engine.handle(event);
    }
    assert!(engine.wait_idle(PATIENCE));

    let summary = engine.import_summary(task).unwrap();
    assert!(summary.finished && summary.imported.is_empty());
    let reported: Vec<&PathBuf> = summary.errors.iter().map(|issue| &issue.path).collect();
    assert_eq!(reported, [&copies[0], &copies[1]], "each file is reported");
    assert!(
        copies.iter().all(|copy| !copy.exists()),
        "and no copy is left without a record"
    );
    other.execute_batch("ROLLBACK").unwrap();
}
