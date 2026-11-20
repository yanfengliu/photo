//! Importing files into the catalog.
//!
//! Workers read each file's metadata and sidecar (and in copy mode copy the
//! file into place); the engine's thread decides what to skip and inserts
//! the photos in batches, in the order the files were listed, all as one
//! import run. Cancelling stops the queued work; files already copied are
//! still recorded, so every copy has its catalog record.

mod files;
mod scan;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use photo_catalog::{CatalogError, NewPhoto, Source};
use photo_core::meta::{ImportId, PhotoId};
use photo_core::time::DateTime;

pub use files::DatePattern;
pub(crate) use files::sidecar_keywords;
pub use scan::{ImportCandidate, ImportScan};

use crate::browse::{Change, Refresh};
use crate::engine::{Engine, JobFailure};
use crate::error::EngineError;
use crate::jobs::{CancelToken, Lane};
use crate::tasks::{TaskId, TaskKind, TaskState};
use files::{CopyError, Copied, ReadFile};

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
    /// Files that failed (not imported) or whose sidecar could not be read
    /// (imported without it).
    pub errors: Vec<FileIssue>,
    /// True once every file has been dealt with.
    pub finished: bool,
    /// True when the import was cancelled.
    pub cancelled: bool,
}

/// Where one listed file is.
enum Slot {
    /// Being read, or copied.
    Working(PathBuf),
    /// Read (and copied) and waiting for its turn to be inserted.
    Ready(Box<NewPhoto>),
    /// Inserted, skipped or failed.
    Done,
}

/// An import in progress.
struct Run {
    task: TaskId,
    mode: ImportMode,
    skip_duplicates: bool,
    keywords: Vec<String>,
    slots: Vec<Slot>,
    /// Slots before this one have been inserted or dropped.
    next_insert: usize,
    working: usize,
    /// File name, size and capture time of each file accepted so far.
    accepted: HashSet<(String, u64, Option<DateTime>)>,
}

/// Every import and the latest scan.
#[derive(Default)]
pub(crate) struct Imports {
    runs: Vec<Run>,
    summaries: HashMap<TaskId, ImportSummary>,
    finished: Vec<TaskId>,
    pub(crate) scan: Option<ImportScan>,
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

    fn run_mut(&mut self, task: TaskId) -> Option<&mut Run> {
        self.imports.runs.iter_mut().find(|run| run.task == task)
    }

    fn summary_mut(&mut self, task: TaskId) -> Option<&mut ImportSummary> {
        self.imports.summaries.get_mut(&task)
    }

    fn import_read(&mut self, task: TaskId, index: usize, read: Result<ReadFile, String>) -> bool {
        let cancelled = self.tasks.is_cancelled(task);
        let Some(path) = self.working_path(task, index) else {
            return false;
        };
        let outcome = match read {
            Err(message) if !cancelled => Err(FileIssue { path, message }),
            Err(_) => Ok(None),
            Ok(_) if cancelled => Ok(None),
            Ok(read) => self.accept(task, &path, read),
        };
        match outcome {
            Ok(Some(photo)) => self.dispatch(task, index, photo),
            Ok(None) => self.settle_slot(task, index, None),
            Err(issue) => {
                if let Some(summary) = self.summary_mut(task) {
                    summary.errors.push(issue);
                }
                self.settle_slot(task, index, None);
            }
        }
        self.advance_import(task);
        true
    }

    fn working_path(&self, task: TaskId, index: usize) -> Option<PathBuf> {
        let run = self.imports.runs.iter().find(|run| run.task == task)?;
        match run.slots.get(index)? {
            Slot::Working(path) => Some(path.clone()),
            Slot::Ready(_) | Slot::Done => None,
        }
    }

