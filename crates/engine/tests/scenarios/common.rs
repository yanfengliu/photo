//! Helpers shared by the scenarios: an engine on a temporary catalog, and
//! importing fixtures into it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use photo_core::meta::PhotoId;
use photo_engine::{
    Engine, EngineConfig, ImportMode, ImportRequest, ImportSummary, Sort, SortDirection, SortKey,
};
use photo_testkit::TempLibrary;
use tempfile::TempDir;

/// Long enough for anything a test asks of the engine; a test that needs
/// it is broken.
pub const PATIENCE: Duration = Duration::from_secs(60);

/// The engine's files in a temporary directory.
pub fn config(dir: &Path, worker_threads: usize) -> EngineConfig {
    EngineConfig {
        catalog_path: dir.join("library.photocat"),
        previews_dir: dir.join("library.previews"),
        worker_threads,
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
