//! Recently seen folder listings, shown at once when the user goes back to a
//! folder while a fresh listing is fetched.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use crate::backend::Entry;
use crate::location::Location;

/// Folders remembered at most.
const MAX_FOLDERS: usize = 64;
/// Entries remembered at most, over all folders.
const MAX_ENTRIES: usize = 250_000;

#[derive(Debug, Default)]
pub struct ListingCache {
    map: HashMap<Location, Arc<Vec<Entry>>>,
    /// Least recently used first.
    order: VecDeque<Location>,
    total: usize,
}

impl ListingCache {
    pub fn get(&mut self, loc: &Location) -> Option<Arc<Vec<Entry>>> {
        let hit = self.map.get(loc)?.clone();
        self.touch(loc);
        Some(hit)
    }

    pub fn insert(&mut self, loc: Location, entries: Arc<Vec<Entry>>) {
        self.remove(&loc);
        if entries.len() > MAX_ENTRIES {
            return;
        }
        self.total += entries.len();
        self.order.push_back(loc.clone());
        self.map.insert(loc, entries);
        while self.map.len() > MAX_FOLDERS || self.total > MAX_ENTRIES {
            let Some(old) = self.order.pop_front() else { break };
            if let Some(e) = self.map.remove(&old) {
                self.total -= e.len();
            }
        }
    }

    pub fn remove(&mut self, loc: &Location) {
        if let Some(e) = self.map.remove(loc) {
            self.total -= e.len();
            self.order.retain(|l| l != loc);
        }
    }

    /// Forget `loc` and every folder below it: something there changed.
    pub fn invalidate_within(&mut self, loc: &Location) {
        let gone: Vec<Location> = self.map.keys().filter(|k| k.is_within(loc)).cloned().collect();
        for k in gone {
            self.remove(&k);
        }
    }

    /// Forget every folder of `remote` (the account is gone).
    pub fn forget_remote(&mut self, remote: &crate::location::Remote) {
        let gone: Vec<Location> = self.map.keys().filter(|k| &k.remote == remote).cloned().collect();
        for k in gone {
            self.remove(&k);
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    fn touch(&mut self, loc: &Location) {
        if let Some(i) = self.order.iter().position(|l| l == loc) {
            let l = self.order.remove(i).expect("index in range");
            self.order.push_back(l);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::Remote;

    fn loc(p: &str) -> Location {
        Location::new(Remote::Named("g".into()), p)
    }

    fn entries(n: usize) -> Arc<Vec<Entry>> {
        Arc::new(
            (0..n)
                .map(|i| Entry {
                    name: format!("f{i}"),
                    size: Some(1),
                    mod_time: None,
                    is_dir: false,
                    mime: String::new(),
                })
                .collect(),
        )
    }

    #[test]
    fn least_recently_used_goes_first() {
        let mut c = ListingCache::default();
        for i in 0..MAX_FOLDERS {
            c.insert(loc(&format!("d{i}")), entries(1));
        }
        assert!(c.get(&loc("d0")).is_some());
        c.insert(loc("new"), entries(1));
        assert_eq!(c.len(), MAX_FOLDERS);
        assert!(c.get(&loc("d0")).is_some(), "recently read");
        assert!(c.get(&loc("d1")).is_none(), "oldest evicted");
    }

    #[test]
    fn entry_budget_is_kept() {
        let mut c = ListingCache::default();
        c.insert(loc("a"), entries(MAX_ENTRIES - 10));
        c.insert(loc("b"), entries(20));
        assert!(c.get(&loc("a")).is_none());
        assert!(c.get(&loc("b")).is_some());
        c.insert(loc("huge"), entries(MAX_ENTRIES + 1));
        assert!(c.get(&loc("huge")).is_none());
    }

    #[test]
    fn invalidation_covers_subfolders() {
        let mut c = ListingCache::default();
        for p in ["", "Photos", "Photos/2025", "Docs"] {
            c.insert(loc(p), entries(1));
        }
        c.invalidate_within(&loc("Photos"));
        assert!(c.get(&loc("Photos")).is_none());
        assert!(c.get(&loc("Photos/2025")).is_none());
        assert!(c.get(&loc("Docs")).is_some());
        assert!(c.get(&loc("")).is_some());
    }
}
