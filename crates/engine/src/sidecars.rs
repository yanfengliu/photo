//! XMP sidecars: writing the catalog's metadata to them, and reading it
//! back, both in the background.
//!
//! A sidecar belongs to a file, so virtual copies are left out: a file's
//! master speaks for it. Orientation travels only in the sidecar's private
//! settings property, because the catalog does not know the file's own
//! orientation (see `photo_xmp::ImageInfo::orientation`).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use photo_catalog::{DevelopEdit, PhotoRecord};
use photo_core::develop::DevelopSettings;
use photo_core::meta::{FileKind, KeywordId, PhotoId};
use photo_xmp::{ImageInfo, XmpData};

use crate::commands::{Applied, Target};
use crate::engine::{Engine, JobFailure};
use crate::error::EngineError;
use crate::import::sidecar_keywords;
use crate::jobs::Lane;
use crate::keywording::replace_each;
use crate::tasks::{TaskId, TaskKind, TaskState};
use crate::undo::{Command, Values};

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

/// Sidecar tasks in progress.
#[derive(Default)]
pub(crate) struct Sidecars {
    runs: HashMap<TaskId, Run>,
}

#[derive(Default)]
struct Run {
    waiting: usize,
    /// What the read sidecars said, by photo.
    found: Vec<(PhotoId, XmpData)>,
    first_error: Option<String>,
}

impl Engine {
    /// Write every target photo's rating, flag, label, text, keywords and
    /// develop settings to its XMP sidecar, in the background. Only what the
    /// sidecar format owns is changed in an existing sidecar.
    pub fn save_metadata_to_files(&mut self, target: Target) -> Result<TaskId, EngineError> {
        let masters = self.masters(&target)?;
        let mut jobs = Vec::with_capacity(masters.len());
        for record in &masters {
            jobs.push((record.path.clone(), self.xmp_data(record)?, record));
        }
        let (task, token) = self.start_sidecar_task(
            TaskKind::SaveMetadata,
            format!("Saving metadata to {} files", jobs.len()),
            jobs.len(),
        );
        for (path, data, record) in jobs {
            let sidecar = photo_xmp::sidecar_path(&path, record.kind);
            let info = image_info(record.kind, record.metadata.width, record.metadata.height);
            self.submit(
                Lane::Background,
                None,
                Some(token.clone()),
                move |_| {
                    let written = photo_xmp::write_sidecar(&sidecar, &data, info)
                        .map_err(|error| error.to_string());
                    Box::new(move |engine: &mut Engine| engine.sidecar_done(task, None, written))
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
                    let read = photo_xmp::read_sidecar(&sidecar, info)
                        .map(|data| data.map(|data| (photo, data)))
                        .map_err(|error| error.to_string());
                    Box::new(move |engine: &mut Engine| match read {
                        Ok(found) => engine.sidecar_done(task, found, Ok(())),
                        Err(error) => engine.sidecar_done(task, None, Err(error)),
                    })
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

    /// What a sidecar should say about a photo.
    fn xmp_data(&self, record: &PhotoRecord) -> Result<XmpData, EngineError> {
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
        Ok(XmpData {
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
            develop: Some(self.catalog.develop_settings(record.id)?),
            // Only reading fills this in; it plays no part in a write.
            newer_settings_format: None,
        })
    }

    fn sidecar_done(
        &mut self,
        task: TaskId,
        found: Option<(PhotoId, XmpData)>,
        result: Result<(), String>,
    ) -> bool {
        if let Some(run) = self.sidecars.runs.get_mut(&task) {
            run.waiting = run.waiting.saturating_sub(1);
            run.found.extend(found);
            if let Err(error) = result {
                run.first_error.get_or_insert(error);
                if let Some(info) = self.tasks.get_mut(task) {
                    info.errors += 1;
                }
            }
        }
        self.sidecar_progress(task);
        true
    }

    fn sidecar_failed(&mut self, task: TaskId, failure: JobFailure) -> bool {
        let result = match failure {
            JobFailure::Skipped => Ok(()),
            JobFailure::Panicked(message) => Err(message),
        };
        self.sidecar_done(task, None, result)
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
        if let Some(error) = run.first_error {
            self.toasts.error(format!("Some sidecars failed: {error}"));
        } else if kind == Some(TaskKind::SaveMetadata) && !cancelled {
            self.toasts.info("Saved Metadata to Files");
        }
        self.tasks.finish(task, state);
    }

    /// Lay what the sidecars said over the photos, as one undoable step.
    fn apply_sidecars(&mut self, found: Vec<(PhotoId, XmpData)>) -> Result<(), EngineError> {
        if found.is_empty() {
            return Ok(());
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
        let mut wanted: BTreeMap<PhotoId, BTreeSet<KeywordId>> = BTreeMap::new();
        for (record, data) in pairs() {
            let ids = sidecar_keywords(data)
                .iter()
                .filter(|name| !name.trim().is_empty())
                .map(|name| catalog.ensure_keyword(name))
                .collect::<Result<BTreeSet<_>, _>>()?;
            wanted.insert(record.id, ids);
        }
        replace_each(catalog, applied, wanted)?;
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
