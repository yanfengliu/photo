//! Sidecar files on disk: where they live, reading them, and replacing them
//! without ever leaving a damaged one behind.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use photo_core::develop::DevelopSettings;
use photo_core::meta::{ColorLabel, FileKind, Rating};
use photo_xmp::{ImageInfo, XmpData, XmpError, read_sidecar, sidecar_path, write_sidecar};

const RAW: ImageInfo = ImageInfo::new(FileKind::Raw);

fn sample() -> XmpData {
    let mut settings = DevelopSettings::default_for(FileKind::Raw);
    settings.tone.exposure = 0.75;
    XmpData {
        rating: Rating::new(4),
        label: ColorLabel::Red,
        title: "Harbour".to_owned(),
        keywords: vec!["harbour".to_owned(), "blue hour".to_owned()],
        develop: Some(settings),
        ..XmpData::default()
    }
}

fn entries(directory: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    paths
}

#[test]
fn raw_sidecars_replace_the_extension_and_raster_sidecars_append_to_it() {
    let cases = [
        ("/shoot/IMG_0001.ARW", FileKind::Raw, "/shoot/IMG_0001.xmp"),
        (
            "/shoot/pano.final.v2.CR3",
            FileKind::Raw,
            "/shoot/pano.final.v2.xmp",
        ),
        ("/shoot.2024/IMG.NEF", FileKind::Raw, "/shoot.2024/IMG.xmp"),
        ("/shoot/scan", FileKind::Raw, "/shoot/scan.xmp"),
        ("IMG_0002.dng", FileKind::Raw, "IMG_0002.xmp"),
        (
            "/shoot/IMG_0001.jpg",
            FileKind::Raster,
            "/shoot/IMG_0001.jpg.xmp",
        ),
        (
            "/shoot/pano.final.v2.tiff",
            FileKind::Raster,
            "/shoot/pano.final.v2.tiff.xmp",
        ),
        (
            "/shoot.2024/IMG.PNG",
            FileKind::Raster,
            "/shoot.2024/IMG.PNG.xmp",
        ),
        ("/shoot/scan", FileKind::Raster, "/shoot/scan.xmp"),
        ("IMG_0002.jpeg", FileKind::Raster, "IMG_0002.jpeg.xmp"),
    ];
    for (image, kind, expected) in cases {
        assert_eq!(
            sidecar_path(Path::new(image), kind),
            Path::new(expected),
            "{image} {kind:?}"
        );
    }
}

#[test]
fn a_raw_file_and_its_rendered_neighbour_never_share_a_sidecar() {
    let raw = sidecar_path(Path::new("/shoot/IMG_0001.ARW"), FileKind::Raw);
    let jpeg = sidecar_path(Path::new("/shoot/IMG_0001.JPG"), FileKind::Raster);
    assert_ne!(raw, jpeg);
}

#[test]
fn a_missing_sidecar_reads_as_none() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("IMG_0001.xmp");
    assert_eq!(read_sidecar(&path, RAW).unwrap(), None);
    let nowhere = directory.path().join("no-such-folder").join("IMG_0001.xmp");
    assert_eq!(read_sidecar(&nowhere, RAW).unwrap(), None);
}

#[test]
fn writing_creates_a_sidecar_and_then_updates_it() {
    let directory = tempfile::tempdir().unwrap();
    let path = sidecar_path(&directory.path().join("IMG_0001.ARW"), FileKind::Raw);

    let first = sample();
    write_sidecar(&path, &first, RAW).unwrap();
    assert_eq!(read_sidecar(&path, RAW).unwrap(), Some(first.clone()));

    let second = XmpData {
        rating: Rating::new(1),
        keywords: vec!["harbour".to_owned()],
        ..first
    };
    write_sidecar(&path, &second, RAW).unwrap();
    assert_eq!(read_sidecar(&path, RAW).unwrap(), Some(second));
    assert_eq!(
        entries(directory.path()),
        [path],
        "nothing but the sidecar is left behind"
    );
}

