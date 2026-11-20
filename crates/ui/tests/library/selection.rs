//! Selecting by click, Command-click, Shift-click and the arrow keys, and
//! the views that show the selection: loupe, survey and compare.

use egui::{Key, Modifiers, Role};
use photo_testkit::TempLibrary;
use photo_ui::{View, Zoom};

use crate::{Studio, TestResult, active, click_cell, files, selected, settle};

#[test]
fn clicks_select_like_a_file_browser_and_arrows_move_the_active_photo() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = studio.app_with(files(&library));
    settle(&mut harness)?;
    // In the grid: lake, meadow, dawn, ridge, harbour, fireworks.

    click_cell(&mut harness, "meadow.tif", Modifiers::NONE)?;
    assert_eq!(selected(&harness), ["meadow.tif"]);
    assert_eq!(
        harness
            .control(egui::Role::ListBoxOption, "meadow.tif")?
            .selected,
        Some(true)
    );

    click_cell(&mut harness, "harbour.dng", Modifiers::SHIFT)?;
    assert_eq!(
        selected(&harness),
        ["meadow.tif", "dawn.jpg", "ridge.jpg", "harbour.dng"]
    );
    assert_eq!(active(&harness).as_deref(), Some("harbour.dng"));
    assert!(crate::shows(&harness, "6 of 6 photos, 4 selected"));

    click_cell(&mut harness, "ridge.jpg", Modifiers::COMMAND)?;
    assert_eq!(
        selected(&harness),
        ["meadow.tif", "dawn.jpg", "harbour.dng"]
    );
    assert_eq!(active(&harness).as_deref(), Some("harbour.dng"));
    click_cell(&mut harness, "ridge.jpg", Modifiers::COMMAND)?;
    assert_eq!(selected(&harness).len(), 4);
    assert_eq!(active(&harness).as_deref(), Some("ridge.jpg"));

    harness.press_key(Key::ArrowRight, Modifiers::NONE)?;
    assert_eq!(selected(&harness), ["harbour.dng"]);
    harness.press_key(Key::ArrowRight, Modifiers::SHIFT)?;
    assert_eq!(selected(&harness), ["harbour.dng", "fireworks.jpg"]);
    harness.press_key(Key::ArrowUp, Modifiers::NONE)?;
    assert_eq!(selected(&harness).len(), 1, "up moves a row, alone");

    harness.press_key(Key::A, Modifiers::COMMAND)?;
    assert_eq!(selected(&harness).len(), 6);
    harness.press_key(Key::D, Modifiers::COMMAND)?;
    assert!(selected(&harness).is_empty());
    Ok(())
}

#[test]
fn a_double_click_opens_the_loupe_where_space_and_z_zoom() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = studio.app_with(files(&library));
    settle(&mut harness)?;

    harness.double_click(Role::ListBoxOption, "ridge.jpg")?;
    assert_eq!(harness.app().view(), View::Loupe);
    assert_eq!(active(&harness).as_deref(), Some("ridge.jpg"));
    settle(&mut harness)?;
    assert_eq!(harness.app().zoom(), Zoom::Fit);
    harness.press_key(Key::Space, Modifiers::NONE)?;
    assert_eq!(harness.app().zoom(), Zoom::OneToOne);
    assert_eq!(harness.control(Role::Button, "1:1")?.toggled, Some(true));
    harness.press_key(Key::Z, Modifiers::NONE)?;
    assert_eq!(harness.app().zoom(), Zoom::Fit);
    harness.click(Role::Button, "Fill")?;
    assert_eq!(harness.app().zoom(), Zoom::Fill);

    harness.press_key(Key::ArrowRight, Modifiers::NONE)?;
    assert_eq!(active(&harness).as_deref(), Some("harbour.dng"));
    harness.press_key(Key::G, Modifiers::NONE)?;
    assert_eq!(harness.app().view(), View::Grid);
    harness.press_key(Key::E, Modifiers::NONE)?;
    assert_eq!(harness.app().view(), View::Loupe);
    Ok(())
}

