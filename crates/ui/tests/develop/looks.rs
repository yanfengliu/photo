//! Presets (apply on click, preview in the Navigator on hover, new, rename,
//! delete), snapshots (new on Cmd+N, apply, rename, delete) and the history
//! (every step listed, a click goes back to one, undoably).

use egui::{Key, Modifiers, PointerButton, Role};
use photo_core::develop::{SettingsGroup, Treatment};
use photo_engine::{BuiltInPreset, PresetKey};

use crate::{Studio, TestResult, develop, history, scene_files, session, settings, settle, shows};

#[test]
fn a_preset_previews_in_the_navigator_under_the_pointer_and_applies_on_a_click() -> TestResult {
    let (_dir, files) = scene_files();
    let studio = Studio::new();
    let mut harness = studio.app_with(files);
    develop(&mut harness, "scene.png")?;
    let row = harness
        .control(Role::TreeItem, "Preset Mono Contrast")?
        .rect
        .center();
    harness.hover_at(row)?;
    settle(&mut harness)?;
    harness.hover_at(row)?;
    let (key, preview) = session(&harness)
        .preset_preview()
        .ok_or("a preview of the preset")?;
    assert_eq!(*key, PresetKey::BuiltIn(BuiltInPreset::MonoContrast));
    let image = preview.image();
    // The colour chart, where colour would show if there were any.
    let pixel = image.pixel(image.width / 4, image.height * 72 / 100);
    let spread = pixel[..3].iter().max().unwrap_or(&0) - pixel[..3].iter().min().unwrap_or(&0);
    assert!(spread <= 2, "black and white: {pixel:?}");
    assert!(
        shows(&harness, "Preset: Mono Contrast"),
        "the Navigator says so"
    );
    assert_eq!(
        settings(&harness).treatment,
        Treatment::Color,
        "a preview changes nothing"
    );

    harness.pointer_leave()?;
    settle(&mut harness)?;
    assert!(
        session(&harness).preset_preview().is_none(),
        "gone with the pointer"
    );
    assert!(!shows(&harness, "Preset: Mono Contrast"));

    harness.click(Role::TreeItem, "Preset Vivid Colour")?;
    settle(&mut harness)?;
    assert_eq!(settings(&harness).presence.vibrance, 40.0);
    assert_eq!(
        history(&harness).last().map(String::as_str),
        Some("Preset: Vivid Colour")
    );
    harness.press_key(Key::Z, Modifiers::COMMAND)?;
    settle(&mut harness)?;
    assert_eq!(settings(&harness).presence.vibrance, 0.0, "undone");
    Ok(())
}

#[test]
fn a_new_preset_keeps_the_groups_ticked_and_is_renamed_and_deleted_from_its_menu() -> TestResult {
    let (_dir, files) = scene_files();
    let studio = Studio::new();
    let mut harness = studio.app_with(files);
    develop(&mut harness, "scene.png")?;
    harness.set_value(Role::Slider, "Exposure", 0.7)?;
    harness.set_value(Role::Slider, "Clarity", 30.0)?;
    settle(&mut harness)?;

    harness.click(Role::Button, "New preset")?;
    assert!(harness.app().dialog_open());
    harness.type_text("Bright")?;
    harness.click(Role::Button, "Check None")?;
    harness.click(Role::CheckBox, "Tone")?;
    harness.click(Role::Button, "Create")?;
    settle(&mut harness)?;
    let presets = harness.app().engine().presets()?;
    let mine = presets
        .iter()
        .find(|preset| preset.name == "Bright")
        .ok_or("the new preset")?;
    assert_eq!(mine.group, "User Presets");
    assert_eq!(
        mine.groups.iter().collect::<Vec<_>>(),
        [SettingsGroup::Tone]
    );
    assert_eq!(
        harness.count(Role::TreeItem, "Preset group User Presets"),
        1
    );

    // On the other photo it brings the tone and nothing else.
    let other = harness
        .control(Role::ListBoxOption, "Filmstrip scene.dng")?
        .rect
        .center();
    harness.click_at(other)?;
    settle(&mut harness)?;
    harness.click(Role::TreeItem, "Preset Bright")?;
    settle(&mut harness)?;
    let applied = settings(&harness);
    assert_eq!(applied.tone.exposure, 0.7);
    assert_eq!(applied.presence.clarity, 0.0);

    let row = harness
        .control(Role::TreeItem, "Preset Bright")?
        .rect
        .center();
    harness.click_at_with(row, PointerButton::Secondary, Modifiers::NONE)?;
    harness.click(Role::Button, "Rename…")?;
    harness.set_text(Role::TextInput, "Preset name", "Brighter")?;
    harness.set_text(Role::TextInput, "Preset group", "Mine")?;
    harness.click(Role::Button, "Rename")?;
    settle(&mut harness)?;
    assert_eq!(harness.count(Role::TreeItem, "Preset Brighter"), 1);
    assert_eq!(harness.count(Role::TreeItem, "Preset group Mine"), 1);

    let row = harness
        .control(Role::TreeItem, "Preset Brighter")?
        .rect
        .center();
    harness.click_at_with(row, PointerButton::Secondary, Modifiers::NONE)?;
    harness.click(Role::Button, "Delete…")?;
    harness.click(Role::Button, "Delete")?;
    settle(&mut harness)?;
    assert_eq!(harness.count(Role::TreeItem, "Preset Brighter"), 0);
    assert_eq!(
        settings(&harness).tone.exposure,
        0.7,
        "the photo keeps what the preset gave it"
    );
    Ok(())
}

