//! The worker's side of a scan: walking a folder and reading what its
//! files are.
//!
//! One folder can hold a whole card, on a drive that answers slowly, so the
//! walk asks whether to stop at every entry: while a folder is being
//! listed, before the listing is sorted, and while its files are looked at.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use photo_core::meta::FileKind;
use photo_core::time::DateTime;
use rayon::prelude::*;

use super::FileIssue;
use super::files::unix_seconds;

/// Files handed on at a time while a folder is scanned.
const CHUNK: usize = 32;

/// What a worker found about one file.
pub(super) struct Found {
    pub(super) path: PathBuf,
    pub(super) kind: FileKind,
    pub(super) bytes: u64,
    pub(super) modified: i64,
    pub(super) capture_time: Option<DateTime>,
}

/// Walk the folder, handing what is found to `found` in chunks; returns the
/// folders that could not be listed. Once `stop` says so, nothing more is
/// listed, looked at or handed on.
pub(super) fn walk(
    root: &Path,
    recursive: bool,
    stop: &dyn Fn() -> bool,
    found: &mut dyn FnMut(Vec<Found>),
) -> Vec<FileIssue> {
    let mut errors = Vec::new();
    let mut folders = vec![root.to_path_buf()];
    let mut pending = Vec::new();
    while let Some(folder) = folders.pop() {
        if stop() {
            break;
        }
        let listing = match fs::read_dir(&folder) {
            Ok(listing) => listing,
            Err(error) => {
                errors.push(FileIssue {
                    path: folder,
                    message: error.to_string(),
                });
                continue;
            }
        };
        let Some(entries) = sorted_entries(listing, stop) else {
            return errors;
        };
        let mut subfolders = Vec::new();
        for entry in entries {
            // At every entry: one folder can hold a whole card.
            if stop() {
                return errors;
            }
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if is_hidden(&path, &entry) {
                continue;
            }
            if file_type.is_dir() {
                subfolders.push(path);
            } else if let Some(kind) = photo_decode::file_kind(&path)
                && let Ok(stat) = fs::metadata(&path)
                && stat.is_file()
            {
                pending.push((path, kind, stat));
                if pending.len() == CHUNK {
                    post(stop, found, std::mem::take(&mut pending));
                }
            }
        }
        if recursive {
            folders.extend(subfolders.into_iter().rev());
        }
    }
    if !pending.is_empty() {
        post(stop, found, pending);
    }
    errors
}

/// The entries of a folder, by name; an entry that cannot be read is left
/// out. `None` once `stop` says so, which is asked at every entry as it is
/// listed and again before the sorting: nothing of a listing that was
/// cancelled is wanted.
fn sorted_entries(
    listing: impl Iterator<Item = io::Result<fs::DirEntry>>,
    stop: &dyn Fn() -> bool,
) -> Option<Vec<fs::DirEntry>> {
    let mut entries = Vec::new();
    for entry in listing {
        if stop() {
            return None;
        }
        entries.extend(entry);
    }
    if stop() {
        return None;
    }
    entries.sort_by_key(fs::DirEntry::file_name);
    Some(entries)
}

/// Read the capture times of a chunk of files (in parallel) and hand them
/// on; nothing is read once the scan has been told to stop.
fn post(
    stop: &dyn Fn() -> bool,
    found: &mut dyn FnMut(Vec<Found>),
    files: Vec<(PathBuf, FileKind, fs::Metadata)>,
) {
    if stop() {
        return;
    }
    let read: Vec<Found> = files
        .into_par_iter()
        .map(|(path, kind, stat)| Found {
            capture_time: photo_decode::read_metadata(&path)
                .ok()
                .and_then(|metadata| metadata.capture_time),
            bytes: stat.len(),
            modified: stat.modified().map(unix_seconds).unwrap_or(0),
            path,
            kind,
        })
        .collect();
    found(read);
}

/// Dot files everywhere, and files marked hidden on Windows.
fn is_hidden(path: &Path, entry: &fs::DirEntry) -> bool {
    let dotted = path
        .file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with('.'));
    dotted || hidden_attribute(entry)
}

#[cfg(windows)]
fn hidden_attribute(entry: &fs::DirEntry) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    entry
        .metadata()
        .is_ok_and(|metadata| metadata.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0)
}

