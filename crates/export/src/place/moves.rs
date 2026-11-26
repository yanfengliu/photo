//! Moving a file to a name without ever replacing what has that name: the
//! one operation every placement policy is built on.
//!
//! [`move_new`] tries, in turn, the ways a file system can do that:
//!
//! 1. A hard link, then removing the old name. The link is made only if
//!    nothing has the name at that instant.
//! 2. An exclusive rename, where the platform has one. One step.
//! 3. Where the file system offers neither (FAT and exFAT on macOS, some
//!    network shares): the name is created exclusively and the bytes are
//!    copied through that same open handle ([`copy_new`]). Nothing is ever
//!    renamed onto the name, so nothing can be replaced there either; the
//!    price is that a reader can watch the file grow under its final name.
//!    A symbolic link, which has no bytes of its own to copy, is made anew
//!    under the name instead ([`relink_new`]), which is exclusive too.
//!
//! No step looks at a name first and acts on it afterwards: whatever another
//! program does between two steps, a file this export did not create is
//! neither replaced nor removed.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::disk::{Disk, Step};
use super::held::Held;
use crate::naming::occupied;

/// Distinguishes the hidden names made by one process.
static NEXT_HIDDEN: AtomicU64 = AtomicU64::new(0);

/// How many visible names are tried for a file that cannot go back under
/// its own.
const KEPT_NAMES: u32 = 99;

/// A hidden name in `folder` that nothing has:
/// `.photo-export-<process>-<number>.<role>`. The roles are `part` (an
/// export being written), `previous` (a file moved aside by the overwrite
/// policy) and `discard` (a failed copy on its way out).
pub(crate) fn hidden_name(folder: &Path, role: &str) -> PathBuf {
    loop {
        let number = NEXT_HIDDEN.fetch_add(1, Ordering::Relaxed);
        let name = format!(".photo-export-{}-{number}.{role}", std::process::id());
        let path = folder.join(name);
        // A name left behind by an earlier process with this number is
        // passed over, never reused.
        if !occupied(&path) {
            return path;
        }
    }
}

/// The folder a path's last component is in.
pub(crate) fn folder_of(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

/// Moves the file at `from` to the name `to` without replacing anything:
/// `Ok(false)` when something has that name, in which case `from` is still
/// where it was. See the module documentation for how.
pub(crate) fn move_new(disk: &mut dyn Disk, from: &Path, to: &Path) -> io::Result<bool> {
    disk.reached(Step::Link);
    match disk.link(from, to) {
        Ok(()) => {
            disk.reached(Step::Unlink);
            remove_old_name(from);
            return Ok(true);
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
        // The file or the folder is gone: no other way would fare better.
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(error),
        // No hard links here, or none for this file.
        Err(_) => {}
    }
    disk.reached(Step::RenameExclusive);
    match disk.rename_exclusive(from, to) {
        Ok(()) => return Ok(true),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
        Err(_) => {}
    }
    copy_new(disk, from, to)
}

/// Gives the name `to` the content of `from` where no move can avoid
/// replacing: creates the name exclusively (which fails when anything has
/// it), copies the bytes through that same handle, syncs, and only then
/// removes `from`.
///
/// `Ok(false)` when the name is taken, and also when another program took
/// the name over while the bytes were copied: what is there then is not
/// this export's to touch, and `from` is kept. When the copy fails, the name
/// is given back only if it still leads to the file created here.
fn copy_new(disk: &mut dyn Disk, from: &Path, to: &Path) -> io::Result<bool> {
    let kind = fs::symlink_metadata(from)?.file_type();
    if kind.is_symlink() {
        return relink_new(disk, from, to);
    }
    if !kind.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "only a regular file or a symbolic link can be copied to a new name",
        ));
    }
    disk.reached(Step::Create);
    let created = match OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(to)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
        Err(error) => return Err(error),
    };
    // The open file is how this file is told from any other that comes to
    // have its name. Its identity is read when it is compared, not noted
    // here: on FAT and exFAT, the very file systems this is for, a file's
    // identity changes with its first bytes.
    let mut created = Held::keep(created)?;
    let filled = fill(disk, from, created.file());
    let still_here = created.is_at(to);
    match filled {
        Ok(()) if still_here => {
            drop(created);
            disk.reached(Step::Unlink);
            remove_old_name(from);
            Ok(true)
        }
        Ok(()) => {
            drop(created);
            if occupied(to) {
                Ok(false)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "the file was removed while it was being written",
                ))
            }
        }
        Err(error) => {
            give_back(disk, to, created);
            Err(error)
        }
    }
}

/// Gives the name `to` to the symbolic link at `from` where no move can
/// avoid replacing: makes the same link under the new name, which fails when
/// anything has that name, and only then removes the old one. `Ok(false)`
/// when the name is taken, in which case `from` is still where it was.
///
/// A relative link leads somewhere else from another folder, so it is
/// remade only beside where it was (which is where a file that was moved
/// aside always goes back to).
#[cfg(unix)]
fn relink_new(disk: &mut dyn Disk, from: &Path, to: &Path) -> io::Result<bool> {
    let leads_to = fs::read_link(from)?;
    if leads_to.is_relative() && folder_of(from) != folder_of(to) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "a relative symbolic link cannot be remade in another folder",
        ));
    }
    disk.reached(Step::Create);
    match std::os::unix::fs::symlink(&leads_to, to) {
        Ok(()) => {
            disk.reached(Step::Unlink);
            remove_old_name(from);
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(error),
    }
}

