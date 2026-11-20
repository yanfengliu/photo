//! A catalog on a write-protected file opens for reading, says so, and
//! refuses every change with a typed error rather than a bare SQLite one.

mod common;

use std::fs;
use std::path::Path;

use common::import_paths;
use photo_catalog::{Catalog, CatalogError};
use photo_core::meta::Rating;

/// Make the file read-only. False if this user can write it anyway (root
/// ignores the read-only bit), in which case there is nothing to test.
fn protect(path: &Path) -> bool {
    set_read_only(path, true);
    fs::OpenOptions::new().write(true).open(path).is_err()
}

#[allow(clippy::permissions_set_readonly_false)] // a test file, deleted next
fn set_read_only(path: &Path, read_only: bool) {
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_readonly(read_only);
    fs::set_permissions(path, permissions).unwrap();
}

#[test]
fn a_write_protected_catalog_opens_for_reading_and_refuses_changes_with_a_typed_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.photocat");
    let photo = {
        let mut catalog = Catalog::open(&path).unwrap();
        assert!(!catalog.is_read_only());
        import_paths(&mut catalog, &["/r/a.jpg"])[0]
    };
    if !protect(&path) {
        eprintln!("skipped: this user can write a read-only file");
        return;
    }
    let before = fs::read(&path).unwrap();

    let mut catalog = Catalog::open(&path).unwrap();
    assert!(catalog.is_read_only());
    assert_eq!(catalog.photo(photo).unwrap().file_name, "a.jpg");
    assert!(matches!(
        catalog.set_ratings(&[(photo, Rating::new(3))]),
        Err(CatalogError::ReadOnly)
    ));
    assert!(matches!(
        catalog.create_keyword("New", None),
        Err(CatalogError::ReadOnly)
    ));
    assert_eq!(catalog.photo(photo).unwrap().rating, Rating::new(0));
    drop(catalog);
    assert!(fs::read(&path).unwrap() == before, "the file changed");
    set_read_only(&path, false);
}

#[test]
fn a_write_protected_file_that_would_need_setting_up_is_refused_at_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.photocat");
    fs::write(&path, b"").unwrap();
    if !protect(&path) {
        eprintln!("skipped: this user can write a read-only file");
        return;
    }
    assert!(matches!(Catalog::open(&path), Err(CatalogError::ReadOnly)));
    assert!(fs::read(&path).unwrap().is_empty());
    set_read_only(&path, false);
}
