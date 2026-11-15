//! One file pane: a listing of a folder with cursor, filter, sort and marks.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::sync::Arc;

use crate::backend::Entry;
use crate::input::keymap::SortKey;
use crate::location::Location;

#[derive(Clone, Debug, PartialEq)]
pub enum Listing {
    Loading,
    Ready,
    Failed(String),
}

#[derive(Debug)]
pub struct PaneState {
    pub loc: Location,
    /// Every entry of the folder, in the order the backend gave them
    /// (shared with the listing cache).
    all: Arc<Vec<Entry>>,
    /// Position of each entry of `all` in natural name order (computed once per listing).
    name_rank: Vec<u32>,
    /// Lower-cased names, parallel to `all`; built the first time a filter is typed.
    lower: Option<Vec<String>>,
    /// Every index of `all` in the current sort order (hidden ones included).
    sorted: Vec<u32>,
    /// The visible rows as indices into `all`: sorted, filtered, hidden files removed.
    view: Vec<u32>,
    pub cursor: usize,
    pub offset: usize,
    /// Marked names (changed only through the pane's methods, which keep
    /// `marked_bytes` in step). Marks hidden by a filter still count.
    pub marks: HashSet<String>,
    /// Sum of the sizes of the marked entries.
    marked_bytes: u64,
    /// Sum of the sizes of every entry of the folder.
    total_bytes: u64,
    pub visual_anchor: Option<usize>,
    pub filter: String,
    pub sort: SortKey,
    pub reverse: bool,
    pub show_hidden: bool,
    pub listing: Listing,
    /// The rows come from the listing cache and a fresh listing is on its way.
    pub refreshing: bool,
    /// Id of the listing request whose answer this pane is waiting for.
    pub req: u64,
    /// Name to put the cursor on when the pending listing arrives.
    pub pending_cursor: Option<String>,
    pub history: Vec<(Location, String)>,
    pub viewport_height: usize,
}

impl PaneState {
    pub fn new(loc: Location, show_hidden: bool) -> Self {
        PaneState {
            loc,
            all: Arc::default(),
            name_rank: Vec::new(),
            lower: None,
            sorted: Vec::new(),
            view: Vec::new(),
            cursor: 0,
            offset: 0,
            marks: HashSet::new(),
            marked_bytes: 0,
            total_bytes: 0,
            visual_anchor: None,
            filter: String::new(),
            sort: SortKey::Name,
            reverse: false,
            show_hidden,
            listing: Listing::Loading,
            refreshing: false,
            req: 0,
            pending_cursor: None,
            history: Vec::new(),
            viewport_height: 20,
        }
    }

    /// Replace the folder contents, keeping the cursor on `keep_cursor_on`
    /// (or on the same name as before) when it is still there.
    pub fn set_entries(&mut self, all: Vec<Entry>, keep_cursor_on: Option<&str>) {
        self.set_shared_entries(Arc::new(all), keep_cursor_on);
    }

    /// [`PaneState::set_entries`] for a listing that is also kept elsewhere.
    pub fn set_shared_entries(&mut self, all: Arc<Vec<Entry>>, keep_cursor_on: Option<&str>) {
        self.install(all, keep_cursor_on, false);
    }

    /// [`PaneState::set_shared_entries`] for a listing already put in order by [`sort_by_name`].
    pub fn set_sorted_entries(&mut self, all: Arc<Vec<Entry>>, keep_cursor_on: Option<&str>) {
        debug_assert!(all.is_sorted_by(|a, b| natural_cmp(&a.name, &b.name) != Ordering::Greater));
        self.install(all, keep_cursor_on, true);
    }

    fn install(&mut self, all: Arc<Vec<Entry>>, keep_cursor_on: Option<&str>, sorted: bool) {
        let keep = keep_cursor_on
            .map(str::to_string)
            .or_else(|| self.current().map(|e| e.name.clone()));
        if !self.marks.is_empty() {
            let names: HashSet<&str> = all.iter().map(|e| e.name.as_str()).collect();
            self.marks.retain(|m| names.contains(m.as_str()));
        }
        self.marked_bytes = if self.marks.is_empty() {
            0
        } else {
            all.iter()
                .filter(|e| self.marks.contains(&e.name))
                .filter_map(|e| e.size)
                .sum()
        };
        self.total_bytes = all.iter().filter_map(|e| e.size).sum();
        self.all = all;
        self.lower = None;
        self.name_rank = if sorted {
            (0..self.all.len() as u32).collect()
        } else {
            name_ranks(&self.all)
        };
        self.resort();
        self.refilter(false, keep.as_deref());
    }

