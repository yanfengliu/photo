//! The selection: an active photo, an ordered set of selected photos and the
//! anchor a range selection extends from.
//!
//! Everything here is relative to the current photo list. Photos that are
//! not in the list cannot be selected, and when the list changes the
//! selection is carried over ([`Selection::retain`]).

use std::collections::{HashMap, HashSet};

use photo_core::meta::PhotoId;

/// Where the active photo moves to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Movement {
    /// This many places along the list (negative is backwards), stopping at
    /// either end.
    Step(isize),
    /// The first photo in the list.
    First,
    /// The last photo in the list.
    Last,
}

/// The photo list as the selection sees it: ids in order.
pub(crate) trait Listing {
    /// How many photos the list holds.
    fn count(&self) -> usize;
    /// The photo at `index`.
    fn id_at(&self, index: usize) -> Option<PhotoId>;
    /// Where `photo` is in the list.
    fn index_of(&self, photo: PhotoId) -> Option<usize>;
}

/// The active photo, the selected photos and the range anchor.
///
/// Invariants, kept by every operation: every selected photo, the active
/// photo and the anchor are in the current list; the selected photos are in
/// list order, each once; an active photo is also selected.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    active: Option<PhotoId>,
    selected: Vec<PhotoId>,
    members: HashSet<PhotoId>,
    anchor: Option<PhotoId>,
}

impl Selection {
    /// The photo the loupe and the panels show, if any.
    pub fn active(&self) -> Option<PhotoId> {
        self.active
    }

    /// The selected photos, in list order.
    pub fn selected(&self) -> &[PhotoId] {
        &self.selected
    }

    /// Where a range selection extends from.
    pub fn anchor(&self) -> Option<PhotoId> {
        self.anchor
    }

    /// Whether `photo` is selected. Constant time.
    pub fn is_selected(&self, photo: PhotoId) -> bool {
        self.members.contains(&photo)
    }

    /// Select exactly `photos` (those in the list), making `active` the
    /// active photo and anchor when it is one of them, else the first.
    pub(crate) fn set(&mut self, list: &impl Listing, photos: &[PhotoId], active: Option<PhotoId>) {
        let selected: Vec<PhotoId> = photos
            .iter()
            .copied()
            .filter(|photo| list.index_of(*photo).is_some())
            .collect();
        let active = active
            .filter(|photo| selected.contains(photo))
            .or_else(|| selected.first().copied());
        self.replace(list, selected, active, active);
    }

    /// Select only `photo` and make it active and the anchor. Returns false,
    /// changing nothing, when it is not in the list.
    pub(crate) fn select_only(&mut self, list: &impl Listing, photo: PhotoId) -> bool {
        if list.index_of(photo).is_none() {
            return false;
        }
        self.replace(list, vec![photo], Some(photo), Some(photo));
        true
    }

    /// Add `photo` to the selection, or take it out if it is selected. An
    /// added photo becomes active; when the active photo is taken out, the
    /// nearest selected photo becomes active. Either way `photo` becomes the
    /// anchor.
    pub(crate) fn toggle(&mut self, list: &impl Listing, photo: PhotoId) -> bool {
        let Some(at) = list.index_of(photo) else {
            return false;
        };
        let mut selected = self.selected.clone();
        let mut active = self.active;
        if self.is_selected(photo) {
            selected.retain(|id| *id != photo);
            if active == Some(photo) {
                active = nearest(list, &selected, at);
            }
        } else {
            selected.push(photo);
            active = Some(photo);
        }
        self.replace(list, selected, active, Some(photo));
        true
    }

    /// Select the range from the anchor (or the active photo) to `photo`,
    /// replacing the selection, and make `photo` active. The anchor stays.
    pub(crate) fn extend_to(&mut self, list: &impl Listing, photo: PhotoId) -> bool {
        let Some(to) = list.index_of(photo) else {
            return false;
        };
        let anchor = self
            .anchor
            .or(self.active)
            .filter(|anchor| list.index_of(*anchor).is_some())
            .unwrap_or(photo);
        let from = list.index_of(anchor).unwrap_or(to);
        let range = from.min(to)..=from.max(to);
        let selected = range.filter_map(|index| list.id_at(index)).collect();
        self.replace(list, selected, Some(photo), Some(anchor));
        true
    }

    /// Select every photo in the list. The active photo stays active; with
    /// none, the first photo becomes active.
    pub(crate) fn select_all(&mut self, list: &impl Listing) {
        let selected: Vec<PhotoId> = (0..list.count()).filter_map(|i| list.id_at(i)).collect();
        let active = self.active.or_else(|| selected.first().copied());
        let anchor = self.anchor.or(active);
        self.replace(list, selected, active, anchor);
    }

