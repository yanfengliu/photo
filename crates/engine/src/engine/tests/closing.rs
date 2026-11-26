//! Dropping the engine with an edit still open in Develop: what closing
//! does when the catalog cannot take the edit, and when a panic is what
//! drops the engine. (That the edit is kept when nothing is in the way is
//! a scenario, `open_edit::closing_the_application_keeps_the_edit_that_is_still_open`.)

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use photo_catalog::Catalog;
use photo_core::develop::Adj;
use photo_core::meta::PhotoId;
use tempfile::TempDir;

use super::{Engine, PATIENCE, engine_with, jpegs, logged};
use crate::jobs::Lane;

/// An engine with a photo open in Develop and its exposure moved, not
/// committed; the catalog's file, and the photo's.
struct Editing {
    engine: Engine,
    photo: PhotoId,
    catalog_path: PathBuf,
    file: PathBuf,
    _dirs: [TempDir; 2],
}

fn editing() -> Editing {
    let (files_dir, files) = jpegs(1, 96, 64);
    let (dir, mut engine, ids) = engine_with(&files, 1);
    engine.open_develop(ids[0]).unwrap();
    engine.adjust(Adj::Exposure, 0.8).unwrap();
    assert!(engine.develop().unwrap().is_editing());
    Editing {
        engine,
        photo: ids[0],
        catalog_path: dir.path().join("catalog").join("library.photocat"),
        file: files[0].clone(),
        _dirs: [files_dir, dir],
    }
}

fn saved_exposure(catalog_path: PathBuf, photo: PhotoId) -> f32 {
    let catalog = Catalog::open(catalog_path).unwrap();
    catalog.develop_settings(photo).unwrap().tone.exposure
}

#[test]
fn closing_saves_the_open_edit_before_it_waits_for_a_worker() {
    let Editing {
        mut engine,
        photo,
        catalog_path,
        _dirs,
        ..
    } = editing();
    // A worker that is stuck (in a call to a drive that has stalled, say)
    // and comes loose only once the edit is in the catalog, as another
    // connection reads it. So that closing in the other order does not
    // wait for ever, it gives up after a minute.
    let reader = Catalog::open(&catalog_path).unwrap();
    let (started, has_started) = mpsc::channel();
    let (saw, seen) = mpsc::channel();
    engine.submit(
        Lane::Urgent,
        None,
        None,
        move |_| {
            started.send(()).unwrap();
            let gives_up = Instant::now() + PATIENCE;
            let saved = || reader.develop_settings(photo).unwrap().tone.exposure != 0.0;
            while !saved() && Instant::now() < gives_up {
                std::thread::sleep(Duration::from_millis(5));
            }
            saw.send(saved()).unwrap();
            Box::new(|_: &mut Engine| false)
        },
        |_, _| false,
    );
    has_started.recv_timeout(PATIENCE).unwrap();

    drop(engine);
    assert!(
        seen.recv().unwrap(),
        "the edit was in the catalog while the worker was still stuck"
    );
    assert_eq!(saved_exposure(catalog_path, photo), 0.8);
}

#[test]
fn closing_does_not_wait_out_a_locked_catalog_and_names_the_edit_it_could_not_save() {
    let Editing {
        mut engine,
        photo,
        catalog_path,
        file,
        _dirs,
    } = editing();
    // Another connection is in the middle of a change, and stays there.
    let other = rusqlite::Connection::open(&catalog_path).unwrap();
    other.execute_batch("BEGIN IMMEDIATE").unwrap();
    engine.closing_patience = Duration::ZERO;
    let started = Instant::now();
    let lines = logged(|| drop(engine));
    let took = started.elapsed();
    // A wait for a locked catalog is five seconds unless closing shortens
    // it; told to have no patience, closing has none to wait out.
    assert!(
        took < Catalog::BUSY_TIMEOUT - Duration::from_secs(1),
        "closing took {took:?}"
    );
    let name = file.display().to_string();
    assert!(
        lines.iter().any(|line| line.contains(&name)),
        "the log names the photo whose edit could not be saved: {lines:?}"
    );

    other.execute_batch("ROLLBACK").unwrap();
    drop(other);
    assert_eq!(saved_exposure(catalog_path, photo), 0.0, "it was not saved");
}

#[test]
fn an_engine_dropped_by_a_panic_leaves_the_catalog_as_it_is() {
    let Editing {
        engine,
        photo,
        catalog_path,
        _dirs,
        ..
    } = editing();
    let unwound = catch_unwind(AssertUnwindSafe(move || {
        let _engine = engine;
        panic!("the interface fell over");
    }));
    assert!(unwound.is_err());
    assert_eq!(saved_exposure(catalog_path, photo), 0.0);
}
