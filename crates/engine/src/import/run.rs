//! One import in progress: where each listed file is, deciding what goes
//! in, and inserting the accepted files in order, a batch at a time.

use std::collections::HashSet;
use std::path::PathBuf;

use photo_catalog::{CatalogError, NewPhoto, Source};
use photo_core::time::DateTime;

use super::files::{self, Copied, CopyError, ReadFile};
use super::record::record;
use super::{BATCH, CopyLayout, FileIssue, ImportMode, ImportSummary, SUMMARIES_KEPT};
use crate::browse::{Change, Refresh};
use crate::engine::{Engine, JobFailure};
use crate::jobs::{CancelToken, Lane};
use crate::tasks::{TaskId, TaskState};

/// Where one listed file is.
pub(super) enum Slot {
    /// Being read, or copied.
    Working(PathBuf),
    /// Read (and copied) and waiting for its turn to be inserted.
    Ready(Box<NewPhoto>),
    /// Inserted, skipped or failed.
    Done,
}

/// An import in progress.
pub(super) struct Run {
    pub(super) task: TaskId,
    pub(super) mode: ImportMode,
    pub(super) skip_duplicates: bool,
    pub(super) keywords: Vec<String>,
    pub(super) slots: Vec<Slot>,
    /// Slots before this one have been inserted or dropped.
    pub(super) next_insert: usize,
    pub(super) working: usize,
    /// File name, size and capture time of each file accepted so far.
    pub(super) accepted: HashSet<(String, u64, Option<DateTime>)>,
    /// Photos accepted with the starting settings because this build
    /// cannot read the develop settings in their sidecar.
    pub(super) unreadable_settings: usize,
}

impl Engine {
    fn run_mut(&mut self, task: TaskId) -> Option<&mut Run> {
        self.imports.runs.iter_mut().find(|run| run.task == task)
    }

    fn summary_mut(&mut self, task: TaskId) -> Option<&mut ImportSummary> {
        self.imports.summaries.get_mut(&task)
    }

