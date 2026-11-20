//! The order inside a collection survives any sequence of changes being
//! undone in reverse.

mod common;

use common::{import_paths, memory};
use photo_catalog::{Catalog, CollectionEntry, RemovedPhotos};
use photo_core::meta::{CollectionId, PhotoId};

fn order(catalog: &Catalog, collection: CollectionId) -> Vec<PhotoId> {
    catalog.collection_photos(collection).unwrap()
}

fn album_of(count: usize) -> (Catalog, CollectionId, Vec<PhotoId>) {
    let paths: Vec<String> = (0..count).map(|index| format!("/o/{index}.jpg")).collect();
    let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
    let mut catalog = memory();
    let ids = import_paths(&mut catalog, &paths);
    let album = catalog.create_collection("Album", None).unwrap();
    (catalog, album, ids)
}

#[test]
fn removed_entries_report_their_place_in_line() {
    let (mut catalog, album, ids) = album_of(5);
    catalog
        .add_to_collection(album, &[ids[4], ids[3], ids[2], ids[1], ids[0]])
        .unwrap();

    let taken = catalog
        .remove_from_collection(album, &[ids[0], ids[3], ids[2]])
        .unwrap();
    // In the collection's order, with places counted over all five members.
    assert_eq!(
        taken,
        [
            CollectionEntry {
                photo: ids[3],
                index: 1
            },
            CollectionEntry {
                photo: ids[2],
                index: 2
            },
            CollectionEntry {
                photo: ids[0],
                index: 4
            },
        ]
    );
}

#[test]
fn photos_removed_one_after_another_come_back_in_their_old_order() {
    let (mut catalog, album, ids) = album_of(3);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    catalog.add_to_collection(album, &[a, b, c]).unwrap();

    let first = catalog.remove_photos(&[a]).unwrap();
    let second = catalog.remove_photos(&[b]).unwrap();
    // Giving the one photo left "a new order" renumbers it to the front,
    // where the numbers of the two that left used to be.
    catalog.set_collection_order(album, &[c]).unwrap();

    catalog.restore_photos(&second).unwrap();
    assert_eq!(order(&catalog, album), [b, c]);
    catalog.restore_photos(&first).unwrap();
    assert_eq!(order(&catalog, album), [a, b, c]);
}

#[test]
fn entries_removed_one_after_another_come_back_in_their_old_order() {
    let (mut catalog, album, ids) = album_of(4);
    let (a, b, c, d) = (ids[0], ids[1], ids[2], ids[3]);
    catalog.add_to_collection(album, &[a, b, c, d]).unwrap();

    let first = catalog.remove_from_collection(album, &[b]).unwrap();
    let second = catalog.remove_from_collection(album, &[a, c]).unwrap();
    catalog.set_collection_order(album, &[d]).unwrap();

    catalog.restore_collection_entries(album, &second).unwrap();
    assert_eq!(order(&catalog, album), [a, c, d]);
    catalog.restore_collection_entries(album, &first).unwrap();
    assert_eq!(order(&catalog, album), [a, b, c, d]);
}

#[test]
fn renumbering_between_removals_and_their_undo_does_not_move_anything() {
    let (mut catalog, album, ids) = album_of(5);
    let (a, b, c, d, e) = (ids[0], ids[1], ids[2], ids[3], ids[4]);
    catalog.add_to_collection(album, &[e, d, c, b, a]).unwrap();

    // Reordered and back again: the same order as the removals left, under
    // numbers that no longer have gaps where the removed entries were.
    let first = catalog.remove_from_collection(album, &[d]).unwrap();
    let second = catalog.remove_from_collection(album, &[b]).unwrap();
    catalog.set_collection_order(album, &[a, c, e]).unwrap();
    catalog.set_collection_order(album, &[e, c, a]).unwrap();
    catalog.restore_collection_entries(album, &second).unwrap();
    assert_eq!(order(&catalog, album), [e, c, b, a]);
    catalog.restore_collection_entries(album, &first).unwrap();
    assert_eq!(order(&catalog, album), [e, d, c, b, a]);

    // The same when the photos leave the catalog altogether.
    let first = catalog.remove_photos(&[d]).unwrap();
    let second = catalog.remove_photos(&[b]).unwrap();
    catalog.set_collection_order(album, &[a, c, e]).unwrap();
    catalog.set_collection_order(album, &[e, c, a]).unwrap();
    catalog.restore_photos(&second).unwrap();
    assert_eq!(order(&catalog, album), [e, c, b, a]);
    catalog.restore_photos(&first).unwrap();
    assert_eq!(order(&catalog, album), [e, d, c, b, a]);
}

