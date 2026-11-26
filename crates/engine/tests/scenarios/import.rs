//! Scanning a folder and importing from it.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use photo_catalog::Catalog;
use photo_core::develop::DevelopSettings;
use photo_core::meta::{ColorLabel, FileKind, Flag, Rating};
use photo_core::time::DateTime;
use photo_engine::{CopyLayout, DatePattern, ImportMode, ImportRequest, Source, TaskState};
use photo_testkit::TempLibrary;
use photo_xmp::{ImageInfo, XmpData};

use crate::common::{PATIENCE, Studio, add, config, many_jpegs};

fn entry(library: &TempLibrary, name: &str) -> PathBuf {
    library
        .entries()
        .iter()
        .find(|entry| entry.path.ends_with(name))
        .unwrap()
        .path
        .clone()
}

#[test]
fn importing_a_folder_lists_its_photos_in_capture_time_order() {
    let library = TempLibrary::new();
    photo_testkit::write_jpeg(library.root(), ".hidden.jpg", 16, 16);
    let cache = library.root().join(".cache");
    fs::create_dir(&cache).unwrap();
    photo_testkit::write_jpeg(&cache, "inside.jpg", 16, 16);
    let mut studio = Studio::new();

    let task = studio.engine.scan_folder(library.root(), false);
    studio.settle();
    let flat = studio.engine.import_scan().unwrap();
    assert_eq!((flat.task, flat.finished), (task, true));
    assert!(flat.candidates.is_empty(), "the root holds only folders");

    studio.engine.scan_folder(library.root(), true);
    studio.settle();
    let scan = studio.engine.import_scan().unwrap();
    let found: Vec<&Path> = scan.candidates.iter().map(|c| c.path.as_path()).collect();
    let expected: Vec<&Path> = library.entries().iter().map(|e| e.path.as_path()).collect();
    assert_eq!(
        found, expected,
        "hidden files and the text file are skipped"
    );
    for candidate in &scan.candidates {
        assert!(!candidate.already_in_catalog && !candidate.suspected_duplicate);
        assert_eq!(
            candidate.file_size,
            fs::metadata(&candidate.path).unwrap().len()
        );
    }
    let ridge = scan
        .candidates
        .iter()
        .find(|c| c.path.ends_with("ridge.jpg"))
        .unwrap();
    assert_eq!(ridge.capture_time, DateTime::new(2024, 3, 15, 8, 5, 33));
    assert_eq!(ridge.kind, FileKind::Raster);

    let files = scan.candidates.iter().map(|c| c.path.clone()).collect();
    let summary = studio.import(files);
    assert_eq!(summary.imported.len(), 6);
    assert!(summary.finished && !summary.cancelled && summary.errors.is_empty());
    assert_eq!(
        studio.names(),
        [
            "lake.png",
            "meadow.tif",
            "dawn.jpg",
            "ridge.jpg",
            "harbour.dng",
            "fireworks.jpg"
        ],
        "undated photos first, then by capture time"
    );
    assert_eq!(studio.engine.source(), &Source::PreviousImport);
    assert_eq!(studio.engine.active_photo(), Some(studio.ids()[0]));
    assert_eq!(studio.engine.selection().selected(), [studio.ids()[0]]);
    assert!(studio.toasts().contains(&"Imported 6 photos".to_string()));

    let counts = studio.engine.source_counts();
    assert_eq!((counts.all_photos, counts.previous_import), (6, 6));
    let folders: Vec<u64> = studio
        .engine
        .folders()
        .iter()
        .map(|f| f.photo_count)
        .collect();
    assert_eq!(folders, [3, 3]);
    let record = studio.engine.photo(studio.id("ridge.jpg")).unwrap();
    assert_eq!((record.metadata.width, record.metadata.height), (64, 96));
    assert_eq!(
        record.creator, "Test Kit",
        "the file's artist is the creator"
    );
    assert_eq!(record.kind, FileKind::Raster);
}

