//! An export never touches the original, never chooses it as the
//! destination, and leaves nothing behind when it fails or is cancelled.

use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use photo_develop::CancelToken;
use photo_export::{
    BitDepth, ExistingFile, ExportError, ExportOutcome, Folder, Format, export, planned_path,
};

use crate::common::{Fixture, linear_source, names, written};

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
    fs::hard_link(&fixture.original, fixture.out_dir().join("linked.tif")).unwrap();
    settings.naming.template = "linked".into();
    let result = export(&fixture.item(), &settings, &cancel);
    assert!(
        matches!(result, Err(ExportError::WouldOverwriteSource(_))),
        "{result:?}"
    );

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
