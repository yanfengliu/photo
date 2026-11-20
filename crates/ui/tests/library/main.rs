//! The Library, met the way a user meets it: the real app on a temporary
//! catalog, driven through the headless harness by key presses, clicks and
//! drags on controls found by role and accessible name, and checked against
//! what the engine then holds.

mod attributes;
mod collections;
mod import;
mod keywords;
mod narrow;
mod performance;
mod selection;
mod shell;
mod snapshots;

use std::path::PathBuf;
use std::time::Duration;

use egui::Role;
use photo_core::meta::PhotoId;
use photo_engine::{Engine, ImportMode, ImportRequest};
use photo_testkit::TempLibrary;
use photo_ui::harness::{Harness, HarnessBuilder, HarnessError};
use photo_ui::{AppOptions, PhotoApp};
use tempfile::TempDir;

/// The result type of a test: any harness, render or engine error fails it.
pub type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Long enough for anything a test asks of the engine.
pub const PATIENCE: Duration = Duration::from_secs(60);

/// The window size every test runs at.
pub const WINDOW: [f32; 2] = [1600.0, 1000.0];

/// A temporary folder holding a catalog, its previews and the export
/// folder, all removed when the value is dropped.
pub struct Studio {
    _dir: TempDir,
    pub options: AppOptions,
}

impl Default for Studio {
    fn default() -> Self {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let options = AppOptions {
            catalog_path: dir.path().join("Test.photocat"),
            previews_dir: dir.path().join("Test.previews"),
            export_dir: dir.path().join("exports"),
        };
        Self { _dir: dir, options }
    }
}

impl Studio {
    pub fn new() -> Self {
        Self::default()
    }

    /// The app on this studio's catalog, as it is now.
    pub fn app(&self) -> Harness<PhotoApp> {
        self.app_at(1.0)
    }

    /// The app at `pixels_per_point`.
    pub fn app_at(&self, pixels_per_point: f32) -> Harness<PhotoApp> {
        let engine = self.options.open_engine().expect("the catalog opens");
        self.harness(engine, pixels_per_point)
    }

    /// The app after `files` were imported in place, as the Previous Import
    /// with its first photo active.
    pub fn app_with(&self, files: Vec<PathBuf>) -> Harness<PhotoApp> {
        self.app_with_at(files, 1.0)
    }

    pub fn app_with_at(&self, files: Vec<PathBuf>, pixels_per_point: f32) -> Harness<PhotoApp> {
        let mut engine = self.options.open_engine().expect("the catalog opens");
        import(&mut engine, files);
        self.harness(engine, pixels_per_point)
    }

    /// The app after `files` were imported, in a window of `size` points.
    pub fn app_with_size(&self, files: Vec<PathBuf>, size: [f32; 2]) -> Harness<PhotoApp> {
        let mut engine = self.options.open_engine().expect("the catalog opens");
        import(&mut engine, files);
        let options = self.options.clone();
        HarnessBuilder::new()
            .size(size)
            .strict_glyphs(true)
            .max_steps(240)
            .build(move |cc| PhotoApp::new(cc, engine, options))
    }

    /// The app in a harness that does not wait for the UI to come to rest:
    /// each action runs at most two frames after its input. For watching
    /// background work, during which the UI keeps changing; actions then
    /// report [`HarnessError::NotSettled`], which [`restless`] accepts.
    pub fn app_restless(&self) -> Harness<PhotoApp> {
        let engine = self.options.open_engine().expect("the catalog opens");
        let options = self.options.clone();
        HarnessBuilder::new()
            .size(WINDOW)
            .strict_glyphs(true)
            .max_steps(2)
            .build(move |cc| PhotoApp::new(cc, engine, options))
    }

    fn harness(&self, engine: Engine, pixels_per_point: f32) -> Harness<PhotoApp> {
        let options = self.options.clone();
        HarnessBuilder::new()
            .size(WINDOW)
            .pixels_per_point(pixels_per_point)
            .strict_glyphs(true)
            .max_steps(240)
            .build(move |cc| PhotoApp::new(cc, engine, options))
    }
}

