//! An export whose guard could not see, when the export started, that the
//! original of another photo is in its way. The worker is held, so that
//! what changes on disk certainly changes between the start of the export
//! and the writing of the file.

use std::fs;
use std::path::Path;

use photo_export::{ExistingFile, ExportSettings, Folder, Location};

use super::{PATIENCE, engine_with, hold_worker};
use crate::commands::Target;

fn overwriting_into(folder: &Path) -> ExportSettings {
    ExportSettings {
        location: Location {
            folder: Folder::Specific(folder.to_path_buf()),
            subfolder: String::new(),
            existing: ExistingFile::Overwrite,
        },
        ..ExportSettings::default()
    }
}

/// The reviewer's case: the catalog reached the original through a link,
/// and the drive the link leads to is away when the export starts, so the
/// folder cannot be asked who it is and its path says nothing.
#[cfg(unix)]
#[test]
fn an_original_on_a_drive_that_was_away_when_the_export_started_is_not_replaced() {
    let base = tempfile::tempdir().unwrap();
    let actual = base.path().join("disk").join("actual");
    let photos = base.path().join("photos");
    let elsewhere = base.path().join("elsewhere");
    for folder in [&actual, &photos, &elsewhere] {
        fs::create_dir_all(folder).unwrap();
    }
    let link = photos.join("link");
    std::os::unix::fs::symlink(&actual, &link).unwrap();
    let original = photo_testkit::write_jpeg(&actual, "IMG_1.jpg", 96, 64);
    let other = photo_testkit::write_jpeg(&elsewhere, "IMG_1.jpg", 64, 96);
    let (_dir, mut engine, ids) = engine_with(&[link.join("IMG_1.jpg"), other], 1);
    let before = fs::read(&original).unwrap();

    // The drive is away: the link leads nowhere, and the folder the export
    // is to go into is not there.
    let away = base.path().join("disk").join("away");
    fs::rename(&actual, &away).unwrap();
    let release = hold_worker(&mut engine);
    let task = engine
        .start_export(Target::Photos(vec![ids[1]]), overwriting_into(&actual))
        .unwrap();
    // It is back before a worker gets to the photo.
    fs::rename(&away, &actual).unwrap();
    release.send(()).unwrap();
    assert!(engine.wait_idle(PATIENCE));

    assert!(
        fs::read(&original).unwrap() == before,
        "the original was written over"
    );
    let results = engine.export_results(task).unwrap();
    let reason = results[0]
        .result
        .as_ref()
        .expect_err("nothing may be written");
    assert_eq!(
        *reason,
        format!(
            "{} is the original of a photo in the catalog, which an export never writes over",
            actual.join("IMG_1.jpg").display()
        )
    );
}
