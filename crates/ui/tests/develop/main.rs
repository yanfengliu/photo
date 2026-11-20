//! The Develop module, met the way a photographer meets it: the real app on
//! a temporary catalog, driven through the headless harness by keys,
//! clicks and drags on controls found by role and accessible name, and
//! checked against what the engine then holds and what the canvas shows.

mod looks;
mod module;
mod performance;
mod protected;
mod quick;
mod settings;
mod sliders;
mod snapshots;
mod tools;

use std::path::{Path, PathBuf};
use std::time::Duration;

use egui::{Key, Modifiers, Role, Vec2, pos2};
use photo_core::develop::DevelopSettings;
use photo_core::image::Rgba8Image;
use photo_core::meta::PhotoId;
use photo_engine::{DevelopSession, Engine, ImportMode, ImportRequest};
use photo_ui::harness::{Control, Harness, HarnessBuilder, HarnessError};
use photo_ui::{AppOptions, Module, PhotoApp};
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
    pub dir: TempDir,
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
        Self { dir, options }
    }
}

impl Studio {
    pub fn new() -> Self {
        Self::default()
    }

    /// The app on this studio's catalog as it is now.
    pub fn app(&self) -> Harness<PhotoApp> {
        self.app_at(1.0)
    }

    /// The app at `pixels_per_point`.
    pub fn app_at(&self, pixels_per_point: f32) -> Harness<PhotoApp> {
        let engine = self.options.open_engine().expect("the catalog opens");
        self.harness(engine, pixels_per_point)
    }

    /// The app after `files` were imported in place.
    pub fn app_with(&self, files: Vec<PathBuf>) -> Harness<PhotoApp> {
        self.app_with_at(files, 1.0)
    }

    pub fn app_with_at(&self, files: Vec<PathBuf>, pixels_per_point: f32) -> Harness<PhotoApp> {
        let mut engine = self.options.open_engine().expect("the catalog opens");
        import(&mut engine, files);
        self.harness(engine, pixels_per_point)
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

/// Two photos of the test scene, 1200 x 800: a camera raw file (with its
/// embedded preview) and a PNG, in a temporary folder.
pub fn scene_files() -> (TempDir, Vec<PathBuf>) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let files = vec![
        photo_testkit::write_dng(dir.path(), "scene.dng", 1200, 800),
        photo_testkit::write_png8(dir.path(), "scene.png", 1200, 800),
    ];
    (dir, files)
}

/// Let the engine finish what it was asked and run frames until what came
/// in is on screen. A frame can ask for more, so this goes round until the
/// engine stays idle.
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
    Err(HarnessError::NotSettled("work kept being asked for".into()))
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

/// Whether a label saying exactly `text` is shown.
pub fn shows(harness: &Harness<PhotoApp>, text: &str) -> bool {
    harness.count(Role::Label, text) > 0
}

/// Click the grid cell of the photo called `name`, press D, and wait for
/// its first frame.
pub fn develop(harness: &mut Harness<PhotoApp>, name: &str) -> TestResult {
    settle(harness)?;
    let cell = harness.control(Role::ListBoxOption, name)?.rect.center();
    harness.click_at(cell)?;
    harness.press_key(Key::D, Modifiers::NONE)?;
    settle(harness)?;
    assert_eq!(harness.app().module(), Module::Develop);
    Ok(())
}

/// The open develop session.
pub fn session(harness: &Harness<PhotoApp>) -> &DevelopSession {
    harness
        .app()
        .engine()
        .develop()
        .expect("a photo is open in Develop")
}

/// The open photo's settings as shown.
pub fn settings(harness: &Harness<PhotoApp>) -> DevelopSettings {
    session(harness).settings().clone()
}

/// The labels of the open photo's history steps, oldest first.
pub fn history(harness: &Harness<PhotoApp>) -> Vec<String> {
    let engine = harness.app().engine();
    engine
        .history(session(harness).photo())
        .expect("the history reads")
        .into_iter()
        .map(|step| step.label)
        .collect()
}

/// The slider called `name`.
pub fn slider(harness: &Harness<PhotoApp>, name: &str) -> Result<Control, HarnessError> {
    harness.control(Role::Slider, name)
}

/// Where along a slider's track a linear slider shows `value`.
pub fn track_x(control: &Control, value: f32) -> f32 {
    let (min, max) = (control_min(control), control_max(control));
    let half_handle = 3.5;
    let (left, right) = (
        control.rect.left() + half_handle,
        control.rect.right() - half_handle,
    );
    left + (value - min) / (max - min) * (right - left)
}

fn control_min(control: &Control) -> f32 {
    control.min.unwrap_or(-100.0) as f32
}

fn control_max(control: &Control) -> f32 {
    control.max.unwrap_or(100.0) as f32
}

/// Drag the slider `name` from its handle to where it shows `value`, in
/// steps, and let go.
pub fn drag_slider(harness: &mut Harness<PhotoApp>, name: &str, value: f32) -> TestResult {
    let control = slider(harness, name)?;
    let now = control.value.unwrap_or(0.0) as f32;
    let y = control.rect.center().y;
    let from = pos2(track_x(&control, now), y);
    let to = pos2(track_x(&control, value), y);
    harness.drag(from, to)?;
    Ok(())
}

/// Open the right column's panel `title` alone (closing the others), so
/// its controls are on screen.
pub fn open_panel(harness: &mut Harness<PhotoApp>, title: &str) -> TestResult {
    let open: Vec<String> = harness
        .app()
        .right_panels()
        .open_panels()
        .map(str::to_owned)
        .collect();
    for other in open
        .iter()
        .filter(|other| *other != title && *other != "Histogram")
    {
        harness.click(Role::DisclosureTriangle, other)?;
    }
    if !harness.app().right_panels().is_open(title) {
        harness.click(Role::DisclosureTriangle, title)?;
    }
    Ok(())
}

/// The mean of the red, green and blue code values of an image.
pub fn mean_luma(image: &Rgba8Image) -> f64 {
    let sum: u64 = image
        .pixels
        .chunks_exact(4)
        .map(|pixel| u64::from(pixel[0]) + u64::from(pixel[1]) + u64::from(pixel[2]))
        .sum();
    sum as f64 / (3.0 * f64::from(image.width * image.height))
}

/// The mean red, green and blue of a rectangle of an image.
pub fn mean_rgb(image: &Rgba8Image, x: u32, y: u32, width: u32, height: u32) -> [f64; 3] {
    let mut sum = [0.0; 3];
    for row in y..y + height {
        for column in x..x + width {
            let pixel = image.pixel(column, row);
            for (total, value) in sum.iter_mut().zip(pixel) {
                *total += f64::from(value);
            }
        }
    }
    let count = f64::from(width * height);
    sum.map(|total| total / count)
}

/// The mean luma of the open photo's newest frame.
pub fn frame_luma(harness: &Harness<PhotoApp>) -> f64 {
    let frame = session(harness).frame().expect("a frame");
    mean_luma(frame.image())
}

/// Write-protect `path`; false when this user can write it anyway.
pub fn protect(path: &Path) -> bool {
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(path, permissions).unwrap();
    std::fs::OpenOptions::new().write(true).open(path).is_err()
}

/// The size of the window in points.
pub fn window() -> Vec2 {
    Vec2::from(WINDOW)
}
