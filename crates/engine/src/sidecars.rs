//! XMP sidecars: writing the catalog's metadata to them, and reading it
//! back, both in the background.
//!
//! A sidecar belongs to a file, so virtual copies are left out: a file's
//! master speaks for it. Saves of one sidecar are put in order (see
//! `files::SaveOrder`): the file ends with what the last save asked for
//! held. Orientation travels only in the sidecar's private
//! settings property, because the catalog does not know the file's own
//! orientation (see `photo_xmp::ImageInfo::orientation`).
//!
//! What goes wrong goes wrong per photo and is listed per photo (see
//! [`Engine::metadata_issues`]): a sidecar whose develop settings this build
//! cannot read keeps them (everything else is still written, and nothing of
//! them is read), and a sidecar another application kept changing is left
//! as that application saved it.

mod files;

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use photo_catalog::{CatalogError, DevelopEdit, PhotoRecord};
use photo_core::develop::DevelopSettings;
use photo_core::meta::{FileKind, KeywordId, PhotoId};
use photo_xmp::{ImageInfo, XmpData};

use files::{Outcome, SaveOrder, issue, read_one, save_one};

use crate::commands::{Applied, Target};
use crate::engine::{Engine, JobFailure};
use crate::error::EngineError;
use crate::import::{FileIssue, sidecar_keywords};
use crate::jobs::Lane;
use crate::keywording::set_lists;
use crate::tasks::{TaskId, TaskKind, TaskState};
use crate::undo::{Command, Values};

/// How many finished sidecar tasks keep their list of issues.
const REPORTS_KEPT: usize = 8;

/// What the sidecar mapping is told about a photo's file. The same for
/// writing and reading, as `photo_xmp` requires.
pub(crate) fn image_info(kind: FileKind, width: u32, height: u32) -> ImageInfo {
    let info = ImageInfo::new(kind);
    if width > 0 && height > 0 {
        info.with_size(width, height)
    } else {
        info
    }
}

/// Sidecar tasks in progress, and the issues of the ones finished lately.
#[derive(Default)]
pub(crate) struct Sidecars {
    runs: HashMap<TaskId, Run>,
    reports: VecDeque<(TaskId, Vec<FileIssue>)>,
    /// The order of the saves of each sidecar.
    order: SaveOrder,
}

#[derive(Default)]
struct Run {
    waiting: usize,
    /// What the read sidecars said, by photo.
    found: Vec<(PhotoId, XmpData)>,
    issues: Vec<FileIssue>,
}

impl Engine {
    /// Write every target photo's rating, flag, label, text, keywords and
    /// develop settings to its XMP sidecar, in the background. Only what the
    /// sidecar format owns is changed in an existing sidecar.
    pub fn save_metadata_to_files(&mut self, target: Target) -> Result<TaskId, EngineError> {
        self.commit_edit()?;
        let masters = self.masters(&target)?;
        let mut jobs = Vec::with_capacity(masters.len());
        for record in &masters {
            let sidecar = photo_xmp::sidecar_path(&record.path, record.kind);
            let (data, note) = self.xmp_data(record, &sidecar)?;
            jobs.push((sidecar, data, note, record));
        }
        let (task, token) = self.start_sidecar_task(
            TaskKind::SaveMetadata,
            format!("Saving metadata to {} files", jobs.len()),
            jobs.len(),
        );
        for (sidecar, data, note, record) in jobs {
            let info = image_info(record.kind, record.metadata.width, record.metadata.height);
            let ticket = self.sidecars.order.claim(&sidecar);
            self.submit(
                Lane::Background,
                None,
                Some(token.clone()),
                move |_| {
                    let outcome = save_one(&ticket, &sidecar, &data, info, note);
                    Box::new(move |engine: &mut Engine| engine.sidecar_done(task, outcome))
                },
                move |engine, failure| engine.sidecar_failed(task, failure),
            );
        }
        self.sidecar_progress(task);
        Ok(task)
    }

    /// Read every target photo's XMP sidecar in the background and, once all
    /// are read, apply what they say (rating, flag, label, text, keywords
    /// and develop settings) as one undoable change. Photos without a
    /// sidecar are left alone.
    pub fn read_metadata_from_files(&mut self, target: Target) -> Result<TaskId, EngineError> {
        let masters = self.masters(&target)?;
        let (task, token) = self.start_sidecar_task(
            TaskKind::ReadMetadata,
            format!("Reading metadata from {} files", masters.len()),
            masters.len(),
        );
        for record in masters {
            let photo = record.id;
            let sidecar = photo_xmp::sidecar_path(&record.path, record.kind);
            let info = image_info(record.kind, record.metadata.width, record.metadata.height);
            self.submit(
                Lane::Background,
                None,
                Some(token.clone()),
                move |_| {
                    let outcome = read_one(&sidecar, photo, info);
                    Box::new(move |engine: &mut Engine| engine.sidecar_done(task, outcome))
                },
                move |engine, failure| engine.sidecar_failed(task, failure),
            );
        }
        self.sidecar_progress(task);
        Ok(task)
    }

