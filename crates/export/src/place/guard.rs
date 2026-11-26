//! The files an export must never replace: the original photo and the
//! files its caller protects, recognised by file identity rather than by
//! name. Each is kept open ([`Held`]) and its identity read afresh at every
//! comparison, because on FAT and exFAT a file's identity changes over its
//! life.
//!
//! A guarded path is looked up three times, because a path can stop or
//! start leading to its file while an export runs:
//!
//! - when the export starts ([`Guarded::capture`]), which recognises a file
//!   that is moved or renamed afterwards;
//! - just before the overwrite policy moves aside what has the export's
//!   name ([`Guarded::look`]), which recognises a file whose folder was away
//!   at the start and has come back, while its path still leads to it;
//! - after that move ([`Guarded::identify`]), which catches a path that
//!   began to lead somewhere in between.
//!
//! All three compare identities of open files. A guarded path that leads to
//! a file that cannot be opened, at any of the three, is not taken for a
//! path that leads nowhere: nothing then tells that file from the one being
//! compared, so the answer is a [`Doubt`], which refuses the replacement.
//!
//! One case identity cannot settle: a guarded path that led nowhere until
//! the very file it names had been moved aside. That one is settled by
//! name: the file came from the guarded path's own folder (by identity) and
//! file name (ignoring case).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::held::Held;
use super::moves::folder_of;
use crate::item::ExportError;

/// Why a file is guarded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Guard {
    /// It is the original photo.
    Source,
    /// It is one of the item's protected files.
    Protected,
}

impl Guard {
    /// The refusal to replace the guarded file that is now at `path`.
    pub(crate) fn refusal(self, path: PathBuf) -> ExportError {
        match self {
            Self::Source => ExportError::WouldOverwriteSource(path),
            Self::Protected => ExportError::WouldOverwriteProtected(path),
        }
    }
}

/// Why it cannot be told whether a file is a guarded one. Either way
/// nothing may replace the file unverified.
#[derive(Debug)]
pub(crate) enum Doubt {
    /// The identity of the file compared cannot be read.
    File(io::Error),
    /// A guarded path leads to a file, but it cannot be opened to read its
    /// identity.
    Guarded {
        /// The guarded path.
        path: PathBuf,
        /// Why it cannot be opened.
        error: io::Error,
    },
}

impl Doubt {
    /// The error an export fails with, when the file compared is (or was)
    /// at `at`.
    pub(crate) fn error(self, at: &Path) -> ExportError {
        match self {
            Self::File(error) => ExportError::io(at, error),
            Self::Guarded { path, error } => ExportError::CannotIdentify {
                path,
                source: error,
            },
        }
    }
}

/// What a guarded path led to as the export started.
enum AtStart {
    /// This file, kept open.
    File(Held),
    /// No file: the path, or its folder, was not there.
    Nothing,
    /// A file that could not be opened (for want of permission, say).
    Unreadable(io::Error),
}

/// One guarded file: the path it was named by and what that path led to as
/// the export started.
struct Kept {
    guard: Guard,
    path: PathBuf,
    at_start: AtStart,
}

impl Kept {
    /// Whether `file` is the file this path led to at the start, or the
    /// file it leads to now. Doubted when this path led to a file that
    /// could not be opened at the start, or leads to one now.
    fn is(&self, file: &Held) -> Result<bool, Doubt> {
        match &self.at_start {
            AtStart::File(start) => {
                if start.is(file).map_err(Doubt::File)? {
                    return Ok(true);
                }
            }
            AtStart::Nothing => {}
            AtStart::Unreadable(error) => {
                // Where that file has gone since, nothing can say.
                return Err(self.doubt(io::Error::new(error.kind(), error.to_string())));
            }
        }
        match open(&self.path) {
            Ok(Some(now)) => now.is(file).map_err(Doubt::File),
            Ok(None) => Ok(false),
            Err(error) => Err(self.doubt(error)),
        }
    }

    fn doubt(&self, error: io::Error) -> Doubt {
        Doubt::Guarded {
            path: self.path.clone(),
            error,
        }
    }
}

