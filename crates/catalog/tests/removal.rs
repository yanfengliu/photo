//! Removing photos from the catalog and putting them back.

mod common;

use std::path::Path;

use common::{date, import, import_paths, memory, raster, raw};
use photo_catalog::{
    Catalog, CatalogError, Entity, HistoryStep, PhotoQuery, PhotoRecord, PhotoSummary, Snapshot,
    Source,
};
use photo_core::develop::DevelopSettings;
use photo_core::meta::{CollectionId, ColorLabel, Flag, PhotoId, Rating};

fn exposure(stops: f32) -> DevelopSettings {
    let mut settings = DevelopSettings::default();
    settings.tone.exposure = stops;
    settings
}

/// Everything the public API can say about one photo, except its develop
/// version: a restore renews that on purpose (see `develop_versions.rs`),
/// and the tests here check it separately.
#[derive(Debug, PartialEq)]
struct Everything {
    record: PhotoRecord,
    settings: DevelopSettings,
    history: Vec<HistoryStep>,
    history_settings: Vec<DevelopSettings>,
    snapshots: Vec<Snapshot>,
    keywords: Vec<String>,
    collections: Vec<CollectionId>,
}

fn everything(catalog: &Catalog, photo: PhotoId) -> Everything {
    let history = catalog.history(photo).unwrap();
    let mut record = catalog.photo(photo).unwrap();
    record.develop_version = 0;
    Everything {
        record,
        settings: catalog.develop_settings(photo).unwrap(),
        history_settings: history
            .iter()
            .map(|step| catalog.history_settings(step.id).unwrap())
            .collect(),
        history,
        snapshots: catalog.snapshots(photo).unwrap(),
        keywords: catalog
            .photo_keywords(photo)
            .unwrap()
            .into_iter()
            .map(|keyword| keyword.name)
            .collect(),
        collections: catalog.photo_collections(photo).unwrap(),
    }
}

/// The catalog as the Library shows it: every cell (develop versions left
/// out, as in [`everything`]), and each collection's own order.
fn library_view(
    catalog: &Catalog,
    collections: &[CollectionId],
) -> (Vec<PhotoSummary>, Vec<Vec<PhotoId>>) {
    let mut cells = catalog.query(&PhotoQuery::default()).unwrap();
    for cell in &mut cells {
        cell.develop_version = 0;
    }
    (
        cells,
        collections
            .iter()
            .map(|id| catalog.collection_photos(*id).unwrap())
            .collect(),
    )
}

/// A master with a rich state, a virtual copy with its own, and a bystander.
struct Scene {
    catalog: Catalog,
    master: PhotoId,
    copy: PhotoId,
    bystander: PhotoId,
    album: CollectionId,
}

fn scene() -> Scene {
    let mut catalog = memory();
    let mut photo = raw("/shoot/DSC_0001.ARW");
    photo.file_size = 61_000_000;
    photo.metadata.capture_time = Some(date(2024, 8, 15));
    photo.metadata.iso = Some(200);
    photo.rating = Rating::new(4);
    photo.flag = Flag::Picked;
    photo.label = ColorLabel::Yellow;
    photo.title = "Harbour".into();
    photo.caption = "Low tide".into();
    photo.keywords = vec!["sea".into(), "boats".into()];
    let ids = import(&mut catalog, &[photo, raster("/shoot/bystander.jpg")]);
    let (master, bystander) = (ids[0], ids[1]);

    for (stops, label) in [
        (0.3, "Exposure"),
        (0.6, "More exposure"),
        (0.9, "Even more"),
    ] {
        catalog
            .set_develop_settings(master, &exposure(stops), Some(label))
            .unwrap();
    }
    let middle = catalog.history(master).unwrap()[1].id;
    catalog.go_to_history_step(master, middle).unwrap();
    catalog
        .create_snapshot(master, "Before", &exposure(0.3))
        .unwrap();
    catalog
        .create_snapshot(master, "After", &exposure(0.9))
        .unwrap();

    let copy = catalog.create_virtual_copy(master).unwrap();
    catalog
        .set_develop_settings(copy, &exposure(-1.0), Some("Darker"))
        .unwrap();
    catalog.set_ratings(&[(copy, Rating::new(1))]).unwrap();
    catalog.set_photo_keywords(copy, &["mono"]).unwrap();
    catalog
        .create_snapshot(copy, "Copy look", &exposure(-1.0))
        .unwrap();

    let album = catalog.create_collection("Album", None).unwrap();
    catalog
        .add_to_collection(album, &[bystander, master, copy])
        .unwrap();
    catalog
        .set_collection_order(album, &[copy, bystander, master])
        .unwrap();
    catalog
        .add_to_collection(catalog.quick_collection(), &[master])
        .unwrap();
    Scene {
        catalog,
        master,
        copy,
        bystander,
        album,
    }
}

