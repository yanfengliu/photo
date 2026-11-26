//! Writing safely: every export is written to a temporary file in the
//! destination folder and only then given its name.
//!
//! What that guarantees, for every way another program can interleave with
//! it (the tests in `place/tests` act between each pair of steps):
//!
//! - The skip and unique-name policies never replace or remove a file they
//!   did not create. The name is taken by an operation that cannot replace
//!   (see [`moves`]).
//! - The overwrite policy replaces what has the name, but never the
//!   original photo or a protected file: one that is seen to have the name
//!   is refused where it is, and what has the name is moved aside and
//!   identified before the export takes its place (see [`overwrite`]).
//! - A failure leaves nothing of the export behind and nothing of anyone
//!   else's missing: a file that was moved aside is put back, without
//!   replacing anything, under its name or a visible name beside it.
//!
//! One thing is out of scope: another program writing to this export's own
//! hidden names, `.photo-export-<process>-<number>.<role>` in the
//! destination folder. The crate documentation lists what else is not
//! guaranteed, among it the few cases in which a file cannot be put back
//! and stays, whole, under its hidden name.
//!
//! # What a crash leaves on disk
//!
//! The process can stop between any two steps. No step leaves a file that
//! was there before both gone and not yet replaced.
//!
//! | The process stops | What is in the folder |
//! | --- | --- |
//! | while the export is rendered and encoded | an unfinished `….part`; nothing under the final name |
//! | hard link: after the link, before the old name is removed | the finished export under its final name and under `….part` (two names of one file) |
//! | exclusive rename | one step: before it the finished `….part`, after it the export under its final name |
//! | copy (no hard links, no exclusive rename): while the bytes are copied | the final name holding part of the bytes, and the complete `….part` |
//! | overwrite: after what had the name is moved aside, before the export has the name | what had the name, whole, under `….previous`; the finished `….part`; nothing under the final name |
//! | overwrite: after the export has the name, before the old file is removed | the export under its final name; the old file under `….previous` |
//!
//! The fifth row is also where an original or protected file is between
//! being moved aside and being put back, which happens only to one that
//! takes the name in the instant after the export's look at it. A file
//! that is being put back goes through the second, third or fourth row
//! with `….previous` in the place of `….part`; a copy that failed is under
//! `….discard` for the instant before it is removed.
//!
//! All of this is about the process stopping. Against power loss only the
//! copy of the fourth row is synced, before the file it was made from is
//! removed; an export that takes its name by a hard link or an exclusive
//! rename is not synced first.

mod disk;
mod guard;
mod held;
mod moves;
mod overwrite;

#[cfg(test)]
mod tests;

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

pub(crate) use guard::Guarded;

use self::disk::{Disk, RealDisk};
use self::moves::{hidden_name, move_new};
use crate::item::ExportError;
use crate::naming::NamePlan;
use crate::settings::ExistingFile;

/// How many names in a row the unique-name policy goes for and finds taken
/// in the instant before it could have them, before it gives up. Losing
/// once takes another program acting at that very moment; a run this long
/// means something is wrong, and going on would write without end.
pub(crate) const UNIQUE_TRIES: u32 = 100;

/// A file under a hidden temporary name, removed when dropped unless it has
/// been placed.
pub(crate) struct TempFile {
    path: PathBuf,
    file: Option<File>,
    placed: bool,
}

/// Where a finished temporary file ended up.
#[derive(Debug, PartialEq, Eq)]
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
            let path = hidden_name(folder, "part");
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
    /// of `plan` from number `first` on. A `guarded` file is never
    /// replaced, whatever its name is by now, and under the skip and
    /// unique-name policies no file is.
    pub(crate) fn place(
        self,
        plan: &NamePlan,
        first: u32,
        policy: ExistingFile,
        guarded: &Guarded,
    ) -> Result<Placed, ExportError> {
        self.place_on(&mut RealDisk, plan, first, policy, guarded)
    }

    /// [`Self::place`] on `disk`.
    fn place_on(
        mut self,
        disk: &mut dyn Disk,
        plan: &NamePlan,
        first: u32,
        policy: ExistingFile,
        guarded: &Guarded,
    ) -> Result<Placed, ExportError> {
        // Closed before it is moved: Windows cannot rename an open file.
        drop(self.file.take());
        let target = match policy {
            ExistingFile::Overwrite => {
                let target = plan.candidate(first);
                overwrite::replace(disk, &self.path, &target, guarded)?;
                target
            }
            ExistingFile::Skip => {
                let target = plan.candidate(first);
                if !take(disk, &self.path, &target)? {
                    return Ok(Placed::Skipped(target));
                }
                target
            }
            ExistingFile::UniqueName => {
                let mut number = first;
                let mut lost = 0;
                loop {
                    let target = plan.candidate(number);
                    if take(disk, &self.path, &target)? {
                        break target;
                    }
                    lost += 1;
                    if lost >= UNIQUE_TRIES {
                        let error =
                            io::Error::other("other files kept taking the names of the export");
                        return Err(ExportError::io(&target, error));
                    }
                    number = plan.first_free(number + 1);
                }
            }
        };
        self.placed = true;
        Ok(Placed::At(target))
    }
}

/// Moves `finished` to the free name `target`; `false` when it is taken.
fn take(disk: &mut dyn Disk, finished: &Path, target: &Path) -> Result<bool, ExportError> {
    move_new(disk, finished, target).map_err(|error| ExportError::io(target, error))
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
