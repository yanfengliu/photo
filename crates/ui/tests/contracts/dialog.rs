//! The dialog frame: Enter and Escape, its buttons, what is behind it, and
//! its size as its body changes.

use egui::{Key, Modifiers, Role};
use photo_ui::harness::{Harness, HarnessBuilder, Host};
use photo_ui::widgets::{Dialog, DialogOutcome};

use crate::TestResult;

#[derive(Default)]
struct DialogProbe {
    open: bool,
    name: String,
    outcomes: Vec<DialogOutcome>,
    behind: u32,
}

fn dialog() -> Harness<Host<DialogProbe>> {
    let probe = DialogProbe {
        open: true,
        ..Default::default()
    };
    HarnessBuilder::new()
        .size([640.0, 420.0])
        .strict_glyphs(true)
        .build_ui(probe, |ui, probe| {
            if ui.button("Behind").clicked() {
                probe.behind += 1;
            }
            if !probe.open {
                return;
            }
            let response = Dialog::new("New Collection")
                .confirm_label("Create")
                .can_confirm(!probe.name.trim().is_empty())
                .show(ui.ctx(), |ui| {
                    ui.add(egui::TextEdit::singleline(&mut probe.name))
                        .accessible_name("Name");
                });
            if response.outcome != DialogOutcome::Open {
                probe.outcomes.push(response.outcome);
                probe.open = false;
            }
        })
}

#[test]
fn enter_confirms_a_dialog() -> TestResult {
    let mut harness = dialog();
    harness.run()?;

    harness.click(Role::TextInput, "Name")?;
    harness.type_text("Trips")?;
    assert!(harness.app().state.outcomes.is_empty());
    harness.press_key(Key::Enter, Modifiers::NONE)?;

    assert_eq!(harness.app().state.outcomes, [DialogOutcome::Confirmed]);
    assert_eq!(harness.app().state.name, "Trips");
    assert_eq!(
        harness.count(Role::Button, "Create"),
        0,
        "the dialog is gone"
    );
    Ok(())
}

#[test]
fn escape_cancels_a_dialog() -> TestResult {
    let mut harness = dialog();
    harness.run()?;
    harness.press_key(Key::Escape, Modifiers::NONE)?;
    assert_eq!(harness.app().state.outcomes, [DialogOutcome::Cancelled]);
    Ok(())
}

#[test]
fn the_buttons_confirm_and_cancel() -> TestResult {
    let mut harness = dialog();
    harness.app_mut().state.name = "Trips".to_owned();
    harness.run()?;
    harness.click(Role::Button, "Create")?;
    assert_eq!(harness.app().state.outcomes, [DialogOutcome::Confirmed]);

    let mut harness = dialog();
    harness.run()?;
    harness.click(Role::Button, "Cancel")?;
    assert_eq!(harness.app().state.outcomes, [DialogOutcome::Cancelled]);
    Ok(())
}

#[test]
fn a_dialog_that_cannot_be_confirmed_ignores_enter_and_its_confirm_button() -> TestResult {
    let mut harness = dialog();
    harness.run()?;
    assert!(harness.control(Role::Button, "Create")?.disabled);

    harness.press_key(Key::Enter, Modifiers::NONE)?;
    harness.click(Role::Button, "Create")?;
    assert!(harness.app().state.outcomes.is_empty());
    assert!(harness.app().state.open);
    Ok(())
}

#[test]
fn the_confirm_button_sits_to_the_right_of_cancel() -> TestResult {
    let mut harness = dialog();
    harness.run()?;
    let create = harness.control(Role::Button, "Create")?.rect;
    let cancel = harness.control(Role::Button, "Cancel")?.rect;
    assert!(cancel.right() < create.left(), "{cancel:?} {create:?}");
    assert_eq!(cancel.center().y, create.center().y);
    assert_eq!(cancel.size(), create.size());
    Ok(())
}

