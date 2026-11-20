//! The thumbnail cell, the toast overlay and the dialog frame.

use egui::{Key, Modifiers, PointerButton, Role, vec2};
use photo_core::meta::{ColorLabel, Flag, Rating};
use photo_ui::harness::{Harness, HarnessBuilder, Host};
use photo_ui::widgets::{Badges, Dialog, DialogOutcome, SelectionTier, Thumbnail, Toasts};

use crate::TestResult;

// ---- Thumbnail cell ----

/// What a cell reported, with the modifiers held at the time.
#[derive(Clone, Debug, PartialEq)]
struct CellEvent {
    cell: &'static str,
    kind: &'static str,
    shift: bool,
    command: bool,
}

fn event(cell: &'static str, kind: &'static str) -> CellEvent {
    CellEvent {
        cell,
        kind,
        shift: false,
        command: false,
    }
}

fn cells() -> Harness<Host<Vec<CellEvent>>> {
    HarnessBuilder::new()
        .size([400.0, 220.0])
        .strict_glyphs(true)
        .build_ui(Vec::new(), |ui, events: &mut Vec<CellEvent>| {
            ui.horizontal(|ui| {
                for (name, tier) in [
                    ("IMG_0001.jpg", SelectionTier::Active),
                    ("IMG_0002.jpg", SelectionTier::None),
                ] {
                    let response = Thumbnail::new(name, [160.0, 160.0])
                        .tier(tier)
                        .rating(Rating::new(3))
                        .flag(Flag::Picked)
                        .label(ColorLabel::Green)
                        .badges(Badges {
                            edited: true,
                            ..Default::default()
                        })
                        .index(1)
                        .show(ui);
                    for (happened, kind) in [
                        (response.clicked, "click"),
                        (response.double_clicked, "double-click"),
                        (response.secondary_clicked, "secondary-click"),
                        (response.drag_started, "drag"),
                    ] {
                        if happened {
                            events.push(CellEvent {
                                cell: name,
                                kind,
                                shift: response.modifiers.shift,
                                command: response.modifiers.command,
                            });
                        }
                    }
                }
            });
        })
}

#[test]
fn a_click_and_a_double_click_are_told_apart() -> TestResult {
    let mut harness = cells();
    harness.run()?;

    harness.click(Role::ListBoxOption, "IMG_0002.jpg")?;
    assert_eq!(harness.app().state, [event("IMG_0002.jpg", "click")]);

    harness.app_mut().state.clear();
    harness.double_click(Role::ListBoxOption, "IMG_0001.jpg")?;
    assert_eq!(
        harness.app().state,
        [
            event("IMG_0001.jpg", "click"),
            event("IMG_0001.jpg", "double-click")
        ],
        "the first click of a double-click is a click; the second is the double-click"
    );
    Ok(())
}

#[test]
fn clicks_carry_their_modifiers() -> TestResult {
    let mut harness = cells();
    harness.run()?;
    let cell = harness
        .control(Role::ListBoxOption, "IMG_0002.jpg")?
        .rect
        .center();

    harness.click_at_with(cell, PointerButton::Primary, Modifiers::SHIFT)?;
    harness.click_at_with(cell, PointerButton::Primary, Modifiers::COMMAND)?;

    let events = &harness.app().state;
    assert_eq!(events.len(), 2, "{events:?}");
    assert!(events[0].kind == "click" && events[0].shift && !events[0].command);
    assert!(events[1].kind == "click" && events[1].command && !events[1].shift);
    Ok(())
}

#[test]
fn a_secondary_click_is_reported_as_such() -> TestResult {
    let mut harness = cells();
    harness.run()?;
    let cell = harness
        .control(Role::ListBoxOption, "IMG_0001.jpg")?
        .rect
        .center();

    harness.secondary_click_at(cell)?;
    assert_eq!(
        harness.app().state,
        [event("IMG_0001.jpg", "secondary-click")]
    );
    Ok(())
}

#[test]
fn a_drag_start_is_reported_once_and_is_not_a_click() -> TestResult {
    let mut harness = cells();
    harness.run()?;
    let cell = harness
        .control(Role::ListBoxOption, "IMG_0001.jpg")?
        .rect
        .center();

    harness.drag(cell, cell + vec2(60.0, 30.0))?;
    assert_eq!(harness.app().state, [event("IMG_0001.jpg", "drag")]);
    Ok(())
}

#[test]
fn a_focused_cell_is_activated_from_the_keyboard() -> TestResult {
    let mut harness = cells();
    harness.run()?;
    harness.focus(Role::ListBoxOption, "IMG_0002.jpg")?;
    harness.press_key(Key::Enter, Modifiers::NONE)?;
    assert_eq!(harness.app().state, [event("IMG_0002.jpg", "click")]);
    Ok(())
}

#[test]
fn a_cell_describes_its_state_to_the_accessibility_tree() -> TestResult {
    let mut harness = cells();
    harness.run()?;

    let active = harness.control(Role::ListBoxOption, "IMG_0001.jpg")?;
    assert_eq!(active.selected, Some(true));
    assert_eq!(
        active.text.as_deref(),
        Some("active, 3 stars, pick, green, edited")
    );
    assert_eq!((active.rect.width(), active.rect.height()), (160.0, 160.0));
    let other = harness.control(Role::ListBoxOption, "IMG_0002.jpg")?;
    assert_eq!(other.selected, Some(false));
    Ok(())
}

// ---- Toast overlay ----

#[test]
fn a_toast_shows_and_goes_away_on_its_own() -> TestResult {
    let mut harness = HarnessBuilder::new()
        .size([400.0, 300.0])
        .strict_glyphs(true)
        .build_ui(Toasts::new(), |ui, toasts| {
            if ui.button("Rate").clicked() {
                toasts.push("Set Rating to 3");
            }
            toasts.show(ui.ctx(), ui.max_rect());
        });
    harness.run()?;
    assert_eq!(harness.count(Role::Label, "Set Rating to 3"), 0);

    harness.click(Role::Button, "Rate")?;
    assert_eq!(harness.count(Role::Label, "Set Rating to 3"), 1);
    assert!(!harness.app().state.is_empty());

    harness.advance(1.0);
    assert_eq!(
        harness.count(Role::Label, "Set Rating to 3"),
        1,
        "still up after a second"
    );
    harness.advance(2.0);
    harness.run()?;
    assert_eq!(harness.count(Role::Label, "Set Rating to 3"), 0);
    assert!(harness.app().state.is_empty());
    Ok(())
}

#[test]
fn a_toast_does_not_take_clicks_meant_for_what_is_under_it() -> TestResult {
    let mut harness = HarnessBuilder::new().size([400.0, 300.0]).build_ui(
        (Toasts::new(), 0_u32),
        |ui, (toasts, presses)| {
            let everything = ui.available_size();
            if ui
                .add_sized(everything, egui::Button::new("Canvas"))
                .clicked()
            {
                *presses += 1;
            }
            toasts.show(ui.ctx(), ui.max_rect());
        },
    );
    harness.app_mut().state.0.push("Exported 12 photos");
    harness.run()?;
    let toast = harness
        .control(Role::Label, "Exported 12 photos")?
        .rect
        .center();

    harness.click_at(toast)?;
    assert_eq!(harness.app().state.1, 1);
    Ok(())
}

// ---- Dialog frame ----

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
