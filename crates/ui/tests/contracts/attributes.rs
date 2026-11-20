//! Rating stars, the flag toggle, colour labels and icon buttons.

use egui::Role;
use photo_core::meta::{ColorLabel, Flag, Rating};
use photo_ui::harness::{Harness, HarnessBuilder, Host};
use photo_ui::widgets::{Icon, IconButton, color_labels, flag_toggle, icon_button, rating_stars};

use crate::TestResult;

#[derive(Default)]
struct Attributes {
    rating: Rating,
    flag: Flag,
    label: ColorLabel,
    changes: u32,
    zooms: u32,
    deletes: u32,
    grid: bool,
}

fn attributes() -> Harness<Host<Attributes>> {
    HarnessBuilder::new()
        .size([360.0, 200.0])
        .strict_glyphs(true)
        .build_ui(Attributes::default(), |ui, state| {
            let rating = rating_stars(ui, "Rating", &mut state.rating);
            let flag = flag_toggle(ui, "Flag", &mut state.flag);
            let label = color_labels(ui, "Label", &mut state.label);
            state.changes += [rating, flag, label]
                .iter()
                .map(|response| u32::from(response.changed()))
                .sum::<u32>();

            ui.horizontal(|ui| {
                if icon_button(ui, Icon::Zoom, "Zoom in").clicked() {
                    state.zooms += 1;
                }
                if ui
                    .add_enabled(false, IconButton::new(Icon::Trash, "Delete photo"))
                    .clicked()
                {
                    state.deletes += 1;
                }
                let grid = IconButton::new(Icon::Grid, "Grid view").selected(state.grid);
                if ui.add(grid).clicked() {
                    state.grid = !state.grid;
                }
            });
        })
}

#[test]
fn clicking_star_n_sets_the_rating_to_n() -> TestResult {
    let mut harness = attributes();
    harness.run()?;

    harness.click(Role::Button, "Rating 3")?;
    assert_eq!(harness.app().state.rating.stars(), 3);
    assert_eq!(
        harness.control(Role::Button, "Rating 2")?.toggled,
        Some(true)
    );
    assert_eq!(
        harness.control(Role::Button, "Rating 3")?.toggled,
        Some(true)
    );
    assert_eq!(
        harness.control(Role::Button, "Rating 4")?.toggled,
        Some(false)
    );

    harness.click(Role::Button, "Rating 5")?;
    assert_eq!(harness.app().state.rating.stars(), 5);
    harness.invoke(Role::Button, "Rating 1")?;
    assert_eq!(harness.app().state.rating.stars(), 1);
    assert_eq!(harness.app().state.changes, 3);
    Ok(())
}

#[test]
fn clicking_the_current_rating_clears_it() -> TestResult {
    let mut harness = attributes();
    harness.run()?;

    harness.click(Role::Button, "Rating 4")?;
    harness.click(Role::Button, "Rating 4")?;
    assert_eq!(harness.app().state.rating, Rating::UNRATED);
    assert_eq!(
        harness.control(Role::Button, "Rating 1")?.toggled,
        Some(false)
    );
    assert_eq!(harness.app().state.changes, 2);
    Ok(())
}

#[test]
fn the_flag_toggle_sets_switches_and_removes_the_flag() -> TestResult {
    let mut harness = attributes();
    harness.run()?;

    harness.click(Role::Button, "Flag pick")?;
    assert_eq!(harness.app().state.flag, Flag::Picked);
    assert_eq!(
        harness.control(Role::Button, "Flag pick")?.toggled,
        Some(true)
    );
    assert_eq!(
        harness.control(Role::Button, "Flag reject")?.toggled,
        Some(false)
    );

    harness.click(Role::Button, "Flag reject")?;
    assert_eq!(harness.app().state.flag, Flag::Rejected);
    harness.click(Role::Button, "Flag reject")?;
    assert_eq!(harness.app().state.flag, Flag::Unflagged);
    assert_eq!(harness.app().state.changes, 3);
    Ok(())
}

#[test]
fn a_colour_label_is_set_by_its_swatch_and_cleared_by_clicking_it_again() -> TestResult {
    let mut harness = attributes();
    harness.run()?;

    harness.click(Role::Button, "Label green")?;
    assert_eq!(harness.app().state.label, ColorLabel::Green);
    assert_eq!(
        harness.control(Role::Button, "Label green")?.toggled,
        Some(true)
    );
    assert_eq!(
        harness.control(Role::Button, "Label red")?.toggled,
        Some(false)
    );

    harness.click(Role::Button, "Label purple")?;
    assert_eq!(harness.app().state.label, ColorLabel::Purple);
    harness.click(Role::Button, "Label purple")?;
    assert_eq!(harness.app().state.label, ColorLabel::None);
    Ok(())
}

#[test]
fn an_icon_button_is_found_by_its_label_and_reports_clicks() -> TestResult {
    let mut harness = attributes();
    harness.run()?;

    harness.click(Role::Button, "Zoom in")?;
    harness.invoke(Role::Button, "Zoom in")?;
    assert_eq!(harness.app().state.zooms, 2);
    Ok(())
}

#[test]
fn a_disabled_icon_button_does_not_click() -> TestResult {
    let mut harness = attributes();
    harness.run()?;

    assert!(harness.control(Role::Button, "Delete photo")?.disabled);
    harness.click(Role::Button, "Delete photo")?;
    harness.invoke(Role::Button, "Delete photo")?;
    assert_eq!(harness.app().state.deletes, 0);
    Ok(())
}

#[test]
fn a_toggle_icon_button_exposes_whether_it_is_on() -> TestResult {
    let mut harness = attributes();
    harness.run()?;

    assert_eq!(
        harness.control(Role::Button, "Grid view")?.toggled,
        Some(false)
    );
    assert_eq!(
        harness.control(Role::Button, "Zoom in")?.toggled,
        None,
        "not a toggle"
    );
    harness.click(Role::Button, "Grid view")?;
    assert_eq!(
        harness.control(Role::Button, "Grid view")?.toggled,
        Some(true)
    );
    Ok(())
}

#[test]
fn an_icon_button_explains_itself_in_a_tooltip() -> TestResult {
    let mut harness = attributes();
    harness.run()?;
    assert_eq!(harness.count(Role::Label, "Zoom in"), 0);

    let button = harness.control(Role::Button, "Zoom in")?.rect.center();
    harness.hover_at(button)?;
    harness.advance(1.5);
    assert_eq!(harness.count(Role::Label, "Zoom in"), 1);
    Ok(())
}
