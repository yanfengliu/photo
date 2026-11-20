//! The shell: the empty catalog, the module picker, and hiding the edges.

use egui::{Key, Modifiers, Role};
use photo_testkit::TempLibrary;
use photo_ui::{Module, View};

use crate::{Studio, TestResult, files, settle, shows};

#[test]
fn an_empty_catalog_says_to_import_and_offers_the_way() -> TestResult {
    let studio = Studio::new();
    let mut harness = studio.app();
    harness.run()?;
    assert_eq!(harness.app().module(), Module::Library);
    assert_eq!(harness.app().view(), View::Grid);
    assert!(shows(&harness, "Import photos to get started"));

    harness.click(Role::Button, "Import Photos…")?;
    assert!(harness.app().dialog_open());
    assert_eq!(harness.count(Role::TextInput, "Source folder"), 1);
    harness.press_key(Key::Escape, Modifiers::NONE)?;
    assert!(!harness.app().dialog_open());
    Ok(())
}

#[test]
fn develop_is_shown_as_the_next_module_but_cannot_be_chosen() -> TestResult {
    let studio = Studio::new();
    let mut harness = studio.app();
    harness.run()?;
    assert_eq!(harness.control(Role::Button, "Library")?.toggled, Some(true));
    let develop = harness.control(Role::Button, "Develop")?;
    assert!(develop.disabled);

    harness.hover_at(develop.rect.center())?;
    harness.advance(1.5);
    assert!(shows(&harness, "Develop is the next module to be built."));
    harness.click(Role::Button, "Develop")?;
    assert_eq!(harness.app().module(), Module::Library);
    Ok(())
}

#[test]
fn tab_toggles_the_side_columns_shift_tab_every_edge_and_t_the_toolbar() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = studio.app_with(files(&library));
    settle(&mut harness)?;
    let panels = |harness: &photo_ui::harness::Harness<photo_ui::PhotoApp>| {
        (
            harness.count(Role::DisclosureTriangle, "Navigator"),
            harness.count(Role::DisclosureTriangle, "Metadata"),
        )
    };
    assert_eq!(panels(&harness), (1, 1));

    harness.press_key(Key::Tab, Modifiers::NONE)?;
    let layout = harness.app().layout();
    assert!(!layout.left && !layout.right && layout.top_bar && layout.filmstrip);
    assert_eq!(panels(&harness), (0, 0));
    assert!(
        harness.ctx().memory(|memory| memory.focused().is_none()),
        "Tab moved no keyboard focus"
    );
    harness.press_key(Key::Tab, Modifiers::NONE)?;
    assert_eq!(panels(&harness), (1, 1));

    harness.press_key(Key::Tab, Modifiers::SHIFT)?;
    let layout = harness.app().layout();
    assert!(!layout.top_bar && !layout.left && !layout.right && !layout.filmstrip);
    assert_eq!(harness.count(Role::Button, "Library"), 0, "the top bar is gone");
    assert_eq!(harness.count(Role::ListBoxOption, "Filmstrip dawn.jpg"), 0);
    assert_eq!(harness.count(Role::ListBoxOption, "dawn.jpg"), 1, "the grid stays");
    harness.press_key(Key::Tab, Modifiers::SHIFT)?;
    assert_eq!(harness.count(Role::Button, "Library"), 1);
    assert_eq!(harness.count(Role::ListBoxOption, "Filmstrip dawn.jpg"), 1);

    harness.press_key(Key::T, Modifiers::NONE)?;
    assert!(!harness.app().layout().toolbar);
    assert_eq!(harness.count(Role::Button, "Grid view"), 0);
    harness.press_key(Key::T, Modifiers::NONE)?;
    assert_eq!(harness.count(Role::Button, "Grid view"), 1);
    Ok(())
}

#[test]
fn each_edge_hides_and_comes_back_with_its_own_button() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = studio.app_with(files(&library));
    settle(&mut harness)?;
    for (hide, show) in [
        ("Hide left panels", "Show left panels"),
        ("Hide right panels", "Show right panels"),
        ("Hide filmstrip", "Show filmstrip"),
        ("Hide top bar", "Show top bar"),
    ] {
        harness.click(Role::Button, hide)?;
        assert_eq!(harness.count(Role::Button, hide), 0, "{hide}");
        harness.click(Role::Button, show)?;
        assert_eq!(harness.count(Role::Button, hide), 1, "{show}");
    }
    assert_eq!(harness.app().layout(), photo_ui::Layout::default());
    Ok(())
}

#[test]
fn panels_open_and_close_from_their_headers() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = studio.app_with(files(&library));
    settle(&mut harness)?;
    assert!(harness.app().left_panels().is_open("Folders"));
    assert_eq!(harness.count(Role::TreeItem, "Folder 2024-03-15"), 1);
    harness.click(Role::DisclosureTriangle, "Folders")?;
    assert!(!harness.app().left_panels().is_open("Folders"));
    assert_eq!(harness.count(Role::TreeItem, "Folder 2024-03-15"), 0);

    assert!(harness.app().right_panels().is_open("Histogram"));
    harness.click(Role::DisclosureTriangle, "Histogram")?;
    assert!(!harness.app().right_panels().is_open("Histogram"));
    Ok(())
}
