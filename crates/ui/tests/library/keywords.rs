//! Keywords through Keywording and the Keyword List, and text through the
//! Metadata panel.

use egui::{Key, Modifiers, Role};
use photo_testkit::TempLibrary;
use photo_ui::PhotoApp;
use photo_ui::harness::Harness;

use crate::{Studio, TestResult, click_cell, files, id, settle};

/// The names of the keywords the photo `name` carries.
fn keywords(harness: &Harness<PhotoApp>, name: &str) -> Vec<String> {
    let photo = id(harness, name);
    harness
        .app()
        .engine()
        .photo_keywords(photo)
        .expect("the catalog answers")
        .into_iter()
        .map(|keyword| keyword.name)
        .collect()
}

#[test]
fn keywording_puts_typed_keywords_on_the_selection_and_takes_them_off() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = studio.app_with(files(&library));
    settle(&mut harness)?;
    click_cell(&mut harness, "dawn.jpg", Modifiers::NONE)?;
    click_cell(&mut harness, "ridge.jpg", Modifiers::COMMAND)?;

    harness.click(Role::TextInput, "Add keywords")?;
    harness.type_text("sunrise, coast")?;
    assert!(
        keywords(&harness, "dawn.jpg").is_empty(),
        "nothing until Enter"
    );
    harness.press_key(Key::Enter, Modifiers::NONE)?;
    for name in ["dawn.jpg", "ridge.jpg"] {
        assert_eq!(keywords(&harness, name), ["coast", "sunrise"], "{name}");
    }
    assert!(keywords(&harness, "lake.png").is_empty());
    assert_eq!(harness.count(Role::Button, "Remove keyword sunrise"), 1);

    harness.click(Role::Button, "Remove keyword sunrise")?;
    assert_eq!(keywords(&harness, "dawn.jpg"), ["coast"]);
    assert_eq!(keywords(&harness, "ridge.jpg"), ["coast"]);
    assert_eq!(harness.count(Role::Button, "Remove keyword sunrise"), 0);

    // A keyword only some of the selection carries is marked.
    click_cell(&mut harness, "lake.png", Modifiers::COMMAND)?;
    assert!(crate::shows(&harness, "coast*"));
    Ok(())
}

#[test]
fn a_tick_in_the_keyword_list_follows_the_selection_and_toggles_the_keyword() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = studio.app_with(files(&library));
    settle(&mut harness)?;
    click_cell(&mut harness, "dawn.jpg", Modifiers::NONE)?;
    harness.set_text(Role::TextInput, "Add keywords", "harbour")?;
    assert_eq!(keywords(&harness, "dawn.jpg"), ["harbour"]);
    assert_eq!(
        harness.control(Role::CheckBox, "Keyword harbour")?.toggled,
        Some(true)
    );

    click_cell(&mut harness, "fireworks.jpg", Modifiers::NONE)?;
    assert_eq!(
        harness.control(Role::CheckBox, "Keyword harbour")?.toggled,
        Some(false)
    );
    harness.click(Role::CheckBox, "Keyword harbour")?;
    assert_eq!(keywords(&harness, "fireworks.jpg"), ["harbour"]);
    let listed = harness.app().engine().keywords();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].photo_count, 2);

    harness.click(Role::CheckBox, "Keyword harbour")?;
    assert!(keywords(&harness, "fireworks.jpg").is_empty());
    assert_eq!(keywords(&harness, "dawn.jpg"), ["harbour"]);

    // The active photo carries it but another selected photo does not: the
    // tick is not set, and a click puts the keyword on both.
    click_cell(&mut harness, "dawn.jpg", Modifiers::COMMAND)?;
    assert_eq!(crate::active(&harness).as_deref(), Some("dawn.jpg"));
    assert_eq!(
        harness.control(Role::CheckBox, "Keyword harbour")?.toggled,
        Some(false)
    );
    harness.click(Role::CheckBox, "Keyword harbour")?;
    assert_eq!(keywords(&harness, "fireworks.jpg"), ["harbour"]);
    assert_eq!(keywords(&harness, "dawn.jpg"), ["harbour"]);
    assert_eq!(
        harness.control(Role::CheckBox, "Keyword harbour")?.toggled,
        Some(true)
    );
    Ok(())
}

#[test]
fn metadata_shows_the_file_and_saves_a_title_on_enter_or_when_set() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = studio.app_with(files(&library));
    settle(&mut harness)?;
    click_cell(&mut harness, "dawn.jpg", Modifiers::NONE)?;
    assert!(crate::shows(&harness, "1/250 s at f/5.6"));
    assert!(crate::shows(&harness, "2024-03-15 07:42:10"));
    assert!(crate::shows(&harness, "96 × 64"));

    harness.click(Role::TextInput, "Title")?;
    harness.type_text("First light")?;
    harness.press_key(Key::Enter, Modifiers::NONE)?;
    let dawn = id(&harness, "dawn.jpg");
    assert_eq!(harness.app().engine().photo(dawn)?.title, "First light");

    harness.set_text(Role::TextInput, "Caption", "Over the bay")?;
    assert_eq!(harness.app().engine().photo(dawn)?.caption, "Over the bay");
    assert_eq!(
        harness.control(Role::TextInput, "Creator")?.text.as_deref(),
        Some("Test Kit"),
        "the file's own creator"
    );

    harness.press_key(Key::Z, Modifiers::COMMAND)?;
    assert_eq!(harness.app().engine().photo(dawn)?.caption, "");
    assert_eq!(
        harness.control(Role::TextInput, "Caption")?.text.as_deref(),
        Some(""),
        "the field follows the undo"
    );
    Ok(())
}
