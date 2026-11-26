//! Looking again at the source file of a preview held in memory.
//!
//! A preview made from a file is right for that file as it was. Requests
//! answered from memory never touch the disk, so a file replaced under its
//! name (re-exported from another program, say) would keep its old preview
//! until the memory was dropped. Instead, when a preview is served from
//! memory and its file has not been looked at for a while, a worker reads
//! the file's size and time, nothing more; if they differ from what the
//! preview was made from, the preview is dropped and the next request makes
//! it again. The request itself is still answered at once.
//!
//! A preview loaded from its cache file while the original was away is
//! watched the same way, against what the cache file says it was made
//! from. While the original stays away the preview is kept. When the
//! original is back as it was, the photo stops counting as missing; when it
//! is back changed, the preview is made again.
//!
//! At most one worker is looking at any time, at everything that is due,
//! one file after the other; what becomes due meanwhile waits for the next
//! look. A drive that stalls therefore holds one worker and one job, however
//! many previews are asked for and however long it stalls.

use std::path::{Path, PathBuf};
use std::time::Instant;

use photo_core::meta::PhotoId;

use super::{PreviewSize, STAMPS_KEPT};
use crate::engine::{Context, Engine};
use crate::jobs::Lane;
use crate::stamp::Stamp;

/// What is known of the source file of a preview held in memory.
pub(super) struct Watch {
    /// What the file looked like when the preview's pixels were made.
    stamp: Stamp,
    /// When it was last looked at.
    looked: Instant,
    /// It is waiting to be looked at, or being looked at: it is not asked
    /// for again meanwhile.
    looking: bool,
}

/// One file for a worker to look at.
pub(super) struct Look {
    photo: PhotoId,
    size: PreviewSize,
    /// What the preview was made from.
    stamp: Stamp,
    path: PathBuf,
}

/// What a worker saw of a file.
enum Seen {
    /// It is what the preview was made from.
    Same,
    /// It is another file than the preview was made from.
    Changed,
    /// It could not be read: gone, or on a drive that is away.
    Away,
}

impl Look {
    fn see(&self) -> Seen {
        match Stamp::read(&self.path) {
            Ok(now) if now == self.stamp => Seen::Same,
            Ok(_) => Seen::Changed,
            Err(_) => Seen::Away,
        }
    }
}

/// Look at each file in turn. A worker told to stop looks at no more: what
/// it has not seen counts as away, which changes nothing.
fn look_at(looks: Vec<Look>, context: &Context<'_>) -> Vec<(Look, Seen)> {
    looks
        .into_iter()
        .map(|look| {
            let seen = if context.should_stop() {
                Seen::Away
            } else {
                look.see()
            };
            (look, seen)
        })
        .collect()
}

impl Engine {
    /// Note what the source file looked like when the pixels of the preview
    /// now in memory were made; with no stamp, forget the old one.
    pub(super) fn remember_stamp(
        &mut self,
        photo: PhotoId,
        size: PreviewSize,
        stamp: Option<Stamp>,
    ) {
        let key = (photo, size);
        let Some(stamp) = stamp else {
            self.previews.stamps.remove(&key);
            return;
        };
        if self.previews.stamps.len() >= STAMPS_KEPT {
            let mut stamps = std::mem::take(&mut self.previews.stamps);
            stamps.retain(|(photo, size), _| self.previews.peek(*photo, *size).is_some());
            self.previews.stamps = stamps;
        }
        // A look that is out for the preview this one replaces is still
        // out, and will find that its preview has gone.
        let looking = self
            .previews
            .stamps
            .get(&key)
            .is_some_and(|watch| watch.looking);
        let watch = Watch {
            stamp,
            looked: Instant::now(),
            looking,
        };
        self.previews.stamps.insert(key, watch);
    }

    /// A preview was served from memory: have its source file looked at
    /// again, unless that was done lately or is about to be.
    pub(super) fn recheck(&mut self, photo: PhotoId, size: PreviewSize, path: &Path) {
        let trusted = self.previews.recheck_after;
        let Some(watch) = self.previews.stamps.get_mut(&(photo, size)) else {
            return;
        };
        if watch.looking || watch.looked.elapsed() < trusted {
            return;
        }
        watch.looking = true;
        self.previews.due.push(Look {
            photo,
            size,
            stamp: watch.stamp,
            path: path.to_path_buf(),
        });
        self.send_look();
    }

    /// Send a worker to look at everything that is due, unless one is out.
    fn send_look(&mut self) {
        if self.previews.looking || self.previews.due.is_empty() {
            return;
        }
        let looks = std::mem::take(&mut self.previews.due);
        let keys: Vec<(PhotoId, PreviewSize)> =
            looks.iter().map(|look| (look.photo, look.size)).collect();
        self.previews.looking = true;
        self.submit(
            Lane::Background,
            None,
            None,
            move |context| {
                let seen = look_at(looks, context);
                Box::new(move |engine: &mut Engine| engine.looked(seen))
            },
            move |engine, _| engine.look_lost(&keys),
        );
    }

    /// What the worker saw. A file that is not what its preview was made
    /// from loses the preview, so that the next request makes it again. A
    /// file that is there as it was is no longer missing, if it was. A file
    /// that is away keeps its last preview.
    fn looked(&mut self, seen: Vec<(Look, Seen)>) -> bool {
        self.previews.looking = false;
        let mut changed = false;
        for (look, seen) in seen {
            let key = (look.photo, look.size);
            let Some(watch) = self.previews.stamps.get_mut(&key) else {
                continue;
            };
            watch.looking = false;
            // Another preview has taken this one's place meanwhile.
            if watch.stamp != look.stamp {
                continue;
            }
            watch.looked = Instant::now();
            match seen {
                Seen::Away => {}
                Seen::Same => {
                    let missing = self
                        .browser
                        .summary(look.photo)
                        .is_some_and(|summary| summary.missing);
                    if missing {
                        self.missing.push((look.photo, false));
                        changed = true;
                    }
                }
                Seen::Changed => {
                    self.previews.stamps.remove(&key);
                    self.previews.memory(look.size).remove(&look.photo);
                    changed = true;
                }
            }
        }
        self.send_look();
        changed
    }

    /// The look never ran, or did not come back: its files are due again.
    fn look_lost(&mut self, keys: &[(PhotoId, PreviewSize)]) -> bool {
        self.previews.looking = false;
        for key in keys {
            if let Some(watch) = self.previews.stamps.get_mut(key) {
                watch.looking = false;
            }
        }
        false
    }
}
