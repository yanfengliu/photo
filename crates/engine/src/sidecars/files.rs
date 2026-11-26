//! Reading and writing one sidecar, as a worker does it, and how what went
//! wrong is told to the user.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use photo_core::meta::PhotoId;
use photo_xmp::{ImageInfo, XmpData, XmpError};

use crate::import::FileIssue;
use crate::jobs::lock;

/// The saves of one sidecar: how many have been asked for, and the number
/// of the newest one that has written, which a save holds locked while it
/// writes.
#[derive(Default)]
struct Turn {
    asked: AtomicU64,
    written: Mutex<u64>,
}

/// Puts the saves of each sidecar in order. Two saves of one sidecar never
/// write at once, and a save writes nothing once a newer one has written,
/// so the file ends with what the newest save that ran asked for,
/// whichever worker finishes first. A newer save that is still waiting,
/// was cancelled before it ran, or failed stops no older one.
#[derive(Clone, Default)]
pub(super) struct SaveOrder(Arc<Mutex<HashMap<PathBuf, Arc<Turn>>>>);

/// One save's place among the saves of its sidecar.
pub(super) struct Ticket {
    turn: Arc<Turn>,
    number: u64,
}

impl SaveOrder {
    /// Take the next place for the sidecar at `path`. Called on the
    /// engine's thread, in the order the saves are asked for.
    pub(super) fn claim(&self, path: &Path) -> Ticket {
        let turn = Arc::clone(lock(&self.0).entry(path.to_path_buf()).or_default());
        let number = turn.asked.fetch_add(1, Ordering::SeqCst) + 1;
        Ticket { turn, number }
    }

    /// Forget the sidecars no save holds a place for.
    pub(super) fn forget_idle(&self) {
        lock(&self.0).retain(|_, turn| Arc::strong_count(turn) > 1);
    }
}

impl Ticket {
    /// Take this save's turn at the sidecar: run `write`, unless a newer
    /// save of the same sidecar has already written, in which case nothing
    /// is written and `None` returned. The other saves of the sidecar wait
    /// while `write` runs. A write that fails leaves the file as it was, so
    /// it does not count as written and an older save may still write.
    fn write(&self, write: impl FnOnce() -> Outcome) -> Option<Outcome> {
        let mut written = lock(&self.turn.written);
        if *written > self.number {
            return None;
        }
        let outcome = write();
        if !outcome.failed {
            *written = self.number;
        }
        Some(outcome)
    }
}

/// What a save job does with its place in the order: write the sidecar,
/// unless a newer save of the same sidecar has already written, in which
/// case this one writes nothing (the file holds the newer values).
pub(super) fn save_one(
    ticket: &Ticket,
    path: &Path,
    data: &XmpData,
    info: ImageInfo,
    note: Option<FileIssue>,
) -> Outcome {
    ticket
        .write(|| write_one(path, data, info, note))
        .unwrap_or_else(Outcome::nothing)
}

/// What a worker did with one sidecar.
pub(super) struct Outcome {
    pub(super) found: Option<(PhotoId, XmpData)>,
    /// Something to tell the user about this photo, whether or not the
    /// rest of it worked.
    pub(super) issue: Option<FileIssue>,
    pub(super) failed: bool,
}

impl Outcome {
    /// Nothing done and nothing to say: a save a newer one overtook.
    pub(super) fn nothing() -> Self {
        Self {
            found: None,
            issue: None,
            failed: false,
        }
    }
}

/// An issue about the sidecar at `path`.
pub(super) fn issue(path: &Path, message: impl Into<String>) -> Option<FileIssue> {
    Some(FileIssue {
        path: path.to_path_buf(),
        message: message.into(),
    })
}

/// Write one sidecar. Develop settings that would replace settings this
/// build cannot read are left out, and the rest is written.
pub(super) fn write_one(
    path: &Path,
    data: &XmpData,
    info: ImageInfo,
    note: Option<FileIssue>,
) -> Outcome {
    let failed = |error: XmpError| Outcome {
        found: None,
        issue: issue(path, describe(&error)),
        failed: true,
    };
    match photo_xmp::write_sidecar(path, data, info) {
        Ok(()) => Outcome {
            found: None,
            issue: note,
            failed: false,
        },
        Err(XmpError::UnreadableSettings { .. }) => {
            let without = XmpData {
                develop: None,
                ..data.clone()
            };
            match photo_xmp::write_sidecar(path, &without, info) {
                Ok(()) => Outcome {
                    found: None,
                    issue: issue(
                        path,
                        "the develop settings were not written: the sidecar holds settings this version cannot read",
                    ),
                    failed: false,
                },
                Err(error) => failed(error),
            }
        }
        Err(error) => failed(error),
    }
}

/// Read one sidecar.
pub(super) fn read_one(path: &Path, photo: PhotoId, info: ImageInfo) -> Outcome {
    match photo_xmp::read_sidecar(path, info) {
        Ok(Some(data)) => {
            let note = data.unreadable_settings_format.and_then(|format| {
                issue(
                    path,
                    format!("the develop settings (format {format}) cannot be read by this version and were not applied"),
                )
            });
            Outcome {
                found: Some((photo, data)),
                issue: note,
                failed: false,
            }
        }
        Ok(None) => Outcome {
            found: None,
            issue: None,
            failed: false,
        },
        Err(error) => Outcome {
            found: None,
            issue: issue(path, describe(&error)),
            failed: true,
        },
    }
}

