//! A photo's develop version keys its cached renderings, so it must never
//! name two different looks: it only ever grows, for the life of the catalog,
//! whatever is undone, redone, removed and restored.

mod common;

use common::{import_paths, memory};
use photo_catalog::Catalog;
use photo_core::develop::DevelopSettings;
use photo_core::meta::PhotoId;

fn exposure(stops: f32) -> DevelopSettings {
    let mut settings = DevelopSettings::default();
    settings.tone.exposure = stops;
    settings
}

/// The versions one photo has shown so far.
struct Versions {
    photo: PhotoId,
    seen: Vec<u64>,
}

impl Versions {
    fn start(catalog: &Catalog, photo: PhotoId) -> Self {
        Self {
            photo,
            seen: vec![catalog.photo(photo).unwrap().develop_version],
        }
    }

    /// The photo's version now, which must be newer than every earlier one.
    fn note(&mut self, catalog: &Catalog, step: &str) -> u64 {
        let version = catalog.photo(self.photo).unwrap().develop_version;
        assert!(
            self.seen.iter().all(|earlier| version > *earlier),
            "after {step} the version is {version}, but the photo has already had {:?}",
            self.seen
        );
        self.seen.push(version);
        version
    }
}

#[test]
fn a_version_never_comes_round_again_however_the_photo_is_removed_and_restored() {
    // The sequence the review found: without one counter for the whole
    // catalog, version 2 ended up naming two different looks.
    let mut catalog = memory();
    let photo = import_paths(&mut catalog, &["/v/a.jpg"])[0];
    let mut versions = Versions::start(&catalog, photo);

    let a = catalog
        .set_develop_settings(photo, &exposure(0.5), Some("A"))
        .unwrap();
    versions.note(&catalog, "edit A");
    let first = catalog.remove_photos(&[photo]).unwrap();
    catalog.restore_photos(&first).unwrap();
    versions.note(&catalog, "the first restore");
    catalog.undo_develop(std::slice::from_ref(&a)).unwrap();
    versions.note(&catalog, "undo A");
    catalog.redo_develop(std::slice::from_ref(&a)).unwrap();
    versions.note(&catalog, "redo A");
    catalog.remove_photos(&[photo]).unwrap();
    catalog.restore_photos(&first).unwrap();
    versions.note(&catalog, "restoring the first record again");
    catalog
        .set_develop_settings(photo, &exposure(1.0), Some("B"))
        .unwrap();
    versions.note(&catalog, "edit B");
}

#[test]
fn versions_keep_growing_in_the_next_session_after_the_newest_one_left_with_its_photo() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.photocat");
    let mut catalog = Catalog::open(&path).unwrap();
    let ids = import_paths(&mut catalog, &["/v/a.jpg", "/v/b.jpg"]);
    let mut versions = Versions::start(&catalog, ids[0]);
    for stops in [0.1, 0.2, 0.3] {
        catalog
            .set_develop_settings(ids[0], &exposure(stops), None)
            .unwrap();
    }
    let newest = versions.note(&catalog, "three edits");
    // The only photo that carries the newest version leaves the catalog.
    let removed = catalog.remove_photos(&[ids[0]]).unwrap();
    drop(catalog);

    let mut catalog = Catalog::open(&path).unwrap();
    catalog
        .set_develop_settings(ids[1], &exposure(0.5), None)
        .unwrap();
    assert!(
        catalog.photo(ids[1]).unwrap().develop_version > newest,
        "the counter carried over from the last session"
    );
    catalog.restore_photos(&removed).unwrap();
    versions.note(&catalog, "a restore in the next session");
    catalog
        .set_develop_settings(ids[0], &exposure(0.4), None)
        .unwrap();
    versions.note(&catalog, "an edit after the restore");
}
