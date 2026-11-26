//! Commands that reach the photo being edited while an edit of it is still
//! open in Develop. Whatever they do to the photo, the edit must be there
//! to come back to; and an edit of a photo they do not reach must be left
//! as it is, in progress.

use photo_core::develop::Adj;
use photo_core::meta::{FileKind, PhotoId, Rating};
use photo_engine::Target;
use photo_testkit::TempLibrary;
use photo_xmp::ImageInfo;

use crate::common::{Studio, open_rendered};

const FIT: photo_engine::CanvasView = photo_engine::CanvasView::Fit {
    width: 320,
    height: 240,
};

fn studio() -> (TempLibrary, Studio) {
    let library = TempLibrary::new();
    let mut studio = Studio::new();
    studio.import_library(&library);
    (library, studio)
}

fn saved_exposure(studio: &Studio, photo: PhotoId) -> f32 {
    studio.engine.develop_settings(photo).unwrap().tone.exposure
}

fn steps(studio: &Studio, photo: PhotoId) -> usize {
    studio.engine.history(photo).unwrap().len()
}

/// Open `photo` and move its exposure without letting go.
fn edit_in_progress(studio: &mut Studio, photo: PhotoId) {
    open_rendered(studio, photo, FIT);
    studio.engine.adjust(Adj::Exposure, 0.8).unwrap();
    assert!(studio.engine.develop().unwrap().is_editing());
    assert_eq!(saved_exposure(studio, photo), 0.0, "not committed yet");
}

#[test]
fn removing_the_photo_being_edited_keeps_the_edit_that_is_still_open() {
    let (_library, mut studio) = studio();
    let dawn = studio.id("dawn.jpg");
    edit_in_progress(&mut studio, dawn);
    let before = steps(&studio, dawn);

    studio
        .engine
        .remove_from_catalog(Target::Photos(vec![dawn]))
        .unwrap();
    assert!(studio.engine.summary(dawn).is_none());
    assert!(studio.engine.develop().is_none(), "its session is over");

    assert!(studio.engine.undo().unwrap());
    assert_eq!(
        saved_exposure(&studio, dawn),
        0.8,
        "the photo comes back as the user was looking at it"
    );
    assert_eq!(
        steps(&studio, dawn),
        before + 1,
        "with the edit as one step"
    );
}

#[test]
fn removing_a_master_keeps_the_edit_still_open_on_its_virtual_copy() {
    let (_library, mut studio) = studio();
    let dawn = studio.id("dawn.jpg");
    let copy = studio
        .engine
        .create_virtual_copies(Target::Photos(vec![dawn]))
        .unwrap()[0];
    edit_in_progress(&mut studio, copy);

    // The copy goes where its master goes.
    studio
        .engine
        .remove_from_catalog(Target::Photos(vec![dawn]))
        .unwrap();
    assert!(studio.engine.summary(copy).is_none());
    assert!(studio.engine.develop().is_none());

    assert!(studio.engine.undo().unwrap());
    assert_eq!(saved_exposure(&studio, copy), 0.8);
    assert_eq!(saved_exposure(&studio, dawn), 0.0, "the master had no edit");
}

#[test]
fn removing_other_photos_leaves_an_edit_in_progress_alone() {
    let (_library, mut studio) = studio();
    let (dawn, lake) = (studio.id("dawn.jpg"), studio.id("lake.png"));
    edit_in_progress(&mut studio, dawn);
    let before = steps(&studio, dawn);

    studio
        .engine
        .remove_from_catalog(Target::Photos(vec![lake]))
        .unwrap();
    studio.settle();
    let session = studio.engine.develop().unwrap();
    assert!(session.is_editing(), "the edit is still in progress");
    assert_eq!(session.settings().tone.exposure, 0.8);
    assert_eq!(saved_exposure(&studio, dawn), 0.0, "and not committed");
    assert_eq!(steps(&studio, dawn), before);

    assert!(studio.engine.commit_edit().unwrap());
    assert_eq!(saved_exposure(&studio, dawn), 0.8);
    assert_eq!(steps(&studio, dawn), before + 1);
}

/// The contrast and the exposure the photo open in Develop shows.
fn shown(studio: &Studio) -> (f32, f32) {
    let tone = studio.engine.develop().unwrap().settings().tone;
    (tone.contrast, tone.exposure)
}

