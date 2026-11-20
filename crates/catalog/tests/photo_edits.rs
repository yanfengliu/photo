//! Changing a photo's attributes, virtual copies, and files that move.

mod common;

use std::path::{Path, PathBuf};

use common::{import, import_paths, memory, raster, raw};
use photo_catalog::{Catalog, CatalogError, Entity, PhotoRecord, TextField};
use photo_core::develop::DevelopSettings;
use photo_core::meta::{ColorLabel, FileKind, Flag, PhotoId, Rating};

fn exposure(stops: f32) -> DevelopSettings {
    let mut settings = DevelopSettings::default();
    settings.tone.exposure = stops;
    settings
}

fn three() -> (Catalog, Vec<PhotoId>) {
    let mut catalog = memory();
    let ids = import_paths(&mut catalog, &["/e/a.jpg", "/e/b.jpg", "/e/c.jpg"]);
    (catalog, ids)
}

fn records(catalog: &Catalog, ids: &[PhotoId]) -> Vec<PhotoRecord> {
    catalog.photos(ids).unwrap()
}

#[test]
fn ratings_flags_and_labels_are_set_per_photo() {
    let (mut catalog, ids) = three();
    catalog
        .set_ratings(&[(ids[0], Rating::new(5)), (ids[2], Rating::new(2))])
        .unwrap();
    catalog
        .set_flags(&[(ids[0], Flag::Picked), (ids[1], Flag::Rejected)])
        .unwrap();
    catalog
        .set_labels(&[(ids[1], ColorLabel::Blue), (ids[2], ColorLabel::Purple)])
        .unwrap();

    let after = records(&catalog, &ids);
    assert_eq!(
        after.iter().map(|r| r.rating.stars()).collect::<Vec<_>>(),
        [5, 0, 2]
    );
    assert_eq!(
        after.iter().map(|r| r.flag).collect::<Vec<_>>(),
        [Flag::Picked, Flag::Rejected, Flag::Unflagged]
    );
    assert_eq!(
        after.iter().map(|r| r.label).collect::<Vec<_>>(),
        [ColorLabel::None, ColorLabel::Blue, ColorLabel::Purple]
    );
    // Attributes are not edits: the develop state is untouched.
    assert!(
        after
            .iter()
            .all(|r| r.develop_version == 0 && !r.has_edits && r.edited_at.is_none())
    );
}

#[test]
fn handing_back_the_previous_values_undoes_a_change() {
    let (mut catalog, ids) = three();
    catalog
        .set_ratings(&[(ids[0], Rating::new(1)), (ids[1], Rating::new(4))])
        .unwrap();
    catalog
        .set_text(TextField::Title, &[(ids[0], "Old title")])
        .unwrap();
    let before = records(&catalog, &ids);

    // "Set rating to 3" and "set title" on a selection of three photos...
    let previous_ratings: Vec<(PhotoId, Rating)> =
        before.iter().map(|r| (r.id, r.rating)).collect();
    let previous_titles: Vec<(PhotoId, String)> =
        before.iter().map(|r| (r.id, r.title.clone())).collect();
    catalog
        .set_ratings(
            &ids.iter()
                .map(|id| (*id, Rating::new(3)))
                .collect::<Vec<_>>(),
        )
        .unwrap();
    catalog
        .set_text(
            TextField::Title,
            &ids.iter().map(|id| (*id, "Same")).collect::<Vec<_>>(),
        )
        .unwrap();
    assert!(
        records(&catalog, &ids)
            .iter()
            .all(|r| r.rating == Rating::new(3) && r.title == "Same")
    );

    // ...and their undo.
    catalog.set_ratings(&previous_ratings).unwrap();
    catalog
        .set_text(TextField::Title, &previous_titles)
        .unwrap();
    assert_eq!(records(&catalog, &ids), before);
}

