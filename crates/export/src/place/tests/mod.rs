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
//!
//! The temporary folders are where `TMPDIR` says. A stand-in can take a
//! move away from the real file system but cannot give it one, and it
//! cannot change how the real file system numbers its files, so the tests
//! are worth running on a FAT and an exFAT volume as well (measured under
//! macOS: neither move, and a file's identity changes with its first
//! bytes): `TMPDIR=/Volumes/<name> cargo test -p photo-export`. There only
//! the kinds the volume can stage are run ([`kinds`]).

mod failing;
mod identity;
mod kept;
mod no_replace;
mod overwrite;

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use tempfile::TempDir;

use super::disk::{self, Disk, Step};
use super::guard::{Guard, Guarded};
use super::held::Held;
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

/// Which moves that cannot replace the volume the tests run on really has.
#[derive(Clone, Copy, Debug)]
struct Volume {
    links: bool,
    exclusive: bool,
}

/// What the volume under the temporary folders has, tried once.
fn volume() -> Volume {
    static VOLUME: OnceLock<Volume> = OnceLock::new();
    *VOLUME.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("from");
        fs::write(&from, b"probe").unwrap();
        Volume {
            links: fs::hard_link(&from, dir.path().join("linked")).is_ok(),
            exclusive: disk::rename_exclusive(&from, &dir.path().join("renamed")).is_ok(),
        }
    })
}

/// The kinds of file system that can be staged on this volume: every kind
/// whose moves the volume really has.
fn kinds() -> Vec<Kind> {
    let volume = volume();
    let staged = |kind: &Kind| match kind {
        Kind::Both => volume.links && volume.exclusive,
        Kind::LinksOnly => volume.links,
        Kind::ExclusiveOnly => volume.exclusive,
        Kind::Neither => true,
    };
    let all = [
        Kind::Both,
        Kind::LinksOnly,
        Kind::ExclusiveOnly,
        Kind::Neither,
    ];
    all.into_iter().filter(staged).collect()
}

/// The real file system as a file system of `kind` would behave, calling
/// `intruder` with each step's number and name just before the step.
struct Staged<'a> {
    kind: Kind,
    steps: Vec<Step>,
    intruder: &'a mut dyn FnMut(usize, Step),
    /// Hidden names to hand out before real ones, first one first: names
    /// that another exporter came to use too.
    hidden: Vec<PathBuf>,
    /// Whether reading an identity fails, by the number of the read (the
    /// first is 0): as when no descriptor is left to open the file with.
    identity_fails: fn(usize) -> bool,
    /// How many identities have been read.
    identity_reads: usize,
    /// The volume has no room for a copy.
    full: bool,
    /// A replaced file cannot be removed (read-only on Windows, say).
    removal_fails: bool,
}

impl<'a> Staged<'a> {
    fn new(kind: Kind, intruder: &'a mut dyn FnMut(usize, Step)) -> Self {
        Self {
            kind,
            steps: Vec::new(),
            intruder,
            hidden: Vec::new(),
            identity_fails: |_| false,
            identity_reads: 0,
            full: false,
            removal_fails: false,
        }
    }
}

