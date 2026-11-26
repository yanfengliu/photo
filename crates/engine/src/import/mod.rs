//! Importing files into the catalog.
//!
//! Workers read each file's metadata and sidecar (and in copy mode copy the
//! file into place); the engine's thread decides what to skip and inserts
//! the photos in batches, in the order the files were listed, all as one
//! import run. Cancelling stops the queued work; files already copied are
//! still recorded, so every copy has its catalog record.

mod files;
mod record;
mod run;
mod scan;
mod walk;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use photo_core::meta::{ImportId, PhotoId};

pub use files::DatePattern;
pub(crate) use files::sidecar_keywords;
pub use scan::{ImportCandidate, ImportScan};

use crate::engine::Engine;
use crate::error::EngineError;
use crate::jobs::Lane;
use crate::tasks::{TaskId, TaskKind};
use run::{Run, Slot};

/// Photos inserted per catalog write while importing.
const BATCH: usize = 100;
/// How many finished imports keep their summary.
const SUMMARIES_KEPT: usize = 8;

/// Where imported files end up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportMode {
    /// Catalogue the files where they are.
    Add,
    /// Copy the files under `destination` first and catalogue the copies.
    Copy {
        /// The folder the copies go under.
        destination: PathBuf,
        /// How the copies are arranged there.
        layout: CopyLayout,
    },
}

/// How copied files are arranged under the destination.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CopyLayout {
    /// All in the destination folder itself.
    OneFolder,
    /// In folders named after the capture date (the file's modification
    /// date when it records none).
    ByDate(DatePattern),
}

/// What to import and how.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportRequest {
    /// The files, in the order their photos are added. A file listed twice
    /// is imported once.
    pub files: Vec<PathBuf>,
    /// In place, or copied.
    pub mode: ImportMode,
    /// Leave out files the catalog probably holds already: same file name,
    /// size and capture time as a catalogued photo or as a file earlier in
    /// this import.
    pub skip_suspected_duplicates: bool,
    /// Keywords to give every imported photo.
    pub keywords: Vec<String>,
    /// Take ratings, labels, keywords, text and develop settings from XMP
    /// sidecars beside the files.
    pub read_sidecars: bool,
}

/// One file that could not be imported, or imported with a problem.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileIssue {
    /// The file.
    pub path: PathBuf,
    /// What went wrong.
    pub message: String,
}

/// What an import has done so far; complete once `finished`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportSummary {
    /// The import run the photos belong to.
    pub import: ImportId,
    /// The photos added, in the order their files were listed.
    pub imported: Vec<PhotoId>,
    /// Files left out because their path is already in the catalog.
    pub already_in_catalog: Vec<PathBuf>,
    /// Files left out as suspected duplicates.
    pub suspected_duplicates: Vec<PathBuf>,
    /// Files that failed (not imported), whose sidecar could not be read
    /// (imported without it), or whose sidecar holds develop settings this
    /// version cannot read (imported with the starting settings; the
    /// sidecar keeps its own).
    pub errors: Vec<FileIssue>,
    /// True once every file has been dealt with.
    pub finished: bool,
    /// True when the import was cancelled.
    pub cancelled: bool,
}

/// Every import and the latest scan.
#[derive(Default)]
pub(crate) struct Imports {
    runs: Vec<Run>,
    summaries: HashMap<TaskId, ImportSummary>,
    finished: Vec<TaskId>,
    pub(crate) scan: Option<ImportScan>,
    /// Set while the engine closes: when it stops waiting for a catalog
    /// that another connection has locked. While closing, a photo that
    /// cannot be recorded is named in the log.
    pub(crate) closing: Option<Instant>,
}

impl Engine {
    /// Start importing. Returns the task to follow with
    /// [`Engine::import_summary`] and [`Engine::tasks`].
    ///
    /// When the import ends, if it added any photo, the source becomes
    /// Previous Import with no filter and the first photo becomes active.
    pub fn start_import(&mut self, request: ImportRequest) -> Result<TaskId, EngineError> {
        let mut seen = HashSet::new();
        let paths: Vec<PathBuf> = request
            .files
            .into_iter()
            .filter(|path| seen.insert(path.clone()))
            .collect();
        let import = self.catalog.begin_import()?;
        let label = format!("Importing {} photos", paths.len());
        let (task, token) = self
            .tasks
            .start(TaskKind::Import, label, paths.len() as u64);
        self.imports.summaries.insert(
            task,
            ImportSummary {
                import,
                imported: Vec::new(),
                already_in_catalog: Vec::new(),
                suspected_duplicates: Vec::new(),
                errors: Vec::new(),
                finished: false,
                cancelled: false,
            },
        );
        self.imports.runs.push(Run {
            task,
            mode: request.mode,
            skip_duplicates: request.skip_suspected_duplicates,
            keywords: request.keywords,
            slots: paths.iter().cloned().map(Slot::Working).collect(),
            next_insert: 0,
            working: paths.len(),
            accepted: HashSet::new(),
            unreadable_settings: 0,
        });
        let read_sidecars = request.read_sidecars;
        for (index, path) in paths.into_iter().enumerate() {
            self.submit(
                Lane::Background,
                None,
                Some(token.clone()),
                move |_| {
                    let read = files::read(&path, read_sidecars);
                    Box::new(move |engine: &mut Engine| engine.import_read(task, index, read))
                },
                move |engine, failure| engine.import_slot_failed(task, index, failure),
            );
        }
        self.advance_import(task);
        Ok(task)
    }

    /// What an import has done so far, while it runs and for a while after.
    pub fn import_summary(&self, task: TaskId) -> Option<&ImportSummary> {
        self.imports.summaries.get(&task)
    }
}
