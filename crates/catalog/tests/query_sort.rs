//! Every sort key, both directions, and what a grid cell gets back.

mod common;

use std::path::PathBuf;

use common::{date, import, import_paths, memory, names, raster, raw};
use photo_catalog::{Catalog, NewPhoto, PhotoQuery, Sort, SortDirection, SortKey, Source};
use photo_core::develop::DevelopSettings;
use photo_core::meta::{ColorLabel, FileKind, Flag, PhotoId, Rating};

fn sorted(catalog: &Catalog, key: SortKey, direction: SortDirection) -> Vec<String> {
    let query = PhotoQuery {
        sort: Sort { key, direction },
        ..PhotoQuery::default()
    };
    names(catalog, &query)
}

fn ascending(catalog: &Catalog, key: SortKey) -> Vec<String> {
    sorted(catalog, key, SortDirection::Ascending)
}

/// Descending must be the exact mirror image of ascending, ties included.
/// Compared by id, because photos that tie on everything else (a master and
/// its virtual copy, or files of one name in two folders) also share a name.
fn assert_descending_mirrors(catalog: &Catalog, key: SortKey) {
    let ids = |direction| {
        let query = PhotoQuery {
            sort: Sort { key, direction },
            ..PhotoQuery::default()
        };
        catalog.query_ids(&query).unwrap()
    };
    let mut reversed = ids(SortDirection::Descending);
    reversed.reverse();
    assert_eq!(reversed, ids(SortDirection::Ascending), "{key:?}");
}

fn library() -> (Catalog, Vec<PhotoId>) {
    let shot = |mut photo: NewPhoto, day: Option<u8>, stars: u8, flag: Flag, label: ColorLabel| {
        photo.metadata.capture_time = day.map(|day| date(2023, 3, day));
        photo.rating = Rating::new(stars);
        photo.flag = flag;
        photo.label = label;
        photo
    };
    let photos = [
        shot(
            raster("/s/c.jpg"),
            Some(9),
            2,
            Flag::Unflagged,
            ColorLabel::Blue,
        ),
        shot(raw("/s/a.dng"), Some(3), 5, Flag::Picked, ColorLabel::None),
        shot(raster("/s/d.png"), None, 0, Flag::Rejected, ColorLabel::Red),
        shot(
            raw("/s/b.arw"),
            Some(5),
            5,
            Flag::Unflagged,
            ColorLabel::Yellow,
        ),
        shot(
            raster("/s/e.jpg"),
            Some(1),
            3,
            Flag::Picked,
            ColorLabel::Purple,
        ),
    ];
    let mut catalog = memory();
    let ids = import(&mut catalog, &photos);
    (catalog, ids)
}

#[test]
fn the_default_sort_is_capture_time_ascending() {
    let (catalog, _) = library();
    assert_eq!(Sort::default().key, SortKey::CaptureTime);
    assert_eq!(Sort::default().direction, SortDirection::Ascending);
    assert_eq!(
        names(&catalog, &PhotoQuery::default()),
        ascending(&catalog, SortKey::CaptureTime)
    );
}

#[test]
fn capture_time_puts_undated_photos_first_when_ascending() {
    let (catalog, _) = library();
    assert_eq!(
        ascending(&catalog, SortKey::CaptureTime),
        ["d.png", "e.jpg", "a.dng", "b.arw", "c.jpg"]
    );
    assert_descending_mirrors(&catalog, SortKey::CaptureTime);
}

#[test]
fn added_order_is_the_order_of_import() {
    let (mut catalog, ids) = library();
    assert_eq!(
        ascending(&catalog, SortKey::Added),
        ["c.jpg", "a.dng", "d.png", "b.arw", "e.jpg"]
    );
    assert_descending_mirrors(&catalog, SortKey::Added);

    // A restored photo returns to its old place, not to the end.
    let removed = catalog.remove_photos(&[ids[1]]).unwrap();
    catalog.restore_photos(&removed).unwrap();
    assert_eq!(
        ascending(&catalog, SortKey::Added),
        ["c.jpg", "a.dng", "d.png", "b.arw", "e.jpg"]
    );
}

#[test]
fn file_names_sort_in_natural_order_ignoring_case() {
    let mut catalog = memory();
    import_paths(
        &mut catalog,
        &[
            "/n/IMG_10.jpg",
            "/n/IMG_2.jpg",
            "/n/img_1.JPG",
            "/n/IMG_100.jpg",
            "/n/Zebra.jpg",
            "/n/apple.jpg",
            "/n/IMG_02.jpg",
            "/other/IMG_2.jpg",
        ],
    );
    assert_eq!(
        ascending(&catalog, SortKey::FileName),
        [
            "apple.jpg",
            "img_1.JPG",
            "IMG_02.jpg",
            "IMG_2.jpg",
            "IMG_2.jpg",
            "IMG_10.jpg",
            "IMG_100.jpg",
            "Zebra.jpg"
        ]
    );
    assert_descending_mirrors(&catalog, SortKey::FileName);
}

#[test]
fn rating_pick_and_label_sort_by_their_values() {
    let (catalog, _) = library();
    // Equal values fall back to capture time.
    assert_eq!(
        ascending(&catalog, SortKey::Rating),
        ["d.png", "c.jpg", "e.jpg", "a.dng", "b.arw"]
    );
    assert_eq!(
        ascending(&catalog, SortKey::Pick),
        ["d.png", "b.arw", "c.jpg", "e.jpg", "a.dng"]
    );
    assert_eq!(
        ascending(&catalog, SortKey::Label),
        ["a.dng", "d.png", "b.arw", "c.jpg", "e.jpg"]
    );
    for key in [SortKey::Rating, SortKey::Pick, SortKey::Label] {
        assert_descending_mirrors(&catalog, key);
    }
}

