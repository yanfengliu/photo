//! Rating, flagging and labelling with the keyboard, filtering by each,
//! and undoing and redoing it all; the engine is asked after every step.

use egui::{Key, Modifiers, Role};
use photo_core::meta::{ColorLabel, Flag, Rating};
use photo_engine::{Facet, Filter};
use photo_testkit::TempLibrary;
use photo_ui::PhotoApp;
use photo_ui::harness::Harness;

use crate::{Studio, TestResult, cell, click_cell, files, id, names, settle};

/// The rating, flag and label the engine has for the photo `name`.
fn marks(harness: &Harness<PhotoApp>, name: &str) -> (Rating, Flag, ColorLabel) {
    let photo = id(harness, name);
    let summary = harness.app().engine().summary(photo).expect("listed");
    (summary.rating, summary.flag, summary.label)
}

fn key(harness: &mut Harness<PhotoApp>, key: Key) -> TestResult {
    harness.press_key(key, Modifiers::NONE)?;
    Ok(())
}

#[test]
fn keys_rate_flag_and_label_and_undo_and_redo_take_them_back_and_forth() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = studio.app_with(files(&library));
    settle(&mut harness)?;
    click_cell(&mut harness, "dawn.jpg", Modifiers::NONE)?;

    key(&mut harness, Key::Num3)?;
    assert_eq!(marks(&harness, "dawn.jpg").0, Rating::new(3));
    assert_eq!(cell(&harness, "dawn.jpg")?, "active, 3 stars");
    key(&mut harness, Key::CloseBracket)?;
    assert_eq!(marks(&harness, "dawn.jpg").0, Rating::new(4));
    key(&mut harness, Key::OpenBracket)?;
    key(&mut harness, Key::OpenBracket)?;
    assert_eq!(marks(&harness, "dawn.jpg").0, Rating::new(2));
    key(&mut harness, Key::P)?;
    assert_eq!(marks(&harness, "dawn.jpg").1, Flag::Picked);
    key(&mut harness, Key::X)?;
    assert_eq!(marks(&harness, "dawn.jpg").1, Flag::Rejected);
    key(&mut harness, Key::U)?;
    assert_eq!(marks(&harness, "dawn.jpg").1, Flag::Unflagged);
    key(&mut harness, Key::Num6)?;
    assert_eq!(marks(&harness, "dawn.jpg").2, ColorLabel::Red);
    key(&mut harness, Key::Num8)?;
    assert_eq!(marks(&harness, "dawn.jpg").2, ColorLabel::Green);
    assert_eq!(cell(&harness, "dawn.jpg")?, "active, 2 stars, green");
    assert!(harness.app().toasts().any(|toast| toast == "Set Color Label to Green"));

    let undo = Modifiers::COMMAND;
    let redo = Modifiers::COMMAND | Modifiers::SHIFT;
    harness.press_key(Key::Z, undo)?;
    assert_eq!(marks(&harness, "dawn.jpg").2, ColorLabel::Red);
    harness.press_key(Key::Z, undo)?;
    assert_eq!(marks(&harness, "dawn.jpg").2, ColorLabel::None);
    harness.press_key(Key::Z, undo)?;
    assert_eq!(marks(&harness, "dawn.jpg").1, Flag::Rejected);
    harness.press_key(Key::Z, redo)?;
    assert_eq!(marks(&harness, "dawn.jpg").1, Flag::Unflagged);
    harness.press_key(Key::Z, redo)?;
    assert_eq!(marks(&harness, "dawn.jpg").2, ColorLabel::Red);
    Ok(())
}

#[test]
fn the_toolbar_marks_the_selection_in_the_grid() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = studio.app_with(files(&library));
    settle(&mut harness)?;
    click_cell(&mut harness, "dawn.jpg", Modifiers::NONE)?;
    click_cell(&mut harness, "ridge.jpg", Modifiers::COMMAND)?;

    harness.click(Role::Button, "Rating 5")?;
    harness.click(Role::Button, "Flag pick")?;
    harness.click(Role::Button, "Label blue")?;
    for name in ["dawn.jpg", "ridge.jpg"] {
        assert_eq!(
            marks(&harness, name),
            (Rating::new(5), Flag::Picked, ColorLabel::Blue),
            "{name}"
        );
    }
    assert_eq!(marks(&harness, "lake.png").0, Rating::UNRATED);
    Ok(())
}

