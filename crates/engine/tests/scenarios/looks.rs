//! Presets, snapshots and history steps.

use photo_core::develop::{Adj, GroupSet, SettingsGroup, Treatment, WbPreset};
use photo_engine::{BUILT_IN_GROUP, BuiltInPreset, CanvasView, PresetKey, Target};
use photo_testkit::TempLibrary;

use crate::common::{Studio, open_rendered};

const FIT: CanvasView = CanvasView::Fit {
    width: 320,
    height: 240,
};

fn studio() -> (TempLibrary, Studio) {
    let library = TempLibrary::new();
    let mut studio = Studio::new();
    studio.import_library(&library);
    (library, studio)
}

#[test]
fn built_in_presets_set_only_their_groups_on_every_selected_photo_in_one_step() {
    let (_library, mut studio) = studio();
    let (dawn, lake, harbour) = (
        studio.id("dawn.jpg"),
        studio.id("lake.png"),
        studio.id("harbour.dng"),
    );
    let presets = studio.engine.presets().unwrap();
    let built_in: Vec<&str> = presets
        .iter()
        .filter(|preset| preset.group == BUILT_IN_GROUP)
        .map(|preset| preset.name.as_str())
        .collect();
    assert_eq!(
        built_in,
        [
            "Mono Contrast",
            "Vivid Colour",
            "Soft Detail",
            "Warm Light",
            "Cool Light",
            "Faded Print"
        ]
    );

    open_rendered(&mut studio, dawn, FIT);
    studio.engine.adjust(Adj::Exposure, 0.5).unwrap();
    studio.engine.commit_edit().unwrap();
    studio.engine.select_only(dawn);
    studio.engine.toggle_selected(lake);
    studio.engine.toggle_selected(harbour);
    let vivid = PresetKey::BuiltIn(BuiltInPreset::Vivid);
    studio
        .engine
        .apply_preset(&vivid, Target::Selected)
        .unwrap();
    assert_eq!(studio.engine.undo_label(), Some("Preset: Vivid Colour"));
    for photo in [dawn, lake, harbour] {
        let settings = studio.engine.develop_settings(photo).unwrap();
        assert_eq!(settings.presence.vibrance, 40.0);
        assert_eq!(settings.presence.saturation, 10.0);
        let history = studio.engine.history(photo).unwrap();
        assert_eq!(history.last().unwrap().label, "Preset: Vivid Colour");
    }
    assert_eq!(
        studio.engine.develop_settings(dawn).unwrap().tone.exposure,
        0.5,
        "other groups are kept"
    );
    assert_eq!(
        studio
            .engine
            .develop()
            .unwrap()
            .settings()
            .presence
            .vibrance,
        40.0,
        "the open photo follows"
    );

    studio.engine.undo().unwrap();
    for photo in [lake, harbour] {
        let settings = studio.engine.develop_settings(photo).unwrap();
        assert!(settings.is_default_for(studio.engine.photo(photo).unwrap().kind));
    }
    assert_eq!(
        studio
            .engine
            .develop_settings(dawn)
            .unwrap()
            .presence
            .vibrance,
        0.0
    );
    assert_eq!(
        studio.engine.develop_settings(dawn).unwrap().tone.exposure,
        0.5
    );
}

#[test]
fn white_balance_looks_shift_raster_files_and_choose_an_illuminant_for_raw_files() {
    let (_library, mut studio) = studio();
    let (dawn, harbour) = (studio.id("dawn.jpg"), studio.id("harbour.dng"));
    let warm = PresetKey::BuiltIn(BuiltInPreset::Warm);
    studio
        .engine
        .apply_preset(&warm, Target::Photos(vec![dawn, harbour]))
        .unwrap();
    let raster = studio.engine.develop_settings(dawn).unwrap().white_balance;
    assert_eq!((raster.preset, raster.temp), (WbPreset::Custom, 25.0));
    let raw = studio
        .engine
        .develop_settings(harbour)
        .unwrap()
        .white_balance;
    assert_eq!(raw.preset, WbPreset::Cloudy);
    assert_eq!(Some((raw.temp, raw.tint)), WbPreset::Cloudy.illuminant());

    let mono = PresetKey::BuiltIn(BuiltInPreset::MonoContrast);
    studio
        .engine
        .apply_preset(&mono, Target::Photos(vec![dawn]))
        .unwrap();
    let settings = studio.engine.develop_settings(dawn).unwrap();
    assert_eq!(settings.treatment, Treatment::BlackAndWhite);
    assert_eq!(settings.white_balance, raster, "the earlier look stays");
}

