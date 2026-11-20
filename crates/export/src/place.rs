//! Writing safely: every export is written to a temporary file in the
//! destination folder and only then given its name, so a reader never sees
//! a half-written file, a failure leaves nothing behind, and the original
//! photo is never the file replaced.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::item::ExportError;
use crate::naming::{NamePlan, occupied};
use crate::settings::ExistingFile;

/// Distinguishes temporary files made by one process.
static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

/// A file under a hidden temporary name, removed when dropped unless it has
/// been placed.
pub(crate) struct TempFile {
    path: PathBuf,
    file: Option<File>,
    placed: bool,
}

/// Where a finished temporary file ended up.
pub(crate) enum Placed {
    /// It now has this name.
    At(PathBuf),
    /// The policy was to skip and the name was taken while the file was
    /// being written; the temporary file has been removed.
    Skipped(PathBuf),
}

impl TempFile {
    /// A new, empty temporary file in `folder`, which must exist.
    pub(crate) fn create(folder: &Path) -> Result<Self, ExportError> {
        loop {
            let number = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
            let path = folder.join(format!(
                ".photo-export-{}-{number}.part",
                std::process::id()
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => {
                    return Ok(Self {
                        path,
                        file: Some(file),
                        placed: false,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(ExportError::io(&path, error)),
            }
        }
    }

    /// The open file, for writing.
    pub(crate) fn file(&mut self) -> &mut File {
        self.file
            .as_mut()
            .expect("the file stays open until the temporary file is placed")
    }

    /// Gives the finished file its name under `policy`, trying candidates
    /// of `plan` from number `first` on. The original photo at `source` is
    /// never replaced.
    pub(crate) fn place(
        mut self,
        plan: &NamePlan,
        first: u32,
        policy: ExistingFile,
        source: &Path,
    ) -> Result<Placed, ExportError> {
        // Closed before it is renamed: Windows cannot rename an open file.
        drop(self.file.take());
        match policy {
            ExistingFile::Overwrite => {
                let target = plan.candidate(first);
                if is_source(&target, source) {
                    return Err(ExportError::WouldOverwriteSource(target));
                }
                fs::rename(&self.path, &target).map_err(|error| ExportError::io(&target, error))?;
                self.placed = true;
                Ok(Placed::At(target))
            }
            ExistingFile::Skip => {
                let target = plan.candidate(first);
                if self.link_new(&target)? {
                    Ok(Placed::At(target))
                } else {
                    Ok(Placed::Skipped(target))
                }
            }
            ExistingFile::UniqueName => {
                let mut number = first;
                loop {
                    let target = plan.candidate(number);
                    if self.link_new(&target)? {
                        return Ok(Placed::At(target));
                    }
                    number = plan.first_free(number + 1);
                }
            }
        }
    }

    /// Gives the file the name `target` only if nothing has it: `false`
    /// when it is taken. Hard linking is atomic, so two exports racing for
    /// one name cannot both win; a file system without hard links falls
    /// back to a check followed by a rename.
    fn link_new(&mut self, target: &Path) -> Result<bool, ExportError> {
        match fs::hard_link(&self.path, target) {
            Ok(()) => {
                self.placed = true;
                if let Err(error) = fs::remove_file(&self.path) {
                    log::warn!(
                        "cannot remove the temporary file {}: {error}",
                        self.path.display()
                    );
                }
                Ok(true)
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
            Err(_) if occupied(target) => Ok(false),
            Err(_) => {
                fs::rename(&self.path, target).map_err(|error| ExportError::io(target, error))?;
                self.placed = true;
                Ok(true)
            }
        }
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if !self.placed {
            drop(self.file.take());
            if let Err(error) = fs::remove_file(&self.path)
                && error.kind() != io::ErrorKind::NotFound
            {
                log::warn!(
                    "cannot remove the temporary file {}: {error}",
                    self.path.display()
                );
            }
        }
    }
}

/// True when `target` names the original photo itself: the same file, not
/// merely the same name, so a hard link, a symbolic link or a name differing
/// only in case on a case-insensitive file system all count. A path that
/// does not exist is never the original, which does.
pub(crate) fn is_source(target: &Path, source: &Path) -> bool {
    occupied(target) && same_file(target, source)
}

#[cfg(unix)]
fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

#[cfg(not(unix))]
fn same_file(a: &Path, b: &Path) -> bool {
    // Canonical paths name the file as the file system stores it; Windows
    // file systems compare names without regard to case.
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase(),
        _ => false,
    }
}
