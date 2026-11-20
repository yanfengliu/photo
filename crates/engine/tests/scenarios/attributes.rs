//! Changing photos on a selection, then undoing and redoing.

use photo_core::develop::Orientation;
use photo_core::meta::{ColorLabel, Flag, PhotoId, Rating};
use photo_engine::{Source, Target, TextField};
use photo_testkit::TempLibrary;

use crate::common::{Studio, own_order};

fn studio() -> (TempLibrary, Studio, Vec<PhotoId>) {
    let library = TempLibrary::new();
    let mut studio = Studio::new();
    studio.import_library(&library);
    let ids = studio.ids();
    (library, studio, ids)
}

fn ratings(studio: &Studio, photos: &[PhotoId]) -> Vec<u8> {
    photos
        .iter()
        .map(|photo| studio.engine.summary(*photo).unwrap().rating.stars())
        .collect()
}

fn keyword_names(studio: &Studio, photo: PhotoId) -> Vec<String> {
    let mut names: Vec<String> = studio
        .engine
        .photo_keywords(photo)
        .unwrap()
        .into_iter()
        .map(|keyword| keyword.name)
        .collect();
    names.sort();
    names
}

#[test]
fn ratings_on_a_selection_undo_and_redo_per_photo() {
    let (_library, mut studio, p) = studio();
    let three = [p[0], p[1], p[2]];
    for (photo, stars) in three.iter().zip([1, 2, 3]) {
        studio
            .engine
            .set_rating(Target::Photos(vec![*photo]), Rating::new(stars))
            .unwrap();
    }
    studio.toasts();
    studio.engine.select_only(p[0]);
    studio.engine.extend_selection_to(p[2]);

    studio
        .engine
        .set_rating(Target::Selected, Rating::new(5))
        .unwrap();
    assert_eq!(ratings(&studio, &three), [5, 5, 5]);
    assert_eq!(studio.toasts(), ["Set Rating to 5"]);
    assert_eq!(studio.engine.undo_label(), Some("Set Rating to 5"));

    assert!(studio.engine.undo().unwrap());
    assert_eq!(
        ratings(&studio, &three),
        [1, 2, 3],
        "each photo gets its own value back"
    );
    assert_eq!(studio.toasts(), ["Undo Set Rating to 5"]);
    assert!(studio.engine.redo().unwrap());
    assert_eq!(ratings(&studio, &three), [5, 5, 5]);
    assert!(studio.engine.undo().unwrap());
    assert_eq!(
        studio.toasts(),
        ["Redo Set Rating to 5", "Undo Set Rating to 5"]
    );

    studio.engine.increase_rating(Target::Selected).unwrap();
    assert_eq!(ratings(&studio, &three), [2, 3, 4]);
    assert_eq!(
        studio.engine.redo_label(),
        None,
        "a new change forgets what could be redone"
    );
    studio.engine.decrease_rating(Target::Active).unwrap();
    assert_eq!(ratings(&studio, &three), [2, 3, 3]);
    assert_eq!(studio.toasts(), ["Increase Rating", "Set Rating to 3"]);

    while studio.engine.undo().unwrap() {}
    assert_eq!(ratings(&studio, &three), [0, 0, 0]);
    assert!(!studio.engine.undo().unwrap(), "nothing left to undo");
}

