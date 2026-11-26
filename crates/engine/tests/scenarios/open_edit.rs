//! Commands that reach the photo being edited while an edit of it is still
//! open in Develop. Whatever they do to the photo, the edit must be there
//! to come back to; and an edit of a photo they do not reach must be left
//! as it is, in progress.

use photo_core::develop::Adj;
use photo_core::meta::PhotoId;
use photo_engine::Target;
use photo_testkit::TempLibrary;

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