#[test]
fn updating_on_disk_keeps_what_another_application_wrote() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("IMG_0001.xmp");
    fs::write(&path, include_str!("fixtures/foreign.xmp")).unwrap();

    let mut data = read_sidecar(&path, RAW).unwrap().unwrap();
    data.rating = Rating::new(5);
    write_sidecar(&path, &data, RAW).unwrap();

    let text = fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("<crs:MaskGroupBasedCorrections>") && text.contains(r#"acme:Score="7.5""#)
    );
    assert_eq!(read_sidecar(&path, RAW).unwrap(), Some(data));
}

#[test]
fn a_sidecar_that_already_says_the_same_is_not_rewritten() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("IMG_0001.xmp");
    write_sidecar(&path, &sample(), RAW).unwrap();
    let long_ago = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(long_ago)
        .unwrap();

    write_sidecar(&path, &sample(), RAW).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), long_ago);
}

#[test]
fn a_sidecar_that_cannot_be_parsed_is_an_error_and_is_left_exactly_as_it_was() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("IMG_0001.xmp");
    let damaged = &include_str!("fixtures/foreign.xmp")[..900];
    fs::write(&path, damaged).unwrap();

    assert!(matches!(
        read_sidecar(&path, RAW),
        Err(XmpError::Malformed { .. })
    ));
    assert!(matches!(
        write_sidecar(&path, &sample(), RAW),
        Err(XmpError::Malformed { .. })
    ));
    assert_eq!(fs::read_to_string(&path).unwrap(), damaged);
    assert_eq!(entries(directory.path()), [path]);
}

#[test]
fn an_empty_sidecar_is_an_error_to_read_and_is_replaced_by_a_write() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("IMG_0001.xmp");
    fs::write(&path, "").unwrap();
    assert!(matches!(
        read_sidecar(&path, RAW),
        Err(XmpError::Malformed { .. })
    ));
    write_sidecar(&path, &sample(), RAW).unwrap();
    assert_eq!(read_sidecar(&path, RAW).unwrap(), Some(sample()));
}

#[test]
fn a_sidecar_that_is_not_utf8_is_malformed() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("IMG_0001.xmp");
    let bytes = b"<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" note=\"\xff\xfe\"/>";
    fs::write(&path, bytes).unwrap();
    let error = read_sidecar(&path, RAW).unwrap_err();
    assert!(
        matches!(error, XmpError::Malformed { offset: 42, .. }),
        "{error:?}"
    );
    assert!(write_sidecar(&path, &sample(), RAW).is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn a_path_that_cannot_be_read_as_a_file_is_an_io_error() {
    let directory = tempfile::tempdir().unwrap();
    let error = read_sidecar(directory.path(), RAW).unwrap_err();
    assert!(matches!(error, XmpError::Io { .. }), "{error:?}");
    assert!(
        error
            .to_string()
            .contains(&directory.path().display().to_string()),
        "{error}"
    );
}

#[test]
fn a_write_into_a_missing_folder_fails_without_creating_anything() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gone").join("IMG_0001.xmp");
    assert!(matches!(
        write_sidecar(&path, &sample(), RAW),
        Err(XmpError::Io { .. })
    ));
    assert_eq!(entries(directory.path()), Vec::<PathBuf>::new());
}

#[cfg(unix)]
#[test]
fn a_write_that_cannot_complete_leaves_the_previous_sidecar_intact() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("IMG_0001.xmp");
    write_sidecar(&path, &sample(), RAW).unwrap();
    let before = fs::read(&path).unwrap();

    let locked = fs::Permissions::from_mode(0o555);
    let unlocked = fs::Permissions::from_mode(0o755);
    fs::set_permissions(directory.path(), locked).unwrap();
    let can_still_create = fs::write(directory.path().join("probe"), b"").is_ok();
    let changed = XmpData {
        rating: Rating::new(1),
        ..sample()
    };
    let result = write_sidecar(&path, &changed, RAW);
    fs::set_permissions(directory.path(), unlocked).unwrap();

    if can_still_create {
        // Running with privileges that ignore directory permissions: the
        // failure cannot be provoked this way.
        return;
    }
    assert!(matches!(result, Err(XmpError::Io { .. })), "{result:?}");
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(entries(directory.path()), [path]);
}
