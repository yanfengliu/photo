//! The develop slider.

use egui::{Key, Modifiers, Role, pos2};
use photo_core::develop::{Adj, AdjSpec};
use photo_core::meta::FileKind;
use photo_ui::harness::{Harness, HarnessBuilder, Host};
use photo_ui::widgets::{AdjSlider, Scale};

use crate::{TestResult, across};

/// One slider and a tally of what it reported.
pub struct Probe {
    spec: AdjSpec,
    pub value: f32,
    default: f32,
    scale: Scale,
    enabled: bool,
    /// Leave the slider out, as when its panel is closed.
    pub hidden: bool,
    pub changes: u32,
    pub finishes: u32,
}

/// A harness around one slider for `spec`.
pub fn slider(spec: AdjSpec, value: f32, default: f32) -> Harness<Host<Probe>> {
    let probe = Probe {
        spec,
        value,
        default,
        scale: Scale::LINEAR,
        enabled: true,
        hidden: false,
        changes: 0,
        finishes: 0,
    };
    HarnessBuilder::new()
        .size([320.0, 120.0])
        .strict_glyphs(true)
        .build_ui(probe, |ui, probe| {
            if probe.hidden {
                return;
            }
            ui.add_enabled_ui(probe.enabled, |ui| {
                let response = AdjSlider::new(probe.spec, &mut probe.value, probe.default)
                    .scale(probe.scale)
                    .show(ui);
                probe.changes += u32::from(response.changed);
                probe.finishes += u32::from(response.finished);
            });
        })
}

/// A harness around a Contrast slider (-100 to 100, default 0).
pub fn contrast(value: f32) -> Harness<Host<Probe>> {
    slider(Adj::Contrast.spec(), value, 0.0)
}

#[test]
fn the_value_is_set_through_the_accessibility_tree() -> TestResult {
    let mut harness = slider(Adj::Exposure.spec(), 0.0, 0.0);
    harness.run()?;

    harness.set_value(Role::Slider, "Exposure", 1.25)?;
    assert_eq!(harness.app().state.value, 1.25);
    assert_eq!(harness.control(Role::Slider, "Exposure")?.value, Some(1.25));
    assert_eq!(harness.app().state.finishes, 1);

    harness.set_value(Role::Slider, "Exposure", 99.0)?;
    assert_eq!(harness.app().state.value, 5.0, "clamped to the range");
    harness.set_value(Role::Slider, "Exposure", 0.123_456)?;
    assert_eq!(
        harness.app().state.value,
        0.12,
        "rounded to the shown decimals"
    );
    Ok(())
}

#[test]
fn arrow_keys_nudge_by_one_step_and_shift_by_ten() -> TestResult {
    let mut harness = contrast(0.0);
    harness.run()?;
    harness.focus(Role::Slider, "Contrast")?;

    harness.press_key(Key::ArrowRight, Modifiers::NONE)?;
    assert_eq!(harness.app().state.value, 1.0);
    harness.press_key(Key::ArrowUp, Modifiers::NONE)?;
    assert_eq!(harness.app().state.value, 2.0);
    harness.press_key(Key::ArrowLeft, Modifiers::NONE)?;
    harness.press_key(Key::ArrowDown, Modifiers::NONE)?;
    assert_eq!(harness.app().state.value, 0.0);
    harness.press_key(Key::ArrowRight, Modifiers::SHIFT)?;
    assert_eq!(harness.app().state.value, 10.0);
    assert_eq!(
        harness.app().state.finishes,
        5,
        "each nudge is a complete edit"
    );
    assert!(harness.control(Role::Slider, "Contrast")?.focused);
    Ok(())
}

#[test]
fn the_step_comes_from_the_adjustment() -> TestResult {
    let mut harness = slider(Adj::Exposure.spec(), 0.0, 0.0);
    harness.run()?;
    harness.focus(Role::Slider, "Exposure")?;
    harness.press_key(Key::ArrowRight, Modifiers::NONE)?;
    assert_eq!(harness.app().state.value, 0.01);
    Ok(())
}