#[test]
fn survey_tiles_the_selection_and_takes_photos_out_of_it() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = studio.app_with(files(&library));
    settle(&mut harness)?;
    click_cell(&mut harness, "dawn.jpg", Modifiers::NONE)?;
    click_cell(&mut harness, "harbour.dng", Modifiers::SHIFT)?;
    click_cell(&mut harness, "lake.png", Modifiers::COMMAND)?;

    harness.press_key(Key::N, Modifiers::NONE)?;
    assert_eq!(harness.app().view(), View::Survey);
    settle(&mut harness)?;
    for name in ["lake.png", "dawn.jpg", "ridge.jpg", "harbour.dng"] {
        assert_eq!(
            harness.count(Role::ListBoxOption, &format!("Survey {name}")),
            1,
            "{name}"
        );
    }
    assert_eq!(
        harness.count(Role::ListBoxOption, "Survey fireworks.jpg"),
        0
    );

    harness.click(Role::ListBoxOption, "Survey ridge.jpg")?;
    assert_eq!(active(&harness).as_deref(), Some("ridge.jpg"));
    assert_eq!(
        selected(&harness).len(),
        4,
        "a click in the survey keeps it"
    );
    harness.click(Role::Button, "Remove dawn.jpg from the survey")?;
    assert_eq!(selected(&harness), ["lake.png", "ridge.jpg", "harbour.dng"]);
    assert_eq!(harness.count(Role::ListBoxOption, "Survey dawn.jpg"), 0);
    Ok(())
}

#[test]
fn compare_shows_the_select_and_a_candidate_and_swaps_them() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = studio.app_with(files(&library));
    settle(&mut harness)?;
    click_cell(&mut harness, "dawn.jpg", Modifiers::NONE)?;
    click_cell(&mut harness, "harbour.dng", Modifiers::COMMAND)?;
    click_cell(&mut harness, "dawn.jpg", Modifiers::COMMAND)?;
    click_cell(&mut harness, "dawn.jpg", Modifiers::COMMAND)?;
    assert_eq!(active(&harness).as_deref(), Some("dawn.jpg"));

    harness.press_key(Key::C, Modifiers::NONE)?;
    assert_eq!(harness.app().view(), View::Compare);
    settle(&mut harness)?;
    assert_eq!(harness.count(Role::ListBoxOption, "Select dawn.jpg"), 1);
    assert_eq!(
        harness.count(Role::ListBoxOption, "Candidate harbour.dng"),
        1
    );

    harness.click(Role::Button, "Swap")?;
    assert_eq!(harness.count(Role::ListBoxOption, "Select harbour.dng"), 1);
    assert_eq!(harness.count(Role::ListBoxOption, "Candidate dawn.jpg"), 1);

    harness.press_key(Key::ArrowRight, Modifiers::NONE)?;
    assert_eq!(
        harness.count(Role::ListBoxOption, "Candidate ridge.jpg"),
        1,
        "the candidate steps along the list, over the select"
    );
    harness.click(Role::Button, "Make Select")?;
    settle(&mut harness)?;
    assert_eq!(harness.count(Role::ListBoxOption, "Select ridge.jpg"), 1);
    assert_eq!(
        harness.count(Role::ListBoxOption, "Candidate harbour.dng"),
        1
    );
    assert_eq!(active(&harness).as_deref(), Some("ridge.jpg"));

    harness.click(Role::ListBoxOption, "Candidate harbour.dng")?;
    assert_eq!(active(&harness).as_deref(), Some("harbour.dng"));
    harness.press_key(Key::Num2, Modifiers::NONE)?;
    let engine = harness.app().engine();
    let rated: Vec<_> = engine
        .photos()
        .iter()
        .filter(|photo| photo.rating.stars() == 2)
        .map(|photo| photo.file_name.as_str())
        .collect();
    assert_eq!(
        rated,
        ["harbour.dng"],
        "keys in Compare apply to the active photo only"
    );
    Ok(())
}
