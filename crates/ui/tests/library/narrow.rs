//! The smallest window the app allows, 1100 x 700 points. In every view
//! the toolbar's controls, and the filter bar's, lie inside the work area
//! and none on another. With both side columns open the least needed may
//! give way (sort order, rotation, the colour labels); Tab hides the
//! columns and brings every one of them back.

use std::ops::RangeInclusive;

use egui::{Key, Modifiers, Rect, Role};
use photo_testkit::TempLibrary;
use photo_ui::PhotoApp;
use photo_ui::harness::Harness;

use crate::snapshots::snap;
use crate::{Studio, TestResult, click_cell, files, settle};

/// The smallest window the app allows.
const SMALLEST: [f32; 2] = [1100.0, 700.0];

/// The toolbar controls of every view, shown at any width.
const ALWAYS: [(Role, &str); 5] = [
    (Role::Button, "Grid view"),
    (Role::Button, "Survey view"),
    (Role::Button, "Flag pick"),
    (Role::Button, "Flag reject"),
    (Role::Button, "Rating 5"),
];

/// The toolbar controls that may give way while the columns are open.
const OPTIONAL: [(Role, &str); 4] = [
    (Role::Button, "Label red"),
    (Role::Button, "Label purple"),
    (Role::Button, "Rotate left"),
    (Role::Button, "Rotate right"),
];

/// The view's own toolbar controls, shown at any width, and whether the
/// view offers the sort order.
fn own(view: Key) -> (&'static [(Role, &'static str)], bool) {
    match view {
        Key::G => (&[(Role::Slider, "Thumbnail size")], true),
        Key::E => (&[(Role::Button, "Fit"), (Role::Button, "1:1")], false),
        Key::C => (
            &[(Role::Button, "Swap"), (Role::Button, "Make Select")],
            false,
        ),
        _ => (&[], false),
    }
}

/// The filter bar's tabs and the Attribute tab's conditions.
const ATTRIBUTE_FILTER: [(Role, &str); 11] = [
    (Role::Button, "Text filter"),
    (Role::Button, "Metadata filter"),
    (Role::Button, "No filter"),
    (Role::Button, "Show picked"),
    (Role::Button, "Show rejected"),
    (Role::Button, "Filter rating 5"),
    (Role::Button, "Filter label red"),
    (Role::Button, "Filter label unlabelled"),
    (Role::ComboBox, "File kind filter"),
    (Role::ComboBox, "Copies filter"),
    (Role::ComboBox, "Edited filter"),
];

/// The Metadata tab's columns.
const METADATA_FILTER: [(Role, &str); 4] = [
    (Role::Button, "Date all"),
    (Role::Button, "Camera all"),
    (Role::Button, "Lens all"),
    (Role::Button, "ISO all"),
];

#[test]
fn the_smallest_window_keeps_every_control_inside_the_work_area_and_apart() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = studio.app_with_size(files(&library), SMALLEST);
    settle(&mut harness)?;
    click_cell(&mut harness, "dawn.jpg", Modifiers::NONE)?;
    click_cell(&mut harness, "ridge.jpg", Modifiers::COMMAND)?;

    // Both columns open: the work area lies between them.
    let left = harness
        .control(Role::TreeItem, "All Photographs")?
        .rect
        .right();
    let right = harness
        .control(Role::TextInput, "Add keywords")?
        .rect
        .left();
    harness.click(Role::Button, "Metadata filter")?;
    apart(&harness, &METADATA_FILTER, left..=right)?;
    harness.click(Role::Button, "Attribute filter")?;
    apart(&harness, &ATTRIBUTE_FILTER, left..=right)?;
    snap(&mut harness, "narrow", 1)?;
    for view in [Key::E, Key::C, Key::N, Key::G] {
        harness.press_key(view, Modifiers::NONE)?;
        apart(&harness, &toolbar(&harness, view, false), left..=right)?;
        if view == Key::C {
            snap(&mut harness, "narrow-compare", 1)?;
        }
    }

    harness.press_key(Key::Tab, Modifiers::NONE)?;
    assert!(!harness.app().layout().left && !harness.app().layout().right);
    let window = 0.0..=SMALLEST[0];
    for view in [Key::E, Key::C, Key::N, Key::G] {
        harness.press_key(view, Modifiers::NONE)?;
        apart(&harness, &toolbar(&harness, view, true), window.clone())?;
    }
    apart(&harness, &ATTRIBUTE_FILTER, window)?;
    snap(&mut harness, "narrow-no-panels", 1)
}

/// The toolbar controls `view` must show: those shown at any width, and
/// with `everything` the ones that give way too; without it, those of them
/// that are shown.
fn toolbar(harness: &Harness<PhotoApp>, view: Key, everything: bool) -> Vec<(Role, &'static str)> {
    let (own, sorts) = own(view);
    let mut controls: Vec<(Role, &str)> = ALWAYS.iter().chain(own).copied().collect();
    let sort = sorts.then_some((Role::ComboBox, "Sort order"));
    controls.extend(
        OPTIONAL
            .into_iter()
            .chain(sort)
            .filter(|(role, name)| everything || harness.count(*role, name) > 0),
    );
    controls
}

/// Each of `controls` is shown, lies inside the window and between the
/// horizontal bounds `x`, and on none of the others.
fn apart(
    harness: &Harness<PhotoApp>,
    controls: &[(Role, &str)],
    x: RangeInclusive<f32>,
) -> TestResult {
    let mut seen: Vec<(&str, Rect)> = Vec::new();
    for (role, name) in controls {
        let rect = harness.control(*role, name)?.rect;
        let inside = rect.left() >= *x.start()
            && rect.right() <= *x.end()
            && rect.top() >= 0.0
            && rect.bottom() <= SMALLEST[1];
        assert!(inside, "{name} {rect:?} is outside {x:?}");
        for (other, other_rect) in &seen {
            // Touching is fine (a row of swatches); covering is not.
            let shared = rect.intersect(*other_rect);
            assert!(
                shared.width() < 0.5 || shared.height() < 0.5,
                "{name} {rect:?} lies on {other} {other_rect:?}"
            );
        }
        seen.push((name, rect));
    }
    Ok(())
}