#[test]
fn arrow_keys_do_nothing_without_focus() -> TestResult {
    let mut harness = contrast(0.0);
    harness.run()?;
    harness.press_key(Key::ArrowRight, Modifiers::NONE)?;
    assert_eq!(harness.app().state.value, 0.0);
    Ok(())
}

#[test]
fn double_clicking_the_track_resets_to_the_default() -> TestResult {
    let mut harness = slider(Adj::Contrast.spec(), 40.0, 15.0);
    harness.run()?;
    let track = harness.control(Role::Slider, "Contrast")?.rect;

    harness.double_click_at(across(track, 0.2))?;
    assert_eq!(harness.app().state.value, 15.0);
    Ok(())
}

#[test]
fn double_clicking_the_handle_resets_in_a_single_edit() -> TestResult {
    let mut harness = contrast(40.0);
    harness.run()?;
    let track = harness.control(Role::Slider, "Contrast")?.rect;

    // +40 on a -100..100 track is 70% of the way along.
    harness.double_click_at(across(track, 0.7))?;
    assert_eq!(harness.app().state.value, 0.0);
    assert_eq!(harness.app().state.changes, 1);
    assert_eq!(harness.app().state.finishes, 1);
    Ok(())
}

#[test]
fn double_clicking_the_label_resets_to_the_default() -> TestResult {
    let mut harness = contrast(-30.0);
    harness.run()?;
    let track = harness.control(Role::Slider, "Contrast")?.rect;

    harness.double_click_at(pos2(track.left() - 40.0, track.center().y))?;
    assert_eq!(harness.app().state.value, 0.0);
    assert_eq!(harness.app().state.finishes, 1);
    Ok(())
}

#[test]
fn a_typed_value_commits_on_enter_and_clamps_to_the_range() -> TestResult {
    let mut harness = contrast(0.0);
    harness.run()?;

    harness.click(Role::TextInput, "Contrast")?;
    harness.type_text("55")?;
    assert_eq!(
        harness.app().state.value,
        0.0,
        "nothing is applied while typing"
    );
    harness.press_key(Key::Enter, Modifiers::NONE)?;
    assert_eq!(harness.app().state.value, 55.0);
    assert_eq!(harness.app().state.finishes, 1);
    assert_eq!(
        harness
            .control(Role::TextInput, "Contrast")?
            .text
            .as_deref(),
        Some("+55")
    );

    harness.click(Role::TextInput, "Contrast")?;
    harness.type_text("900")?;
    harness.press_key(Key::Enter, Modifiers::NONE)?;
    assert_eq!(harness.app().state.value, 100.0);
    assert_eq!(harness.app().state.finishes, 2);
    Ok(())
}

#[test]
fn escape_discards_what_was_typed_and_nonsense_is_ignored() -> TestResult {
    let mut harness = contrast(20.0);
    harness.run()?;

    harness.click(Role::TextInput, "Contrast")?;
    harness.type_text("-70")?;
    harness.press_key(Key::Escape, Modifiers::NONE)?;
    assert_eq!(harness.app().state.value, 20.0);

    harness.click(Role::TextInput, "Contrast")?;
    harness.type_text("lots")?;
    harness.press_key(Key::Enter, Modifiers::NONE)?;
    assert_eq!(harness.app().state.value, 20.0);
    assert_eq!(harness.app().state.finishes, 0);
    Ok(())
}

#[test]
fn leaving_the_field_commits_what_was_typed() -> TestResult {
    let mut harness = contrast(0.0);
    harness.run()?;

    harness.click(Role::TextInput, "Contrast")?;
    harness.type_text("-12")?;
    harness.press_key(Key::Tab, Modifiers::NONE)?;
    assert_eq!(harness.app().state.value, -12.0);
    assert_eq!(harness.app().state.finishes, 1);
    Ok(())
}

