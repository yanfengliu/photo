//! Thumbnails and standard previews of catalogued photos, and thumbnails of
//! files offered for import.
//!
//! A preview is asked for with [`Engine::preview`] (or [`Engine::thumbnail`]
//! and [`Engine::standard_preview`]); the answer is immediate: ready pixels,
//! pending, or failed. Behind it, in order:
//!
//! 1. The memory cache, bounded by bytes, keyed by photo and size, holding
//!    the pixels of one develop version.
//! 2. The preview file on disk for the photo's current develop version (see
//!    `disk`). A worker reads it only after checking, in the same job, that
//!    the source file still has the size and modification time recorded in
//!    it.
//! 3. A render through the seam in `render`, written back to disk, with the
//!    older versions' files deleted. The seam renders the photo's develop
//!    settings with the develop pipeline, from the decoded source shared
//!    with the develop session (see `sources`); while a RAW photo renders,
//!    its embedded camera preview is shown. A photo whose settings were
//!    saved by a newer version shows the decoder's own picture, which is
//!    what its origin says.
//!
//! Requests are served newest first, and asking again for a preview that is
//! still queued moves it to the front, so scrolling stays responsive. A
//! source file that has gone is recorded as missing in the catalog, and the
//! failure is remembered for a few seconds instead of being retried on
//! every request, then tried again, so a drive plugged back in shows its
//! photos; a photo whose file is missing still shows the last preview
//! written for its current version. A preview served from memory has its
//! source file looked at again now and then, on a worker (see `recheck`):
//! a file replaced since gets a new preview, and so does one that was away
//! when the preview was loaded and has come back changed.

mod disk;
mod job;
mod recheck;
pub(crate) mod render;
mod types;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use photo_catalog::CatalogError;
use photo_core::meta::{FileKind, PhotoId};
use photo_imaging::Histogram;

pub use types::{LoadState, Preview, PreviewOrigin, PreviewSize, PreviewState};

use crate::engine::{Engine, JobFailure};
use crate::jobs::{JobKey, Lane};
use crate::lru::Lru;
use job::{JobResult, PreviewJob, SourceSeen};
use recheck::{Look, Watch};

/// Bytes of thumbnail pixels kept in memory.
const THUMBNAIL_MEMORY: usize = 192 << 20;
/// Bytes of standard-preview pixels kept in memory.
const STANDARD_MEMORY: usize = 256 << 20;
/// Bytes of import-candidate thumbnails kept in memory.
const CANDIDATE_MEMORY: usize = 64 << 20;
/// How long a failure is believed before the preview is tried again.
const RETRY_FAILED_AFTER: Duration = Duration::from_secs(5);
/// How long a preview in memory is trusted before its source file is looked
/// at again.
const RECHECK_AFTER: Duration = Duration::from_secs(2);
/// How many source stamps are kept before those of previews no longer in
/// memory are dropped.
const STAMPS_KEPT: usize = 1 << 16;

/// The memory side of the preview service.
pub(crate) struct Previews {
    thumbnails: Lru<PhotoId, Preview>,
    standard: Lru<PhotoId, Preview>,
    /// The develop version each preview being made is for.
    in_flight: HashMap<(PhotoId, PreviewSize), u64>,
    /// The develop version a preview failed for, why and when.
    failed: HashMap<(PhotoId, PreviewSize), (u64, String, Instant)>,
    /// What the source file of each preview in memory looked like when the
    /// preview was made, and when it was last looked at.
    stamps: HashMap<(PhotoId, PreviewSize), Watch>,
    /// The source files to look at next: at most one entry per preview.
    due: Vec<Look>,
    /// A worker is out looking, or about to: one at a time.
    looking: bool,
    /// How long a failure is believed.
    pub(crate) retry_after: Duration,
    /// How long a preview in memory is trusted.
    pub(crate) recheck_after: Duration,
    candidates: Lru<PathBuf, Preview>,
    candidates_in_flight: HashSet<PathBuf>,
    candidates_failed: HashMap<PathBuf, String>,
}

impl Default for Previews {
    fn default() -> Self {
        Self {
            thumbnails: Lru::new(THUMBNAIL_MEMORY),
            standard: Lru::new(STANDARD_MEMORY),
            in_flight: HashMap::new(),
            failed: HashMap::new(),
            stamps: HashMap::new(),
            due: Vec::new(),
            looking: false,
            retry_after: RETRY_FAILED_AFTER,
            recheck_after: RECHECK_AFTER,
            candidates: Lru::new(CANDIDATE_MEMORY),
            candidates_in_flight: HashSet::new(),
            candidates_failed: HashMap::new(),
        }
    }
}

impl Previews {
    fn memory(&mut self, size: PreviewSize) -> &mut Lru<PhotoId, Preview> {
        match size {
            PreviewSize::Thumbnail => &mut self.thumbnails,
            PreviewSize::Standard => &mut self.standard,
        }
    }

