//! Saves of metadata to sidecars, with the worker held so that the order
//! in which they are asked for, cancelled and run is fixed.

use photo_core::meta::{FileKind, Rating};

use super::{PATIENCE, engine_with, hold_worker, jpegs};
use crate::commands::Target;
use crate::sidecars::image_info;
use crate::tasks::TaskState;

#[test]
fn cancelling_the_newest_save_of_a_sidecar_leaves_the_older_one_to_reach_the_file() {
    let (_files_dir, files) = jpegs(1, 96, 64);
    let (_dir, mut engine, ids) = engine_with(&files, 1);
    let photo = || Target::Photos(vec![ids[0]]);
    // The one worker is busy, so both saves wait for it.
    let release = hold_worker(&mut engine);
    engine.set_rating(photo(), Rating::new(1)).unwrap();
    let older = engine.save_metadata_to_files(photo()).unwrap();
    engine.set_rating(photo(), Rating::new(5)).unwrap();
    let newer = engine.save_metadata_to_files(photo()).unwrap();
    assert!(engine.cancel_task(newer));
    release.send(()).unwrap();
    assert!(engine.wait_idle(PATIENCE));

    let state = |task| engine.task(task).map(|info| info.state.clone());
    assert_eq!(state(older), Some(TaskState::Finished));
    assert_eq!(state(newer), Some(TaskState::Cancelled));
    let sidecar = photo_xmp::sidecar_path(&files[0], FileKind::Raster);
    let kept = photo_xmp::read_sidecar(&sidecar, image_info(FileKind::Raster, 96, 64)).unwrap();
    assert_eq!(
        kept.map(|data| data.rating),
        Some(Rating::new(1)),
        "the save that was not cancelled is in the file"
    );
}
