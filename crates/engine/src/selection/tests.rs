use super::*;

/// A list of photos 1..=n, or of the given ids, in order.
struct Ids(Vec<PhotoId>, HashMap<PhotoId, usize>);

impl Ids {
    fn of(ids: &[i64]) -> Self {
        let ids: Vec<PhotoId> = ids.iter().map(|id| PhotoId(*id)).collect();
        let index = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
        Self(ids, index)
    }

    fn upto(count: i64) -> Self {
        Self::of(&(1..=count).collect::<Vec<_>>())
    }
}

impl Listing for Ids {
    fn count(&self) -> usize {
        self.0.len()
    }
    fn id_at(&self, index: usize) -> Option<PhotoId> {
        self.0.get(index).copied()
    }
    fn index_of(&self, photo: PhotoId) -> Option<usize> {
        self.1.get(&photo).copied()
    }
}

fn p(id: i64) -> PhotoId {
    PhotoId(id)
}

fn ids(selection: &Selection) -> Vec<i64> {
    selection.selected().iter().map(|id| id.0).collect()
}

#[test]
fn a_range_extends_from_the_anchor_and_replaces_the_selection() {
    let list = Ids::upto(10);
    let mut selection = Selection::default();
    assert!(selection.select_only(&list, p(3)));
    assert!(selection.extend_to(&list, p(6)));
    assert_eq!(ids(&selection), [3, 4, 5, 6]);
    assert_eq!(selection.active(), Some(p(6)));
    assert_eq!(selection.anchor(), Some(p(3)));
    // Extending again pivots on the same anchor.
    selection.extend_to(&list, p(1));
    assert_eq!(ids(&selection), [1, 2, 3]);
    assert!(!selection.extend_to(&list, p(99)));
}

#[test]
fn toggling_adds_and_removes_and_moves_the_active_photo_to_a_neighbour() {
    let list = Ids::upto(10);
    let mut selection = Selection::default();
    selection.select_only(&list, p(2));
    selection.toggle(&list, p(5));
    selection.toggle(&list, p(8));
    assert_eq!(ids(&selection), [2, 5, 8]);
    assert_eq!(selection.active(), Some(p(8)));
    selection.toggle(&list, p(5));
    assert_eq!(ids(&selection), [2, 8]);
    assert_eq!(selection.active(), Some(p(8)));
    selection.toggle(&list, p(8));
    assert_eq!(selection.active(), Some(p(2)));
    assert!(selection.is_selected(p(2)) && !selection.is_selected(p(8)));
    selection.toggle(&list, p(2));
    assert_eq!(selection.active(), None);
    assert!(selection.selected().is_empty());
}

#[test]
fn a_selected_photo_can_be_made_active_without_changing_the_selection() {
    let list = Ids::upto(10);
    let mut selection = Selection::default();
    selection.select_only(&list, p(2));
    selection.extend_to(&list, p(5));
    assert!(selection.make_active(p(3)));
    assert_eq!(ids(&selection), [2, 3, 4, 5]);
    assert_eq!(selection.active(), Some(p(3)));
    assert_eq!(selection.anchor(), Some(p(3)));
    let before = selection.clone();
    assert!(!selection.make_active(p(7)), "not selected");
    assert_eq!(selection, before);
}

#[test]
fn moving_steps_along_the_list_and_stops_at_the_ends() {
    let list = Ids::upto(5);
    let mut selection = Selection::default();
    assert!(selection.move_active(&list, Movement::Step(1), false));
    assert_eq!(selection.active(), Some(p(1)));
    selection.move_active(&list, Movement::Step(2), false);
    assert_eq!(ids(&selection), [3]);
    selection.move_active(&list, Movement::Step(10), false);
    assert_eq!(selection.active(), Some(p(5)));
    selection.move_active(&list, Movement::Step(-10), false);
    assert_eq!(selection.active(), Some(p(1)));
    selection.move_active(&list, Movement::Last, false);
    assert_eq!(ids(&selection), [5]);
    assert!(!selection.move_active(&Ids::of(&[]), Movement::First, false));
}

#[test]
fn moving_with_extend_grows_a_range_from_the_anchor() {
    let list = Ids::upto(6);
    let mut selection = Selection::default();
    selection.select_only(&list, p(3));
    selection.move_active(&list, Movement::Step(1), true);
    selection.move_active(&list, Movement::Step(1), true);
    assert_eq!(ids(&selection), [3, 4, 5]);
    assert_eq!(selection.active(), Some(p(5)));
    selection.move_active(&list, Movement::First, true);
    assert_eq!(ids(&selection), [1, 2, 3]);
    assert_eq!(selection.anchor(), Some(p(3)));
}

#[test]
fn select_all_keeps_the_active_photo_and_clear_empties_everything() {
    let list = Ids::upto(4);
    let mut selection = Selection::default();
    selection.select_all(&list);
    assert_eq!(ids(&selection), [1, 2, 3, 4]);
    assert_eq!(selection.active(), Some(p(1)));
    selection.select_only(&list, p(3));
    selection.select_all(&list);
    assert_eq!(selection.active(), Some(p(3)));
    selection.clear();
    assert_eq!(selection, Selection::default());
}

#[test]
fn photos_that_leave_the_list_are_dropped_and_a_selected_neighbour_takes_over() {
    let before = Ids::upto(10);
    let mut selection = Selection::default();
    selection.select_only(&before, p(4));
    selection.extend_to(&before, p(7));
    // Active is 7; the new list loses 6 and 7.
    let after = Ids::of(&[1, 2, 3, 4, 5, 8, 9, 10]);
    selection.retain(&before.0, &after);
    assert_eq!(ids(&selection), [4, 5]);
    assert_eq!(selection.active(), Some(p(5)));
    assert_eq!(selection.anchor(), Some(p(4)));
}

#[test]
fn when_nothing_selected_survives_the_nearest_photo_in_the_list_becomes_active() {
    let before = Ids::upto(6);
    let mut selection = Selection::default();
    selection.select_only(&before, p(3));
    let after = Ids::of(&[1, 2, 5, 6]);
    selection.retain(&before.0, &after);
    assert_eq!(
        selection.active(),
        Some(p(2)),
        "2 is one place from 3, 5 is two"
    );
    assert_eq!(ids(&selection), [2]);

    let mut selection = Selection::default();
    selection.select_only(&before, p(3));
    let after = Ids::of(&[1, 2, 4, 5]);
    selection.retain(&before.0, &after);
    assert_eq!(
        selection.active(),
        Some(p(4)),
        "the following photo wins a tie"
    );

    let mut selection = Selection::default();
    selection.select_only(&before, p(3));
    selection.retain(&before.0, &Ids::of(&[20, 21]));
    assert_eq!(selection, Selection::default());
}

#[test]
fn setting_a_selection_ignores_photos_outside_the_list() {
    let list = Ids::upto(5);
    let mut selection = Selection::default();
    selection.set(&list, &[p(4), p(9), p(2)], Some(p(9)));
    assert_eq!(ids(&selection), [2, 4]);
    assert_eq!(selection.active(), Some(p(4)));
    selection.set(&list, &[p(4), p(2)], Some(p(2)));
    assert_eq!(selection.active(), Some(p(2)));
}
