//! Collections: made through their dialogs, filled by dragging photos onto
//! them, shown as the source; a smart collection that follows a rating; and
//! renaming, deleting and removing photos, each asked about first.

use egui::{Key, Modifiers, Role};
use photo_engine::{CollectionInfo, CollectionKind, Source};
use photo_testkit::TempLibrary;
use photo_ui::PhotoApp;
use photo_ui::harness::Harness;

use crate::{Studio, TestResult, click_cell, files, names, settle, shows};

/// The collection called `name`, as the engine lists it.
fn collection(harness: &Harness<PhotoApp>, name: &str) -> Option<CollectionInfo> {
    harness
        .app()
        .engine()
        .collections()
        .iter()
        .find(|collection| collection.name == name)
        .cloned()
}

#[test]
fn a_new_collection_takes_photos_dragged_onto_it_and_shows_them() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = studio.app_with(files(&library));
    settle(&mut harness)?;

    harness.press_key(Key::N, Modifiers::COMMAND)?;
    assert!(harness.control(Role::Button, "Create")?.disabled, "it needs a name");
    harness.type_text("Best of 2024")?;
    harness.press_key(Key::Enter, Modifiers::NONE)?;
    assert!(!harness.app().dialog_open());
    let best = collection(&harness, "Best of 2024").ok_or("the collection was made")?;
    assert_eq!((best.kind, best.photo_count), (CollectionKind::Regular, 0));

    click_cell(&mut harness, "dawn.jpg", Modifiers::NONE)?;
    click_cell(&mut harness, "fireworks.jpg", Modifiers::COMMAND)?;
    let from = harness.control(Role::ListBoxOption, "dawn.jpg")?.rect.center();
    let to = harness
        .control(Role::TreeItem, "Collection Best of 2024")?
        .rect
        .center();
    harness.drag(from, to)?;
    let best = collection(&harness, "Best of 2024").ok_or("still there")?;
    assert_eq!(best.photo_count, 2, "both selected photos went in");
    assert_eq!(
        harness
            .control(Role::TreeItem, "Collection Best of 2024")?
            .text
            .as_deref(),
        Some("2")
    );

    harness.click(Role::TreeItem, "Collection Best of 2024")?;
    assert_eq!(harness.app().engine().source(), &Source::Collection(best.id));
    assert_eq!(names(&harness), ["dawn.jpg", "fireworks.jpg"]);
    assert_eq!(
        harness
            .control(Role::TreeItem, "Collection Best of 2024")?
            .selected,
        Some(true)
    );

    // Delete in a collection takes photos out of it; nothing is asked.
    click_cell(&mut harness, "dawn.jpg", Modifiers::NONE)?;
    harness.press_key(Key::Backspace, Modifiers::NONE)?;
    assert!(!harness.app().dialog_open());
    assert_eq!(names(&harness), ["fireworks.jpg"]);
    assert_eq!(harness.app().engine().source_counts().all_photos, 6);
    Ok(())
}

#[test]
fn photos_dragged_onto_the_quick_collection_go_into_it_and_b_toggles_it() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = studio.app_with(files(&library));
    settle(&mut harness)?;
    let from = harness.control(Role::ListBoxOption, "ridge.jpg")?.rect.center();
    let to = harness.control(Role::TreeItem, "Quick Collection")?.rect.center();
    harness.drag(from, to)?;
    assert_eq!(harness.app().engine().source_counts().quick_collection, 1);
    assert!(crate::cell(&harness, "ridge.jpg")?.contains("in quick collection"));

    click_cell(&mut harness, "lake.png", Modifiers::NONE)?;
    harness.press_key(Key::B, Modifiers::NONE)?;
    assert_eq!(harness.app().engine().source_counts().quick_collection, 2);
    harness.press_key(Key::B, Modifiers::NONE)?;
    assert_eq!(harness.app().engine().source_counts().quick_collection, 1);
    Ok(())
}

