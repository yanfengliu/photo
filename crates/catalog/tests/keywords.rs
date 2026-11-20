//! The keyword hierarchy and keyword assignment.

mod common;

use common::{import_paths, memory, names};
use photo_catalog::{Catalog, CatalogError, Entity, KeywordInfo, PhotoQuery, Source};
use photo_core::meta::{KeywordId, PhotoId};

fn listing(catalog: &Catalog) -> Vec<(String, Option<KeywordId>, u64)> {
    let keywords = catalog.keywords().unwrap();
    keywords
        .into_iter()
        .map(|k| (k.name, k.parent, k.photo_count))
        .collect()
}

fn keyword_names(keywords: &[KeywordInfo]) -> Vec<&str> {
    keywords
        .iter()
        .map(|keyword| keyword.name.as_str())
        .collect()
}

fn tagged(catalog: &Catalog, keyword: KeywordId) -> Vec<String> {
    names(catalog, &PhotoQuery::from(Source::Keyword(keyword)))
}

#[test]
fn keywords_form_a_hierarchy_listed_in_natural_name_order() {
    let mut catalog = memory();
    let places = catalog.create_keyword("Places", None).unwrap();
    let france = catalog.create_keyword("France", Some(places)).unwrap();
    catalog.create_keyword("Paris", Some(france)).unwrap();
    catalog.create_keyword("roll 10", None).unwrap();
    catalog.create_keyword("Roll 9", None).unwrap();
    catalog.create_keyword("  animals  ", None).unwrap();

    assert_eq!(
        listing(&catalog),
        [
            ("animals".to_string(), None, 0),
            ("France".to_string(), Some(places), 0),
            ("Paris".to_string(), Some(france), 0),
            ("Places".to_string(), None, 0),
            ("Roll 9".to_string(), None, 0),
            ("roll 10".to_string(), None, 0),
        ]
    );
}

#[test]
fn sibling_names_are_unique_ignoring_case_but_cousins_may_share_a_name() {
    let mut catalog = memory();
    let europe = catalog.create_keyword("Europe", None).unwrap();
    let america = catalog.create_keyword("America", None).unwrap();
    catalog.create_keyword("Georgia", Some(europe)).unwrap();
    catalog.create_keyword("Georgia", Some(america)).unwrap();

    let clash = catalog.create_keyword("GEORGIA", Some(europe)).unwrap_err();
    assert!(matches!(clash, CatalogError::NameTaken(name) if name == "GEORGIA"));
    assert!(matches!(
        catalog.create_keyword("europe", None),
        Err(CatalogError::NameTaken(_))
    ));
    assert!(matches!(
        catalog.create_keyword("   ", None),
        Err(CatalogError::EmptyName)
    ));
    assert!(matches!(
        catalog.create_keyword("Lost", Some(KeywordId(404))),
        Err(CatalogError::NotFound {
            entity: Entity::Keyword,
            id: 404
        })
    ));
    assert_eq!(catalog.keywords().unwrap().len(), 4);
}

#[test]
fn renaming_keeps_assignments_and_respects_sibling_names() {
    let mut catalog = memory();
    let photos = import_paths(&mut catalog, &["/k/a.jpg"]);
    let cat = catalog.create_keyword("cat", None).unwrap();
    catalog.create_keyword("dog", None).unwrap();
    catalog.assign_keyword(cat, &photos).unwrap();

    catalog.rename_keyword(cat, "Feline").unwrap();
    catalog.rename_keyword(cat, "FELINE").unwrap();
    assert_eq!(
        keyword_names(&catalog.photo_keywords(photos[0]).unwrap()),
        ["FELINE"]
    );
    assert!(matches!(
        catalog.rename_keyword(cat, "Dog"),
        Err(CatalogError::NameTaken(_))
    ));
    assert!(matches!(
        catalog.rename_keyword(cat, ""),
        Err(CatalogError::EmptyName)
    ));
    assert!(matches!(
        catalog.rename_keyword(KeywordId(404), "x"),
        Err(CatalogError::NotFound { .. })
    ));
}