/// Opens what `path` leads to; `None` when it leads nowhere (nothing has
/// the name, its folder is away, or it is a symbolic link that leads
/// nowhere). Any other failure is an error: the file may be there.
fn open(path: &Path) -> io::Result<Option<Held>> {
    match Held::open(path) {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// The original photo and the protected files of one export, as they were
/// when it started. They stay open for the whole export, so that each can
/// be recognised wherever it has been moved; they are opened for reading
/// and shared, so the files can still be read, moved or renamed meanwhile.
pub(crate) struct Guarded {
    files: Vec<Kept>,
}

/// What the guarded paths led to at one moment, kept open so that it can
/// still be recognised after a file has been moved.
pub(crate) struct Sighting(Vec<Option<Held>>);

impl Guarded {
    /// The files `source` and `protected` lead to now. A path that leads to
    /// no file yet (its folder is away, say) is looked at again whenever a
    /// file is compared; one that leads to a file that cannot be opened is
    /// remembered as such, and [`Self::readable`] says so.
    pub(crate) fn capture(source: &Path, protected: &[PathBuf]) -> Self {
        let kept = |guard, path: &Path| Kept {
            guard,
            path: path.to_path_buf(),
            at_start: match open(path) {
                Ok(Some(file)) => AtStart::File(file),
                Ok(None) => AtStart::Nothing,
                Err(error) => AtStart::Unreadable(error),
            },
        };
        let mut files = vec![kept(Guard::Source, source)];
        files.extend(protected.iter().map(|path| kept(Guard::Protected, path)));
        Self { files }
    }

    /// Doubted when a guarded path led to a file that could not be opened
    /// as the export started: then no file can be shown not to be that one,
    /// and the overwrite policy refuses before anything is rendered.
    pub(crate) fn readable(&self) -> Result<(), Doubt> {
        for kept in &self.files {
            if let AtStart::Unreadable(error) = &kept.at_start {
                return Err(kept.doubt(io::Error::new(error.kind(), error.to_string())));
            }
        }
        Ok(())
    }

    /// Which guarded file `path` leads to, if any: the same file under
    /// whatever name it has by now, through a hard link, a symbolic link or
    /// a name that differs only in case. A name that leads to no file is
    /// guarded by nothing. A doubt means that a file is there but an
    /// identity cannot be read, so nothing should replace it unverified.
    pub(crate) fn guards(&self, path: &Path) -> Result<Option<Guard>, Doubt> {
        let Some(file) = open(path).map_err(Doubt::File)? else {
            return Ok(None);
        };
        for kept in &self.files {
            if kept.is(&file)? {
                return Ok(Some(kept.guard));
            }
        }
        Ok(None)
    }

    /// What every guarded path leads to right now. Taken just before the
    /// holder of a name is moved aside, while a guarded path that names it
    /// can still lead to it. Doubted when one leads to a file that cannot
    /// be opened.
    pub(crate) fn look(&self) -> Result<Sighting, Doubt> {
        let seen = |kept: &Kept| open(&kept.path).map_err(|error| kept.doubt(error));
        let seen: Result<Vec<_>, Doubt> = self.files.iter().map(seen).collect();
        seen.map(Sighting)
    }

    /// Which guarded file the thing at `aside` is, having just been moved
    /// there from `name` in one rename, with `seen` looked up before that
    /// move. A symbolic link counts as the file it leads to.
    ///
    /// It is guarded when it is the file a guarded path led to at the
    /// start, at `seen` or now, or when a guarded path names the very
    /// place it was moved from (see the module documentation). A doubt
    /// means it is nothing an export may replace (a folder) or an identity
    /// cannot be read.
    pub(crate) fn identify(
        &self,
        aside: &Path,
        seen: &Sighting,
        name: &Path,
    ) -> Result<Option<Guard>, Doubt> {
        if fs::symlink_metadata(aside).map_err(Doubt::File)?.is_dir() {
            return Err(Doubt::File(io::Error::new(
                io::ErrorKind::IsADirectory,
                "a folder has the name of the export",
            )));
        }
        // `None`: a symbolic link that leads nowhere.
        let file = open(aside).map_err(Doubt::File)?;
        for (kept, seen) in self.files.iter().zip(&seen.0) {
            if let Some(file) = &file {
                if let Some(seen) = seen
                    && seen.is(file).map_err(Doubt::File)?
                {
                    return Ok(Some(kept.guard));
                }
                if kept.is(file)? {
                    return Ok(Some(kept.guard));
                }
            }
            if names_entry(&kept.path, name) {
                return Ok(Some(kept.guard));
            }
        }
        Ok(None)
    }
}

/// True when `path` names the directory entry `name`: the same folder by
/// identity and the same file name, ignoring case as the common file
/// systems do. Deliberately generous: a path that might name the entry is
/// treated as naming it.
fn names_entry(path: &Path, name: &Path) -> bool {
    let lower = |path: &Path| {
        path.file_name()
            .map(|name| name.to_string_lossy().to_lowercase())
    };
    let folder = |path: &Path| Held::open(folder_of(path)).ok();
    // Two folders that are there but cannot be compared might be one.
    lower(path).is_some()
        && lower(path) == lower(name)
        && folder(path).is_some_and(|a| folder(name).is_some_and(|b| a.is(&b).unwrap_or(true)))
}
