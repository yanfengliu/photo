//! The overwrite policy: the export replaces what has its name, but the
//! original and the protected files survive whatever another program does
//! between any two steps, and nothing is lost when the export fails (review
//! F29, F33).

use std::fs::{self, OpenOptions};
use std::io::{self, Write};

use super::{Folder, Kind, Staged, at_every_step, kinds, same_file, volume};
use crate::item::ExportError;
use crate::place::Placed;
use crate::place::disk::Step;
use crate::place::guard::{Guard, Guarded};
use crate::place::held::Held;
use crate::place::moves::EXCLUSIVE_ASIDE;
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
    for kind in kinds() {
        let (folder, guarded) = scene();
        let (result, steps) = folder.place(kind, Overwrite, &guarded, &mut |_, _| {});
        assert_eq!(result.unwrap(), Placed::At(folder.at("x.jpg")), "{kind:?}");
        assert_eq!(folder.read("x.jpg").unwrap(), b"export", "{kind:?}");
        assert_eq!(folder.names(), ["a.tif", "p.tif", "x.jpg"], "{kind:?}");
        assert!(steps.contains(&Step::SetAside) && steps.contains(&Step::Discard));
    }
}

/// Review finding 6: two exporters on two machines can come to use one
/// hidden name in a shared folder. Where a rename that cannot replace is
/// used for moving aside, a hidden name that is taken is refused and
/// another one used: the other exporter's file under it is not replaced.
#[test]
fn a_hidden_name_another_exporter_has_is_not_replaced() {
    for kind in kinds() {
        if !(EXCLUSIVE_ASIDE && matches!(kind, Kind::Both | Kind::ExclusiveOnly)) {
            continue;
        }
        let (folder, guarded) = scene();
        // Another exporter's file, moved aside under the very name this
        // export is about to use.
        let theirs = folder.put(".photo-export-7-00000000000000aa.previous", b"theirs");
        let mut quiet = |_: usize, _: Step| {};
        let mut disk = Staged::new(kind, &mut quiet);
        disk.hidden = vec![theirs.clone()];
        let result = folder.place_on(&mut disk, b"export", Overwrite, &guarded);
        assert_eq!(result.unwrap(), Placed::At(folder.at("x.jpg")), "{kind:?}");
        assert_eq!(folder.read("x.jpg").unwrap(), b"export", "{kind:?}");
        assert_eq!(
            fs::read(&theirs).ok().as_deref(),
            Some(&b"theirs"[..]),
            "{kind:?}: the other exporter's file was replaced"
        );
        assert_eq!(folder.hidden().len(), 1, "{kind:?}: {:?}", folder.names());
    }
}

/// How a guarded file comes to have the export's name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Arrival {
    /// The file itself is moved there.
    Moved,
    /// A hard link to it is.
    HardLink,
    /// A symbolic link to it is.
    SymbolicLink,
}

impl Arrival {
    /// The ways this platform and this volume have (making a symbolic link
    /// needs a privilege on Windows; FAT and exFAT have no hard links).
    fn all() -> Vec<Self> {
        let mut all = vec![Self::Moved];
        if volume().links {
            all.push(Self::HardLink);
        }
        if cfg!(unix) {
            all.push(Self::SymbolicLink);
        }
        all
    }

    /// Gives the name `x.jpg` to the file under `guarded`, replacing what
    /// has it, in one rename.
    fn arrive(self, folder: &Folder, guarded: &str) {
        let file = folder.at(guarded);
        let link = folder.at("their.link");
        match self {
            Self::Moved => return fs::rename(file, folder.at("x.jpg")).unwrap(),
            Self::HardLink => fs::hard_link(file, &link).unwrap(),
            #[cfg(unix)]
            Self::SymbolicLink => std::os::unix::fs::symlink(file, &link).unwrap(),
            #[cfg(not(unix))]
            Self::SymbolicLink => unreachable!("not in Arrival::all here"),
        }
        fs::rename(link, folder.at("x.jpg")).unwrap();
    }
}