    /// The visible rows, top to bottom.
    pub fn entries(&self) -> impl ExactSizeIterator<Item = &Entry> + DoubleEndedIterator + Clone + '_ {
        self.view.iter().map(|&i| &self.all[i as usize])
    }

    /// The visible row at `i`.
    pub fn entry(&self, i: usize) -> Option<&Entry> {
        self.view.get(i).map(|&j| &self.all[j as usize])
    }

    /// Number of visible rows.
    pub fn len(&self) -> usize {
        self.view.len()
    }

    pub fn is_empty(&self) -> bool {
        self.view.is_empty()
    }

    /// Order every entry by the current sort key; directories always come first.
    fn resort(&mut self) {
        let (all, rank, reverse) = (&self.all, &self.name_rank, self.reverse);
        self.sorted = match self.sort {
            SortKey::Name => {
                // Walking the entries in name order needs no comparisons at all.
                let mut by_name = vec![0u32; all.len()];
                for (i, &r) in rank.iter().enumerate() {
                    by_name[r as usize] = i as u32;
                }
                let (mut dirs, mut files): (Vec<u32>, Vec<u32>) =
                    by_name.into_iter().partition(|&i| all[i as usize].is_dir);
                if reverse {
                    dirs.reverse();
                    files.reverse();
                }
                dirs.extend(files);
                dirs
            }
            // Larger and newer first; ties in name order.
            // Count only exists in the Space tab; files fall back to size.
            SortKey::Size | SortKey::Count => sort_by_key(all, rank, reverse, |e| std::cmp::Reverse(e.size)),
            SortKey::Modified => sort_by_key(all, rank, reverse, |e| std::cmp::Reverse(e.mod_time)),
        };
    }

    /// Recompute the visible rows from `sorted`. With `narrowing`, the new filter
    /// only matches a subset of the current rows, so only those are checked.
    fn refilter(&mut self, narrowing: bool, keep: Option<&str>) {
        let needle = self.filter.to_lowercase();
        if !needle.is_empty() && self.lower.is_none() {
            self.lower = Some(self.all.iter().map(|e| e.name.to_lowercase()).collect());
        }
        let (all, lower, show_hidden) = (&self.all, self.lower.as_deref(), self.show_hidden);
        let visible = |i: &u32| {
            let i = *i as usize;
            (show_hidden || !all[i].name.starts_with('.'))
                && (needle.is_empty() || lower.is_some_and(|l| l[i].contains(&needle)))
        };
        self.view = if narrowing {
            self.view.iter().copied().filter(visible).collect()
        } else {
            self.sorted.iter().copied().filter(visible).collect()
        };
        self.visual_anchor = None;
        match keep.and_then(|k| self.entries().position(|e| e.name == k)) {
            Some(i) => self.cursor = i,
            None => self.cursor = self.cursor.min(self.view.len().saturating_sub(1)),
        }
    }

    pub fn move_by(&mut self, delta: isize) {
        let last = self.view.len().saturating_sub(1) as isize;
        self.cursor = (self.cursor as isize + delta).clamp(0, last.max(0)) as usize;
    }

    pub fn top(&mut self) {
        self.cursor = 0;
    }

    pub fn bottom(&mut self) {
        self.cursor = self.view.len().saturating_sub(1);
    }

    /// Scroll so the cursor row is inside a viewport of `height` rows.
    pub fn ensure_visible(&mut self, height: usize) {
        let height = height.max(1);
        if self.cursor < self.offset {
            self.offset = self.cursor;
        } else if self.cursor >= self.offset + height {
            self.offset = self.cursor + 1 - height;
        }
        let max_offset = self.view.len().saturating_sub(height);
        self.offset = self.offset.min(max_offset);
    }

    pub fn current(&self) -> Option<&Entry> {
        self.entry(self.cursor)
    }

    /// What an operation applies to: marked entries (those a filter or the hidden
    /// files setting keeps out of sight too, as the count in the header says), else
    /// the visual range, else the cursor row.
    pub fn selection(&self) -> Vec<Entry> {
        if !self.marks.is_empty() {
            return self
                .sorted
                .iter()
                .map(|&i| &self.all[i as usize])
                .filter(|e| self.marks.contains(&e.name))
                .cloned()
                .collect();
        }
        if let Some(anchor) = self.visual_anchor {
            let (lo, hi) = (anchor.min(self.cursor), anchor.max(self.cursor));
            return self.entries().skip(lo).take(hi + 1 - lo).cloned().collect();
        }
        self.current().cloned().into_iter().collect()
    }

    pub fn set_filter(&mut self, f: &str) {
        let keep = self.current().map(|e| e.name.clone());
        let narrowing = !self.filter.is_empty() && f.to_lowercase().contains(&self.filter.to_lowercase());
        self.filter = f.to_string();
        self.refilter(narrowing, keep.as_deref());
    }

    /// With a filter active, cycle through the matching rows.
    pub fn next_match(&mut self, forward: bool) {
        let n = self.view.len();
        if n == 0 || self.filter.is_empty() {
            return;
        }
        self.cursor = if forward {
            (self.cursor + 1) % n
        } else {
            (self.cursor + n - 1) % n
        };
    }

    pub fn set_sort(&mut self, key: SortKey) {
        self.sort = key;
        let keep = self.current().map(|e| e.name.clone());
        self.resort();
        self.refilter(false, keep.as_deref());
    }

    pub fn toggle_reverse(&mut self) {
        self.reverse = !self.reverse;
        let keep = self.current().map(|e| e.name.clone());
        self.resort();
        self.refilter(false, keep.as_deref());
    }

    pub fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        let keep = self.current().map(|e| e.name.clone());
        self.refilter(false, keep.as_deref());
    }

    /// Mark or unmark the cursor row, then move down (like Space in ranger).
    pub fn toggle_mark(&mut self) {
        if let Some((name, size)) = self.current().map(|e| (e.name.clone(), e.size.unwrap_or(0))) {
            if self.marks.remove(&name) {
                self.marked_bytes = self.marked_bytes.saturating_sub(size);
            } else {
                self.marks.insert(name);
                self.marked_bytes += size;
            }
            self.move_by(1);
        }
    }

    /// How many entries are marked, and their total size, without walking the folder.
    pub fn marked_summary(&self) -> (usize, u64) {
        (self.marks.len(), self.marked_bytes)
    }

    /// Total size of the folder's files (computed once per listing).
    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    pub fn clear_selection(&mut self) {
        self.marks.clear();
        self.marked_bytes = 0;
        self.visual_anchor = None;
    }

    /// Forget the current folder's rows before showing another folder.
    pub fn clear_contents(&mut self) {
        self.all = Arc::default();
        self.total_bytes = 0;
        self.name_rank.clear();
        self.lower = None;
        self.sorted.clear();
        self.view.clear();
        self.cursor = 0;
        self.offset = 0;
        self.clear_selection();
        self.filter.clear();
    }

    /// Names of every entry in the folder, hidden or filtered out included.
    pub fn all_names(&self) -> HashSet<String> {
        self.all.iter().map(|e| e.name.clone()).collect()
    }

    /// Whether the folder holds an entry called `name`, hidden or filtered out included.
    pub fn has_name(&self, name: &str) -> bool {
        self.all.iter().any(|e| e.name == name)
    }

    pub fn all_entries(&self) -> &[Entry] {
        &self.all
    }
}

