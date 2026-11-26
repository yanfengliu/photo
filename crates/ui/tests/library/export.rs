//! The Export dialog: two photos are written under the names the dialog
//! showed, with progress and Cancel in the top bar and a way to the folder
//! afterwards; beside the originals nothing can be replaced; the settings
//! are saved as presets.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use egui::{Key, Modifiers, Role};
use photo_engine::{BitDepth, ExistingFile, ExportOutcome, Format, TaskKind};
use photo_testkit::TempLibrary;
use photo_ui::PhotoApp;
use photo_ui::harness::Harness;

use crate::{Studio, TestResult, click_cell, files, settle, shows};

/// What the dialog says about originals.
const PROMISE: &str = "Originals are never written. Every export is a new file, and a name that belongs to a photo in the catalog is refused.";

/// The names of the files in `folder`, sorted.
fn names_in(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// The fixture library in the grid with `dawn.jpg` and `lake.png`
/// selected, and the Export dialog open on them.
fn exporting(
    library: &TempLibrary,
    studio: &Studio,
) -> Result<Harness<PhotoApp>, Box<dyn std::error::Error>> {
    let mut harness = studio.app_with(files(library));
    settle(&mut harness)?;
    click_cell(&mut harness, "dawn.jpg", Modifiers::NONE)?;
    click_cell(&mut harness, "lake.png", Modifiers::COMMAND)?;
    harness.press_key(Key::E, Modifiers::COMMAND | Modifiers::SHIFT)?;
    assert!(harness.app().dialog_open());
    Ok(harness)
}

#[test]
fn two_photos_are_exported_under_the_names_the_dialog_showed() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = exporting(&library, &studio)?;
    assert!(shows(&harness, PROMISE), "the dialog says so");
    // The folder starts as the engine's export folder.
    let folder = harness.control(Role::TextInput, "Export folder")?;
    let default = studio.options.export_dir.display().to_string();
    assert_eq!(folder.text.as_deref(), Some(default.as_str()));

    let out = tempfile::tempdir()?;
    let target = out.path().join("web");
    harness.set_text(
        Role::TextInput,
        "Export folder",
        &target.display().to_string(),
    )?;
    harness.set_text(Role::TextInput, "File name template", "{seq:2}-{name}")?;

    // The photos are numbered in the selection's order; the example is the
    // first one's name, as the engine plans it.
    let engine = harness.app().engine();
    let stems: Vec<String> = engine
        .selection()
        .selected()
        .iter()
        .filter_map(|photo| engine.summary(*photo))
        .map(|summary| {
            let name = Path::new(&summary.file_name);
            name.file_stem().unwrap().to_string_lossy().into_owned()
        })
        .collect();
    assert_eq!(stems.len(), 2);
    let planned: Vec<String> = stems
        .iter()
        .enumerate()
        .map(|(index, stem)| format!("{:02}-{stem}.jpg", index + 1))
        .collect();
    assert!(
        shows(&harness, &planned[0]),
        "the example is {}",
        planned[0]
    );
    assert!(shows(&harness, &format!("in {}", target.display())));

    // With the workers' results held back, the export is seen running.
    harness.app_mut().hold_results(true);
    harness.click(Role::Button, "Export 2 Photos")?;
    assert!(!harness.app().dialog_open());
    assert!(shows(&harness, "Exporting 2 photos"));
    assert_eq!(harness.count(Role::Button, "Cancel Exporting 2 photos"), 1);
    harness.app_mut().hold_results(false);
    settle(&mut harness)?;

    let mut expected = planned.clone();
    expected.sort();
    assert_eq!(
        names_in(&target),
        expected,
        "the planned names, and no others"
    );
    assert!(
        harness
            .app()
            .toasts()
            .any(|toast| toast == "Exported 2 photos"),
        "{:?}",
        harness.app().toasts().collect::<Vec<_>>()
    );
    let engine = harness.app().engine();
    let task = engine
        .tasks()
        .iter()
        .find(|task| task.kind == TaskKind::Export)
        .ok_or("the export is listed")?
        .id;
    let results = engine.export_results(task).ok_or("its results are kept")?;
    for (exported, name) in results.iter().zip(&planned) {
        match &exported.result {
            Ok(ExportOutcome::Written { path, .. }) => assert_eq!(path, &target.join(name)),
            other => panic!("{name} was not written: {other:?}"),
        }
    }
    // The originals are where they were, untouched.
    for entry in library.entries() {
        assert!(entry.path.is_file(), "{}", entry.path.display());
    }

    // The top bar keeps a way to the folder until it is dismissed.
    let shown: Rc<RefCell<Vec<PathBuf>>> = Rc::default();
    let record = Rc::clone(&shown);
    harness
        .app_mut()
        .set_reveal(move |folder| record.borrow_mut().push(folder.to_path_buf()));
    assert!(shows(&harness, "Exported 2 of 2"));
    harness.click(Role::Button, "Show Exported Files")?;
    assert_eq!(*shown.borrow(), std::slice::from_ref(&target));
    harness.click(Role::Button, "Dismiss the export notice")?;
    assert_eq!(harness.count(Role::Button, "Show Exported Files"), 0);
    Ok(())
}

