//! Deleting keywords and collections, undoably: a deletion returns what it
//! took, and restoring that puts everything back under the same ids.

mod common;

use common::smart::rule;
use common::{import_paths, memory};
use photo_catalog::{
    Catalog, CatalogError, CollectionInfo, CollectionKind, Entity, KeywordInfo, SmartField,
    SmartOp, SmartRules,
};
use photo_core::meta::{CollectionId, KeywordId, PhotoId};

/// The keyword panel, and the keywords of each photo.
fn keyword_state(catalog: &Catalog, photos: &[PhotoId]) -> (Vec<KeywordInfo>, Vec<Vec<KeywordId>>) {
    let tagged = photos
        .iter()
        .map(|photo| {
            let keywords = catalog.photo_keywords(*photo).unwrap();
            keywords.into_iter().map(|keyword| keyword.id).collect()
        })
        .collect();
    (catalog.keywords().unwrap(), tagged)
}

/// The collections panel, and the members of each regular collection in
/// their order.
fn collection_state(catalog: &Catalog) -> (Vec<CollectionInfo>, Vec<Vec<PhotoId>>) {
    let listed = catalog.collections().unwrap();
    let members = listed
        .iter()
        .filter(|collection| collection.kind == CollectionKind::Regular)
        .map(|collection| catalog.collection_photos(collection.id).unwrap())
        .collect();
    (listed, members)
}

fn clean(catalog: &Catalog) {
    assert_eq!(catalog.check_integrity().unwrap(), Vec::<String>::new());
}

#[test]
fn a_deleted_keyword_comes_back_with_its_ids_hierarchy_and_assignments() {
    let mut catalog = memory();
    let photos = import_paths(&mut catalog, &["/k/a.jpg", "/k/b.jpg"]);
    // Created first, so its id is smaller than that of the keyword it ends
    // up under: restoring in id order would put it back before its parent.
    let older = catalog.create_keyword("Older", None).unwrap();
    let places = catalog.create_keyword("Places", None).unwrap();
    let france = catalog.create_keyword("France", Some(places)).unwrap();
    let paris = catalog.create_keyword("Paris", Some(france)).unwrap();
    catalog.create_keyword("Italy", Some(places)).unwrap();
    catalog.move_keyword(older, Some(paris)).unwrap();
    catalog.assign_keyword(france, &photos[..1]).unwrap();
    catalog.assign_keyword(paris, &photos).unwrap();
    catalog.assign_keyword(older, &photos[1..]).unwrap();
    let before = keyword_state(&catalog, &photos);

    let deleted = catalog.delete_keyword(france).unwrap();
    assert_eq!(deleted.keyword_ids(), [france, paris, older]);
    let names: Vec<String> = catalog
        .keywords()
        .unwrap()
        .into_iter()
        .map(|k| k.name)
        .collect();
    assert_eq!(names, ["Italy", "Places"]);
    assert!(catalog.photo_keywords(photos[1]).unwrap().is_empty());
    clean(&catalog);

    catalog.restore_keywords(&deleted).unwrap();
    assert_eq!(keyword_state(&catalog, &photos), before);
    clean(&catalog);
}

#[test]
fn a_deleted_collection_set_comes_back_with_its_ids_kinds_rules_and_ordered_members() {
    let mut catalog = memory();
    let photos = import_paths(&mut catalog, &["/c/a.jpg", "/c/b.jpg", "/c/c.jpg"]);
    let older = catalog.create_collection("Older", None).unwrap();
    let trips = catalog.create_collection_set("Trips", None).unwrap();
    let summer = catalog.create_collection("Summer", Some(trips)).unwrap();
    let rules = SmartRules {
        match_all: false,
        rules: vec![rule(SmartField::Rating, SmartOp::AtLeast, 3)],
    };
    let rated = catalog
        .create_smart_collection("Rated", Some(trips), &rules)
        .unwrap();
    let nested = catalog
        .create_collection_set("Nested", Some(trips))
        .unwrap();
    catalog.move_collection(older, Some(nested)).unwrap();
    catalog.add_to_collection(summer, &photos).unwrap();
    catalog
        .set_collection_order(summer, &[photos[2], photos[0], photos[1]])
        .unwrap();
    catalog.add_to_collection(older, &photos[1..2]).unwrap();
    let keep = catalog.create_collection("Keep", None).unwrap();
    catalog.add_to_collection(keep, &photos[..1]).unwrap();
    let before = collection_state(&catalog);

    let deleted = catalog.delete_collection(trips).unwrap();
    assert_eq!(
        deleted.collection_ids(),
        [trips, summer, rated, nested, older]
    );
    let listed: Vec<CollectionId> = catalog
        .collections()
        .unwrap()
        .into_iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(listed, [keep]);
    assert!(catalog.photo_collections(photos[1]).unwrap().is_empty());
    clean(&catalog);

    catalog.restore_collections(&deleted).unwrap();
    assert_eq!(collection_state(&catalog), before);
    assert_eq!(catalog.smart_rules(rated).unwrap(), rules);
    clean(&catalog);
}

