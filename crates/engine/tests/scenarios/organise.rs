//! Collections and keywords: editing the trees, with undo.

use photo_core::meta::PhotoId;
use photo_engine::{CollectionKind, SmartField, SmartOp, SmartRule, SmartRules, Source, Target};
use photo_testkit::TempLibrary;

use crate::common::{Studio, own_order};

fn studio() -> (TempLibrary, Studio, Vec<PhotoId>) {
    let library = TempLibrary::new();
    let mut studio = Studio::new();
    studio.import_library(&library);
    let ids = studio.ids();
    (library, studio, ids)
}

/// Every collection as (name, kind, parent's name, photo count).
fn tree(studio: &Studio) -> Vec<(String, CollectionKind, Option<String>, u64)> {
    let collections = studio.engine.collections();
    let name_of = |id| {
        collections
            .iter()
            .find(|collection| collection.id == id)
            .map(|collection| collection.name.clone())
    };
    collections
        .iter()
        .map(|c| {
            (
                c.name.clone(),
                c.kind,
                c.parent.and_then(name_of),
                c.photo_count,
            )
        })
        .collect()
}

#[test]
fn deleting_a_collection_set_and_undoing_brings_everything_back() {
    let (_library, mut studio, p) = studio();
    let engine = &mut studio.engine;
    let trips = engine.create_collection_set("Trips", None).unwrap();
    let coast = engine.create_collection("Coast", Some(trips)).unwrap();
    let rules = SmartRules {
        match_all: true,
        rules: vec![SmartRule {
            field: SmartField::FileName,
            op: SmartOp::Contains,
            value: "a".into(),
        }],
    };
    let named = engine
        .create_smart_collection("Named", Some(trips), &rules)
        .unwrap();
    engine
        .add_to_collection(coast, Target::Photos(vec![p[3], p[0]]))
        .unwrap();
    let before = tree(&studio);
    assert_eq!(before.len(), 3);

    let engine = &mut studio.engine;
    engine.set_source(Source::Collection(coast)).unwrap();
    engine.delete_collection(trips).unwrap();
    assert!(engine.collections().is_empty());
    assert_eq!(
        engine.source(),
        &Source::AllPhotos,
        "the source fell back to all photos"
    );
    assert_eq!(
        studio.toasts().last().map(String::as_str),
        Some("Delete \"Trips\"")
    );

    studio.engine.undo().unwrap();
    assert_eq!(tree(&studio), before);
    studio.engine.set_source(Source::Collection(coast)).unwrap();
    studio.engine.set_sort(own_order()).unwrap();
    assert_eq!(
        studio.ids(),
        [p[3], p[0]],
        "members come back in their order"
    );
    assert_eq!(studio.engine.smart_rules(named).unwrap(), rules);

    studio.engine.redo().unwrap();
    assert!(studio.engine.collections().is_empty());
    studio.engine.undo().unwrap();
    assert_eq!(tree(&studio), before);
}

#[test]
fn creating_renaming_moving_and_editing_collections_undo() {
    let (_library, mut studio, _p) = studio();
    let engine = &mut studio.engine;
    let set = engine.create_collection_set("Work", None).unwrap();
    let picks = engine.create_collection("Picks", None).unwrap();
    engine.rename_collection(picks, "Best").unwrap();
    engine.move_collection(picks, Some(set)).unwrap();
    let rules = SmartRules::default();
    let smart = engine.create_smart_collection("All", None, &rules).unwrap();
    let narrower = SmartRules {
        match_all: true,
        rules: vec![SmartRule {
            field: SmartField::Rating,
            op: SmartOp::AtLeast,
            value: 1.into(),
        }],
    };
    engine.set_smart_rules(smart, &narrower).unwrap();
    assert_eq!(
        tree(&studio),
        [
            ("All".to_string(), CollectionKind::Smart, None, 0),
            (
                "Best".to_string(),
                CollectionKind::Regular,
                Some("Work".to_string()),
                0
            ),
            ("Work".to_string(), CollectionKind::Set, None, 0),
        ]
    );

    let engine = &mut studio.engine;
    engine.undo().unwrap();
    assert_eq!(engine.smart_rules(smart).unwrap(), rules);
    engine.undo().unwrap();
    engine.undo().unwrap();
    assert_eq!(
        tree(&studio)[0],
        ("Best".to_string(), CollectionKind::Regular, None, 0)
    );
    studio.engine.undo().unwrap();
    assert_eq!(tree(&studio)[0].0, "Picks");
    studio.engine.undo().unwrap();
    assert_eq!(
        tree(&studio).len(),
        1,
        "the created collection is gone again"
    );
    studio.engine.redo().unwrap();
    assert!(
        studio.engine.collections().iter().any(|c| c.id == picks),
        "back under its id"
    );
}

