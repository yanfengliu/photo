//! Selecting photos in the list.

use photo_core::meta::{Flag, PhotoId};
use photo_engine::{Filter, Movement, Target};
use photo_testkit::TempLibrary;

use crate::common::Studio;

fn studio() -> (TempLibrary, Studio, Vec<PhotoId>) {
    let library = TempLibrary::new();
    let mut studio = Studio::new();
    studio.import_library(&library);
    let ids = studio.ids();
    (library, studio, ids)
}

#[test]
fn range_toggle_and_movement_with_and_without_extend() {
    let (_library, mut studio, p) = studio();
    let engine = &mut studio.engine;
    assert!(engine.select_only(p[0]));
    assert!(engine.extend_selection_to(p[3]));
    assert_eq!(engine.selection().selected(), &p[0..=3]);
    assert_eq!(engine.active_photo(), Some(p[3]));

    engine.toggle_selected(p[1]);
    assert_eq!(engine.selection().selected(), [p[0], p[2], p[3]]);
    assert!(engine.selection().is_selected(p[2]) && !engine.selection().is_selected(p[1]));

    assert!(engine.move_active(Movement::Step(1), false));
    assert_eq!(engine.selection().selected(), [p[4]]);
    engine.move_active(Movement::Step(-1), true);
    assert_eq!(engine.selection().selected(), [p[3], p[4]]);
    assert_eq!(engine.active_photo(), Some(p[3]));
    engine.move_active(Movement::First, true);
    assert_eq!(engine.selection().selected(), &p[0..=4]);
    assert_eq!(engine.selection().anchor(), Some(p[4]));
    engine.move_active(Movement::Last, false);
    assert_eq!(engine.selection().selected(), [p[5]]);
    engine.move_active(Movement::Step(3), false);
    assert_eq!(
        engine.active_photo(),
        Some(p[5]),
        "movement stops at the end"
    );
}

#[test]
fn select_all_and_select_none() {
    let (_library, mut studio, p) = studio();
    let engine = &mut studio.engine;
    engine.select_only(p[2]);
    engine.select_all();
    assert_eq!(engine.selection().selected(), p.as_slice());
    assert_eq!(engine.active_photo(), Some(p[2]));
    engine.select_none();
    assert!(engine.selection().selected().is_empty());
    assert_eq!(engine.active_photo(), None);
    // Nothing selected: an action on the selection does nothing.
    engine.set_flag(Target::Selected, Flag::Picked).unwrap();
    assert!(
        engine
            .photos()
            .iter()
            .all(|photo| photo.flag == Flag::Unflagged)
    );
}

#[test]
fn the_selection_survives_a_filter_change() {
    let (_library, mut studio, p) = studio();
    let engine = &mut studio.engine;
    engine
        .set_flag(Target::Photos(vec![p[1], p[4]]), Flag::Picked)
        .unwrap();
    engine.select_only(p[1]);
    engine.extend_selection_to(p[3]);
    assert_eq!(engine.active_photo(), Some(p[3]));

    let picked = Filter {
        flags: vec![Flag::Picked],
        ..Filter::default()
    };
    engine.set_filter(picked.clone()).unwrap();
    assert_eq!(
        engine.selection().selected(),
        [p[1]],
        "photos that left are dropped"
    );
    assert_eq!(
        engine.active_photo(),
        Some(p[1]),
        "the nearest selected photo takes over"
    );

    engine.set_filter(Filter::default()).unwrap();
    assert_eq!(engine.selection().selected(), [p[1]]);
    engine.select_only(p[2]);
    engine.set_filter(picked).unwrap();
    assert_eq!(
        engine.active_photo(),
        Some(p[1]),
        "with nothing left selected, the nearest photo"
    );
    assert_eq!(engine.selection().selected(), [p[1]]);
}
