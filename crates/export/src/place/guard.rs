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
//! All three compare identities of open files. One case identity cannot
//! settle: a guarded path that led nowhere until the very file it names had
//! been moved aside. That one is settled by name: the file came from the
//! guarded path's own folder (by identity) and file name (ignoring case).

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

/// One guarded file: the path it was named by and, when that path led to a
/// file as the export started, that file, kept open.
struct Kept {
    guard: Guard,
    path: PathBuf,
    at_start: Option<Held>,
}

impl Kept {
    /// True when `file` is the file this path led to at the start, or the
    /// file it leads to now. A path that cannot be opened now leads to
    /// nothing that could be compared.
    fn is(&self, file: &Held) -> io::Result<bool> {
        if let Some(start) = &self.at_start
            && start.is(file)?
        {
            return Ok(true);
        }
        match Held::open(&self.path) {
            Ok(now) => now.is(file),
            Err(_) => Ok(false),
        }
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
    /// file is compared.
    pub(crate) fn capture(source: &Path, protected: &[PathBuf]) -> Self {
        let kept = |guard, path: &Path| Kept {
            guard,
            path: path.to_path_buf(),
            at_start: Held::open(path).ok(),
        };
        let mut files = vec![kept(Guard::Source, source)];
        files.extend(protected.iter().map(|path| kept(Guard::Protected, path)));
        Self { files }
    }

    /// Which guarded file `path` leads to, if any: the same file under
    /// whatever name it has by now, through a hard link, a symbolic link or
    /// a name that differs only in case. A name that leads to no file is
    /// guarded by nothing. An error means the file is there but its
    /// identity cannot be read, so nothing should replace it unverified.
    pub(crate) fn guards(&self, path: &Path) -> io::Result<Option<Guard>> {
        let file = match Held::open(path) {
            Ok(file) => file,
            // Nothing has the name, or a symbolic link that leads nowhere.
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
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
    /// can still lead to it.
    pub(crate) fn look(&self) -> Sighting {
        let seen = |kept: &Kept| Held::open(&kept.path).ok();
        Sighting(self.files.iter().map(seen).collect())
    }

    /// Which guarded file the thing at `aside` is, having just been moved
    /// there from `name` in one rename, with `seen` looked up before that
    /// move. A symbolic link counts as the file it leads to.
    ///
    /// It is guarded when it is the file a guarded path led to at the
    /// start, at `seen` or now, or when a guarded path names the very
    /// place it was moved from (see the module documentation). An error
    /// means it is nothing an export may replace (a folder) or its identity
    /// cannot be read.
    pub(crate) fn identify(
        &self,
        aside: &Path,
        seen: &Sighting,
        name: &Path,
    ) -> io::Result<Option<Guard>> {
        if fs::symlink_metadata(aside)?.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::IsADirectory,
                "a folder has the name of the export",
            ));
        }
        let file = match Held::open(aside) {
            Ok(file) => Some(file),
            // A symbolic link that leads nowhere.
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        for (kept, seen) in self.files.iter().zip(&seen.0) {
            if let Some(file) = &file {
                if let Some(seen) = seen
                    && seen.is(file)?
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