/// Indices of `all`, directories first, then by `key`, then by name rank;
/// `reverse` flips everything but the directories-first rule.
fn sort_by_key<K: Ord>(all: &[Entry], rank: &[u32], reverse: bool, key: impl Fn(&Entry) -> K) -> Vec<u32> {
    // Compact keys next to the index: no pointer chasing while sorting.
    let mut keyed: Vec<(bool, K, u32, u32)> = all
        .iter()
        .enumerate()
        .map(|(i, e)| (!e.is_dir, key(e), rank[i], i as u32))
        .collect();
    // Ranks are unique, so every key is a total order and an unstable sort is exact.
    keyed.sort_unstable_by(|a, b| {
        let by_key = a.1.cmp(&b.1).then(a.2.cmp(&b.2));
        a.0.cmp(&b.0)
            .then(if reverse { by_key.reverse() } else { by_key })
    });
    keyed.into_iter().map(|k| k.3).collect()
}

/// Put a listing in natural name order. Listings are sorted this way in the
/// background task that fetched them, so [`PaneState::set_entries`] only has to
/// check the order instead of sorting on the UI thread.
pub fn sort_by_name(entries: &mut Vec<Entry>) {
    // Sort small (name, index) pairs rather than whole entries, then move each
    // entry once into place. Equal names keep their order (the index breaks ties).
    let order: Vec<(&str, u32)> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| (e.name.as_str(), i as u32))
        .collect();
    let cmp = |a: &(&str, u32), b: &(&str, u32)| natural_cmp(a.0, b.0).then(a.1.cmp(&b.1));
    let order: Vec<u32> = parallel_sort(order, cmp).into_iter().map(|(_, i)| i).collect();
    let mut slots: Vec<Option<Entry>> = std::mem::take(entries).into_iter().map(Some).collect();
    *entries = order
        .into_iter()
        .map(|i| slots[i as usize].take().expect("each index once"))
        .collect();
}