    /// Select nothing, with no active photo and no anchor.
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    /// Move the active photo. Without `extend` the photo moved to becomes the
    /// only selected one; with it, the selection becomes the range from the
    /// anchor to that photo. Returns false when the list is empty.
    pub(crate) fn move_active(
        &mut self,
        list: &impl Listing,
        movement: Movement,
        extend: bool,
    ) -> bool {
        let count = list.count();
        if count == 0 {
            return false;
        }
        let last = count - 1;
        let current = self.active.and_then(|photo| list.index_of(photo));
        let target = match (movement, current) {
            (Movement::First, _) => 0,
            (Movement::Last, _) => last,
            (Movement::Step(step), Some(at)) => at.saturating_add_signed(step).min(last),
            (Movement::Step(step), None) if step < 0 => last,
            (Movement::Step(_), None) => 0,
        };
        let Some(photo) = list.id_at(target) else {
            return false;
        };
        if extend {
            if self.anchor.is_none() {
                self.anchor = self.active;
            }
            self.extend_to(list, photo)
        } else {
            self.select_only(list, photo)
        }
    }

    /// Carry the selection over to a new list. `previous` is the list it
    /// was made on, in its order.
    ///
    /// Photos that left the list are dropped. If the active photo left, the
    /// nearest photo that is still selected becomes active; when none is,
    /// the nearest photo still in the list becomes active and selected. An
    /// anchor that left is replaced by the active photo.
    pub(crate) fn retain(&mut self, previous: &[PhotoId], list: &impl Listing) {
        let selected: Vec<PhotoId> = self
            .selected
            .iter()
            .copied()
            .filter(|photo| list.index_of(*photo).is_some())
            .collect();
        let mut active = self.active;
        let mut extra = None;
        if let Some(gone) = self.active.filter(|photo| list.index_of(*photo).is_none()) {
            let was_at = previous.iter().position(|photo| *photo == gone);
            active = was_at.and_then(|at| nearest_by_old_place(previous, &selected, at));
            if active.is_none() && selected.is_empty() {
                extra = was_at.and_then(|at| nearest_present(previous, list, at));
                active = extra;
            }
        }
        let anchor = self
            .anchor
            .filter(|photo| list.index_of(*photo).is_some())
            .or(active);
        let selected = selected.into_iter().chain(extra).collect();
        self.replace(list, selected, active, anchor);
    }

    /// Install a new state, putting the selected photos in list order and
    /// keeping the invariants.
    fn replace(
        &mut self,
        list: &impl Listing,
        mut selected: Vec<PhotoId>,
        active: Option<PhotoId>,
        anchor: Option<PhotoId>,
    ) {
        let mut members = HashSet::with_capacity(selected.len());
        selected.retain(|photo| members.insert(*photo));
        selected.sort_by_key(|photo| list.index_of(*photo));
        if let Some(active) = active
            && members.insert(active)
        {
            selected.push(active);
            selected.sort_by_key(|photo| list.index_of(*photo));
        }
        *self = Self {
            active,
            selected,
            members,
            anchor,
        };
    }
}

/// The photo of `candidates` closest to list position `at`, preferring the
/// following one on a tie.
fn nearest(list: &impl Listing, candidates: &[PhotoId], at: usize) -> Option<PhotoId> {
    candidates
        .iter()
        .filter_map(|photo| list.index_of(*photo).map(|index| (index, *photo)))
        .min_by_key(|(index, _)| (index.abs_diff(at), *index < at))
        .map(|(_, photo)| photo)
}

/// The photo of `candidates` whose place in `previous` is closest to `at`,
/// preferring the following one on a tie.
fn nearest_by_old_place(previous: &[PhotoId], candidates: &[PhotoId], at: usize) -> Option<PhotoId> {
    let places: HashMap<PhotoId, usize> = previous
        .iter()
        .enumerate()
        .map(|(index, photo)| (*photo, index))
        .collect();
    candidates
        .iter()
        .filter_map(|photo| places.get(photo).map(|index| (*index, *photo)))
        .min_by_key(|(index, _)| (index.abs_diff(at), *index < at))
        .map(|(_, photo)| photo)
}

/// The photo nearest to place `at` of `previous` that is still in `list`,
/// looking at the following photo before the preceding one.
fn nearest_present(previous: &[PhotoId], list: &impl Listing, at: usize) -> Option<PhotoId> {
    let present = |index: usize| {
        previous
            .get(index)
            .copied()
            .filter(|photo| list.index_of(*photo).is_some())
    };
    (1..previous.len()).find_map(|distance| {
        present(at + distance).or_else(|| at.checked_sub(distance).and_then(present))
    })
}

#[cfg(test)]
mod tests;
