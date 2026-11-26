//! A file whose identity cannot be read is neither taken for no file nor
//! for somebody else's: the export refuses or fails rather than replace
//! it, report it skipped or go on to another name (review F52, F59).

#![cfg(unix)]

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use super::{Folder, at_every_step, kinds};
use crate::item::ExportError;
use crate::place::Placed;
use crate::place::disk::Step;
use crate::place::guard::Guarded;
use crate::settings::ExistingFile::Overwrite;

/// Makes `path` a file that is there but cannot be opened. False when
/// whoever runs the tests (root) is stopped by no permission.
fn unreadable(path: &Path) -> bool {
    fs::write(path, b"protected").unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o000)).unwrap();
    fs::File::open(path).is_err()
}

fn readable_again(path: &Path) {
    fs::set_permissions(path, fs::Permissions::from_mode(0o644)).unwrap();
}

/// True when `result` refuses because the identity of `path` cannot be
/// read.
fn doubted(result: &Result<Placed, ExportError>, path: &Path) -> bool {
    matches!(result, Err(ExportError::CannotIdentify { path: doubted, source })
        if doubted == path && source.kind() == io::ErrorKind::PermissionDenied)
}

/// Review F52 at placement: a protected path that is there but cannot be
/// opened, from the start or only by the time the export places its file,
/// could lead to the very file that has the export's name. That file is
/// neither replaced nor even moved, and the refusal names the protected
/// path.
#[test]
fn a_protected_path_that_cannot_be_opened_is_not_taken_for_no_file() {
    for kind in kinds() {
        for from_the_start in [true, false] {
            let folder = Folder::new();
            folder.put("x.jpg", b"previous");
            let protected = folder.at("p.tif");
            if from_the_start && !unreadable(&protected) {
                return;
            }
            let guarded = Guarded::capture(&folder.at("a.tif"), std::slice::from_ref(&protected));
            if !from_the_start && !unreadable(&protected) {
                return;
            }
            let (result, steps) = folder.place(kind, Overwrite, &guarded, &mut |_, _| {});
            let context = format!("{kind:?}, unreadable from the start: {from_the_start}");
            assert!(doubted(&result, &protected), "{context}: {result:?}");
            assert!(!steps.contains(&Step::SetAside), "{context}: {steps:?}");
            readable_again(&protected);
            assert_eq!(folder.read("x.jpg").unwrap(), b"previous", "{context}");
            assert_eq!(folder.names(), ["p.tif", "x.jpg"], "{context}");
        }
    }
}

/// The protected path begins to lead to a file that cannot be opened at
/// some step. Up to the moment what was moved aside is identified, that
/// file could be it, so nothing is replaced: refused where it is before the
/// move, put back after it. Later it no longer protects anything (a path
/// that begins to lead to a file only after the replacement never does).
#[test]
fn a_protected_path_that_becomes_unreadable_is_doubted_until_the_replacement() {
    at_every_step(|kind, when| {
        let folder = Folder::new();
        folder.put("x.jpg", b"previous");
        let protected = folder.at("p.tif");
        let guarded = Guarded::capture(&folder.at("a.tif"), std::slice::from_ref(&protected));
        let mut locked = None;
        let (result, steps) = folder.place_with(kind, Overwrite, &guarded, when, &mut || {
            locked = Some(unreadable(&protected));
        });
        if locked.is_some() {
            readable_again(&protected);
        }
        if locked == Some(false) {
            // Whoever runs this (root) is stopped by no permission.
            return steps.len();
        }
        let context = format!("{kind:?}, the path unreadable from step {when:?}");
        let identified = steps.iter().position(|step| *step == Step::Identify);
        let in_time =
            locked.is_some() && when.is_some_and(|when| identified.is_none_or(|at| when <= at));
        if in_time {
            assert!(doubted(&result, &protected), "{context}: {result:?}");
            assert_eq!(folder.read("x.jpg").unwrap(), b"previous", "{context}");
        } else {
            assert_eq!(result.unwrap(), Placed::At(folder.at("x.jpg")), "{context}");
            assert_eq!(folder.read("x.jpg").unwrap(), b"export", "{context}");
        }
        let names: &[&str] = if locked.is_some() {
            &["p.tif", "x.jpg"]
        } else {
            &["x.jpg"]
        };
        assert_eq!(folder.names(), names, "{context}");
        steps.len()
    });
}
