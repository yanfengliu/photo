//! Looking again at the source files of previews held in memory: how many
//! looks are out at once, and what becomes of a preview that was loaded
//! while its original was away. No test waits for the two seconds a
//! preview is trusted: each sets that time itself.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use photo_core::meta::PhotoId;
use tempfile::TempDir;

use super::{Engine, PATIENCE, engine_with, hold_worker, jpegs, next_event, open_in};
use crate::previews::{Preview, PreviewState};

/// Long enough that nothing in a test is looked at again unasked.
const NEVER: Duration = Duration::from_secs(3600);

/// Another picture, of another shape, to put under a photo's name.
fn replace(file: &Path) {
    let other = tempfile::tempdir().unwrap();
    let replacement = photo_testkit::write_jpeg(other.path(), "replacement.jpg", 64, 96);
    fs::copy(replacement, file).unwrap();
}

fn shape(preview: &Preview) -> (u32, u32) {
    (preview.image().width, preview.image().height)
}

#[test]
fn while_a_look_is_out_asking_again_sends_no_other() {
    let (_files_dir, files) = jpegs(3, 96, 64);
    let (_dir, mut engine, ids) = engine_with(&files, 1);
    for id in &ids {
        engine.thumbnail(*id);
    }
    assert!(engine.wait_idle(PATIENCE));
    replace(&files[2]);
    // Every preview is due to be looked at again, and the one worker does
    // not get to it: from here, a drive that has stalled looks the same.
    engine.previews.recheck_after = Duration::ZERO;
    let release = hold_worker(&mut engine);
    for _ in 0..50 {
        for id in &ids {
            assert!(
                engine.thumbnail(*id).ready().is_some(),
                "answered from memory"
            );
        }
    }
    assert_eq!(
        engine.pending_work().queued,
        1,
        "one look is out, however often and for however many previews it is asked"
    );
    assert_eq!(
        engine.previews.looks_waiting(),
        2,
        "and each of the other previews waits for the next look once"
    );

    release.send(()).unwrap();
    assert!(engine.wait_idle(PATIENCE));
    engine.previews.recheck_after = NEVER;
    assert!(engine.thumbnail(ids[0]).ready().is_some());
    assert!(
        matches!(engine.thumbnail(ids[2]), PreviewState::Pending),
        "what was due meanwhile was looked at in its turn"
    );
}

#[test]
fn a_preview_whose_stamp_is_dropped_leaves_the_looks_waiting_and_none_waits_twice() {
    let (_files_dir, files) = jpegs(4, 96, 64);
    let (_dir, mut engine, ids) = engine_with(&files, 1);
    let &[a, b, c, d] = ids.as_slice() else {
        panic!("four photos");
    };
    // Memory for two thumbnails, and stamps kept for two: browsing past two
    // photos here is what browsing past thousands is with the real limits.
    engine.thumbnail(a);
    assert!(engine.wait_idle(PATIENCE));
    let bytes = engine.thumbnail(a).ready().unwrap().bytes();
    engine.previews.limit_memory(2 * bytes);
    engine.previews.stamps_kept = 2;
    for photo in [a, b] {
        engine.thumbnail(photo);
        assert!(engine.wait_idle(PATIENCE));
    }

    // A look at a's file is out, held up as on a drive that has stalled,
    // and b waits for the next one.
    let release = hold_worker(&mut engine);
    engine.previews.recheck_after = Duration::ZERO;
    for photo in [a, b] {
        assert!(engine.thumbnail(photo).ready().is_some());
    }
    assert_eq!(engine.previews.looks_waiting(), 1);

    // Browsing on makes c and d, which push a and b out of memory, and
    // their stamps go the next time stamps are dropped.
    for photo in [c, d] {
        assert!(matches!(engine.thumbnail(photo), PreviewState::Pending));
    }
    release.send(()).unwrap();
    for _ in 0..3 {
        // The held job, then c's and d's previews; the look stays out.
        let event = next_event(&engine);
        engine.handle(event);
    }
    assert_eq!(
        engine.previews.looks_waiting(),
        0,
        "b's preview has left memory and its stamp is gone: it waits for no look"
    );
    let look = next_event(&engine);

    // b is browsed to again, made again, and asked for while the look is
    // still out.
    assert!(matches!(engine.thumbnail(b), PreviewState::Pending));
    let made = next_event(&engine);
    engine.handle(made);
    for _ in 0..10 {
        assert!(engine.thumbnail(b).ready().is_some());
    }
    assert_eq!(
        engine.previews.looks_waiting(),
        1,
        "b waits for the next look once"
    );

    engine.handle(look);
    assert!(engine.wait_idle(PATIENCE));
    assert_eq!(engine.previews.looks_waiting(), 0);
}

