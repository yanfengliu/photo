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
//! soon as it changes. Reading an identity can fail (when no descriptor is
//! left to read it with, say); that is an error, never an answer.

use std::fs::File;
use std::io;
use std::path::Path;

use same_file::Handle;

/// A file kept open. Anyone can still read, move, rename or remove it
/// meanwhile (on Windows it is opened shared for all three).
pub(crate) struct Held(File);

impl Held {
    /// Opens what `path` leads to, a file or a folder; a symbolic link is
    /// followed.
    pub(crate) fn open(path: &Path) -> io::Result<Self> {
        // `same_file` knows how to open a folder on Windows as well.
        Handle::from_path(path)?.as_file().try_clone().map(Self)
    }

    /// Keeps a file that is already open; it stays usable through
    /// [`Self::file`]. Nothing is read from it yet, so this cannot fail.
    pub(crate) fn keep(file: File) -> Self {
        Self(file)
    }

    /// The open file.
    pub(crate) fn file(&mut self) -> &mut File {
        &mut self.0
    }

    /// True when the two are one file at this moment. An error means an
    /// identity could not be read, so nothing is known either way.
    pub(crate) fn is(&self, other: &Self) -> io::Result<bool> {
        Ok(self.now()? == other.now()?)
    }

    /// True when `path` leads to this file at this moment; false when it
    /// leads to another or to nothing. An error means it could not be
    /// told: `path` leads to something that cannot be opened, or an
    /// identity could not be read.
    pub(crate) fn is_at(&self, path: &Path) -> io::Result<bool> {
        match Self::open(path) {
            Ok(there) => self.is(&there),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// The identity the file has now, read from the open file.
    fn now(&self) -> io::Result<Handle> {
        Handle::from_file(self.0.try_clone()?)
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
        let mut held = Held::keep(created);
        assert!(held.is_at(&first).unwrap());
        fs::rename(&first, &second).unwrap();
        assert!(held.is_at(&second).unwrap(), "after a rename while empty");
        held.file().write_all(b"first bytes").unwrap();
        assert!(held.is_at(&second).unwrap(), "after its first bytes");
        fs::rename(&second, &first).unwrap();
        assert!(held.is_at(&first).unwrap(), "after a rename");
        let other = dir.path().join("other");
        fs::write(&other, b"first bytes").unwrap();
        assert!(!held.is_at(&other).unwrap() && !held.is_at(&second).unwrap());
        assert!(!held.is(&Held::open(&other).unwrap()).unwrap());
        assert!(held.is(&Held::open(&first).unwrap()).unwrap());
    }

    /// A path that leads to something that cannot be opened says nothing
    /// about whether it is this file: an error, not "another file".
    #[cfg(unix)]
    #[test]
    fn a_path_that_cannot_be_opened_is_not_another_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file");
        fs::write(&path, b"bytes").unwrap();
        let held = Held::open(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::File::open(&path).is_ok() {
            // Whoever runs this (root) is stopped by no permission.
            return;
        }
        let error = held.is_at(&path).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(!held.is_at(&dir.path().join("absent")).unwrap());
    }
}
