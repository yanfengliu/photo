//! The order in which queued jobs are handed to workers.
//!
//! Three lanes. The urgent lane (the photo open in Develop) goes first, in
//! order. The interactive lane (pictures someone is looking at) comes next
//! and serves the newest request first, so the cells a user has just
//! scrolled to are filled before the ones scrolled past. The background
//! lane (imports, scans, sidecars) is first come, first served.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::hash::Hash;

/// Queued items of type `T`, interactive ones optionally identified by a key
/// of type `K` so that a repeated request can replace or promote them.
#[derive(Debug)]
pub(crate) struct Queue<T, K> {
    urgent: VecDeque<T>,
    /// Interactive items by sequence number; the largest is the newest.
    interactive: BTreeMap<u64, (Option<K>, T)>,
    /// Where each keyed interactive item sits in `interactive`.
    keyed: HashMap<K, u64>,
    background: VecDeque<T>,
    next_sequence: u64,
}

impl<T, K: Hash + Eq + Clone> Queue<T, K> {
    pub(crate) fn new() -> Self {
        Self {
            urgent: VecDeque::new(),
            interactive: BTreeMap::new(),
            keyed: HashMap::new(),
            background: VecDeque::new(),
            next_sequence: 0,
        }
    }

    fn sequence(&mut self) -> u64 {
        self.next_sequence += 1;
        self.next_sequence
    }

    /// Queue an interactive item as the newest. An item already queued under
    /// the same key is taken out and returned: it will never be handed out.
    pub(crate) fn push_interactive(&mut self, key: Option<K>, item: T) -> Option<T> {
        let sequence = self.sequence();
        let replaced = key
            .as_ref()
            .and_then(|key| self.keyed.insert(key.clone(), sequence))
            .and_then(|old| self.interactive.remove(&old))
            .map(|(_, item)| item);
        self.interactive.insert(sequence, (key, item));
        replaced
    }

    /// Queue a background item behind every other background item.
    pub(crate) fn push_background(&mut self, item: T) {
        self.background.push_back(item);
    }

    /// Queue an urgent item behind every other urgent item, ahead of
    /// everything else.
    pub(crate) fn push_urgent(&mut self, item: T) {
        self.urgent.push_back(item);
    }

    /// Make the interactive item queued under `key` the newest. Returns
    /// whether such an item was queued.
    pub(crate) fn promote(&mut self, key: &K) -> bool {
        let Some(old) = self.keyed.get(key).copied() else {
            return false;
        };
        let Some(entry) = self.interactive.remove(&old) else {
            return false;
        };
        let sequence = self.sequence();
        self.keyed.insert(key.clone(), sequence);
        self.interactive.insert(sequence, entry);
        true
    }

    /// The next item to run: the oldest urgent item, else the newest
    /// interactive item, else the oldest background item.
    pub(crate) fn pop(&mut self) -> Option<T> {
        if let Some(item) = self.urgent.pop_front() {
            return Some(item);
        }
        if let Some((_, (key, item))) = self.interactive.pop_last() {
            if let Some(key) = key {
                self.keyed.remove(&key);
            }
            return Some(item);
        }
        self.background.pop_front()
    }

    /// Take every queued item out.
    pub(crate) fn drain(&mut self) -> Vec<T> {
        self.keyed.clear();
        let interactive = std::mem::take(&mut self.interactive);
        self.urgent
            .drain(..)
            .chain(interactive.into_values().map(|(_, item)| item))
            .chain(self.background.drain(..))
            .collect()
    }

    /// How many items are queued.
    pub(crate) fn len(&self) -> usize {
        self.urgent.len() + self.interactive.len() + self.background.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain_order(queue: &mut Queue<&'static str, u32>) -> Vec<&'static str> {
        std::iter::from_fn(|| queue.pop()).collect()
    }

    #[test]
    fn urgent_items_go_first_then_interactive_ones_newest_first() {
        let mut queue = Queue::new();
        queue.push_background("import a");
        queue.push_interactive(None, "thumb 1");
        queue.push_urgent("decode 1");
        queue.push_background("import b");
        queue.push_interactive(None, "thumb 2");
        queue.push_urgent("decode 2");
        assert_eq!(queue.len(), 6);
        assert_eq!(
            drain_order(&mut queue),
            [
                "decode 1", "decode 2", "thumb 2", "thumb 1", "import a", "import b"
            ]
        );
    }

    #[test]
    fn a_keyed_item_is_replaced_by_a_newer_request_under_the_same_key() {
        let mut queue = Queue::new();
        assert_eq!(queue.push_interactive(Some(7), "old"), None);
        queue.push_interactive(Some(8), "other");
        assert_eq!(queue.push_interactive(Some(7), "new"), Some("old"));
        assert_eq!(drain_order(&mut queue), ["new", "other"]);
    }

    #[test]
    fn promoting_moves_an_item_to_the_front() {
        let mut queue = Queue::new();
        queue.push_interactive(Some(1), "first");
        queue.push_interactive(Some(2), "second");
        assert!(queue.promote(&1));
        assert!(!queue.promote(&3));
        assert_eq!(drain_order(&mut queue), ["first", "second"]);
        // Once handed out, the key is free again.
        assert!(!queue.promote(&1));
        assert_eq!(queue.push_interactive(Some(1), "again"), None);
    }

    #[test]
    fn draining_empties_both_lanes() {
        let mut queue = Queue::new();
        queue.push_interactive(Some(1), "a");
        queue.push_background("b");
        let mut drained = queue.drain();
        drained.sort_unstable();
        assert_eq!(drained, ["a", "b"]);
        assert_eq!(queue.len(), 0);
        assert!(!queue.promote(&1));
    }
}