#[test]
fn a_smart_collection_on_the_rating_follows_rating_changes() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = studio.app_with(files(&library));
    settle(&mut harness)?;

    harness.click(Role::Button, "Create collection")?;
    harness.click(Role::Button, "New Smart Collection…")?;
    harness.set_text(Role::TextInput, "Smart collection name", "Three and up")?;
    assert_eq!(
        harness.control(Role::ComboBox, "Rule 1 field")?.text.as_deref(),
        Some("Rating")
    );
    harness.click(Role::ComboBox, "Rule 1 operator")?;
    harness.click(Role::Button, "is at least")?;
    assert_eq!(
        harness.control(Role::Button, "Rule 1 rating 3")?.toggled,
        Some(true),
        "a rating rule starts at three stars"
    );
    harness.click(Role::Button, "Create")?;
    let smart = collection(&harness, "Three and up").ok_or("the smart collection was made")?;
    assert_eq!((smart.kind, smart.photo_count), (CollectionKind::Smart, 0));
    let rules = harness.app().engine().smart_rules(smart.id)?;
    assert_eq!(rules.rules.len(), 1);

    click_cell(&mut harness, "dawn.jpg", Modifiers::NONE)?;
    harness.press_key(Key::Num4, Modifiers::NONE)?;
    let smart = collection(&harness, "Three and up").ok_or("still there")?;
    assert_eq!(smart.photo_count, 1);

    harness.click(Role::TreeItem, "Smart Collection Three and up")?;
    assert_eq!(names(&harness), ["dawn.jpg"]);
    click_cell(&mut harness, "dawn.jpg", Modifiers::NONE)?;
    harness.press_key(Key::Num1, Modifiers::NONE)?;
    assert!(names(&harness).is_empty(), "a one-star photo leaves it");
    assert!(shows(&harness, "No photo matches its rules"));
    Ok(())
}

#[test]
fn a_collection_is_renamed_and_deleted_from_its_menu_and_the_delete_undone() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = studio.app_with(files(&library));
    settle(&mut harness)?;
    harness.click(Role::Button, "Create collection")?;
    harness.click(Role::Button, "New Collection Set…")?;
    harness.type_text("Trips")?;
    harness.press_key(Key::Enter, Modifiers::NONE)?;
    assert_eq!(
        collection(&harness, "Trips").map(|set| set.kind),
        Some(CollectionKind::Set)
    );

    harness.press_key(Key::N, Modifiers::COMMAND)?;
    harness.type_text("Iceland")?;
    harness.click(Role::ComboBox, "Inside set")?;
    harness.click(Role::Button, "Trips")?;
    harness.click(Role::Button, "Create")?;
    let trips = collection(&harness, "Trips").ok_or("the set")?;
    let iceland = collection(&harness, "Iceland").ok_or("the collection")?;
    assert_eq!(iceland.parent, Some(trips.id));

    let row = harness.control(Role::TreeItem, "Collection Iceland")?.rect.center();
    harness.secondary_click_at(row)?;
    harness.click(Role::Button, "Rename…")?;
    harness.set_text(Role::TextInput, "New name", "Iceland 2024")?;
    harness.click(Role::Button, "Rename")?;
    assert!(collection(&harness, "Iceland 2024").is_some());

    let row = harness.control(Role::TreeItem, "Collection Set Trips")?.rect.center();
    harness.secondary_click_at(row)?;
    harness.click(Role::Button, "Delete…")?;
    assert!(shows(&harness, "Delete the collection set \"Trips\" and everything inside it?"));
    harness.click(Role::Button, "Delete")?;
    assert!(collection(&harness, "Trips").is_none());
    assert!(collection(&harness, "Iceland 2024").is_none());

    harness.press_key(Key::Z, Modifiers::COMMAND)?;
    assert!(collection(&harness, "Trips").is_some());
    assert!(collection(&harness, "Iceland 2024").is_some());
    Ok(())
}

#[test]
fn removing_photos_from_the_catalog_is_asked_first_and_leaves_the_files() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = studio.app_with(files(&library));
    settle(&mut harness)?;
    click_cell(&mut harness, "ridge.jpg", Modifiers::NONE)?;

    harness.press_key(Key::Delete, Modifiers::NONE)?;
    assert!(harness.app().dialog_open());
    assert!(shows(&harness, "Remove 1 photo from the catalog?"));
    harness.press_key(Key::Escape, Modifiers::NONE)?;
    assert_eq!(names(&harness).len(), 6, "cancelled, nothing removed");

    harness.press_key(Key::Delete, Modifiers::NONE)?;
    harness.click(Role::Button, "Remove")?;
    assert_eq!(names(&harness).len(), 5);
    assert!(!names(&harness).iter().any(|name| name == "ridge.jpg"));
    let ridge = library
        .entries()
        .iter()
        .find(|entry| entry.path.ends_with("ridge.jpg"))
        .ok_or("the fixture")?;
    assert!(ridge.path.is_file(), "the file on disk is untouched");

    harness.press_key(Key::Z, Modifiers::COMMAND)?;
    assert_eq!(names(&harness).len(), 6);
    Ok(())
}
