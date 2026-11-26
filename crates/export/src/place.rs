//! Writing safely: every export is written to a temporary file in the
//! destination folder and only then given its name, so a reader never sees
//! a half-written file, a failure leaves nothing behind, a name taken by
//! another file is never claimed by the unique-name and skip policies, and
//! the original photo is never the file replaced.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use same_file::Handle;

use crate::item::ExportError;
use crate::naming::{NamePlan, occupied};
use crate::settings::ExistingFile;

/// Distinguishes temporary files made by one process.
static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

/// The original photo's file, recognised by identity (device and inode on
/// Unix, volume serial number and file index on Windows) rather than by
/// name, as it was when the export started. Its handle stays open for the
/// whole export, which keeps the identity valid on Windows; it is opened
/// for reading and shared, so the original can still be read, moved or
/// renamed meanwhile.
pub(crate) struct Original {
    path: PathBuf,
    handle: Option<Handle>,
}

impl Original {
    /// The file at `path` now. When it cannot be opened (it is missing, say),
    /// it is recognised later by its path alone.
    pub(crate) fn capture(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            handle: Handle::from_path(path).ok(),
        }
    }

    /// Whether `target` is the original: the same file, under whatever name
    /// it has by now, through a hard link, a symbolic link or a name that
    /// differs only in case. A name that leads to no file is never the
    /// original. An error means `target` exists but its identity cannot be
    /// read, so nothing should replace it unverified.
    pub(crate) fn is(&self, target: &Path) -> io::Result<bool> {
        if !occupied(target) {
            return Ok(false);
        }
        let candidate = match Handle::from_path(target) {
            Ok(handle) => handle,
            // A dangling symbolic link leads to no file.
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        };
        Ok(match &self.handle {
            Some(original) => *original == candidate,
            None => Handle::from_path(&self.path).is_ok_and(|original| original == candidate),
        })
    }
}

/// A file under a hidden temporary name, removed when dropped unless it has
/// been placed.
pub(crate) struct TempFile {
    path: PathBuf,
    file: Option<File>,
    placed: bool,
    /// How the file is given a second name: a hard link, except in tests
    /// standing in for a file system that has none.
    link: fn(&Path, &Path) -> io::Result<()>,
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
                        link: |from, to| fs::hard_link(from, to),
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

    /// The same file as if its folder were on a file system without hard
    /// links (FAT, exFAT, many network shares), where linking fails.
    #[cfg(test)]
    fn without_hard_links(mut self) -> Self {
        self.link = |_, _| Err(io::ErrorKind::Unsupported.into());
        self
    }

    /// Gives the finished file its name under `policy`, trying candidates
    /// of `plan` from number `first` on. The `original` is never replaced,
    /// whatever its name is by now.
    pub(crate) fn place(
        mut self,
        plan: &NamePlan,
        first: u32,
        policy: ExistingFile,
        original: &Original,
    ) -> Result<Placed, ExportError> {
        // Closed before it is renamed: Windows cannot rename an open file.
        drop(self.file.take());
        match policy {
            ExistingFile::Overwrite => {
                let target = plan.candidate(first);
                if original
                    .is(&target)
                    .map_err(|error| ExportError::io(&target, error))?
                {
                    return Err(ExportError::WouldOverwriteSource(target));
                }
                fs::rename(&self.path, &target).map_err(|error| ExportError::io(&target, error))?;
                self.placed = true;
                Ok(Placed::At(target))
            }
            ExistingFile::Skip => {
                let target = plan.candidate(first);
                if self.claim(&target)? {
                    Ok(Placed::At(target))
                } else {
                    Ok(Placed::Skipped(target))
                }
            }
            ExistingFile::UniqueName => {
                let mut number = first;
                loop {
                    let target = plan.candidate(number);
                    if self.claim(&target)? {
                        return Ok(Placed::At(target));
                    }
                    number = plan.first_free(number + 1);
                }
            }
        }
    }

