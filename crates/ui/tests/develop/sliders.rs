//! The sliders as photographers expect them: a drag edits with draft
//! renders and commits one step on release; a double-click on the label or
//! the track resets; arrow keys nudge the focused slider; a number can be
//! typed. White balance starts at, and resets to, the camera's.

use egui::{Key, Modifiers, Role, pos2};
use photo_core::develop::WbPreset;
use photo_engine::Quality;

use crate::{
    Studio, TestResult, develop, frame_luma, history, scene_files, session, settings, settle,
    slider, track_x,
};

#[test]
fn a_drag_renders_drafts_brightens_the_frame_and_commits_one_step_that_undo_takes_back()
-> TestResult {
    let (_dir, files) = scene_files();
    let studio = Studio::new();
    let mut harness = studio.app_with(files);
    develop(&mut harness, "scene.png")?;
    let before = frame_luma(&harness);
    let steps = history(&harness).len();
    let control = slider(&harness, "Exposure")?;
    let y = control.rect.center().y;
    let start = pos2(track_x(&control, 0.0), y);
    harness.pointer_down(start)?;
    for value in [0.2, 0.4, 0.6, 0.8] {
        harness.pointer_move(pos2(track_x(&control, value), y))?;
    }
    // Let the newest draft arrive while the slider is still held.
    settle(&mut harness)?;
    let held = session(&harness);
    assert!(held.is_editing(), "nothing committed while held");
    let draft = held.frame().ok_or("a frame")?;
    assert_eq!(draft.quality(), Quality::Draft);
    assert_eq!(
        draft.version(),
        held.version(),
        "the draft of the newest value"
    );
    assert_eq!(history(&harness).len(), steps, "no step yet");
    assert!(harness.app().canvas_shows().current, "the canvas shows it");

    harness.pointer_up(pos2(track_x(&control, 0.8), y))?;
    settle(&mut harness)?;
    let exposure = settings(&harness).tone.exposure;
    assert!((exposure - 0.8).abs() < 0.03, "{exposure}");
    let after = frame_luma(&harness);
    assert!(after > before + 10.0, "{before} -> {after}");
    let frame = session(&harness).frame().ok_or("a frame")?;
    assert_eq!(frame.quality(), Quality::Full, "a full render once let go");
    let labels = history(&harness);
    assert_eq!(labels.len(), steps + 1, "one step for the drag: {labels:?}");
    assert!(
        labels
            .last()
            .is_some_and(|label| label.starts_with("Exposure +0."))
    );
    assert_eq!(
        harness.count(
            Role::TreeItem,
            &format!("History {}: {}", steps + 1, labels[steps])
        ),
        1
    );

    harness.press_key(Key::Z, Modifiers::COMMAND)?;
    settle(&mut harness)?;
    assert_eq!(settings(&harness).tone.exposure, 0.0);
    assert_eq!(history(&harness).len(), steps);
    assert!(
        (frame_luma(&harness) - before).abs() < 0.5,
        "the frame is as it was"
    );
    Ok(())
}

#[test]
fn a_double_click_on_the_label_or_the_track_resets_the_slider() -> TestResult {
    let (_dir, files) = scene_files();
    let studio = Studio::new();
    let mut harness = studio.app_with(files);
    develop(&mut harness, "scene.png")?;
    harness.set_value(Role::Slider, "Contrast", 40.0)?;
    settle(&mut harness)?;
    assert_eq!(settings(&harness).tone.contrast, 40.0);
    let track = slider(&harness, "Contrast")?.rect;
    harness.double_click_at(pos2(track.left() - 40.0, track.center().y))?;
    settle(&mut harness)?;
    assert_eq!(settings(&harness).tone.contrast, 0.0, "the label resets");

    harness.set_value(Role::Slider, "Contrast", -25.0)?;
    settle(&mut harness)?;
    let steps = history(&harness).len();
    harness.double_click_at(pos2(track.left() + 20.0, track.center().y))?;
    settle(&mut harness)?;
    assert_eq!(settings(&harness).tone.contrast, 0.0, "the track resets");
    assert_eq!(slider(&harness, "Contrast")?.value, Some(0.0));
    assert!(history(&harness).len() > steps, "a reset is a step");
    Ok(())
}

#[test]
fn arrow_keys_nudge_the_focused_slider_by_its_step_and_shift_by_ten() -> TestResult {
    let (_dir, files) = scene_files();
    let studio = Studio::new();
    let mut harness = studio.app_with(files);
    develop(&mut harness, "scene.png")?;
    let photo = session(&harness).photo();
    harness.focus(Role::Slider, "Contrast")?;
    harness.press_key(Key::ArrowRight, Modifiers::NONE)?;
    harness.press_key(Key::ArrowRight, Modifiers::SHIFT)?;
    settle(&mut harness)?;
    assert_eq!(settings(&harness).tone.contrast, 11.0);
    harness.focus(Role::Slider, "Exposure")?;
    harness.press_key(Key::ArrowLeft, Modifiers::NONE)?;
    settle(&mut harness)?;
    assert_eq!(settings(&harness).tone.exposure, -0.01);
    assert_eq!(
        session(&harness).photo(),
        photo,
        "the arrows stayed with the slider"
    );
    Ok(())
}

#[test]
fn temp_and_tint_show_the_cameras_balance_and_a_double_click_goes_back_to_it() -> TestResult {
    let (_dir, files) = scene_files();
    let studio = Studio::new();
    let mut harness = studio.app_with(files);
    develop(&mut harness, "scene.dng")?;
    let (kelvin, tint) = session(&harness)
        .as_shot_white_balance()
        .ok_or("the camera's balance")?;
    let temp = slider(&harness, "Temp")?;
    assert!(
        (temp.value.unwrap_or(0.0) as f32 - kelvin).abs() < 1.0,
        "{:?} vs {kelvin}",
        temp.value
    );
    assert_eq!(
        (temp.min, temp.max),
        (Some(2000.0), Some(50_000.0)),
        "kelvin"
    );
    assert!((slider(&harness, "Tint")?.value.unwrap_or(0.0) as f32 - tint).abs() < 1.0);

    let handle = harness.control(Role::Slider, "Temp")?.rect;
    let from = pos2(
        handle.left() + 3.5 + (handle.width() - 7.0) * log_position(kelvin),
        handle.center().y,
    );
    harness.drag(from, from + egui::vec2(40.0, 0.0))?;
    settle(&mut harness)?;
    let balance = settings(&harness).white_balance;
    assert_eq!(balance.preset, WbPreset::Custom);
    assert!(balance.temp > kelvin + 100.0, "warmer: {}", balance.temp);

    harness.double_click_at(pos2(handle.left() - 40.0, handle.center().y))?;
    settle(&mut harness)?;
    assert_eq!(
        settings(&harness).white_balance.preset,
        WbPreset::AsShot,
        "back to the camera's"
    );
    Ok(())
}

/// Where kelvin sits along the logarithmic Temp track, 0 to 1.
fn log_position(kelvin: f32) -> f32 {
    (kelvin / 2000.0).ln() / (50_000.0_f32 / 2000.0).ln()
}
