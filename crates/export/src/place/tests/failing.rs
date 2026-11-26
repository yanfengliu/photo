//! Placement when something fails on the way. An identity that cannot be
//! read is an error, never "another file has the name" (review F59).

use super::{Folder, Kind, Staged, kinds, nothing_guarded};
use crate::item::ExportError;
use crate::place::Placed;
use crate::place::disk::Step;
use crate::settings::ExistingFile;

/// How many names one placement created to copy into: one per name tried,
/// on a file system where the export is copied to its name.
fn created(steps: &[Step]) -> usize {
    steps.iter().filter(|step| **step == Step::Create).count()
}

/// True when `result` is an I/O failure at `x.jpg`.
fn failed_at_x(folder: &Folder, result: &Result<Placed, ExportError>) -> bool {
    matches!(result, Err(ExportError::Io { path, .. }) if *path == folder.at("x.jpg"))
}

/// Review F59: the identity read that says whether the name still leads to
/// the copy the export made fails once (no descriptor is left, say). That
/// is an error: under skip it is not "skipped", under unique name not
/// "taken, try the next", and the copy is given back, so nothing of the
/// export is left behind. Under overwrite the previous file is back.
#[test]
fn an_identity_that_cannot_be_read_is_an_error_not_a_taken_name() {
    let policies = [
        ExistingFile::Skip,
        ExistingFile::UniqueName,
        ExistingFile::Overwrite,
    ];
    for policy in policies {
        for kind in kinds() {
            let folder = Folder::new();
            if policy == ExistingFile::Overwrite {
                folder.put("x.jpg", b"previous");
            }
            let mut quiet = |_: usize, _: Step| {};
            let mut disk = Staged::new(kind, &mut quiet);
            disk.identity_fails = |read| read == 0;
            let result = folder.place_on(&mut disk, b"export", policy, &nothing_guarded(&folder));
            let context = format!("{policy:?} on {kind:?}, {:?}", disk.steps);
            if kind != Kind::Neither {
                // Moved to its name, not copied: no identity is read.
                assert_eq!(result.unwrap(), Placed::At(folder.at("x.jpg")), "{context}");
                continue;
            }
            assert!(failed_at_x(&folder, &result), "{context}: {result:?}");
            assert!(
                folder.hidden().is_empty(),
                "{context}: {:?}",
                folder.names()
            );
            if policy == ExistingFile::Overwrite {
                assert_eq!(folder.names(), ["x.jpg"], "{context}");
                assert_eq!(folder.read("x.jpg").unwrap(), b"previous", "{context}");
            } else {
                assert!(folder.names().is_empty(), "{context}: {:?}", folder.names());
                assert_eq!(created(&disk.steps), 1, "{context}: another name was tried");
            }
        }
    }
}

/// No identity can be read at all. The copy cannot be shown to be the
/// export's, so it is not removed (it could be a file another program put
/// under the name): it is left under that name and the export fails there,
/// at its first name, leaving nothing under a hidden name.
#[test]
fn when_no_identity_can_be_read_the_export_fails_at_its_first_name() {
    for policy in [ExistingFile::Skip, ExistingFile::UniqueName] {
        for kind in kinds() {
            let folder = Folder::new();
            let mut quiet = |_: usize, _: Step| {};
            let mut disk = Staged::new(kind, &mut quiet);
            disk.identity_fails = |_| true;
            let result = folder.place_on(&mut disk, b"export", policy, &nothing_guarded(&folder));
            let context = format!("{policy:?} on {kind:?}, {:?}", disk.steps);
            if kind != Kind::Neither {
                assert_eq!(result.unwrap(), Placed::At(folder.at("x.jpg")), "{context}");
                continue;
            }
            assert!(failed_at_x(&folder, &result), "{context}: {result:?}");
            assert_eq!(folder.names(), ["x.jpg"], "{context}");
            assert_eq!(created(&disk.steps), 1, "{context}: another name was tried");
        }
    }
}