#[test]
fn deleting_a_keyword_and_undoing_restores_its_assignments() {
    let (_library, mut studio, p) = studio();
    let engine = &mut studio.engine;
    let places = engine.create_keyword("Places", None).unwrap();
    let paris = engine.create_keyword("Paris", Some(places)).unwrap();
    engine
        .add_keywords(Target::Photos(vec![p[0], p[1]]), &["Paris"])
        .unwrap();
    engine.rename_keyword(places, "Cities").unwrap();
    assert_eq!(engine.photo_keywords(p[0]).unwrap()[0].id, paris);

    engine.delete_keyword(places).unwrap();
    assert!(engine.keywords().is_empty());
    assert!(engine.photo_keywords(p[0]).unwrap().is_empty());
    engine.undo().unwrap();
    let keywords = engine.keywords().to_vec();
    assert_eq!(keywords.len(), 2);
    let restored = keywords.iter().find(|keyword| keyword.id == paris).unwrap();
    assert_eq!((restored.parent, restored.photo_count), (Some(places), 2));
    engine.undo().unwrap();
    assert!(
        engine
            .keywords()
            .iter()
            .any(|keyword| keyword.name == "Places")
    );
    engine.move_keyword(paris, None).unwrap();
    assert_eq!(
        engine
            .keywords()
            .iter()
            .find(|k| k.id == paris)
            .unwrap()
            .parent,
        None
    );
    engine.undo().unwrap();
    assert_eq!(
        engine
            .keywords()
            .iter()
            .find(|k| k.id == paris)
            .unwrap()
            .parent,
        Some(places)
    );
}

#[test]
fn undo_unwinds_a_removal_and_a_deletion_in_order() {
    let (_library, mut studio, p) = studio();
    let engine = &mut studio.engine;
    let album = engine.create_collection("Album", None).unwrap();
    engine
        .add_to_collection(album, Target::Photos(vec![p[0], p[1]]))
        .unwrap();
    engine
        .add_keywords(Target::Photos(vec![p[0]]), &["coast"])
        .unwrap();

    // A deletion after a removal, and a removal after a deletion.
    engine
        .remove_from_catalog(Target::Photos(vec![p[0]]))
        .unwrap();
    engine.delete_collection(album).unwrap();
    let coast = engine.keywords()[0].id;
    engine.delete_keyword(coast).unwrap();
    engine
        .remove_from_catalog(Target::Photos(vec![p[1]]))
        .unwrap();

    for _ in 0..4 {
        engine.undo().unwrap();
    }
    assert_eq!(engine.photo_collections(p[0]).unwrap(), [album]);
    assert_eq!(engine.photo_collections(p[1]).unwrap(), [album]);
    assert_eq!(engine.photo_keywords(p[0]).unwrap()[0].name, "coast");
    engine.set_source(Source::Collection(album)).unwrap();
    engine.set_sort(own_order()).unwrap();
    assert_eq!(studio.ids(), [p[0], p[1]]);
}