    /// Decide whether a file that was read goes in. `Ok(None)` when it is
    /// skipped; the reason is recorded in the summary.
    fn accept(
        &mut self,
        task: TaskId,
        path: &std::path::Path,
        read: ReadFile,
    ) -> Result<Option<NewPhoto>, FileIssue> {
        let mut photo = read.photo;
        let issue = |message: String| FileIssue {
            path: path.to_path_buf(),
            message,
        };
        let catalog_error = |error: CatalogError| issue(error.to_string());
        let Some(run) = self.imports.runs.iter().find(|run| run.task == task) else {
            return Ok(None);
        };
        let in_place = run.mode == ImportMode::Add;
        let skip_duplicates = run.skip_duplicates;
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let key = (name, photo.file_size, photo.metadata.capture_time);
        if in_place && self.catalog.find_by_path(path).map_err(catalog_error)?.is_some() {
            if let Some(summary) = self.summary_mut(task) {
                summary.already_in_catalog.push(path.to_path_buf());
            }
            return Ok(None);
        }
        let earlier = run.accepted.contains(&key);
        let duplicate = skip_duplicates
            && (earlier
                || !self
                    .catalog
                    .find_suspected_duplicates(&key.0, key.1, key.2)
                    .map_err(catalog_error)?
                    .is_empty());
        if duplicate {
            if let Some(summary) = self.summary_mut(task) {
                summary.suspected_duplicates.push(path.to_path_buf());
            }
            return Ok(None);
        }
        if let Some(problem) = read.sidecar_problem
            && let Some(summary) = self.summary_mut(task)
        {
            summary
                .errors
                .push(issue(format!("the sidecar was not read: {problem}")));
        }
        let Some(run) = self.run_mut(task) else {
            return Ok(None);
        };
        run.accepted.insert(key);
        for keyword in &run.keywords {
            let known = photo
                .keywords
                .iter()
                .any(|name| name.trim().eq_ignore_ascii_case(keyword.trim()));
            if !known {
                photo.keywords.push(keyword.clone());
            }
        }
        Ok(Some(photo))
    }

    /// Send an accepted file on: straight to insertion, or to be copied.
    fn dispatch(&mut self, task: TaskId, index: usize, photo: NewPhoto) {
        let Some(run) = self.run_mut(task) else {
            return;
        };
        let ImportMode::Copy {
            destination,
            layout,
        } = &run.mode
        else {
            self.settle_slot(task, index, Some(photo));
            return;
        };
        let folder = match layout {
            CopyLayout::OneFolder => destination.clone(),
            CopyLayout::ByDate(pattern) => {
                let date = photo
                    .metadata
                    .capture_time
                    .or_else(|| DateTime::from_unix_seconds(photo.file_mtime));
                match date {
                    Some(date) => destination.join(pattern.folder_for(date)),
                    None => destination.clone(),
                }
            }
        };
        let token: Option<CancelToken> = self.tasks_token(task);
        let source = photo.path.clone();
        self.submit(
            Lane::Background,
            None,
            token,
            move |context| {
                let copied = files::copy_into(&source, &folder, || context.should_stop());
                Box::new(move |engine: &mut Engine| engine.import_copied(task, index, photo, copied))
            },
            move |engine, failure| engine.import_slot_failed(task, index, failure),
        );
    }

    fn tasks_token(&self, task: TaskId) -> Option<CancelToken> {
        self.tasks.token(task)
    }

    fn import_copied(
        &mut self,
        task: TaskId,
        index: usize,
        mut photo: NewPhoto,
        copied: Result<Copied, CopyError>,
    ) -> bool {
        match copied {
            Ok(copied) => {
                photo.path = copied.path;
                photo.file_size = copied.bytes;
                photo.file_mtime = copied.modified;
                self.settle_slot(task, index, Some(photo));
            }
            Err(CopyError::Cancelled) => self.settle_slot(task, index, None),
            Err(CopyError::Failed(message)) => {
                let path = photo.path.clone();
                if let Some(summary) = self.summary_mut(task) {
                    summary.errors.push(FileIssue { path, message });
                }
                self.settle_slot(task, index, None);
            }
        }
        self.advance_import(task);
        true
    }

    fn import_slot_failed(&mut self, task: TaskId, index: usize, failure: JobFailure) -> bool {
        if let JobFailure::Panicked(message) = failure
            && let Some(path) = self.working_path(task, index)
            && let Some(summary) = self.summary_mut(task)
        {
            summary.errors.push(FileIssue { path, message });
        }
        self.settle_slot(task, index, None);
        self.advance_import(task);
        true
    }

    /// A file is through its worker stages: ready to insert, or done.
    fn settle_slot(&mut self, task: TaskId, index: usize, photo: Option<NewPhoto>) {
        let Some(run) = self.run_mut(task) else {
            return;
        };
        if let Some(slot) = run.slots.get_mut(index)
            && matches!(slot, Slot::Working(_))
        {
            *slot = photo.map_or(Slot::Done, |photo| Slot::Ready(Box::new(photo)));
            run.working -= 1;
        }
    }