#[test]
fn moving_a_keyword_reparents_its_subtree_and_refuses_cycles() {
    let mut catalog = memory();
    let places = catalog.create_keyword("Places", None).unwrap();
    let france = catalog.create_keyword("France", Some(places)).unwrap();
    let paris = catalog.create_keyword("Paris", Some(france)).unwrap();
    let travel = catalog.create_keyword("Travel", None).unwrap();

    catalog.move_keyword(france, Some(travel)).unwrap();
    let parent_of = |catalog: &Catalog, id: KeywordId| {
        catalog
            .keywords()
            .unwrap()
            .into_iter()
            .find(|keyword| keyword.id == id)
            .unwrap()
            .parent
    };
    assert_eq!(parent_of(&catalog, france), Some(travel));
    assert_eq!(
        parent_of(&catalog, paris),
        Some(france),
        "children come along"
    );

    catalog.move_keyword(france, None).unwrap();
    assert_eq!(parent_of(&catalog, france), None);

    for (id, target) in [(france, france), (france, paris)] {
        assert!(matches!(
            catalog.move_keyword(id, Some(target)),
            Err(CatalogError::Cycle {
                entity: Entity::Keyword,
                ..
            })
        ));
    }
    catalog.create_keyword("France", Some(travel)).unwrap();
    assert!(matches!(
        catalog.move_keyword(france, Some(travel)),
        Err(CatalogError::NameTaken(_))
    ));
    assert_eq!(
        parent_of(&catalog, france),
        None,
        "a refused move changes nothing"
    );
}

#[test]
fn assigning_and_unassigning_report_exactly_what_changed() {
    let mut catalog = memory();
    let photos = import_paths(&mut catalog, &["/k/a.jpg", "/k/b.jpg", "/k/c.jpg"]);
    let sea = catalog.create_keyword("sea", None).unwrap();

    assert_eq!(
        catalog.assign_keyword(sea, &photos[..2]).unwrap(),
        &photos[..2]
    );
    // Only the photo that did not have it yet is reported the second time.
    assert_eq!(catalog.assign_keyword(sea, &photos).unwrap(), [photos[2]]);
    assert_eq!(listing(&catalog), [("sea".to_string(), None, 3)]);

    assert_eq!(
        catalog
            .unassign_keyword(sea, &[photos[0], photos[0]])
            .unwrap(),
        [photos[0]]
    );
    assert_eq!(catalog.unassign_keyword(sea, &[photos[0]]).unwrap(), []);
    assert_eq!(listing(&catalog), [("sea".to_string(), None, 2)]);
    assert_eq!(tagged(&catalog, sea), ["b.jpg", "c.jpg"]);

    let unknown = catalog
        .assign_keyword(sea, &[photos[0], PhotoId(404)])
        .unwrap_err();
    assert!(matches!(
        unknown,
        CatalogError::NotFound {
            entity: Entity::Photo,
            id: 404
        }
    ));
    assert_eq!(
        listing(&catalog),
        [("sea".to_string(), None, 2)],
        "the valid photo was not assigned either"
    );
    assert!(matches!(
        catalog.assign_keyword(KeywordId(404), &photos),
        Err(CatalogError::NotFound { .. })
    ));
}

#[test]
fn setting_keywords_by_name_reuses_existing_keywords_and_creates_the_rest() {
    let mut catalog = memory();
    let photos = import_paths(&mut catalog, &["/k/a.jpg", "/k/b.jpg"]);
    let places = catalog.create_keyword("Places", None).unwrap();
    let paris = catalog.create_keyword("Paris", Some(places)).unwrap();

    let set = catalog
        .set_photo_keywords(photos[0], &["paris", "Night", " night ", ""])
        .unwrap();
    assert_eq!(
        set.added.len(),
        2,
        "blank names are skipped and repeats collapse"
    );
    assert_eq!(
        set.added[0], paris,
        "an existing keyword is matched ignoring case, wherever it sits"
    );
    let night = set.added[1];
    assert_eq!(set.created, [night]);
    assert!(set.removed.is_empty());
    assert_eq!(
        keyword_names(&catalog.photo_keywords(photos[0]).unwrap()),
        ["Night", "Paris"]
    );
    assert_eq!(catalog.keywords().unwrap().len(), 3);

    // Setting replaces the previous set.
    catalog.set_photo_keywords(photos[0], &["Night"]).unwrap();
    assert_eq!(
        keyword_names(&catalog.photo_keywords(photos[0]).unwrap()),
        ["Night"]
    );
    catalog
        .set_photo_keywords(photos[0], &[] as &[&str])
        .unwrap();
    assert!(catalog.photo_keywords(photos[0]).unwrap().is_empty());
    assert_eq!(
        catalog.keywords().unwrap().len(),
        3,
        "keywords outlive their last assignment"
    );

    assert_eq!(catalog.ensure_keyword("NIGHT").unwrap(), night);
    let dusk = catalog.ensure_keyword("Dusk").unwrap();
    assert_eq!(
        catalog
            .keywords()
            .unwrap()
            .iter()
            .find(|k| k.id == dusk)
            .unwrap()
            .parent,
        None
    );
    assert!(matches!(
        catalog.ensure_keyword(" "),
        Err(CatalogError::EmptyName)
    ));
    assert!(matches!(
        catalog.set_photo_keywords(PhotoId(404), &["x"]),
        Err(CatalogError::NotFound { .. })
    ));
}

