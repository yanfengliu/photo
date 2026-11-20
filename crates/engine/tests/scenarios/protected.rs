//! Photos and catalogs that must not be changed: settings saved by a newer
//! version, and a write-protected catalog file. Every edit is refused with
//! its reason, and nothing changes.

use std::fs;
use std::path::Path;

use photo_core::develop::{Adj, GroupSet, SettingsGroup, WbPreset};
use photo_core::meta::Rating;
use photo_engine::{BuiltInPreset, CanvasView, DevelopLoad, EngineError, PresetKey, Target};
use photo_testkit::TempLibrary;

use crate::common::{Studio, config};

const FIT: CanvasView = CanvasView::Fit {
    width: 320,
    height: 240,
};

/// A studio on the fixture library whose `dawn.jpg` has develop settings
/// written by a newer build, which no catalog call can store.
fn studio_with_newer_settings() -> (TempLibrary, Studio) {
    let library = TempLibrary::new();
    let mut studio = Studio::new();
    studio.import_library(&library);
    let Studio { dir, engine } = studio;
    drop(engine);
    let catalog = rusqlite::Connection::open(config(dir.path(), 2).catalog_path).unwrap();
    let changed = catalog
        .execute(
            "UPDATE photos SET develop = '{\"future\": true}', develop_format = 99 WHERE file_name = 'dawn.jpg'",
            [],
        )
        .unwrap();
    assert_eq!(changed, 1);
    drop(catalog);
    let engine = photo_engine::Engine::open(config(dir.path(), 2)).unwrap();
    (library, Studio { dir, engine })
}

#[test]
fn a_photo_with_newer_settings_is_shown_but_cannot_be_edited() {
    let (_library, mut studio) = studio_with_newer_settings();
    let (dawn, lake) = (studio.id("dawn.jpg"), studio.id("lake.png"));
    studio.engine.open_develop(dawn).unwrap();
    studio.engine.set_canvas_view(FIT);
    studio.settle();
    let session = studio.engine.develop().unwrap();
    assert_eq!(
        session.load_state(),
        &DevelopLoad::Rendered,
        "it is still shown"
    );
    let reason = session.read_only_reason().unwrap().to_string();
    assert!(reason.contains("newer version"), "{reason}");
    let version = studio.engine.summary(dawn).unwrap().develop_version;

    let refused = |result: Result<(), EngineError>| match result {
        Err(EngineError::NotEditable(why)) => assert_eq!(why, reason),
        other => panic!("not refused: {other:?}"),
    };
    refused(studio.engine.adjust(Adj::Exposure, 1.0));
    refused(studio.engine.auto_tone());
    studio.engine.select_only(dawn);
    refused(
        studio
            .engine
            .apply_preset(&PresetKey::BuiltIn(BuiltInPreset::Vivid), Target::Active),
    );
    refused(studio.engine.reset_settings(Target::Selected));
    refused(
        studio
            .engine
            .quick_adjust(Target::Selected, Adj::Exposure, 1.0),
    );
    refused(studio.engine.rotate_left(Target::Selected));
    assert!(matches!(
        studio.engine.copy_settings(GroupSet::all()),
        Err(EngineError::Catalog(
            photo_engine::CatalogError::NewerFormat { .. }
        ))
    ));
    assert!(studio.engine.create_snapshot(dawn, "Never").is_err());
    assert!(studio.engine.snapshots(dawn).unwrap().is_empty());

    studio.engine.toggle_selected(lake);
    studio
        .engine
        .quick_white_balance(Target::Selected, WbPreset::AsShot)
        .unwrap();
    studio
        .engine
        .quick_adjust(Target::Selected, Adj::Exposure, 1.0)
        .unwrap();
    let toasts = studio.toasts();
    assert!(
        toasts
            .iter()
            .any(|toast| toast.contains("1 photos were left alone")),
        "{toasts:?}"
    );
    assert_eq!(
        studio.engine.develop_settings(lake).unwrap().tone.exposure,
        1.0
    );
    assert_eq!(
        studio.engine.summary(dawn).unwrap().develop_version,
        version,
        "dawn is untouched"
    );
    assert!(matches!(
        studio.engine.develop_settings(dawn),
        Err(EngineError::Catalog(
            photo_engine::CatalogError::NewerFormat { .. }
        ))
    ));
}

/// Make `path` read-only; false when this user can write it anyway.
fn protect(path: &Path) -> bool {
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(path, permissions).unwrap();
    fs::OpenOptions::new().write(true).open(path).is_err()
}

#[test]
fn a_write_protected_catalog_can_be_browsed_but_refuses_every_edit() {
    let library = TempLibrary::new();
    let mut studio = Studio::new();
    studio.import_library(&library);
    let dawn = studio.id("dawn.jpg");
    let Studio { dir, engine } = studio;
    drop(engine);
    let catalog_path = config(dir.path(), 2).catalog_path;
    if !protect(&catalog_path) {
        eprintln!("skipped: this user can write a read-only file");
        return;
    }
    let bytes = fs::read(&catalog_path).unwrap();
    let mut studio = Studio {
        engine: photo_engine::Engine::open(config(dir.path(), 2)).unwrap(),
        dir,
    };
    assert!(studio.engine.is_read_only());
    assert_eq!(studio.engine.photo_count(), 6, "it can be browsed");
    studio.engine.thumbnail(dawn);
    studio.settle();
    assert!(
        studio.engine.thumbnail(dawn).ready().is_some(),
        "previews still work"
    );

    studio.engine.open_develop(dawn).unwrap();
    studio.engine.set_canvas_view(FIT);
    studio.settle();
    let session = studio.engine.develop().unwrap();
    assert_eq!(session.load_state(), &DevelopLoad::Rendered);
    assert!(
        session
            .read_only_reason()
            .unwrap()
            .contains("write-protected")
    );
    assert!(matches!(
        studio.engine.adjust(Adj::Exposure, 1.0),
        Err(EngineError::NotEditable(_))
    ));
    studio.engine.select_only(dawn);
    let refusals = [
        studio.engine.set_rating(Target::Selected, Rating::new(3)),
        studio.engine.add_keywords(Target::Selected, &["Sea"]),
        studio.engine.rotate_right(Target::Selected),
        studio
            .engine
            .reset_group(Target::Selected, SettingsGroup::Tone),
        studio.engine.quick_auto_tone(Target::Selected),
        studio
            .engine
            .apply_preset(&PresetKey::BuiltIn(BuiltInPreset::Soft), Target::Selected),
        studio.engine.create_keyword("Sea", None).map(drop),
        studio.engine.create_snapshot(dawn, "Never").map(drop),
    ];
    for refusal in refusals {
        let text = refusal.unwrap_err().to_string();
        assert!(text.contains("write-protected"), "{text}");
    }
    studio.settle();
    assert_eq!(studio.engine.undo_label(), None, "nothing was recorded");
    assert_eq!(studio.engine.photo(dawn).unwrap().rating, Rating::new(0));
    drop(studio.engine);
    assert_eq!(
        fs::read(&catalog_path).unwrap(),
        bytes,
        "the file is as it was"
    );
}