#[test]
fn each_text_field_is_stored_separately_and_verbatim() {
    let (mut catalog, ids) = three();
    let id = ids[0];
    catalog
        .set_text(TextField::Title, &[(id, "Title ✓")])
        .unwrap();
    catalog
        .set_text(
            TextField::Caption,
            &[(id, "Two\nlines, with 'quotes' and \"more\"")],
        )
        .unwrap();
    catalog
        .set_text(TextField::Creator, &[(id, String::from("  spaced  "))])
        .unwrap();
    catalog
        .set_text(TextField::Copyright, &[(id, "© 2026")])
        .unwrap();

    let record = catalog.photo(id).unwrap();
    assert_eq!(record.title, "Title ✓");
    assert_eq!(record.caption, "Two\nlines, with 'quotes' and \"more\"");
    assert_eq!(record.creator, "  spaced  ");
    assert_eq!(record.copyright, "© 2026");

    catalog.set_text(TextField::Caption, &[(id, "")]).unwrap();
    assert_eq!(catalog.photo(id).unwrap().caption, "");
    assert_eq!(catalog.photo(id).unwrap().title, "Title ✓");
    assert_eq!(catalog.photo(ids[1]).unwrap().title, "");
}

#[test]
fn a_setter_with_an_unknown_photo_changes_nothing_at_all() {
    let (mut catalog, ids) = three();
    let before = records(&catalog, &ids);
    let ghost = PhotoId(404);
    let not_found = |result: Result<(), CatalogError>| {
        assert!(matches!(
            result,
            Err(CatalogError::NotFound {
                entity: Entity::Photo,
                id: 404
            })
        ));
    };

    not_found(catalog.set_ratings(&[(ids[0], Rating::new(5)), (ghost, Rating::new(5))]));
    not_found(catalog.set_flags(&[(ids[0], Flag::Picked), (ghost, Flag::Picked)]));
    not_found(catalog.set_labels(&[(ids[0], ColorLabel::Red), (ghost, ColorLabel::Red)]));
    not_found(catalog.set_text(TextField::Title, &[(ids[0], "x"), (ghost, "x")]));
    not_found(catalog.set_missing(&[ids[0], ghost], true));
    assert_eq!(records(&catalog, &ids), before);

    // Empty batches are fine.
    catalog.set_ratings(&[]).unwrap();
    catalog.set_missing(&[], true).unwrap();
}

#[test]
fn a_virtual_copy_shares_the_file_and_starts_as_a_duplicate_of_its_source() {
    let mut catalog = memory();
    let mut photo = raw("/v/DSC_1.ARW");
    photo.file_size = 4242;
    photo.rating = Rating::new(4);
    photo.label = ColorLabel::Green;
    photo.title = "Original".into();
    photo.keywords = vec!["tree".into(), "fog".into()];
    photo.metadata.iso = Some(800);
    let master = import(&mut catalog, &[photo])[0];
    catalog
        .set_develop_settings(master, &exposure(0.5), Some("Exposure"))
        .unwrap();
    catalog
        .create_snapshot(master, "Snap", &exposure(0.5))
        .unwrap();
    catalog
        .add_to_collection(catalog.quick_collection(), &[master])
        .unwrap();

    let copy = catalog.create_virtual_copy(master).unwrap();
    let (original, duplicate) = (catalog.photo(master).unwrap(), catalog.photo(copy).unwrap());

    assert_ne!(copy, master);
    assert_eq!(duplicate.master_id, Some(master));
    assert_eq!(duplicate.copy_name.as_deref(), Some("Copy 1"));
    assert_eq!(
        (original.master_id, original.copy_name.clone()),
        (None, None)
    );
    // Same file...
    assert_eq!(duplicate.path, original.path);
    assert_eq!(
        (duplicate.folder_id, duplicate.import_id),
        (original.folder_id, original.import_id)
    );
    assert_eq!(
        (
            duplicate.kind,
            duplicate.file_size,
            duplicate.format.as_str()
        ),
        (FileKind::Raw, 4242, "arw")
    );
    assert_eq!(duplicate.metadata, original.metadata);
    // ...same look and attributes to begin with...
    assert_eq!(catalog.develop_settings(copy).unwrap(), exposure(0.5));
    assert!(duplicate.has_edits);
    assert_eq!(
        (duplicate.rating, duplicate.label, duplicate.title.as_str()),
        (Rating::new(4), ColorLabel::Green, "Original")
    );
    let keywords = |catalog: &Catalog, id| -> Vec<String> {
        catalog
            .photo_keywords(id)
            .unwrap()
            .into_iter()
            .map(|keyword| keyword.name)
            .collect()
    };
    assert_eq!(keywords(&catalog, copy), ["fog", "tree"]);
    // ...but its own history, snapshots and collections.
    assert!(catalog.history(copy).unwrap().is_empty());
    assert!(catalog.snapshots(copy).unwrap().is_empty());
    assert!(catalog.photo_collections(copy).unwrap().is_empty());
    // The path still belongs to the master alone.
    assert_eq!(catalog.find_by_path("/v/DSC_1.ARW").unwrap(), Some(master));
}

