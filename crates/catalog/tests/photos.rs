//! Adding photos and reading them back.

mod common;

use std::path::{Path, PathBuf};

use common::{date, import, import_paths, memory, raster, raw};
use photo_catalog::{CatalogError, Entity, NewPhoto};
use photo_core::develop::DevelopSettings;
use photo_core::meta::{
    ColorLabel, FileKind, Flag, GpsPosition, ImportId, PhotoId, PhotoMetadata, Rating,
};
use photo_core::time::DateTime;

fn full_metadata() -> PhotoMetadata {
    PhotoMetadata {
        width: 6000,
        height: 4000,
        capture_time: DateTime::new(2024, 2, 29, 13, 5, 59),
        camera_make: Some("SONY".into()),
        camera_model: Some("ILCE-7RM5".into()),
        lens: Some("FE 24-70mm F2.8 GM II".into()),
        focal_length_mm: Some(34.5),
        focal_length_35mm: Some(51.75),
        aperture: Some(2.8),
        shutter_seconds: Some(1.0 / 250.0),
        iso: Some(6400),
        exposure_bias_ev: Some(-0.33),
        flash_fired: Some(false),
        gps: Some(GpsPosition {
            latitude: 47.376_887_123_456,
            longitude: -122.335_167_987_654,
            altitude_m: Some(1234.5),
        }),
        artist: Some("Ansel".into()),
        copyright: Some("© 2024 Ansel".into()),
    }
}

#[test]
fn a_new_photo_takes_its_format_from_the_extension_and_starts_unrated() {
    let photo = NewPhoto::new("/shoots/2024/DSC_0042.ARW", FileKind::Raw);
    assert_eq!(photo.format, "arw");
    assert_eq!(photo.path, PathBuf::from("/shoots/2024/DSC_0042.ARW"));
    assert_eq!(photo.rating, Rating::UNRATED);
    assert_eq!(photo.flag, Flag::Unflagged);
    assert_eq!(photo.label, ColorLabel::None);
    assert!(photo.keywords.is_empty() && photo.develop.is_none() && photo.title.is_empty());
}

#[test]
fn everything_given_at_insert_comes_back_in_the_record() {
    let mut catalog = memory();
    let mut develop = DevelopSettings::default_for(FileKind::Raw);
    develop.tone.exposure = 0.5;
    let new = NewPhoto {
        format: "ARW".into(),
        file_size: 61_234_567,
        file_mtime: 1_709_211_959,
        metadata: full_metadata(),
        rating: Rating::new(4),
        flag: Flag::Picked,
        label: ColorLabel::Green,
        title: "Ridge at dawn".into(),
        caption: "First light on the north face".into(),
        creator: "Ansel".into(),
        copyright: "All rights reserved".into(),
        keywords: vec!["mountain".into(), "dawn".into()],
        develop: Some(develop.clone()),
        ..raw("/shoots/2024/DSC_0042.ARW")
    };
    let run = catalog.begin_import().unwrap();
    let id = catalog.insert_photo(run, &new).unwrap();

    let record = catalog.photo(id).unwrap();
    assert_eq!(record.id, id);
    assert_eq!(record.master_id, None);
    assert_eq!(record.copy_name, None);
    assert_eq!(record.import_id, run);
    assert_eq!(record.path, PathBuf::from("/shoots/2024/DSC_0042.ARW"));
    assert_eq!(record.file_name, "DSC_0042.ARW");
    assert_eq!(record.kind, FileKind::Raw);
    assert_eq!(record.format, "arw", "the format is stored in lower case");
    assert_eq!(record.file_size, 61_234_567);
    assert_eq!(record.file_mtime, 1_709_211_959);
    assert!(!record.missing);
    assert_eq!(record.metadata, full_metadata());
    assert_eq!(record.rating, Rating::new(4));
    assert_eq!(record.flag, Flag::Picked);
    assert_eq!(record.label, ColorLabel::Green);
    assert_eq!(record.title, "Ridge at dawn");
    assert_eq!(record.caption, "First light on the north face");
    assert_eq!(record.creator, "Ansel");
    assert_eq!(record.copyright, "All rights reserved");
    assert_eq!(record.develop_version, 0);
    assert!(record.has_edits);
    assert_eq!(record.edited_at, None);
    assert!(record.added_at > 1_600_000_000);

    assert_eq!(catalog.develop_settings(id).unwrap(), develop);
    let keywords: Vec<String> = catalog
        .photo_keywords(id)
        .unwrap()
        .into_iter()
        .map(|k| k.name)
        .collect();
    assert_eq!(keywords, ["dawn", "mountain"]);
    let folder = catalog
        .folders()
        .unwrap()
        .into_iter()
        .find(|f| f.id == record.folder_id)
        .unwrap();
    assert_eq!(folder.path, PathBuf::from("/shoots/2024"));
}