    pub(super) fn import_read(
        &mut self,
        task: TaskId,
        index: usize,
        read: Result<ReadFile, String>,
    ) -> bool {
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
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let key = (name, photo.file_size, photo.metadata.capture_time);
        let Some(run) = self.imports.runs.iter().find(|run| run.task == task) else {
            return Ok(None);
        };
        let in_place = matches!(run.mode, ImportMode::Add);
        let skip_duplicates = run.skip_duplicates;
        let earlier = run.accepted.contains(&key);
        if in_place
            && self
                .catalog
                .find_by_path(path)
                .map_err(catalog_error)?
                .is_some()
        {
            if let Some(summary) = self.summary_mut(task) {
                summary.already_in_catalog.push(path.to_path_buf());
            }
            return Ok(None);
        }
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
        // The photo comes in with the starting settings, and says so: they
        // are not the edit its sidecar holds.
        if let Some(format) = read.unreadable_settings
            && let Some(summary) = self.summary_mut(task)
        {
            summary.errors.push(issue(format!(
                "the develop settings in its sidecar (format {format}) cannot be read by this version: it was imported with the starting settings, and the sidecar keeps its own"
            )));
        }
        let Some(run) = self.run_mut(task) else {
            return Ok(None);
        };
        run.unreadable_settings += usize::from(read.unreadable_settings.is_some());
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
        let folder = match self.imports.runs.iter().find(|run| run.task == task) {
            None => return,
            Some(Run {
                mode: ImportMode::Add,
                ..
            }) => None,
            Some(Run {
                mode:
                    ImportMode::Copy {
                        destination,
                        layout,
                    },
                ..
            }) => Some(copy_folder(destination, layout, &photo)),
        };
        let Some(folder) = folder else {
            self.settle_slot(task, index, Some(photo));
            return;
        };
        let token: Option<CancelToken> = self.tasks.token(task);
        let source = photo.path.clone();
        self.submit(
            Lane::Background,
            None,
            token,
            move |context| {
                let copied = files::copy_into(&source, &folder, || context.should_stop());
                Box::new(move |engine: &mut Engine| {
                    engine.import_copied(task, index, photo, copied)
                })
            },
            move |engine, failure| engine.import_slot_failed(task, index, failure),
        );
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

    pub(super) fn import_slot_failed(
        &mut self,
        task: TaskId,
        index: usize,
        failure: JobFailure,
    ) -> bool {
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
    pub(super) fn advance_import(&mut self, task: TaskId) {
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
        if self
            .run_mut(task)
            .is_some_and(|run| run.working == 0 && run.next_insert == run.slots.len())
        {
            self.finish_import(task);
        }
    }

    /// Hand a batch to the catalog and note what became of it (see
    /// [`record`]). A photo that did not go in is reported, and its copy
    /// deleted again: one the catalog refused, and every photo of a batch
    /// the catalog could not take at all.
    ///
    /// As the engine closes there is no one to report to and no time to
    /// wait or to tidy up. The catalog is waited for only as long as
    /// closing still has, and what it could not take stays where it is and
    /// is named in the log.
    fn insert_batch(&mut self, task: TaskId, batch: Vec<NewPhoto>) {
        let Some(import) = self.import_summary(task).map(|summary| summary.import) else {
            return;
        };
        if batch.is_empty() {
            return;
        }
        let closing = self.imports.closing;
        if let Some(deadline) = closing {
            self.wait_for_catalog_until(deadline);
        }
        let recorded = record(
            &mut self.catalog,
            &batch,
            |catalog, photos| catalog.insert_photos(import, photos),
            |catalog, photo| catalog.insert_photo(import, photo),
        );
        let mut issues = Vec::new();
        let mut issue = |photo: &NewPhoto, error: &CatalogError| {
            issues.push(FileIssue {
                path: photo.path.clone(),
                message: error.to_string(),
            });
        };
        for (photo, error) in &recorded.refused {
            self.forget_copy(task, photo);
            if closing.is_some() {
                log::error!(
                    "{} was not imported as the application closed: {error}",
                    photo.path.display()
                );
            }
            issue(photo, error);
        }
        if let Some((photos, error)) = &recorded.stranded {
            for photo in *photos {
                if closing.is_some() {
                    log::error!(
                        "{} could not be recorded in the catalog as the application closed ({error}); the file is left where it is, and importing its folder adds it",
                        photo.path.display()
                    );
                } else {
                    self.forget_copy(task, photo);
                }
                issue(photo, error);
            }
        }
        if let Some(summary) = self.summary_mut(task) {
            summary.imported.extend(recorded.ids);
            summary.errors.extend(issues);
        }
        self.refresh_after(&[Change::Photos]);
    }

    /// As the engine closes: insert every file that is read (and copied)
    /// and still waiting for its turn, whatever is still being worked on
    /// before it, so that every copy made has its record.
    pub(crate) fn record_ready_imports(&mut self) {
        let tasks: Vec<TaskId> = self.imports.runs.iter().map(|run| run.task).collect();
        for task in tasks {
            let Some(run) = self.run_mut(task) else {
                continue;
            };
            let batch: Vec<NewPhoto> = run
                .slots
                .iter_mut()
                .filter(|slot| matches!(slot, Slot::Ready(_)))
                .filter_map(|slot| match std::mem::replace(slot, Slot::Done) {
                    Slot::Ready(photo) => Some(*photo),
                    Slot::Working(_) | Slot::Done => None,
                })
                .collect();
            self.insert_batch(task, batch);
        }
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
        let unreadable = self.run_mut(task).map_or(0, |run| run.unreadable_settings);
        if unreadable > 0 {
            self.toasts.error(format!(
                "{unreadable} photos were imported with the starting settings: this version cannot read the develop settings in their sidecars"
            ));
        }
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
            self.toasts
                .error(format!("Could not show the import: {error}"));
            return;
        }
        if let Some(first) = self.browser.photos().first().map(|photo| photo.id) {
            self.selection.select_only(&self.browser, first);
        }
    }
}

/// The folder a copy of `photo` goes to.
fn copy_folder(destination: &std::path::Path, layout: &CopyLayout, photo: &NewPhoto) -> PathBuf {
    match layout {
        CopyLayout::OneFolder => destination.to_path_buf(),
        CopyLayout::ByDate(pattern) => {
            let date = photo
                .metadata
                .capture_time
                .or_else(|| DateTime::from_unix_seconds(photo.file_mtime));
            match date {
                Some(date) => destination.join(pattern.folder_for(date)),
                None => destination.to_path_buf(),
            }
        }
    }
}
