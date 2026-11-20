//! Exporting photos as files: names, sizes, cancelling, the originals of
//! the catalog, and export presets.

use std::fs;
use std::path::{Path, PathBuf};

use photo_core::meta::PhotoId;
use photo_engine::{
    EngineError, ExistingFile, ExportOutcome, ExportSettings, Folder, Format, Location, Naming,
    Resize, Sizing, Target, TaskState,
};
use photo_testkit::TempLibrary;

use crate::common::{Studio, config};

/// Small JPEGs into `folder`, named by sequence.
fn web(folder: &Path) -> ExportSettings {
    ExportSettings {
        location: Location {
            folder: Folder::Specific(folder.to_path_buf()),
            ..Location::default()
        },
        naming: Naming {
            template: "trip-{seq:2}".into(),
            ..Naming::default()
        },
        format: Format::Jpeg { quality: 85 },
        sizing: Sizing {
            resize: Resize::LongEdge { pixels: 48 },
            dont_enlarge: true,
        },
        ..ExportSettings::default()
    }
}

/// Every name in `folder`, sorted; empty when it does not exist.
fn names(folder: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn three_photos_land_with_their_names_and_sizes_in_the_selections_order() {
    let library = TempLibrary::new();
    let mut studio = Studio::new();
    studio.import_library(&library);
    let (dawn, ridge, lake) = (
        studio.id("dawn.jpg"),
        studio.id("ridge.jpg"),
        studio.id("lake.png"),
    );
    studio
        .engine
        .rotate_right(Target::Photos(vec![lake]))
        .unwrap();
    studio.engine.select_only(dawn);
    studio.engine.toggle_selected(ridge);
    studio.engine.toggle_selected(lake);
    let order: Vec<PhotoId> = studio.engine.selection().selected().to_vec();
    let out = studio.dir.path().join("out");
    let settings = web(&out);
    let planned = studio
        .engine
        .planned_export_path(Target::Selected, &settings)
        .unwrap();
    assert_eq!(planned, Some(out.join("trip-01.jpg")));
    studio.toasts();

    let task = studio
        .engine
        .start_export(Target::Selected, settings)
        .unwrap();
    studio.settle();
    let info = studio.engine.task(task).unwrap();
    assert_eq!(
        (info.state.clone(), info.done, info.total, info.errors),
        (TaskState::Finished, 3, 3, 0)
    );
    assert_eq!(studio.toasts(), ["Exported 3 photos"]);
    assert_eq!(names(&out), ["trip-01.jpg", "trip-02.jpg", "trip-03.jpg"]);

    let results = studio.engine.export_results(task).unwrap();
    let size = |photo| match photo {
        p if p == dawn => (48, 32),
        p if p == ridge => (32, 48),
        _ => (32, 48),
    };
    for (index, exported) in results.iter().enumerate() {
        assert_eq!(
            exported.photo, order[index],
            "numbered in the selection's order"
        );
        assert_eq!(exported.sequence, index as u32 + 1);
        let Ok(ExportOutcome::Written {
            path,
            width,
            height,
        }) = &exported.result
        else {
            panic!("not written: {exported:?}");
        };
        assert_eq!(path, &out.join(format!("trip-{:02}.jpg", index + 1)));
        assert_eq!(
            (*width, *height),
            size(exported.photo),
            "the turned lake is upright"
        );
        let decoded = photo_decode::decode(path).unwrap();
        assert_eq!(
            (decoded.image.width, decoded.image.height),
            (*width, *height)
        );
    }
}

#[test]
fn a_cancelled_export_leaves_nothing_behind() {
    let (_files, files) = crate::common::many_jpegs(3, 2400, 1600);
    let mut studio = Studio::with_workers(3);
    studio.import(files);
    let out = studio.dir.path().join("out");
    let mut settings = web(&out);
    settings.sizing.resize = Resize::Original;
    settings.format = Format::Tiff {
        depth: photo_engine::BitDepth::Sixteen,
    };
    let task = studio
        .engine
        .start_export(Target::Photos(studio.ids()), settings)
        .unwrap();
    let second = studio
        .engine
        .start_export(Target::Photos(studio.ids()), web(&out))
        .unwrap();
    assert_eq!(
        studio.engine.pending_work().jobs,
        2,
        "two photos at once, over every export"
    );
    assert!(studio.engine.cancel_task(task));
    assert!(studio.engine.cancel_task(second));
    studio.settle();
    assert_eq!(
        studio.engine.task(second).unwrap().state,
        TaskState::Cancelled
    );
    assert_eq!(
        studio.engine.task(task).unwrap().state,
        TaskState::Cancelled
    );
    assert_eq!(
        names(&out),
        Vec::<String>::new(),
        "no file, partial or temporary"
    );
    assert!(studio.engine.export_results(task).unwrap().is_empty());
    let toasts = studio.toasts();
    assert_eq!(
        toasts.last().map(String::as_str),
        Some("Export cancelled: 0 of 3 photos exported")
    );
}

#[test]
fn the_camera_jpeg_of_a_raw_pair_is_never_written_over() {
    let dir = tempfile::tempdir().unwrap();
    let raw = photo_testkit::write_dng(dir.path(), "IMG_1.DNG", 96, 64);
    let jpeg = photo_testkit::write_jpeg(dir.path(), "IMG_1.JPG", 96, 64);
    let camera = fs::read(&jpeg).unwrap();
    let mut studio = Studio::new();
    studio.import(vec![raw.clone(), jpeg.clone()]);
    let pair = studio.id("IMG_1.DNG");
    let beside = ExportSettings {
        location: Location {
            folder: Folder::SameAsOriginal,
            existing: ExistingFile::Overwrite,
            ..Location::default()
        },
        ..ExportSettings::default()
    };
    let task = studio
        .engine
        .start_export(Target::Photos(vec![pair]), beside)
        .unwrap();
    studio.settle();
    let results = studio.engine.export_results(task).unwrap();
    let reason = results[0].result.clone().unwrap_err();
    assert!(
        reason.contains("original of a photo in the catalog"),
        "{reason}"
    );
    assert_eq!(
        fs::read(&jpeg).unwrap(),
        camera,
        "the camera's JPEG is untouched"
    );
    assert_eq!(
        names(dir.path()),
        ["IMG_1.DNG", "IMG_1.JPG"],
        "nothing else was written"
    );
    assert_eq!(studio.engine.task(task).unwrap().errors, 1);
    assert_eq!(
        studio.toasts().last().map(String::as_str),
        Some("Exported 0 of 1 photo, 1 failed")
    );

    let mut unique = ExportSettings::default();
    unique.location.existing = ExistingFile::UniqueName;
    let task = studio
        .engine
        .start_export(Target::Photos(vec![pair]), unique)
        .unwrap();
    studio.settle();
    let written = studio.engine.export_results(task).unwrap()[0]
        .result
        .clone()
        .unwrap();
    assert_eq!(
        written.path().file_name().unwrap(),
        "IMG_1-2.jpg",
        "a name of its own"
    );
    assert_eq!(fs::read(&jpeg).unwrap(), camera);

    let previews = studio.previews_dir();
    let catalog_path = config(studio.dir.path(), 2).catalog_path;
    let catalog_dir = catalog_path.parent().unwrap().to_path_buf();
    for inside in [previews.clone(), previews.join("sub"), catalog_dir] {
        let mut settings = ExportSettings::default();
        settings.location.folder = Folder::Specific(inside.clone());
        let task = studio
            .engine
            .start_export(Target::Photos(vec![pair]), settings)
            .unwrap();
        studio.settle();
        let reason = studio.engine.export_results(task).unwrap()[0]
            .result
            .clone()
            .unwrap_err();
        assert!(reason.contains("catalog's own folders"), "{reason}");
        assert!(!inside.join("IMG_1.jpg").exists(), "{}", inside.display());
    }
}

#[test]
fn export_presets_are_saved_listed_and_deleted_and_the_default_folder_is_the_exports_one() {
    let mut studio = Studio::new();
    let defaults = studio.engine.default_export_settings();
    let exports = config(studio.dir.path(), 2).export_dir;
    assert_eq!(studio.engine.export_dir(), exports.as_path());
    assert_eq!(defaults.location.folder, Folder::Specific(exports));
    assert!(studio.engine.export_presets().unwrap().is_empty());

    let web = web(&PathBuf::from("/somewhere/web"));
    studio.engine.save_export_preset(" Web ", &web).unwrap();
    studio
        .engine
        .save_export_preset("Archive", &defaults)
        .unwrap();
    assert!(matches!(
        studio.engine.save_export_preset("  ", &web),
        Err(EngineError::EmptyName)
    ));
    let mut studio = studio.reopen();
    let presets = studio.engine.export_presets().unwrap();
    assert_eq!(
        presets,
        [
            ("Archive".to_string(), defaults),
            ("Web".to_string(), web.clone())
        ]
    );
    assert!(studio.engine.delete_export_preset("Archive").unwrap());
    assert!(!studio.engine.delete_export_preset("Archive").unwrap());
    assert_eq!(
        studio.engine.export_presets().unwrap(),
        [("Web".to_string(), web)]
    );
    assert!(matches!(
        studio
            .engine
            .start_export(Target::Selected, ExportSettings::default()),
        Err(EngineError::NothingToExport)
    ));
}
