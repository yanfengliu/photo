//! The overwrite policy: the export replaces what has its name, but the
//! original and the protected files survive whatever another program does
//! between any two steps, and nothing is lost when the export fails (review
//! F29, F33).

use std::fs::{self, OpenOptions};
use std::io::{self, Write};

use super::{Folder, KINDS, Kind, at_every_step, same_file};
use crate::item::ExportError;
use crate::place::Placed;
use crate::place::disk::Step;
use crate::place::guard::{Guard, Guarded};
use crate::place::overwrite::ROUNDS;
use crate::settings::ExistingFile::Overwrite;

/// A folder with an earlier export under the name `x.jpg`, the original
/// `a.tif` and a protected `p.tif`, and the guard over those two.
fn scene() -> (Folder, Guarded) {
    let folder = Folder::new();
    folder.put("x.jpg", b"previous");
    let original = folder.put("a.tif", b"the original");
    let protected = folder.put("p.tif", b"protected");
    let guarded = Guarded::capture(&original, &[protected]);
    (folder, guarded)
}

/// The visible names in `folder` that hold exactly `bytes`.
fn holding(folder: &Folder, bytes: &[u8]) -> Vec<String> {
    let holds =
        |name: &String| !name.starts_with('.') && folder.read(name).as_deref() == Some(bytes);
    folder.names().into_iter().filter(holds).collect()
}

#[test]
fn undisturbed_the_export_replaces_what_had_the_name() {
    for kind in KINDS {
        let (folder, guarded) = scene();
        let (result, steps) = folder.place(kind, Overwrite, &guarded, &mut |_, _| {});
        assert_eq!(result.unwrap(), Placed::At(folder.at("x.jpg")), "{kind:?}");
        assert_eq!(folder.read("x.jpg").unwrap(), b"export", "{kind:?}");
        assert_eq!(folder.names(), ["a.tif", "p.tif", "x.jpg"], "{kind:?}");
        assert!(steps.contains(&Step::SetAside) && steps.contains(&Step::Discard));
    }
}

/// Another program moves the original, or a protected file, onto the
/// export's name: after the early check, after the previous file has been
/// moved aside, while the export is taking the name, or after.
#[test]
fn the_original_or_a_protected_file_moved_onto_the_name_at_any_step_survives() {
    let cases = [
        ("a.tif", &b"the original"[..], Guard::Source),
        ("p.tif", &b"protected"[..], Guard::Protected),
    ];
    for (moved, bytes, guard) in cases {
        at_every_step(|kind, when| {
            let (folder, guarded) = scene();
            let name = folder.at("x.jpg");
            let mut acted = false;
            let (result, steps) = folder.place_with(kind, Overwrite, &guarded, when, &mut || {
                fs::rename(folder.at(moved), &name).unwrap();
                acted = true;
            });
            let context = format!("{kind:?}, {moved} moved onto the name before step {when:?}");
            if acted {
                assert_eq!(
                    holding(&folder, bytes),
                    ["x.jpg"],
                    "{context}: it is not whole under its name: {:?}",
                    folder.names()
                );
                match (&result, guard) {
                    // It arrived after the export had the name and replaced
                    // the export, not the other way round.
                    (Ok(placed), _) => assert_eq!(*placed, Placed::At(name), "{context}"),
                    (Err(ExportError::WouldOverwriteSource(path)), Guard::Source)
                    | (Err(ExportError::WouldOverwriteProtected(path)), Guard::Protected) => {
                        assert_eq!(*path, name, "{context}");
                    }
                    (Err(error), _) => panic!("{context}: {error}"),
                }
            } else {
                assert_eq!(result.unwrap(), Placed::At(name), "{context}");
                assert_eq!(folder.read("x.jpg").unwrap(), b"export", "{context}");
            }
            assert!(
                folder.hidden().is_empty(),
                "{context}: {:?}",
                folder.names()
            );
            steps.len()
        });
    }
}

/// Another program creates a new file with the export's name at the one
/// time it can: after the previous file was moved aside. It is not renamed
/// over unseen: it is moved aside and identified like the first.
#[test]
fn a_new_file_at_the_name_is_identified_before_the_export_takes_its_place() {
    at_every_step(|kind, when| {
        let (folder, guarded) = scene();
        let name = folder.at("x.jpg");
        let mut appeared = false;
        let (result, steps) = folder.place_with(kind, Overwrite, &guarded, when, &mut || {
            let created = OpenOptions::new().write(true).create_new(true).open(&name);
            appeared = created
                .and_then(|mut file| file.write_all(b"theirs"))
                .is_ok();
        });
        let context = format!("{kind:?}, a file appearing before step {when:?}");
        assert_eq!(result.unwrap(), Placed::At(name), "{context}");
        assert_eq!(folder.read("x.jpg").unwrap(), b"export", "{context}");
        assert_eq!(folder.names(), ["a.tif", "p.tif", "x.jpg"], "{context}");
        let identified = steps.iter().filter(|step| **step == Step::Identify).count();
        assert_eq!(
            identified,
            1 + usize::from(appeared),
            "{context}: {steps:?}"
        );
        steps.len()
    });
}

