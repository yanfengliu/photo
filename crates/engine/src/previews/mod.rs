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
//!    older versions' files deleted.
//!
//! Requests are served newest first, and asking again for a preview that is
//! still queued moves it to the front, so scrolling stays responsive. A
//! source file that has gone is recorded as missing in the catalog, and the
//! failure is remembered for that develop version instead of being retried
//! on every request; a photo whose file is missing still shows the last
//! preview written for its current version.

mod disk;
mod job;
mod lru;
mod render;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use photo_core::image::Rgba8Image;
use photo_core::meta::{FileKind, PhotoId};
use photo_imaging::Histogram;

use crate::engine::{Engine, JobFailure};
use crate::jobs::{JobKey, Lane};
use job::{PreviewJob, SourceSeen};
use lru::Lru;

/// Bytes of thumbnail pixels kept in memory.
const THUMBNAIL_MEMORY: usize = 192 << 20;
/// Bytes of standard-preview pixels kept in memory.
const STANDARD_MEMORY: usize = 256 << 20;
/// Bytes of import-candidate thumbnails kept in memory.
const CANDIDATE_MEMORY: usize = 64 << 20;

/// The two preview sizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PreviewSize {
    /// For grid cells and the filmstrip: longer edge at most 384 pixels.
    Thumbnail,
    /// For the loupe and the histogram: longer edge at most 2048 pixels.
    Standard,
}

impl PreviewSize {
    /// Both sizes, smaller first.
    pub const ALL: [Self; 2] = [Self::Thumbnail, Self::Standard];

    /// The most pixels along the longer edge.
    pub const fn long_edge(self) -> u32 {
        match self {
            Self::Thumbnail => 384,
            Self::Standard => 2048,
        }
    }

    pub(crate) const fn file_tag(self) -> &'static str {
        match self {
            Self::Thumbnail => "thumb",
            Self::Standard => "standard",
        }
    }

    pub(crate) const fn jpeg_quality(self) -> u8 {
        match self {
            Self::Thumbnail => 85,
            Self::Standard => 90,
        }
    }

    pub(crate) const fn code(self) -> u8 {
        match self {
            Self::Thumbnail => 0,
            Self::Standard => 1,
        }
    }

    pub(crate) const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Thumbnail),
            1 => Some(Self::Standard),
            _ => None,
        }
    }
}

/// Where a preview's pixels came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PreviewOrigin {
    /// The decoder's fast preview: a RAW file's embedded camera JPEG, or a
    /// raster file scaled down. Only the photo's orientation is applied.
    EmbeddedPreview,
    /// A render of the photo's develop settings.
    Rendered,
}

impl PreviewOrigin {
    pub(crate) const fn code(self) -> u8 {
        match self {
            Self::EmbeddedPreview => 1,
            Self::Rendered => 2,
        }
    }

    pub(crate) const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::EmbeddedPreview),
            2 => Some(Self::Rendered),
            _ => None,
        }
    }
}

/// Ready preview pixels. Cloning is cheap: the pixels are shared.
#[derive(Clone, Debug)]
pub struct Preview {
    image: Arc<Rgba8Image>,
    version: u64,
    size: PreviewSize,
    origin: PreviewOrigin,
    histogram: Option<Arc<Histogram>>,
}

impl Preview {
    fn new(
        image: Rgba8Image,
        version: u64,
        size: PreviewSize,
        origin: PreviewOrigin,
        histogram: Option<Histogram>,
    ) -> Self {
        Self {
            image: Arc::new(image),
            version,
            size,
            origin,
            histogram: histogram.map(Arc::new),
        }
    }

    /// The pixels: opaque sRGB RGBA8, upright, longer edge at most the
    /// size's.
    pub fn image(&self) -> &Arc<Rgba8Image> {
        &self.image
    }

    /// The develop version these pixels show (0 for an import candidate).
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Which size this is.
    pub fn size(&self) -> PreviewSize {
        self.size
    }