/// The review's scenario, and its mirror image: a photo that carries a
/// keyword is removed and the keyword deleted, one after the other.
#[test]
fn a_photo_and_its_keyword_taken_away_in_either_order_come_back_linked() {
    for photo_first in [true, false] {
        let mut catalog = memory();
        let photo = import_paths(&mut catalog, &["/x/p.jpg"])[0];
        let keyword = catalog.create_keyword("K", None).unwrap();
        catalog.assign_keyword(keyword, &[photo]).unwrap();
        let (removed, deleted) = if photo_first {
            let removed = catalog.remove_photos(&[photo]).unwrap();
            (removed, catalog.delete_keyword(keyword).unwrap())
        } else {
            let deleted = catalog.delete_keyword(keyword).unwrap();
            (catalog.remove_photos(&[photo]).unwrap(), deleted)
        };
        let emptied = catalog.stats().unwrap();

        // Restored in the order they were taken away, the first restore
        // could only bring its half back by dropping the link, so it is
        // refused and restores nothing.
        let (refused, missing) = if photo_first {
            (
                catalog.restore_photos(&removed),
                (Entity::Keyword, keyword.0),
            )
        } else {
            (catalog.restore_keywords(&deleted), (Entity::Photo, photo.0))
        };
        assert!(
            matches!(refused, Err(CatalogError::NotFound { entity, id }) if (entity, id) == missing),
            "photo first: {photo_first}"
        );
        assert_eq!(catalog.stats().unwrap(), emptied);

        // Restored the other way round, as an undo stack does, nothing is lost.
        if photo_first {
            catalog.restore_keywords(&deleted).unwrap();
            catalog.restore_photos(&removed).unwrap();
        } else {
            catalog.restore_photos(&removed).unwrap();
            catalog.restore_keywords(&deleted).unwrap();
        }
        let ids: Vec<KeywordId> = catalog
            .photo_keywords(photo)
            .unwrap()
            .into_iter()
            .map(|keyword| keyword.id)
            .collect();
        assert_eq!(ids, [keyword], "photo first: {photo_first}");
        clean(&catalog);
    }
}

#[test]
fn a_photo_and_its_collection_taken_away_in_either_order_come_back_with_the_photo_in_its_place() {
    for photo_first in [true, false] {
        let mut catalog = memory();
        let photos = import_paths(&mut catalog, &["/x/a.jpg", "/x/b.jpg", "/x/c.jpg"]);
        let album = catalog.create_collection("Album", None).unwrap();
        catalog.add_to_collection(album, &photos).unwrap();
        let (removed, deleted) = if photo_first {
            let removed = catalog.remove_photos(&photos[1..2]).unwrap();
            (removed, catalog.delete_collection(album).unwrap())
        } else {
            let deleted = catalog.delete_collection(album).unwrap();
            (catalog.remove_photos(&photos[1..2]).unwrap(), deleted)
        };
        let emptied = catalog.stats().unwrap();

        let (refused, missing) = if photo_first {
            (
                catalog.restore_photos(&removed),
                (Entity::Collection, album.0),
            )
        } else {
            (
                catalog.restore_collections(&deleted),
                (Entity::Photo, photos[1].0),
            )
        };
        assert!(
            matches!(refused, Err(CatalogError::NotFound { entity, id }) if (entity, id) == missing),
            "photo first: {photo_first}"
        );
        assert_eq!(catalog.stats().unwrap(), emptied);

        if photo_first {
            catalog.restore_collections(&deleted).unwrap();
            catalog.restore_photos(&removed).unwrap();
        } else {
            catalog.restore_photos(&removed).unwrap();
            catalog.restore_collections(&deleted).unwrap();
        }
        assert_eq!(catalog.collection_photos(album).unwrap(), photos);
        clean(&catalog);
    }
}

#[test]
fn a_restore_that_cannot_put_things_back_exactly_is_refused() {
    let mut catalog = memory();
    let set = catalog.create_collection_set("Set", None).unwrap();
    let inner = catalog.create_collection("Inner", Some(set)).unwrap();
    let deleted = catalog.delete_collection(inner).unwrap();

    // Another collection has taken the name in the meantime.
    let usurper = catalog.create_collection("INNER", Some(set)).unwrap();
    assert!(matches!(
        catalog.restore_collections(&deleted),
        Err(CatalogError::NameTaken(name)) if name == "Inner"
    ));
    catalog.delete_collection(usurper).unwrap();
    catalog.restore_collections(&deleted).unwrap();
    // A restore can only happen once.
    assert!(matches!(
        catalog.restore_collections(&deleted),
        Err(CatalogError::AlreadyExists { entity: Entity::Collection, id }) if id == inner.0
    ));
    // The set it was in has gone.
    let deleted = catalog.delete_collection(inner).unwrap();
    catalog.delete_collection(set).unwrap();
    assert!(matches!(
        catalog.restore_collections(&deleted),
        Err(CatalogError::NotFound { entity: Entity::Collection, id }) if id == set.0
    ));
    assert!(catalog.collections().unwrap().is_empty());

    let parent = catalog.create_keyword("Parent", None).unwrap();
    let child = catalog.create_keyword("Child", Some(parent)).unwrap();
    let deleted = catalog.delete_keyword(child).unwrap();
    let usurper = catalog.create_keyword("child", Some(parent)).unwrap();
    assert!(matches!(
        catalog.restore_keywords(&deleted),
        Err(CatalogError::NameTaken(name)) if name == "Child"
    ));
    catalog.delete_keyword(usurper).unwrap();
    catalog.restore_keywords(&deleted).unwrap();
    assert!(matches!(
        catalog.restore_keywords(&deleted),
        Err(CatalogError::AlreadyExists { entity: Entity::Keyword, id }) if id == child.0
    ));
    let deleted = catalog.delete_keyword(child).unwrap();
    catalog.delete_keyword(parent).unwrap();
    assert!(matches!(
        catalog.restore_keywords(&deleted),
        Err(CatalogError::NotFound { entity: Entity::Keyword, id }) if id == parent.0
    ));
    assert!(catalog.keywords().unwrap().is_empty());
    clean(&catalog);
}