    fn peek(&self, photo: PhotoId, size: PreviewSize) -> Option<&Preview> {
        match size {
            PreviewSize::Thumbnail => self.thumbnails.peek(&photo),
            PreviewSize::Standard => self.standard.peek(&photo),
        }
    }

    /// How many previews (of photos and of import candidates) are being
    /// made.
    pub(crate) fn in_flight(&self) -> usize {
        self.in_flight.len() + self.candidates_in_flight.len()
    }

    /// How many source files are waiting for the next look.
    #[cfg(test)]
    pub(crate) fn looks_waiting(&self) -> usize {
        self.due.len()
    }
}

/// What a request needs to know about a photo.
struct Subject {
    version: u64,
    path: PathBuf,
    kind: FileKind,
}

impl Engine {
    /// The thumbnail of a photo; see [`Engine::preview`].
    pub fn thumbnail(&mut self, photo: PhotoId) -> PreviewState {
        self.preview(photo, PreviewSize::Thumbnail)
    }

    /// The standard preview of a photo; see [`Engine::preview`].
    pub fn standard_preview(&mut self, photo: PhotoId) -> PreviewState {
        self.preview(photo, PreviewSize::Standard)
    }

    /// The histogram of a photo's standard preview, once that preview is
    /// ready (asking for it if it is not).
    pub fn histogram(&mut self, photo: PhotoId) -> Option<Arc<Histogram>> {
        match self.standard_preview(photo) {
            PreviewState::Ready(preview) => preview.histogram().cloned(),
            PreviewState::Pending | PreviewState::Failed(_) => None,
        }
    }

    /// A preview of `photo` at `size` for its current develop version.
    ///
    /// Answers at once from memory; otherwise queues a job (or moves the
    /// queued one to the front) and answers [`PreviewState::Pending`]. Call
    /// it every frame for what is on screen; it is cheap.
    pub fn preview(&mut self, photo: PhotoId, size: PreviewSize) -> PreviewState {
        let subject = match self.subject(photo) {
            Ok(subject) => subject,
            Err(error) => return PreviewState::Failed(error.to_string()),
        };
        let key = (photo, size);
        if let Some(preview) = self.previews.memory(size).get(&photo)
            && preview.version() == subject.version
        {
            let ready = preview.clone();
            self.recheck(photo, size, &subject.path);
            return PreviewState::Ready(ready);
        }
        // A failure is believed for a while, then the preview is tried
        // again: the file may be back.
        if let Some((version, reason, at)) = self.previews.failed.get(&key)
            && *version == subject.version
            && at.elapsed() < self.previews.retry_after
        {
            return PreviewState::Failed(reason.clone());
        }
        if self
            .previews
            .in_flight
            .get(&key)
            .is_some_and(|in_flight| *in_flight == subject.version)
        {
            self.promote(&JobKey::Preview(photo, size));
            return PreviewState::Pending;
        }
        let settings = match self.catalog.develop_settings(photo) {
            Ok(settings) => Some(settings),
            Err(CatalogError::NewerFormat { .. }) => None,
            Err(error) => return PreviewState::Failed(error.to_string()),
        };
        let job = PreviewJob {
            root: self.previews_dir.clone(),
            photo,
            size,
            version: subject.version,
            path: subject.path,
            kind: subject.kind,
            settings,
            sources: self.sources.clone(),
            cancel: self.shutdown.clone(),
        };
        let version = subject.version;
        self.submit(
            Lane::Interactive,
            Some(JobKey::Preview(photo, size)),
            None,
            move |context| {
                let interim = |preview: Preview| {
                    context.progress(Box::new(move |engine: &mut Engine| {
                        engine.preview_interim(photo, size, version, preview)
                    }));
                };
                let result = job.run(&interim);
                Box::new(move |engine: &mut Engine| {
                    engine.preview_done(photo, size, version, result)
                })
            },
            move |engine, failure| engine.preview_failed(photo, size, version, failure),
        );
        self.previews.in_flight.insert(key, version);
        PreviewState::Pending
    }

    /// Where the preview of `photo` at `size` stands, without asking for
    /// anything.
    pub fn load_state(&self, photo: PhotoId, size: PreviewSize) -> LoadState {
        let Some(version) = self
            .browser
            .summary(photo)
            .map(|summary| summary.develop_version)
            .or_else(|| {
                self.catalog
                    .photo(photo)
                    .ok()
                    .map(|record| record.develop_version)
            })
        else {
            return LoadState::Nothing;
        };
        let key = (photo, size);
        if let Some(preview) = self.previews.peek(photo, size)
            && preview.version() == version
        {
            return LoadState::Ready {
                origin: preview.origin(),
            };
        }
        if self
            .previews
            .in_flight
            .get(&key)
            .is_some_and(|in_flight| *in_flight == version)
        {
            return LoadState::Pending;
        }
        match self.previews.failed.get(&key) {
            Some((failed, reason, _)) if *failed == version => LoadState::Failed(reason.clone()),
            _ => LoadState::Nothing,
        }
    }