#[test]
fn a_second_import_of_the_same_folder_adds_nothing() {
    let library = TempLibrary::new();
    let mut studio = Studio::new();
    let first = studio.import_library(&library);
    let files: Vec<PathBuf> = library.entries().iter().map(|e| e.path.clone()).collect();
    let second = studio.import(files);
    assert!(second.imported.is_empty());
    assert_eq!(second.already_in_catalog.len(), 6);
    assert_eq!(studio.engine.source_counts().all_photos, 6);
    // An import that added nothing does not hide the one that did.
    assert_eq!(studio.engine.source_counts().previous_import, 6);
    assert_ne!(first.import, second.import);

    studio.engine.scan_folder(library.root(), true);
    studio.settle();
    let scan = studio.engine.import_scan().unwrap();
    assert!(scan.candidates.iter().all(|c| c.already_in_catalog));
}

#[test]
fn suspected_duplicates_are_skipped_when_asked() {
    let library = TempLibrary::new();
    let mut studio = Studio::new();
    studio.import_library(&library);
    let elsewhere = tempfile::tempdir().unwrap();
    let copies: Vec<PathBuf> = library
        .entries()
        .iter()
        .map(|entry| {
            let copy = elsewhere.path().join(entry.path.file_name().unwrap());
            fs::copy(&entry.path, &copy).unwrap();
            copy
        })
        .collect();

    studio.engine.scan_folder(elsewhere.path(), true);
    studio.settle();
    let scan = studio.engine.import_scan().unwrap();
    assert_eq!(scan.candidates.len(), 6);
    assert!(
        scan.candidates
            .iter()
            .all(|c| c.suspected_duplicate && !c.already_in_catalog)
    );

    let task = studio
        .engine
        .start_import(ImportRequest {
            skip_suspected_duplicates: true,
            ..add(copies.clone())
        })
        .unwrap();
    studio.settle();
    let skipped = studio.engine.import_summary(task).unwrap();
    assert!(skipped.imported.is_empty());
    assert_eq!(skipped.suspected_duplicates.len(), 6);

    let kept = studio.import(copies);
    assert_eq!(
        kept.imported.len(),
        6,
        "without the option they are imported"
    );
    assert_eq!(studio.engine.source_counts().all_photos, 12);
}

#[test]
fn copy_mode_lays_files_out_by_date_and_never_overwrites() {
    let library = TempLibrary::new();
    let destination = tempfile::tempdir().unwrap();
    let spring = destination.path().join("2024").join("2024-03-15");
    fs::create_dir_all(&spring).unwrap();
    fs::write(spring.join("dawn.jpg"), b"already here").unwrap();
    let mut studio = Studio::new();

    let files: Vec<PathBuf> = library.entries().iter().map(|e| e.path.clone()).collect();
    let task = studio
        .engine
        .start_import(ImportRequest {
            mode: ImportMode::Copy {
                destination: destination.path().to_path_buf(),
                layout: CopyLayout::ByDate(DatePattern::new("YYYY/YYYY-MM-DD").unwrap()),
            },
            ..add(files)
        })
        .unwrap();
    studio.settle();
    let summary = studio.engine.import_summary(task).unwrap().clone();
    assert_eq!(summary.imported.len(), 6, "{summary:?}");

    assert_eq!(fs::read(spring.join("dawn.jpg")).unwrap(), b"already here");
    assert_eq!(
        fs::read(spring.join("dawn-1.jpg")).unwrap(),
        fs::read(entry(&library, "dawn.jpg")).unwrap()
    );
    let mut expected = vec![
        spring.join("dawn.jpg"),
        spring.join("dawn-1.jpg"),
        spring.join("harbour.dng"),
        spring.join("ridge.jpg"),
        destination
            .path()
            .join("2024")
            .join("2024-07-04")
            .join("fireworks.jpg"),
    ];
    for undated in ["lake.png", "meadow.tif"] {
        let modified = fs::metadata(entry(&library, undated))
            .unwrap()
            .modified()
            .unwrap();
        let seconds = modified
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let date = DateTime::from_unix_seconds(seconds as i64).unwrap();
        let folder = DatePattern::new("YYYY/YYYY-MM-DD")
            .unwrap()
            .folder_for(date);
        expected.push(destination.path().join(folder).join(undated));
    }
    expected.sort();
    assert_eq!(
        files_under(destination.path()),
        expected,
        "nothing else is left there"
    );

    for id in &summary.imported {
        let record = studio.engine.photo(*id).unwrap();
        assert!(
            record.path.starts_with(destination.path()),
            "{}",
            record.path.display()
        );
        assert_ne!(record.path, spring.join("dawn.jpg"));
        assert_eq!(record.file_size, fs::metadata(&record.path).unwrap().len());
    }
    assert!(library.entries().iter().all(|entry| entry.path.is_file()));
}

