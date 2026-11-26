//! Tests that need to see inside the engine: what was decoded, and when a
//! preview is looked at again.

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use photo_core::meta::PhotoId;
use tempfile::TempDir;

use super::{Engine, EngineConfig};
use crate::develop::DevelopLoad;
use crate::import::{ImportMode, ImportRequest};
use crate::previews::PreviewState;
use crate::stamp::Stamp;

const PATIENCE: Duration = Duration::from_secs(60);

/// An engine on a new catalog with `files` imported in place.
fn engine_with(files: &[PathBuf], workers: usize) -> (TempDir, Engine, Vec<PhotoId>) {
    let dir = tempfile::tempdir().unwrap();
    let catalog = dir.path().join("catalog");
    fs::create_dir_all(&catalog).unwrap();
    let mut engine = Engine::open(EngineConfig {
        catalog_path: catalog.join("library.photocat"),
        previews_dir: catalog.join("library.previews"),
        worker_threads: workers,
        export_dir: dir.path().join("exports"),
    })
    .unwrap();
    let task = engine
        .start_import(ImportRequest {
            files: files.to_vec(),
            mode: ImportMode::Add,
            skip_suspected_duplicates: false,
            keywords: Vec::new(),
            read_sidecars: false,
        })
        .unwrap();
    assert!(engine.wait_idle(PATIENCE));
    let ids = engine.import_summary(task).unwrap().imported.clone();
    assert_eq!(ids.len(), files.len());
    (dir, engine, ids)
}

fn jpegs(count: usize, width: u32, height: u32) -> (TempDir, Vec<PathBuf>) {
    let dir = tempfile::tempdir().unwrap();
    let files = (0..count)
        .map(|index| {
            photo_testkit::write_jpeg(dir.path(), &format!("IMG_{index:04}.jpg"), width, height)
        })
        .collect();
    (dir, files)
}

#[test]
fn opening_photos_in_a_row_decodes_only_what_is_left_open() {
    let (_files_dir, files) = jpegs(6, 1200, 800);
    let (_dir, mut engine, ids) = engine_with(&files, 1);
    for id in &ids {
        engine.open_develop(*id).unwrap();
    }
    assert!(engine.wait_idle(PATIENCE));
    let session = engine.develop().unwrap();
    assert_eq!(session.photo(), ids[5]);
    assert_eq!(session.load_state(), &DevelopLoad::Decoded);
    let decoded = |file: &PathBuf| {
        let stamp = Stamp::read(file).unwrap();
        engine.sources.cached(file, stamp).is_some()
    };
    assert!(decoded(&files[5]), "the photo left open");
    let count = files.iter().filter(|file| decoded(file)).count();
    assert!(count <= 2, "{count} of 6 photos were decoded");
}

#[test]
fn a_failed_preview_is_tried_again_once_the_failure_is_old() {
    let (files_dir, files) = jpegs(1, 96, 64);
    let (_dir, mut engine, ids) = engine_with(&files, 2);
    let away = files_dir.path().join("away.jpg");
    fs::rename(&files[0], &away).unwrap();
    assert!(matches!(engine.thumbnail(ids[0]), PreviewState::Pending));
    assert!(engine.wait_idle(PATIENCE));
    assert!(matches!(engine.thumbnail(ids[0]), PreviewState::Failed(_)));

    // The drive is plugged back in.
    fs::rename(&away, &files[0]).unwrap();
    assert!(
        matches!(engine.thumbnail(ids[0]), PreviewState::Failed(_)),
        "a fresh failure is not tried again at every request"
    );
    assert!(engine.is_idle());
    engine.previews.retry_after = Duration::ZERO;
    assert!(
        matches!(engine.thumbnail(ids[0]), PreviewState::Pending),
        "an old one is"
    );
    assert!(engine.wait_idle(PATIENCE));
    assert!(engine.thumbnail(ids[0]).ready().is_some());
}

#[test]
fn a_preview_in_memory_is_made_again_when_its_file_was_replaced() {
    let (_files_dir, files) = jpegs(1, 96, 64);
    let (_dir, mut engine, ids) = engine_with(&files, 2);
    engine.thumbnail(ids[0]);
    assert!(engine.wait_idle(PATIENCE));
    let first = engine.thumbnail(ids[0]).ready().cloned().unwrap();
    assert_eq!((first.image().width, first.image().height), (96, 64));

    // Another picture is saved under the same name.
    let other = tempfile::tempdir().unwrap();
    let replacement = photo_testkit::write_jpeg(other.path(), "IMG_0000.jpg", 64, 96);
    fs::copy(&replacement, &files[0]).unwrap();
    assert!(engine.thumbnail(ids[0]).ready().is_some());
    assert!(
        engine.is_idle(),
        "a preview just made is not looked at again"
    );
    engine.previews.recheck_after = Duration::ZERO;
    assert!(
        engine.thumbnail(ids[0]).ready().is_some(),
        "answered from memory while a worker looks"
    );
    assert!(engine.wait_idle(PATIENCE));
    assert!(matches!(engine.thumbnail(ids[0]), PreviewState::Pending));
    assert!(engine.wait_idle(PATIENCE));
    let second = engine.thumbnail(ids[0]).ready().cloned().unwrap();
    assert_eq!((second.image().width, second.image().height), (64, 96));
}