    /// A thumbnail of a file offered for import, from the decoder's
    /// embedded preview. Kept in memory only.
    pub fn candidate_thumbnail(&mut self, path: &Path) -> PreviewState {
        let path = path.to_path_buf();
        if let Some(preview) = self.previews.candidates.get(&path) {
            return PreviewState::Ready(preview.clone());
        }
        if let Some(reason) = self.previews.candidates_failed.get(&path) {
            return PreviewState::Failed(reason.clone());
        }
        let key = JobKey::Candidate(path.clone());
        if self.previews.candidates_in_flight.contains(&path) {
            self.promote(&key);
            return PreviewState::Pending;
        }
        let (work_path, done_path, failed_path) = (path.clone(), path.clone(), path.clone());
        self.submit(
            Lane::Interactive,
            Some(key),
            None,
            move |_| {
                let result = job::candidate(&work_path);
                Box::new(move |engine: &mut Engine| engine.candidate_done(done_path, result))
            },
            move |engine, failure| {
                engine.previews.candidates_in_flight.remove(&failed_path);
                if let JobFailure::Panicked(message) = failure {
                    engine
                        .previews
                        .candidates_failed
                        .insert(failed_path, message);
                }
                true
            },
        );
        self.previews.candidates_in_flight.insert(path);
        PreviewState::Pending
    }

    /// A RAW photo's embedded preview, shown while its render runs; kept
    /// unless a render of the same version is already there.
    fn preview_interim(
        &mut self,
        photo: PhotoId,
        size: PreviewSize,
        version: u64,
        preview: Preview,
    ) -> bool {
        let rendered = self.previews.peek(photo, size).is_some_and(|kept| {
            kept.version() > version
                || (kept.version() == version && kept.origin() == PreviewOrigin::Rendered)
        });
        if rendered {
            return false;
        }
        let bytes = preview.bytes();
        self.previews.memory(size).insert(photo, preview, bytes);
        true
    }

    fn subject(&self, photo: PhotoId) -> Result<Subject, CatalogError> {
        if let Some(summary) = self.browser.summary(photo) {
            return Ok(Subject {
                version: summary.develop_version,
                path: summary.path.clone(),
                kind: summary.kind,
            });
        }
        let record = self.catalog.photo(photo)?;
        Ok(Subject {
            version: record.develop_version,
            path: record.path,
            kind: record.kind,
        })
    }

    fn finish_in_flight(&mut self, photo: PhotoId, size: PreviewSize, version: u64) {
        let key = (photo, size);
        if self
            .previews
            .in_flight
            .get(&key)
            .is_some_and(|in_flight| *in_flight == version)
        {
            self.previews.in_flight.remove(&key);
        }
    }

    fn preview_done(
        &mut self,
        photo: PhotoId,
        size: PreviewSize,
        version: u64,
        result: JobResult,
    ) -> bool {
        self.finish_in_flight(photo, size, version);
        match result.preview {
            Ok(preview) => {
                let newer_kept = self
                    .previews
                    .peek(photo, size)
                    .is_some_and(|kept| kept.version() > version);
                if !newer_kept {
                    let bytes = preview.bytes();
                    self.previews.memory(size).insert(photo, preview, bytes);
                    self.remember_stamp(photo, size, result.stamp);
                }
                self.previews.failed.remove(&(photo, size));
            }
            Err(reason) => {
                self.previews
                    .failed
                    .insert((photo, size), (version, reason, Instant::now()));
            }
        }
        let recorded = self.browser.summary(photo).map(|summary| summary.missing);
        match result.source {
            SourceSeen::Missing if recorded != Some(true) => self.missing.push((photo, true)),
            SourceSeen::Present if recorded == Some(true) => self.missing.push((photo, false)),
            SourceSeen::Missing | SourceSeen::Present | SourceSeen::Unknown => {}
        }
        true
    }

    fn preview_failed(
        &mut self,
        photo: PhotoId,
        size: PreviewSize,
        version: u64,
        failure: JobFailure,
    ) -> bool {
        self.finish_in_flight(photo, size, version);
        if let JobFailure::Panicked(message) = failure {
            let reason = format!("the preview could not be made: {message}");
            self.previews
                .failed
                .insert((photo, size), (version, reason, Instant::now()));
        }
        true
    }

    fn candidate_done(&mut self, path: PathBuf, result: Result<Preview, String>) -> bool {
        self.previews.candidates_in_flight.remove(&path);
        match result {
            Ok(preview) => {
                let bytes = preview.bytes();
                self.previews.candidates.insert(path, preview, bytes);
            }
            Err(reason) => {
                self.previews.candidates_failed.insert(path, reason);
            }
        }
        true
    }
}