/// Dawn open in Develop with a contrast of 20 committed.
fn dawn_with_a_committed_edit() -> (TempLibrary, Studio, PhotoId) {
    let (library, mut studio) = studio();
    let dawn = studio.id("dawn.jpg");
    open_rendered(&mut studio, dawn, FIT);
    studio.engine.adjust(Adj::Contrast, 20.0).unwrap();
    assert!(studio.engine.commit_edit().unwrap());
    (library, studio, dawn)
}

#[test]
fn undo_takes_back_the_edit_that_is_still_open_and_redo_brings_it_back() {
    let (_library, mut studio, dawn) = dawn_with_a_committed_edit();
    studio.engine.adjust(Adj::Exposure, 0.8).unwrap();
    assert!(studio.engine.develop().unwrap().is_editing());

    assert!(studio.engine.undo().unwrap());
    assert_eq!(
        shown(&studio),
        (20.0, 0.0),
        "the most recent change is the open edit: that is what is taken back"
    );
    assert!(studio.engine.redo().unwrap());
    assert_eq!(shown(&studio), (20.0, 0.8), "and it can be had back");
    assert_eq!(saved_exposure(&studio, dawn), 0.8);
}

#[test]
fn undoing_and_redoing_a_change_elsewhere_leaves_an_edit_in_progress_alone() {
    let (_library, mut studio) = studio();
    let (dawn, lake) = (studio.id("dawn.jpg"), studio.id("lake.png"));
    let (unrated, rated) = (Rating::new(0), Rating::new(3));
    studio
        .engine
        .set_rating(Target::Photos(vec![lake]), rated)
        .unwrap();
    edit_in_progress(&mut studio, dawn);
    let before = steps(&studio, dawn);

    assert!(studio.engine.undo().unwrap());
    assert_eq!(studio.engine.summary(lake).unwrap().rating, unrated);
    assert!(studio.engine.redo().unwrap());
    assert_eq!(studio.engine.summary(lake).unwrap().rating, rated);
    let session = studio.engine.develop().unwrap();
    assert!(session.is_editing(), "the edit is still in progress");
    assert_eq!(session.settings().tone.exposure, 0.8);
    assert_eq!(saved_exposure(&studio, dawn), 0.0, "and not committed");
    assert_eq!(steps(&studio, dawn), before);
}

#[test]
fn undoing_the_making_of_a_virtual_copy_keeps_the_edit_still_open_on_the_copy() {
    let (_library, mut studio) = studio();
    let dawn = studio.id("dawn.jpg");
    let copy = studio
        .engine
        .create_virtual_copies(Target::Photos(vec![dawn]))
        .unwrap()[0];
    edit_in_progress(&mut studio, copy);

    // The undo would take the copy out of the catalog, and the edit with
    // it. Committed first, the edit is what the undo takes back.
    assert!(studio.engine.undo().unwrap());
    assert!(studio.engine.summary(copy).is_some(), "the copy is there");
    assert_eq!(shown(&studio).1, 0.0);
    // The next undo takes the copy; two redos bring back both.
    assert!(studio.engine.undo().unwrap());
    assert!(studio.engine.summary(copy).is_none());
    assert!(studio.engine.redo().unwrap());
    assert!(studio.engine.redo().unwrap());
    assert_eq!(saved_exposure(&studio, copy), 0.8, "nothing was lost");
}

#[test]
fn redo_does_not_put_an_edit_that_is_still_open_under_what_it_redoes() {
    let (_library, mut studio, dawn) = dawn_with_a_committed_edit();
    assert!(studio.engine.undo().unwrap());
    assert_eq!(shown(&studio), (0.0, 0.0));
    // A new edit since the undo, still open.
    studio.engine.adjust(Adj::Exposure, 0.8).unwrap();

    let redone = studio.engine.redo().unwrap();
    assert_eq!(
        shown(&studio),
        (0.0, 0.8),
        "the edit is not lost under the contrast that could be redone"
    );
    assert!(
        !redone,
        "a change made since the undo ends what could be redone"
    );
    assert_eq!(saved_exposure(&studio, dawn), 0.8);
}

