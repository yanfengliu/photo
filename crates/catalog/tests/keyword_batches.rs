//! Setting the keywords of many photos in one call: one transaction, all or
//! nothing, and undone exactly by handing the former lists back.

mod common;

use common::{import_paths, memory};
use photo_catalog::{Catalog, CatalogError, Entity, KeywordsChange};
use photo_core::meta::{KeywordId, PhotoId};

/// The photo's keywords, by id.
fn ids_of(catalog: &Catalog, photo: PhotoId) -> Vec<KeywordId> {
    let mut ids: Vec<KeywordId> = catalog
        .photo_keywords(photo)
        .unwrap()
        .into_iter()
        .map(|keyword| keyword.id)
        .collect();
    ids.sort();
    ids
}

/// Three photos and three keywords; the first two photos carry the first
/// keyword.
fn tagged(catalog: &mut Catalog) -> (Vec<PhotoId>, [KeywordId; 3]) {
    let photos = import_paths(catalog, &["/k/a.jpg", "/k/b.jpg", "/k/c.jpg"]);
    let sea = catalog.create_keyword("Sea", None).unwrap();
    let sky = catalog.create_keyword("Sky", None).unwrap();
    let hill = catalog.create_keyword("Hill", None).unwrap();
    catalog
        .assign_keyword(sea, &[photos[0], photos[1]])
        .unwrap();
    (photos, [sea, sky, hill])
}

#[test]
fn many_photos_get_their_lists_in_one_call_and_each_change_says_what_moved() {
    let mut catalog = memory();
    let (photos, [sea, sky, hill]) = tagged(&mut catalog);
    let changes = catalog
        .set_keyword_ids_of_photos(&[
            (photos[0], vec![sky, sea, sky]),
            (photos[1], vec![hill]),
            (photos[2], vec![]),
        ])
        .unwrap();
    assert_eq!(
        changes,
        [
            KeywordsChange {
                removed: vec![],
                added: vec![sky],
                created: vec![],
            },
            KeywordsChange {
                removed: vec![sea],
                added: vec![hill],
                created: vec![],
            },
            KeywordsChange::default(),
        ]
    );
    assert_eq!(ids_of(&catalog, photos[0]), [sea, sky]);
    assert_eq!(ids_of(&catalog, photos[1]), [hill]);
    assert!(ids_of(&catalog, photos[2]).is_empty());

    // Handing the former lists back is an exact undo.
    let undone = catalog
        .set_keyword_ids_of_photos(&[
            (photos[0], vec![sea]),
            (photos[1], vec![sea]),
            (photos[2], vec![]),
        ])
        .unwrap();
    assert_eq!(undone[0].removed, [sky]);
    assert_eq!(
        (ids_of(&catalog, photos[0]), ids_of(&catalog, photos[1])),
        (vec![sea], vec![sea])
    );
}

#[test]
fn a_photo_listed_twice_ends_with_its_last_list() {
    let mut catalog = memory();
    let (photos, [sea, sky, hill]) = tagged(&mut catalog);
    let changes = catalog
        .set_keyword_ids_of_photos(&[(photos[0], vec![sky]), (photos[0], vec![hill, sea])])
        .unwrap();
    assert_eq!(changes[0].removed, [sea]);
    assert_eq!(changes[1].removed, [sky]);
    assert_eq!(changes[1].added, [hill, sea]);
    assert_eq!(ids_of(&catalog, photos[0]), [sea, hill]);
}

#[test]
fn an_unknown_keyword_or_photo_fails_the_whole_call_and_changes_nothing() {
    let mut catalog = memory();
    let (photos, [sea, sky, _]) = tagged(&mut catalog);
    assert!(matches!(
        catalog.set_keyword_ids_of_photos(&[
            (photos[0], vec![sky]),
            (photos[1], vec![sea, KeywordId(404)]),
        ]),
        Err(CatalogError::NotFound {
            entity: Entity::Keyword,
            id: 404
        })
    ));
    assert!(matches!(
        catalog.set_keyword_ids_of_photos(&[(photos[0], vec![sky]), (PhotoId(404), vec![sky])]),
        Err(CatalogError::NotFound {
            entity: Entity::Photo,
            id: 404
        })
    ));
    assert_eq!(
        ids_of(&catalog, photos[0]),
        [sea],
        "the first entry was rolled back"
    );
    assert_eq!(ids_of(&catalog, photos[1]), [sea]);
    assert!(catalog.set_keyword_ids_of_photos(&[]).unwrap().is_empty());
}