#[test]
fn a_keyword_as_a_source_includes_photos_tagged_with_its_descendants() {
    let mut catalog = memory();
    let photos = import_paths(
        &mut catalog,
        &["/k/a.jpg", "/k/b.jpg", "/k/c.jpg", "/k/d.jpg"],
    );
    let places = catalog.create_keyword("Places", None).unwrap();
    let france = catalog.create_keyword("France", Some(places)).unwrap();
    let paris = catalog.create_keyword("Paris", Some(france)).unwrap();
    let japan = catalog.create_keyword("Japan", Some(places)).unwrap();
    catalog
        .assign_keyword(paris, &[photos[0], photos[1]])
        .unwrap();
    catalog.assign_keyword(france, &[photos[1]]).unwrap();
    catalog.assign_keyword(japan, &[photos[2]]).unwrap();

    assert_eq!(tagged(&catalog, paris), ["a.jpg", "b.jpg"]);
    assert_eq!(tagged(&catalog, france), ["a.jpg", "b.jpg"]);
    assert_eq!(tagged(&catalog, places), ["a.jpg", "b.jpg", "c.jpg"]);
    assert_eq!(catalog.count(&Source::Keyword(places)).unwrap(), 3);
    assert!(matches!(
        catalog.count(&Source::Keyword(KeywordId(404))),
        Err(CatalogError::NotFound { .. })
    ));

    // The listed count is direct assignments only.
    let count = |name: &str| {
        listing(&catalog)
            .into_iter()
            .find(|entry| entry.0 == name)
            .unwrap()
            .2
    };
    assert_eq!(
        (
            count("Places"),
            count("France"),
            count("Paris"),
            count("Japan")
        ),
        (0, 1, 2, 1)
    );
}

#[test]
fn deleting_a_keyword_deletes_its_subtree_and_every_assignment_but_no_photo() {
    let mut catalog = memory();
    let photos = import_paths(&mut catalog, &["/k/a.jpg", "/k/b.jpg"]);
    let places = catalog.create_keyword("Places", None).unwrap();
    let france = catalog.create_keyword("France", Some(places)).unwrap();
    let paris = catalog.create_keyword("Paris", Some(france)).unwrap();
    let other = catalog.create_keyword("Other", None).unwrap();
    catalog.assign_keyword(paris, &photos).unwrap();
    catalog.assign_keyword(france, &photos[..1]).unwrap();
    catalog.assign_keyword(other, &photos).unwrap();
    assert_eq!(catalog.stats().unwrap().photo_keywords, 5);

    catalog.delete_keyword(france).unwrap();

    let stats = catalog.stats().unwrap();
    assert_eq!(
        (stats.keywords, stats.photo_keywords, stats.photos),
        (2, 2, 2)
    );
    assert_eq!(
        keyword_names(&catalog.photo_keywords(photos[0]).unwrap()),
        ["Other"]
    );
    assert_eq!(catalog.check_integrity().unwrap(), Vec::<String>::new());
    assert!(matches!(
        catalog.delete_keyword(paris),
        Err(CatalogError::NotFound { .. })
    ));
}
