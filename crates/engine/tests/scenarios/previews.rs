//! Thumbnails and standard previews: memory, disk and the source file.

use std::fs;
use std::path::{Path, PathBuf};

use photo_core::meta::PhotoId;
use photo_engine::{LoadState, PreviewOrigin, PreviewSize, PreviewState, Target};
use photo_testkit::TempLibrary;

use crate::common::Studio;

/// Where the documented layout puts a preview file.
fn preview_file(root: &Path, photo: PhotoId, size: &str, version: u64) -> PathBuf {
    root.join(format!("{:02x}", photo.0 % 256))
        .join(photo.0.to_string())
        .join(format!("{size}-{version}.jpg"))
}

fn ready_thumbnails(studio: &mut Studio) -> Vec<(PhotoId, u64)> {
    let photos = studio.ids();
    for photo in &photos {
        assert!(matches!(
            studio.engine.thumbnail(*photo),
            PreviewState::Pending
        ));
        assert_eq!(
            studio.engine.load_state(*photo, PreviewSize::Thumbnail),
            LoadState::Pending
        );
    }
    studio.settle();
    photos
        .iter()
        .map(|photo| {
            let preview = studio.engine.thumbnail(*photo).ready().cloned().unwrap();
            (*photo, preview.version())
        })
        .collect()
}

#[test]
fn visible_thumbnails_are_ready_and_on_disk_after_wait_idle() {
    let library = TempLibrary::new();
    let mut studio = Studio::new();
    studio.import_library(&library);
    let ridge = studio.id("ridge.jpg");
    assert_eq!(
        studio.engine.load_state(ridge, PreviewSize::Thumbnail),
        LoadState::Nothing
    );

    let versions = ready_thumbnails(&mut studio);
    let root = studio.previews_dir();
    for (photo, version) in versions {
        let summary = studio.engine.summary(photo).unwrap().clone();
        assert_eq!(version, summary.develop_version);
        let preview = studio.engine.thumbnail(photo).ready().cloned().unwrap();
        assert_eq!(
            preview.origin(),
            PreviewOrigin::Rendered,
            "{}",
            summary.file_name
        );
        assert_eq!(preview.size(), PreviewSize::Thumbnail);
        let image = preview.image();
        assert!(image.width.max(image.height) <= 384);
        assert_eq!(
            image.width > image.height,
            summary.width > summary.height,
            "{}",
            summary.file_name
        );
        assert_eq!(
            studio.engine.load_state(photo, PreviewSize::Thumbnail),
            LoadState::Ready {
                origin: PreviewOrigin::Rendered
            }
        );
        let file = preview_file(&root, photo, "thumb", version);
        let decoded = image::open(&file).unwrap();
        assert_eq!(
            (decoded.width(), decoded.height()),
            (image.width, image.height)
        );
    }
}

#[test]
fn a_second_engine_serves_previews_from_disk_without_rendering_again() {
    let library = TempLibrary::new();
    let mut studio = Studio::new();
    studio.import_library(&library);
    let versions = ready_thumbnails(&mut studio);
    let root = studio.previews_dir();
    let written: Vec<(PathBuf, Vec<u8>, std::time::SystemTime)> = versions
        .iter()
        .map(|(photo, version)| {
            let file = preview_file(&root, *photo, "thumb", *version);
            let stamp = fs::metadata(&file).unwrap().modified().unwrap();
            (file.clone(), fs::read(&file).unwrap(), stamp)
        })
        .collect();

    let mut studio = studio.reopen();
    let again = ready_thumbnails(&mut studio);
    assert_eq!(again, versions);
    for (file, bytes, stamp) in written {
        assert_eq!(fs::read(&file).unwrap(), bytes, "{}", file.display());
        assert_eq!(fs::metadata(&file).unwrap().modified().unwrap(), stamp);
    }
}

#[test]
fn a_changed_develop_version_writes_a_new_file_and_removes_the_old() {
    let library = TempLibrary::new();
    let mut studio = Studio::new();
    studio.import_library(&library);
    let dawn = studio.id("dawn.jpg");
    studio.engine.thumbnail(dawn);
    studio.settle();
    let old = studio.engine.thumbnail(dawn).ready().cloned().unwrap();
    let root = studio.previews_dir();
    let old_file = preview_file(&root, dawn, "thumb", old.version());
    assert!(old_file.is_file());

    studio
        .engine
        .rotate_right(Target::Photos(vec![dawn]))
        .unwrap();
    assert!(matches!(
        studio.engine.thumbnail(dawn),
        PreviewState::Pending
    ));
    studio.settle();
    let new = studio.engine.thumbnail(dawn).ready().cloned().unwrap();
    assert!(new.version() > old.version());
    let (old_image, new_image) = (old.image(), new.image());
    assert_eq!(
        (new_image.width, new_image.height),
        (old_image.height, old_image.width)
    );
    assert_eq!(
        new_image.pixel(new_image.width - 1, 0),
        old_image.pixel(0, 0),
        "the top-left corner turned to the top-right"
    );
    assert!(preview_file(&root, dawn, "thumb", new.version()).is_file());
    assert!(!old_file.exists(), "the stale version is deleted");
}