/// An action's result in a harness from [`Studio::app_restless`]: done,
/// whether or not the UI came to rest after it.
pub fn restless(result: Result<(), HarnessError>) -> Result<(), HarnessError> {
    match result {
        Err(HarnessError::NotSettled(_)) => Ok(()),
        other => other,
    }
}

/// Import `files` in place and wait for it.
pub fn import(engine: &mut Engine, files: Vec<PathBuf>) {
    let request = ImportRequest {
        files,
        mode: ImportMode::Add,
        skip_suspected_duplicates: false,
        keywords: Vec::new(),
        read_sidecars: true,
    };
    engine.start_import(request).expect("the import starts");
    assert!(engine.wait_idle(PATIENCE), "the import finished");
}

/// The fixture library's files.
pub fn files(library: &TempLibrary) -> Vec<PathBuf> {
    library
        .entries()
        .iter()
        .map(|entry| entry.path.clone())
        .collect()
}

/// Let the engine finish what it was asked and run frames until what came
/// in is on screen (every thumbnail in view uploaded). A frame can ask for
/// more previews, so this goes round until the engine stays idle.
pub fn settle(harness: &mut Harness<PhotoApp>) -> Result<(), HarnessError> {
    for _ in 0..40 {
        if !harness.app_mut().wait_idle(PATIENCE) {
            return Err(HarnessError::NotSettled(
                "the engine never became idle".into(),
            ));
        }
        harness.run()?;
        if harness.app().engine().is_idle() {
            return Ok(());
        }
    }
    Err(HarnessError::NotSettled(
        "previews kept being asked for".into(),
    ))
}

/// The photo in the list whose file is called `name`.
pub fn id(harness: &Harness<PhotoApp>, name: &str) -> PhotoId {
    harness
        .app()
        .engine()
        .photos()
        .iter()
        .find(|photo| photo.file_name == name && photo.master_id.is_none())
        .unwrap_or_else(|| panic!("{name} is not in the list"))
        .id
}

/// The file names of the photo list, in order.
pub fn names(harness: &Harness<PhotoApp>) -> Vec<String> {
    harness
        .app()
        .engine()
        .photos()
        .iter()
        .map(|photo| photo.file_name.clone())
        .collect()
}

/// The file names of the selected photos, in list order.
pub fn selected(harness: &Harness<PhotoApp>) -> Vec<String> {
    let engine = harness.app().engine();
    engine
        .selection()
        .selected()
        .iter()
        .filter_map(|photo| engine.summary(*photo))
        .map(|summary| summary.file_name.clone())
        .collect()
}

/// The file name of the active photo.
pub fn active(harness: &Harness<PhotoApp>) -> Option<String> {
    let engine = harness.app().engine();
    let active = engine.active_photo()?;
    engine
        .summary(active)
        .map(|summary| summary.file_name.clone())
}

/// What the grid cell of the photo called `name` says about it.
pub fn cell(harness: &Harness<PhotoApp>, name: &str) -> Result<String, HarnessError> {
    Ok(harness
        .control(Role::ListBoxOption, name)?
        .text
        .unwrap_or_default())
}

/// Click the centre of the cell called `name` with `modifiers` held.
pub fn click_cell(
    harness: &mut Harness<PhotoApp>,
    name: &str,
    modifiers: egui::Modifiers,
) -> Result<(), HarnessError> {
    let center = harness.control(Role::ListBoxOption, name)?.rect.center();
    harness.click_at_with(center, egui::PointerButton::Primary, modifiers)
}

/// Whether a label (a toast, a message) saying exactly `text` is shown.
pub fn shows(harness: &Harness<PhotoApp>, text: &str) -> bool {
    harness.count(Role::Label, text) > 0
}