    fn masters(&self, target: &Target) -> Result<Vec<PhotoRecord>, EngineError> {
        let photos = self.resolve(target);
        let records = if photos.is_empty() {
            Vec::new()
        } else {
            self.catalog.photos(&photos)?
        };
        Ok(records
            .into_iter()
            .filter(|record| record.master_id.is_none())
            .collect())
    }

    fn start_sidecar_task(
        &mut self,
        kind: TaskKind,
        label: String,
        count: usize,
    ) -> (TaskId, crate::jobs::CancelToken) {
        let started = self.tasks.start(kind, label, count as u64);
        self.sidecars.runs.insert(
            started.0,
            Run {
                waiting: count,
                ..Run::default()
            },
        );
        started
    }

    /// What went wrong, per photo, in a recently finished (or running)
    /// metadata task; empty when nothing did.
    pub fn metadata_issues(&self, task: TaskId) -> Option<&[FileIssue]> {
        if let Some(run) = self.sidecars.runs.get(&task) {
            return Some(&run.issues);
        }
        self.sidecars
            .reports
            .iter()
            .find(|(id, _)| *id == task)
            .map(|(_, issues)| issues.as_slice())
    }

    /// What a sidecar should say about a photo, with a note when its
    /// develop settings cannot be written (they were saved by a newer
    /// version, so the sidecar's own are left as they are).
    fn xmp_data(
        &self,
        record: &PhotoRecord,
        sidecar: &Path,
    ) -> Result<(XmpData, Option<FileIssue>), EngineError> {
        let (develop, note) = match self.catalog.develop_settings(record.id) {
            Ok(settings) => (Some(settings), None),
            Err(CatalogError::NewerFormat { .. }) => (
                None,
                issue(
                    sidecar,
                    "the develop settings were not written: they were saved by a newer version",
                ),
            ),
            Err(error) => return Err(error.into()),
        };
        let keywords = self.catalog.photo_keywords(record.id)?;
        let tree: HashMap<KeywordId, (&str, Option<KeywordId>)> = self
            .browser
            .keywords()
            .iter()
            .map(|keyword| (keyword.id, (keyword.name.as_str(), keyword.parent)))
            .collect();
        let path_of = |id: KeywordId| {
            let mut path = Vec::new();
            let mut next = Some(id);
            while let Some(id) = next.filter(|_| path.len() <= tree.len()) {
                let Some((name, parent)) = tree.get(&id) else {
                    break;
                };
                path.push((*name).to_string());
                next = *parent;
            }
            path.reverse();
            path
        };
        let data = XmpData {
            rating: record.rating,
            flag: record.flag,
            label: record.label,
            title: record.title.clone(),
            caption: record.caption.clone(),
            creators: if record.creator.is_empty() {
                Vec::new()
            } else {
                vec![record.creator.clone()]
            },
            copyright: record.copyright.clone(),
            keywords: keywords
                .iter()
                .map(|keyword| keyword.name.clone())
                .collect(),
            hierarchical_keywords: keywords.iter().map(|keyword| path_of(keyword.id)).collect(),
            capture_time: record.metadata.capture_time,
            develop,
            // Only reading fills this in; it plays no part in a write.
            unreadable_settings_format: None,
        };
        Ok((data, note))
    }

    fn sidecar_done(&mut self, task: TaskId, outcome: Outcome) -> bool {
        if let Some(run) = self.sidecars.runs.get_mut(&task) {
            run.waiting = run.waiting.saturating_sub(1);
            run.found.extend(outcome.found);
            run.issues.extend(outcome.issue);
            if outcome.failed
                && let Some(info) = self.tasks.get_mut(task)
            {
                info.errors += 1;
            }
        }
        self.sidecar_progress(task);
        true
    }

    fn sidecar_failed(&mut self, task: TaskId, failure: JobFailure) -> bool {
        let outcome = match failure {
            JobFailure::Skipped => Outcome {
                found: None,
                issue: None,
                failed: false,
            },
            JobFailure::Panicked(message) => Outcome {
                found: None,
                issue: Some(FileIssue {
                    path: PathBuf::new(),
                    message,
                }),
                failed: true,
            },
        };
        self.sidecar_done(task, outcome)
    }

