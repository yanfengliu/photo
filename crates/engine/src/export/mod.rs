//! Exporting photos as image files, through photo-export, as a background
//! task.
//!
//! [`Engine::start_export`] reads everything each photo needs from the
//! catalog at once, numbers the photos in the target's order for the
//! `{seq}` token, and renders them on the workers at most [`AT_ONCE`] at a
//! time, however many exports run (the oldest export goes first), since a
//! 60-megapixel export holds about 600 MB. A photo whose
//! decoded source is in memory already (open in Develop, say) is exported
//! from it. Cancelling stops the photos rendering, which leave no file
//! behind, and starts no more. Before and after each photo (see `job`) the
//! [`Guard`] checks what the export crate cannot know: whether the file
//! would be the original of another photo in the catalog, or inside the
//! catalog's own folders. Presets are in `presets`.

mod guard;
mod job;
mod presets;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use photo_catalog::{CatalogError, PhotoRecord};
use photo_core::develop::DevelopSettings;
use photo_core::meta::PhotoId;
use photo_export::{ExportAttributes, ExportOutcome, ExportSettings, Folder, Location};

use crate::commands::Target;
use crate::engine::{Engine, JobFailure};
use crate::error::EngineError;
use crate::jobs::{CancelToken, Lane};
use crate::tasks::{TaskId, TaskKind, TaskState};
use guard::Guard;
use job::{Failure, Pending, export_one, summary};

/// Photos rendering at once, over every export.
const AT_ONCE: usize = 2;
/// How many finished exports keep their per-photo results.
const RESULTS_KEPT: usize = 8;
/// Why a photo is not exported when its settings come from a newer build.
const NEWER: &str =
    "its develop settings were saved by a newer version of Photo, so it cannot be rendered here";

/// What became of one photo of an export.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportedPhoto {
    /// The photo.
    pub photo: PhotoId,
    /// Its number in the export, as the `{seq}` token writes it.
    pub sequence: u32,
    /// The file written, or the existing file skipped; or why nothing was
    /// written.
    pub result: Result<ExportOutcome, String>,
}

/// An export in progress.
pub(crate) struct ExportRun {
    settings: Arc<ExportSettings>,
    guard: Arc<Guard>,
    waiting: VecDeque<Pending>,
    running: usize,
    total: usize,
    results: Vec<ExportedPhoto>,
    /// The task's token: queued jobs are skipped once it is cancelled.
    token: CancelToken,
    /// Stops the renders under way.
    renders: photo_develop::CancelToken,
}

/// The engine's exports: those running and the results of the latest.
#[derive(Default)]
pub(crate) struct Exports {
    runs: HashMap<TaskId, ExportRun>,
    /// Photos rendering, over every run.
    rendering: usize,
    results: VecDeque<(TaskId, Vec<ExportedPhoto>)>,
}

impl Exports {
    /// Stop the renders of the export of `task`.
    pub(crate) fn cancel(&self, task: TaskId) {
        if let Some(run) = self.runs.get(&task) {
            run.renders.cancel();
        }
    }

    /// Stop every export's renders, for shutting down.
    pub(crate) fn cancel_all(&self) {
        for run in self.runs.values() {
            run.renders.cancel();
        }
    }
}

impl Engine {
    /// The folder exports go to unless the user chooses another.
    pub fn export_dir(&self) -> &Path {
        &self.export_dir
    }

    /// Export settings as a new export starts: the export crate's defaults
    /// (full-size JPEG at quality 90, named as the original, a numbered
    /// name when it is taken), written to [`Engine::export_dir`].
    pub fn default_export_settings(&self) -> ExportSettings {
        ExportSettings {
            location: Location {
                folder: Folder::Specific(self.export_dir.clone()),
                ..Location::default()
            },
            ..ExportSettings::default()
        }
    }

    /// Where the first photo of `target` would be written now, for the
    /// export dialog's preview of the name; `None` when the target holds no
    /// photo. Nothing is written.
    pub fn planned_export_path(
        &self,
        target: Target,
        settings: &ExportSettings,
    ) -> Result<Option<PathBuf>, EngineError> {
        let Some(first) = self.resolve(&target).first().copied() else {
            return Ok(None);
        };
        let record = self.catalog.photo(first)?;
        let pending = Pending {
            photo: first,
            sequence: 1,
            path: record.path.clone(),
            settings: DevelopSettings::default_for(record.kind),
            metadata: record.metadata,
            attributes: ExportAttributes::default(),
        };
        Ok(Some(photo_export::planned_path(
            &pending.item(None),
            settings,
        )))
    }