#[test]
fn removing_a_master_removes_its_virtual_copies_and_everything_attached() {
    let Scene {
        mut catalog,
        master,
        copy,
        bystander,
        album,
    } = scene();
    let before = catalog.stats().unwrap();
    assert_eq!((before.photos, before.history, before.snapshots), (3, 4, 3));

    let removed = catalog.remove_photos(&[master]).unwrap();
    assert_eq!(
        removed.photo_ids(),
        [master, copy],
        "the copy is reported too"
    );

    let after = catalog.stats().unwrap();
    assert_eq!(
        (
            after.photos,
            after.history,
            after.snapshots,
            after.photo_keywords
        ),
        (1, 0, 0, 0)
    );
    assert_eq!(
        after.collection_photos, 1,
        "only the bystander's membership is left"
    );
    assert_eq!(after.keywords, before.keywords, "keywords themselves stay");
    assert_eq!(after.folders, before.folders);
    assert_eq!(catalog.check_integrity().unwrap(), Vec::<String>::new());

    for gone in [master, copy] {
        assert!(matches!(
            catalog.photo(gone),
            Err(CatalogError::NotFound {
                entity: Entity::Photo,
                ..
            })
        ));
        assert!(matches!(
            catalog.history(gone),
            Err(CatalogError::NotFound { .. })
        ));
    }
    assert_eq!(catalog.collection_photos(album).unwrap(), [bystander]);
    assert_eq!(catalog.count(&Source::AllPhotos).unwrap(), 1);
    assert_eq!(catalog.count(&Source::QuickCollection).unwrap(), 0);
    assert_eq!(catalog.find_by_path("/shoot/DSC_0001.ARW").unwrap(), None);
}

#[test]
fn restoring_reproduces_ids_attributes_history_snapshots_keywords_and_membership() {
    let Scene {
        mut catalog,
        master,
        copy,
        bystander,
        album,
    } = scene();
    let collections = [album, catalog.quick_collection()];
    let before = [master, copy, bystander].map(|id| everything(&catalog, id));
    let view_before = library_view(&catalog, &collections);
    let stats_before = catalog.stats().unwrap();
    let versions = |catalog: &Catalog| {
        [master, copy, bystander].map(|id| catalog.photo(id).unwrap().develop_version)
    };
    let versions_before = versions(&catalog);
    assert_eq!(before[0].history.len(), 3);
    assert!(
        before[0].history[1].is_current,
        "the scene left the master on its middle step"
    );

    let removed = catalog.remove_photos(&[master]).unwrap();
    catalog.restore_photos(&removed).unwrap();

    let after = [master, copy, bystander].map(|id| everything(&catalog, id));
    assert_eq!(after, before);
    let versions_after = versions(&catalog);
    assert!(
        versions_after[0] > versions_before[0] && versions_after[1] > versions_before[1],
        "the restored photos get new develop versions: {versions_before:?} -> {versions_after:?}"
    );
    assert_eq!(
        versions_after[2], versions_before[2],
        "the bystander keeps its version"
    );
    assert_eq!(library_view(&catalog, &collections), view_before);
    assert_eq!(catalog.stats().unwrap(), stats_before);
    assert_eq!(catalog.check_integrity().unwrap(), Vec::<String>::new());
    assert_eq!(
        catalog.find_by_path("/shoot/DSC_0001.ARW").unwrap(),
        Some(master)
    );

    // The restored photo is fully alive: editing continues from where it was
    // and new history steps get ids past every old one.
    let newest_old = before[0].history.last().unwrap().id;
    let change = catalog
        .set_develop_settings(master, &exposure(2.0), Some("After restore"))
        .unwrap();
    assert!(change.history_step().unwrap() > newest_old);
    let labels: Vec<String> = catalog
        .history(master)
        .unwrap()
        .into_iter()
        .map(|step| step.label)
        .collect();
    assert_eq!(labels, ["Exposure", "More exposure", "After restore"]);
}

#[test]
fn a_removal_can_be_redone_and_undone_again() {
    let Scene {
        mut catalog,
        master,
        copy,
        bystander,
        ..
    } = scene();
    let before = [master, copy, bystander].map(|id| everything(&catalog, id));
    for _ in 0..3 {
        let removed = catalog.remove_photos(&[master, copy]).unwrap();
        assert_eq!(catalog.stats().unwrap().photos, 1);
        catalog.restore_photos(&removed).unwrap();
        assert_eq!(
            [master, copy, bystander].map(|id| everything(&catalog, id)),
            before
        );
    }
}

#[test]
fn a_virtual_copy_can_be_removed_and_restored_on_its_own() {
    let Scene {
        mut catalog,
        master,
        copy,
        ..
    } = scene();
    let before = [master, copy].map(|id| everything(&catalog, id));

    let removed = catalog.remove_photos(&[copy]).unwrap();
    assert_eq!(removed.photo_ids(), [copy]);
    assert_eq!(
        everything(&catalog, master),
        before[0],
        "the master is untouched"
    );
    assert!(matches!(
        catalog.photo(copy),
        Err(CatalogError::NotFound { .. })
    ));

    catalog.restore_photos(&removed).unwrap();
    assert_eq!([master, copy].map(|id| everything(&catalog, id)), before);
}