    /// Gives the file the name `target` only if nothing has it: `false`
    /// when it is taken. A hard link is made atomically or not at all; on
    /// a file system without hard links (FAT, exFAT, many network shares)
    /// the name is reserved instead (see [`claim_by_reservation`]). Either
    /// way no file this export did not create is ever replaced.
    fn claim(&mut self, target: &Path) -> Result<bool, ExportError> {
        match (self.link)(&self.path, target) {
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
            Err(_) => {
                let claimed = claim_by_reservation(&self.path, target)
                    .map_err(|error| ExportError::io(target, error))?;
                self.placed = claimed;
                Ok(claimed)
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

/// Moves `temporary` to `target` without hard links, only if nothing has
/// the name `target`: `false` when it is taken.
///
/// The name is first reserved with an exclusive create, which fails when
/// anything has it, so two exports racing for one name cannot both get it
/// and a file that appeared meanwhile is left alone. The finished file is
/// then renamed onto the reservation, the one file this can replace. When
/// that rename fails, the name is given back while it still holds the
/// empty reservation.
fn claim_by_reservation(temporary: &Path, target: &Path) -> io::Result<bool> {
    match OpenOptions::new().write(true).create_new(true).open(target) {
        // Closed at once: Windows cannot always rename onto an open file.
        Ok(reservation) => drop(reservation),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
        Err(error) => return Err(error),
    }
    if let Err(error) = fs::rename(temporary, target) {
        let still_reserved =
            fs::symlink_metadata(target).is_ok_and(|meta| meta.is_file() && meta.len() == 0);
        if still_reserved && let Err(cleanup) = fs::remove_file(target) {
            log::warn!(
                "cannot give back the reserved name {}: {cleanup}",
                target.display()
            );
        }
        return Err(error);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn file_with(path: &Path, bytes: &[u8]) {
        fs::write(path, bytes).unwrap();
    }

    fn finished(folder: &Path, bytes: &[u8]) -> TempFile {
        let mut temporary = TempFile::create(folder).unwrap();
        temporary.file().write_all(bytes).unwrap();
        temporary
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// Review finding E2: without hard links the name was checked and then
    /// renamed onto, so a file created in between was replaced.
    #[test]
    fn without_hard_links_a_file_that_appeared_is_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let (ours, target) = (dir.path().join("ours.part"), dir.path().join("x.jpg"));
        file_with(&ours, b"ours");
        file_with(&target, b"theirs");
        assert!(!claim_by_reservation(&ours, &target).unwrap());
        assert_eq!(fs::read(&target).unwrap(), b"theirs");
        assert_eq!(
            fs::read(&ours).unwrap(),
            b"ours",
            "the finished file is kept"
        );
    }

    #[test]
    fn without_hard_links_a_free_name_is_claimed_by_one_export_only() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("x.jpg");
        let (first, second) = (dir.path().join("1.part"), dir.path().join("2.part"));
        file_with(&first, b"first");
        file_with(&second, b"second");
        assert!(claim_by_reservation(&first, &target).unwrap());
        assert!(!first.exists());
        assert!(!claim_by_reservation(&second, &target).unwrap());
        assert_eq!(fs::read(&target).unwrap(), b"first");
    }

    #[test]
    fn without_hard_links_a_failed_move_gives_the_reserved_name_back() {
        let dir = tempfile::tempdir().unwrap();
        let (missing, target) = (dir.path().join("gone.part"), dir.path().join("x.jpg"));
        assert!(claim_by_reservation(&missing, &target).is_err());
        assert!(names(dir.path()).is_empty(), "{:?}", names(dir.path()));
    }

    /// Review finding E2 through the policies themselves: where hard links
    /// fail, unique-name and skip still never replace a file.
    #[test]
    fn without_hard_links_the_policies_still_never_replace_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let plan = NamePlan {
            folder: dir.path().to_path_buf(),
            stem: "x".into(),
            extension: "jpg".into(),
        };
        let original = Original::capture(&dir.path().join("absent.tif"));
        let export = |bytes: &[u8]| finished(dir.path(), bytes).without_hard_links();
        // The name was free when the export was planned; then this appeared.
        file_with(&plan.candidate(1), b"theirs");
        let placed = export(b"first").place(&plan, 1, ExistingFile::UniqueName, &original);
        assert!(matches!(placed, Ok(Placed::At(path)) if path == plan.candidate(2)));
        let skipped = export(b"second").place(&plan, 1, ExistingFile::Skip, &original);
        assert!(matches!(skipped, Ok(Placed::Skipped(path)) if path == plan.candidate(1)));
        assert_eq!(fs::read(plan.candidate(1)).unwrap(), b"theirs");
        assert_eq!(fs::read(plan.candidate(2)).unwrap(), b"first");
        // A free name is still taken.
        let free = NamePlan {
            stem: "y".into(),
            ..plan.clone()
        };
        let placed = export(b"third").place(&free, 1, ExistingFile::Skip, &original);
        assert!(matches!(placed, Ok(Placed::At(path)) if path == free.candidate(1)));
        assert_eq!(fs::read(free.candidate(1)).unwrap(), b"third");
        assert_eq!(names(dir.path()), ["x-2.jpg", "x.jpg", "y.jpg"]);
    }

    /// Review finding E1: the original was recognised by its path, so an
    /// original moved onto the destination during the render was replaced.
    #[test]
    fn an_original_moved_onto_the_destination_while_rendering_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (original, destination) = (dir.path().join("a.tif"), dir.path().join("b.tif"));
        file_with(&original, b"the only copy");
        let identity = Original::capture(&original);
        let temporary = finished(dir.path(), b"an export");
        fs::rename(&original, &destination).unwrap();
        let plan = NamePlan {
            folder: dir.path().to_path_buf(),
            stem: "b".into(),
            extension: "tif".into(),
        };
        let result = temporary.place(&plan, 1, ExistingFile::Overwrite, &identity);
        assert!(
            matches!(&result, Err(ExportError::WouldOverwriteSource(path)) if *path == destination),
            "{:?}",
            result.as_ref().map(|_| ())
        );
        assert_eq!(fs::read(&destination).unwrap(), b"the only copy");
        assert_eq!(names(dir.path()), ["b.tif"], "the temporary file is gone");
    }

    #[test]
    fn another_file_or_no_file_is_not_the_original() {
        let dir = tempfile::tempdir().unwrap();
        let (original, other) = (dir.path().join("a.tif"), dir.path().join("b.tif"));
        file_with(&original, b"original");
        file_with(&other, b"original");
        let identity = Original::capture(&original);
        assert!(identity.is(&original).unwrap());
        assert!(
            !identity.is(&other).unwrap(),
            "equal bytes are not the same file"
        );
        assert!(!identity.is(&dir.path().join("absent.tif")).unwrap());
        fs::hard_link(&original, dir.path().join("link.tif")).unwrap();
        assert!(identity.is(&dir.path().join("link.tif")).unwrap());
        // An original that was missing at the start is looked up by name.
        let late = Original::capture(&dir.path().join("late.tif"));
        assert!(!late.is(&other).unwrap());
        file_with(&dir.path().join("late.tif"), b"late");
        assert!(late.is(&dir.path().join("late.tif")).unwrap());
    }
}
