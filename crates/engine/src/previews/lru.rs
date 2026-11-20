//! A least-recently-used map bounded by the bytes its values hold.

use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;

struct Slot<V> {
    value: V,
    cost: usize,
    used_at: u64,
}

/// Values with a cost in bytes; inserting beyond the budget evicts the
/// values used longest ago. Every operation is O(log n).
pub(crate) struct Lru<K, V> {
    budget: usize,
    used: usize,
    slots: HashMap<K, Slot<V>>,
    by_use: BTreeMap<u64, K>,
    clock: u64,
}

impl<K: Hash + Eq + Clone, V> Lru<K, V> {
    /// An empty map that holds at most `budget` bytes. A single value
    /// larger than the budget is still kept, alone.
    pub(crate) fn new(budget: usize) -> Self {
        Self {
            budget,
            used: 0,
            slots: HashMap::new(),
            by_use: BTreeMap::new(),
            clock: 0,
        }
    }

    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    /// The value under `key`, marking it as just used.
    pub(crate) fn get(&mut self, key: &K) -> Option<&V> {
        let now = self.tick();
        let slot = self.slots.get_mut(key)?;
        self.by_use.remove(&slot.used_at);
        slot.used_at = now;
        self.by_use.insert(now, key.clone());
        Some(&slot.value)
    }

    /// The value under `key`, without marking it as used.
    pub(crate) fn peek(&self, key: &K) -> Option<&V> {
        self.slots.get(key).map(|slot| &slot.value)
    }

    /// Store `value` under `key`, replacing what was there, then evict the
    /// least recently used values until the budget holds again.
    pub(crate) fn insert(&mut self, key: K, value: V, cost: usize) {
        self.remove(&key);
        let now = self.tick();
        self.used += cost;
        self.by_use.insert(now, key.clone());
        self.slots.insert(
            key,
            Slot {
                value,
                cost,
                used_at: now,
            },
        );
        while self.used > self.budget && self.slots.len() > 1 {
            let Some((_, oldest)) = self.by_use.pop_first() else {
                break;
            };
            if let Some(slot) = self.slots.remove(&oldest) {
                self.used -= slot.cost;
            }
        }
    }

    /// Forget the value under `key`.
    pub(crate) fn remove(&mut self, key: &K) {
        if let Some(slot) = self.slots.remove(key) {
            self.by_use.remove(&slot.used_at);
            self.used -= slot.cost;
        }
    }

    /// Bytes held.
    #[cfg(test)]
    pub(crate) fn used(&self) -> usize {
        self.used
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_least_recently_used_values_go_first_when_the_budget_is_exceeded() {
        let mut lru = Lru::new(100);
        lru.insert("a", 1, 40);
        lru.insert("b", 2, 40);
        assert_eq!(lru.get(&"a"), Some(&1));
        lru.insert("c", 3, 40);
        assert_eq!(lru.peek(&"b"), None, "b was used longest ago");
        assert_eq!(lru.peek(&"a"), Some(&1));
        assert_eq!(lru.used(), 80);
    }

    #[test]
    fn replacing_a_value_updates_the_bytes_held() {
        let mut lru = Lru::new(100);
        lru.insert("a", 1, 60);
        lru.insert("a", 2, 30);
        assert_eq!(lru.used(), 30);
        assert_eq!(lru.peek(&"a"), Some(&2));
        lru.remove(&"a");
        assert_eq!(lru.used(), 0);
        assert_eq!(lru.peek(&"a"), None);
    }

    #[test]
    fn a_value_larger_than_the_budget_is_kept_alone() {
        let mut lru = Lru::new(10);
        lru.insert("small", 1, 5);
        lru.insert("huge", 2, 50);
        assert_eq!(lru.peek(&"small"), None);
        assert_eq!(lru.peek(&"huge"), Some(&2));
        assert_eq!(lru.used(), 50);
    }
}
