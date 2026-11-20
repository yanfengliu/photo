//! Collection sets, collections, the quick collection, and membership.

mod common;

use common::{import_paths, memory, names};
use photo_catalog::{
    Catalog, CatalogError, CollectionEntry, CollectionInfo, CollectionKind, Entity, PhotoQuery,
    SmartRules, Sort, SortDirection, SortKey, Source,
};
use photo_core::meta::{CollectionId, PhotoId};

fn listing(catalog: &Catalog) -> Vec<(String, CollectionKind, Option<CollectionId>, u64)> {
    let collections: Vec<CollectionInfo> = catalog.collections().unwrap();
    collections
        .into_iter()
        .map(|c| (c.name, c.kind, c.parent, c.photo_count))
        .collect()
}

fn custom_order(catalog: &Catalog, collection: CollectionId) -> Vec<String> {
    let query = PhotoQuery {
        source: Source::Collection(collection),
        sort: Sort {
            key: SortKey::Custom,
            direction: SortDirection::Ascending,
        },
        ..PhotoQuery::default()
    };
    names(catalog, &query)
}

fn photos(catalog: &mut Catalog) -> Vec<PhotoId> {
    import_paths(catalog, &["/c/a.jpg", "/c/b.jpg", "/c/c.jpg", "/c/d.jpg"])
}

#[test]
fn sets_hold_collections_and_the_list_carries_kind_parent_and_count() {
    let mut catalog = memory();
    let ids = photos(&mut catalog);
    let trips = catalog.create_collection_set("Trips", None).unwrap();
    let japan = catalog
        .create_collection("Japan 2024", Some(trips))
        .unwrap();
    let alps = catalog.create_collection("alps 10", Some(trips)).unwrap();
    let alps9 = catalog.create_collection("Alps 9", Some(trips)).unwrap();
    let best = catalog
        .create_smart_collection("Best", None, &SmartRules::default())
        .unwrap();
    catalog.add_to_collection(japan, &ids[..3]).unwrap();
    catalog.add_to_collection(alps, &ids[2..]).unwrap();

    assert_eq!(
        listing(&catalog),
        [
            (
                "Alps 9".to_string(),
                CollectionKind::Regular,
                Some(trips),
                0
            ),
            (
                "alps 10".to_string(),
                CollectionKind::Regular,
                Some(trips),
                2
            ),
            ("Best".to_string(), CollectionKind::Smart, None, 4),
            (
                "Japan 2024".to_string(),
                CollectionKind::Regular,
                Some(trips),
                3
            ),
            // A set counts each photo of its collections once.
            ("Trips".to_string(), CollectionKind::Set, None, 4),
        ]
    );
    assert_eq!(
        names(&catalog, &PhotoQuery::from(Source::Collection(trips))),
        ["a.jpg", "b.jpg", "c.jpg", "d.jpg"]
    );
    assert_eq!(catalog.count(&Source::Collection(japan)).unwrap(), 3);
    assert!(
        listing(&catalog)
            .iter()
            .all(|entry| entry.0 != "Quick Collection")
    );
    let _ = (alps9, best);
}

#[test]
fn only_sets_can_contain_collections() {
    let mut catalog = memory();
    let plain = catalog.create_collection("Plain", None).unwrap();
    let smart = catalog
        .create_smart_collection("Smart", None, &SmartRules::default())
        .unwrap();
    for parent in [plain, smart, catalog.quick_collection()] {
        let refused = catalog
            .create_collection("Child", Some(parent))
            .unwrap_err();
        assert!(matches!(refused, CatalogError::CollectionKind { id, .. } if id == parent));
    }
    assert!(matches!(
        catalog.create_collection_set("Lost", Some(CollectionId(404))),
        Err(CatalogError::NotFound {
            entity: Entity::Collection,
            id: 404
        })
    ));
    assert_eq!(listing(&catalog).len(), 2);
}