/// Give three photos different marks with the keyboard.
fn mark_three(harness: &mut Harness<PhotoApp>) -> TestResult {
    click_cell(harness, "dawn.jpg", Modifiers::NONE)?;
    key(harness, Key::Num4)?;
    key(harness, Key::P)?;
    click_cell(harness, "ridge.jpg", Modifiers::NONE)?;
    key(harness, Key::Num2)?;
    key(harness, Key::Num6)?;
    click_cell(harness, "harbour.dng", Modifiers::NONE)?;
    key(harness, Key::Num5)?;
    key(harness, Key::X)?;
    Ok(())
}

#[test]
fn the_attribute_filter_keeps_the_flags_ratings_and_labels_asked_for() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = studio.app_with(files(&library));
    settle(&mut harness)?;
    mark_three(&mut harness)?;
    harness.click(Role::Button, "Attribute filter")?;

    harness.click(Role::Button, "Show picked")?;
    assert_eq!(harness.app().engine().filter().flags, [Flag::Picked]);
    assert_eq!(names(&harness), ["dawn.jpg"]);
    harness.click(Role::Button, "Show rejected")?;
    assert_eq!(names(&harness), ["dawn.jpg", "harbour.dng"]);
    harness.click(Role::Button, "Show picked")?;
    harness.click(Role::Button, "Show rejected")?;
    assert_eq!(names(&harness).len(), 6);

    harness.click(Role::Button, "Filter rating 4")?;
    assert_eq!(names(&harness), ["dawn.jpg", "harbour.dng"], "at least four stars");
    harness.click(Role::ComboBox, "Rating comparison")?;
    harness.click(Role::Button, "exactly")?;
    assert_eq!(names(&harness), ["dawn.jpg"]);
    harness.click(Role::Button, "Filter rating 4")?;
    assert_eq!(harness.app().engine().filter().rating, None, "a second click clears it");

    harness.click(Role::Button, "Filter label red")?;
    assert_eq!(names(&harness), ["ridge.jpg"]);
    harness.click(Role::Button, "Filter label unlabelled")?;
    assert_eq!(names(&harness).len(), 6, "red or no label");

    harness.click(Role::Button, "No filter")?;
    assert_eq!(harness.app().engine().filter(), &Filter::default());
    assert_eq!(names(&harness).len(), 6);
    Ok(())
}

#[test]
fn the_text_filter_takes_what_is_typed_and_command_f_puts_the_keyboard_there() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = studio.app_with(files(&library));
    settle(&mut harness)?;

    harness.press_key(Key::F, Modifiers::COMMAND)?;
    assert!(harness.control(Role::TextInput, "Search text")?.focused);
    harness.type_text("ridge")?;
    assert_eq!(harness.app().engine().filter().text, "ridge");
    assert_eq!(names(&harness), ["ridge.jpg"]);
    // Typing into the field is not a shortcut: "e" did not open the loupe.
    assert_eq!(harness.app().view(), photo_ui::View::Grid);

    harness.set_text(Role::TextInput, "Search text", "")?;
    assert_eq!(names(&harness).len(), 6);
    Ok(())
}

#[test]
fn the_metadata_filter_narrows_by_camera_and_date() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = studio.app_with(files(&library));
    settle(&mut harness)?;
    harness.click(Role::Button, "Metadata filter")?;

    let cameras = harness.app().engine().facet_values(Facet::Camera)?;
    let camera = cameras
        .iter()
        .find(|value| !value.value.is_empty())
        .ok_or("a photo records its camera")?
        .clone();
    harness.click(Role::Button, &format!("Camera {}", camera.value))?;
    assert_eq!(harness.app().engine().filter().cameras, [camera.value.clone()]);
    assert_eq!(names(&harness).len() as u64, camera.count);
    harness.click(Role::Button, "Camera all")?;
    assert_eq!(names(&harness).len(), 6);

    harness.click(Role::Button, "Date 2024-07-04")?;
    assert_eq!(names(&harness), ["fireworks.jpg"]);
    assert!(
        harness.control(Role::Button, "ISO Unknown").is_err()
            || harness.control(Role::Button, "ISO Unknown")?.disabled,
        "a photo with no ISO cannot be chosen by its ISO"
    );
    harness.click(Role::Button, "Date all")?;
    assert_eq!(names(&harness).len(), 6);
    Ok(())
}