#[test]
fn a_deleted_source_file_marks_the_photo_missing() {
    let library = TempLibrary::new();
    let mut studio = Studio::new();
    studio.import_library(&library);
    let (lake, dawn) = (studio.id("lake.png"), studio.id("dawn.jpg"));
    studio.engine.thumbnail(dawn);
    studio.settle();
    let dawn_path = studio.engine.photo(dawn).unwrap().path;
    fs::remove_file(studio.engine.photo(lake).unwrap().path).unwrap();

    studio.engine.thumbnail(lake);
    studio.settle();
    match studio.engine.thumbnail(lake) {
        PreviewState::Failed(reason) => assert!(reason.contains("missing"), "{reason}"),
        other => panic!("expected a failure, got {other:?}"),
    }
    assert!(studio.engine.summary(lake).unwrap().missing);
    assert!(studio.engine.photo(lake).unwrap().missing);
    assert!(
        studio.engine.is_idle(),
        "asking again does not start another job"
    );
    assert!(matches!(
        studio.engine.load_state(lake, PreviewSize::Thumbnail),
        LoadState::Failed(_)
    ));

    // A photo whose file goes missing still shows the preview written for
    // it, from disk.
    let mut studio = studio.reopen();
    fs::remove_file(&dawn_path).unwrap();
    studio.engine.thumbnail(dawn);
    studio.settle();
    assert!(studio.engine.thumbnail(dawn).ready().is_some());
    assert!(studio.engine.summary(dawn).unwrap().missing);
}

#[test]
fn the_standard_preview_carries_its_histogram() {
    let library = TempLibrary::new();
    let mut studio = Studio::new();
    studio.import_library(&library);
    let harbour = studio.id("harbour.dng");
    assert!(
        studio.engine.histogram(harbour).is_none(),
        "asks for the preview"
    );
    studio.settle();
    let preview = studio
        .engine
        .standard_preview(harbour)
        .ready()
        .cloned()
        .unwrap();
    let histogram = preview.histogram().unwrap();
    let image = preview.image();
    assert_eq!(
        histogram.pixel_count(),
        u64::from(image.width * image.height)
    );
    assert_eq!(
        studio.engine.histogram(harbour).as_deref(),
        Some(&**histogram)
    );
    let first = (**histogram).clone();

    let mut studio = studio.reopen();
    studio.engine.standard_preview(harbour);
    studio.settle();
    assert_eq!(
        studio.engine.histogram(harbour).as_deref(),
        Some(&first),
        "kept with the file"
    );
}

#[test]
fn import_candidates_get_thumbnails_from_the_embedded_preview() {
    let library = TempLibrary::new();
    let mut studio = Studio::new();
    studio.engine.scan_folder(library.root(), true);
    studio.settle();
    let paths: Vec<PathBuf> = studio
        .engine
        .import_scan()
        .unwrap()
        .candidates
        .iter()
        .map(|candidate| candidate.path.clone())
        .collect();
    for path in &paths {
        assert!(matches!(
            studio.engine.candidate_thumbnail(path),
            PreviewState::Pending
        ));
    }
    studio.settle();
    for path in &paths {
        let preview = studio
            .engine
            .candidate_thumbnail(path)
            .ready()
            .cloned()
            .unwrap();
        assert!(preview.image().width.max(preview.image().height) <= 384);
    }
}

#[test]
fn a_cell_knows_the_shape_of_its_thumbnail_before_it_arrives() {
    let library = TempLibrary::new();
    let mut studio = Studio::new();
    studio.import_library(&library);
    let (dawn, ridge) = (studio.id("dawn.jpg"), studio.id("ridge.jpg"));
    assert_eq!(studio.engine.displayed_size(dawn), Some((96, 64)));
    assert_eq!(
        studio.engine.displayed_size(ridge),
        Some((64, 96)),
        "upright"
    );
    studio
        .engine
        .rotate_right(Target::Photos(vec![dawn]))
        .unwrap();
    assert_eq!(
        studio.engine.displayed_size(dawn),
        Some((64, 96)),
        "follows the edit"
    );
    let mut settings = studio.engine.develop_settings(dawn).unwrap();
    settings.crop.rect = Some(photo_core::develop::CropRect {
        cx: 0.5,
        cy: 0.5,
        width: 0.5,
        height: 1.0,
    });
    studio.engine.open_develop(dawn).unwrap();
    studio.engine.apply_settings(settings).unwrap();
    let shape = studio.engine.displayed_size(dawn).unwrap();
    studio.engine.thumbnail(dawn);
    studio.settle();
    let thumbnail = studio.engine.thumbnail(dawn).ready().cloned().unwrap();
    let image = thumbnail.image();
    assert_eq!(shape, (32, 96), "a half-width crop of the turned frame");
    assert_eq!(
        (image.width, image.height),
        shape,
        "the thumbnail has that shape"
    );
    assert_eq!(
        studio
            .engine
            .displayed_size(photo_core::meta::PhotoId(9999)),
        None
    );
}
