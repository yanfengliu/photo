//! A file kept open so that it can be recognised later.
//!
//! Files are told apart by identity: device and inode on Unix, volume
//! serial number and file index on Windows. On most file systems that
//! number belongs to a file for as long as the file exists. On FAT and
//! exFAT it does not: it follows where the file's first bytes or its
//! directory entry are. Measured on FAT32 and exFAT under macOS, it changes
//! when an empty file gets its first bytes, when an empty file is renamed
//! and (exFAT) when a file is cut to nothing and written again; for
//! Windows the documentation says that renaming a file on FAT can change
//! it.
//!
//! An identity noted at one moment therefore proves nothing at a later
//! one. What is kept is the open file, and the identity is read afresh
//! from it, and from the file it is compared with, at the moment of every
//! comparison: the open file and its path report the same new number as
//! soon as it changes.

use std::fs::File;
use std::io;
use std::path::Path;

use same_file::Handle;

/// A file kept open. Anyone can still read, move, rename or remove it
/// meanwhile (on Windows it is opened shared for all three).
pub(crate) struct Held(Handle);

impl Held {
    /// Opens what `path` leads to, a file or a folder; a symbolic link is
    /// followed.
    pub(crate) fn open(path: &Path) -> io::Result<Self> {
        Handle::from_path(path).map(Self)
    }

    /// Keeps a file that is already open; it stays usable through
    /// [`Self::file`].
    pub(crate) fn keep(file: File) -> io::Result<Self> {
        Handle::from_file(file).map(Self)
    }

    /// The open file.
    pub(crate) fn file(&mut self) -> &mut File {
        self.0.as_file_mut()
    }

    /// True when the two are one file at this moment. An error means an
    /// identity could not be read, so nothing is known either way.
    pub(crate) fn is(&self, other: &Self) -> io::Result<bool> {
        Ok(self.now()? == other.now()?)
    }

    /// True when `path` can be shown to lead to this file at this moment;
    /// false when it leads elsewhere, nowhere, or cannot be read.
    pub(crate) fn is_at(&self, path: &Path) -> bool {
        Self::open(path).is_ok_and(|there| self.is(&there).unwrap_or(false))
    }

    /// The identity the file has now, read from the open file.
    fn now(&self) -> io::Result<Handle> {
        Handle::from_file(self.0.as_file().try_clone()?)
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{self, OpenOptions};
    use std::io::Write;

    use super::*;

    /// On FAT and exFAT each of these steps gives the file a new number;
    /// everywhere it stays the file that was created.
    #[test]
    fn a_held_file_is_recognised_through_its_first_bytes_and_a_rename() {
        let dir = tempfile::tempdir().unwrap();
        let (first, second) = (dir.path().join("first"), dir.path().join("second"));
        let created = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&first)
            .unwrap();
        let mut held = Held::keep(created).unwrap();
        assert!(held.is_at(&first));
        fs::rename(&first, &second).unwrap();
        assert!(held.is_at(&second), "after a rename while empty");
        held.file().write_all(b"first bytes").unwrap();
        assert!(held.is_at(&second), "after its first bytes");
        fs::rename(&second, &first).unwrap();
        assert!(held.is_at(&first), "after a rename");
        let other = dir.path().join("other");
        fs::write(&other, b"first bytes").unwrap();
        assert!(!held.is_at(&other) && !held.is_at(&second));
        assert!(!held.is(&Held::open(&other).unwrap()).unwrap());
        assert!(held.is(&Held::open(&first).unwrap()).unwrap());
    }
}