#[test]
fn removing_is_all_or_nothing() {
    let Scene {
        mut catalog,
        master,
        bystander,
        ..
    } = scene();
    let before = catalog.stats().unwrap();
    let error = catalog
        .remove_photos(&[bystander, PhotoId(404), master])
        .unwrap_err();
    assert!(matches!(
        error,
        CatalogError::NotFound {
            entity: Entity::Photo,
            id: 404
        }
    ));
    assert_eq!(catalog.stats().unwrap(), before);
    assert!(catalog.remove_photos(&[]).unwrap().photo_ids().is_empty());
}

#[test]
fn ids_of_removed_photos_are_never_given_to_new_photos() {
    let mut catalog = memory();
    let ids = import_paths(&mut catalog, &["/n/a.jpg", "/n/b.jpg"]);
    let removed = catalog.remove_photos(&ids).unwrap();

    let newcomer = import_paths(&mut catalog, &["/n/c.jpg"])[0];
    assert!(
        newcomer > ids[1],
        "a cached rendering of a removed photo can never be shown for another"
    );
    catalog.restore_photos(&removed).unwrap();
    assert_eq!(catalog.count(&Source::AllPhotos).unwrap(), 3);
    assert_eq!(catalog.photo(ids[0]).unwrap().file_name, "a.jpg");
}

#[test]
fn restoring_waits_for_deleted_keywords_and_collections_and_recreates_a_removed_folder() {
    let Scene {
        mut catalog,
        master,
        copy,
        bystander,
        album,
    } = scene();
    let sea = catalog
        .photo_keywords(master)
        .unwrap()
        .into_iter()
        .find(|keyword| keyword.name == "sea")
        .unwrap();
    let folder = catalog.photo(master).unwrap().folder_id;
    let removed = catalog.remove_photos(&[master]).unwrap();

    // While the photos are gone, a keyword, a collection and (after moving
    // the bystander out) their folder are deleted.
    let keyword = catalog.delete_keyword(sea.id).unwrap();
    let collection = catalog.delete_collection(album).unwrap();
    catalog
        .relink_photo(bystander, Path::new("/elsewhere/bystander.jpg"))
        .unwrap();
    catalog.remove_folder(folder).unwrap();

    // The master cannot come back without the keyword it carried, so
    // rather than drop that link the restore is refused.
    let before = catalog.stats().unwrap();
    assert!(matches!(
        catalog.restore_photos(&removed),
        Err(CatalogError::NotFound { entity: Entity::Keyword, id }) if id == sea.id.0
    ));
    assert_eq!(catalog.stats().unwrap(), before);

    // Undone in reverse order, everything comes back.
    catalog.restore_collections(&collection).unwrap();
    catalog.restore_keywords(&keyword).unwrap();
    catalog.restore_photos(&removed).unwrap();

    let names = |id| -> Vec<String> {
        catalog
            .photo_keywords(id)
            .unwrap()
            .into_iter()
            .map(|keyword| keyword.name)
            .collect()
    };
    assert_eq!(names(master), ["boats", "sea"]);
    assert_eq!(names(copy), ["mono"]);
    assert_eq!(
        catalog.photo_collections(master).unwrap(),
        [catalog.quick_collection(), album]
    );
    assert_eq!(
        catalog.collection_photos(album).unwrap(),
        [copy, bystander, master]
    );
    let restored_folder = catalog.photo(master).unwrap().folder_id;
    let folders = catalog.folders().unwrap();
    let listed = folders.iter().find(|f| f.id == restored_folder).unwrap();
    assert_eq!(
        (listed.path.to_str(), listed.photo_count),
        (Some("/shoot"), 2)
    );
    assert_eq!(catalog.photo(copy).unwrap().folder_id, restored_folder);
    assert_eq!(catalog.history(master).unwrap().len(), 3);
    assert_eq!(catalog.check_integrity().unwrap(), Vec::<String>::new());
}

#[test]
fn restoring_is_refused_when_the_file_was_imported_again_or_is_already_back() {
    let Scene {
        mut catalog,
        master,
        ..
    } = scene();
    let removed = catalog.remove_photos(&[master]).unwrap();
    let reimported = import_paths(&mut catalog, &["/shoot/DSC_0001.ARW"])[0];
    let before = catalog.stats().unwrap();

    let error = catalog.restore_photos(&removed).unwrap_err();
    assert!(matches!(error, CatalogError::DuplicatePath(path) if path == "/shoot/DSC_0001.ARW"));
    assert_eq!(
        catalog.stats().unwrap(),
        before,
        "a refused restore restores nothing"
    );

    catalog.remove_photos(&[reimported]).unwrap();
    catalog.restore_photos(&removed).unwrap();
    assert!(
        matches!(
            catalog.restore_photos(&removed),
            Err(CatalogError::AlreadyExists { entity: Entity::Photo, id }) if id == master.0
        ),
        "restoring twice cannot duplicate anything"
    );
    assert_eq!(catalog.stats().unwrap().photos, 3);
    assert_eq!(catalog.check_integrity().unwrap(), Vec::<String>::new());
}