#[test]
fn flags_labels_and_text_change_and_come_back_per_photo() {
    let (_library, mut studio, p) = studio();
    let engine = &mut studio.engine;
    engine
        .set_flag(Target::Photos(vec![p[0]]), Flag::Picked)
        .unwrap();
    engine
        .toggle_label(Target::Photos(vec![p[0]]), ColorLabel::Red)
        .unwrap();
    engine
        .set_text(Target::Photos(vec![p[1]]), TextField::Title, "Lake")
        .unwrap();
    studio.toasts();

    let engine = &mut studio.engine;
    engine
        .set_flag(Target::Photos(vec![p[0], p[1]]), Flag::Rejected)
        .unwrap();
    engine
        .toggle_label(Target::Photos(vec![p[0], p[1]]), ColorLabel::Red)
        .unwrap();
    assert_eq!(
        engine.summary(p[0]).unwrap().label,
        ColorLabel::None,
        "a photo with the label loses it"
    );
    assert_eq!(engine.summary(p[1]).unwrap().label, ColorLabel::Red);
    engine
        .set_text(Target::Photos(vec![p[0], p[1]]), TextField::Title, "Shore")
        .unwrap();
    assert_eq!(engine.photo(p[0]).unwrap().title, "Shore");
    assert_eq!(
        studio.toasts(),
        ["Flag as Rejected", "Set Color Label to Red", "Set Title"]
    );

    let engine = &mut studio.engine;
    engine.undo().unwrap();
    assert_eq!(engine.photo(p[0]).unwrap().title, "");
    assert_eq!(engine.photo(p[1]).unwrap().title, "Lake");
    engine.undo().unwrap();
    assert_eq!(engine.summary(p[0]).unwrap().label, ColorLabel::Red);
    assert_eq!(engine.summary(p[1]).unwrap().label, ColorLabel::None);
    engine.undo().unwrap();
    assert_eq!(engine.summary(p[0]).unwrap().flag, Flag::Picked);
    assert_eq!(engine.summary(p[1]).unwrap().flag, Flag::Unflagged);
    engine
        .set_flag(Target::Photos(vec![p[2]]), Flag::Picked)
        .unwrap();
    assert_eq!(
        studio.toasts().last().map(String::as_str),
        Some("Flag as Pick")
    );
}

#[test]
fn keywords_are_added_removed_and_set_by_name_and_undone_exactly() {
    let (_library, mut studio, p) = studio();
    let (a, b) = (p[0], p[1]);
    studio
        .engine
        .add_keywords(Target::Photos(vec![a]), &["coast"])
        .unwrap();
    studio
        .engine
        .add_keywords(Target::Photos(vec![a, b]), &["Coast", "sea", " "])
        .unwrap();
    assert_eq!(keyword_names(&studio, a), ["coast", "sea"]);
    assert_eq!(keyword_names(&studio, b), ["coast", "sea"]);
    assert_eq!(studio.engine.undo_label(), Some("Add 2 Keywords"));
    studio.engine.undo().unwrap();
    assert_eq!(
        keyword_names(&studio, a),
        ["coast"],
        "a kept the keyword it had before"
    );
    assert!(keyword_names(&studio, b).is_empty());
    studio.engine.redo().unwrap();
    assert_eq!(keyword_names(&studio, b), ["coast", "sea"]);

    studio
        .engine
        .remove_keywords(Target::Photos(vec![a, b]), &["COAST"])
        .unwrap();
    assert_eq!(keyword_names(&studio, a), ["sea"]);
    studio.engine.undo().unwrap();
    assert_eq!(keyword_names(&studio, a), ["coast", "sea"]);

    studio
        .engine
        .set_keywords(Target::Photos(vec![a]), &["one", "two", "sea"])
        .unwrap();
    assert_eq!(keyword_names(&studio, a), ["one", "sea", "two"]);
    assert_eq!(keyword_names(&studio, b), ["coast", "sea"]);
    let id_of = |studio: &Studio, name: &str| {
        studio
            .engine
            .keywords()
            .iter()
            .find(|keyword| keyword.name == name)
            .map(|keyword| keyword.id)
    };
    let one = id_of(&studio, "one").unwrap();
    studio.engine.undo().unwrap();
    assert_eq!(keyword_names(&studio, a), ["coast", "sea"]);
    let counts: Vec<(String, u64)> = studio
        .engine
        .keywords()
        .iter()
        .map(|keyword| (keyword.name.clone(), keyword.photo_count))
        .collect();
    assert_eq!(
        counts,
        [("coast".to_string(), 2), ("sea".to_string(), 2)],
        "the keywords the change created are gone again"
    );
    studio.engine.redo().unwrap();
    assert_eq!(keyword_names(&studio, a), ["one", "sea", "two"]);
    assert_eq!(id_of(&studio, "one"), Some(one), "back under its own id");
}