#[test]
fn a_virtual_copy_and_its_master_are_edited_independently() {
    let mut catalog = memory();
    let master = import(&mut catalog, &[raster("/v/a.jpg")])[0];
    catalog
        .set_develop_settings(master, &exposure(0.5), Some("Start"))
        .unwrap();
    let copy = catalog.create_virtual_copy(master).unwrap();

    catalog
        .set_develop_settings(copy, &exposure(-2.0), Some("Darker"))
        .unwrap();
    catalog.set_ratings(&[(copy, Rating::new(5))]).unwrap();
    catalog
        .set_text(TextField::Title, &[(copy, "B&W version")])
        .unwrap();
    catalog.set_photo_keywords(copy, &["mono"]).unwrap();

    assert_eq!(catalog.develop_settings(master).unwrap(), exposure(0.5));
    assert_eq!(catalog.develop_settings(copy).unwrap(), exposure(-2.0));
    assert_eq!(catalog.history(master).unwrap().len(), 1);
    assert_eq!(catalog.history(copy).unwrap().len(), 1);
    let original = catalog.photo(master).unwrap();
    assert_eq!(
        (original.rating, original.title.as_str()),
        (Rating::UNRATED, "")
    );
    assert!(catalog.photo_keywords(master).unwrap().is_empty());

    // And the other way round.
    catalog
        .set_develop_settings(master, &DevelopSettings::default(), Some("Reset"))
        .unwrap();
    assert!(!catalog.photo(master).unwrap().has_edits);
    assert!(catalog.photo(copy).unwrap().has_edits);
    assert_eq!(catalog.develop_settings(copy).unwrap(), exposure(-2.0));
}

#[test]
fn copies_are_numbered_and_a_copy_of_a_copy_belongs_to_the_same_master() {
    let mut catalog = memory();
    let master = import(&mut catalog, &[raster("/v/a.jpg")])[0];
    let first = catalog.create_virtual_copy(master).unwrap();
    catalog
        .set_develop_settings(first, &exposure(1.0), None)
        .unwrap();
    let second = catalog.create_virtual_copy(first).unwrap();
    let name = |catalog: &Catalog, id| catalog.photo(id).unwrap().copy_name.unwrap();

    assert_eq!(name(&catalog, first), "Copy 1");
    assert_eq!(name(&catalog, second), "Copy 2");
    assert_eq!(
        catalog.photo(second).unwrap().master_id,
        Some(master),
        "never a copy of a copy"
    );
    assert_eq!(
        catalog.develop_settings(second).unwrap(),
        exposure(1.0),
        "but it starts from the copy it was made of"
    );

    // A name is not reused after a copy is removed.
    catalog.remove_photos(&[first]).unwrap();
    let third = catalog.create_virtual_copy(master).unwrap();
    assert_eq!(name(&catalog, third), "Copy 3");
    assert!(matches!(
        catalog.create_virtual_copy(PhotoId(404)),
        Err(CatalogError::NotFound { .. })
    ));
}

