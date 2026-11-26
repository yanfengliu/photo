//! Finding the catalogued files that have a given name, the way a file
//! system that ignores letter case would find them.

mod common;

use std::path::PathBuf;

use common::{import_paths, memory};

fn paths(paths: &[&str]) -> Vec<PathBuf> {
    paths.iter().map(PathBuf::from).collect()
}

#[test]
fn files_are_found_by_name_whatever_the_letter_case_and_the_folder() {
    let mut catalog = memory();
    let ids = import_paths(
        &mut catalog,
        &[
            "/card/IMG_1.JPG",
            "/shots/img_1.jpg",
            "/shots/IMG_2.jpg",
            "/shots/IMG_10.jpg",
            "/shots/Straße.jpg",
        ],
    );
    // A virtual copy shares its master's file and has none of its own.
    catalog.create_virtual_copy(ids[1]).unwrap();

    let found = catalog
        .files_named(&["IMG_1.jpg", "img_2.JPG", "IMG_3.jpg", "STRASSE.JPG"])
        .unwrap();
    assert_eq!(
        found,
        [
            paths(&["/card/IMG_1.JPG", "/shots/img_1.jpg"]),
            paths(&["/shots/IMG_2.jpg"]),
            paths(&[]),
            paths(&["/shots/Straße.jpg"]),
        ]
    );
}

#[test]
fn a_name_asked_for_twice_is_answered_twice_and_no_names_ask_nothing() {
    let mut catalog = memory();
    import_paths(&mut catalog, &["/shots/a.jpg"]);
    let found = catalog.files_named(&["a.jpg", "A.JPG"]).unwrap();
    assert_eq!(found, [paths(&["/shots/a.jpg"]), paths(&["/shots/a.jpg"])]);
    assert!(catalog.files_named(&[]).unwrap().is_empty());
}

#[test]
fn a_name_is_not_a_pattern() {
    let mut catalog = memory();
    import_paths(&mut catalog, &["/shots/IMG_1.jpg", "/shots/IMGX1.jpg"]);
    let found = catalog.files_named(&["IMG_1.jpg", "IMG%.jpg"]).unwrap();
    assert_eq!(found, [paths(&["/shots/IMG_1.jpg"]), paths(&[])]);
}
