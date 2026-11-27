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
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use super::aside::{put_back, set_aside};
use super::disk::{Disk, Step};
use super::held::Held;
use crate::naming::occupied;

/// How much of a file is copied at a time. Far more than the usual 8 KiB on
/// purpose: on FAT and exFAT under macOS, where files are copied to their
/// names, every write is a round trip to a file system outside the kernel.
/// Measured on a FAT32 volume, 144 MB took 5.2 s in 8 KiB pieces and 0.2 s
/// in pieces of this size.
const COPY_PIECE: usize = 1 << 20;

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
///
/// A copy that fails gives its name back, and can find another program's
/// file there by then; where such a file ends up, when it cannot go back
/// under `to`, is added to `kept`.
pub(crate) fn move_new(
    disk: &mut dyn Disk,
    from: &Path,
    to: &Path,
    kept: &mut Vec<PathBuf>,
) -> io::Result<bool> {
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
    copy_new(disk, from, to, kept)
}

/// Gives the name `to` the content of `from` where no move can avoid
/// replacing: creates the name exclusively (which fails when anything has
/// it), copies the bytes through that same handle, syncs, and only then
/// removes `from`.
///
/// `Ok(false)` when the name is taken, and also when another program took
/// the name over while the bytes were copied: what is there then is not
/// this export's to touch, and `from` is kept. When the copy fails, or
/// whether the name still leads to the copy cannot be read, that is the
/// error, and the name is given back only if it can be shown to lead to
/// the file created here.
fn copy_new(
    disk: &mut dyn Disk,
    from: &Path,
    to: &Path,
    kept: &mut Vec<PathBuf>,
) -> io::Result<bool> {
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
        Err(error) if taken(&error, to) => return Ok(false),
        Err(error) => return Err(error),
    };
    // The open file is how this file is told from any other that comes to
    // have its name. Its identity is read when it is compared, not noted
    // here: on FAT and exFAT, the very file systems this is for, a file's
    // identity changes with its first bytes. So nothing can fail between
    // creating the name and the point from which a failure gives it back.
    let mut created = Held::keep(created);
    let filled = fill(disk, from, created.file()).and_then(|()| disk.is_at(&created, to));
    match filled {
        Ok(true) => {
            drop(created);
            disk.reached(Step::Unlink);
            remove_old_name(from);
            Ok(true)
        }
        // Another program renamed its own file onto the name while the bytes
        // were copied, which replaced the copy; or it removed the copy.
        Ok(false) => {
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
            give_back(disk, to, created, kept);
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
    disk.room()?;
    let mut source = File::open(from)?;
    let mut piece = vec![0; COPY_PIECE];
    loop {
        match source.read(&mut piece) {
            Ok(0) => break,
            Ok(read) => into.write_all(&piece[..read])?,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
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

/// Gives back a name [`copy_new`] created and could not fill or confirm,
/// but only the file created there. What has the name is moved aside in one
/// rename, so that what is examined is exactly what had it: if that is the
/// file `created`, it is removed; if another program put its own file there
/// in between, that file is put back untouched, under the name or, when it
/// cannot have that back, a visible name beside it, which goes into `kept`.
///
/// A file that cannot be told to be `created` (an identity cannot be read)
/// is treated as somebody else's: left where it is, or put back. It may be
/// this export's own copy, which then stays.
fn give_back(disk: &mut dyn Disk, to: &Path, created: Held, kept: &mut Vec<PathBuf>) {
    disk.reached(Step::GiveBack);
    // Looked at first, so that another program's file is not even moved.
    match disk.is_at(&created, to) {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            log::warn!(
                "cannot tell whether {} is the unfinished export, so it stays: {error}",
                to.display()
            );
            return;
        }
    }
    let aside = match set_aside(disk, to, "discard") {
        Ok(Some(aside)) => aside,
        Ok(None) => return,
        Err(error) => {
            log::warn!("cannot remove the unfinished {}: {error}", to.display());
            return;
        }
    };
    let ours = disk.is_at(&created, &aside);
    drop(created);
    match ours {
        Ok(true) => {
            if let Err(error) = fs::remove_file(&aside) {
                log::warn!("cannot remove the unfinished {}: {error}", aside.display());
            }
        }
        ours => {
            if let Err(error) = ours {
                log::warn!(
                    "cannot tell whether {} is the unfinished export, so it goes back: {error}",
                    to.display()
                );
            }
            let now = put_back(disk, &aside, to, kept);
            if now != to {
                log::warn!("{} is now {}", to.display(), now.display());
                kept.push(now);
            }
        }
    }
}

/// Removes the name a file was moved from, once it has its new one.
pub(crate) fn remove_old_name(path: &Path) {
    if let Err(error) = fs::remove_file(path)
        && error.kind() != io::ErrorKind::NotFound
    {
        log::warn!("cannot remove {}: {error}", path.display());
    }
}

/// Whether an exclusive create of `name` that failed with `error` failed
/// because something has the name. Windows answers an exclusive create on
/// a folder's name with "access is denied" rather than "already exists"
/// (CI run 45886744658), so a denial counts too when something is there:
/// the name is then taken like any other, and the overwrite policy looks at
/// what has it and refuses a folder where it stands.
fn taken(error: &io::Error, name: &Path) -> bool {
    match error.kind() {
        io::ErrorKind::AlreadyExists => true,
        io::ErrorKind::PermissionDenied => fs::symlink_metadata(name).is_ok(),
        _ => false,
    }
}
