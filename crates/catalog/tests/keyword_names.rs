//! Setting a photo's keywords by name or by id: keywords the photo already
//! has stay the same keywords, a bare name never lands on one of several
//! nested keywords, and every change says what an undo needs.

mod common;

use common::{import, import_paths, memory, raster};
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

/// Two keywords called Paris, under Texas (the older) and under France.
struct Places {
    texas_paris: KeywordId,
    france: KeywordId,
    france_paris: KeywordId,
}

fn places(catalog: &mut Catalog) -> Places {
    let texas = catalog.create_keyword("Texas", None).unwrap();
    let texas_paris = catalog.create_keyword("Paris", Some(texas)).unwrap();
    let france = catalog.create_keyword("France", None).unwrap();
    let france_paris = catalog.create_keyword("Paris", Some(france)).unwrap();
    Places {
        texas_paris,
        france,
        france_paris,
    }
}

#[test]
fn naming_a_photos_keywords_again_keeps_the_keywords_it_already_has() {
    // Adding "Eiffel" to a photo tagged France > Paris must not move it to
    // the older Texas > Paris.
    let mut catalog = memory();
    let photo = import_paths(&mut catalog, &["/k/a.jpg"])[0];
    let places = places(&mut catalog);
    catalog
        .set_photo_keyword_ids(photo, &[places.france_paris])
        .unwrap();
    let listed = catalog.keywords().unwrap();

    let change = catalog
        .set_photo_keywords(photo, &["Paris", "Eiffel"])
        .unwrap();
    let eiffel = *change.created.first().unwrap();
    assert_eq!(
        change,
        KeywordsChange {
            removed: vec![],
            added: vec![eiffel],
            created: vec![eiffel],
        }
    );
    assert_eq!(ids_of(&catalog, photo), [places.france_paris, eiffel]);

    // What the change reports is exactly what undoing it needs.
    for added in &change.added {
        catalog.unassign_keyword(*added, &[photo]).unwrap();
    }
    for removed in &change.removed {
        catalog.assign_keyword(*removed, &[photo]).unwrap();
    }
    for created in &change.created {
        catalog.delete_keyword(*created).unwrap();
    }
    assert_eq!(ids_of(&catalog, photo), [places.france_paris]);
    assert_eq!(catalog.keywords().unwrap(), listed);
}

#[test]
fn a_new_name_means_the_only_keyword_of_that_name_or_else_the_top_level_one() {
    let mut catalog = memory();
    let photos = import_paths(&mut catalog, &["/k/a.jpg", "/k/b.jpg", "/k/c.jpg"]);
    let places = places(&mut catalog);
    let lyon = catalog.create_keyword("Lyon", Some(places.france)).unwrap();

    // One keyword has the name, nested or not: that one.
    let change = catalog.set_photo_keywords(photos[0], &["LYON"]).unwrap();
    assert_eq!((change.added, change.created), (vec![lyon], vec![]));

    // Two nested keywords have it and none sits at the top level: a bare
    // name means neither, so a top-level Paris is created...
    let change = catalog.set_photo_keywords(photos[1], &["Paris"]).unwrap();
    let top = *change.created.first().unwrap();
    assert_eq!(change.added, [top]);
    assert!(top != places.texas_paris && top != places.france_paris);
    let listed = catalog.keywords().unwrap();
    assert_eq!(listed.iter().find(|k| k.id == top).unwrap().parent, None);

    // ...and that is the one the name means from then on, everywhere.
    let change = catalog.set_photo_keywords(photos[2], &["paris"]).unwrap();
    assert_eq!((change.added, change.created), (vec![top], vec![]));
    assert_eq!(catalog.ensure_keyword("PARIS").unwrap(), top);
    let mut photo = raster("/k/d.jpg");
    photo.keywords = vec!["Paris".into()];
    let imported = import(&mut catalog, &[photo])[0];
    assert_eq!(ids_of(&catalog, imported), [top]);
}

#[test]
fn an_imported_name_shared_by_nested_keywords_does_not_go_to_the_oldest() {
    let mut catalog = memory();
    let places = places(&mut catalog);
    let mut photo = raster("/k/a.jpg");
    photo.keywords = vec!["Paris".into()];
    let imported = import(&mut catalog, &[photo])[0];
    let ids = ids_of(&catalog, imported);
    assert_eq!(ids.len(), 1);
    assert!(ids[0] != places.texas_paris && ids[0] != places.france_paris);
}

#[test]
fn keywords_can_be_set_by_id_and_the_change_says_what_moved() {
    let mut catalog = memory();
    let photo = import_paths(&mut catalog, &["/k/a.jpg"])[0];
    let places = places(&mut catalog);
    let first = catalog
        .set_photo_keyword_ids(photo, &[places.texas_paris, places.france])
        .unwrap();
    assert_eq!(first.added, [places.texas_paris, places.france]);

    let change = catalog
        .set_photo_keyword_ids(
            photo,
            &[places.france_paris, places.france, places.france_paris],
        )
        .unwrap();
    assert_eq!(
        change,
        KeywordsChange {
            removed: vec![places.texas_paris],
            added: vec![places.france_paris],
            created: vec![],
        }
    );
    assert_eq!(
        ids_of(&catalog, photo),
        [places.france, places.france_paris]
    );

    // An unknown id fails the call and changes nothing.
    assert!(matches!(
        catalog.set_photo_keyword_ids(photo, &[places.france, KeywordId(404)]),
        Err(CatalogError::NotFound {
            entity: Entity::Keyword,
            id: 404
        })
    ));
    assert_eq!(
        ids_of(&catalog, photo),
        [places.france, places.france_paris]
    );

    // Setting the old ids again is an exact undo.
    catalog
        .set_photo_keyword_ids(photo, &[places.texas_paris, places.france])
        .unwrap();
    assert_eq!(ids_of(&catalog, photo), [places.texas_paris, places.france]);
}