#[test]
fn an_entry_whose_place_is_past_the_end_goes_last() {
    let (mut catalog, album, ids) = album_of(4);
    catalog.add_to_collection(album, &ids).unwrap();
    let taken = catalog.remove_from_collection(album, &[ids[3]]).unwrap();
    catalog
        .remove_from_collection(album, &[ids[0], ids[1]])
        .unwrap();

    // Not undone in order: the collection is shorter than the place asks for.
    catalog.restore_collection_entries(album, &taken).unwrap();
    assert_eq!(order(&catalog, album), [ids[2], ids[3]]);
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

    fn pick(&mut self, from: &[PhotoId], at_most: usize) -> Vec<PhotoId> {
        let mut pool = from.to_vec();
        let mut picked = Vec::new();
        for _ in 0..at_most.min(pool.len()) {
            picked.push(pool.swap_remove(self.roll(pool.len())));
        }
        picked
    }
}

enum Undo {
    Remove(Vec<PhotoId>),
    PutBack(Vec<CollectionEntry>),
    Reorder(Vec<PhotoId>),
    Restore(Box<RemovedPhotos>),
}

#[test]
fn any_sequence_of_changes_undoes_step_by_step_to_exactly_the_earlier_orders() {
    for seed in 0..25 {
        let (mut catalog, album, ids) = album_of(12);
        let mut dice = Dice(seed);
        let start = dice.pick(&ids, 7);
        catalog.add_to_collection(album, &start).unwrap();
        // The photos still in the catalog, and what to do to take each step
        // back, with the order the collection had before the step.
        let mut alive = ids.clone();
        let mut steps: Vec<(Vec<PhotoId>, Undo)> = Vec::new();

        for _ in 0..40 {
            let before = order(&catalog, album);
            let outside: Vec<PhotoId> = alive
                .iter()
                .copied()
                .filter(|id| !before.contains(id))
                .collect();
            let undo = match dice.roll(4) {
                0 => {
                    let chosen = dice.pick(&outside, 3);
                    Undo::Remove(catalog.add_to_collection(album, &chosen).unwrap())
                }
                1 => {
                    let chosen = dice.pick(&before, 3);
                    Undo::PutBack(catalog.remove_from_collection(album, &chosen).unwrap())
                }
                2 => {
                    let shuffled = dice.pick(&before, before.len());
                    catalog.set_collection_order(album, &shuffled).unwrap();
                    Undo::Reorder(before.clone())
                }
                _ => {
                    // Photos leave the catalog altogether, members or not.
                    let chosen = dice.pick(&alive, 2);
                    alive.retain(|id| !chosen.contains(id));
                    Undo::Restore(Box::new(catalog.remove_photos(&chosen).unwrap()))
                }
            };
            steps.push((before, undo));
        }

        while let Some((before, undo)) = steps.pop() {
            match undo {
                Undo::Remove(added) => {
                    catalog.remove_from_collection(album, &added).unwrap();
                }
                Undo::PutBack(entries) => {
                    catalog.restore_collection_entries(album, &entries).unwrap()
                }
                Undo::Reorder(previous) => catalog.set_collection_order(album, &previous).unwrap(),
                Undo::Restore(removed) => catalog.restore_photos(&removed).unwrap(),
            }
            assert_eq!(
                order(&catalog, album),
                before,
                "seed {seed}, {} steps left",
                steps.len()
            );
        }
        assert_eq!(order(&catalog, album), start, "seed {seed}");
        assert_eq!(catalog.check_integrity().unwrap(), Vec::<String>::new());
    }
}