#[test]
fn a_sidecar_s_rating_label_keywords_and_develop_settings_arrive_with_the_photo() {
    let library = TempLibrary::new();
    let dawn = entry(&library, "dawn.jpg");
    let mut settings = DevelopSettings::default();
    settings.tone.exposure = 0.5;
    let data = XmpData {
        rating: Rating::new(4),
        flag: Flag::Picked,
        label: ColorLabel::Green,
        title: "Dawn over the bay".to_string(),
        keywords: vec!["sunrise".to_string(), "coast".to_string()],
        develop: Some(settings),
        ..XmpData::default()
    };
    let sidecar = photo_xmp::sidecar_path(&dawn, FileKind::Raster);
    let info = ImageInfo::new(FileKind::Raster).with_size(96, 64);
    photo_xmp::write_sidecar(&sidecar, &data, info).unwrap();
    let mut studio = Studio::new();

    let files: Vec<PathBuf> = library.entries().iter().map(|e| e.path.clone()).collect();
    let task = studio
        .engine
        .start_import(ImportRequest {
            keywords: vec!["imported".to_string(), "Coast".to_string()],
            ..add(files)
        })
        .unwrap();
    studio.settle();
    assert!(
        studio
            .engine
            .import_summary(task)
            .unwrap()
            .errors
            .is_empty()
    );

    let photo = studio.id("dawn.jpg");
    let record = studio.engine.photo(photo).unwrap();
    assert_eq!(record.rating, Rating::new(4));
    assert_eq!(record.flag, Flag::Picked);
    assert_eq!(record.label, ColorLabel::Green);
    assert_eq!(record.title, "Dawn over the bay");
    assert!(record.has_edits);
    assert_eq!(
        studio.engine.develop_settings(photo).unwrap().tone.exposure,
        0.5
    );
    let mut keywords: Vec<String> = studio
        .engine
        .photo_keywords(photo)
        .unwrap()
        .into_iter()
        .map(|keyword| keyword.name)
        .collect();
    keywords.sort();
    assert_eq!(keywords, ["coast", "imported", "sunrise"]);

    let ridge = studio.engine.photo(studio.id("ridge.jpg")).unwrap();
    assert_eq!(
        ridge.rating,
        Rating::UNRATED,
        "photos without a sidecar start plain"
    );
}

#[test]
fn cancelling_an_import_stops_promptly_and_keeps_what_was_imported_consistent() {
    let (_files_dir, files) = many_jpegs(400, 24, 16);
    let mut studio = Studio::with_workers(1);
    let task = studio.engine.start_import(add(files)).unwrap();
    wait_for_first_batch(&mut studio, task);

    assert!(studio.engine.cancel_task(task));
    let start = Instant::now();
    studio.settle();
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "{:?}",
        start.elapsed()
    );

    let summary = studio.engine.import_summary(task).unwrap().clone();
    assert!(summary.finished && summary.cancelled);
    assert!(
        !summary.imported.is_empty() && summary.imported.len() < 400,
        "{} imported",
        summary.imported.len()
    );
    assert_eq!(
        studio.engine.task(task).unwrap().state,
        TaskState::Cancelled
    );
    let counts = studio.engine.source_counts();
    assert_eq!(counts.all_photos, summary.imported.len() as u64);
    assert_eq!(counts.previous_import, summary.imported.len() as u64);
    for id in &summary.imported {
        let record = studio.engine.photo(*id).unwrap();
        assert!(record.path.is_file() && record.file_size > 0);
    }
    let dir = studio.dir;
    drop(studio.engine);
    let catalog = Catalog::open(config(dir.path(), 1).catalog_path).unwrap();
    assert!(catalog.check_integrity().unwrap().is_empty());
}