    /// Export every target photo, in the target's order, as `settings` say,
    /// in the background; returns the task, which reports progress and can
    /// be cancelled. An edit open in Develop is committed first, so what is
    /// exported is what is shown. Each photo's result is in
    /// [`Engine::export_results`]; a toast sums them up at the end.
    ///
    /// A photo is refused, with its reason and without anything written,
    /// when its file would be the original of a photo in the catalog
    /// (whatever the existing-file policy, and ignoring letter case, as the
    /// file systems photos live on do) or would go inside the catalog's or
    /// the previews' folder.
    pub fn start_export(
        &mut self,
        target: Target,
        settings: ExportSettings,
    ) -> Result<TaskId, EngineError> {
        let photos = self.resolve(&target);
        if photos.is_empty() {
            return Err(EngineError::NothingToExport);
        }
        self.commit_edit()?;
        let records: HashMap<PhotoId, PhotoRecord> = self
            .catalog
            .photos(&photos)?
            .into_iter()
            .map(|record| (record.id, record))
            .collect();
        let mut waiting = VecDeque::with_capacity(photos.len());
        let mut results = Vec::new();
        for (index, photo) in photos.iter().enumerate() {
            let sequence = u32::try_from(index + 1).unwrap_or(u32::MAX);
            let Some(record) = records.get(photo) else {
                return Err(CatalogError::NotFound {
                    entity: photo_catalog::Entity::Photo,
                    id: photo.0,
                }
                .into());
            };
            match self.catalog.develop_settings(*photo) {
                Ok(develop) => waiting.push_back(self.pending(record, develop, sequence)?),
                Err(CatalogError::NewerFormat { .. }) => results.push(ExportedPhoto {
                    photo: *photo,
                    sequence,
                    result: Err(NEWER.to_string()),
                }),
                Err(error) => return Err(error.into()),
            }
        }
        let destinations: HashSet<PathBuf> = waiting
            .iter()
            .filter_map(|pending| {
                let planned = photo_export::planned_path(&pending.item(None), &settings);
                planned.parent().map(Path::to_path_buf)
            })
            .collect();
        let own = [self.catalog_dir.as_path(), self.previews_dir.as_path()];
        let guard = Guard::new(&self.catalog, &own, destinations)?;
        let total = photos.len();
        let label = match total {
            1 => "Exporting 1 photo".to_string(),
            count => format!("Exporting {count} photos"),
        };
        let (task, token) = self.tasks.start(TaskKind::Export, label, total as u64);
        if let Some(info) = self.tasks.get_mut(task) {
            info.done = results.len() as u64;
            info.errors = results.len() as u64;
        }
        self.exports.runs.insert(
            task,
            ExportRun {
                settings: Arc::new(settings),
                guard: Arc::new(guard),
                waiting,
                running: 0,
                total,
                results,
                token,
                renders: photo_develop::CancelToken::new(),
            },
        );
        self.pump_exports();
        Ok(task)
    }

    /// What became of each photo of a running or recently finished export,
    /// in the order they were given. Photos a cancellation stopped are not
    /// listed.
    pub fn export_results(&self, task: TaskId) -> Option<&[ExportedPhoto]> {
        if let Some(run) = self.exports.runs.get(&task) {
            return Some(&run.results);
        }
        self.exports
            .results
            .iter()
            .find(|(id, _)| *id == task)
            .map(|(_, results)| results.as_slice())
    }

    /// What the job of one photo needs, read from the catalog.
    fn pending(
        &self,
        record: &PhotoRecord,
        settings: DevelopSettings,
        sequence: u32,
    ) -> Result<Pending, EngineError> {
        let text = |value: &str| (!value.trim().is_empty()).then(|| value.to_string());
        let keywords = self
            .catalog
            .photo_keywords(record.id)?
            .into_iter()
            .map(|keyword| keyword.name)
            .collect();
        Ok(Pending {
            photo: record.id,
            sequence,
            path: record.path.clone(),
            settings,
            metadata: record.metadata.clone(),
            attributes: ExportAttributes {
                title: text(&record.title),
                caption: text(&record.caption),
                creator: text(&record.creator),
                copyright: text(&record.copyright),
                keywords,
                rating: record.rating,
            },
        })
    }