    fn sidecar_progress(&mut self, task: TaskId) {
        let Some(run) = self.sidecars.runs.get(&task) else {
            return;
        };
        let waiting = run.waiting;
        if let Some(info) = self.tasks.get_mut(task) {
            info.done = info.total.saturating_sub(waiting as u64);
        }
        if waiting > 0 {
            return;
        }
        let Some(run) = self.sidecars.runs.remove(&task) else {
            return;
        };
        let cancelled = self.tasks.is_cancelled(task);
        let kind = self.tasks.get(task).map(|info| info.kind);
        let mut state = if cancelled {
            TaskState::Cancelled
        } else {
            TaskState::Finished
        };
        if kind == Some(TaskKind::ReadMetadata)
            && !cancelled
            && let Err(error) = self.apply_sidecars(run.found)
        {
            self.toasts.error(format!(
                "Could not apply the metadata read from files: {error}"
            ));
            state = TaskState::Failed(error.to_string());
        }
        if let Some(first) = run.issues.first() {
            self.toasts.error(format!(
                "{} files had problems, among them {}: {}",
                run.issues.len(),
                first.path.display(),
                first.message
            ));
        } else if kind == Some(TaskKind::SaveMetadata) && !cancelled {
            self.toasts.info("Saved Metadata to Files");
        }
        self.tasks.finish(task, state);
        self.sidecars.order.forget_idle();
        self.sidecars.reports.push_back((task, run.issues));
        if self.sidecars.reports.len() > REPORTS_KEPT {
            self.sidecars.reports.pop_front();
        }
    }

    /// Lay what the sidecars said over the photos, as one undoable step.
    ///
    /// A sidecar whose develop settings differ from the photo's replaces
    /// them. When that photo is open in Develop with an edit not yet
    /// committed, the edit is committed first, so that it is a step to go
    /// back to and not gone under the sidecar's settings. An edit of a
    /// photo whose settings no sidecar replaces is left open.
    fn apply_sidecars(&mut self, found: Vec<(PhotoId, XmpData)>) -> Result<(), EngineError> {
        if found.is_empty() {
            return Ok(());
        }
        let editing = self
            .develop
            .as_ref()
            .filter(|session| session.is_editing())
            .map(|session| session.photo);
        if let Some(open) = editing
            && let Some((_, data)) = found.iter().find(|(photo, _)| *photo == open)
            && let Some(read) = &data.develop
            && *read != self.catalog.develop_settings(open)?
        {
            self.commit_edit()?;
        }
        let photos: Vec<PhotoId> = found.iter().map(|(photo, _)| *photo).collect();
        let records: HashMap<PhotoId, PhotoRecord> = self
            .catalog
            .photos(&photos)?
            .into_iter()
            .map(|record| (record.id, record))
            .collect();
        let mut settings = Vec::new();
        for (photo, data) in &found {
            if let Some(read) = &data.develop
                && *read != self.catalog.develop_settings(*photo)?
            {
                settings.push((*photo, read.clone()));
            }
        }
        let mut applied = Applied::default();
        if let Err(error) = self.lay_over(&mut applied, &found, &records, &settings) {
            applied.roll_back(&mut self.catalog);
            return Err(error);
        }
        self.record_applied("Read Metadata from Files".to_string(), applied);
        Ok(())
    }

    fn lay_over(
        &mut self,
        applied: &mut Applied,
        found: &[(PhotoId, XmpData)],
        records: &HashMap<PhotoId, PhotoRecord>,
        settings: &[(PhotoId, DevelopSettings)],
    ) -> Result<(), EngineError> {
        let pairs = || {
            found
                .iter()
                .filter_map(|(photo, data)| records.get(photo).map(|record| (record, data)))
        };
        let catalog = &mut self.catalog;
        let ratings = Values::changes(pairs().map(|(r, d)| (r.id, r.rating, d.rating)));
        let flags = Values::changes(pairs().map(|(r, d)| (r.id, r.flag, d.flag)));
        let labels = Values::changes(pairs().map(|(r, d)| (r.id, r.label, d.label)));
        let commands = [
            ratings.map(Command::Ratings),
            flags.map(Command::Flags),
            labels.map(Command::Labels),
        ];
        for command in commands.into_iter().flatten() {
            applied.apply(catalog, command)?;
        }
        use photo_catalog::TextField::{Caption, Copyright, Creator, Title};
        let texts = [
            (
                Title,
                pairs()
                    .map(|(r, d)| (r.id, r.title.clone(), d.title.clone()))
                    .collect::<Vec<_>>(),
            ),
            (
                Caption,
                pairs()
                    .map(|(r, d)| (r.id, r.caption.clone(), d.caption.clone()))
                    .collect(),
            ),
            (
                Creator,
                pairs()
                    .map(|(r, d)| (r.id, r.creator.clone(), d.creators.join("; ")))
                    .collect(),
            ),
            (
                Copyright,
                pairs()
                    .map(|(r, d)| (r.id, r.copyright.clone(), d.copyright.clone()))
                    .collect(),
            ),
        ];
        for (field, changes) in texts {
            if let Some(values) = Values::changes(changes) {
                applied.apply(catalog, Command::Text(field, values))?;
            }
        }
        let lists = pairs()
            .map(|(record, data)| (record.id, sidecar_keywords(data)))
            .collect();
        set_lists(catalog, applied, lists)?;
        if !settings.is_empty() {
            let edits: Vec<DevelopEdit<'_>> = settings
                .iter()
                .map(|(photo, settings)| DevelopEdit {
                    photo: *photo,
                    settings,
                    history_label: Some("Read Metadata"),
                })
                .collect();
            applied.push(Command::Develop(catalog.apply_develop_edits(&edits)?));
        }
        Ok(())
    }
}