/// An engine whose memory holds a thumbnail that was loaded from its cache
/// file while the original was away, and where the original now is.
struct LoadedAway {
    engine: Engine,
    photo: PhotoId,
    /// Where the catalog says the original is.
    home: PathBuf,
    /// Where it is now.
    away: PathBuf,
    loaded: Preview,
    _dirs: [TempDir; 2],
}

fn a_thumbnail_loaded_while_its_original_was_away() -> LoadedAway {
    let (files_dir, files) = jpegs(1, 96, 64);
    let (dir, mut engine, ids) = engine_with(&files, 2);
    engine.thumbnail(ids[0]);
    assert!(engine.wait_idle(PATIENCE));
    drop(engine);
    let away = files_dir.path().join("away.jpg");
    fs::rename(&files[0], &away).unwrap();

    let mut engine = open_in(dir.path(), 2);
    engine.previews.recheck_after = NEVER;
    assert!(matches!(engine.thumbnail(ids[0]), PreviewState::Pending));
    assert!(engine.wait_idle(PATIENCE));
    let loaded = engine
        .thumbnail(ids[0])
        .ready()
        .cloned()
        .expect("the last preview written for the photo");
    assert_eq!(shape(&loaded), (96, 64));
    assert!(engine.summary(ids[0]).unwrap().missing);
    assert!(engine.is_idle(), "a preview just loaded is trusted for now");
    LoadedAway {
        engine,
        photo: ids[0],
        home: files[0].clone(),
        away,
        loaded,
        _dirs: [files_dir, dir],
    }
}

#[test]
fn a_preview_loaded_while_its_original_was_away_is_made_again_when_the_original_is_back_changed() {
    let LoadedAway {
        mut engine,
        photo,
        home,
        _dirs,
        ..
    } = a_thumbnail_loaded_while_its_original_was_away();
    // The drive is back, with another picture under the photo's name.
    replace(&home);
    engine.previews.recheck_after = Duration::ZERO;
    assert!(
        engine.thumbnail(photo).ready().is_some(),
        "answered from memory while a worker looks"
    );
    assert!(engine.wait_idle(PATIENCE));
    engine.previews.recheck_after = NEVER;
    assert!(matches!(engine.thumbnail(photo), PreviewState::Pending));
    assert!(engine.wait_idle(PATIENCE));
    let remade = engine.thumbnail(photo).ready().cloned().unwrap();
    assert_eq!(shape(&remade), (64, 96));
    assert!(!engine.summary(photo).unwrap().missing);
}

#[test]
fn a_preview_loaded_while_its_original_was_away_is_kept_when_the_original_is_back_as_it_was() {
    let LoadedAway {
        mut engine,
        photo,
        home,
        away,
        loaded,
        _dirs,
    } = a_thumbnail_loaded_while_its_original_was_away();
    let look = |engine: &mut Engine| {
        engine.previews.recheck_after = Duration::ZERO;
        let shown = engine.thumbnail(photo).ready().cloned();
        assert!(engine.wait_idle(PATIENCE));
        engine.previews.recheck_after = NEVER;
        shown.expect("answered from memory")
    };
    // Still away: the photo keeps its last preview and stays missing.
    look(&mut engine);
    assert!(engine.thumbnail(photo).ready().is_some());
    assert!(engine.summary(photo).unwrap().missing);

    // The drive is back with the file as it was.
    fs::rename(&away, &home).unwrap();
    look(&mut engine);
    assert!(
        !engine.summary(photo).unwrap().missing,
        "the original is back"
    );
    let kept = engine.thumbnail(photo).ready().cloned().unwrap();
    assert!(
        Arc::ptr_eq(kept.image(), loaded.image()),
        "and is what the preview was made from: nothing is made again"
    );
    assert!(engine.is_idle());

    // From here it is watched like any other: replaced, it is made again.
    replace(&home);
    look(&mut engine);
    assert!(matches!(engine.thumbnail(photo), PreviewState::Pending));
    assert!(engine.wait_idle(PATIENCE));
    let remade = engine.thumbnail(photo).ready().cloned().unwrap();
    assert_eq!(shape(&remade), (64, 96));
}