    /// Start photos, the oldest export's first, until [`AT_ONCE`] are
    /// rendering; then finish every export with nothing left to wait for.
    fn pump_exports(&mut self) {
        while self.exports.rendering < AT_ONCE {
            let next = self
                .exports
                .runs
                .iter()
                .filter(|(_, run)| !run.waiting.is_empty() && !run.token.is_cancelled())
                .map(|(task, _)| *task)
                .min();
            let Some(task) = next else {
                break;
            };
            self.start_photo(task);
        }
        let over: Vec<TaskId> = self
            .exports
            .runs
            .iter()
            .filter(|(_, run)| {
                run.running == 0 && (run.waiting.is_empty() || run.token.is_cancelled())
            })
            .map(|(task, _)| *task)
            .collect();
        for task in over {
            self.finish_export(task);
        }
    }

    /// Submit the next photo of the export of `task`.
    fn start_photo(&mut self, task: TaskId) {
        let Some(run) = self.exports.runs.get_mut(&task) else {
            return;
        };
        let Some(pending) = run.waiting.pop_front() else {
            return;
        };
        run.running += 1;
        self.exports.rendering += 1;
        let settings = Arc::clone(&run.settings);
        let guard = Arc::clone(&run.guard);
        let (token, renders) = (run.token.clone(), run.renders.clone());
        let sources = self.sources.clone();
        let (photo, sequence) = (pending.photo, pending.sequence);
        self.submit(
            Lane::Background,
            None,
            Some(token),
            move |_| {
                let result = export_one(&pending, &settings, &guard, &sources, &renders);
                Box::new(move |engine: &mut Engine| {
                    engine.export_done(task, photo, sequence, result)
                })
            },
            move |engine, failure| {
                let result = match failure {
                    JobFailure::Skipped => Err(Failure::Cancelled),
                    JobFailure::Panicked(message) => {
                        Err(Failure::Reason(format!("the export failed: {message}")))
                    }
                };
                engine.export_done(task, photo, sequence, result)
            },
        );
    }

    fn export_done(
        &mut self,
        task: TaskId,
        photo: PhotoId,
        sequence: u32,
        result: Result<ExportOutcome, Failure>,
    ) -> bool {
        self.exports.rendering = self.exports.rendering.saturating_sub(1);
        let Some(run) = self.exports.runs.get_mut(&task) else {
            self.pump_exports();
            return false;
        };
        run.running = run.running.saturating_sub(1);
        let (result, failed) = match result {
            Ok(outcome) => (Ok(outcome), false),
            Err(Failure::Cancelled) => {
                self.pump_exports();
                return true;
            }
            Err(Failure::Reason(reason)) => (Err(reason), true),
        };
        let at = run
            .results
            .partition_point(|exported| exported.sequence < sequence);
        run.results.insert(
            at,
            ExportedPhoto {
                photo,
                sequence,
                result,
            },
        );
        if let Some(info) = self.tasks.get_mut(task) {
            info.done += 1;
            info.errors += u64::from(failed);
        }
        self.pump_exports();
        true
    }

    /// End an export that has no photo rendering and none left to start
    /// (or was cancelled): set the task's state, sum it up in a toast and
    /// keep the results.
    fn finish_export(&mut self, task: TaskId) {
        let Some(run) = self.exports.runs.remove(&task) else {
            return;
        };
        let cancelled = run.token.is_cancelled();
        let written = run
            .results
            .iter()
            .filter(|exported| matches!(exported.result, Ok(ExportOutcome::Written { .. })))
            .count();
        let skipped = run
            .results
            .iter()
            .filter(|exported| matches!(exported.result, Ok(ExportOutcome::Skipped { .. })))
            .count();
        let failed = run
            .results
            .iter()
            .filter(|exported| exported.result.is_err())
            .count();
        let message = summary(written, skipped, failed, run.total, cancelled);
        if failed > 0 {
            self.toasts.error(message);
        } else {
            self.toasts.info(message);
        }
        let state = if cancelled {
            TaskState::Cancelled
        } else {
            TaskState::Finished
        };
        self.tasks.finish(task, state);
        self.exports.results.push_back((task, run.results));
        if self.exports.results.len() > RESULTS_KEPT {
            self.exports.results.pop_front();
        }
    }
}
