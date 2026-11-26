//! A catalog another connection has locked: a change waits for it no longer
//! than it was told to, and the failure says that the catalog as a whole
//! was the problem, not what the change asked for.

mod common;

use std::time::{Duration, Instant};

use common::{import_paths, memory, raster};
use photo_catalog::{Catalog, CatalogError};
use rusqlite::Connection;

#[test]
fn changes_made_under_one_deadline_share_it_and_none_is_tried_once_it_has_passed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.photocat");
    let mut catalog = Catalog::open(&path).unwrap();
    let import = catalog.begin_import().unwrap();
    let budget = Duration::from_millis(400);
    catalog.set_deadline(Some(Instant::now() + budget)).unwrap();
    catalog
        .insert_photos(import, &[raster("/r/a.jpg")])
        .unwrap();

    // Another connection is in the middle of a change, and stays there,
    // while three quarters of the time go by.
    let other = Connection::open(&path).unwrap();
    other.execute_batch("BEGIN IMMEDIATE").unwrap();
    std::thread::sleep(budget * 3 / 4);
    let asked = Instant::now();
    let error = catalog
        .insert_photos(import, &[raster("/r/b.jpg")])
        .unwrap_err();
    let waited = asked.elapsed();
    assert!(error.is_catalog_wide(), "{error}");
    assert!(
        waited < budget / 2,
        "a change waits only for what is left of the deadline, not for all of it again: it waited {waited:?}"
    );

    // The deadline has passed: a change is not tried, though nothing is in
    // its way any more.
    other.execute_batch("ROLLBACK").unwrap();
    let error = catalog
        .insert_photos(import, &[raster("/r/b.jpg")])
        .unwrap_err();
    assert!(matches!(error, CatalogError::OutOfTime), "{error}");
    assert!(error.is_catalog_wide());
    assert!(catalog.find_by_path("/r/b.jpg").unwrap().is_none());

    // Without a deadline, it goes in.
    catalog.set_deadline(None).unwrap();
    catalog
        .insert_photos(import, &[raster("/r/b.jpg")])
        .unwrap();
}

#[test]
fn a_change_to_a_locked_catalog_fails_at_once_when_told_not_to_wait_and_says_it_was_the_catalog() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.photocat");
    let mut catalog = Catalog::open(&path).unwrap();
    let import = catalog.begin_import().unwrap();
    catalog.set_busy_timeout(Duration::ZERO).unwrap();
    // Another connection is in the middle of a change, and stays there.
    let other = Connection::open(&path).unwrap();
    other.execute_batch("BEGIN IMMEDIATE").unwrap();

    let error = catalog
        .insert_photos(import, &[raster("/r/a.jpg")])
        .unwrap_err();
    assert!(error.is_catalog_wide(), "{error}");
    let error = catalog
        .insert_photo(import, &raster("/r/a.jpg"))
        .unwrap_err();
    assert!(error.is_catalog_wide(), "{error}");

    // Once the other connection is done, the same change goes in.
    other.execute_batch("ROLLBACK").unwrap();
    let ids = catalog
        .insert_photos(import, &[raster("/r/a.jpg")])
        .unwrap();
    assert_eq!(ids.len(), 1);
}

#[test]
fn a_refusal_of_one_record_is_not_catalog_wide() {
    let mut catalog = memory();
    import_paths(&mut catalog, &["/r/a.jpg"]);
    let import = catalog.begin_import().unwrap();
    let error = catalog
        .insert_photos(import, &[raster("/r/b.jpg"), raster("/r/a.jpg")])
        .unwrap_err();
    assert!(matches!(error, CatalogError::DuplicatePath(_)), "{error}");
    assert!(!error.is_catalog_wide());
    let error = catalog.insert_photo(import, &raster("a.jpg")).unwrap_err();
    assert!(matches!(error, CatalogError::InvalidPath(_)), "{error}");
    assert!(!error.is_catalog_wide());
}