#[test]
fn a_text_value_set_through_accessibility_is_a_complete_edit() -> TestResult {
    let mut harness = contrast(0.0);
    harness.run()?;
    harness.set_text(Role::TextInput, "Contrast", "-20")?;
    assert_eq!(harness.app().state.value, -20.0);
    assert_eq!(harness.app().state.finishes, 1);
    Ok(())
}

#[test]
fn a_drag_changes_continuously_and_finishes_once() -> TestResult {
    let mut harness = contrast(0.0);
    harness.run()?;
    let track = harness.control(Role::Slider, "Contrast")?.rect;

    harness.pointer_down(across(track, 0.5))?;
    harness.pointer_move(across(track, 0.6))?;
    let midway = harness.app().state.value;
    assert!(midway > 10.0 && midway < 30.0, "{midway}");
    harness.pointer_move(across(track, 0.75))?;
    assert!(harness.app().state.changes >= 2);
    assert_eq!(
        harness.app().state.finishes,
        0,
        "not finished while the button is down"
    );

    harness.pointer_up(across(track, 0.75))?;
    let value = harness.app().state.value;
    assert!((value - 50.0).abs() <= 2.0, "{value}");
    assert_eq!(value, value.round(), "dragging lands on whole steps");
    assert_eq!(harness.app().state.finishes, 1);

    harness.step(5);
    assert_eq!(
        harness.app().state.finishes,
        1,
        "finished fires once per drag"
    );
    Ok(())
}

#[test]
fn pressing_the_handle_does_not_move_it() -> TestResult {
    let mut harness = contrast(40.0);
    harness.run()?;
    let track = harness.control(Role::Slider, "Contrast")?.rect;
    let handle = across(track, 0.7) + egui::vec2(2.0, 0.0);

    harness.pointer_down(handle)?;
    harness.pointer_up(handle)?;
    assert_eq!(harness.app().state.value, 40.0);
    assert_eq!(harness.app().state.changes, 0);
    assert_eq!(harness.app().state.finishes, 0);
    Ok(())
}

#[test]
fn clicking_the_track_moves_the_handle_there() -> TestResult {
    let mut harness = contrast(40.0);
    harness.run()?;
    let track = harness.control(Role::Slider, "Contrast")?.rect;

    harness.click_at(across(track, 0.25))?;
    let value = harness.app().state.value;
    assert!((value + 50.0).abs() <= 2.0, "{value}");
    assert_eq!(harness.app().state.finishes, 1);
    Ok(())
}

#[test]
fn a_log_scale_spreads_kelvin_along_the_track() -> TestResult {
    let mut harness = slider(Adj::Temperature.spec_for(FileKind::Raw), 5500.0, 5500.0);
    harness.app_mut().state.scale = Scale::LOG;
    harness.run()?;
    let track = harness.control(Role::Slider, "Temp")?.rect;

    // The geometric middle of 2000..50000 is 10000; the linear middle is 26000.
    harness.click_at(across(track, 0.5))?;
    let value = harness.app().state.value;
    assert!((9000.0..=11_000.0).contains(&value), "{value}");
    assert_eq!(value % 50.0, 0.0, "dragging lands on the 50 K step grid");
    Ok(())
}

#[test]
fn a_disabled_slider_ignores_input() -> TestResult {
    let mut harness = contrast(10.0);
    harness.app_mut().state.enabled = false;
    harness.run()?;
    let track = harness.control(Role::Slider, "Contrast")?;
    assert!(track.disabled);

    harness.set_value(Role::Slider, "Contrast", 60.0)?;
    harness.click_at(across(track.rect, 0.9))?;
    harness.set_text(Role::TextInput, "Contrast", "70")?;
    assert_eq!(harness.app().state.value, 10.0);
    assert_eq!(harness.app().state.changes, 0);
    Ok(())
}