#[test]
fn a_photo_with_no_metadata_at_all_round_trips_as_empty_metadata() {
    let mut catalog = memory();
    let id = import(&mut catalog, &[raster("/scans/unknown.png")])[0];
    let record = catalog.photo(id).unwrap();
    assert_eq!(record.metadata, PhotoMetadata::default());
    assert_eq!(record.format, "png");
    assert!(!record.has_edits);
    assert_eq!((record.title.as_str(), record.creator.as_str()), ("", ""));
}

#[test]
fn a_batch_gets_ids_in_order_and_creates_each_folder_once() {
    let mut catalog = memory();
    let ids = import_paths(
        &mut catalog,
        &[
            "/lib/2023/a.jpg",
            "/lib/2023/b.jpg",
            "/lib/2024/c.jpg",
            "/lib/2023/trip/d.jpg",
        ],
    );
    assert_eq!(ids.len(), 4);
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));

    let folders = catalog.folders().unwrap();
    let listed: Vec<(String, u64)> = folders
        .iter()
        .map(|f| (f.path.to_string_lossy().into_owned(), f.photo_count))
        .collect();
    assert_eq!(
        listed,
        [
            ("/lib/2023".to_string(), 2),
            ("/lib/2023/trip".to_string(), 1),
            ("/lib/2024".to_string(), 1)
        ]
    );
    assert_eq!(
        catalog.photo(ids[0]).unwrap().folder_id,
        catalog.photo(ids[1]).unwrap().folder_id
    );
}

#[test]
fn a_batch_is_all_or_nothing() {
    let mut catalog = memory();
    import_paths(&mut catalog, &["/lib/existing.jpg"]);
    let before = catalog.stats().unwrap();

    let run = catalog.begin_import().unwrap();
    let mut tagged = raster("/lib/new/one.jpg");
    tagged.keywords = vec!["fresh".into()];
    let batch = [
        tagged,
        raster("/lib/new/two.jpg"),
        raster("/lib/existing.jpg"),
    ];
    let error = catalog.insert_photos(run, &batch).unwrap_err();
    assert!(matches!(error, CatalogError::DuplicatePath(path) if path == "/lib/existing.jpg"));

    // Not the two good photos, not their folder, not the keyword.
    let after = catalog.stats().unwrap();
    assert_eq!(after.photos, before.photos);
    assert_eq!(after.folders, before.folders);
    assert_eq!(after.keywords, before.keywords);
    assert_eq!(catalog.find_by_path("/lib/new/one.jpg").unwrap(), None);
}

#[test]
fn the_same_path_twice_in_one_batch_is_a_duplicate() {
    let mut catalog = memory();
    let run = catalog.begin_import().unwrap();
    let error = catalog
        .insert_photos(run, &[raster("/lib/a.jpg"), raster("/lib/a.jpg")])
        .unwrap_err();
    assert!(matches!(error, CatalogError::DuplicatePath(_)));
    assert_eq!(catalog.stats().unwrap().photos, 0);
}