/// Below this many items a single thread sorts faster than splitting the work.
const PARALLEL_SORT_MIN: usize = 16_384;

/// Sort with every core for big inputs: chunks sorted on scoped threads, then
/// merged pairwise. `cmp` must be a total order (ties broken), so the result is
/// exactly what a plain sort gives.
fn parallel_sort<T: Send + Copy>(mut v: Vec<T>, cmp: impl Fn(&T, &T) -> Ordering + Sync) -> Vec<T> {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(8);
    if v.len() < PARALLEL_SORT_MIN || threads < 2 {
        v.sort_unstable_by(&cmp);
        return v;
    }
    let chunk = v.len().div_ceil(threads);
    std::thread::scope(|s| {
        for part in v.chunks_mut(chunk) {
            let cmp = &cmp;
            s.spawn(move || part.sort_unstable_by(cmp));
        }
    });
    let mut runs: Vec<Vec<T>> = v.chunks(chunk).map(<[T]>::to_vec).collect();
    while runs.len() > 1 {
        let mut next = Vec::with_capacity(runs.len().div_ceil(2));
        let mut it = runs.into_iter();
        std::thread::scope(|s| {
            let mut handles = Vec::new();
            while let Some(a) = it.next() {
                match it.next() {
                    Some(b) => {
                        let cmp = &cmp;
                        handles.push(s.spawn(move || merge(&a, &b, cmp)));
                    }
                    None => next.push(a),
                }
            }
            // Merged runs go before a leftover one; order of runs does not matter for merging.
            let merged: Vec<Vec<T>> = handles
                .into_iter()
                .map(|h| h.join().expect("merge thread"))
                .collect();
            next.splice(0..0, merged);
        });
        runs = next;
    }
    runs.pop().unwrap_or_default()
}

fn merge<T: Copy>(a: &[T], b: &[T], cmp: &impl Fn(&T, &T) -> Ordering) -> Vec<T> {
    let mut out = Vec::with_capacity(a.len() + b.len());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if cmp(&b[j], &a[i]) == Ordering::Less {
            out.push(b[j]);
            j += 1;
        } else {
            out.push(a[i]);
            i += 1;
        }
    }
    out.extend_from_slice(&a[i..]);
    out.extend_from_slice(&b[j..]);
    out
}

/// For each entry, its position when the folder is sorted by [`natural_cmp`].
fn name_ranks(all: &[Entry]) -> Vec<u32> {
    if all.is_sorted_by(|a, b| natural_cmp(&a.name, &b.name) != Ordering::Greater) {
        return (0..all.len() as u32).collect();
    }
    // Keep the name next to the index: one pointer less to follow per comparison.
    let order: Vec<(&str, u32)> = all
        .iter()
        .enumerate()
        .map(|(i, e)| (e.name.as_str(), i as u32))
        .collect();
    let order = parallel_sort(order, |a, b| natural_cmp(a.0, b.0).then(a.1.cmp(&b.1)));
    let mut rank = vec![0u32; all.len()];
    for (pos, &(_, i)) in order.iter().enumerate() {
        rank[i as usize] = pos as u32;
    }
    rank
}