/// Windows has an exclusive rename on every file system, so a symbolic
/// link never has to be remade there (and making one needs a privilege).
#[cfg(not(unix))]
fn relink_new(_: &mut dyn Disk, _: &Path, _: &Path) -> io::Result<bool> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "a symbolic link cannot be remade under a new name here",
    ))
}

/// Copies `from` into the open file `into`, with its modification time and
/// permissions, and syncs it.
fn fill(disk: &mut dyn Disk, from: &Path, into: &mut File) -> io::Result<()> {
    disk.reached(Step::Fill);
    let mut source = File::open(from)?;
    io::copy(&mut source, into)?;
    if let Ok(meta) = source.metadata() {
        // Best effort: the copy stands for the file it copies.
        if let Ok(modified) = meta.modified() {
            let _ = into.set_modified(modified);
        }
        let _ = into.set_permissions(meta.permissions());
    }
    into.sync_all()?;
    disk.reached(Step::Confirm);
    Ok(())
}

/// Gives back a name [`copy_new`] created and could not fill, but only the
/// file created there. What has the name is moved aside in one rename, so
/// that what is examined is exactly what had it: if that is the file
/// `created`, it is removed; if another program put its own file there in
/// between, that file is put back untouched.
fn give_back(disk: &mut dyn Disk, to: &Path, created: Held) {
    disk.reached(Step::GiveBack);
    // Looked at first, so that another program's file is not even moved.
    if !created.is_at(to) {
        return;
    }
    let aside = hidden_name(folder_of(to), "discard");
    if let Err(error) = fs::rename(to, &aside) {
        log::warn!("cannot remove the unfinished {}: {error}", to.display());
        return;
    }
    let ours = created.is_at(&aside);
    drop(created);
    if ours {
        if let Err(error) = fs::remove_file(&aside) {
            log::warn!("cannot remove the unfinished {}: {error}", aside.display());
        }
    } else {
        let now = put_back(disk, &aside, to);
        if now != to {
            log::warn!("{} is now {}", to.display(), now.display());
        }
    }
}

/// Puts a file that was moved aside back under `name` without replacing
/// anything, and returns where it is now: `name`, or, when something else
/// has taken that name meanwhile, the first free of `stem-kept.ext`,
/// `stem-kept-2.ext` and so on beside it. Only when none of those can be
/// had (the folder can no longer be written, say) does the file stay where
/// it was, under its hidden name, which is then what is returned.
pub(crate) fn put_back(disk: &mut dyn Disk, aside: &Path, name: &Path) -> PathBuf {
    disk.reached(Step::PutBack);
    let kept = (1..=KEPT_NAMES).map(|number| kept_name(name, number));
    for candidate in std::iter::once(name.to_path_buf()).chain(kept) {
        match move_new(disk, aside, &candidate) {
            Ok(true) => return candidate,
            Ok(false) => {}
            Err(error) => {
                log::error!(
                    "cannot put {} back as {}: {error}",
                    aside.display(),
                    candidate.display()
                );
                break;
            }
        }
    }
    aside.to_path_buf()
}

/// The `number`th visible name beside `name` for a file that could not go
/// back under it: `stem-kept.ext`, then `stem-kept-2.ext` and so on.
fn kept_name(name: &Path, number: u32) -> PathBuf {
    let stem = name.file_stem().unwrap_or_default().to_string_lossy();
    let mark = if number <= 1 {
        "-kept".to_string()
    } else {
        format!("-kept-{number}")
    };
    let file = match name.extension() {
        Some(extension) => format!("{stem}{mark}.{}", extension.to_string_lossy()),
        None => format!("{stem}{mark}"),
    };
    name.with_file_name(file)
}

/// Removes the name a file was moved from, once it has its new one.
fn remove_old_name(path: &Path) {
    if let Err(error) = fs::remove_file(path)
        && error.kind() != io::ErrorKind::NotFound
    {
        log::warn!("cannot remove {}: {error}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kept_names_sit_beside_the_name_and_count_up() {
        let name = Path::new("out").join("IMG 1.jpg");
        assert_eq!(kept_name(&name, 1), Path::new("out").join("IMG 1-kept.jpg"));
        assert_eq!(
            kept_name(&name, 3),
            Path::new("out").join("IMG 1-kept-3.jpg")
        );
        assert_eq!(kept_name(Path::new("bare"), 2), Path::new("bare-kept-2"));
        assert_eq!(folder_of(Path::new("bare")), Path::new("."));
        assert_eq!(folder_of(&name), Path::new("out"));
    }

    #[test]
    fn hidden_names_are_fresh_and_pass_over_leftovers() {
        let dir = tempfile::tempdir().unwrap();
        let first = hidden_name(dir.path(), "part");
        let second = hidden_name(dir.path(), "part");
        assert_ne!(first, second);
        let name = first.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with(".photo-export-") && name.ends_with(".part"));
    }
}
