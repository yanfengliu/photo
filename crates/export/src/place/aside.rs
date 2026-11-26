//! Setting a file aside under a hidden name beside its own, and putting it
//! back: how the overwrite policy examines what has the export's name, and
//! how a failed copy gives its name back, without leaving anybody else's
//! file out of sight.
//!
//! [`set_aside`] moves what has a name to a hidden name in one rename, so
//! that what is aside is exactly what had the name. [`put_back`] returns it
//! without replacing anything: under its own name when that can be had,
//! otherwise under a visible name beside it ([`keep_beside`]), given by a
//! rename within the folder, which needs no room on the volume. Nothing
//! that is not the export's own file stays under a hidden name, unless even
//! that rename fails (the folder can no longer be written, say) or the
//! process stops between the two renames.

use std::collections::hash_map::RandomState;
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::disk::{Disk, Step};
use super::moves::{folder_of, move_new, remove_old_name};
use crate::naming::occupied;

/// How many hidden names are tried for moving a file aside when each turns
/// out to be taken.
const HIDDEN_TRIES: u32 = 16;

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
/// `.photo-export-<process>-<random>.<role>`. The roles are `part` (an
/// export being written), `previous` (a file moved aside by the overwrite
/// policy) and `discard` (a failed copy on its way out). Every hidden name
/// placement makes is made here.
pub(crate) fn hidden_name(folder: &Path, role: &str) -> PathBuf {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    loop {
        let count = COUNT.fetch_add(1, Ordering::Relaxed);
        let path = folder.join(hidden_file_name(std::process::id(), count, role));
        if !occupied(&path) {
            return path;
        }
    }
}

/// The hidden name that process `process` makes as its `count`th, for a
/// file in `role`. The process id is there for a person reading the folder.
/// It and the count are all that an exporter on another machine, writing
/// into the same shared folder, can have in common with this one; the
/// sixty-four random bits that follow are what keeps their names apart.
pub(crate) fn hidden_file_name(process: u32, count: u64, role: &str) -> String {
    format!(".photo-export-{process}-{:016x}.{role}", random(count))
}

/// Sixty-four bits that cannot be guessed and that no other process shares:
/// the standard library's randomly keyed hasher, which the operating system
/// seeds, over `count` and the time.
fn random(count: u64) -> u64 {
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(count);
    if let Ok(now) = SystemTime::now().duration_since(UNIX_EPOCH) {
        hasher.write_u128(now.as_nanos());
    }
    hasher.finish()
}

/// Moves what has `name` aside under a fresh hidden name beside it, in one
/// step, so that what is aside is exactly what had the name. Returns the
/// hidden name, or `None` when nothing had the name. The only way anything
/// is moved aside.
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

/// Puts a file that was moved aside back under `name` without replacing
/// anything, and returns where it is now. When `name` cannot be had (another
/// program has taken it meanwhile, there is no room for the copy that a file
/// system without a move needs, or the move fails) the file is renamed to a
/// visible name beside it instead ([`keep_beside`]). Only when even that
/// fails does it stay under its hidden name, which is then what is returned.
///
/// Moving it back can itself set aside a file that another program put
/// under `name` meanwhile (a copy that fails gives the name back); where
/// such a file ends up when that is not `name` is added to `kept`.
pub(crate) fn put_back(
    disk: &mut dyn Disk,
    aside: &Path,
    name: &Path,
    kept: &mut Vec<PathBuf>,
) -> PathBuf {
    disk.reached(Step::PutBack);
    match move_new(disk, aside, name, kept) {
        Ok(true) => return name.to_path_buf(),
        Ok(false) => {}
        Err(error) => log::warn!(
            "cannot put {} back as {}: {error}",
            aside.display(),
            name.display()
        ),
    }
    match keep_beside(disk, aside, name) {
        Ok(now) => now,
        Err(error) => {
            log::error!(
                "{} stays where it is: it cannot have a name beside {}: {error}",
                aside.display(),
                name.display()
            );
            aside.to_path_buf()
        }
    }
}

/// Renames the file at `aside` to the first free visible name beside
/// `name` (`stem-kept.ext`, then `stem-kept-2.ext` and so on) and returns
/// it. A rename within the folder, never a copy: it needs no room and keeps
/// the file itself.
pub(crate) fn keep_beside(disk: &mut dyn Disk, aside: &Path, name: &Path) -> io::Result<PathBuf> {
    for number in 1..=KEPT_NAMES {
        let candidate = kept_name(name, number);
        disk.reached(Step::Keep);
        if rename_new(disk, aside, &candidate)? {
            return Ok(candidate);
        }
    }
    Err(io::Error::other("every name tried beside it was taken"))
}

/// Moves `from` to the name `to` in the same folder by renaming it, never
/// by copying: `Ok(false)` when the name is taken. A hard link (then the old
/// name is removed) or an exclusive rename where there is one; where the
/// file system has neither, a plain rename onto a name that nothing has a
/// moment before.
///
/// That last is the one place where placement looks at a name and then acts
/// on it: a file another program creates under that very name in the
/// instant between would be replaced. It is used only for the `-kept`
/// names, which nothing else has a reason to create.
fn rename_new(disk: &mut dyn Disk, from: &Path, to: &Path) -> io::Result<bool> {
    match disk.link(from, to) {
        Ok(()) => {
            remove_old_name(from);
            return Ok(true);
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(error),
        // No hard links here, or none for this (a folder, say).
        Err(_) => {}
    }
    match disk.rename_exclusive(from, to) {
        Ok(()) => return Ok(true),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
        Err(_) => {}
    }
    if occupied(to) {
        return Ok(false);
    }
    fs::rename(from, to).map(|()| true)
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