#[test]
fn sibling_names_are_unique_ignoring_case() {
    let mut catalog = memory();
    let set = catalog.create_collection_set("Work", None).unwrap();
    catalog.create_collection("Selects", Some(set)).unwrap();
    catalog.create_collection("Selects", None).unwrap();
    assert!(matches!(
        catalog.create_collection("SELECTS", Some(set)),
        Err(CatalogError::NameTaken(_))
    ));
    assert!(matches!(
        catalog.create_collection_set("selects", Some(set)),
        Err(CatalogError::NameTaken(_))
    ));
    assert!(matches!(
        catalog.create_collection("work", None),
        Err(CatalogError::NameTaken(_))
    ));
    assert!(matches!(
        catalog.create_collection("  ", None),
        Err(CatalogError::EmptyName)
    ));
    assert!(matches!(
        catalog.create_collection("quick collection", None),
        Err(CatalogError::NameTaken(_))
    ));
}

#[test]
fn collections_can_be_renamed_and_moved_between_sets() {
    let mut catalog = memory();
    let ids = photos(&mut catalog);
    let outer = catalog.create_collection_set("Outer", None).unwrap();
    let inner = catalog.create_collection_set("Inner", Some(outer)).unwrap();
    let other = catalog.create_collection_set("Other", None).unwrap();
    let album = catalog.create_collection("Album", Some(inner)).unwrap();
    catalog.add_to_collection(album, &ids[..2]).unwrap();
    let parent_of = |catalog: &Catalog, id| {
        catalog
            .collections()
            .unwrap()
            .into_iter()
            .find(|c| c.id == id)
            .unwrap()
            .parent
    };

    catalog.rename_collection(album, "  Portfolio ").unwrap();
    catalog.move_collection(album, Some(other)).unwrap();
    assert_eq!(parent_of(&catalog, album), Some(other));
    assert_eq!(
        catalog.count(&Source::Collection(album)).unwrap(),
        2,
        "membership survives rename and move"
    );
    assert_eq!(catalog.count(&Source::Collection(outer)).unwrap(), 0);
    catalog.move_collection(album, None).unwrap();
    assert_eq!(parent_of(&catalog, album), None);

    // A set cannot move into itself or its own descendants, nor under a
    // collection, nor onto a sibling of the same name.
    for target in [outer, inner] {
        assert!(matches!(
            catalog.move_collection(outer, Some(target)),
            Err(CatalogError::Cycle {
                entity: Entity::Collection,
                ..
            })
        ));
    }
    assert!(matches!(
        catalog.move_collection(inner, Some(album)),
        Err(CatalogError::CollectionKind { .. })
    ));
    catalog.create_collection("Portfolio", Some(other)).unwrap();
    assert!(matches!(
        catalog.move_collection(album, Some(other)),
        Err(CatalogError::NameTaken(_))
    ));
    assert!(matches!(
        catalog.rename_collection(album, "OTHER"),
        Err(CatalogError::NameTaken(_))
    ));
    assert_eq!(parent_of(&catalog, inner), Some(outer));
    assert_eq!(parent_of(&catalog, album), None);
}

#[test]
fn adding_reports_new_members_and_keeps_the_order_of_addition() {
    let mut catalog = memory();
    let ids = photos(&mut catalog);
    let album = catalog.create_collection("Album", None).unwrap();

    assert_eq!(
        catalog.add_to_collection(album, &[ids[2], ids[0]]).unwrap(),
        [ids[2], ids[0]]
    );
    // Photos already in the collection keep their place and are not reported.
    assert_eq!(
        catalog
            .add_to_collection(album, &[ids[0], ids[3], ids[3], ids[1]])
            .unwrap(),
        [ids[3], ids[1]]
    );
    assert_eq!(
        custom_order(&catalog, album),
        ["c.jpg", "a.jpg", "d.jpg", "b.jpg"]
    );
    assert_eq!(
        catalog.collection_photos(album).unwrap(),
        [ids[2], ids[0], ids[3], ids[1]]
    );

    // Any other sort key still works inside a collection.
    assert_eq!(
        names(&catalog, &PhotoQuery::from(Source::Collection(album))),
        ["a.jpg", "b.jpg", "c.jpg", "d.jpg"]
    );

    let unknown = catalog
        .add_to_collection(album, &[PhotoId(404)])
        .unwrap_err();
    assert!(matches!(
        unknown,
        CatalogError::NotFound {
            entity: Entity::Photo,
            id: 404
        }
    ));
}