/// The run of ASCII digits starting at `*k`; leaves `*k` just after it.
fn digit_run<'a>(s: &'a [u8], k: &mut usize) -> &'a [u8] {
    let start = *k;
    while s.get(*k).is_some_and(u8::is_ascii_digit) {
        *k += 1;
    }
    &s[start..*k]
}

/// Case-insensitive natural order: `img2` < `img10`.
///
/// Works on bytes with an ASCII fast path and never allocates; digit runs are
/// compared by value, then by length (fewer leading zeros first), and names
/// that compare equal fall back to plain byte order.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (x, y) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    loop {
        let (c, d) = match (x.get(i), y.get(j)) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(&c), Some(&d)) => (c, d),
        };
        let ord = if c.is_ascii_digit() && d.is_ascii_digit() {
            let (m, n) = (digit_run(x, &mut i), digit_run(y, &mut j));
            let skip_zeros = |s: &[u8]| s.iter().position(|&b| b != b'0').unwrap_or(s.len());
            let (mt, nt) = (&m[skip_zeros(m)..], &n[skip_zeros(n)..]);
            mt.len()
                .cmp(&nt.len())
                .then_with(|| mt.cmp(nt))
                .then_with(|| m.len().cmp(&n.len()))
        } else if c.is_ascii() && d.is_ascii() {
            i += 1;
            j += 1;
            c.to_ascii_lowercase().cmp(&d.to_ascii_lowercase())
        } else {
            // Both positions are on char boundaries: digit runs and ASCII bytes are whole chars.
            let cc = a[i..].chars().next().unwrap_or_default();
            let dd = b[j..].chars().next().unwrap_or_default();
            i += cc.len_utf8();
            j += dd.len_utf8();
            cc.to_lowercase().cmp(dd.to_lowercase())
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::Remote;
    use std::time::Instant;

    fn e(name: &str, dir: bool, size: u64, t: &str) -> Entry {
        Entry {
            name: name.into(),
            size: (!dir).then_some(size),
            mod_time: Some(t.parse().unwrap()),
            is_dir: dir,
            mime: String::new(),
        }
    }

    fn pane(entries: Vec<Entry>) -> PaneState {
        let mut p = PaneState::new(Location::new(Remote::Named("g".into()), ""), false);
        p.set_entries(entries, None);
        p
    }

    fn names(p: &PaneState) -> Vec<&str> {
        p.entries().map(|e| e.name.as_str()).collect()
    }

    fn sample() -> Vec<Entry> {
        vec![
            e("img10.jpg", false, 5, "2025-01-03T00:00:00Z"),
            e("Zeta", true, 0, "2025-01-01T00:00:00Z"),
            e("img2.jpg", false, 50, "2025-01-02T00:00:00Z"),
            e(".hidden", false, 1, "2025-01-05T00:00:00Z"),
            e("alpha", true, 0, "2025-01-04T00:00:00Z"),
            e("Beta.txt", false, 7, "2025-01-06T00:00:00Z"),
        ]
    }

    /// The original char-by-char implementation, kept as the reference.
    fn natural_cmp_reference(a: &str, b: &str) -> Ordering {
        let mut x = a.chars().peekable();
        let mut y = b.chars().peekable();
        loop {
            match (x.peek().copied(), y.peek().copied()) {
                (None, None) => return a.cmp(b),
                (None, Some(_)) => return Ordering::Less,
                (Some(_), None) => return Ordering::Greater,
                (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                    let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                        let mut s = String::new();
                        while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                            s.push(c);
                            it.next();
                        }
                        s
                    };
                    let (m, n) = (take(&mut x), take(&mut y));
                    let (mt, nt) = (m.trim_start_matches('0'), n.trim_start_matches('0'));
                    let ord = mt
                        .len()
                        .cmp(&nt.len())
                        .then_with(|| mt.cmp(nt))
                        .then_with(|| n.len().cmp(&m.len()).reverse());
                    if ord != Ordering::Equal {
                        return ord;
                    }
                }
                (Some(c), Some(d)) => {
                    let ord = c.to_lowercase().cmp(d.to_lowercase());
                    if ord != Ordering::Equal {
                        return ord;
                    }
                    x.next();
                    y.next();
                }
            }
        }
    }

    #[test]
    fn natural_cmp_matches_reference() {
        // Deterministic pseudo-random names over a small alphabet that stresses
        // digit runs, leading zeros, case and multi-byte characters.
        let alphabet: Vec<char> = "aAbZ0019 ._-éÉßİıÑ日".chars().collect();
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let names: Vec<String> = (0..600)
            .map(|_| {
                let len = (next() % 7) as usize;
                (0..len)
                    .map(|_| alphabet[(next() % alphabet.len() as u64) as usize])
                    .collect()
            })
            .collect();
        for a in &names {
            for b in &names {
                assert_eq!(natural_cmp(a, b), natural_cmp_reference(a, b), "{a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn filter_narrowing_and_widening() {
        let mut p = pane(sample());
        p.set_filter("i");
        assert_eq!(names(&p), ["img2.jpg", "img10.jpg"]);
        p.set_filter("img1");
        assert_eq!(names(&p), ["img10.jpg"]);
        p.set_filter("img");
        assert_eq!(names(&p), ["img2.jpg", "img10.jpg"]);
        p.toggle_hidden();
        p.set_filter(".h");
        assert_eq!(names(&p), [".hidden"]);
        p.set_filter("");
        assert_eq!(p.len(), 6);
    }

    #[test]
    fn sort_by_name_is_natural_and_stable() {
        let mut v = sample();
        v.push(e("img2.jpg", false, 99, "2025-01-09T00:00:00Z"));
        sort_by_name(&mut v);
        let n: Vec<&str> = v.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            n,
            [
                ".hidden",
                "alpha",
                "Beta.txt",
                "img2.jpg",
                "img2.jpg",
                "img10.jpg",
                "Zeta"
            ]
        );
        assert_eq!(v[3].size, Some(50), "equal names keep their order");
    }

    #[test]
    fn parallel_sort_matches_plain_sort() {
        let mut seed = 7u64;
        let v: Vec<(u32, u32)> = (0..100_003u32)
            .map(|i| {
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                ((seed >> 40) as u32 % 1000, i)
            })
            .collect();
        let mut plain = v.clone();
        plain.sort();
        assert_eq!(parallel_sort(v, |a, b| a.cmp(b)), plain);
    }

    #[test]
    fn natural_order() {
        assert_eq!(natural_cmp("img2", "img10"), Ordering::Less);
        assert_eq!(natural_cmp("B", "a"), Ordering::Greater);
        assert_eq!(natural_cmp("a01", "a1"), Ordering::Greater);
    }

    #[test]
    fn dirs_first_then_natural_name() {
        assert_eq!(
            names(&pane(sample())),
            ["alpha", "Zeta", "Beta.txt", "img2.jpg", "img10.jpg"]
        );
    }

    #[test]
    fn sort_keys_and_reverse() {
        let mut p = pane(sample());
        p.set_sort(SortKey::Size);
        assert_eq!(names(&p)[2..], ["img2.jpg", "Beta.txt", "img10.jpg"]);
        p.set_sort(SortKey::Modified);
        assert_eq!(names(&p), ["alpha", "Zeta", "Beta.txt", "img10.jpg", "img2.jpg"]);
        p.set_sort(SortKey::Name);
        p.toggle_reverse();
        assert_eq!(names(&p), ["Zeta", "alpha", "img10.jpg", "img2.jpg", "Beta.txt"]);
    }

    #[test]
    fn hidden_toggle() {
        let mut p = pane(sample());
        assert!(!names(&p).contains(&".hidden"));
        p.toggle_hidden();
        assert!(names(&p).contains(&".hidden"));
    }

    #[test]
    fn filter_narrows_and_clamps_cursor() {
        let mut p = pane(sample());
        p.bottom();
        p.set_filter("IMG");
        assert_eq!(names(&p), ["img2.jpg", "img10.jpg"]);
        assert!(p.cursor < 2);
        p.set_filter("");
        assert_eq!(p.len(), 5);
    }

    #[test]
    fn next_match_wraps() {
        let mut p = pane(sample());
        p.set_filter("img");
        p.top();
        p.next_match(true);
        assert_eq!(p.current().unwrap().name, "img10.jpg");
        p.next_match(true);
        assert_eq!(p.current().unwrap().name, "img2.jpg");
        p.next_match(false);
        assert_eq!(p.current().unwrap().name, "img10.jpg");
    }

    #[test]
    fn cursor_moves_are_clamped() {
        let mut p = pane(sample());
        p.move_by(-3);
        assert_eq!(p.cursor, 0);
        p.move_by(100);
        assert_eq!(p.cursor, 4);
        p.top();
        assert_eq!(p.cursor, 0);
        let mut empty = pane(vec![]);
        empty.move_by(1);
        assert_eq!(empty.cursor, 0);
        assert!(empty.current().is_none());
    }

    #[test]
    fn keep_cursor_on_name() {
        let mut p = pane(sample());
        p.set_entries(sample(), Some("img2.jpg"));
        assert_eq!(p.current().unwrap().name, "img2.jpg");
        p.set_entries(sample(), Some("gone"));
        assert!(p.cursor < p.len());
    }

    #[test]
    fn marks_hidden_by_a_filter_still_count() {
        let mut p = pane(sample());
        p.set_filter("img");
        p.toggle_mark();
        p.set_filter("beta");
        p.toggle_mark();
        assert_eq!(p.marks.len(), 2);
        let mut got: Vec<String> = p.selection().into_iter().map(|e| e.name).collect();
        got.sort();
        assert_eq!(
            got,
            ["Beta.txt", "img2.jpg"],
            "what the header counts is what is used"
        );
        // Every mark out of sight: still the marks, not the row under the cursor.
        p.set_filter("alpha");
        let got: Vec<String> = p.selection().into_iter().map(|e| e.name).collect();
        assert_eq!(got.len(), 2, "{got:?}");
        let bytes: u64 = p.selection().iter().filter_map(|e| e.size).sum();
        assert_eq!(p.marked_summary(), (2, bytes));
        p.set_filter("img");
        p.toggle_mark();
        assert_eq!(p.marked_summary().0, 1);
        assert_eq!(
            p.marked_summary().1,
            p.selection().iter().filter_map(|e| e.size).sum::<u64>()
        );
        p.clear_selection();
        assert_eq!(p.marked_summary(), (0, 0));
    }

    #[test]
    fn selection_priority() {
        let mut p = pane(sample());
        assert_eq!(
            p.selection().iter().map(|e| e.name.clone()).collect::<Vec<_>>(),
            ["alpha"]
        );
        p.visual_anchor = Some(1);
        p.cursor = 3;
        let v: Vec<String> = p.selection().into_iter().map(|e| e.name).collect();
        assert_eq!(v, ["Zeta", "Beta.txt", "img2.jpg"]);
        p.toggle_mark(); // marks img2.jpg and moves down
        p.visual_anchor = None;
        p.cursor = 0;
        p.toggle_mark();
        let m: Vec<String> = p.selection().into_iter().map(|e| e.name).collect();
        assert_eq!(m, ["alpha", "img2.jpg"]);
        p.clear_selection();
        assert!(p.marks.is_empty() && p.visual_anchor.is_none());
    }

    #[test]
    fn ensure_visible_scrolls() {
        let mut p = pane(
            (0..100)
                .map(|i| e(&format!("f{i}"), false, 1, "2025-01-01T00:00:00Z"))
                .collect(),
        );
        p.cursor = 50;
        p.ensure_visible(10);
        assert!(p.offset <= 50 && 50 < p.offset + 10);
        p.cursor = 0;
        p.ensure_visible(10);
        assert_eq!(p.offset, 0);
    }

    #[test]
    fn large_folder_stays_fast() {
        let many: Vec<Entry> = (0..50_000)
            .map(|i| e(&format!("img{i}.jpg"), false, i, "2025-01-01T00:00:00Z"))
            .collect();
        let start = Instant::now();
        let mut p = pane(many);
        for _ in 0..1000 {
            p.move_by(1);
            p.ensure_visible(40);
        }
        p.set_filter("img4");
        let ms = start.elapsed().as_millis();
        // 200 ms is the target in optimised builds; debug builds get headroom.
        let limit = if cfg!(debug_assertions) { 1000 } else { 200 };
        assert!(ms < limit, "took {ms} ms");
        assert!(!p.is_empty());
    }
}