#[test]
fn a_cancelled_copy_import_leaves_no_file_without_a_record() {
    let (_files_dir, files) = many_jpegs(300, 24, 16);
    let destination = tempfile::tempdir().unwrap();
    let mut studio = Studio::with_workers(1);
    let request = ImportRequest {
        mode: ImportMode::Copy {
            destination: destination.path().to_path_buf(),
            layout: CopyLayout::OneFolder,
        },
        ..add(files)
    };
    let task = studio.engine.start_import(request).unwrap();
    wait_for_first_batch(&mut studio, task);
    assert!(studio.engine.cancel_task(task));
    studio.settle();

    let summary = studio.engine.import_summary(task).unwrap().clone();
    assert!(summary.cancelled && summary.imported.len() < 300);
    let mut recorded: Vec<PathBuf> = summary
        .imported
        .iter()
        .map(|id| studio.engine.photo(*id).unwrap().path)
        .collect();
    recorded.sort();
    assert_eq!(files_under(destination.path()), recorded);
}

/// Poll until the import has inserted its first batch. Each wait blocks on
/// the workers' results, not on a timer, and one poll brings in a bounded
/// number of them, so the run cannot finish unseen; the deadline only
/// catches a hang.
fn wait_for_first_batch(studio: &mut Studio, task: photo_engine::TaskId) {
    let deadline = Instant::now() + PATIENCE;
    while studio
        .engine
        .import_summary(task)
        .unwrap()
        .imported
        .is_empty()
    {
        assert!(
            Instant::now() < deadline,
            "the import never inserted anything"
        );
        let idle = studio.engine.wait_idle(Duration::from_millis(1));
        assert!(!idle, "the import ended before it could be cancelled");
    }
}

/// Every file under `root`, sorted.
fn files_under(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut folders = vec![root.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in fs::read_dir(folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                folders.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// Closing wherever the workers happen to be. Whether a finished copy is
/// waiting for its record at that moment is left to chance here; the test
/// that puts one in place is
/// `engine::tests::imports::closing_the_app_during_a_copy_import_leaves_no_file_without_a_record`.
#[test]
fn closing_the_app_part_way_through_a_copy_import_leaves_no_file_without_a_record() {
    let (_files_dir, files) = many_jpegs(300, 24, 16);
    let destination = tempfile::tempdir().unwrap();
    let mut studio = Studio::with_workers(1);
    let request = ImportRequest {
        mode: ImportMode::Copy {
            destination: destination.path().to_path_buf(),
            layout: CopyLayout::OneFolder,
        },
        ..add(files)
    };
    let task = studio.engine.start_import(request).unwrap();
    wait_for_first_batch(&mut studio, task);
    let Studio { dir, engine } = studio;
    drop(engine);

    let catalog = Catalog::open(config(dir.path(), 1).catalog_path).unwrap();
    let copies = files_under(destination.path());
    assert!(!copies.is_empty(), "the first batch is kept");
    for copy in copies {
        assert!(
            catalog.find_by_path(&copy).unwrap().is_some(),
            "{} has no record",
            copy.display()
        );
    }
}

#[test]
fn cancelling_the_scan_of_one_large_folder_stops_it_part_way() {
    const FILES: usize = 5000;
    let (_files_dir, files) = many_jpegs(FILES, 24, 16);
    let folder = files[0].parent().unwrap().to_path_buf();
    let mut studio = Studio::with_workers(1);
    let task = studio.engine.scan_folder(&folder, false);
    let deadline = Instant::now() + PATIENCE;
    while studio.engine.import_scan().unwrap().candidates.is_empty() {
        assert!(Instant::now() < deadline, "the scan never found anything");
        let idle = studio.engine.wait_idle(Duration::from_millis(1));
        assert!(!idle, "the scan ended before it could be cancelled");
    }
    assert!(studio.engine.cancel_task(task));
    studio.settle();
    let scan = studio.engine.import_scan().unwrap();
    assert!(scan.finished);
    // Before every entry was checked, only the last few files of a
    // cancelled scan were left out.
    assert!(
        scan.candidates.len() < FILES / 2,
        "the scan ran on to file {} of {FILES}",
        scan.candidates.len()
    );
    assert_eq!(
        studio.engine.task(task).unwrap().state,
        TaskState::Cancelled
    );
}
