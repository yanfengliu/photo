//! Folding consecutive develop changes into one, so that a run of writes (a
//! slider drag, say) is a single step for the caller's undo stack.

mod common;

use common::{import_paths, memory};
use photo_catalog::{Catalog, CatalogError, DevelopChange, HistoryStep};
use photo_core::develop::DevelopSettings;
use photo_core::meta::PhotoId;

fn exposure(stops: f32) -> DevelopSettings {
    let mut settings = DevelopSettings::default();
    settings.tone.exposure = stops;
    settings
}

/// Everything about a photo's develop state that a caller can see, apart
/// from `develop_version`, which only ever moves forward.
type State = (DevelopSettings, bool, Option<i64>, Vec<HistoryStep>);

fn state(catalog: &Catalog, photo: PhotoId) -> State {
    let record = catalog.photo(photo).unwrap();
    (
        catalog.develop_settings(photo).unwrap(),
        record.has_edits,
        record.edited_at,
        catalog.history(photo).unwrap(),
    )
}

#[test]
fn a_run_of_writes_merges_into_one_change_that_undoes_and_redoes_exactly() {
    let mut catalog = memory();
    let photo = import_paths(&mut catalog, &["/m/a.jpg"])[0];
    catalog
        .set_develop_settings(photo, &exposure(0.1), Some("Earlier"))
        .unwrap();
    let start = state(&catalog, photo);

    // A drag: settings written as the slider moves, then a labelled write
    // when it is let go.
    let mut drag = catalog
        .set_develop_settings(photo, &exposure(0.2), None)
        .unwrap();
    for stops in [0.3, 0.4] {
        let moved = catalog
            .set_develop_settings(photo, &exposure(stops), None)
            .unwrap();
        drag.merge(&moved).unwrap();
    }
    let released = catalog
        .set_develop_settings(photo, &exposure(0.5), Some("Exposure"))
        .unwrap();
    drag.merge(&released).unwrap();
    let end = state(&catalog, photo);
    assert_eq!(drag.photo(), photo);
    assert_eq!(drag.history_step(), released.history_step());

    catalog.undo_develop(std::slice::from_ref(&drag)).unwrap();
    assert_eq!(state(&catalog, photo), start);
    catalog.redo_develop(std::slice::from_ref(&drag)).unwrap();
    assert_eq!(state(&catalog, photo), end);
    catalog.undo_develop(std::slice::from_ref(&drag)).unwrap();
    assert_eq!(state(&catalog, photo), start);
}

#[test]
fn a_merged_change_keeps_only_what_differs_between_its_two_ends() {
    let mut catalog = memory();
    let photo = import_paths(&mut catalog, &["/m/a.jpg"])[0];
    let first = catalog
        .set_develop_settings(photo, &exposure(0.5), Some("A"))
        .unwrap();
    let start = state(&catalog, photo);

    // Step B is recorded, left behind by going back to A, and then replaced
    // by C: across the three changes it was never there.
    let mut merged = catalog
        .set_develop_settings(photo, &exposure(1.0), Some("B"))
        .unwrap();
    let back = catalog
        .go_to_history_step(photo, first.history_step().unwrap())
        .unwrap();
    merged.merge(&back).unwrap();
    let replaced = catalog
        .set_develop_settings(photo, &exposure(1.5), Some("C"))
        .unwrap();
    merged.merge(&replaced).unwrap();
    let end = state(&catalog, photo);
    let labels = |catalog: &Catalog| -> Vec<String> {
        let steps = catalog.history(photo).unwrap();
        steps.into_iter().map(|step| step.label).collect()
    };
    assert_eq!(labels(&catalog), ["A", "C"]);

    catalog.undo_develop(std::slice::from_ref(&merged)).unwrap();
    assert_eq!(state(&catalog, photo), start);
    assert_eq!(labels(&catalog), ["A"]);
    catalog.redo_develop(std::slice::from_ref(&merged)).unwrap();
    assert_eq!(state(&catalog, photo), end);

    // The merged change sits in the undo order where its parts did.
    catalog.undo_develop(&[first, merged]).unwrap();
    assert!(catalog.history(photo).unwrap().is_empty());
    assert!(!catalog.photo(photo).unwrap().has_edits);
}

