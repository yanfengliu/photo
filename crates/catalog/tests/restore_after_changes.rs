//! Restoring removed photos into a catalog that has moved on in the
//! meantime.

mod common;

use std::path::{Path, PathBuf};

use common::{import_paths, memory};
use photo_catalog::{Catalog, CatalogError, PhotoQuery, Sort, SortKey, Source};
use photo_core::develop::DevelopSettings;
use photo_core::meta::{CollectionId, PhotoId};

fn exposure(stops: f32) -> DevelopSettings {
    let mut settings = DevelopSettings::default();
    settings.tone.exposure = stops;
    settings
}

fn order(catalog: &Catalog, collection: CollectionId) -> Vec<PhotoId> {
    catalog.collection_photos(collection).unwrap()
}

#[test]
fn a_copy_restored_on_its_own_follows_its_master_to_where_the_file_is_now() {
    let mut catalog = memory();
    let master = import_paths(&mut catalog, &["/old/a.jpg"])[0];
    let copy = catalog.create_virtual_copy(master).unwrap();
    let old_folder = catalog.photo(master).unwrap().folder_id;
    let removed = catalog.remove_photos(&[copy]).unwrap();

    // While the copy is gone the file moves, its old folder is forgotten,
    // and a scan reports the file missing.
    catalog
        .relink_photo(master, Path::new("/new/IMG_2.jpg"))
        .unwrap();
    catalog.remove_folder(old_folder).unwrap();
    catalog.set_missing(&[master], true).unwrap();

    catalog.restore_photos(&removed).unwrap();

    let (original, restored) = (catalog.photo(master).unwrap(), catalog.photo(copy).unwrap());
    assert_eq!(restored.path, PathBuf::from("/new/IMG_2.jpg"));
    assert_eq!(restored.file_name, "IMG_2.jpg");
    assert_eq!(restored.folder_id, original.folder_id);
    assert!(restored.missing);
    assert_eq!(restored.master_id, Some(master));
    // The old folder was not brought back just to hold a stale copy.
    let folders: Vec<PathBuf> = catalog
        .folders()
        .unwrap()
        .into_iter()
        .map(|f| f.path)
        .collect();
    assert_eq!(folders, [PathBuf::from("/new")]);
    let by_name = PhotoQuery {
        sort: Sort {
            key: SortKey::FileName,
            ..Sort::default()
        },
        ..PhotoQuery::default()
    };
    assert_eq!(
        common::names(&catalog, &by_name),
        ["IMG_2.jpg", "IMG_2.jpg"]
    );
    assert_eq!(catalog.check_integrity().unwrap(), Vec::<String>::new());
}

#[test]
fn a_restored_photo_returns_to_its_place_in_a_collection_that_was_reordered_meanwhile() {
    let mut catalog = memory();
    let ids = import_paths(
        &mut catalog,
        &["/c/a.jpg", "/c/b.jpg", "/c/c.jpg", "/c/d.jpg"],
    );
    let (a, b, c, d) = (ids[0], ids[1], ids[2], ids[3]);
    let album = catalog.create_collection("Album", None).unwrap();
    catalog.add_to_collection(album, &[d, c, b, a]).unwrap();

    // Undo stack, bottom to top: remove c; then reorder what is left.
    let removed = catalog.remove_photos(&[c]).unwrap();
    assert_eq!(order(&catalog, album), [d, b, a]);
    catalog.set_collection_order(album, &[a, b, d]).unwrap();

    // Undone in reverse: the old order first, then the removal.
    catalog.set_collection_order(album, &[d, b, a]).unwrap();
    catalog.restore_photos(&removed).unwrap();
    assert_eq!(order(&catalog, album), [d, c, b, a]);

    // Several photos at once, from the front, the middle and the end.
    let removed = catalog.remove_photos(&[d, b, a]).unwrap();
    assert_eq!(order(&catalog, album), [c]);
    catalog.set_collection_order(album, &[c]).unwrap();
    catalog.restore_photos(&removed).unwrap();
    assert_eq!(order(&catalog, album), [d, c, b, a]);
}

#[test]
fn entries_taken_out_of_a_collection_go_back_between_the_same_neighbours() {
    let mut catalog = memory();
    let ids = import_paths(
        &mut catalog,
        &["/c/a.jpg", "/c/b.jpg", "/c/c.jpg", "/c/d.jpg", "/c/e.jpg"],
    );
    let (a, b, c, d, e) = (ids[0], ids[1], ids[2], ids[3], ids[4]);
    let album = catalog.create_collection("Album", None).unwrap();
    catalog.add_to_collection(album, &[e, d, c, b, a]).unwrap();

    let taken = catalog.remove_from_collection(album, &[d, b]).unwrap();
    // Giving the rest a new order and back renumbers their positions...
    catalog.set_collection_order(album, &[a, c, e]).unwrap();
    catalog.set_collection_order(album, &[e, c, a]).unwrap();
    // ...and the entries still land where they were, in whatever order
    // they are handed back.
    let reversed: Vec<_> = taken.iter().rev().copied().collect();
    catalog
        .restore_collection_entries(album, &reversed)
        .unwrap();
    assert_eq!(order(&catalog, album), [e, d, c, b, a]);

    // New members still go to the very end afterwards.
    let late = import_paths(&mut catalog, &["/c/f.jpg"])[0];
    catalog.add_to_collection(album, &[late]).unwrap();
    assert_eq!(order(&catalog, album), [e, d, c, b, a, late]);
}

#[test]
fn develop_changes_made_before_a_removal_still_undo_and_redo_after_the_restore() {
    let mut catalog = memory();
    let photo = import_paths(&mut catalog, &["/r/a.jpg"])[0];
    let first = catalog
        .set_develop_settings(photo, &exposure(0.5), Some("A"))
        .unwrap();
    let second = catalog
        .set_develop_settings(photo, &exposure(1.0), Some("B"))
        .unwrap();
    let history_before = catalog.history(photo).unwrap();

    let removed = catalog.remove_photos(&[photo]).unwrap();
    catalog.restore_photos(&removed).unwrap();
    assert_eq!(catalog.history(photo).unwrap(), history_before);

    // The undo stack below the removal is still good.
    catalog.undo_develop(std::slice::from_ref(&second)).unwrap();
    assert_eq!(catalog.develop_settings(photo).unwrap(), exposure(0.5));
    catalog.undo_develop(std::slice::from_ref(&first)).unwrap();
    assert!(!catalog.photo(photo).unwrap().has_edits);
    assert!(catalog.history(photo).unwrap().is_empty());

    catalog.redo_develop(&[first, second]).unwrap();
    assert_eq!(catalog.develop_settings(photo).unwrap(), exposure(1.0));
    assert_eq!(catalog.history(photo).unwrap(), history_before);
}

#[test]
fn a_copy_whose_master_is_gone_cannot_be_restored() {
    let mut catalog = memory();
    let master = import_paths(&mut catalog, &["/g/a.jpg"])[0];
    let copy = catalog.create_virtual_copy(master).unwrap();
    let removed_copy = catalog.remove_photos(&[copy]).unwrap();
    catalog.remove_photos(&[master]).unwrap();
    let before = catalog.stats().unwrap();

    assert!(matches!(
        catalog.restore_photos(&removed_copy),
        Err(CatalogError::Sqlite(_))
    ));
    assert_eq!(catalog.stats().unwrap(), before);
    assert_eq!(catalog.count(&Source::AllPhotos).unwrap(), 0);
}