#[cfg(not(windows))]
fn hidden_attribute(_entry: &fs::DirEntry) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use tempfile::TempDir;

    use super::*;

    /// A folder of `count` files a scan would offer. They are empty: a
    /// scan goes by the name, and finds no capture time.
    fn folder_of(count: usize) -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        for index in 0..count {
            fs::write(dir.path().join(format!("IMG_{index:04}.jpg")), b"").unwrap();
        }
        dir
    }

    #[test]
    fn a_cancel_that_lands_while_a_folder_is_listed_stops_the_listing_there() {
        let dir = folder_of(100);
        let listed = Cell::new(0);
        let listing = fs::read_dir(dir.path())
            .unwrap()
            .inspect(|_| listed.set(listed.get() + 1));
        // The scan is cancelled when the third entry has been listed.
        let entries = sorted_entries(listing, &|| listed.get() >= 3);
        assert!(entries.is_none(), "a cancelled listing hands nothing on");
        assert_eq!(listed.get(), 3, "and reads no further into the folder");
    }

    #[test]
    fn a_cancel_that_lands_when_a_folder_has_been_listed_stops_before_the_sorting() {
        let dir = folder_of(5);
        let mut entries = fs::read_dir(dir.path()).unwrap();
        let through = Cell::new(false);
        let listing = std::iter::from_fn(|| {
            let next = entries.next();
            through.set(next.is_none());
            next
        });
        // The scan is cancelled when the listing has run out.
        let entries = sorted_entries(listing, &|| through.get());
        assert!(entries.is_none(), "nothing is sorted or handed on");
    }

    #[test]
    fn a_folder_is_listed_by_name() {
        let dir = folder_of(40);
        let listing = fs::read_dir(dir.path()).unwrap();
        let entries = sorted_entries(listing, &|| false).unwrap();
        let names: Vec<_> = entries.iter().map(fs::DirEntry::file_name).collect();
        assert_eq!(names.len(), 40);
        assert!(names.is_sorted());
    }

    #[test]
    fn a_scan_cancelled_while_its_folder_is_listed_finds_nothing() {
        let dir = folder_of(100);
        // The scan asks whether to stop once when it comes to the folder
        // and then at every entry, so the fiftieth time it asks it has not
        // listed half of the folder. That is when the cancel lands.
        let asked = Cell::new(0);
        let stop = || {
            asked.set(asked.get() + 1);
            asked.get() >= 50
        };
        let mut found = Vec::new();
        let errors = walk(dir.path(), false, &stop, &mut |chunk| found.extend(chunk));
        assert!(errors.is_empty());
        assert!(
            found.is_empty(),
            "{} files were examined after the cancel",
            found.len()
        );
        assert_eq!(asked.get(), 50, "and the scan went no further");
    }

    #[test]
    fn a_scan_cancelled_while_its_files_are_looked_at_stops_at_the_next_file() {
        let dir = folder_of(100);
        // The cancel lands when the first chunk of files has been handed
        // on, with most of the folder still to be looked at.
        let handed = Cell::new(0);
        let asked_since = Cell::new(0);
        let stop = || {
            let cancelled = handed.get() > 0;
            asked_since.set(asked_since.get() + usize::from(cancelled));
            cancelled
        };
        let mut found = Vec::new();
        walk(dir.path(), false, &stop, &mut |chunk| {
            handed.set(handed.get() + 1);
            found.extend(chunk);
        });
        assert_eq!(found.len(), CHUNK, "what was handed on before the cancel");
        assert_eq!(
            asked_since.get(),
            1,
            "the scan stops at the next file, not at the end of a later chunk"
        );
    }

    #[test]
    fn a_scan_that_is_not_stopped_finds_every_file_in_order() {
        let dir = folder_of(70);
        fs::create_dir(dir.path().join("more")).unwrap();
        fs::write(dir.path().join("more").join("IMG_9000.jpg"), b"").unwrap();
        fs::write(dir.path().join(".hidden.jpg"), b"").unwrap();
        fs::write(dir.path().join("notes.txt"), b"").unwrap();
        let mut found = Vec::new();
        let errors = walk(dir.path(), true, &|| false, &mut |chunk| {
            found.extend(chunk);
        });
        assert!(errors.is_empty());
        let names: Vec<String> = found
            .iter()
            .map(|file| {
                file.path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(names.len(), 71);
        assert_eq!(names[0], "IMG_0000.jpg");
        assert_eq!(names[69], "IMG_0069.jpg");
        assert_eq!(names[70], "IMG_9000.jpg", "a subfolder after its parent");
    }
}
