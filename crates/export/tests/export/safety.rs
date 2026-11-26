//! An export never touches the original, never chooses it as the
//! destination, and leaves nothing behind when it fails or is cancelled.

use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use photo_develop::CancelToken;
use photo_export::{
    BitDepth, ExistingFile, ExportError, ExportOutcome, Folder, Format, export, planned_path,
};

use crate::common::{Fixture, linear_source, names, second_name, written};

/// Runs `action` from another thread as soon as an export's temporary file
/// appears in `folder`; true when it ran.
fn when_writing_starts(
    folder: std::path::PathBuf,
    action: impl FnOnce() + Send + 'static,
) -> thread::JoinHandle<bool> {
    thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(120);
        while Instant::now() < deadline {
            let partial = fs::read_dir(&folder)
                .map(|entries| {
                    entries
                        .flatten()
                        .any(|entry| entry.file_name().to_string_lossy().ends_with(".part"))
                })
                .unwrap_or(false);
            if partial {
                action();
                return true;
            }
            thread::sleep(Duration::from_millis(1));
        }
        false
    })
}

/// Review finding E1: an original moved onto the destination while the
/// export renders is still the original, and is not replaced.
#[test]
fn an_original_moved_onto_the_destination_during_the_export_survives() {
    let mut fixture = Fixture::named("a.tif", 64, 48);
    let before = fs::read(&fixture.original).unwrap();
    fixture.source = linear_source(3000, 2000, |x, y| {
        [(x % 97) as f32 / 97.0, (y % 89) as f32 / 89.0, 0.5]
    });
    let mut settings = fixture.settings_out();
    settings.location.folder = Folder::SameAsOriginal;
    settings.location.existing = ExistingFile::Overwrite;
    settings.naming.template = "b".into();
    settings.format = Format::Tiff {
        depth: BitDepth::Sixteen,
    };
    let destination = fixture.original.with_file_name("b.tif");
    let mover = {
        let (from, to) = (fixture.original.clone(), destination.clone());
        when_writing_starts(fixture.dir.path().to_path_buf(), move || {
            fs::rename(from, to).unwrap()
        })
    };
    let result = export(&fixture.item(), &settings, &CancelToken::new());
    assert!(mover.join().unwrap(), "the temporary file was never seen");
    assert!(
        matches!(&result, Err(ExportError::WouldOverwriteSource(path)) if *path == destination),
        "{result:?}"
    );
    assert_eq!(
        fs::read(&destination).unwrap(),
        before,
        "the original survives"
    );
    assert_eq!(names(fixture.dir.path()), ["b.tif"]);
}

/// Review findings F29 and F33 through the public API: a protected file is
/// moved onto the destination while the export renders, after the check
/// made before rendering. It is recognised when the export comes to place
/// its file, and survives where it is.
#[test]
fn a_protected_file_moved_onto_the_destination_during_the_export_survives() {
    let mut fixture = Fixture::named("a.tif", 64, 48);
    fixture.source = linear_source(3000, 2000, |x, y| {
        [(x % 97) as f32 / 97.0, (y % 89) as f32 / 89.0, 0.5]
    });
    let protected = fixture.dir.path().join("catalogued.tif");
    fs::write(&protected, b"a catalogued original").unwrap();
    fixture.protected = vec![protected.clone()];
    let mut settings = fixture.settings_out();
    settings.location.existing = ExistingFile::Overwrite;
    settings.naming.template = "b".into();
    settings.format = Format::Tiff {
        depth: BitDepth::Sixteen,
    };
    fs::create_dir_all(fixture.out_dir()).unwrap();
    let destination = fixture.out_dir().join("b.tif");
    fs::write(&destination, b"an older export").unwrap();
    let mover = {
        let (from, to) = (protected.clone(), destination.clone());
        when_writing_starts(fixture.out_dir(), move || fs::rename(from, to).unwrap())
    };
    let result = export(&fixture.item(), &settings, &CancelToken::new());
    assert!(mover.join().unwrap(), "the temporary file was never seen");
    assert!(
        matches!(&result, Err(ExportError::WouldOverwriteProtected(path)) if *path == destination),
        "{result:?}"
    );
    assert_eq!(
        fs::read(&destination).unwrap(),
        b"a catalogued original",
        "the protected file survives"
    );
    assert_eq!(names(&fixture.out_dir()), ["b.tif"]);
}

