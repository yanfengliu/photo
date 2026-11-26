//! Placement with another program in the way.
//!
//! Every test places a finished export in a real temporary folder through
//! a stand-in [`Disk`]: the real file system minus the no-replace moves a
//! kind of file system lacks, with an intruder that is called before every
//! step and can do there what another program could: create a file at the
//! name, rename one onto it, move the original onto it. [`at_every_step`]
//! repeats a scene with the intruder acting before each step in turn, so a
//! property is shown for every interleaving of one such act, not for one
//! chosen order.

mod no_replace;
mod overwrite;

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use same_file::Handle;
use tempfile::TempDir;

use super::disk::{self, Disk, Step};
use super::guard::{Guard, Guarded};
use super::{Placed, TempFile};
use crate::item::ExportError;
use crate::naming::NamePlan;
use crate::settings::ExistingFile;

/// Which moves that cannot replace a stand-in file system has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Hard links and an exclusive rename: APFS, ext4, NTFS.
    Both,
    /// Hard links only.
    LinksOnly,
    /// An exclusive rename only: FAT and exFAT on Windows.
    ExclusiveOnly,
    /// Neither: FAT and exFAT on macOS.
    Neither,
}

const KINDS: [Kind; 4] = [
    Kind::Both,
    Kind::LinksOnly,
    Kind::ExclusiveOnly,
    Kind::Neither,
];

/// The real file system as a file system of `kind` would behave, calling
/// `intruder` with each step's number and name just before the step.
struct Staged<'a> {
    kind: Kind,
    steps: Vec<Step>,
    intruder: &'a mut dyn FnMut(usize, Step),
}

impl Disk for Staged<'_> {
    fn link(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        if matches!(self.kind, Kind::Both | Kind::LinksOnly) {
            fs::hard_link(from, to)
        } else {
            Err(io::ErrorKind::Unsupported.into())
        }
    }

    fn rename_exclusive(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        if matches!(self.kind, Kind::Both | Kind::ExclusiveOnly) {
            disk::rename_exclusive(from, to)
        } else {
            Err(io::ErrorKind::Unsupported.into())
        }
    }

    fn reached(&mut self, step: Step) {
        (self.intruder)(self.steps.len(), step);
        self.steps.push(step);
    }
}

/// A temporary folder in which one export, holding `export`, is placed as
/// `x.jpg`.
struct Folder(TempDir);

impl Folder {
    fn new() -> Self {
        Self(tempfile::tempdir().unwrap())
    }

    fn at(&self, name: &str) -> PathBuf {
        self.0.path().join(name)
    }

    fn put(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.at(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    /// The bytes under `name`, or `None` when nothing has it.
    fn read(&self, name: &str) -> Option<Vec<u8>> {
        fs::read(self.at(name)).ok()
    }

    /// Every name in the folder, sorted.
    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.0.path())
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// The hidden names this crate makes that are in the folder.
    fn hidden(&self) -> Vec<String> {
        let ours = |name: &String| name.starts_with(".photo-export-");
        self.names().into_iter().filter(ours).collect()
    }

    /// The export's temporary file while it has not been placed.
    fn temporary(&self) -> Option<PathBuf> {
        let part = |name: &String| name.ends_with(".part");
        self.hidden()
            .iter()
            .find(|name| part(name))
            .map(|name| self.at(name))
    }

    /// Places a finished export as `x.jpg` under `policy` on a file system
    /// of `kind`, with `intruder` called before every step. Returns what
    /// placement reported and the steps it went through.
    fn place(
        &self,
        kind: Kind,
        policy: ExistingFile,
        guarded: &Guarded,
        intruder: &mut dyn FnMut(usize, Step),
    ) -> (Result<Placed, ExportError>, Vec<Step>) {
        let mut finished = TempFile::create(self.0.path()).unwrap();
        finished.file().write_all(b"export").unwrap();
        let plan = NamePlan {
            folder: self.0.path().to_path_buf(),
            stem: "x".into(),
            extension: "jpg".into(),
        };
        let mut disk = Staged {
            kind,
            steps: Vec::new(),
            intruder,
        };
        let result = finished.place_on(&mut disk, &plan, 1, policy, guarded);
        (result, disk.steps)
    }

    /// [`Self::place`] with `act` run once, before step number `when`.
    fn place_with(
        &self,
        kind: Kind,
        policy: ExistingFile,
        guarded: &Guarded,
        when: Option<usize>,
        act: &mut dyn FnMut(),
    ) -> (Result<Placed, ExportError>, Vec<Step>) {
        let mut intruder = |index: usize, _: Step| {
            if Some(index) == when {
                act();
            }
        };
        self.place(kind, policy, guarded, &mut intruder)
    }
}

/// A guard that guards nothing in the folder.
fn nothing_guarded(folder: &Folder) -> Guarded {
    Guarded::capture(&folder.at("no-original.tif"), &[])
}

/// Runs `scene` on every kind of file system: once undisturbed (`None`),
/// then with the intruder acting before step 0, before step 1, and so on,
/// for as long as the run reaches the step asked for. `scene` builds its
/// own fresh folder, checks its own outcome and returns the number of steps
/// its placement went through.
fn at_every_step(mut scene: impl FnMut(Kind, Option<usize>) -> usize) {
    for kind in KINDS {
        assert!(scene(kind, None) > 0, "{kind:?}: no step was reached");
        let mut when = 0;
        while when < scene(kind, Some(when)) {
            when += 1;
        }
    }
}

/// True when the two paths lead to one file.
fn same_file(a: &Path, b: &Path) -> bool {
    Handle::from_path(a).unwrap() == Handle::from_path(b).unwrap()
}

#[test]
fn the_guard_knows_a_file_by_identity_not_by_bytes_or_name() {
    let folder = Folder::new();
    let original = folder.put("a.tif", b"same bytes");
    let other = folder.put("b.tif", b"same bytes");
    let guarded = Guarded::capture(&original, &[]);
    assert_eq!(guarded.guards(&original).unwrap(), Some(Guard::Source));
    assert_eq!(guarded.guards(&other).unwrap(), None);
    assert_eq!(guarded.guards(&folder.at("absent.tif")).unwrap(), None);
    fs::hard_link(&original, folder.at("link.tif")).unwrap();
    assert_eq!(
        guarded.guards(&folder.at("link.tif")).unwrap(),
        Some(Guard::Source)
    );
    // Moved: its old name no longer leads to it, its identity still does.
    fs::rename(&original, folder.at("moved.tif")).unwrap();
    assert_eq!(
        guarded.guards(&folder.at("moved.tif")).unwrap(),
        Some(Guard::Source)
    );
    // A path that led nowhere at the start is looked up again.
    let late = Guarded::capture(&folder.at("late.tif"), std::slice::from_ref(&other));
    assert_eq!(late.guards(&other).unwrap(), Some(Guard::Protected));
    folder.put("late.tif", b"late");
    assert_eq!(
        late.guards(&folder.at("late.tif")).unwrap(),
        Some(Guard::Source)
    );
}
