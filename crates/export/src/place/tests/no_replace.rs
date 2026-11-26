//! The skip and unique-name policies: whatever another program does at
//! the name, between any two steps and on every kind of file system, no
//! file the export did not create is replaced or removed (review F30, F46).

use std::fs::{self, OpenOptions};
use std::io::Write;

use super::{Folder, Staged, at_every_step, kinds, nothing_guarded};
use crate::item::ExportError;
use crate::place::disk::Step;
use crate::place::held::Held;
use crate::place::{Placed, TempFile, UNIQUE_TRIES};
use crate::settings::ExistingFile;

const POLICIES: [ExistingFile; 2] = [ExistingFile::Skip, ExistingFile::UniqueName];

/// Another program creates a new file with the export's name whenever the
/// name is free at the moment it tries.
#[test]
fn a_file_that_appears_at_the_name_at_any_step_keeps_its_bytes() {
    for policy in POLICIES {
        at_every_step(|kind, when| {
            let folder = Folder::new();
            let name = folder.at("x.jpg");
            let mut appeared = false;
            let (result, steps) =
                folder.place_with(kind, policy, &nothing_guarded(&folder), when, &mut || {
                    let created = OpenOptions::new().write(true).create_new(true).open(&name);
                    appeared = created
                        .and_then(|mut file| file.write_all(b"theirs"))
                        .is_ok();
                });
            let context = format!("{policy:?} on {kind:?}, a file appearing before step {when:?}");
            let placed = result.unwrap_or_else(|error| panic!("{context}: {error}"));
            if appeared {
                assert_eq!(folder.read("x.jpg").unwrap(), b"theirs", "{context}");
                if policy == ExistingFile::Skip {
                    assert_eq!(placed, Placed::Skipped(name), "{context}");
                    assert_eq!(folder.names(), ["x.jpg"], "{context}");
                } else {
                    assert_eq!(placed, Placed::At(folder.at("x-2.jpg")), "{context}");
                    assert_eq!(folder.read("x-2.jpg").unwrap(), b"export", "{context}");
                    assert_eq!(folder.names(), ["x-2.jpg", "x.jpg"], "{context}");
                }
            } else {
                assert_eq!(placed, Placed::At(name), "{context}");
                assert_eq!(folder.read("x.jpg").unwrap(), b"export", "{context}");
                assert_eq!(folder.names(), ["x.jpg"], "{context}");
            }
            steps.len()
        });
    }
}