#[test]
fn redoing_the_removal_of_the_photo_being_edited_keeps_the_photo_and_the_edit() {
    let (_library, mut studio) = studio();
    let dawn = studio.id("dawn.jpg");
    studio
        .engine
        .remove_from_catalog(Target::Photos(vec![dawn]))
        .unwrap();
    assert!(studio.engine.undo().unwrap());
    // The photo is back; it is opened and an edit begun.
    edit_in_progress(&mut studio, dawn);

    let redone = studio.engine.redo().unwrap();
    assert!(
        studio.engine.summary(dawn).is_some(),
        "the photo is not removed again with an edit of it open"
    );
    assert!(!redone);
    assert_eq!(saved_exposure(&studio, dawn), 0.8, "and the edit is kept");
}

/// Give dawn a sidecar that says its contrast is 30 and nothing else of
/// its develop settings differs from what the catalog holds.
fn sidecar_with_contrast(studio: &mut Studio, dawn: PhotoId) {
    let info = ImageInfo::new(FileKind::Raster).with_size(96, 64);
    studio
        .engine
        .save_metadata_to_files(Target::Photos(vec![dawn]))
        .unwrap();
    studio.settle();
    let path = studio.engine.photo(dawn).unwrap().path;
    let sidecar = photo_xmp::sidecar_path(&path, FileKind::Raster);
    let mut data = photo_xmp::read_sidecar(&sidecar, info).unwrap().unwrap();
    let mut settings = data.develop.clone().unwrap();
    settings.tone.contrast = 30.0;
    data.develop = Some(settings);
    photo_xmp::write_sidecar(&sidecar, &data, info).unwrap();
}

#[test]
fn reading_metadata_from_its_file_keeps_the_edit_still_open_on_the_photo() {
    let (_library, mut studio) = studio();
    let dawn = studio.id("dawn.jpg");
    sidecar_with_contrast(&mut studio, dawn);
    edit_in_progress(&mut studio, dawn);

    studio
        .engine
        .read_metadata_from_files(Target::Photos(vec![dawn]))
        .unwrap();
    studio.settle();
    assert_eq!(shown(&studio), (30.0, 0.0), "the photo is as its file says");

    assert!(studio.engine.undo().unwrap());
    assert_eq!(
        shown(&studio),
        (0.0, 0.8),
        "and the edit that was open is there to go back to"
    );
}

#[test]
fn reading_metadata_from_other_files_leaves_an_edit_in_progress_alone() {
    let (_library, mut studio) = studio();
    let (dawn, lake) = (studio.id("dawn.jpg"), studio.id("lake.png"));
    sidecar_with_contrast(&mut studio, dawn);
    edit_in_progress(&mut studio, lake);
    let before = steps(&studio, lake);

    studio
        .engine
        .read_metadata_from_files(Target::Photos(vec![dawn]))
        .unwrap();
    studio.settle();
    assert_eq!(
        studio.engine.develop_settings(dawn).unwrap().tone.contrast,
        30.0
    );
    let session = studio.engine.develop().unwrap();
    assert!(session.is_editing(), "the edit is still in progress");
    assert_eq!(session.settings().tone.exposure, 0.8);
    assert_eq!(saved_exposure(&studio, lake), 0.0, "and not committed");
    assert_eq!(steps(&studio, lake), before);
}

#[test]
fn reading_a_file_that_replaces_no_settings_leaves_the_edit_on_its_photo_in_progress() {
    let (_library, mut studio) = studio();
    let dawn = studio.id("dawn.jpg");
    // The sidecar holds the settings the catalog holds.
    studio
        .engine
        .save_metadata_to_files(Target::Photos(vec![dawn]))
        .unwrap();
    studio.settle();
    edit_in_progress(&mut studio, dawn);
    let before = steps(&studio, dawn);

    studio
        .engine
        .read_metadata_from_files(Target::Photos(vec![dawn]))
        .unwrap();
    studio.settle();
    let session = studio.engine.develop().unwrap();
    assert!(session.is_editing(), "the edit is still in progress");
    assert_eq!(session.settings().tone.exposure, 0.8);
    assert_eq!(saved_exposure(&studio, dawn), 0.0, "and not committed");
    assert_eq!(steps(&studio, dawn), before);
}