#[test]
fn rotating_changes_the_stored_orientation_and_the_develop_version() {
    let (_library, mut studio, p) = studio();
    let photo = p[2];
    let first = studio.engine.summary(photo).unwrap().develop_version;
    studio
        .engine
        .rotate_right(Target::Photos(vec![photo]))
        .unwrap();
    let turned = studio.engine.develop_settings(photo).unwrap().orientation;
    assert_eq!(turned, Orientation::new(1, false));
    let second = studio.engine.summary(photo).unwrap().develop_version;
    assert!(second > first);
    let history = studio.engine.history(photo).unwrap();
    assert_eq!(
        history.last().map(|step| step.label.as_str()),
        Some("Rotate Right")
    );

    studio
        .engine
        .flip_horizontal(Target::Photos(vec![photo]))
        .unwrap();
    let flipped = studio.engine.develop_settings(photo).unwrap().orientation;
    assert_eq!(flipped, Orientation::new(1, false).flipped_horizontal());
    studio
        .engine
        .rotate_left(Target::Photos(vec![photo]))
        .unwrap();
    studio
        .engine
        .flip_vertical(Target::Photos(vec![photo]))
        .unwrap();

    for _ in 0..4 {
        studio.engine.undo().unwrap();
    }
    assert_eq!(
        studio.engine.develop_settings(photo).unwrap().orientation,
        Orientation::IDENTITY
    );
    assert!(!studio.engine.summary(photo).unwrap().has_edits);
    assert!(
        studio.engine.summary(photo).unwrap().develop_version > second,
        "versions only grow"
    );
    studio.engine.redo().unwrap();
    assert_eq!(
        studio.engine.develop_settings(photo).unwrap().orientation,
        turned
    );
}

#[test]
fn a_virtual_copy_is_created_selected_and_undone() {
    let (_library, mut studio, p) = studio();
    studio
        .engine
        .set_rating(Target::Photos(vec![p[2]]), Rating::new(2))
        .unwrap();
    let copies = studio
        .engine
        .create_virtual_copies(Target::Photos(vec![p[2]]))
        .unwrap();
    let copy = copies[0];
    let summary = studio.engine.summary(copy).unwrap();
    assert_eq!(summary.master_id, Some(p[2]));
    assert_eq!(summary.rating, Rating::new(2));
    assert_eq!(studio.engine.selection().selected(), [copy]);
    assert_eq!(studio.engine.source_counts().all_photos, 7);

    studio.engine.undo().unwrap();
    assert!(studio.engine.summary(copy).is_none());
    assert_eq!(studio.engine.source_counts().all_photos, 6);
    studio.engine.redo().unwrap();
    assert_eq!(
        studio.engine.summary(copy).unwrap().master_id,
        Some(p[2]),
        "back under its id"
    );
}

#[test]
fn removing_then_undoing_restores_ids_attributes_keywords_and_collections() {
    let (_library, mut studio, p) = studio();
    let photo = p[2];
    let engine = &mut studio.engine;
    engine
        .set_rating(Target::Photos(vec![photo]), Rating::new(4))
        .unwrap();
    engine
        .add_keywords(Target::Photos(vec![photo]), &["coast"])
        .unwrap();
    let trip = engine.create_collection("Trip", None).unwrap();
    engine
        .add_to_collection(trip, Target::Photos(vec![p[1], photo, p[3]]))
        .unwrap();
    engine
        .toggle_quick_collection(Target::Photos(vec![photo]))
        .unwrap();
    let copy = engine
        .create_virtual_copies(Target::Photos(vec![photo]))
        .unwrap()[0];
    engine.select_only(photo);

    engine.remove_from_catalog(Target::Selected).unwrap();
    assert!(engine.summary(photo).is_none() && engine.summary(copy).is_none());
    assert_eq!(engine.source_counts().all_photos, 5);
    // The copy sat right after its master, so the nearest photo still
    // listed is the one before.
    assert_eq!(engine.active_photo(), Some(p[1]));
    assert_eq!(
        studio.toasts().last().map(String::as_str),
        Some("Remove 2 Photos from Catalog")
    );

    let engine = &mut studio.engine;
    engine.undo().unwrap();
    let record = engine.photo(photo).unwrap();
    assert_eq!(record.rating, Rating::new(4));
    assert_eq!(engine.summary(copy).unwrap().master_id, Some(photo));
    assert_eq!(keyword_names(&studio, photo), ["coast"]);
    let engine = &mut studio.engine;
    let mut collections = engine.photo_collections(photo).unwrap();
    collections.sort();
    let mut expected = vec![trip, engine.quick_collection()];
    expected.sort();
    assert_eq!(collections, expected);
    engine.set_source(Source::Collection(trip)).unwrap();
    engine.set_sort(own_order()).unwrap();
    assert_eq!(
        studio.ids(),
        [p[1], photo, p[3]],
        "back in its place in the collection"
    );

    studio.engine.redo().unwrap();
    assert!(studio.engine.photo(photo).is_err());
    studio.engine.undo().unwrap();
    assert_eq!(studio.engine.photo(photo).unwrap().rating, Rating::new(4));
}