#[test]
fn file_type_sorts_by_format() {
    let (catalog, _) = library();
    assert_eq!(
        ascending(&catalog, SortKey::FileType),
        ["b.arw", "a.dng", "e.jpg", "c.jpg", "d.png"]
    );
    assert_descending_mirrors(&catalog, SortKey::FileType);
}

#[test]
fn edit_time_follows_the_order_of_edits_with_unedited_photos_first() {
    let (mut catalog, ids) = library();
    let mut settings = DevelopSettings::default();
    settings.tone.exposure = 1.0;
    // Edited in the order e, c, a; then c again, making it the most recent.
    for index in [4, 0, 1, 0] {
        catalog
            .set_develop_settings(ids[index], &settings, None)
            .unwrap();
    }
    assert_eq!(
        ascending(&catalog, SortKey::EditTime),
        ["d.png", "b.arw", "e.jpg", "a.dng", "c.jpg"]
    );
    assert_descending_mirrors(&catalog, SortKey::EditTime);
}

#[test]
fn ties_are_broken_the_same_way_every_time() {
    let mut catalog = memory();
    // Same rating, no capture time, and two pairs sharing a file name.
    let ids = import_paths(
        &mut catalog,
        &["/t/2/b.jpg", "/t/1/b.jpg", "/t/1/a.jpg", "/t/2/a.jpg"],
    );
    // By name first; identical names fall back to the order of import.
    let expected = [ids[2], ids[3], ids[0], ids[1]];
    for key in [
        SortKey::Rating,
        SortKey::CaptureTime,
        SortKey::Pick,
        SortKey::Label,
        SortKey::FileType,
        SortKey::EditTime,
        SortKey::FileName,
    ] {
        let query = PhotoQuery {
            sort: Sort {
                key,
                direction: SortDirection::Ascending,
            },
            ..PhotoQuery::default()
        };
        assert_eq!(catalog.query_ids(&query).unwrap(), expected, "{key:?}");
        assert_eq!(
            common::ids(&catalog.query(&query).unwrap()),
            expected,
            "{key:?}"
        );
        assert_descending_mirrors(&catalog, key);
    }
}

#[test]
fn custom_order_outside_a_collection_is_the_order_of_import() {
    let (catalog, _) = library();
    assert_eq!(
        ascending(&catalog, SortKey::Custom),
        ascending(&catalog, SortKey::Added)
    );
    assert_descending_mirrors(&catalog, SortKey::Custom);
}

#[test]
fn a_summary_carries_everything_a_grid_cell_shows() {
    let mut catalog = memory();
    let mut photo = raw("/g/DSC_7.ARW");
    photo.metadata.width = 7008;
    photo.metadata.height = 4672;
    photo.metadata.capture_time = Some(date(2024, 1, 2));
    photo.rating = Rating::new(4);
    photo.flag = Flag::Picked;
    photo.label = ColorLabel::Yellow;
    let ids = import(&mut catalog, &[photo, raster("/g/plain.jpg")]);
    let copy = catalog.create_virtual_copy(ids[0]).unwrap();
    let mut settings = DevelopSettings::default_for(FileKind::Raw);
    settings.tone.exposure = 0.5;
    catalog.set_develop_settings(copy, &settings, None).unwrap();
    catalog
        .add_to_collection(catalog.quick_collection(), &[copy])
        .unwrap();
    catalog.set_missing(&[ids[1]], true).unwrap();

    let query = PhotoQuery {
        sort: Sort {
            key: SortKey::Added,
            direction: SortDirection::Ascending,
        },
        ..PhotoQuery::from(Source::AllPhotos)
    };
    let cells = catalog.query(&query).unwrap();
    assert_eq!(common::ids(&cells), [ids[0], ids[1], copy]);

    let master = &cells[0];
    assert_eq!(master.file_name, "DSC_7.ARW");
    assert_eq!(master.path, PathBuf::from("/g/DSC_7.ARW"));
    assert_eq!(master.kind, FileKind::Raw);
    assert_eq!(
        (master.rating, master.flag, master.label),
        (Rating::new(4), Flag::Picked, ColorLabel::Yellow)
    );
    assert_eq!((master.width, master.height), (7008, 4672));
    assert_eq!(master.capture_time, Some(date(2024, 1, 2)));
    assert_eq!(master.develop_version, 0);
    assert!(
        !master.has_edits
            && !master.is_virtual_copy()
            && !master.in_quick_collection
            && !master.missing
    );
    assert_eq!(
        (master.master_id, master.copy_name.as_deref()),
        (None, None)
    );

    let plain = &cells[1];
    assert!(plain.missing);
    assert_eq!(
        (plain.width, plain.height, plain.capture_time),
        (0, 0, None)
    );

    let copy_cell = &cells[2];
    assert_eq!(copy_cell.master_id, Some(ids[0]));
    assert_eq!(copy_cell.copy_name.as_deref(), Some("Copy 1"));
    assert!(copy_cell.is_virtual_copy() && copy_cell.has_edits && copy_cell.in_quick_collection);
    assert_eq!(copy_cell.develop_version, 1);
    assert_eq!(
        (copy_cell.path.clone(), copy_cell.rating),
        (master.path.clone(), master.rating)
    );

    // The same cells can be refreshed by id, in the order asked for.
    let refreshed = catalog.summaries(&[copy, PhotoId(404), ids[0]]).unwrap();
    assert_eq!(refreshed, [cells[2].clone(), cells[0].clone()]);
}