#[test]
fn a_dialog_whose_body_gets_shorter_closes_up_at_once() -> TestResult {
    let build = |extra: bool| {
        HarnessBuilder::new()
            .size([640.0, 420.0])
            .strict_glyphs(true)
            .build_ui(extra, |ui, extra: &mut bool| {
                let _ = Dialog::new("Export").show(ui.ctx(), |ui| {
                    ui.label("Body");
                    if *extra {
                        ui.label("One more row");
                    }
                });
            })
    };
    // From the body's last line down to the buttons.
    let gap = |harness: &Harness<Host<bool>>| -> Result<f32, Box<dyn std::error::Error>> {
        let body = harness.control(Role::Label, "Body")?.rect;
        let button = harness.control(Role::Button, "OK")?.rect;
        Ok(button.top() - body.bottom())
    };
    let mut short = build(false);
    short.run()?;
    let mut shrunk = build(true);
    shrunk.run()?;
    assert!(gap(&shrunk)? > gap(&short)?, "the row is between them");

    // The row goes away: one frame later nothing is left of it.
    shrunk.app_mut().state = false;
    shrunk.step(1);
    assert_eq!(gap(&shrunk)?, gap(&short)?);
    Ok(())
}

#[test]
fn a_dialog_blocks_what_is_behind_it() -> TestResult {
    let mut harness = dialog();
    harness.run()?;
    harness.click(Role::Button, "Behind")?;
    assert_eq!(harness.app().state.behind, 0);
    assert!(
        harness.app().state.open,
        "clicking outside does not dismiss it"
    );

    harness.press_key(Key::Escape, Modifiers::NONE)?;
    harness.click(Role::Button, "Behind")?;
    assert_eq!(harness.app().state.behind, 1);
    Ok(())
}

#[test]
fn enter_answers_only_the_dialog_on_top() -> TestResult {
    // Two dialogs, the second opened over the first.
    let mut harness = HarnessBuilder::new()
        .size([640.0, 420.0])
        .strict_glyphs(true)
        .build_ui(vec!["Export", "Replace Files"], |ui, open| {
            for title in open.clone() {
                let response = Dialog::new(title).show(ui.ctx(), |ui| {
                    ui.label("Body");
                });
                if response.outcome == DialogOutcome::Confirmed {
                    open.retain(|other| *other != title);
                }
            }
        });
    harness.run()?;

    harness.press_key(Key::Enter, Modifiers::NONE)?;
    assert_eq!(harness.app().state, ["Export"], "the one underneath stays");
    harness.press_key(Key::Enter, Modifiers::NONE)?;
    assert!(harness.app().state.is_empty());
    Ok(())
}

#[test]
fn the_key_press_that_opens_a_dialog_does_not_answer_it() -> TestResult {
    let state = (false, Vec::new());
    let mut harness = HarnessBuilder::new()
        .size([640.0, 420.0])
        .strict_glyphs(true)
        .build_ui(state, |ui, (open, outcomes)| {
            // The app opens its dialog on Enter, as a grid might for a photo.
            if !*open && ui.input(|input| input.key_pressed(Key::Enter)) {
                *open = true;
            }
            if *open {
                let response = Dialog::new("Open Photo").show(ui.ctx(), |ui| {
                    ui.label("Body");
                });
                if response.outcome != DialogOutcome::Open {
                    outcomes.push(response.outcome);
                    *open = false;
                }
            }
        });
    harness.run()?;

    harness.press_key(Key::Enter, Modifiers::NONE)?;
    assert!(harness.app().state.0, "the dialog is up");
    assert!(harness.app().state.1.is_empty(), "and still unanswered");
    harness.press_key(Key::Enter, Modifiers::NONE)?;
    assert_eq!(harness.app().state.1, [DialogOutcome::Confirmed]);
    assert!(!harness.app().state.0, "that Enter did not open it again");
    Ok(())
}

#[test]
fn a_held_down_enter_does_not_answer_a_dialog() -> TestResult {
    let mut harness = HarnessBuilder::new()
        .size([640.0, 420.0])
        .strict_glyphs(true)
        .build_ui(Vec::new(), |ui, outcomes: &mut Vec<DialogOutcome>| {
            // Enter held down since before the dialog opened: the system
            // repeats it, and every frame has a repeated press in it.
            ui.input_mut(|input| {
                input.events.push(egui::Event::Key {
                    key: Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: true,
                    modifiers: Modifiers::NONE,
                });
            });
            if outcomes.is_empty() {
                let response = Dialog::new("Replace Files").show(ui.ctx(), |ui| {
                    ui.label("Body");
                });
                if response.outcome != DialogOutcome::Open {
                    outcomes.push(response.outcome);
                }
            }
        });
    harness.run()?;
    harness.step(10);
    assert!(
        harness.app().state.is_empty(),
        "the repeats are not answers"
    );

    // A fresh press is.
    harness.press_key(Key::Enter, Modifiers::NONE)?;
    assert_eq!(harness.app().state, [DialogOutcome::Confirmed]);
    Ok(())
}
