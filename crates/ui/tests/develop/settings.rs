//! Moving settings between photos: Copy Settings with its checklist and
//! Paste onto the filmstrip's selection, Sync with several photos selected,
//! Previous, and Reset.

use egui::{Key, Modifiers, Role};
use photo_core::develop::{DevelopSettings, GroupSet};
use photo_core::meta::FileKind;
use photo_ui::PhotoApp;
use photo_ui::harness::Harness;

use crate::{Studio, TestResult, develop, id, scene_files, settings, settle};

/// The settings the catalog holds for the photo called `name`.
fn stored(harness: &Harness<PhotoApp>, name: &str) -> DevelopSettings {
    harness
        .app()
        .engine()
        .develop_settings(id(harness, name))
        .expect("the settings read")
}

/// Click the filmstrip cell of `name` with `modifiers`.
fn filmstrip(harness: &mut Harness<PhotoApp>, name: &str, modifiers: Modifiers) -> TestResult {
    let cell = harness
        .control(Role::ListBoxOption, &format!("Filmstrip {name}"))?
        .rect
        .center();
    harness.click_at_with(cell, egui::PointerButton::Primary, modifiers)?;
    settle(harness)?;
    Ok(())
}

/// Give the open photo an exposure and a clarity.
fn edit(harness: &mut Harness<PhotoApp>) -> TestResult {
    harness.set_value(Role::Slider, "Exposure", 1.0)?;
    harness.set_value(Role::Slider, "Clarity", 30.0)?;
    settle(harness)?;
    Ok(())
}

#[test]
fn copied_groups_paste_onto_the_selection_and_nothing_else_does() -> TestResult {
    let (_dir, files) = scene_files();
    let studio = Studio::new();
    let mut harness = studio.app_with(files);
    develop(&mut harness, "scene.png")?;
    edit(&mut harness)?;

    harness.press_key(Key::C, Modifiers::COMMAND | Modifiers::SHIFT)?;
    assert!(harness.app().dialog_open());
    for (group, ticked) in [("Tone", true), ("Presence", true), ("Crop", false)] {
        let tick = harness.control(Role::CheckBox, group)?.toggled;
        assert_eq!(
            tick,
            Some(ticked),
            "{group} starts as the engine's default copy set"
        );
    }
    harness.click(Role::Button, "Check All")?;
    assert_eq!(harness.control(Role::CheckBox, "Crop")?.toggled, Some(true));
    harness.click(Role::Button, "Check None")?;
    harness.click(Role::CheckBox, "Tone")?;
    harness.click(Role::Button, "Copy")?;
    settle(&mut harness)?;
    let clipboard = harness.app().engine().clipboard().ok_or("copied")?;
    assert_eq!(
        clipboard.groups(),
        GroupSet::NONE.with(photo_core::develop::SettingsGroup::Tone)
    );

    filmstrip(&mut harness, "scene.dng", Modifiers::NONE)?;
    harness.press_key(Key::V, Modifiers::COMMAND | Modifiers::SHIFT)?;
    settle(&mut harness)?;
    let pasted = settings(&harness);
    assert_eq!(pasted.tone.exposure, 1.0, "the tone");
    assert_eq!(pasted.presence.clarity, 0.0, "only the tone");
    assert_eq!(
        pasted.detail,
        DevelopSettings::default_for(FileKind::Raw).detail,
        "the raw file keeps its own detail"
    );

    // The dialog remembers the groups chosen last.
    harness.press_key(Key::C, Modifiers::COMMAND | Modifiers::SHIFT)?;
    assert_eq!(
        harness.control(Role::CheckBox, "Presence")?.toggled,
        Some(false)
    );
    harness.press_key(Key::Escape, Modifiers::NONE)?;
    assert!(!harness.app().dialog_open());
    Ok(())
}

#[test]
fn previous_gives_the_photo_every_setting_of_the_photo_before_it() -> TestResult {
    let (_dir, files) = scene_files();
    let studio = Studio::new();
    let mut harness = studio.app_with(files);
    develop(&mut harness, "scene.png")?;
    edit(&mut harness)?;
    harness.set_value(Role::Slider, "Vibrance", 25.0)?;
    settle(&mut harness)?;
    let wanted = stored(&harness, "scene.png");
    filmstrip(&mut harness, "scene.dng", Modifiers::NONE)?;
    harness.click(Role::Button, "Previous")?;
    settle(&mut harness)?;
    let now = stored(&harness, "scene.dng");
    assert_eq!(now.tone, wanted.tone);
    assert_eq!(now.presence, wanted.presence);
    assert_eq!(now.detail, wanted.detail, "everything, detail too");
    Ok(())
}

#[test]
fn sync_gives_the_other_selected_photos_the_groups_chosen() -> TestResult {
    let (_dir, files) = scene_files();
    let studio = Studio::new();
    let mut harness = studio.app_with(files);
    develop(&mut harness, "scene.png")?;
    edit(&mut harness)?;
    // Both selected, the edited one active.
    filmstrip(&mut harness, "scene.dng", Modifiers::NONE)?;
    filmstrip(&mut harness, "scene.png", Modifiers::SHIFT)?;
    assert_eq!(harness.app().engine().selection().selected().len(), 2);
    assert_eq!(
        harness.app().engine().active_photo(),
        Some(id(&harness, "scene.png"))
    );
    assert_eq!(
        harness.count(Role::Button, "Previous"),
        0,
        "Sync takes its place"
    );
    harness.click(Role::Button, "Sync…")?;
    harness.click(Role::Button, "Check None")?;
    harness.click(Role::CheckBox, "Presence")?;
    harness.click(Role::Button, "Synchronize")?;
    settle(&mut harness)?;
    let synced = stored(&harness, "scene.dng");
    assert_eq!(synced.presence.clarity, 30.0);
    assert_eq!(synced.tone.exposure, 0.0, "only presence");
    Ok(())
}

#[test]
fn reset_puts_the_photo_back_as_imported_and_undo_brings_the_edit_back() -> TestResult {
    let (_dir, files) = scene_files();
    let studio = Studio::new();
    let mut harness = studio.app_with(files);
    develop(&mut harness, "scene.dng")?;
    edit(&mut harness)?;
    harness.press_key(Key::R, Modifiers::COMMAND | Modifiers::SHIFT)?;
    settle(&mut harness)?;
    assert!(settings(&harness).is_default_for(FileKind::Raw));
    harness.press_key(Key::Z, Modifiers::COMMAND)?;
    settle(&mut harness)?;
    assert_eq!(settings(&harness).tone.exposure, 1.0);
    harness.click(Role::Button, "Reset")?;
    settle(&mut harness)?;
    assert!(settings(&harness).is_default_for(FileKind::Raw));
    Ok(())
}
