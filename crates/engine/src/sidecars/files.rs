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

/// The saves of one sidecar: the number of the newest one asked for, and
/// the lock a save holds while it writes.
#[derive(Default)]
struct Turn {
    newest: AtomicU64,
    writing: Mutex<()>,
}

/// Puts the saves of each sidecar in order. Two saves of one sidecar never
/// write at once, and a save that a newer one has overtaken writes nothing,
/// so the file ends with what the last save asked for held, whichever
/// worker finishes first.
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
        let number = turn.newest.fetch_add(1, Ordering::SeqCst) + 1;
        Ticket { turn, number }
    }

    /// Forget the sidecars no save holds a place for.
    pub(super) fn forget_idle(&self) {
        lock(&self.0).retain(|_, turn| Arc::strong_count(turn) > 1);
    }
}

impl Ticket {
    /// Run `write`, unless a newer save of the same sidecar has been asked
    /// for: then nothing is written and `None` returned.
    pub(super) fn write<T>(&self, write: impl FnOnce() -> T) -> Option<T> {
        let _writing = lock(&self.turn.writing);
        (self.turn.newest.load(Ordering::SeqCst) == self.number).then(write)
    }
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
    use super::*;

    #[test]
    fn an_older_save_of_a_sidecar_never_lands_after_a_newer_one() {
        let order = SaveOrder::default();
        let sidecar = Path::new("/photos/a.xmp");
        let (older, newer) = (order.claim(sidecar), order.claim(sidecar));
        let other = order.claim(Path::new("/photos/b.xmp"));
        // The newer save's worker happens to finish first.
        assert_eq!(newer.write(|| "newer"), Some("newer"));
        assert_eq!(
            older.write(|| "older"),
            None,
            "overtaken: nothing is written"
        );
        assert_eq!(other.write(|| "other"), Some("other"), "another sidecar");
        // In the order asked for, both write, the newer one last.
        let (first, second) = (order.claim(sidecar), order.claim(sidecar));
        assert_eq!(first.write(|| 1), None, "the second was already asked for");
        assert_eq!(second.write(|| 2), Some(2));

        drop((older, newer, other, first, second));
        order.forget_idle();
        assert!(lock(&order.0).is_empty());
    }
}