/// Another program gives the export's name to the original, or to a
/// protected file, itself or through a hard or symbolic link: after the
/// early check, after the previous file has been moved aside, while the
/// export is taking the name, or after.
#[test]
fn the_original_or_a_protected_file_moved_onto_the_name_at_any_step_survives() {
    let files = [
        ("a.tif", &b"the original"[..], Guard::Source),
        ("p.tif", &b"protected"[..], Guard::Protected),
    ];
    let cases = files.into_iter().flat_map(|file| {
        Arrival::all()
            .into_iter()
            .map(move |arrival| (file, arrival))
    });
    for ((guarded_name, bytes, guard), arrival) in cases {
        at_every_step(|kind, when| {
            let (folder, guarded) = scene();
            let name = folder.at("x.jpg");
            let mut acted = false;
            let (result, steps) = folder.place_with(kind, Overwrite, &guarded, when, &mut || {
                arrival.arrive(&folder, guarded_name);
                acted = true;
            });
            let context =
                format!("{kind:?}, {guarded_name} {arrival:?} onto the name before step {when:?}");
            if acted {
                // The file is whole under the name, and under its own too
                // when only a link to it arrived.
                let mut names = vec!["x.jpg"];
                if arrival != Arrival::Moved {
                    names.insert(0, guarded_name);
                }
                assert_eq!(
                    holding(&folder, bytes),
                    names,
                    "{context}: it is not whole under its name: {:?}",
                    folder.names()
                );
                // A link is still a link to it, except a hard link put back
                // by copying, which comes back as a file of its own.
                if arrival == Arrival::SymbolicLink
                    || (arrival == Arrival::HardLink && kind != Kind::Neither)
                {
                    assert!(same_file(&name, &folder.at(guarded_name)), "{context}");
                }
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
    for kind in kinds() {
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

/// The export's name leads to a protected file when the export comes to
/// place its file: the file itself got there while the export rendered, or
/// the name is a hard link or a symbolic link to it. The export is refused
/// and what has the name is not even moved aside: on a file system where it
/// could only be put back as a copy, it is still the very same file.
#[test]
fn a_guarded_file_that_has_the_name_is_refused_where_it_is() {
    for kind in kinds() {
        for arrival in Arrival::all() {
            let (folder, guarded) = scene();
            let name = folder.at("x.jpg");
            let before = Held::open(&folder.at("p.tif")).unwrap();
            arrival.arrive(&folder, "p.tif");
            let (result, steps) = folder.place(kind, Overwrite, &guarded, &mut |_, _| {});
            let context = format!("{kind:?}, {arrival:?}");
            assert!(
                matches!(&result, Err(ExportError::WouldOverwriteProtected(path)) if *path == name),
                "{context}: {result:?}"
            );
            assert!(!steps.contains(&Step::SetAside), "{context}: {steps:?}");
            assert!(
                before.is_at(&name).unwrap(),
                "{context}: another file has the name"
            );
            let link = fs::symlink_metadata(&name).unwrap().is_symlink();
            assert_eq!(link, arrival == Arrival::SymbolicLink, "{context}");
            assert_eq!(folder.read("x.jpg").unwrap(), b"protected", "{context}");
            let names: &[&str] = match arrival {
                Arrival::Moved => &["a.tif", "x.jpg"],
                Arrival::HardLink | Arrival::SymbolicLink => &["a.tif", "p.tif", "x.jpg"],
            };
            assert_eq!(folder.names(), names, "{context}");
        }
    }
}

/// The original takes the name after the export's look, so it is moved
/// aside to be identified, and before it can go back another program takes
/// its name: it gets a visible name beside it, which the refusal gives, and
/// the other program's file is left alone.
#[test]
fn a_guarded_file_whose_name_is_taken_again_gets_a_visible_name_beside_it() {
    for kind in kinds() {
        let (folder, guarded) = scene();
        let name = folder.at("x.jpg");
        let mut intruder = |_: usize, step: Step| match step {
            Step::SetAside if folder.at("a.tif").exists() => {
                fs::rename(folder.at("a.tif"), &name).unwrap();
            }
            Step::PutBack if !name.exists() => {
                folder.put("x.jpg", b"theirs");
            }
            _ => {}
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
    for kind in kinds() {
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

/// A file that cannot be opened cannot be identified, so nothing replaces
/// it: the export fails and the file is not moved.
#[cfg(unix)]
#[test]
fn a_file_that_cannot_be_identified_is_refused_where_it_is() {
    use std::os::unix::fs::PermissionsExt;
    for kind in kinds() {
        let (folder, guarded) = scene();
        let name = folder.at("x.jpg");
        fs::set_permissions(&name, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::File::open(&name).is_ok() {
            // Whoever runs this (root) is stopped by no permission.
            return;
        }
        let (result, steps) = folder.place(kind, Overwrite, &guarded, &mut |_, _| {});
        assert!(
            matches!(&result, Err(ExportError::Io { path, source })
                if *path == name && source.kind() == io::ErrorKind::PermissionDenied),
            "{kind:?}: {result:?}"
        );
        assert!(!steps.contains(&Step::SetAside), "{kind:?}: {steps:?}");
        fs::set_permissions(&name, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(folder.read("x.jpg").unwrap(), b"previous", "{kind:?}");
        assert_eq!(folder.names(), ["a.tif", "p.tif", "x.jpg"], "{kind:?}");
    }
}

/// A folder takes the name in the instant between the export's look and its
/// move, so it is the folder that is moved aside. It is kept whole: put back
/// under the name where a folder can be moved without replacing (an
/// exclusive rename), and elsewhere left under the hidden name that the
/// error gives.
#[test]
fn a_folder_that_takes_the_name_at_the_last_instant_is_kept_whole() {
    for kind in kinds() {
        let (folder, guarded) = scene();
        let name = folder.at("x.jpg");
        let mut intruder = |_: usize, step: Step| {
            if step == Step::SetAside && name.is_file() {
                fs::remove_file(&name).unwrap();
                fs::create_dir(&name).unwrap();
                fs::write(name.join("inside"), b"kept").unwrap();
            }
        };
        let (result, _) = folder.place(kind, Overwrite, &guarded, &mut intruder);
        let Err(ExportError::Io { path, source }) = &result else {
            panic!("{kind:?}: {result:?}");
        };
        assert_eq!(source.kind(), io::ErrorKind::IsADirectory, "{kind:?}");
        assert_eq!(fs::read(path.join("inside")).unwrap(), b"kept", "{kind:?}");
        let exclusive = matches!(kind, Kind::Both | Kind::ExclusiveOnly);
        assert_eq!(*path == name, exclusive, "{kind:?}: {}", path.display());
        let hidden = folder.hidden();
        assert_eq!(hidden.is_empty(), exclusive, "{kind:?}: {hidden:?}");
        // The export itself is gone, the original and the protected file are
        // where they were.
        assert_eq!(folder.read("a.tif").unwrap(), b"the original", "{kind:?}");
        assert_eq!(folder.read("p.tif").unwrap(), b"protected", "{kind:?}");
        assert!(holding(&folder, b"export").is_empty(), "{kind:?}");
    }
}

/// Another program puts a new file at the name every time it is free. The
/// export gives up after a bounded number of rounds, and every file it
/// moved aside is back: the latest arrival under the name, the others and
/// the previous file under visible names beside it.
#[test]
fn after_too_many_new_files_the_export_gives_up_and_everything_is_back() {
    for kind in kinds() {
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
