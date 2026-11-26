//! The file-system operations placement is built from, behind a seam: the
//! two moves that cannot replace a file, and a call before every step so
//! that a test can act as another program between any two of them.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use tempfile::TempPath;

use super::held::Held;
use super::moves::hidden_name;

/// The point just before one step of placing a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// A hard link is about to be tried.
    Link,
    /// The file has its new name too; the old one is about to be removed.
    Unlink,
    /// An exclusive rename is about to be tried.
    RenameExclusive,
    /// The name is about to be created exclusively, to be copied into.
    Create,
    /// The name has been created; the bytes are about to be copied.
    Fill,
    /// The bytes are copied and synced; whether the name still leads to
    /// the file created is about to be checked.
    Confirm,
    /// A copy failed; the created name is about to be given back.
    GiveBack,
    /// The name is taken; what the guarded paths lead to is about to be
    /// looked up.
    Look,
    /// What has the name is about to be moved aside.
    SetAside,
    /// What had the name is aside and about to be identified.
    Identify,
    /// A file that was moved aside is about to be put back.
    PutBack,
    /// The export has the name; what was moved aside is about to be
    /// removed.
    Discard,
}

/// The operations whose behaviour differs between file systems, and the
/// seam between steps. The provided methods are the real file system.
pub(crate) trait Disk {
    /// Gives the file at `from` the further name `to`: a hard link. Fails
    /// with [`io::ErrorKind::AlreadyExists`] when anything has that name
    /// and never replaces it; fails some other way where the file system
    /// has no hard links (FAT, exFAT, many network shares).
    fn link(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        fs::hard_link(from, to)
    }

    /// Moves the file at `from` to the name `to` in one step that fails
    /// with [`io::ErrorKind::AlreadyExists`] when anything has that name
    /// and never replaces it; fails some other way where the file system
    /// cannot do that. See [`rename_exclusive`].
    fn rename_exclusive(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        rename_exclusive(from, to)
    }

    /// A hidden name in `folder` that nothing has, for a file in `role`.
    fn hidden_name(&mut self, folder: &Path, role: &str) -> PathBuf {
        hidden_name(folder, role)
    }

    /// Whether `path` leads to the file `held` at this moment. An error
    /// means that it could not be told.
    fn is_at(&mut self, held: &Held, path: &Path) -> io::Result<bool> {
        Ok(held.is_at(path))
    }

    /// Whether the volume has room for a copy. Always, on the real file
    /// system: a copy that does not fit fails by itself there.
    fn room(&mut self) -> io::Result<()> {
        Ok(())
    }

    /// Called just before each step. Does nothing on the real file system.
    fn reached(&mut self, _step: Step) {}
}

/// The file system as it is.
pub(crate) struct RealDisk;

impl Disk for RealDisk {}

/// A rename that fails instead of replacing: `renameat2` with
/// `RENAME_NOREPLACE` on Linux, `renamex_np` with `RENAME_EXCL` on macOS and
/// `MoveFileExW` without its replace flag on Windows, through `tempfile`'s
/// `persist_noclobber`.
///
/// Where the file system cannot do it the call fails: "not supported" on
/// macOS (FAT, exFAT); on Linux `tempfile` then tries a hard link itself and
/// reports that failure. On Windows it works on every file system, and has
/// two side effects on `from` that `tempfile` wants for its own temporary
/// files: its attributes are reset to normal before the move, and it is
/// marked temporary when the move fails.
pub(crate) fn rename_exclusive(from: &Path, to: &Path) -> io::Result<()> {
    let mut path = TempPath::try_from_path(from)?;
    // `tempfile` deletes a temporary path it is left holding. A move that
    // fails must leave the file where it is.
    path.disable_cleanup(true);
    path.persist_noclobber(to).map_err(|failed| failed.error)
}