    /// Where the pixels came from.
    pub fn origin(&self) -> PreviewOrigin {
        self.origin
    }

    /// For a standard preview, the histogram of exactly these pixels.
    pub fn histogram(&self) -> Option<&Arc<Histogram>> {
        self.histogram.as_ref()
    }

    fn bytes(&self) -> usize {
        self.image.pixels.len() + self.histogram.as_ref().map_or(0, |_| 4 * 256 * 4)
    }
}

/// The answer to a preview request.
#[derive(Clone, Debug)]
pub enum PreviewState {
    /// The pixels for the photo's current develop version.
    Ready(Preview),
    /// A job is queued or running for it (possibly queued by this call);
    /// poll and ask again.
    Pending,
    /// It cannot be made, for this reason. Not retried for the same develop
    /// version.
    Failed(String),
}

impl PreviewState {
    /// The preview, when ready.
    pub fn ready(&self) -> Option<&Preview> {
        match self {
            Self::Ready(preview) => Some(preview),
            Self::Pending | Self::Failed(_) => None,
        }
    }
}

/// How far the pixels for one photo at one size have got, for the photo's
/// current develop version. The UI shows what is there and never has to
/// infer it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadState {
    /// Nothing has been asked for, or what was there has been evicted.
    Nothing,
    /// A job is queued or running.
    Pending,
    /// Pixels are in memory.
    Ready {
        /// Where they came from: the embedded preview, or a render.
        origin: PreviewOrigin,
    },
    /// Making them failed, for this reason.
    Failed(String),
}

/// The memory side of the preview service.
pub(crate) struct Previews {
    thumbnails: Lru<PhotoId, Preview>,
    standard: Lru<PhotoId, Preview>,
    /// The develop version each preview being made is for.
    in_flight: HashMap<(PhotoId, PreviewSize), u64>,
    failed: HashMap<(PhotoId, PreviewSize), (u64, String)>,
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
            PreviewState::Ready(preview) => preview.histogram,
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
            && preview.version == subject.version
        {
            return PreviewState::Ready(preview.clone());
        }
        if let Some((version, reason)) = self.previews.failed.get(&key)
            && *version == subject.version
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
            Ok(settings) => settings,
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
        };
        let version = subject.version;
        self.submit(
            Lane::Interactive,
            Some(JobKey::Preview(photo, size)),
            None,
            move |_| {
                let result = job.run();
                Box::new(move |engine: &mut Engine| {
                    engine.preview_done(photo, size, version, result.preview, result.source)
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
            .or_else(|| self.catalog.photo(photo).ok().map(|record| record.develop_version))
        else {
            return LoadState::Nothing;
        };
        let key = (photo, size);
        if let Some(preview) = self.previews.peek(photo, size)
            && preview.version == version
        {
            return LoadState::Ready {
                origin: preview.origin,
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
            Some((failed, reason)) if *failed == version => LoadState::Failed(reason.clone()),
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
                    engine.previews.candidates_failed.insert(failed_path, message);
                }
                true
            },
        );
        self.previews.candidates_in_flight.insert(path);
        PreviewState::Pending
    }

    fn subject(&self, photo: PhotoId) -> Result<Subject, photo_catalog::CatalogError> {
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
        preview: Result<Preview, String>,
        source: SourceSeen,
    ) -> bool {
        self.finish_in_flight(photo, size, version);
        match preview {
            Ok(preview) => {
                let newer_kept = self
                    .previews
                    .peek(photo, size)
                    .is_some_and(|kept| kept.version > version);
                if !newer_kept {
                    let bytes = preview.bytes();
                    self.previews.memory(size).insert(photo, preview, bytes);
                }
                self.previews.failed.remove(&(photo, size));
            }
            Err(reason) => {
                self.previews.failed.insert((photo, size), (version, reason));
            }
        }
        let recorded = self.browser.summary(photo).map(|summary| summary.missing);
        match source {
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
            self.previews.failed.insert((photo, size), (version, reason));
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
