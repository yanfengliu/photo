//! The number of a develop slider when the slider around it changes: left
//! without typing, edited from elsewhere, pressed mid-typing, taken away.
//!
//! The basics of typing a number (Enter, Escape, clamping) are with the
//! slider's own contracts in `slider.rs`.

use egui::{Key, Modifiers, Role};
use photo_core::develop::Adj;
use photo_ui::harness::HarnessBuilder;
use photo_ui::widgets::AdjSlider;

use crate::slider::{contrast, slider};
use crate::{TestResult, across};

#[test]
fn leaving_the_number_without_typing_changes_nothing() -> TestResult {
    // Finer than the two decimals shown: passing through must not round it.
    let mut harness = slider(Adj::Exposure.spec(), 0.354, 0.0);
    harness.run()?;

    harness.click(Role::TextInput, "Exposure")?;
    harness.press_key(Key::Tab, Modifiers::NONE)?;
    assert_eq!(harness.app().state.value, 0.354);
    assert_eq!(harness.app().state.changes, 0);
    assert_eq!(harness.app().state.finishes, 0);
    Ok(())
}

#[test]
fn the_number_follows_the_value_until_something_is_typed() -> TestResult {
    let mut harness = contrast(10.0);
    harness.run()?;

    harness.click(Role::TextInput, "Contrast")?;
    harness.set_value(Role::Slider, "Contrast", 40.0)?;
    let field = harness.control(Role::TextInput, "Contrast")?;
    assert_eq!(field.text.as_deref(), Some("+40"));
    harness.press_key(Key::Tab, Modifiers::NONE)?;
    assert_eq!(
        harness.app().state.value,
        40.0,
        "the old number is not put back"
    );
    assert_eq!(harness.app().state.finishes, 1);
    Ok(())
}

#[test]
fn a_press_on_the_track_while_typing_only_ends_the_typing() -> TestResult {
    let mut harness = contrast(0.0);
    harness.run()?;
    let track = harness.control(Role::Slider, "Contrast")?.rect;

    harness.click(Role::TextInput, "Contrast")?;
    harness.type_text("30")?;
    harness.click_at(across(track, 0.9))?;
    assert_eq!(
        harness.app().state.value,
        30.0,
        "the typed number, not the spot"
    );
    assert_eq!(harness.app().state.finishes, 1);
    Ok(())
}

#[test]
fn nothing_is_left_over_from_a_slider_that_went_away_mid_edit() -> TestResult {
    let mut harness = contrast(0.0);
    harness.run()?;
    let track = harness.control(Role::Slider, "Contrast")?.rect;

    // Typed but not committed when the slider's panel closes: thrown away.
    harness.click(Role::TextInput, "Contrast")?;
    harness.type_text("55")?;
    harness.app_mut().state.hidden = true;
    harness.step(3);
    harness.app_mut().state.hidden = false;
    harness.run()?;
    assert_eq!(harness.app().state.value, 0.0);

    // A drag under way when it closes is over: no "finished" turns up later.
    harness.pointer_down(across(track, 0.5))?;
    harness.pointer_move(across(track, 0.8))?;
    assert!(harness.app().state.value > 50.0);
    harness.app_mut().state.hidden = true;
    harness.pointer_up(across(track, 0.8))?;
    harness.step(3);
    harness.app_mut().state.hidden = false;
    harness.run()?;
    assert_eq!(harness.app().state.finishes, 0);
    Ok(())
}

#[test]
fn editing_the_number_does_not_disturb_the_widgets_after_the_slider() -> TestResult {
    let state = (0.0_f32, String::new());
    let mut harness = HarnessBuilder::new().build_ui(state, |ui, (value, note)| {
        AdjSlider::new(Adj::Contrast.spec(), value, 0.0).show(ui);
        // No id of its own: egui numbers it by its place among its siblings.
        ui.add(egui::TextEdit::singleline(note))
            .accessible_name("Note");
    });
    harness.run()?;

    // The number becomes a text field, then goes back to plain text as the
    // click below takes the focus away. The field below must stay itself.
    harness.click(Role::TextInput, "Contrast")?;
    harness.click(Role::TextInput, "Note")?;
    harness.type_text("kept")?;
    assert_eq!(harness.app().state.1, "kept");
    Ok(())
}