fn describe(error: &XmpError) -> String {
    match error {
        XmpError::Conflict { .. } => {
            "the sidecar kept changing while it was being updated; it was left as the other application saved it".to_string()
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use photo_core::meta::{FileKind, Rating};

    use super::*;

    fn info() -> ImageInfo {
        ImageInfo::new(FileKind::Raster).with_size(96, 64)
    }

    fn rated(stars: u8) -> XmpData {
        XmpData {
            rating: Rating::new(stars),
            ..XmpData::default()
        }
    }

    /// The rating the sidecar at `path` holds, if there is a sidecar.
    fn rating_at(path: &Path) -> Option<Rating> {
        photo_xmp::read_sidecar(path, info())
            .unwrap()
            .map(|data| data.rating)
    }

    #[test]
    fn a_newer_save_that_finishes_first_is_not_overwritten_by_the_older_one() {
        let dir = tempfile::tempdir().unwrap();
        let sidecar = dir.path().join("a.xmp");
        let order = SaveOrder::default();
        // Two saves of one photo, asked for in this order on the engine's
        // thread...
        let (older, newer) = (order.claim(&sidecar), order.claim(&sidecar));
        // ...run as independent jobs, and the newer one's worker is first.
        let first = save_one(&newer, &sidecar, &rated(5), info(), None);
        assert!(!first.failed && first.issue.is_none());
        let late = save_one(&older, &sidecar, &rated(1), info(), None);
        assert!(
            !late.failed && late.issue.is_none(),
            "overtaken, not failed"
        );
        assert_eq!(
            rating_at(&sidecar),
            Some(Rating::new(5)),
            "the last save asked for"
        );
    }

    #[test]
    fn an_older_save_still_writes_when_the_newer_one_never_ran() {
        let dir = tempfile::tempdir().unwrap();
        let sidecar = dir.path().join("a.xmp");
        let order = SaveOrder::default();
        let (older, newer) = (order.claim(&sidecar), order.claim(&sidecar));
        // The newer save was cancelled while it waited for a worker: the
        // pool drops such a job without running it.
        drop(newer);
        let outcome = save_one(&older, &sidecar, &rated(1), info(), None);
        assert!(!outcome.failed && outcome.issue.is_none());
        assert_eq!(
            rating_at(&sidecar),
            Some(Rating::new(1)),
            "the save that was not cancelled reached the file"
        );
    }

    #[test]
    fn saves_that_run_in_the_order_asked_for_all_write() {
        let dir = tempfile::tempdir().unwrap();
        let (sidecar, other) = (dir.path().join("a.xmp"), dir.path().join("b.xmp"));
        let order = SaveOrder::default();
        let (first, second) = (order.claim(&sidecar), order.claim(&sidecar));
        let elsewhere = order.claim(&other);
        save_one(&first, &sidecar, &rated(1), info(), None);
        assert_eq!(
            rating_at(&sidecar),
            Some(Rating::new(1)),
            "a newer save that is only waiting stops nothing"
        );
        save_one(&second, &sidecar, &rated(5), info(), None);
        assert_eq!(rating_at(&sidecar), Some(Rating::new(5)));
        save_one(&elsewhere, &other, &rated(3), info(), None);
        assert_eq!(rating_at(&other), Some(Rating::new(3)), "another sidecar");

        drop((first, second, elsewhere));
        order.forget_idle();
        assert!(lock(&order.0).is_empty());
    }

    #[test]
    fn an_older_save_still_writes_when_the_newer_one_failed() {
        let dir = tempfile::tempdir().unwrap();
        let sidecar = dir.path().join("a.xmp");
        let order = SaveOrder::default();
        let (older, newer) = (order.claim(&sidecar), order.claim(&sidecar));
        // Something is in the way when the newer save's turn comes.
        std::fs::create_dir(&sidecar).unwrap();
        let newer = save_one(&newer, &sidecar, &rated(5), info(), None);
        assert!(newer.failed && newer.issue.is_some());
        std::fs::remove_dir(&sidecar).unwrap();
        let older = save_one(&older, &sidecar, &rated(1), info(), None);
        assert!(!older.failed && older.issue.is_none());
        assert_eq!(
            rating_at(&sidecar),
            Some(Rating::new(1)),
            "nothing newer is in the file, so the older save is not held back"
        );
    }

    #[test]
    fn the_other_saves_of_a_sidecar_wait_while_one_writes() {
        use std::sync::TryLockError;

        let dir = tempfile::tempdir().unwrap();
        let (sidecar, other) = (dir.path().join("a.xmp"), dir.path().join("b.xmp"));
        let order = SaveOrder::default();
        let (older, newer) = (order.claim(&sidecar), order.claim(&sidecar));
        let elsewhere = order.claim(&other);
        // What a worker bringing this save would meet if it came now.
        let waits = |ticket: &Ticket| {
            matches!(
                ticket.turn.written.try_lock(),
                Err(TryLockError::WouldBlock)
            )
        };
        let outcome = older.write(|| {
            assert!(waits(&newer), "a save of the same sidecar waits");
            assert!(!waits(&elsewhere), "a save of another sidecar does not");
            write_one(&sidecar, &rated(1), info(), None)
        });
        assert!(outcome.is_some_and(|outcome| !outcome.failed));
        assert!(!waits(&newer), "and goes on once the write is over");
    }
}