/// Another program renames its own file onto the export's name, which
/// replaces whatever has the name at that moment: before the export takes
/// the name, while it is being taken, or after.
#[test]
fn a_file_renamed_onto_the_name_at_any_step_survives() {
    for policy in POLICIES {
        at_every_step(|kind, when| {
            let folder = Folder::new();
            let name = folder.at("x.jpg");
            let mut renamed = false;
            let (result, steps) =
                folder.place_with(kind, policy, &nothing_guarded(&folder), when, &mut || {
                    fs::rename(folder.put("theirs.tmp", b"theirs"), &name).unwrap();
                    renamed = true;
                });
            let context = format!("{policy:?} on {kind:?}, a file renamed on before step {when:?}");
            let placed = result.unwrap_or_else(|error| panic!("{context}: {error}"));
            if renamed {
                assert_eq!(
                    folder.read("x.jpg").unwrap(),
                    b"theirs",
                    "{context}: their file was replaced or removed"
                );
                // Only once the export has the name, and is giving up its
                // old one, can their rename have replaced the export.
                // Before that the export must see the name as taken.
                let expected = match (steps[when.unwrap()], policy) {
                    (Step::Unlink, _) => Placed::At(name),
                    (_, ExistingFile::Skip) => Placed::Skipped(name),
                    _ => Placed::At(folder.at("x-2.jpg")),
                };
                if expected == Placed::At(folder.at("x-2.jpg")) {
                    assert_eq!(folder.read("x-2.jpg").unwrap(), b"export", "{context}");
                }
                assert_eq!(placed, expected, "{context}");
            } else {
                assert_eq!(placed, Placed::At(name), "{context}");
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

/// Placement is made to fail at each step (the finished file vanishes),
/// alone and together with another program renaming its own empty file
/// onto the name. A name the export created and could not fill is given
/// back; a file that is not the one it created never is, however much it
/// looks like it.
#[test]
fn a_failure_gives_back_only_what_the_export_created() {
    for policy in POLICIES {
        for with_their_file in [false, true] {
            at_every_step(|kind, when| {
                let folder = Folder::new();
                let name = folder.at("x.jpg");
                let mut acted = false;
                // Their file, kept open: that is how it is told afterwards
                // from the empty file the export creates.
                let mut theirs = None;
                let (result, steps) =
                    folder.place_with(kind, policy, &nothing_guarded(&folder), when, &mut || {
                        acted = true;
                        if with_their_file {
                            let file = folder.put("theirs.tmp", b"");
                            theirs = Some(Held::open(&file).unwrap());
                            fs::rename(file, &name).unwrap();
                        }
                        if let Some(finished) = folder.temporary() {
                            fs::remove_file(finished).unwrap();
                        }
                    });
                let context = format!(
                    "{policy:?} on {kind:?}, failing before step {when:?}, their file: {with_their_file}"
                );
                if let Some(theirs) = &theirs {
                    assert!(
                        theirs.is_at(&name),
                        "{context}: their file is gone: {:?}",
                        folder.names()
                    );
                } else if let Ok(Placed::At(path)) = &result {
                    assert_eq!(fs::read(path).unwrap(), b"export", "{context}");
                } else {
                    assert!(result.is_err(), "{context}");
                    assert!(folder.names().is_empty(), "{context}: {:?}", folder.names());
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
}

/// Another program takes every name the export goes for, just before the
/// export can have it. The export gives up after a bounded number of names
/// instead of going on without end, and every one of their files is intact.
#[test]
fn after_too_many_taken_names_the_unique_name_policy_gives_up() {
    for kind in kinds() {
        let folder = Folder::new();
        let mut taken = 0;
        let mut intruder = |_: usize, step: Step| {
            // Each try starts here: they take the name it is about to go
            // for. They stop a little after the export should have.
            if step == Step::Link && taken < UNIQUE_TRIES + 5 {
                taken += 1;
                let name = match taken {
                    1 => "x.jpg".to_string(),
                    number => format!("x-{number}.jpg"),
                };
                folder.put(&name, b"theirs");
            }
        };
        let guarded = nothing_guarded(&folder);
        let (result, _) = folder.place(kind, ExistingFile::UniqueName, &guarded, &mut intruder);
        assert!(
            matches!(result, Err(ExportError::Io { .. })),
            "{kind:?}: {result:?}"
        );
        let names = folder.names();
        assert_eq!(names.len() as u32, UNIQUE_TRIES, "{kind:?}: {names:?}");
        for name in &names {
            assert_eq!(folder.read(name).unwrap(), b"theirs", "{kind:?}: {name}");
        }
    }
}

/// Two exports want the same name at once: the second runs to the end
/// between any two steps of the first. Each gets a name of its own.
#[test]
fn two_exports_racing_for_one_name_both_keep_their_file() {
    at_every_step(|kind, when| {
        let folder = Folder::new();
        let guarded = nothing_guarded(&folder);
        let plan = crate::naming::NamePlan {
            folder: folder.at(""),
            stem: "x".into(),
            extension: "jpg".into(),
        };
        let mut rival = Some(TempFile::create(&folder.at("")).unwrap());
        rival.as_mut().unwrap().file().write_all(b"rival").unwrap();
        let mut rival_placed = None;
        let (result, steps) =
            folder.place_with(kind, ExistingFile::UniqueName, &guarded, when, &mut || {
                let mut quiet = |_: usize, _: Step| {};
                let mut disk = Staged {
                    kind,
                    steps: Vec::new(),
                    intruder: &mut quiet,
                };
                let rival = rival.take().unwrap();
                rival_placed =
                    Some(rival.place_on(&mut disk, &plan, 1, ExistingFile::UniqueName, &guarded));
            });
        let context = format!("{kind:?}, the rival export running before step {when:?}");
        drop(rival);
        let Placed::At(ours) = result.unwrap() else {
            panic!("{context}: the export was skipped");
        };
        assert_eq!(fs::read(&ours).unwrap(), b"export", "{context}");
        if let Some(rival) = rival_placed {
            let Placed::At(theirs) = rival.unwrap() else {
                panic!("{context}: the rival was skipped");
            };
            assert_ne!(ours, theirs, "{context}");
            assert_eq!(fs::read(&theirs).unwrap(), b"rival", "{context}");
            assert_eq!(folder.names(), ["x-2.jpg", "x.jpg"], "{context}");
        } else {
            assert_eq!(folder.names(), ["x.jpg"], "{context}");
        }
        steps.len()
    });
}
