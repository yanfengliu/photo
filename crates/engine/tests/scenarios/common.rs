//! Helpers shared by the scenarios: an engine on a temporary catalog, and
//! importing fixtures into it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use photo_core::image::Rgba8Image;
use photo_core::meta::PhotoId;
use photo_engine::{
    CanvasView, DevelopLoad, Engine, EngineConfig, ImportMode, ImportRequest, ImportSummary, Sort,
    SortDirection, SortKey,
};
use photo_testkit::TempLibrary;
use tempfile::TempDir;

/// Long enough for anything a test asks of the engine; a test that needs
/// it is broken.
pub const PATIENCE: Duration = Duration::from_secs(60);

/// The engine's files in a temporary directory, laid out as the app lays
/// them out: the catalog and its previews in a folder of their own, the
/// exports beside it. Creates the catalog's folder.
pub fn config(dir: &Path, worker_threads: usize) -> EngineConfig {
    let catalog = dir.join("catalog");
    std::fs::create_dir_all(&catalog).unwrap();
    EngineConfig {
        catalog_path: catalog.join("library.photocat"),
        previews_dir: catalog.join("library.previews"),
        worker_threads,
        export_dir: dir.join("exports"),
    }
}

/// A temporary directory with an engine on a new catalog in it.
pub struct Studio {
    pub dir: TempDir,
    pub engine: Engine,
}

impl Studio {
    pub fn new() -> Self {
        Self::with_workers(2)
    }

    pub fn with_workers(worker_threads: usize) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::open(config(dir.path(), worker_threads)).unwrap();
        Self { dir, engine }
    }

    /// Open a second engine on the same catalog and previews, after the
    /// first has been dropped.
    pub fn reopen(self) -> Self {
        let Self { dir, engine } = self;
        drop(engine);
        let engine = Engine::open(config(dir.path(), 2)).unwrap();
        Self { dir, engine }
    }

    pub fn previews_dir(&self) -> PathBuf {
        config(self.dir.path(), 2).previews_dir
    }

    pub fn settle(&mut self) {
        settle(&mut self.engine);
    }

    /// Import `files` in place, reading sidecars, and wait for it.
    pub fn import(&mut self, files: Vec<PathBuf>) -> ImportSummary {
        let task = self.engine.start_import(add(files)).unwrap();
        self.settle();
        self.engine.import_summary(task).unwrap().clone()
    }

    /// Import the whole fixture library in place.
    pub fn import_library(&mut self, library: &TempLibrary) -> ImportSummary {
        let files = library
            .entries()
            .iter()
            .map(|entry| entry.path.clone())
            .collect();
        let summary = self.import(files);
        assert_eq!(summary.imported.len(), 6, "{summary:?}");
        summary
    }

    /// The list's photos in order.
    pub fn ids(&self) -> Vec<PhotoId> {
        self.engine.photos().iter().map(|photo| photo.id).collect()
    }

    /// The list's file names in order.
    pub fn names(&self) -> Vec<String> {
        self.engine
            .photos()
            .iter()
            .map(|photo| photo.file_name.clone())
            .collect()
    }

    /// The photo in the list whose file is called `name` (the master, when a
    /// virtual copy shares the name).
    pub fn id(&self, name: &str) -> PhotoId {
        self.engine
            .photos()
            .iter()
            .find(|photo| photo.file_name == name && photo.master_id.is_none())
            .unwrap_or_else(|| panic!("{name} is not in the list"))
            .id
    }

    /// The messages of the toasts since the last call.
    pub fn toasts(&mut self) -> Vec<String> {
        self.engine
            .take_toasts()
            .into_iter()
            .map(|toast| toast.message)
            .collect()
    }
}

/// Wait for the engine, failing the test if it never settles.
pub fn settle(engine: &mut Engine) {
    assert!(engine.wait_idle(PATIENCE), "the engine did not become idle");
}

/// A request to import `files` in place, reading sidecars, skipping
/// nothing.
pub fn add(files: Vec<PathBuf>) -> ImportRequest {
    ImportRequest {
        files,
        mode: ImportMode::Add,
        skip_suspected_duplicates: false,
        keywords: Vec::new(),
        read_sidecars: true,
    }
}

/// A collection's own order.
pub fn own_order() -> Sort {
    Sort {
        key: SortKey::Custom,
        direction: SortDirection::Ascending,
    }
}

/// Open `photo` in Develop on `canvas` and wait for its first frame.
pub fn open_rendered(studio: &mut Studio, photo: PhotoId, canvas: CanvasView) {
    studio.engine.open_develop(photo).unwrap();
    studio.engine.set_canvas_view(canvas);
    studio.settle();
    let session = studio.engine.develop().unwrap();
    assert_eq!(session.load_state(), &DevelopLoad::Rendered);
}

/// The mean of the red, green and blue code values of an image.
pub fn mean_luma(image: &Rgba8Image) -> f64 {
    mean_rgb(image, 0, 0, image.width, image.height)
        .iter()
        .sum::<f64>()
        / 3.0
}

/// The mean red, green and blue code values of a rectangle of an image.
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

/// `count` small JPEG files in a new temporary directory.
pub fn many_jpegs(count: usize, width: u32, height: u32) -> (TempDir, Vec<PathBuf>) {
    let dir = tempfile::tempdir().unwrap();
    let files = (0..count)
        .map(|index| {
            photo_testkit::write_jpeg(dir.path(), &format!("IMG_{index:04}.jpg"), width, height)
        })
        .collect();
    (dir, files)
}
