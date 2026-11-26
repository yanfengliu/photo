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

use std::path::Path;
use std::time::Instant;

use photo_core::meta::PhotoId;

use super::{PreviewSize, STAMPS_KEPT};
use crate::engine::Engine;
use crate::jobs::Lane;
use crate::stamp::Stamp;

impl Engine {
    /// Note what the source file looked like when the preview now in memory
    /// was made; with no stamp (the file was away), forget the old one.
    pub(super) fn remember_stamp(
        &mut self,
        photo: PhotoId,
        size: PreviewSize,
        stamp: Option<Stamp>,
    ) {
        let Some(stamp) = stamp else {
            self.previews.stamps.remove(&(photo, size));
            return;
        };
        if self.previews.stamps.len() >= STAMPS_KEPT {
            let mut stamps = std::mem::take(&mut self.previews.stamps);
            stamps.retain(|(photo, size), _| self.previews.peek(*photo, *size).is_some());
            self.previews.stamps = stamps;
        }
        self.previews
            .stamps
            .insert((photo, size), (stamp, Instant::now()));
    }

    /// A preview was served from memory: have a worker look at its source
    /// file again, unless that was done lately.
    pub(super) fn recheck(&mut self, photo: PhotoId, size: PreviewSize, path: &Path) {
        let trusted = self.previews.recheck_after;
        let Some((stamp, looked)) = self.previews.stamps.get_mut(&(photo, size)) else {
            return;
        };
        if looked.elapsed() < trusted {
            return;
        }
        *looked = Instant::now();
        let (stamp, path) = (*stamp, path.to_path_buf());
        self.submit(
            Lane::Background,
            None,
            None,
            move |_| {
                let replaced = Stamp::read(&path).is_ok_and(|now| now != stamp);
                Box::new(move |engine: &mut Engine| engine.rechecked(photo, size, stamp, replaced))
            },
            |_, _| false,
        );
    }

    /// The file is not what the preview was made from: drop the preview, so
    /// that the next request makes it again. A file that is away keeps its
    /// last preview.
    fn rechecked(
        &mut self,
        photo: PhotoId,
        size: PreviewSize,
        stamp: Stamp,
        replaced: bool,
    ) -> bool {
        let key = (photo, size);
        let current = self
            .previews
            .stamps
            .get(&key)
            .is_some_and(|(kept, _)| *kept == stamp);
        if !replaced || !current {
            return false;
        }
        self.previews.stamps.remove(&key);
        self.previews.memory(size).remove(&photo);
        true
    }
}