#[test]
fn inserting_into_an_import_run_that_does_not_exist_is_a_typed_error() {
    let mut catalog = memory();
    let error = catalog
        .insert_photo(ImportId(77), &raster("/lib/a.jpg"))
        .unwrap_err();
    assert!(matches!(
        error,
        CatalogError::NotFound {
            entity: Entity::Import,
            id: 77
        }
    ));
}

#[test]
fn paths_that_do_not_name_a_file_in_a_directory_are_rejected() {
    let mut catalog = memory();
    let run = catalog.begin_import().unwrap();
    for path in ["orphan.jpg", "/lib/", ""] {
        let error = catalog.insert_photo(run, &raster(path)).unwrap_err();
        assert!(matches!(error, CatalogError::InvalidPath(_)), "{path:?}");
    }
    assert_eq!(catalog.stats().unwrap().photos, 0);
}

#[cfg(unix)]
#[test]
fn a_path_that_is_not_utf8_is_rejected_with_a_typed_error() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let mut catalog = memory();
    let run = catalog.begin_import().unwrap();
    let bad = PathBuf::from(OsStr::from_bytes(b"/lib/caf\xe9.jpg"));
    let photos = [
        raster("/lib/fine.jpg"),
        NewPhoto::new(bad.clone(), FileKind::Raster),
    ];

    let error = catalog.insert_photos(run, &photos).unwrap_err();
    assert!(matches!(&error, CatalogError::NonUtf8Path(path) if *path == bad));
    assert_eq!(
        catalog.stats().unwrap().photos,
        0,
        "the valid photo of the batch is not kept either"
    );
    assert!(matches!(
        catalog.find_by_path(&bad),
        Err(CatalogError::NonUtf8Path(_))
    ));
    let id = import_paths(&mut catalog, &["/lib/fine.jpg"])[0];
    assert!(matches!(
        catalog.relink_photo(id, &bad),
        Err(CatalogError::NonUtf8Path(_))
    ));
}

#[test]
fn a_photo_is_found_by_its_exact_path() {
    let mut catalog = memory();
    let ids = import_paths(&mut catalog, &["/lib/a.jpg", "/lib/sub/a.jpg"]);
    assert_eq!(catalog.find_by_path("/lib/a.jpg").unwrap(), Some(ids[0]));
    assert_eq!(
        catalog.find_by_path(Path::new("/lib/sub/a.jpg")).unwrap(),
        Some(ids[1])
    );
    assert_eq!(catalog.find_by_path("/lib/A.jpg").unwrap(), None);
    assert_eq!(catalog.find_by_path("/lib/%.jpg").unwrap(), None);
}

#[test]
fn suspected_duplicates_share_file_name_size_and_capture_time() {
    let mut catalog = memory();
    let shot = |path: &str, size: u64, taken: Option<DateTime>| {
        let mut photo = raster(path);
        photo.file_size = size;
        photo.metadata.capture_time = taken;
        photo
    };
    let noon = Some(date(2024, 5, 1));
    let ids = import(
        &mut catalog,
        &[
            shot("/card/DCIM/IMG_0001.JPG", 5000, noon),
            shot("/backup/IMG_0001.JPG", 5000, noon),
            shot("/other/IMG_0001.JPG", 5001, noon),
            shot("/later/IMG_0001.JPG", 5000, Some(date(2024, 5, 2))),
            shot("/undated/IMG_0001.JPG", 5000, None),
            shot("/card/DCIM/IMG_0002.JPG", 5000, noon),
        ],
    );

    assert_eq!(
        catalog
            .find_suspected_duplicates("IMG_0001.JPG", 5000, noon)
            .unwrap(),
        [ids[0], ids[1]]
    );
    assert_eq!(
        catalog
            .find_suspected_duplicates("IMG_0001.JPG", 5000, None)
            .unwrap(),
        [ids[4]]
    );
    assert!(
        catalog
            .find_suspected_duplicates("IMG_0001.JPG", 4999, noon)
            .unwrap()
            .is_empty()
    );
    assert!(
        catalog
            .find_suspected_duplicates("img_0001.jpg", 5000, noon)
            .unwrap()
            .is_empty()
    );

    // A virtual copy is the same file, not a second copy of it on disk.
    catalog.create_virtual_copy(ids[5]).unwrap();
    assert_eq!(
        catalog
            .find_suspected_duplicates("IMG_0002.JPG", 5000, noon)
            .unwrap(),
        [ids[5]]
    );
}