#[test]
fn user_presets_save_the_chosen_groups_and_can_be_renamed_and_deleted() {
    let (_library, mut studio) = studio();
    let (dawn, lake) = (studio.id("dawn.jpg"), studio.id("lake.png"));
    open_rendered(&mut studio, dawn, FIT);
    studio.engine.adjust(Adj::Exposure, 1.0).unwrap();
    studio.engine.adjust(Adj::Contrast, 20.0).unwrap();
    studio.engine.adjust(Adj::Vibrance, 30.0).unwrap();
    let tone = GroupSet::NONE.with(SettingsGroup::Tone);
    let id = studio.engine.create_preset("Mine", "Bright", tone).unwrap();
    assert!(
        !studio.engine.develop().unwrap().is_editing(),
        "the edit was committed first"
    );
    let listed = studio.engine.presets().unwrap();
    let mine = listed
        .iter()
        .find(|preset| preset.key == PresetKey::User(id))
        .unwrap();
    assert_eq!(
        (mine.group.as_str(), mine.name.as_str(), mine.groups),
        ("Mine", "Bright", tone)
    );

    studio
        .engine
        .apply_preset(&PresetKey::User(id), Target::Photos(vec![lake]))
        .unwrap();
    let settings = studio.engine.develop_settings(lake).unwrap();
    assert_eq!(
        (settings.tone.exposure, settings.tone.contrast),
        (1.0, 20.0)
    );
    assert_eq!(settings.presence.vibrance, 0.0, "presence was not saved");

    studio.engine.rename_preset(id, "Mine", "Brighter").unwrap();
    let renamed = studio.engine.presets().unwrap();
    assert!(renamed.iter().any(|preset| preset.name == "Brighter"));
    studio.engine.delete_preset(id).unwrap();
    let left = studio.engine.presets().unwrap();
    assert_eq!(left.len(), BuiltInPreset::ALL.len());
    assert_eq!(
        studio.engine.develop_settings(lake).unwrap().tone.exposure,
        1.0
    );
    assert!(
        studio
            .engine
            .apply_preset(&PresetKey::User(id), Target::Active)
            .is_err()
    );
}

#[test]
fn snapshots_are_saved_renamed_applied_and_deleted() {
    let (_library, mut studio) = studio();
    let dawn = studio.id("dawn.jpg");
    open_rendered(&mut studio, dawn, FIT);
    studio.engine.adjust(Adj::Exposure, 0.8).unwrap();
    let first = studio.engine.create_snapshot(dawn, "Bright").unwrap();
    studio.engine.adjust(Adj::Exposure, -0.4).unwrap();
    studio.engine.adjust(Adj::Clarity, 15.0).unwrap();
    studio.engine.commit_edit().unwrap();
    let second = studio.engine.create_snapshot(dawn, "Dark").unwrap();
    let snapshots = studio.engine.snapshots(dawn).unwrap();
    assert_eq!(snapshots.len(), 2);
    assert_eq!(snapshots[0].settings.tone.exposure, 0.8);
    assert_eq!(snapshots[1].settings.presence.clarity, 15.0);

    studio.engine.apply_snapshot(dawn, first).unwrap();
    assert_eq!(studio.engine.undo_label(), Some("Snapshot: Bright"));
    let session = studio.engine.develop().unwrap();
    assert_eq!(session.settings(), &snapshots[0].settings);
    studio.engine.undo().unwrap();
    assert_eq!(
        studio.engine.develop_settings(dawn).unwrap(),
        snapshots[1].settings
    );

    studio.engine.rename_snapshot(second, "Moody").unwrap();
    assert_eq!(studio.engine.snapshots(dawn).unwrap()[1].name, "Moody");
    studio.engine.delete_snapshot(first).unwrap();
    let left = studio.engine.snapshots(dawn).unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].id, second);
}

#[test]
fn going_to_a_history_step_is_undoable_and_keeps_the_later_steps() {
    let (_library, mut studio) = studio();
    let dawn = studio.id("dawn.jpg");
    open_rendered(&mut studio, dawn, FIT);
    for value in [0.25, 0.5, 0.75] {
        studio.engine.adjust(Adj::Exposure, value).unwrap();
        studio.engine.commit_edit().unwrap();
    }
    let history = studio.engine.history(dawn).unwrap();
    let steps = history.len();
    let back = history[steps - 3].id;
    assert_eq!(history[steps - 3].label, "Exposure +0.25");

    studio.engine.go_to_history_step(dawn, back).unwrap();
    assert_eq!(
        studio.engine.develop().unwrap().settings().tone.exposure,
        0.25
    );
    let history = studio.engine.history(dawn).unwrap();
    assert_eq!(history.len(), steps, "the later steps are still listed");
    assert!(history[steps - 3].is_current);
    assert_eq!(studio.engine.undo_label(), Some("History: Exposure +0.25"));

    studio.engine.undo().unwrap();
    assert_eq!(
        studio.engine.develop().unwrap().settings().tone.exposure,
        0.75
    );
    studio.engine.redo().unwrap();
    assert_eq!(
        studio.engine.develop_settings(dawn).unwrap().tone.exposure,
        0.25
    );

    studio.engine.adjust(Adj::Exposure, 0.3).unwrap();
    studio.engine.commit_edit().unwrap();
    let history = studio.engine.history(dawn).unwrap();
    assert_eq!(
        history.len(),
        steps - 1,
        "the next edit replaces the later steps"
    );
}

#[test]
fn hovering_a_preset_renders_a_small_preview_with_it() {
    let (_library, mut studio) = studio();
    let dawn = studio.id("dawn.jpg");
    open_rendered(&mut studio, dawn, FIT);
    let mono = PresetKey::BuiltIn(BuiltInPreset::MonoContrast);
    studio.engine.preview_preset(Some(mono), 48, 48).unwrap();
    studio.settle();
    let session = studio.engine.develop().unwrap();
    let (key, frame) = session.preset_preview().unwrap();
    assert_eq!(*key, mono);
    let image = frame.image();
    assert!(image.width <= 48 && image.height <= 48);
    assert_eq!((image.width, image.height), (48, 32));
    let grey = image
        .pixels
        .chunks_exact(4)
        .all(|pixel| pixel[0].abs_diff(pixel[1]) <= 1 && pixel[1].abs_diff(pixel[2]) <= 1);
    assert!(grey, "the preview is black and white");
    assert_eq!(
        session.settings().treatment,
        Treatment::Color,
        "nothing was applied"
    );
    assert!(!session.is_editing());

    studio.engine.preview_preset(None, 48, 48).unwrap();
    assert!(studio.engine.develop().unwrap().preset_preview().is_none());
}