#[test]
fn removed_members_can_be_put_back_exactly_where_they_were() {
    let mut catalog = memory();
    let ids = photos(&mut catalog);
    let album = catalog.create_collection("Album", None).unwrap();
    catalog.add_to_collection(album, &ids).unwrap();

    let removed: Vec<CollectionEntry> = catalog
        .remove_from_collection(album, &[ids[1], ids[2], PhotoId(404)])
        .unwrap();
    assert_eq!(
        removed.iter().map(|entry| entry.photo).collect::<Vec<_>>(),
        [ids[1], ids[2]]
    );
    assert_eq!(custom_order(&catalog, album), ["a.jpg", "d.jpg"]);
    assert!(
        catalog
            .remove_from_collection(album, &[ids[1]])
            .unwrap()
            .is_empty()
    );

    catalog.restore_collection_entries(album, &removed).unwrap();
    assert_eq!(
        custom_order(&catalog, album),
        ["a.jpg", "b.jpg", "c.jpg", "d.jpg"]
    );
    // Putting back something that is already there changes nothing.
    catalog.restore_collection_entries(album, &removed).unwrap();
    assert_eq!(catalog.count(&Source::Collection(album)).unwrap(), 4);
}

#[test]
fn the_order_inside_a_collection_can_be_set() {
    let mut catalog = memory();
    let ids = photos(&mut catalog);
    let album = catalog.create_collection("Album", None).unwrap();
    catalog.add_to_collection(album, &ids).unwrap();

    catalog
        .set_collection_order(album, &[ids[3], ids[1], ids[0], ids[2]])
        .unwrap();
    assert_eq!(
        custom_order(&catalog, album),
        ["d.jpg", "b.jpg", "a.jpg", "c.jpg"]
    );
    // Photos added later still go to the end.
    let late = import_paths(&mut catalog, &["/c/e.jpg"]);
    catalog.add_to_collection(album, &late).unwrap();
    assert_eq!(
        custom_order(&catalog, album),
        ["d.jpg", "b.jpg", "a.jpg", "c.jpg", "e.jpg"]
    );

    // The new order must name exactly the members, each once.
    let all_but_one = vec![ids[0], ids[1], ids[2], ids[3]];
    let one_twice = vec![ids[0], ids[1], ids[2], ids[3], ids[3]];
    let a_stranger = vec![ids[0], ids[1], ids[2], ids[3], PhotoId(404)];
    for order in [all_but_one, one_twice, a_stranger, Vec::new()] {
        assert!(matches!(
            catalog.set_collection_order(album, &order),
            Err(CatalogError::OrderMismatch(id)) if id == album
        ));
    }
    assert_eq!(
        custom_order(&catalog, album),
        ["d.jpg", "b.jpg", "a.jpg", "c.jpg", "e.jpg"]
    );
}

#[test]
fn photos_can_only_be_put_into_plain_collections_and_the_quick_collection() {
    let mut catalog = memory();
    let ids = photos(&mut catalog);
    let set = catalog.create_collection_set("Set", None).unwrap();
    let smart = catalog
        .create_smart_collection("Smart", None, &SmartRules::default())
        .unwrap();
    for id in [set, smart] {
        assert!(matches!(
            catalog.add_to_collection(id, &ids),
            Err(CatalogError::CollectionKind { .. })
        ));
        assert!(matches!(
            catalog.remove_from_collection(id, &ids),
            Err(CatalogError::CollectionKind { .. })
        ));
        assert!(matches!(
            catalog.set_collection_order(id, &[]),
            Err(CatalogError::CollectionKind { .. })
        ));
    }
    assert!(matches!(
        catalog.smart_rules(set),
        Err(CatalogError::CollectionKind { .. })
    ));
    assert!(matches!(
        catalog.set_smart_rules(set, &SmartRules::default()),
        Err(CatalogError::CollectionKind { .. })
    ));
    assert_eq!(catalog.stats().unwrap().collection_photos, 0);
}

