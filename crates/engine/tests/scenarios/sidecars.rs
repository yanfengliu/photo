//! Writing metadata to XMP sidecars and reading it back.

use std::path::PathBuf;

use photo_core::develop::Orientation;
use photo_core::meta::{ColorLabel, FileKind, Flag, PhotoId, Rating};
use photo_engine::{Target, TaskState, TextField};
use photo_testkit::TempLibrary;
use photo_xmp::ImageInfo;

use crate::common::Studio;

const INFO: ImageInfo = ImageInfo::new(FileKind::Raster).with_size(96, 64);

/// The library imported, with dawn marked up in every way a sidecar holds.
fn marked_dawn() -> (TempLibrary, Studio, PhotoId, PathBuf) {
    let library = TempLibrary::new();
    let mut studio = Studio::new();
    studio.import_library(&library);
    let dawn = studio.id("dawn.jpg");
    let target = || Target::Photos(vec![dawn]);
    let engine = &mut studio.engine;
    engine.set_rating(target(), Rating::new(4)).unwrap();
    engine.set_flag(target(), Flag::Picked).unwrap();
    engine.toggle_label(target(), ColorLabel::Blue).unwrap();
    engine
        .set_text(target(), TextField::Title, "First light")
        .unwrap();
    engine
        .set_text(target(), TextField::Caption, "Over the bay")
        .unwrap();
    engine
        .set_text(target(), TextField::Copyright, "(c) Someone")
        .unwrap();
    engine
        .add_keywords(target(), &["sunrise", "coast"])
        .unwrap();
    engine.rotate_right(target()).unwrap();
    let sidecar = photo_xmp::sidecar_path(&engine.photo(dawn).unwrap().path, FileKind::Raster);
    (library, studio, dawn, sidecar)
}

#[test]
fn saving_writes_a_sidecar_that_parses_back_to_the_same_values() {
    let (_library, mut studio, dawn, sidecar) = marked_dawn();
    let copy = studio
        .engine
        .create_virtual_copies(Target::Photos(vec![dawn]))
        .unwrap()[0];
    let task = studio
        .engine
        .save_metadata_to_files(Target::Photos(vec![dawn, copy]))
        .unwrap();
    studio.settle();
    let info = studio.engine.task(task).unwrap();
    assert_eq!(
        (info.state.clone(), info.done, info.total, info.errors),
        (TaskState::Finished, 1, 1, 0)
    );

    let data = photo_xmp::read_sidecar(&sidecar, INFO).unwrap().unwrap();
    assert_eq!(data.rating, Rating::new(4));
    assert_eq!(data.flag, Flag::Picked);
    assert_eq!(data.label, ColorLabel::Blue);
    assert_eq!(data.title, "First light");
    assert_eq!(data.caption, "Over the bay");
    assert_eq!(data.copyright, "(c) Someone");
    assert_eq!(data.creators, ["Test Kit"]);
    let mut keywords = data.keywords.clone();
    keywords.sort();
    assert_eq!(keywords, ["coast", "sunrise"]);
    let settings = data.develop.unwrap();
    assert_eq!(settings, studio.engine.develop_settings(dawn).unwrap());
    assert_eq!(settings.orientation, Orientation::new(1, false));
}

#[test]
fn reading_an_edited_sidecar_applies_it_and_is_undoable() {
    let (_library, mut studio, dawn, sidecar) = marked_dawn();
    studio
        .engine
        .save_metadata_to_files(Target::Photos(vec![dawn]))
        .unwrap();
    studio.settle();
    let mut data = photo_xmp::read_sidecar(&sidecar, INFO).unwrap().unwrap();
    data.rating = Rating::new(1);
    data.label = ColorLabel::Red;
    data.title = "Edited elsewhere".to_string();
    data.keywords = vec!["night".to_string()];
    data.hierarchical_keywords = Vec::new();
    let mut settings = data.develop.clone().unwrap();
    settings.tone.exposure = 1.0;
    data.develop = Some(settings);
    photo_xmp::write_sidecar(&sidecar, &data, INFO).unwrap();
    let before = studio.engine.develop_settings(dawn).unwrap();
    studio.toasts();

    let task = studio
        .engine
        .read_metadata_from_files(Target::Photos(vec![dawn]))
        .unwrap();
    studio.settle();
    assert_eq!(studio.engine.task(task).unwrap().state, TaskState::Finished);
    let record = studio.engine.photo(dawn).unwrap();
    assert_eq!(
        (record.rating, record.label),
        (Rating::new(1), ColorLabel::Red)
    );
    assert_eq!(record.title, "Edited elsewhere");
    assert_eq!(record.flag, Flag::Picked);
    assert_eq!(keywords(&studio, dawn), ["night"]);
    assert_eq!(
        studio.engine.develop_settings(dawn).unwrap().tone.exposure,
        1.0
    );
    assert_eq!(studio.toasts(), ["Read Metadata from Files"]);

    studio.engine.undo().unwrap();
    let record = studio.engine.photo(dawn).unwrap();
    assert_eq!(
        (record.rating, record.label),
        (Rating::new(4), ColorLabel::Blue)
    );
    assert_eq!(record.title, "First light");
    assert_eq!(keywords(&studio, dawn), ["coast", "sunrise"]);
    assert_eq!(studio.engine.develop_settings(dawn).unwrap(), before);
    studio.engine.redo().unwrap();
    assert_eq!(studio.engine.photo(dawn).unwrap().rating, Rating::new(1));
}

fn keywords(studio: &Studio, photo: PhotoId) -> Vec<String> {
    let mut names: Vec<String> = studio
        .engine
        .photo_keywords(photo)
        .unwrap()
        .into_iter()
        .map(|keyword| keyword.name)
        .collect();
    names.sort();
    names
}

#[test]
fn a_photo_without_a_sidecar_is_left_alone() {
    let library = TempLibrary::new();
    let mut studio = Studio::new();
    studio.import_library(&library);
    let lake = studio.id("lake.png");
    studio
        .engine
        .set_rating(Target::Photos(vec![lake]), Rating::new(2))
        .unwrap();
    let task = studio
        .engine
        .read_metadata_from_files(Target::Photos(vec![lake]))
        .unwrap();
    studio.settle();
    assert_eq!(studio.engine.task(task).unwrap().state, TaskState::Finished);
    assert_eq!(studio.engine.photo(lake).unwrap().rating, Rating::new(2));
    assert_eq!(studio.engine.undo_label(), Some("Set Rating to 2"));
}