#[test]
fn a_name_prefix_tells_sliders_with_the_same_label_apart() -> TestResult {
    let mut harness = HarnessBuilder::new().build_ui((0.0_f32, 0.0_f32), |ui, (sharpen, grain)| {
        AdjSlider::new(Adj::SharpenAmount.spec(), sharpen, 0.0)
            .name_prefix("Sharpening")
            .show(ui);
        AdjSlider::new(Adj::GrainAmount.spec(), grain, 0.0)
            .name_prefix("Grain")
            .compact()
            .show(ui);
    });
    harness.run()?;

    assert_eq!(harness.count(Role::Slider, "Amount"), 0);
    harness.set_value(Role::Slider, "Sharpening Amount", 40.0)?;
    harness.set_value(Role::Slider, "Grain Amount", 25.0)?;
    assert_eq!(harness.app().state, (40.0, 25.0));
    Ok(())
}

#[test]
fn pressing_the_handle_does_not_snap_a_value_between_steps() -> TestResult {
    // Kelvin moves in steps of 50, but a typed 5625 is a value like any other.
    let mut harness = slider(Adj::Temperature.spec_for(FileKind::Raw), 5625.0, 5500.0);
    harness.run()?;
    let track = harness.control(Role::Slider, "Temp")?.rect;
    let handle = across(track, (5625.0 - 2000.0) / 48_000.0);

    harness.pointer_down(handle)?;
    assert_eq!(harness.app().state.value, 5625.0);
    harness.pointer_move(handle + egui::vec2(20.0, 0.0))?;
    let dragged = harness.app().state.value;
    assert!(dragged > 5625.0 && dragged % 50.0 == 0.0, "{dragged}");
    harness.pointer_move(handle)?;
    harness.pointer_up(handle)?;
    assert_eq!(
        harness.app().state.value,
        5625.0,
        "back where it was grabbed"
    );
    Ok(())
}

#[test]
fn a_secondary_click_does_not_move_the_slider() -> TestResult {
    let mut harness = contrast(40.0);
    harness.run()?;
    let track = harness.control(Role::Slider, "Contrast")?.rect;

    harness.secondary_click_at(across(track, 0.1))?;
    assert_eq!(harness.app().state.value, 40.0);
    assert_eq!(harness.app().state.changes, 0);
    Ok(())
}

#[test]
fn one_click_too_many_does_not_undo_a_double_click_reset() -> TestResult {
    let mut harness = contrast(40.0);
    harness.run()?;
    let track = harness.control(Role::Slider, "Contrast")?.rect;
    let handle = across(track, 0.7);

    harness.double_click_at(handle)?;
    assert_eq!(harness.app().state.value, 0.0);
    // A third click right away, where the handle no longer is.
    harness.pointer_down(handle)?;
    harness.pointer_up(handle)?;
    assert_eq!(harness.app().state.value, 0.0);
    assert_eq!(harness.app().state.finishes, 1);

    // Later, a click there is a click again.
    harness.click_at(handle)?;
    assert!(harness.app().state.value > 30.0);
    Ok(())
}

#[test]
fn sliders_reset_one_after_another_by_double_clicking_each() -> TestResult {
    let values = (40.0_f32, -30.0_f32, 25.0_f32);
    let mut harness = HarnessBuilder::new().build_ui(values, |ui, (lights, darks, whites)| {
        AdjSlider::new(Adj::Highlights.spec(), lights, 0.0).show(ui);
        AdjSlider::new(Adj::Shadows.spec(), darks, 0.0).show(ui);
        AdjSlider::new(Adj::Whites.spec(), whites, 0.0).show(ui);
    });
    harness.run()?;
    let first = harness.control(Role::Slider, "Highlights")?.rect;
    let track = across(harness.control(Role::Slider, "Shadows")?.rect, 0.2);
    let whites = harness.control(Role::Slider, "Whites")?.rect;
    let label = egui::pos2(whites.left() - 40.0, whites.center().y);

    // Straight on from one to the next, as quickly as a hand moves: egui
    // counts the second click of each later double-click as a triple.
    harness.double_click_at(across(first, 0.7))?;
    for spot in [track, track, label, label] {
        harness.pointer_down(spot)?;
        harness.pointer_up(spot)?;
    }
    assert_eq!(harness.app().state, (0.0, 0.0, 0.0));
    Ok(())
}