#[test]
fn missing_is_a_property_of_the_file_shared_by_a_master_and_its_copies() {
    let mut catalog = memory();
    let ids = import_paths(&mut catalog, &["/m/a.jpg", "/m/b.jpg"]);
    let copy = catalog.create_virtual_copy(ids[0]).unwrap();
    let missing =
        |catalog: &Catalog| [ids[0], copy, ids[1]].map(|id| catalog.photo(id).unwrap().missing);

    // Reported through the copy, recorded for the file.
    catalog.set_missing(&[copy], true).unwrap();
    assert_eq!(missing(&catalog), [true, true, false]);
    catalog.set_missing(&[ids[0]], false).unwrap();
    assert_eq!(missing(&catalog), [false, false, false]);
}

#[test]
fn relinking_moves_a_master_and_its_copies_to_the_new_path_and_folder() {
    let mut catalog = memory();
    let ids = import_paths(&mut catalog, &["/old/IMG_10.jpg", "/old/other.jpg"]);
    let copy = catalog.create_virtual_copy(ids[0]).unwrap();
    catalog.set_missing(&[ids[0]], true).unwrap();

    catalog
        .relink_photo(copy, Path::new("/new/place/IMG_2.jpg"))
        .unwrap();

    for id in [ids[0], copy] {
        let record = catalog.photo(id).unwrap();
        assert_eq!(record.path, PathBuf::from("/new/place/IMG_2.jpg"));
        assert_eq!(record.file_name, "IMG_2.jpg");
        assert!(!record.missing);
        let folder = catalog
            .folders()
            .unwrap()
            .into_iter()
            .find(|f| f.id == record.folder_id)
            .unwrap();
        assert_eq!(folder.path, PathBuf::from("/new/place"));
    }
    assert_eq!(
        catalog.find_by_path("/new/place/IMG_2.jpg").unwrap(),
        Some(ids[0])
    );
    assert_eq!(catalog.find_by_path("/old/IMG_10.jpg").unwrap(), None);
    // The new name is what sorting and searching see from now on.
    let by_name = photo_catalog::PhotoQuery {
        sort: photo_catalog::Sort {
            key: photo_catalog::SortKey::FileName,
            ..Default::default()
        },
        ..Default::default()
    };
    assert_eq!(
        common::names(&catalog, &by_name),
        ["IMG_2.jpg", "IMG_2.jpg", "other.jpg"]
    );
}

#[test]
fn relinking_onto_another_photos_file_is_refused() {
    let mut catalog = memory();
    let ids = import_paths(&mut catalog, &["/r/a.jpg", "/r/b.jpg"]);
    let taken = catalog
        .relink_photo(ids[0], Path::new("/r/b.jpg"))
        .unwrap_err();
    assert!(matches!(taken, CatalogError::DuplicatePath(path) if path == "/r/b.jpg"));
    assert_eq!(
        catalog.photo(ids[0]).unwrap().path,
        PathBuf::from("/r/a.jpg")
    );

    // Relinking to where it already is, is fine (it clears the missing state).
    catalog.set_missing(&[ids[0]], true).unwrap();
    catalog.relink_photo(ids[0], Path::new("/r/a.jpg")).unwrap();
    assert!(!catalog.photo(ids[0]).unwrap().missing);

    assert!(matches!(
        catalog.relink_photo(ids[0], Path::new("nowhere.jpg")),
        Err(CatalogError::InvalidPath(_))
    ));
    assert!(matches!(
        catalog.relink_photo(PhotoId(404), Path::new("/r/c.jpg")),
        Err(CatalogError::NotFound { .. })
    ));
}