impl Disk for Staged<'_> {
    fn hidden_name(&mut self, folder: &Path, role: &str) -> PathBuf {
        if self.hidden.is_empty() {
            super::aside::hidden_name(folder, role)
        } else {
            self.hidden.remove(0)
        }
    }

    fn is_at(&mut self, held: &Held, path: &Path) -> io::Result<bool> {
        let read = self.identity_reads;
        self.identity_reads += 1;
        if (self.identity_fails)(read) {
            Err(io::Error::other("the identity cannot be read"))
        } else {
            held.is_at(path)
        }
    }

    fn room(&mut self) -> io::Result<()> {
        if self.full {
            Err(io::ErrorKind::StorageFull.into())
        } else {
            Ok(())
        }
    }

    fn remove(&mut self, path: &Path) -> io::Result<()> {
        if self.removal_fails {
            Err(io::ErrorKind::PermissionDenied.into())
        } else {
            fs::remove_file(path)
        }
    }

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

    /// Every name in the folder, sorted. The `._name` companions that macOS
    /// keeps beside files on a volume without extended attributes are its
    /// own business and are left out.
    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.0.path())
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| !name.starts_with("._"))
            .collect();
        names.sort();
        names
    }

    /// The hidden names this crate makes that are in the folder.
    fn hidden(&self) -> Vec<String> {
        let ours = |name: &String| name.starts_with(".photo-export-");
        self.names().into_iter().filter(ours).collect()
    }

    /// The visible names in the folder that hold exactly `bytes`.
    fn holding(&self, bytes: &[u8]) -> Vec<String> {
        let holds =
            |name: &String| !name.starts_with('.') && self.read(name).as_deref() == Some(bytes);
        self.names().into_iter().filter(holds).collect()
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
        self.place_bytes(b"export", kind, policy, guarded, intruder)
    }

    /// [`Self::place`] for an export that holds `bytes`.
    fn place_bytes(
        &self,
        bytes: &[u8],
        kind: Kind,
        policy: ExistingFile,
        guarded: &Guarded,
        intruder: &mut dyn FnMut(usize, Step),
    ) -> (Result<Placed, ExportError>, Vec<Step>) {
        let mut disk = Staged::new(kind, intruder);
        let result = self.place_on(&mut disk, bytes, policy, guarded);
        (result, disk.steps)
    }

    /// Places a finished export that holds `bytes` as `x.jpg` under
    /// `policy` on `disk`.
    fn place_on(
        &self,
        disk: &mut Staged,
        bytes: &[u8],
        policy: ExistingFile,
        guarded: &Guarded,
    ) -> Result<Placed, ExportError> {
        let mut finished = TempFile::create(self.0.path()).unwrap();
        finished.file().write_all(bytes).unwrap();
        let plan = NamePlan {
            folder: self.0.path().to_path_buf(),
            stem: "x".into(),
            extension: "jpg".into(),
        };
        finished.place_on(disk, &plan, 1, policy, guarded)
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
        self.place_failing_with(kind, policy, guarded, |_| {}, when, act)
    }

    /// [`Self::place_with`] on a stand-in that `fail` first makes fail in
    /// some way (no room for a copy, say).
    fn place_failing_with(
        &self,
        kind: Kind,
        policy: ExistingFile,
        guarded: &Guarded,
        fail: impl FnOnce(&mut Staged),
        when: Option<usize>,
        act: &mut dyn FnMut(),
    ) -> (Result<Placed, ExportError>, Vec<Step>) {
        let mut intruder = |index: usize, _: Step| {
            if Some(index) == when {
                act();
            }
        };
        let mut disk = Staged::new(kind, &mut intruder);
        fail(&mut disk);
        let result = self.place_on(&mut disk, b"export", policy, guarded);
        (result, disk.steps)
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
    for kind in kinds() {
        assert!(scene(kind, None) > 0, "{kind:?}: no step was reached");
        let mut when = 0;
        while when < scene(kind, Some(when)) {
            when += 1;
        }
    }
}

/// True when the two paths lead to one file.
fn same_file(a: &Path, b: &Path) -> bool {
    Held::open(a).unwrap().is_at(b).unwrap()
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
    if volume().links {
        fs::hard_link(&original, folder.at("link.tif")).unwrap();
        assert_eq!(
            guarded.guards(&folder.at("link.tif")).unwrap(),
            Some(Guard::Source)
        );
    }
    // Moved: its old name no longer leads to it, its identity still does.
    fs::rename(&original, folder.at("moved.tif")).unwrap();
    assert_eq!(
        guarded.guards(&folder.at("moved.tif")).unwrap(),
        Some(Guard::Source)
    );
    // Cut to nothing, written again and moved once more. On exFAT that
    // gives the file another number; it is the original all the same.
    fs::write(folder.at("moved.tif"), b"written again").unwrap();
    fs::rename(folder.at("moved.tif"), folder.at("again.tif")).unwrap();
    assert_eq!(
        guarded.guards(&folder.at("again.tif")).unwrap(),
        Some(Guard::Source)
    );
    assert_eq!(guarded.guards(&other).unwrap(), None);
    // A path that led nowhere at the start is looked up again.
    let late = Guarded::capture(&folder.at("late.tif"), std::slice::from_ref(&other));
    assert_eq!(late.guards(&other).unwrap(), Some(Guard::Protected));
    folder.put("late.tif", b"late");
    assert_eq!(
        late.guards(&folder.at("late.tif")).unwrap(),
        Some(Guard::Source)
    );
}
