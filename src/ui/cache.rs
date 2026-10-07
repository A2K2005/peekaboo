//! A least-recently-used cache with a byte budget. The window keeps page
//! tiles and thumbnails in one; the document worker keeps pre-decoded
//! neighbor images in another.
use std::{collections::HashMap, hash::Hash};

/// Tiles and thumbnails share 48 MiB. The PRD caps a 20-page PDF at 120 MB
/// in total. A 1080p view needs about 9 MiB of 512-pixel tiles (1 MiB each)
/// and a 4K view about 40 MiB, so 48 MiB holds the visible pages plus the
/// prefetch margin while leaving about 70 MB for the process itself (code,
/// Direct2D, DirectWrite, PDFium). Unverified until a GUI memory run.
pub(super) const TILE_BUDGET: usize = 48 << 20;

struct Entry<V> {
    value: V,
    bytes: usize,
    stamp: u64,
}

pub(super) struct Lru<K, V> {
    map: HashMap<K, Entry<V>>,
    budget: usize,
    used: usize,
    clock: u64,
    /// Entries touched after this stamp are not evicted.
    floor: u64,
    frame: u64,
}

impl<K: Eq + Hash + Clone, V> Lru<K, V> {
    pub(super) fn new(budget: usize) -> Self {
        Self { map: HashMap::new(), budget, used: 0, clock: 0, floor: u64::MAX, frame: u64::MAX }
    }
    pub(super) fn set_budget(&mut self, budget: usize) {
        self.budget = budget;
        self.evict();
    }
    #[cfg(test)]
    pub(super) fn used(&self) -> usize {
        self.used
    }
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.map.len()
    }
    /// Starts a frame. Entries drawn in this frame or the one before are
    /// kept even over budget, so new tiles never evict what is on screen.
    pub(super) fn new_frame(&mut self) {
        self.floor = self.frame;
        self.frame = self.clock;
    }
    /// Reads and marks as recently used.
    pub(super) fn get(&mut self, key: &K) -> Option<&mut V> {
        self.clock += 1;
        let clock = self.clock;
        self.map.get_mut(key).map(|e| {
            e.stamp = clock;
            &mut e.value
        })
    }
    /// Reads without changing the order.
    pub(super) fn peek(&self, key: &K) -> Option<&V> {
        self.map.get(key).map(|e| &e.value)
    }
    pub(super) fn insert(&mut self, key: K, value: V, bytes: usize) {
        self.clock += 1;
        if let Some(old) = self.map.insert(key, Entry { value, bytes, stamp: self.clock }) {
            self.used -= old.bytes;
        }
        self.used += bytes;
        self.evict();
    }
    pub(super) fn values_mut(&mut self) -> impl Iterator<Item = (&K, &mut V)> {
        self.map.iter_mut().map(|(k, e)| (k, &mut e.value))
    }
    pub(super) fn clear(&mut self) {
        self.map.clear();
        self.used = 0;
    }
    // ponytail: O(n) scan per eviction; fine for a few hundred entries. A
    // linked list helps if the cache grows to thousands.
    fn evict(&mut self) {
        while self.used > self.budget {
            let floor = self.floor;
            let oldest = self.map.iter().filter(|(_, e)| floor == u64::MAX || e.stamp <= floor).min_by_key(|(_, e)| e.stamp);
            let Some(key) = oldest.map(|(k, _)| k.clone()) else {
                break;
            };
            if let Some(entry) = self.map.remove(&key) {
                self.used -= entry.bytes;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_least_recently_used_entries_to_the_byte_budget() {
        let mut lru = Lru::new(300);
        lru.insert(1, "a", 100);
        lru.insert(2, "b", 100);
        lru.insert(3, "c", 100);
        assert!(lru.get(&1).is_some());
        lru.insert(4, "d", 100);
        assert!(lru.peek(&2).is_none(), "2 was the least recently used");
        assert!(lru.peek(&1).is_some() && lru.peek(&3).is_some() && lru.peek(&4).is_some());
        assert_eq!(lru.used(), 300);
        lru.insert(4, "d2", 50);
        assert_eq!((lru.used(), lru.len()), (250, 3), "replacing a key recounts its bytes");
        lru.set_budget(100);
        assert_eq!(lru.used(), 50);
        lru.clear();
        assert_eq!((lru.used(), lru.len()), (0, 0));
    }

    #[test]
    fn entries_on_screen_survive_until_two_frames_pass() {
        let mut lru = Lru::new(200);
        lru.new_frame();
        lru.insert("old", 0, 100);
        lru.new_frame();
        lru.insert("visible", 0, 100);
        lru.insert("new", 0, 100);
        assert!(lru.peek(&"old").is_some(), "drawn in the previous frame");
        assert_eq!(lru.used(), 300, "over budget while everything is on screen");
        lru.new_frame();
        lru.new_frame();
        assert!(lru.get(&"new").is_some());
        lru.insert("next", 0, 100);
        assert!(lru.peek(&"old").is_none() && lru.peek(&"visible").is_none());
        assert!(lru.peek(&"new").is_some() && lru.peek(&"next").is_some());
        assert_eq!(lru.used(), 200);
    }
}
