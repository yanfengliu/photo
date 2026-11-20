//! Listing a folder's importable files in the background, for the import
//! dialog.

use std::fs;
use std::path::{Path, PathBuf};

use photo_core::meta::FileKind;
use photo_core::time::DateTime;
use rayon::prelude::*;

use super::FileIssue;
use super::files::unix_seconds;
use crate::engine::{Context, Engine};
use crate::jobs::Lane;
use crate::tasks::{TaskId, TaskKind, TaskState};

/// Files sent to the engine at a time while a folder is scanned.
const CHUNK: usize = 32;

/// A file a scan found that could be imported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportCandidate {
    /// Where the file is.
    pub path: PathBuf,
    /// Raster or RAW, by extension.
    pub kind: FileKind,
    /// Size in bytes.
    pub file_size: u64,
    /// Modification time, in Unix seconds.
    pub modified: i64,
    /// When the picture was taken, if the file says.
    pub capture_time: Option<DateTime>,
    /// True when exactly this path is already in the catalog.
    pub already_in_catalog: bool,
    /// True when the catalog holds another photo with the same file name,
    /// size and capture time.
    pub suspected_duplicate: bool,
}

/// The files found under one folder so far.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportScan {
    /// The task doing the scanning.
    pub task: TaskId,
    /// The folder scanned.
    pub root: PathBuf,
    /// Whether subfolders are included.
    pub recursive: bool,
    /// The importable files, in the order found: folder by folder, by name.
    pub candidates: Vec<ImportCandidate>,
    /// Folders that could not be listed.
    pub errors: Vec<FileIssue>,
    /// True once the whole folder has been listed (or the scan stopped).
    pub finished: bool,
}

/// What a worker found about one file.
struct Found {
    path: PathBuf,
    kind: FileKind,
    bytes: u64,
    modified: i64,
    capture_time: Option<DateTime>,
}

impl Engine {
    /// Start listing the importable files in `root` (and its subfolders if
    /// `recursive`), skipping hidden files and folders and files of
    /// unsupported types. Candidates arrive while the scan runs; see
    /// [`Engine::import_scan`]. A scan already running is cancelled.
    pub fn scan_folder(&mut self, root: &Path, recursive: bool) -> TaskId {
        if let Some(previous) = &self.imports.scan {
            let task = previous.task;
            self.tasks.cancel(task);
        }
        let label = format!("Scanning {}", root.display());
        let (task, token) = self.tasks.start(TaskKind::Scan, label, 0);
        self.imports.scan = Some(ImportScan {
            task,
            root: root.to_path_buf(),
            recursive,
            candidates: Vec::new(),
            errors: Vec::new(),
            finished: false,
        });
        let root = root.to_path_buf();
        self.submit(
            Lane::Background,
            None,
            Some(token),
            move |context| {
                let errors = walk(&root, recursive, context, task);
                Box::new(move |engine: &mut Engine| engine.scan_finished(task, errors))
            },
            move |engine, _| engine.scan_finished(task, Vec::new()),
        );
        task
    }

    /// The most recent scan, as far as it has got.
    pub fn import_scan(&self) -> Option<&ImportScan> {
        self.imports.scan.as_ref()
    }

    fn scan_found(&mut self, task: TaskId, found: Vec<Found>) -> bool {
        let candidates: Vec<ImportCandidate> =
            found.into_iter().map(|file| self.candidate(file)).collect();
        let Some(scan) = self.imports.scan.as_mut().filter(|scan| scan.task == task) else {
            return false;
        };
        scan.candidates.extend(candidates);
        let count = scan.candidates.len() as u64;
        if let Some(info) = self.tasks.get_mut(task) {
            info.done = count;
            info.total = count;
        }
        true
    }

    fn candidate(&self, file: Found) -> ImportCandidate {
        let already_in_catalog = matches!(self.catalog.find_by_path(&file.path), Ok(Some(_)));
        let name = file
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let suspected_duplicate = !already_in_catalog
            && self
                .catalog
                .find_suspected_duplicates(&name, file.bytes, file.capture_time)
                .is_ok_and(|duplicates| !duplicates.is_empty());
        ImportCandidate {
            path: file.path,
            kind: file.kind,
            file_size: file.bytes,
            modified: file.modified,
            capture_time: file.capture_time,
            already_in_catalog,
            suspected_duplicate,
        }
    }

    fn scan_finished(&mut self, task: TaskId, errors: Vec<FileIssue>) -> bool {
        let state = if self.tasks.is_cancelled(task) {
            TaskState::Cancelled
        } else {
            TaskState::Finished
        };
        if let Some(info) = self.tasks.get_mut(task) {
            info.errors = errors.len() as u64;
        }
        self.tasks.finish(task, state);
        if let Some(scan) = self.imports.scan.as_mut().filter(|scan| scan.task == task) {
            scan.errors = errors;
            scan.finished = true;
        }
        true
    }
}

/// Walk the folder, posting what is found in chunks; returns the folders
/// that could not be listed.
fn walk(root: &Path, recursive: bool, context: &Context<'_>, task: TaskId) -> Vec<FileIssue> {
    let mut errors = Vec::new();
    let mut folders = vec![root.to_path_buf()];
    let mut pending = Vec::new();
    while let Some(folder) = folders.pop() {
        if context.should_stop() {
            break;
        }
        let mut entries = match fs::read_dir(&folder) {
            Ok(entries) => entries.flatten().collect::<Vec<_>>(),
            Err(error) => {
                errors.push(FileIssue {
                    path: folder,
                    message: error.to_string(),
                });
                continue;
            }
        };
        entries.sort_by_key(|entry| entry.file_name());
        let mut subfolders = Vec::new();
        for entry in entries {
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
                    post(context, task, std::mem::take(&mut pending));
                }
            }
        }
        if recursive {
            folders.extend(subfolders.into_iter().rev());
        }
    }
    if !pending.is_empty() && !context.should_stop() {
        post(context, task, pending);
    }
    errors
}

/// Read the capture times of a chunk of files (in parallel) and post them.
fn post(context: &Context<'_>, task: TaskId, files: Vec<(PathBuf, FileKind, fs::Metadata)>) {
    let found: Vec<Found> = files
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
    context.progress(Box::new(move |engine: &mut Engine| {
        engine.scan_found(task, found)
    }));
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