/// Review finding F52: the original, or a protected file, is there but
/// cannot be opened, so its identity cannot be read and nothing could tell
/// it from the file at the destination (it could be moved there while the
/// export renders). Under the overwrite policy the export is refused before
/// anything is rendered: before the original is even decoded. Skip and
/// unique name replace nothing and go on. An original that is simply not
/// there, rendered from memory, is no reason to refuse.
#[cfg(unix)]
#[test]
fn a_guarded_file_whose_identity_cannot_be_read_refuses_an_overwrite_before_rendering() {
    use std::os::unix::fs::PermissionsExt;
    let mut fixture = Fixture::named("a.tif", 64, 48);
    let protected = fixture.dir.path().join("catalogued.tif");
    fs::write(&protected, b"a catalogued original").unwrap();
    fixture.protected = vec![protected.clone()];
    let mut settings = fixture.settings_out();
    settings.naming.template = "b".into();
    settings.format = Format::Tiff {
        depth: BitDepth::Eight,
    };
    fs::create_dir_all(fixture.out_dir()).unwrap();
    let destination = fixture.out_dir().join("b.tif");
    fs::write(&destination, b"an older export").unwrap();
    let cancel = CancelToken::new();
    for unreadable in [fixture.original.clone(), protected.clone()] {
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::File::open(&unreadable).is_ok() {
            // Whoever runs this (root) is stopped by no permission.
            return;
        }
        settings.location.existing = ExistingFile::Overwrite;
        for from_memory in [true, false] {
            let mut item = fixture.item();
            if !from_memory {
                item.source = None;
            }
            let result = export(&item, &settings, &cancel);
            let context = format!("{}, from memory: {from_memory}", unreadable.display());
            assert!(
                matches!(&result, Err(ExportError::CannotIdentify { path, source })
                    if *path == unreadable && source.kind() == std::io::ErrorKind::PermissionDenied),
                "{context}: {result:?}"
            );
            assert_eq!(fs::read(&destination).unwrap(), b"an older export");
            assert_eq!(names(&fixture.out_dir()), ["b.tif"], "{context}");
        }
        settings.location.existing = ExistingFile::Skip;
        let skipped = export(&fixture.item(), &settings, &cancel).unwrap();
        assert_eq!(
            skipped,
            ExportOutcome::Skipped {
                path: destination.clone()
            }
        );
        settings.location.existing = ExistingFile::UniqueName;
        let (path, _, _) = written(&export(&fixture.item(), &settings, &cancel).unwrap());
        assert_eq!(path, fixture.out_dir().join("b-2.tif"));
        fs::remove_file(path).unwrap();
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o644)).unwrap();
    }
    // Not there at all: rendered from memory, and the older export replaced.
    fs::remove_file(&fixture.original).unwrap();
    settings.location.existing = ExistingFile::Overwrite;
    let (path, _, _) = written(&export(&fixture.item(), &settings, &cancel).unwrap());
    assert_eq!(path, destination);
    assert_ne!(fs::read(&destination).unwrap(), b"an older export");
    assert_eq!(names(&fixture.out_dir()), ["b.tif"]);
}

/// The engine names files an export must never replace (catalogued
/// originals). Under the overwrite policy each is refused by identity:
/// under its own name, through a hard link and through a symbolic link.
#[test]
fn a_protected_file_is_never_replaced_whatever_name_reaches_it() {
    let mut fixture = Fixture::named("photo.tif", 64, 48);
    let elsewhere = fixture.dir.path().join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    fs::create_dir_all(fixture.out_dir()).unwrap();
    let protected = elsewhere.join("keep.tif");
    fs::write(&protected, b"a catalogued original").unwrap();
    fixture.protected = vec![protected.clone()];
    let cancel = CancelToken::new();
    let mut settings = fixture.settings_out();
    settings.format = Format::Tiff {
        depth: BitDepth::Eight,
    };
    settings.location.existing = ExistingFile::Overwrite;
    let intact = || assert_eq!(fs::read(&protected).unwrap(), b"a catalogued original");
    let refused = |settings: &photo_export::ExportSettings, destination: &std::path::Path| {
        let result = export(&fixture.item(), settings, &cancel);
        assert!(
            matches!(&result, Err(ExportError::WouldOverwriteProtected(path)) if path == destination),
            "{}: {result:?}",
            destination.display()
        );
        intact();
    };

    // Under its own name.
    settings.location.folder = Folder::Specific(elsewhere.clone());
    settings.naming.template = "keep".into();
    refused(&settings, &protected);

    // Through a hard link in the export folder.
    settings.location.folder = Folder::Specific(fixture.out_dir());
    let linked = fixture.out_dir().join("linked.tif");
    second_name(&protected, &linked);
    settings.naming.template = "linked".into();
    refused(&settings, &linked);

    // Through a symbolic link, which stays a link.
    #[cfg(unix)]
    {
        let symbolic = fixture.out_dir().join("symbolic.tif");
        std::os::unix::fs::symlink(&protected, &symbolic).unwrap();
        settings.naming.template = "symbolic".into();
        refused(&settings, &symbolic);
        assert!(fs::symlink_metadata(&symbolic).unwrap().is_symlink());
        fs::remove_file(&symbolic).unwrap();
    }

    // The other policies never replace anything, protected or not.
    settings.naming.template = "linked".into();
    settings.location.existing = ExistingFile::Skip;
    let skipped = export(&fixture.item(), &settings, &cancel).unwrap();
    assert_eq!(
        skipped,
        ExportOutcome::Skipped {
            path: linked.clone()
        }
    );
    settings.location.existing = ExistingFile::UniqueName;
    let (path, _, _) = written(&export(&fixture.item(), &settings, &cancel).unwrap());
    assert_eq!(path, fixture.out_dir().join("linked-2.tif"));
    intact();

    // A file that is not protected is still replaced.
    let plain = fixture.out_dir().join("plain.tif");
    fs::write(&plain, b"an older export").unwrap();
    settings.naming.template = "plain".into();
    settings.location.existing = ExistingFile::Overwrite;
    let (path, _, _) = written(&export(&fixture.item(), &settings, &cancel).unwrap());
    assert_eq!(path, plain);
    assert_ne!(fs::read(&plain).unwrap(), b"an older export");
    intact();
    assert_eq!(
        names(&fixture.out_dir()),
        ["linked-2.tif", "linked.tif", "plain.tif"]
    );
}

