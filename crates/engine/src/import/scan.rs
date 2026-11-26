//! Listing a folder's importable files in the background, for the import
//! dialog.

use std::path::{Path, PathBuf};

use photo_core::meta::FileKind;
use photo_core::time::DateTime;

use super::FileIssue;
use super::walk::{Found, walk};
use crate::engine::Engine;
use crate::jobs::Lane;
use crate::tasks::{TaskId, TaskKind, TaskState};

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
                let stop = || context.should_stop();
                let mut post = |found: Vec<Found>| {
                    context.progress(Box::new(move |engine: &mut Engine| {
                        engine.scan_found(task, found)
                    }));
                };
                let errors = walk(&root, recursive, &stop, &mut post);
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
