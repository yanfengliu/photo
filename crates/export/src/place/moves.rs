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

use std::collections::hash_map::RandomState;
use std::fs::{self, File, OpenOptions};
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::disk::{Disk, Step};
use super::held::Held;
use crate::naming::occupied;

/// How many hidden names are tried for moving a file aside when each turns
/// out to be taken.
const HIDDEN_TRIES: u32 = 16;

/// How much of a file is copied at a time. Far more than the usual 8 KiB on
/// purpose: on FAT and exFAT under macOS, where files are copied to their
/// names, every write is a round trip to a file system outside the kernel.
/// Measured on a FAT32 volume, 144 MB took 5.2 s in 8 KiB pieces and 0.2 s
/// in pieces of this size.
const COPY_PIECE: usize = 1 << 20;

/// How many visible names are tried for a file that cannot go back under
/// its own.
const KEPT_NAMES: u32 = 99;

/// Whether the rename that cannot replace is what moves aside a file that
/// is not this export's own. It is on Apple's systems, where that rename is
/// one step and nothing else. It is not on Linux: on a file system without
/// such a rename `tempfile` makes a hard link and removes the old name,
/// two steps on a visible name, between which the name could come to lead
/// to another file that would then be unlinked unseen. And it is not on
/// Windows, where `tempfile` first resets the attributes of the file.
/// There a plain rename moves the file aside, and the random bits of the
/// hidden name are what keeps that name free.
pub(crate) const EXCLUSIVE_ASIDE: bool = cfg!(target_vendor = "apple");

/// A hidden name in `folder` that nothing has:
/// `.photo-export-<process>-<random>.<role>`. The process id is for a
/// person reading the folder; the sixty-four random bits are what keeps two
/// exporters, on two machines writing into one shared folder, from ever
/// using the same name. The roles are `part` (an export being written),
/// `previous` (a file moved aside by the overwrite policy) and `discard`
/// (a failed copy on its way out).
pub(crate) fn hidden_name(folder: &Path, role: &str) -> PathBuf {
    loop {
        let name = format!(
            ".photo-export-{}-{:016x}.{role}",
            std::process::id(),
            random()
        );
        let path = folder.join(name);
        if !occupied(&path) {
            return path;
        }
    }
}

/// Sixty-four bits that cannot be guessed and that no other process shares:
/// the standard library's randomly keyed hasher, which the operating system
/// seeds, over a count and the time.
pub(crate) fn random() -> u64 {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(COUNT.fetch_add(1, Ordering::Relaxed));
    if let Ok(now) = SystemTime::now().duration_since(UNIX_EPOCH) {
        hasher.write_u128(now.as_nanos());
    }
    hasher.finish()
}

/// Moves what has `name` aside under a fresh hidden name beside it, in one
/// step, so that what is aside is exactly what had the name. Returns the
/// hidden name, or `None` when nothing had the name.
///
/// Where [`EXCLUSIVE_ASIDE`] says so the step is the rename that cannot
/// replace: a hidden name that another exporter has come to use as well is
/// refused, and another one tried. Elsewhere, and on a file system without
/// such a rename, it is a plain rename.
pub(crate) fn set_aside(
    disk: &mut dyn Disk,
    name: &Path,
    role: &str,
) -> io::Result<Option<PathBuf>> {
    disk.reached(Step::SetAside);
    for _ in 0..HIDDEN_TRIES {
        let hidden = disk.hidden_name(folder_of(name), role);
        if EXCLUSIVE_ASIDE {
            match disk.rename_exclusive(name, &hidden) {
                Ok(()) => return Ok(Some(hidden)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                // No such rename on this file system.
                Err(_) => {}
            }
        }
        return match fs::rename(name, &hidden) {
            Ok(()) => Ok(Some(hidden)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        };
    }
    Err(io::Error::other("every hidden name tried was taken"))
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
/// this export's to touch, and `from` is kept. When the copy fails, or
/// whether the name still leads to the copy cannot be read, that is the
/// error, and the name is given back only if it can be shown to lead to
/// the file created here.
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
/// in between, that file is put back untouched.
///
/// A file that cannot be told to be `created` (an identity cannot be read)
/// is treated as somebody else's: left where it is, or put back. It may be
/// this export's own copy, which then stays.
fn give_back(disk: &mut dyn Disk, to: &Path, created: Held) {
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
            let now = put_back(disk, &aside, to);
            if now != to {
                log::warn!("{} is now {}", to.display(), now.display());
            }
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

    /// Review finding 6: a process id and a count are the same on two
    /// machines. The names carry the process id, for a person reading the
    /// folder, and sixty-four bits that are not a count.
    #[test]
    fn hidden_names_carry_the_process_id_and_sixty_four_random_bits() {
        let dir = tempfile::tempdir().unwrap();
        let prefix = format!(".photo-export-{}-", std::process::id());
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            let path = hidden_name(dir.path(), "previous");
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let random = name
                .strip_prefix(&prefix)
                .and_then(|rest| rest.strip_suffix(".previous"))
                .unwrap_or_else(|| panic!("{name}"));
            assert_eq!(random.len(), 16, "{name}");
            let number = u64::from_str_radix(random, 16).unwrap_or_else(|_| panic!("{name}"));
            assert!(seen.insert(number), "{name} came twice");
        }
        let (least, most) = (seen.iter().min().unwrap(), seen.iter().max().unwrap());
        assert!(most - least > 1 << 40, "the names follow one another");
    }
}