#[test]
fn the_quick_collection_is_built_in_and_cannot_be_changed_itself() {
    let mut catalog = memory();
    let ids = photos(&mut catalog);
    let quick = catalog.quick_collection();
    assert_eq!(catalog.count(&Source::QuickCollection).unwrap(), 0);

    assert_eq!(
        catalog
            .add_to_collection(quick, &[ids[1], ids[0]])
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        names(&catalog, &PhotoQuery::from(Source::QuickCollection)),
        ["a.jpg", "b.jpg"]
    );
    assert_eq!(custom_order(&catalog, quick), ["b.jpg", "a.jpg"]);
    let cells = catalog.query(&PhotoQuery::default()).unwrap();
    assert_eq!(
        cells
            .iter()
            .map(|cell| cell.in_quick_collection)
            .collect::<Vec<_>>(),
        [true, true, false, false]
    );

    catalog.remove_from_collection(quick, &[ids[1]]).unwrap();
    assert_eq!(catalog.count(&Source::QuickCollection).unwrap(), 1);
    assert_eq!(catalog.photo_collections(ids[0]).unwrap(), [quick]);

    for refused in [
        catalog.rename_collection(quick, "Mine"),
        catalog.move_collection(quick, None),
        catalog.delete_collection(quick),
    ] {
        assert!(matches!(
            refused,
            Err(CatalogError::CollectionKind {
                kind: CollectionKind::Quick,
                ..
            })
        ));
    }
    assert_eq!(catalog.quick_collection(), quick);
}

#[test]
fn deleting_a_set_deletes_what_is_inside_it_and_leaves_no_membership_rows() {
    let mut catalog = memory();
    let ids = photos(&mut catalog);
    let set = catalog.create_collection_set("Set", None).unwrap();
    let nested = catalog.create_collection_set("Nested", Some(set)).unwrap();
    let inner = catalog.create_collection("Inner", Some(nested)).unwrap();
    let outer = catalog.create_collection("Outer", None).unwrap();
    catalog
        .create_smart_collection("Rules", Some(set), &SmartRules::default())
        .unwrap();
    catalog.add_to_collection(inner, &ids).unwrap();
    catalog.add_to_collection(outer, &ids[..2]).unwrap();
    catalog
        .add_to_collection(catalog.quick_collection(), &ids[..1])
        .unwrap();
    assert_eq!(catalog.stats().unwrap().collection_photos, 7);
    let mut memberships = catalog.photo_collections(ids[0]).unwrap();
    memberships.sort();
    assert_eq!(memberships, [catalog.quick_collection(), inner, outer]);

    catalog.delete_collection(set).unwrap();

    let stats = catalog.stats().unwrap();
    assert_eq!(
        stats.collection_photos, 3,
        "only the outer and quick memberships remain"
    );
    assert_eq!(
        stats.photos, 4,
        "photos are never deleted with a collection"
    );
    assert_eq!(listing(&catalog).len(), 1);
    assert_eq!(catalog.check_integrity().unwrap(), Vec::<String>::new());
    assert!(matches!(
        catalog.count(&Source::Collection(inner)),
        Err(CatalogError::NotFound { .. })
    ));
    assert!(matches!(
        catalog.delete_collection(nested),
        Err(CatalogError::NotFound { .. })
    ));

    // Removing a photo removes its memberships too.
    catalog.remove_photos(&ids[..1]).unwrap();
    assert_eq!(catalog.stats().unwrap().collection_photos, 1);
    assert_eq!(catalog.count(&Source::Collection(outer)).unwrap(), 1);
}