#[test]
fn a_folder_with_a_long_path_does_not_widen_the_dialog() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = exporting(&library, &studio)?;
    let right = harness
        .control(Role::Button, "Export 2 Photos")?
        .rect
        .right();
    let mut deep = std::env::temp_dir();
    for level in 0..8 {
        deep.push(format!("a folder with a long name {level}"));
    }
    harness.set_text(
        Role::TextInput,
        "Export folder",
        &deep.display().to_string(),
    )?;
    // The whole path is still what the example says, cut short on screen.
    let place = harness
        .control(Role::Label, &format!("in {}", deep.display()))?
        .rect;
    assert!(place.right() <= right, "{place:?} ends after {right}");
    assert_eq!(
        harness
            .control(Role::Button, "Export 2 Photos")?
            .rect
            .right(),
        right,
        "the dialog is as wide as it was"
    );
    Ok(())
}

#[test]
fn beside_the_originals_nothing_can_be_replaced() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = exporting(&library, &studio)?;

    // Into a folder, an earlier export may be replaced.
    harness.click(Role::ComboBox, "Existing files")?;
    harness.click(Role::Button, "Replace the earlier export")?;

    // Beside the originals the choice is gone, and the one made is undone:
    // the JPEG that would land on its own original gets another name.
    harness.click(Role::RadioButton, "Beside each original")?;
    assert!(shows(&harness, "Nothing beside an original is replaced."));
    assert!(harness.control(Role::TextInput, "Export folder")?.disabled);
    harness.click(Role::ComboBox, "Existing files")?;
    assert_eq!(harness.count(Role::Button, "Replace the earlier export"), 0);
    assert_eq!(harness.count(Role::Button, "Skip the photo"), 1);
    harness.click(Role::Button, "Give the new file another name")?;
    let first = harness
        .app()
        .engine()
        .selection()
        .selected()
        .first()
        .and_then(|photo| harness.app().engine().summary(*photo))
        .map(|summary| summary.path.clone())
        .ok_or("a photo is selected")?;
    let folder = first.parent().ok_or("it is in a folder")?;
    assert!(shows(&harness, &format!("in {}", folder.display())));
    let stem = first.file_stem().unwrap().to_string_lossy();
    let own_name = format!("{stem}.jpg");
    let example = if first.extension().is_some_and(|ext| ext == "jpg") {
        format!("{stem}-2.jpg")
    } else {
        own_name
    };
    assert!(shows(&harness, &example), "the example is {example}");

    harness.click(Role::Button, "Export 2 Photos")?;
    settle(&mut harness)?;
    let engine = harness.app().engine();
    let task = engine
        .tasks()
        .iter()
        .find(|task| task.kind == TaskKind::Export)
        .ok_or("the export is listed")?
        .id;
    let originals: Vec<&PathBuf> = library.entries().iter().map(|entry| &entry.path).collect();
    for exported in engine.export_results(task).ok_or("its results are kept")? {
        match &exported.result {
            Ok(ExportOutcome::Written { path, .. }) => {
                assert!(
                    !originals.contains(&path),
                    "{} is an original",
                    path.display()
                );
                assert!(path.is_file());
            }
            other => panic!("not written: {other:?}"),
        }
    }
    Ok(())
}

#[test]
fn the_settings_are_saved_as_a_preset_and_come_back_from_it() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = exporting(&library, &studio)?;

    harness.click(Role::ComboBox, "Export format")?;
    harness.click(Role::Button, "TIFF")?;
    harness.click(Role::RadioButton, "16 bits")?;
    harness.click(Role::ComboBox, "Existing files")?;
    harness.click(Role::Button, "Skip the photo")?;
    harness.click(Role::Button, "Save Preset…")?;
    harness.set_text(Role::TextInput, "Export preset name", "Archive")?;
    harness.click(Role::Button, "Save")?;
    settle(&mut harness)?;
    assert!(
        harness.app().dialog_open(),
        "saving a preset does not export"
    );
    let presets = harness.app().engine().export_presets()?;
    assert_eq!(presets.len(), 1);
    let (name, saved) = &presets[0];
    assert_eq!(name, "Archive");
    assert_eq!(
        saved.format,
        Format::Tiff {
            depth: BitDepth::Sixteen
        }
    );
    assert_eq!(saved.location.existing, ExistingFile::Skip);

    // Change the format, then take the preset back from the menu. The menu
    // says when the settings are no longer the preset's.
    let preset = |harness: &Harness<PhotoApp>| {
        harness
            .control(Role::ComboBox, "Export preset")
            .map(|menu| menu.text.unwrap_or_default())
    };
    assert_eq!(preset(&harness)?, "Archive");
    harness.click(Role::ComboBox, "Export format")?;
    harness.click(Role::Button, "JPEG")?;
    assert!(shows(&harness, "Quality"));
    assert_eq!(preset(&harness)?, "Archive (changed)");
    harness.click(Role::ComboBox, "Export preset")?;
    harness.click(Role::Button, "Archive")?;
    assert_eq!(
        harness.control(Role::RadioButton, "16 bits")?.toggled,
        Some(true)
    );
    assert_eq!(preset(&harness)?, "Archive");

    harness.click(Role::Button, "Delete Preset")?;
    settle(&mut harness)?;
    assert!(harness.app().engine().export_presets()?.is_empty());

    // The dialog is cancelled, and its choices are there the next time.
    harness.click(Role::Button, "Cancel")?;
    assert!(!harness.app().dialog_open());
    assert_eq!(names_in(&studio.options.export_dir), Vec::<String>::new());
    harness.click(Role::Button, "Export…")?;
    assert!(harness.app().dialog_open());
    assert_eq!(
        harness.control(Role::RadioButton, "16 bits")?.toggled,
        Some(true)
    );
    Ok(())
}
