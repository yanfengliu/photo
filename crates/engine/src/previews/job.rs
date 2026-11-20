//! The worker side of a preview: check the file on disk against the source,
//! or make the pixels afresh and write them back.
//!
//! The source file's size and modification time are read once, at the start
//! of the job, before any pixels: a cache file is accepted only if it
//! recorded exactly those, and a new file records them. If the source
//! changes after that, the recorded stamp is the older one and the next
//! request renders again; the pixels can never claim a newer file than they
//! came from.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use photo_core::develop::DevelopSettings;
use photo_core::meta::{FileKind, PhotoId};

use super::disk::{self, Header, Stamp};
use super::render::{self, RenderError, RenderSource};
use super::{Preview, PreviewOrigin, PreviewSize};

/// What a worker learned about the source file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SourceSeen {
    /// The file was there.
    Present,
    /// The file was not there.
    Missing,
    /// The file could not be checked.
    Unknown,
}

/// The outcome of a preview job.
pub(crate) struct JobResult {
    pub(crate) preview: Result<Preview, String>,
    pub(crate) source: SourceSeen,
}

/// Everything a worker needs to make one preview, captured on the engine's
/// thread when the job was queued.
pub(crate) struct PreviewJob {
    pub(crate) root: PathBuf,
    pub(crate) photo: PhotoId,
    pub(crate) size: PreviewSize,
    pub(crate) version: u64,
    pub(crate) path: PathBuf,
    pub(crate) kind: FileKind,
    pub(crate) settings: DevelopSettings,
}

impl PreviewJob {
    pub(crate) fn run(self) -> JobResult {
        let file = disk::file_path(&self.root, self.photo, self.size, self.version);
        let stamp = match fs::metadata(&self.path) {
            Ok(metadata) => Stamp::of(&metadata),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return self.offline(&file),
            Err(error) => {
                return JobResult {
                    preview: Err(format!("cannot read {}: {error}", self.path.display())),
                    source: SourceSeen::Unknown,
                };
            }
        };
        if let Some((header, image)) = disk::read(&file)
            && header.version == self.version
            && header.size == self.size
            && header.source == stamp
        {
            return JobResult {
                preview: Ok(Preview::new(
                    image,
                    self.version,
                    self.size,
                    header.origin,
                    header.histogram,
                )),
                source: SourceSeen::Present,
            };
        }
        let source = RenderSource {
            path: &self.path,
            kind: self.kind,
            settings: &self.settings,
        };
        match render::render(&source, self.size.long_edge()) {
            Ok(rendered) => {
                let histogram = (self.size == PreviewSize::Standard)
                    .then(|| photo_imaging::histogram(&rendered.image));
                let header = Header {
                    version: self.version,
                    size: self.size,
                    origin: rendered.origin,
                    source: stamp,
                    histogram,
                };
                // A job for a newer version can finish first; this one's
                // file would then be stale before it was written.
                if !disk::newer_exists(&self.root, self.photo, self.size, self.version) {
                    match disk::write(&file, &header, &rendered.image) {
                        Ok(()) => {
                            disk::remove_older(&self.root, self.photo, self.size, self.version);
                        }
                        Err(error) => {
                            log::warn!("cannot write preview {}: {error}", file.display());
                        }
                    }
                }
                JobResult {
                    preview: Ok(Preview::new(
                        rendered.image,
                        self.version,
                        self.size,
                        header.origin,
                        header.histogram,
                    )),
                    source: SourceSeen::Present,
                }
            }
            Err(RenderError::Missing) => self.offline(&file),
            Err(RenderError::Failed(reason)) => JobResult {
                preview: Err(reason),
                source: SourceSeen::Present,
            },
        }
    }

    /// The source is gone: show the last preview written for this version,
    /// if there is one. There is no file left to check it against.
    fn offline(&self, file: &Path) -> JobResult {
        let preview = match disk::read(file) {
            Some((header, image)) if header.version == self.version && header.size == self.size => {
                Ok(Preview::new(
                    image,
                    self.version,
                    self.size,
                    header.origin,
                    header.histogram,
                ))
            }
            _ => Err(format!("the file is missing: {}", self.path.display())),
        };
        JobResult {
            preview,
            source: SourceSeen::Missing,
        }
    }
}

/// The thumbnail of a file offered for import: the decoder's embedded
/// preview, as the file stores it.
pub(crate) fn candidate(path: &Path) -> Result<Preview, String> {
    match photo_decode::embedded_preview(path, PreviewSize::Thumbnail.long_edge()) {
        Ok(Some(image)) => Ok(Preview::new(
            image,
            0,
            PreviewSize::Thumbnail,
            PreviewOrigin::EmbeddedPreview,
            None,
        )),
        Ok(None) => Err("the file has no embedded preview".to_string()),
        Err(error) => Err(error.to_string()),
    }
}