#[test]
fn the_original_is_never_the_file_written_under_any_policy() {
    let fixture = Fixture::named("photo.tif", 64, 48);
    let before = fs::read(&fixture.original).unwrap();
    let mut settings = fixture.settings_out();
    settings.location.folder = Folder::SameAsOriginal;
    settings.format = Format::Tiff {
        depth: BitDepth::Sixteen,
    };
    let cancel = CancelToken::new();

    settings.location.existing = ExistingFile::Overwrite;
    assert_eq!(planned_path(&fixture.item(), &settings), fixture.original);
    let refused = export(&fixture.item(), &settings, &cancel);
    assert!(
        matches!(&refused, Err(ExportError::WouldOverwriteSource(path)) if *path == fixture.original),
        "{refused:?}"
    );

    settings.location.existing = ExistingFile::Skip;
    let skipped = export(&fixture.item(), &settings, &cancel).unwrap();
    assert_eq!(
        skipped,
        ExportOutcome::Skipped {
            path: fixture.original.clone()
        }
    );

    settings.location.existing = ExistingFile::UniqueName;
    let (path, _, _) = written(&export(&fixture.item(), &settings, &cancel).unwrap());
    assert_eq!(path, fixture.original.with_file_name("photo-2.tif"));

    assert_eq!(
        fs::read(&fixture.original).unwrap(),
        before,
        "the original is unchanged"
    );
    assert_eq!(names(fixture.dir.path()), ["photo-2.tif", "photo.tif"]);
}