/// At no step is the previous file gone before the export has the name.
#[test]
fn what_was_moved_aside_is_removed_only_once_the_export_has_the_name() {
    for kind in KINDS {
        let (folder, guarded) = scene();
        let mut discarding = false;
        let mut intruder = |index: usize, step: Step| {
            let previous = folder
                .names()
                .iter()
                .any(|name| folder.read(name).as_deref() == Some(b"previous"));
            assert!(
                previous,
                "{kind:?}: before step {index} ({step:?}) it is gone"
            );
            if step == Step::Discard {
                assert_eq!(folder.read("x.jpg").unwrap(), b"export", "{kind:?}");
                discarding = true;
            }
        };
        let (result, _) = folder.place(kind, Overwrite, &guarded, &mut intruder);
        result.unwrap();
        assert!(discarding, "{kind:?}");
        assert!(holding(&folder, b"previous").is_empty() && folder.hidden().is_empty());
    }
}

/// The name leads to a protected file through a hard link or a symbolic
/// link: it is put back as it was and the export is refused.
#[test]
fn a_protected_file_reached_through_a_link_is_put_back_and_refused() {
    for kind in KINDS {
        for symbolic in [false, true] {
            // File systems without either move have no symbolic links, and
            // one could not be put back there by copying.
            if symbolic && (cfg!(not(unix)) || kind == Kind::Neither) {
                continue;
            }
            let (folder, guarded) = scene();
            let name = folder.at("x.jpg");
            fs::remove_file(&name).unwrap();
            if symbolic {
                #[cfg(unix)]
                std::os::unix::fs::symlink(folder.at("p.tif"), &name).unwrap();
            } else {
                fs::hard_link(folder.at("p.tif"), &name).unwrap();
            }
            let (result, _) = folder.place(kind, Overwrite, &guarded, &mut |_, _| {});
            let context = format!("{kind:?}, symbolic link: {symbolic}");
            assert!(
                matches!(&result, Err(ExportError::WouldOverwriteProtected(path)) if *path == name),
                "{context}: {result:?}"
            );
            // Where a file can only be put back by copying it there are no
            // hard links to begin with; the bytes are what comes back.
            if kind != Kind::Neither {
                assert!(same_file(&name, &folder.at("p.tif")), "{context}");
            }
            assert_eq!(folder.read("x.jpg").unwrap(), b"protected", "{context}");
            let link = fs::symlink_metadata(&name).unwrap().is_symlink();
            assert_eq!(link, symbolic, "{context}");
            assert_eq!(folder.read("p.tif").unwrap(), b"protected", "{context}");
            assert_eq!(folder.names(), ["a.tif", "p.tif", "x.jpg"], "{context}");
        }
    }
}

/// The original is moved aside to be identified, and before it can go back
/// another program takes its name: it gets a visible name beside it, which
/// the refusal gives, and the other program's file is left alone.
#[test]
fn a_guarded_file_whose_name_is_taken_again_gets_a_visible_name_beside_it() {
    for kind in KINDS {
        let (folder, guarded) = scene();
        let name = folder.at("x.jpg");
        fs::rename(folder.at("a.tif"), &name).unwrap();
        let mut intruder = |_: usize, step: Step| {
            if step == Step::PutBack && !name.exists() {
                folder.put("x.jpg", b"theirs");
            }
        };
        let (result, _) = folder.place(kind, Overwrite, &guarded, &mut intruder);
        let kept = folder.at("x-kept.jpg");
        assert!(
            matches!(&result, Err(ExportError::WouldOverwriteSource(path)) if *path == kept),
            "{kind:?}: {result:?}"
        );
        assert_eq!(
            folder.read("x-kept.jpg").unwrap(),
            b"the original",
            "{kind:?}"
        );
        assert_eq!(folder.read("x.jpg").unwrap(), b"theirs", "{kind:?}");
        assert_eq!(folder.names(), ["p.tif", "x-kept.jpg", "x.jpg"], "{kind:?}");
    }
}

#[test]
fn a_folder_with_the_name_is_never_moved_or_replaced() {
    for kind in KINDS {
        let (folder, guarded) = scene();
        let name = folder.at("x.jpg");
        fs::remove_file(&name).unwrap();
        fs::create_dir(&name).unwrap();
        fs::write(name.join("inside"), b"kept").unwrap();
        let (result, steps) = folder.place(kind, Overwrite, &guarded, &mut |_, _| {});
        assert!(
            matches!(&result, Err(ExportError::Io { source, .. }) if source.kind() == io::ErrorKind::IsADirectory),
            "{kind:?}: {result:?}"
        );
        assert_eq!(fs::read(name.join("inside")).unwrap(), b"kept", "{kind:?}");
        assert!(!steps.contains(&Step::SetAside), "{kind:?}: {steps:?}");
        assert!(folder.hidden().is_empty(), "{kind:?}: {:?}", folder.names());
    }
}

