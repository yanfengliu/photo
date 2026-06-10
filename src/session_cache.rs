//! Source-file fingerprinting and the in-session full-image cache.

use crate::decode::ImageData;
use crate::loading::BaseImageSource;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(crate) const FULL_IMAGE_SESSION_CACHE_MAX_ENTRIES: usize = 4;
// Keep enough headroom for a single large RAW decode to stay hot across repeat opens.
pub(crate) const FULL_IMAGE_SESSION_CACHE_MAX_BYTES: usize = 1024 * 1024 * 1024;
// Retain a small recent history even when large detail images overflow the byte budget.
pub(crate) const FULL_IMAGE_SESSION_CACHE_MIN_RECENT_ENTRIES: usize = 2;
pub(crate) const SOURCE_FINGERPRINT_BUFFER_BYTES: usize = 64 * 1024;
pub(crate) const FILE_SHARE_READ: u32 = 0x00000001;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SourceFileFingerprint {
    file_size: u64,
    modified: std::time::Duration,
    content_signature: u64,
}

impl SourceFileFingerprint {
    #[cfg(test)]
    pub(crate) fn from_path(path: &Path) -> Option<Self> {
        let mut file = File::open(path).ok()?;
        Self::from_file(&mut file)
    }

    pub(crate) fn from_file(file: &mut File) -> Option<Self> {
        let metadata = file.metadata().ok()?;
        let modified = metadata
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?;
        let content_signature = source_file_signature(file, metadata.len())?;
        Some(Self {
            file_size: metadata.len(),
            modified,
            content_signature,
        })
    }
}

pub(crate) fn open_cache_validation_handle(path: &Path) -> Option<File> {
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(path)
        .ok()
}

pub(crate) fn source_file_signature(file: &mut File, file_size: u64) -> Option<u64> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    file.seek(SeekFrom::Start(0)).ok()?;
    let mut hasher = DefaultHasher::new();
    file_size.hash(&mut hasher);
    let mut buffer = vec![0; SOURCE_FINGERPRINT_BUFFER_BYTES];

    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        buffer[..read].hash(&mut hasher);
    }

    Some(hasher.finish())
}

pub(crate) fn metadata_matches_fingerprint(
    path: &Path,
    fingerprint: SourceFileFingerprint,
) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return true;
    };
    let Ok(modified) = metadata.modified() else {
        return true;
    };
    let Ok(modified) = modified.duration_since(std::time::UNIX_EPOCH) else {
        return true;
    };
    metadata.len() == fingerprint.file_size && modified == fingerprint.modified
}

pub(crate) struct SessionFullImageCacheEntry {
    fingerprint: SourceFileFingerprint,
    image: Arc<ImageData>,
    base_source: BaseImageSource,
    logical_dimensions: (u32, u32),
    bytes: usize,
}

pub(crate) struct SessionFullImageCacheHit {
    pub(crate) image: Arc<ImageData>,
    pub(crate) logical_dimensions: (u32, u32),
    pub(crate) _write_guard: File,
}

pub(crate) struct SessionFullImageCache {
    entries: std::collections::HashMap<PathBuf, SessionFullImageCacheEntry>,
    lru: std::collections::VecDeque<PathBuf>,
    total_bytes: usize,
    max_entries: usize,
    max_bytes: usize,
    min_recent_entries: usize,
}

impl Default for SessionFullImageCache {
    fn default() -> Self {
        Self::new(
            FULL_IMAGE_SESSION_CACHE_MAX_ENTRIES,
            FULL_IMAGE_SESSION_CACHE_MAX_BYTES,
        )
    }
}

impl SessionFullImageCache {
    pub(crate) fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: std::collections::HashMap::new(),
            lru: std::collections::VecDeque::new(),
            total_bytes: 0,
            max_entries,
            max_bytes,
            min_recent_entries: FULL_IMAGE_SESSION_CACHE_MIN_RECENT_ENTRIES.min(max_entries),
        }
    }

    pub(crate) fn get(
        &mut self,
        path: &Path,
        expected_base_source: BaseImageSource,
    ) -> Option<SessionFullImageCacheHit> {
        let (cached_fingerprint, image, base_source, logical_dimensions) =
            match self.entries.get(path) {
                Some(entry) => (
                    entry.fingerprint,
                    entry.image.clone(),
                    entry.base_source,
                    entry.logical_dimensions,
                ),
                None => return None,
            };
        if base_source != expected_base_source {
            self.remove(path);
            return None;
        }

        let mut guard = open_cache_validation_handle(path)?;
        let metadata = guard.metadata().ok()?;
        let modified = metadata
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?;
        if metadata.len() != cached_fingerprint.file_size || modified != cached_fingerprint.modified
        {
            self.remove(path);
            return None;
        }

        let Some(fingerprint) = SourceFileFingerprint::from_file(&mut guard) else {
            self.remove(path);
            return None;
        };
        if fingerprint != cached_fingerprint {
            self.remove(path);
            return None;
        }

        self.touch(path);
        Some(SessionFullImageCacheHit {
            image,
            logical_dimensions,
            _write_guard: guard,
        })
    }

    pub(crate) fn contains_path(&self, path: &Path) -> bool {
        self.entries.contains_key(path)
    }

    pub(crate) fn entry_matches_base_source(
        &self,
        path: &Path,
        expected_base_source: BaseImageSource,
    ) -> bool {
        self.entries
            .get(path)
            .is_some_and(|entry| entry.base_source == expected_base_source)
    }

    pub(crate) fn metadata_matches_path(&self, path: &Path) -> bool {
        self.entries
            .get(path)
            .is_some_and(|entry| metadata_matches_fingerprint(path, entry.fingerprint))
    }

    pub(crate) fn insert(
        &mut self,
        path: &Path,
        fingerprint: SourceFileFingerprint,
        image: Arc<ImageData>,
        base_source: BaseImageSource,
        logical_dimensions: (u32, u32),
    ) {
        self.remove(path);

        let bytes = image.pixels.len();
        let path_buf = path.to_path_buf();

        self.total_bytes = self.total_bytes.saturating_add(bytes);
        self.entries.insert(
            path_buf.clone(),
            SessionFullImageCacheEntry {
                fingerprint,
                image,
                base_source,
                logical_dimensions,
                bytes,
            },
        );
        self.lru.push_back(path_buf);
        self.evict_as_needed();
    }

    pub(crate) fn touch(&mut self, path: &Path) {
        if let Some(position) = self.lru.iter().position(|candidate| candidate == path) {
            self.lru.remove(position);
        }
        self.lru.push_back(path.to_path_buf());
    }

    pub(crate) fn remove(&mut self, path: &Path) {
        if let Some(entry) = self.entries.remove(path) {
            self.total_bytes = self.total_bytes.saturating_sub(entry.bytes);
        }
        if let Some(position) = self.lru.iter().position(|candidate| candidate == path) {
            self.lru.remove(position);
        }
    }

    pub(crate) fn evict_as_needed(&mut self) {
        while self.entries.len() > self.max_entries
            || (self.entries.len() > self.min_recent_entries && self.total_bytes > self.max_bytes)
        {
            let Some(oldest_path) = self.lru.pop_front() else {
                break;
            };
            if let Some(entry) = self.entries.remove(&oldest_path) {
                self.total_bytes = self.total_bytes.saturating_sub(entry.bytes);
            }
        }
    }
}