#[test]
fn records_by_ids_follow_the_order_asked_for_and_skip_unknown_ids() {
    let mut catalog = memory();
    let ids = import_paths(&mut catalog, &["/lib/a.jpg", "/lib/b.jpg", "/lib/c.jpg"]);
    let wanted = [ids[2], PhotoId(9999), ids[0], ids[2]];
    let records = catalog.photos(&wanted).unwrap();
    let names: Vec<&str> = records
        .iter()
        .map(|record| record.file_name.as_str())
        .collect();
    assert_eq!(names, ["c.jpg", "a.jpg", "c.jpg"]);
    assert!(catalog.photos(&[]).unwrap().is_empty());

    assert!(matches!(
        catalog.photo(PhotoId(9999)),
        Err(CatalogError::NotFound {
            entity: Entity::Photo,
            id: 9999
        })
    ));
}

#[test]
fn has_edits_means_the_settings_differ_from_the_default_for_the_kind() {
    let mut catalog = memory();
    let with = |mut photo: NewPhoto, settings: DevelopSettings| {
        photo.develop = Some(settings);
        photo
    };
    let ids = import(
        &mut catalog,
        &[
            raw("/lib/untouched.arw"),
            with(
                raw("/lib/raw-default.arw"),
                DevelopSettings::default_for(FileKind::Raw),
            ),
            with(raw("/lib/raw-neutral.arw"), DevelopSettings::default()),
            raster("/lib/untouched.jpg"),
            with(
                raster("/lib/raster-default.jpg"),
                DevelopSettings::default_for(FileKind::Raster),
            ),
            with(
                raster("/lib/raster-as-raw.jpg"),
                DevelopSettings::default_for(FileKind::Raw),
            ),
        ],
    );
    let edited: Vec<bool> = ids
        .iter()
        .map(|id| catalog.photo(*id).unwrap().has_edits)
        .collect();
    // A RAW file at the neutral settings has had its capture sharpening
    // removed, and a raster at the RAW defaults has had sharpening added.
    assert_eq!(edited, [false, false, true, false, false, true]);
}

#[test]
fn folders_can_be_registered_ahead_of_photos_and_removed_only_when_empty() {
    let mut catalog = memory();
    let root = catalog.ensure_folder(Path::new("/lib/2024/")).unwrap();
    assert_eq!(
        catalog.ensure_folder(Path::new("/lib/2024")).unwrap(),
        root,
        "trailing separators are ignored"
    );
    let id = import_paths(&mut catalog, &["/lib/2024/a.jpg"])[0];
    assert_eq!(catalog.photo(id).unwrap().folder_id, root);

    assert!(
        matches!(catalog.remove_folder(root), Err(CatalogError::FolderNotEmpty(folder)) if folder == root)
    );
    catalog.remove_photos(&[id]).unwrap();
    assert_eq!(
        catalog.folders().unwrap().len(),
        1,
        "an emptied folder stays until it is removed"
    );
    catalog.remove_folder(root).unwrap();
    assert!(catalog.folders().unwrap().is_empty());
    assert!(matches!(
        catalog.remove_folder(root),
        Err(CatalogError::NotFound {
            entity: Entity::Folder,
            ..
        })
    ));
}