/// Another program puts a new file at the name every time it is free. The
/// export gives up after a bounded number of rounds, and every file it
/// moved aside is back: the latest arrival under the name, the others and
/// the previous file under visible names beside it.
#[test]
fn after_too_many_new_files_the_export_gives_up_and_everything_is_back() {
    for kind in KINDS {
        let (folder, guarded) = scene();
        let name = folder.at("x.jpg");
        let mut arrivals = 0;
        let mut intruder = |_: usize, step: Step| {
            if step == Step::Link && !name.exists() && arrivals < ROUNDS {
                arrivals += 1;
                folder.put("x.jpg", format!("arrival {arrivals}").as_bytes());
            }
        };
        let (result, steps) = folder.place(kind, Overwrite, &guarded, &mut intruder);
        assert!(
            matches!(result, Err(ExportError::Io { .. })),
            "{kind:?}: {result:?}"
        );
        let set_aside = steps.iter().filter(|step| **step == Step::SetAside).count();
        assert_eq!(set_aside as u32, ROUNDS, "{kind:?}");
        assert_eq!(folder.read("x.jpg").unwrap(), b"arrival 4", "{kind:?}");
        for bytes in ["previous", "arrival 1", "arrival 2", "arrival 3"] {
            let holders = holding(&folder, bytes.as_bytes());
            assert_eq!(holders.len(), 1, "{kind:?}: {bytes}: {:?}", folder.names());
            assert!(holders[0].starts_with("x-kept"), "{kind:?}: {holders:?}");
        }
        assert!(holding(&folder, b"export").is_empty(), "{kind:?}");
        assert!(folder.hidden().is_empty(), "{kind:?}: {:?}", folder.names());
    }
}

/// The export fails to take the name at each step (its finished file
/// vanishes): the previous file is back under its name, never lost.
#[test]
fn when_the_export_cannot_take_the_name_the_previous_file_is_back_under_it() {
    at_every_step(|kind, when| {
        let (folder, guarded) = scene();
        let (result, steps) = folder.place_with(kind, Overwrite, &guarded, when, &mut || {
            if let Some(finished) = folder.temporary() {
                fs::remove_file(finished).unwrap();
            }
        });
        let context = format!("{kind:?}, the finished file vanishing before step {when:?}");
        let expected: &[u8] = if result.is_ok() {
            b"export"
        } else {
            b"previous"
        };
        assert_eq!(folder.read("x.jpg").unwrap(), expected, "{context}");
        assert_eq!(folder.names(), ["a.tif", "p.tif", "x.jpg"], "{context}");
        steps.len()
    });
}

/// Review F33: a protected file's folder is away when the export starts,
/// so its path leads nowhere, and comes back at some step. Whenever the
/// path leads to it by the time the file moved aside is identified, it is
/// refused; the file is the one at the export's name, reached by the
/// export through another path.
#[cfg(unix)]
#[test]
fn a_protected_file_whose_folder_comes_back_is_recognised_at_the_replacement() {
    at_every_step(|kind, when| {
        let folder = Folder::new();
        let name = folder.put("x.jpg", b"a catalogued original");
        let elsewhere = tempfile::tempdir().unwrap();
        let alias = elsewhere.path().join("alias");
        let guarded = Guarded::capture(&folder.at("no-original.tif"), &[alias.join("x.jpg")]);
        let mut back = false;
        let (result, steps) = folder.place_with(kind, Overwrite, &guarded, when, &mut || {
            std::os::unix::fs::symlink(folder.at(""), &alias).unwrap();
            back = true;
        });
        let context = format!("{kind:?}, the folder coming back before step {when:?}");
        let identified = steps.iter().position(|step| *step == Step::Identify);
        let in_time = back && when.is_some_and(|when| identified.is_none_or(|at| when <= at));
        if in_time {
            assert!(
                matches!(&result, Err(ExportError::WouldOverwriteProtected(path)) if *path == name),
                "{context}: {result:?}"
            );
            assert_eq!(
                folder.read("x.jpg").unwrap(),
                b"a catalogued original",
                "{context}"
            );
        } else {
            // Its path led nowhere until after it had been replaced.
            assert_eq!(result.unwrap(), Placed::At(name), "{context}");
            assert_eq!(folder.read("x.jpg").unwrap(), b"export", "{context}");
        }
        assert_eq!(folder.names(), ["x.jpg"], "{context}");
        steps.len()
    });
}