#[test]
fn a_mixed_selection_gets_one_label_and_each_photo_gets_its_own_back() {
    let (_library, mut studio, p) = studio();
    let engine = &mut studio.engine;
    engine
        .set_label(Target::Photos(vec![p[0]]), ColorLabel::Red)
        .unwrap();
    engine
        .set_label(Target::Photos(vec![p[1]]), ColorLabel::Blue)
        .unwrap();
    engine
        .set_label(Target::Photos(p[..3].to_vec()), ColorLabel::Red)
        .unwrap();
    let labels = |engine: &photo_engine::Engine| -> Vec<ColorLabel> {
        p[..3]
            .iter()
            .map(|photo| engine.summary(*photo).unwrap().label)
            .collect()
    };
    assert_eq!(labels(engine), [ColorLabel::Red; 3], "set, not toggled");
    assert_eq!(engine.undo_label(), Some("Set Color Label to Red"));
    engine.undo().unwrap();
    assert_eq!(
        labels(engine),
        [ColorLabel::Red, ColorLabel::Blue, ColorLabel::None]
    );
    engine
        .set_label(Target::Photos(p[..3].to_vec()), ColorLabel::None)
        .unwrap();
    assert_eq!(labels(engine), [ColorLabel::None; 3]);
    assert_eq!(engine.undo_label(), Some("Clear Color Label"));
}

#[test]
fn keywords_chosen_by_id_are_exact_where_names_are_shared() {
    let (_library, mut studio, p) = studio();
    let engine = &mut studio.engine;
    let places = engine.create_keyword("Places", None).unwrap();
    let people = engine.create_keyword("People", None).unwrap();
    let city = engine.create_keyword("Paris", Some(places)).unwrap();
    let person = engine.create_keyword("Paris", Some(people)).unwrap();
    let both = Target::Photos(vec![p[0], p[1]]);
    engine.add_keyword_ids(both.clone(), &[person]).unwrap();
    engine
        .add_keyword_ids(Target::Photos(vec![p[1]]), &[city, places])
        .unwrap();
    assert_eq!(engine.undo_label(), Some("Add 2 Keywords"));
    let ids = |engine: &photo_engine::Engine, photo: PhotoId| -> Vec<_> {
        engine
            .photo_keywords(photo)
            .unwrap()
            .iter()
            .map(|keyword| keyword.id)
            .collect()
    };
    assert_eq!(ids(engine, p[0]), [person]);

    let uses = engine
        .keywords_of(Target::Photos(vec![p[0], p[1], p[2]]))
        .unwrap();
    let counted: Vec<(_, usize)> = uses
        .iter()
        .map(|used| (used.keyword.id, used.photos))
        .collect();
    assert_eq!(counted.len(), 3);
    assert!(counted.contains(&(person, 2)) && counted.contains(&(city, 1)));
    let names: Vec<&str> = uses.iter().map(|used| used.keyword.name.as_str()).collect();
    assert_eq!(names, ["Paris", "Paris", "Places"], "in name order");

    engine.remove_keyword_ids(both.clone(), &[person]).unwrap();
    assert_eq!(engine.undo_label(), Some("Remove Keyword \"Paris\""));
    assert!(ids(engine, p[0]).is_empty());
    assert_eq!(ids(engine, p[1]).len(), 2, "the city keyword stays");
    engine.undo().unwrap();
    assert_eq!(ids(engine, p[0]), [person]);

    let unknown = photo_core::meta::KeywordId(9999);
    assert!(engine.add_keyword_ids(both, &[city, unknown]).is_err());
    assert_eq!(ids(engine, p[0]), [person], "nothing changed");
}