    /// Insert what is ready, in order, a batch at a time; finish the run
    /// when every file is through.
    fn advance_import(&mut self, task: TaskId) {
        let Some(run) = self.run_mut(task) else {
            return;
        };
        let start = run.next_insert;
        let end = run.slots[start..]
            .iter()
            .position(|slot| matches!(slot, Slot::Working(_)))
            .map_or(run.slots.len(), |offset| start + offset);
        let ready = run.slots[start..end]
            .iter()
            .filter(|slot| matches!(slot, Slot::Ready(_)))
            .count();
        let complete = run.working == 0;
        if ready >= BATCH || (complete && end > start) || (ready == 0 && end > start) {
            let batch: Vec<NewPhoto> = run.slots[start..end]
                .iter_mut()
                .filter_map(|slot| match std::mem::replace(slot, Slot::Done) {
                    Slot::Ready(photo) => Some(*photo),
                    Slot::Working(_) | Slot::Done => None,
                })
                .collect();
            run.next_insert = end;
            self.insert_batch(task, batch);
        }
        self.report_import_progress(task);
        if self.run_mut(task).is_some_and(|run| run.working == 0 && run.next_insert == run.slots.len()) {
            self.finish_import(task);
        }
    }

    fn insert_batch(&mut self, task: TaskId, batch: Vec<NewPhoto>) {
        let Some(import) = self.import_summary(task).map(|summary| summary.import) else {
            return;
        };
        if batch.is_empty() {
            return;
        }
        let mut imported = Vec::with_capacity(batch.len());
        let mut issues = Vec::new();
        match self.catalog.insert_photos(import, &batch) {
            Ok(ids) => imported = ids,
            // One bad file must not sink the others: retry them one by one.
            Err(_) => {
                for photo in &batch {
                    match self.catalog.insert_photo(import, photo) {
                        Ok(id) => imported.push(id),
                        Err(error) => {
                            self.forget_copy(task, photo);
                            issues.push(FileIssue {
                                path: photo.path.clone(),
                                message: error.to_string(),
                            });
                        }
                    }
                }
            }
        }
        if let Some(summary) = self.summary_mut(task) {
            summary.imported.extend(imported);
            summary.errors.extend(issues);
        }
        self.refresh_after(&[Change::Photos]);
    }

    /// A copy whose record could not be inserted is deleted again, so that
    /// every copy an import makes has its record.
    fn forget_copy(&self, task: TaskId, photo: &NewPhoto) {
        let copied = self
            .imports
            .runs
            .iter()
            .any(|run| run.task == task && matches!(run.mode, ImportMode::Copy { .. }));
        if copied && let Err(error) = std::fs::remove_file(&photo.path) {
            log::warn!("cannot delete the copy {}: {error}", photo.path.display());
        }
    }

    fn report_import_progress(&mut self, task: TaskId) {
        let Some(run) = self.imports.runs.iter().find(|run| run.task == task) else {
            return;
        };
        let done = (run.slots.len() - run.working) as u64;
        let errors = self
            .import_summary(task)
            .map_or(0, |summary| summary.errors.len() as u64);
        if let Some(info) = self.tasks.get_mut(task) {
            info.done = done;
            info.errors = errors;
        }
    }

    fn finish_import(&mut self, task: TaskId) {
        self.imports.runs.retain(|run| run.task != task);
        let cancelled = self.tasks.is_cancelled(task);
        let state = if cancelled {
            TaskState::Cancelled
        } else {
            TaskState::Finished
        };
        self.tasks.finish(task, state);
        let Some(summary) = self.summary_mut(task) else {
            return;
        };
        summary.finished = true;
        summary.cancelled = cancelled;
        let count = summary.imported.len();
        let message = match (cancelled, count) {
            (true, _) => format!("Import cancelled after {count} photos"),
            (false, 1) => "Imported 1 photo".to_string(),
            (false, _) => format!("Imported {count} photos"),
        };
        self.toasts.info(message);
        self.imports.finished.push(task);
        if self.imports.finished.len() > SUMMARIES_KEPT {
            let oldest = self.imports.finished.remove(0);
            self.imports.summaries.remove(&oldest);
        }
        if count > 0 {
            self.show_previous_import();
        }
    }

    fn show_previous_import(&mut self) {
        self.browser.set_source(Source::PreviousImport);
        self.browser.set_filter(photo_catalog::Filter::default());
        if let Err(error) = self.refresh(&Refresh::list()) {
            self.toasts.error(format!("Could not show the import: {error}"));
            return;
        }
        if let Some(first) = self.browser.photos().first().map(|photo| photo.id) {
            self.selection.select_only(&self.browser, first);
        }
    }
}
