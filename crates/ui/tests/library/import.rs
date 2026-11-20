//! Importing a folder through the Import dialog, with its path typed in.

use egui::{Key, Modifiers, Role};
use photo_engine::Source;
use photo_testkit::TempLibrary;
use photo_ui::PhotoApp;
use photo_ui::harness::Harness;

use crate::{Studio, TestResult, names, settle, shows};

/// Open the dialog with the keyboard and look in `library`'s folder.
fn scan(harness: &mut Harness<PhotoApp>, library: &TempLibrary) -> TestResult {
    harness.press_key(Key::I, Modifiers::COMMAND | Modifiers::SHIFT)?;
    assert!(harness.app().dialog_open());
    let root = library.root().display().to_string();
    harness.set_text(Role::TextInput, "Source folder", &root)?;
    settle(harness)?;
    Ok(())
}

#[test]
fn a_folder_imported_through_the_dialog_fills_the_grid_as_the_previous_import() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = studio.app();
    harness.run()?;
    scan(&mut harness, &library)?;

    for entry in library.entries() {
        let name = entry.path.file_name().unwrap().to_string_lossy().into_owned();
        let candidate = harness.control(Role::ListBoxOption, &format!("Import {name}"))?;
        assert_eq!(candidate.selected, Some(true), "{name} is ticked");
    }
    assert!(shows(&harness, "6 photos found, 6 ticked"));
    assert_eq!(harness.count(Role::ListBoxOption, "Import notes.txt"), 0);

    harness.click(Role::Button, "Import 6 Photos")?;
    assert!(!harness.app().dialog_open());
    settle(&mut harness)?;
    let engine = harness.app().engine();
    assert_eq!(engine.source(), &Source::PreviousImport);
    assert_eq!(engine.photo_count(), 6);
    assert_eq!(
        names(&harness),
        ["lake.png", "meadow.tif", "dawn.jpg", "ridge.jpg", "harbour.dng", "fireworks.jpg"]
    );
    for name in names(&harness) {
        assert_eq!(harness.count(Role::ListBoxOption, &name), 1, "{name} has a cell");
    }
    assert!(shows(&harness, "Previous Import"));
    assert!(shows(&harness, "6 of 6 photos, 1 selected"));
    assert!(harness.app().toasts().any(|toast| toast == "Imported 6 photos"));
    Ok(())
}

#[test]
fn an_unticked_candidate_is_left_out() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = studio.app();
    harness.run()?;
    scan(&mut harness, &library)?;

    harness.click(Role::ListBoxOption, "Import lake.png")?;
    assert_eq!(
        harness.control(Role::ListBoxOption, "Import lake.png")?.selected,
        Some(false)
    );
    harness.click(Role::Button, "Uncheck All")?;
    assert!(harness.control(Role::Button, "Import 0 Photos")?.disabled);
    harness.click(Role::Button, "Check All")?;
    harness.click(Role::ListBoxOption, "Import lake.png")?;
    harness.click(Role::Button, "Import 5 Photos")?;
    settle(&mut harness)?;
    let imported = names(&harness);
    assert_eq!(imported.len(), 5);
    assert!(!imported.iter().any(|name| name == "lake.png"));
    Ok(())
}

#[test]
fn a_second_look_at_the_folder_shows_what_is_already_in_the_catalog() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = studio.app_with(crate::files(&library));
    settle(&mut harness)?;
    scan(&mut harness, &library)?;
    let candidate = harness.control(Role::ListBoxOption, "Import dawn.jpg")?;
    assert_eq!(candidate.selected, Some(false), "already in the catalog");
    assert!(shows(&harness, "6 photos found, 0 ticked"));
    assert!(harness.control(Role::Button, "Import 0 Photos")?.disabled);
    Ok(())
}

#[test]
fn copies_go_into_dated_folders_with_the_keywords_asked_for() -> TestResult {
    let library = TempLibrary::new();
    let destination = tempfile::tempdir()?;
    let studio = Studio::new();
    let mut harness = studio.app();
    harness.run()?;
    scan(&mut harness, &library)?;

    harness.click(Role::RadioButton, "Copy to a folder by date")?;
    let target = destination.path().display().to_string();
    harness.set_text(Role::TextInput, "Destination folder", &target)?;
    assert!(shows_example(&harness), "the pattern is shown with an example");
    harness.set_text(Role::TextInput, "Import keywords", "spring, trip")?;
    harness.click(Role::Button, "Import 6 Photos")?;
    settle(&mut harness)?;

    let copy = destination.path().join("2024").join("2024-03-15").join("dawn.jpg");
    assert!(copy.is_file(), "{} was copied", copy.display());
    let engine = harness.app().engine();
    let dawn = engine
        .photos()
        .iter()
        .find(|photo| photo.file_name == "dawn.jpg")
        .ok_or("dawn.jpg was imported")?;
    assert_eq!(dawn.path, copy);
    let keywords: Vec<String> = engine
        .photo_keywords(dawn.id)?
        .into_iter()
        .map(|keyword| keyword.name)
        .collect();
    assert_eq!(keywords, ["spring", "trip"]);
    Ok(())
}

/// The pattern's example line, written with this platform's separators.
fn shows_example(harness: &Harness<PhotoApp>) -> bool {
    let example = std::path::Path::new("2024").join("2024-03-15").join("dawn.jpg");
    shows(harness, &format!("e.g. {}", example.display()))
}

#[test]
fn a_folder_that_is_not_there_is_said_so() -> TestResult {
    let studio = Studio::new();
    let mut harness = studio.app();
    harness.run()?;
    harness.click(Role::Button, "Import…")?;
    let missing = std::env::temp_dir().join("photo-no-such-folder-here");
    harness.set_text(Role::TextInput, "Source folder", &missing.display().to_string())?;
    let message = format!("There is no folder at {}.", missing.display());
    assert!(shows(&harness, &message));
    assert!(harness.control(Role::Button, "Import 0 Photos")?.disabled);
    Ok(())
}
