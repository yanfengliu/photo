//! Nothing that is not the export's own file is left under a hidden name
//! by a placement that returns. A file that was moved aside and cannot go
//! back under its name is renamed to a visible name beside it, and the
//! error says where (review F56).

use std::fs;

use super::{Folder, Kind, Staged, at_every_step, kinds, nothing_guarded};
use crate::item::ExportError;
use crate::place::Placed;
use crate::place::disk::Step;
use crate::settings::ExistingFile;

/// True when `result` says that a file which had the export's name is
/// now at `path`.
fn says_kept(result: &Result<Placed, ExportError>, path: &std::path::Path) -> bool {
    matches!(result, Err(ExportError::Kept { kept, .. }) if kept.iter().any(|kept| kept == path))
}

/// Review F56, the report's case: on a full volume where the export is
/// copied to its name, the copy fails and its name is given back. Another
/// program renames its own file onto the name at any step, the instant
/// between the give-back's look and its move included, so that it is their
/// file that is moved aside. It is never left under a hidden name: it is
/// back under the name or, when there is no room to copy it back, renamed
/// to a visible name beside it, which the error gives.
#[test]
fn a_file_renamed_onto_the_name_during_a_failed_copy_is_never_left_hidden() {
    for policy in [ExistingFile::Skip, ExistingFile::UniqueName] {
        // Runs in which their file could not go back under the name.
        let mut beside = 0;
        at_every_step(|kind, when| {
            let folder = Folder::new();
            let name = folder.at("x.jpg");
            let mut renamed = false;
            let full = |disk: &mut Staged| disk.full = true;
            let guarded = nothing_guarded(&folder);
            let (result, steps) =
                folder.place_failing_with(kind, policy, &guarded, full, when, &mut || {
                    fs::rename(folder.put("theirs.tmp", b"theirs"), &name).unwrap();
                    renamed = true;
                });
            let context = format!("{policy:?} on {kind:?}, renamed on before step {when:?}");
            let names = folder.names();
            assert!(folder.hidden().is_empty(), "{context}: {names:?}");
            if renamed {
                let theirs = folder.holding(b"theirs");
                assert_eq!(theirs.len(), 1, "{context}: {names:?} {steps:?}");
                let now = folder.at(&theirs[0]);
                if now != name {
                    assert!(says_kept(&result, &now), "{context}: {result:?}");
                    beside += 1;
                }
            }
            if kind == Kind::Neither {
                // No room: nothing of the export can be anywhere.
                assert!(folder.holding(b"export").is_empty(), "{context}: {names:?}");
                assert!(
                    !matches!(result, Ok(Placed::At(_))),
                    "{context}: {result:?}"
                );
            }
            steps.len()
        });
        // Only copying needs room, and copying is staged on every volume.
        assert!(beside > 0, "{policy:?}: the case was never reached");
    }
}

/// Under overwrite on a full volume where the export is copied to its
/// name, the copy fails after the previous file was moved aside, and the
/// previous file cannot be copied back either. It is renamed to a visible
/// name beside the export's, which the error gives.
#[test]
fn on_a_full_volume_the_previous_file_gets_a_visible_name_the_error_gives() {
    for kind in kinds() {
        let folder = Folder::new();
        folder.put("x.jpg", b"previous");
        let full = |disk: &mut Staged| disk.full = true;
        let guarded = nothing_guarded(&folder);
        let (result, _) = folder.place_failing_with(
            kind,
            ExistingFile::Overwrite,
            &guarded,
            full,
            None,
            &mut || {},
        );
        let context = format!("{kind:?}: {:?}", folder.names());
        assert!(folder.hidden().is_empty(), "{context}");
        if kind != Kind::Neither {
            // Moved to its name, not copied: a full volume is no matter.
            assert_eq!(result.unwrap(), Placed::At(folder.at("x.jpg")), "{context}");
            assert_eq!(folder.names(), ["x.jpg"], "{context}");
            continue;
        }
        let kept = folder.at("x-kept.jpg");
        assert!(says_kept(&result, &kept), "{context}: {result:?}");
        let Err(ExportError::Kept { error, .. }) = &result else {
            unreachable!()
        };
        assert!(
            matches!(&**error, ExportError::Io { path, source }
                if *path == folder.at("x.jpg") && source.kind() == std::io::ErrorKind::StorageFull),
            "{context}: {error:?}"
        );
        assert_eq!(folder.names(), ["x-kept.jpg"], "{context}");
        assert_eq!(folder.read("x-kept.jpg").unwrap(), b"previous", "{context}");
    }
}

/// The export has the name, and the file it replaced cannot be removed (a
/// read-only file on Windows, say). That file is not left under its hidden
/// name: it gets a visible one beside the export's, and the export is
/// written all the same.
#[test]
fn a_replaced_file_that_cannot_be_removed_gets_a_visible_name() {
    for kind in kinds() {
        let folder = Folder::new();
        folder.put("x.jpg", b"previous");
        let stuck = |disk: &mut Staged| disk.removal_fails = true;
        let guarded = nothing_guarded(&folder);
        let (result, steps) = folder.place_failing_with(
            kind,
            ExistingFile::Overwrite,
            &guarded,
            stuck,
            None,
            &mut || {},
        );
        let context = format!("{kind:?}: {:?} {steps:?}", folder.names());
        assert_eq!(result.unwrap(), Placed::At(folder.at("x.jpg")), "{context}");
        assert_eq!(folder.read("x.jpg").unwrap(), b"export", "{context}");
        assert_eq!(folder.read("x-kept.jpg").unwrap(), b"previous", "{context}");
        assert_eq!(folder.names(), ["x-kept.jpg", "x.jpg"], "{context}");
        assert!(steps.contains(&Step::Keep), "{context}");
    }
}
