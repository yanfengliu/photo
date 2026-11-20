//! Decoded photos, shared by every render: a thumbnail, a standard preview
//! and the develop session of one photo decode it once.
//!
//! A decoded [`Source`] is cached under the file's path and the [`Stamp`]
//! the decoding task read just before the pixels, so a file that changes is
//! a different entry and its old pixels are never served for it. Two tasks
//! that want the same file at once do not both decode it: the second waits
//! for the first. The cache is bounded by the bytes the sources hold.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

use photo_decode::DecodeError;
use photo_develop::Source;

use crate::jobs::lock;
use crate::lru::Lru;
use crate::stamp::Stamp;

/// Bytes of decoded sources kept.
const BUDGET: usize = 768 << 20;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    path: PathBuf,
    stamp: Stamp,
}

struct State {
    cache: Lru<Key, Source>,
    decoding: HashSet<Key>,
}

struct Shared {
    state: Mutex<State>,
    decoded: Condvar,
}

/// The cache; cloning shares it.
#[derive(Clone)]
pub(crate) struct Sources {
    shared: Arc<Shared>,
}

impl Default for Sources {
    fn default() -> Self {
        Self {
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    cache: Lru::new(BUDGET),
                    decoding: HashSet::new(),
                }),
                decoded: Condvar::new(),
            }),
        }
    }
}

impl Sources {
    /// The decoded file at `path`, whose stamp the caller read just now:
    /// from the cache, or decoded here (waiting instead when another task
    /// is already decoding the same file as it is).
    pub(crate) fn decode(&self, path: &Path, stamp: Stamp) -> Result<Source, DecodeError> {
        self.get_or_decode(path, stamp, || {
            photo_decode::decode(path).map(|decoded| Source::new(decoded.image))
        })
    }

    /// The source cached for `path` at `stamp`, without decoding anything:
    /// for work that can decode for itself and should not fill the cache
    /// (an export).
    pub(crate) fn cached(&self, path: &Path, stamp: Stamp) -> Option<Source> {
        let key = Key {
            path: path.to_path_buf(),
            stamp,
        };
        lock(&self.shared.state).cache.peek(&key).cloned()
    }

    /// The source cached for `path` at `stamp`, or what `decode` makes of it.
    fn get_or_decode(
        &self,
        path: &Path,
        stamp: Stamp,
        decode: impl FnOnce() -> Result<Source, DecodeError>,
    ) -> Result<Source, DecodeError> {
        let key = Key {
            path: path.to_path_buf(),
            stamp,
        };
        {
            let mut state = lock(&self.shared.state);
            loop {
                if let Some(source) = state.cache.get(&key) {
                    return Ok(source.clone());
                }
                if state.decoding.insert(key.clone()) {
                    break;
                }
                state = self
                    .shared
                    .decoded
                    .wait(state)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        }
        let claim = Claim {
            shared: &self.shared,
            key,
        };
        let decoded = decode();
        if let Ok(source) = &decoded {
            let cost = bytes(source);
            lock(&self.shared.state)
                .cache
                .insert(claim.key.clone(), source.clone(), cost);
        }
        drop(claim);
        decoded
    }
}

/// A task's claim to decode one file. Dropping it, also when the decode
/// panics, lets the tasks waiting for that file go on.
struct Claim<'a> {
    shared: &'a Shared,
    key: Key,
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        lock(&self.shared.state).decoding.remove(&self.key);
        self.shared.decoded.notify_all();
    }
}

/// What a source holds: 16-bit RGB at full size, a third more for the
/// pyramid above it, and the analysis reduction.
fn bytes(source: &Source) -> usize {
    let (width, height) = source.size();
    let full = width as usize * height as usize * 6;
    full + full / 3 + (2 << 20)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use photo_core::colorimetry::RgbSpace;
    use photo_core::image::{SourceColor, SourceImage};

    use super::*;

    fn grey(width: u32, height: u32) -> Source {
        let pixels = vec![30_000; width as usize * height as usize * 3];
        let color = SourceColor::Rgb(RgbSpace::Srgb);
        Source::new(SourceImage::new(width, height, pixels, color).unwrap())
    }

    fn stamp(nanos: u32) -> Stamp {
        Stamp {
            bytes: 10,
            modified_secs: 5,
            modified_nanos: nanos,
        }
    }

    #[test]
    fn a_file_is_decoded_once_until_its_stamp_changes() {
        let sources = Sources::default();
        let decodes = AtomicUsize::new(0);
        let decode = || {
            decodes.fetch_add(1, Ordering::SeqCst);
            Ok(grey(8, 6))
        };
        let path = Path::new("/photos/a.jpg");
        assert_eq!(
            sources
                .get_or_decode(path, stamp(1), decode)
                .unwrap()
                .size(),
            (8, 6)
        );
        sources.get_or_decode(path, stamp(1), decode).unwrap();
        assert_eq!(decodes.load(Ordering::SeqCst), 1);
        sources.get_or_decode(path, stamp(2), decode).unwrap();
        assert_eq!(
            decodes.load(Ordering::SeqCst),
            2,
            "a changed file is decoded again"
        );
    }

    #[test]
    fn a_task_wanting_a_file_being_decoded_waits_for_it() {
        let sources = Sources::default();
        let decodes = Arc::new(AtomicUsize::new(0));
        let path = Path::new("/photos/b.jpg");
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let first = {
            let (sources, decodes) = (sources.clone(), Arc::clone(&decodes));
            std::thread::spawn(move || {
                sources.get_or_decode(path, stamp(1), || {
                    decodes.fetch_add(1, Ordering::SeqCst);
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(grey(4, 4))
                })
            })
        };
        started_rx.recv().unwrap();
        let second = {
            let (sources, decodes) = (sources.clone(), Arc::clone(&decodes));
            std::thread::spawn(move || {
                sources.get_or_decode(path, stamp(1), || {
                    decodes.fetch_add(1, Ordering::SeqCst);
                    Ok(grey(4, 4))
                })
            })
        };
        release_tx.send(()).unwrap();
        assert!(first.join().unwrap().is_ok());
        assert!(second.join().unwrap().is_ok());
        assert_eq!(decodes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_failed_decode_is_not_cached() {
        let sources = Sources::default();
        let path = Path::new("/photos/c.jpg");
        let failed = sources.get_or_decode(path, stamp(1), || {
            Err(DecodeError::Corrupt("broken".to_string()))
        });
        assert!(failed.is_err());
        assert!(
            sources
                .get_or_decode(path, stamp(1), || Ok(grey(2, 2)))
                .is_ok()
        );
        assert!(
            sources
                .decode(Path::new("/no/such/file.png"), stamp(1))
                .is_err()
        );
    }
}