#[test]
fn a_snapshot_keeps_the_settings_and_brings_them_back() -> TestResult {
    let (_dir, files) = scene_files();
    let studio = Studio::new();
    let mut harness = studio.app_with(files);
    develop(&mut harness, "scene.png")?;
    harness.set_value(Role::Slider, "Exposure", 0.6)?;
    settle(&mut harness)?;
    harness.press_key(Key::N, Modifiers::COMMAND)?;
    assert!(
        harness.app().dialog_open(),
        "Cmd+N names a snapshot in Develop"
    );
    harness.type_text("Bright")?;
    harness.press_key(Key::Enter, Modifiers::NONE)?;
    settle(&mut harness)?;
    assert_eq!(harness.count(Role::TreeItem, "Snapshot Bright"), 1);

    harness.set_value(Role::Slider, "Exposure", -1.0)?;
    settle(&mut harness)?;
    harness.click(Role::TreeItem, "Snapshot Bright")?;
    settle(&mut harness)?;
    assert_eq!(
        settings(&harness).tone.exposure,
        0.6,
        "back to the snapshot"
    );
    assert_eq!(
        history(&harness).last().map(String::as_str),
        Some("Snapshot: Bright")
    );

    // An unnamed snapshot takes the next number.
    harness.press_key(Key::N, Modifiers::COMMAND)?;
    harness.press_key(Key::Enter, Modifiers::NONE)?;
    settle(&mut harness)?;
    assert_eq!(harness.count(Role::TreeItem, "Snapshot Snapshot 2"), 1);

    let row = harness
        .control(Role::TreeItem, "Snapshot Bright")?
        .rect
        .center();
    harness.click_at_with(row, PointerButton::Secondary, Modifiers::NONE)?;
    harness.click(Role::Button, "Rename…")?;
    harness.set_text(Role::TextInput, "Snapshot name", "Daylight")?;
    harness.press_key(Key::Enter, Modifiers::NONE)?;
    settle(&mut harness)?;
    assert_eq!(harness.count(Role::TreeItem, "Snapshot Daylight"), 1);

    let row = harness
        .control(Role::TreeItem, "Snapshot Daylight")?
        .rect
        .center();
    harness.click_at_with(row, PointerButton::Secondary, Modifiers::NONE)?;
    harness.click(Role::Button, "Delete…")?;
    harness.click(Role::Button, "Delete")?;
    settle(&mut harness)?;
    assert_eq!(harness.count(Role::TreeItem, "Snapshot Daylight"), 0);
    assert_eq!(
        harness
            .app()
            .engine()
            .snapshots(session(&harness).photo())?
            .len(),
        1
    );
    Ok(())
}

#[test]
fn the_history_lists_every_step_and_a_click_goes_back_to_one_undoably() -> TestResult {
    let (_dir, files) = scene_files();
    let studio = Studio::new();
    let mut harness = studio.app_with(files);
    develop(&mut harness, "scene.png")?;
    harness.set_value(Role::Slider, "Exposure", 0.5)?;
    harness.set_value(Role::Slider, "Contrast", 20.0)?;
    harness.set_value(Role::Slider, "Vibrance", 30.0)?;
    settle(&mut harness)?;
    let steps = history(&harness);
    assert_eq!(steps.len(), 3, "{steps:?}");
    for (number, label) in steps.iter().enumerate() {
        let name = format!("History {}: {label}", number + 1);
        assert_eq!(harness.count(Role::TreeItem, &name), 1, "{name}");
    }
    let newest = format!("History 3: {}", steps[2]);
    assert_eq!(
        harness.control(Role::TreeItem, &newest)?.selected,
        Some(true)
    );

    harness.click(Role::TreeItem, &format!("History 1: {}", steps[0]))?;
    settle(&mut harness)?;
    let back = settings(&harness);
    assert_eq!((back.tone.exposure, back.tone.contrast), (0.5, 0.0));
    assert_eq!(back.presence.vibrance, 0.0);

    harness.press_key(Key::Z, Modifiers::COMMAND)?;
    settle(&mut harness)?;
    let again = settings(&harness);
    assert_eq!(
        (again.tone.contrast, again.presence.vibrance),
        (20.0, 30.0),
        "undo takes the jump back"
    );
    Ok(())
}