#[test]
fn another_name_for_the_original_is_still_the_original() {
    let fixture = Fixture::named("photo.tif", 64, 48);
    let before = fs::read(&fixture.original).unwrap();
    let mut settings = fixture.settings_out();
    settings.format = Format::Tiff {
        depth: BitDepth::Eight,
    };
    settings.location.existing = ExistingFile::Overwrite;
    let cancel = CancelToken::new();

    // A hard link elsewhere.
    fs::create_dir_all(fixture.out_dir()).unwrap();
    second_name(&fixture.original, &fixture.out_dir().join("linked.tif"));
    settings.naming.template = "linked".into();
    let result = export(&fixture.item(), &settings, &cancel);
    assert!(
        matches!(result, Err(ExportError::WouldOverwriteSource(_))),
        "{result:?}"
    );

    // The other two policies never replace anything: the link is left
    // alone, or the export takes the next name.
    let linked = fixture.out_dir().join("linked.tif");
    settings.location.existing = ExistingFile::Skip;
    let skipped = export(&fixture.item(), &settings, &cancel).unwrap();
    assert_eq!(
        skipped,
        ExportOutcome::Skipped {
            path: linked.clone()
        }
    );
    settings.location.existing = ExistingFile::UniqueName;
    let (path, _, _) = written(&export(&fixture.item(), &settings, &cancel).unwrap());
    assert_eq!(path, fixture.out_dir().join("linked-2.tif"));
    assert_eq!(fs::read(&linked).unwrap(), before);
    settings.location.existing = ExistingFile::Overwrite;

    // The same name in another case: the original on a file system that
    // ignores case, a different file on one that does not.
    settings.location.folder = Folder::SameAsOriginal;
    settings.naming.template = "PHOTO".into();
    match export(&fixture.item(), &settings, &cancel) {
        Err(ExportError::WouldOverwriteSource(_)) => {}
        Ok(ExportOutcome::Written { path, .. }) => {
            assert_ne!(fs::read(&path).unwrap(), before);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        fs::read(&fixture.original).unwrap(),
        before,
        "the original is unchanged"
    );
}

#[test]
fn exporting_beside_the_original_leaves_it_byte_for_byte_alone() {
    let fixture = Fixture::named("holiday.jpg", 96, 64);
    let before = fs::read(&fixture.original).unwrap();
    let mut item = fixture.item();
    item.source = None;
    let mut settings = fixture.settings_out();
    settings.location.folder = Folder::SameAsOriginal;
    for format in [
        Format::Jpeg { quality: 70 },
        Format::Png {
            depth: BitDepth::Eight,
        },
        Format::Tiff {
            depth: BitDepth::Sixteen,
        },
    ] {
        settings.format = format;
        written(&export(&item, &settings, &CancelToken::new()).unwrap());
    }
    assert_eq!(fs::read(&fixture.original).unwrap(), before);
    assert_eq!(
        names(fixture.dir.path()),
        ["holiday-2.jpg", "holiday.jpg", "holiday.png", "holiday.tif"]
    );
}

#[test]
fn a_cancelled_export_leaves_no_file_and_no_temporary_file() {
    let mut fixture = Fixture::new(32, 32);
    let cancel = CancelToken::new();
    cancel.cancel();
    let result = export(&fixture.item(), &fixture.settings_out(), &cancel);
    assert!(matches!(result, Err(ExportError::Cancelled)), "{result:?}");
    assert!(!fixture.out_dir().exists());

    // Cancelled while the file is being written: a watcher cancels as soon
    // as the temporary file appears.
    fixture.source = linear_source(4000, 2400, |x, y| {
        [(x % 97) as f32 / 97.0, (y % 89) as f32 / 89.0, 0.5]
    });
    let mut settings = fixture.settings_out();
    settings.format = Format::Tiff {
        depth: BitDepth::Sixteen,
    };
    settings.sharpening = photo_export::OutputSharpening::Standard;
    let out = fixture.out_dir();
    let cancel = CancelToken::new();
    let watcher = {
        let cancel = cancel.clone();
        when_writing_starts(out.clone(), move || cancel.cancel())
    };
    let result = export(&fixture.item(), &settings, &cancel);
    assert!(watcher.join().unwrap(), "the temporary file was never seen");
    assert!(matches!(result, Err(ExportError::Cancelled)), "{result:?}");
    assert!(names(&out).is_empty(), "left behind: {:?}", names(&out));
}

#[test]
fn a_failed_export_leaves_no_file_and_no_temporary_file() {
    // Metadata too large for a JPEG is found while writing.
    let mut fixture = Fixture::new(32, 32);
    fixture.attributes.caption = Some("x".repeat(70_000));
    let mut settings = fixture.settings_out();
    let result = export(&fixture.item(), &settings, &CancelToken::new());
    assert!(
        matches!(result, Err(ExportError::Unsupported(_))),
        "{result:?}"
    );
    assert!(
        names(&fixture.out_dir()).is_empty(),
        "{:?}",
        names(&fixture.out_dir())
    );

    // An original that cannot be decoded.
    let broken = fixture.dir.path().join("broken.jpg");
    fs::write(&broken, b"\xff\xd8 not really a JPEG").unwrap();
    let mut item = fixture.item();
    item.source_path = &broken;
    item.source = None;
    let result = export(&item, &settings, &CancelToken::new());
    assert!(matches!(result, Err(ExportError::Decode(_))), "{result:?}");

    // A destination that cannot be a folder.
    let blocker = fixture.dir.path().join("blocker");
    fs::write(&blocker, b"a file, not a folder").unwrap();
    settings.location.folder = Folder::Specific(blocker.join("inside"));
    let result = export(&fixture.item(), &settings, &CancelToken::new());
    assert!(matches!(result, Err(ExportError::Io { .. })), "{result:?}");
    assert_eq!(
        names(fixture.dir.path()),
        ["blocker", "broken.jpg", "original.png", "out"]
    );
    assert!(names(&fixture.out_dir()).is_empty());
}