#[test]
fn only_the_change_made_directly_afterwards_can_be_merged() {
    let mut catalog = memory();
    let ids = import_paths(&mut catalog, &["/m/a.jpg", "/m/b.jpg"]);
    let pristine = state(&catalog, ids[0]);
    let first = catalog
        .set_develop_settings(ids[0], &exposure(0.5), Some("A"))
        .unwrap();
    let second = catalog
        .set_develop_settings(ids[0], &exposure(1.0), None)
        .unwrap();
    let third = catalog
        .set_develop_settings(ids[0], &exposure(1.5), None)
        .unwrap();
    let elsewhere = catalog
        .set_develop_settings(ids[1], &exposure(0.5), None)
        .unwrap();

    let mut change = first.clone();
    for not_next in [&third, &elsewhere, &first] {
        assert!(matches!(
            change.merge(not_next),
            Err(CatalogError::StaleChange(id)) if id == ids[0]
        ));
    }
    // A refused merge changed nothing: the change still undoes as itself.
    catalog.undo_develop(&[change, second, third]).unwrap();
    assert_eq!(state(&catalog, ids[0]), pristine);
}

/// A small deterministic generator, so that a failure can be reproduced from
/// its seed.
struct Dice(u64);

impl Dice {
    fn roll(&mut self, sides: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) as usize) % sides.max(1)
    }
}

/// One develop change of a random kind: a jump to one of the photo's history
/// steps, an unlabelled edit, or a labelled one.
fn random_change(catalog: &mut Catalog, photo: PhotoId, dice: &mut Dice) -> DevelopChange {
    let settings = exposure(dice.roll(9) as f32 / 4.0 - 1.0);
    let steps = catalog.history(photo).unwrap();
    match dice.roll(4) {
        0 if !steps.is_empty() => {
            let step = steps[dice.roll(steps.len())].id;
            catalog.go_to_history_step(photo, step).unwrap()
        }
        1 => catalog
            .set_develop_settings(photo, &settings, None)
            .unwrap(),
        _ => catalog
            .set_develop_settings(photo, &settings, Some("Edit"))
            .unwrap(),
    }
}

#[test]
fn any_session_of_changes_folded_at_random_undoes_and_redoes_entry_by_entry() {
    for seed in 0..40 {
        let mut catalog = memory();
        let photo = import_paths(&mut catalog, &["/m/a.jpg"])[0];
        let mut dice = Dice(seed);
        // An undo stack as a caller would keep it: each entry is one change,
        // or several folded together, with the state it started from.
        let mut stack: Vec<(State, DevelopChange)> = Vec::new();
        for _ in 0..60 {
            if dice.roll(6) == 0 {
                // Undo the newest entry and forget it, as the next edit
                // after an undo forgets what could have been redone.
                if let Some((before, change)) = stack.pop() {
                    catalog.undo_develop(std::slice::from_ref(&change)).unwrap();
                    assert_eq!(state(&catalog, photo), before, "seed {seed}");
                }
                continue;
            }
            let before = state(&catalog, photo);
            let change = random_change(&mut catalog, photo, &mut dice);
            match stack.last_mut() {
                Some((_, entry)) if dice.roll(2) == 0 => entry.merge(&change).unwrap(),
                _ => stack.push((before, change)),
            }
        }

        // Undone in turn, every entry lands on the state it started from;
        // redone, on the state the entry after it started from.
        let mut ends = vec![state(&catalog, photo)];
        for (before, change) in stack.iter().rev() {
            catalog.undo_develop(std::slice::from_ref(change)).unwrap();
            assert_eq!(state(&catalog, photo), *before, "seed {seed}");
            ends.push(before.clone());
        }
        assert!(catalog.history(photo).unwrap().is_empty(), "seed {seed}");
        for ((_, change), end) in stack.iter().zip(ends.iter().rev().skip(1)) {
            catalog.redo_develop(std::slice::from_ref(change)).unwrap();
            assert_eq!(state(&catalog, photo), *end, "seed {seed}");
        }
        assert_eq!(catalog.check_integrity().unwrap(), Vec::<String>::new());
    }
}
