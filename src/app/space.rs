//! The Space tab: rows derived from a scan tree, and the scan's lifecycle.

use std::time::{Duration, Instant};

use super::overlay::{InputPurpose, LineInput, Overlay, PendingOp};
use super::{App, AppEvent, Focus, MessageKind, Tab};
use crate::format::human_size;
use crate::input::keymap::{Action, SortKey};
use crate::location::{Location, Remote};
use crate::scan::tree::{DirNode, ScanTree};
use crate::scan::walk::{ScanHandle, ScanMsg, start_walk};

/// While scanning, rows are re-sorted at most this often so they do not jump around.
const REORDER_EVERY: Duration = Duration::from_secs(1);

/// Where the Space tab is and what it shows.
#[derive(Debug, Default)]
pub struct SpaceState {
    /// The scan root being shown (follows the sidebar).
    pub root: Option<Location>,
    /// Folder shown, relative to the root (`""` is the root).
    pub rel: String,
    pub cursor: usize,
    /// First row on screen (set while drawing, so the list scrolls only when needed).
    pub offset: usize,
    pub sort: SpaceSort,
    pub filter: String,
    /// Rows in screen order.
    pub shown: Vec<SpaceRow>,
    last_reorder: Option<Instant>,
}

/// The scan in progress.
#[derive(Debug)]
pub struct ActiveScan {
    pub handle: ScanHandle,
    pub tree: ScanTree,
    pub started: Instant,
    /// Bytes the account says it uses, to estimate progress.
    pub expected_bytes: Option<u64>,
    /// Folder listed last.
    pub current: String,
    /// Why the top folder could not be listed: then the scan saw nothing at all.
    pub root_error: Option<String>,
}

fn parent_rel(rel: &str) -> Option<(String, String)> {
    if rel.is_empty() {
        return None;
    }
    Some(match rel.rsplit_once('/') {
        Some((p, name)) => (p.to_string(), name.to_string()),
        None => (String::new(), rel.to_string()),
    })
}

fn join_rel(rel: &str, name: &str) -> String {
    if rel.is_empty() {
        name.to_string()
    } else {
        format!("{rel}/{name}")
    }
}

fn account_name(r: &Remote) -> String {
    match r {
        Remote::Local => "local".into(),
        Remote::Named(n) => n.clone(),
    }
}

impl App {
    /// The root the Space tab shows: the sidebar's account (for the local disk,
    /// the folder open in the Files tab).
    pub fn space_root(&self) -> Option<Location> {
        let remote = self.sidebar.current()?.remote.clone();
        Some(match remote {
            // The local folder open in Files, or home when Files shows another account.
            Remote::Local if self.pane.loc.remote == Remote::Local => self.pane.loc.clone(),
            Remote::Local => self.home.clone(),
            other => Location::new(other, ""),
        })
    }

    /// The scan for [`App::space_root`]: the running one, else a finished one.
    pub fn space_tree(&self) -> Option<&ScanTree> {
        let root = self.space_root()?;
        match &self.scan {
            Some(a) if a.tree.root == root => Some(&a.tree),
            _ => self.scans.get(&root),
        }
    }

    /// Follow the sidebar: a new root resets the position and loads a cached scan.
    pub(super) fn sync_space(&mut self) {
        let root = self.space_root();
        if root != self.space.root {
            self.space.root = root.clone();
            self.space.rel.clear();
            self.space.cursor = 0;
            self.space.filter.clear();
            if let (Some(root), Some(dir)) = (root, self.scan_cache_dir.clone())
                && !self.scans.contains_key(&root)
                // Once per account, however fast the accounts are flicked through.
                && self.scans_loading.insert(root.clone())
            {
                // Big accounts make big files: read it off the UI thread.
                self.spawn(async move {
                    let at = root.clone();
                    let tree = tokio::task::spawn_blocking(move || crate::scan::cache::load(&dir, &at))
                        .await
                        .ok()
                        .flatten()
                        .map(Box::new);
                    AppEvent::ScanLoaded { root, tree }
                });
            }
        }
        self.recompute_space_rows(None);
    }

    /// Rebuild the rows, keeping the cursor on `keep` (or on the same name).
    pub(super) fn recompute_space_rows(&mut self, keep: Option<String>) {
        // At the top the cursor stays at the top: the largest row, whatever it is now.
        let at_top = keep.is_none() && self.space.cursor == 0;
        let keep = keep.filter(|_| !at_top).or_else(|| {
            if at_top {
                return None;
            }
            self.space.shown.get(self.space.cursor).map(|r| r.name.clone())
        });
        let rows = match self.space_tree().and_then(|t| t.node(&self.space.rel)) {
            Some(node) => rows(node, self.space.sort, &self.space.filter),
            None => Vec::new(),
        };
        self.space.cursor = if at_top {
            0
        } else {
            keep.and_then(|k| rows.iter().position(|r| r.name == k))
                .unwrap_or(self.space.cursor)
                .min(rows.len().saturating_sub(1))
        };
        self.space.shown = rows;
        self.space.last_reorder = Some(Instant::now());
    }

    pub fn start_scan(&mut self, root: Location) {
        if let Some(old) = self.scan.take() {
            // Its `Finished` will no longer match the active scan: release it here,
            // and keep what it found.
            old.handle.cancel();
            self.in_flight = self.in_flight.saturating_sub(1);
            self.finish_scan(old, true);
        }
        self.next_scan += 1;
        let id = self.next_scan;
        let kind = self
            .sidebar
            .remotes
            .iter()
            .find(|r| r.remote == root.remote)
            .map(|r| r.kind.clone())
            .unwrap_or_default();
        let expected_bytes = match root.remote {
            Remote::Local => None,
            _ => self.sidebar.about.get(&root.remote).and_then(|(a, _)| a.used),
        };
        let concurrency = crate::scan::concurrency_for(&kind, self.scan_concurrency);
        let handle = start_walk(
            self.backend.clone(),
            root.clone(),
            concurrency,
            id,
            self.tx.clone(),
        );
        // One in-flight count for the whole scan, released by `Finished`.
        self.in_flight += 1;
        self.scan = Some(ActiveScan {
            handle,
            tree: ScanTree::new(root),
            started: Instant::now(),
            expected_bytes,
            current: String::new(),
            root_error: None,
        });
        self.recompute_space_rows(None);
    }

    pub fn cancel_scan(&mut self) {
        if let Some(a) = &self.scan {
            a.handle.cancel();
        }
    }

    pub(super) fn on_scan(&mut self, id: u64, msg: ScanMsg) {
        let Some(active) = self.scan.as_mut().filter(|a| a.handle.id == id) else {
            return;
        };
        match msg {
            ScanMsg::Listed { rel, result } => {
                match result {
                    Ok(entries) => active.tree.add_listing(&rel, &entries),
                    Err(e) => {
                        if rel.is_empty() {
                            active.root_error = Some(e.to_string());
                        }
                        active.tree.add_unreadable(&rel)
                    }
                }
                active.current = rel;
                let due = self.space.shown.is_empty()
                    || self
                        .space
                        .last_reorder
                        .is_none_or(|t| t.elapsed() >= REORDER_EVERY);
                if self.tab == Tab::Space && due {
                    self.recompute_space_rows(None);
                }
            }
            ScanMsg::Finished { cancelled } => {
                self.in_flight = self.in_flight.saturating_sub(1);
                if let Some(a) = self.scan.take() {
                    self.finish_scan(a, cancelled);
                }
            }
        }
    }

    /// Mark `remote` as changed by the app now (its scans may be out of date).
    pub fn note_change(&mut self, remote: &Remote) {
        self.changed_since_scan.insert(remote.clone(), Instant::now());
    }

    /// Store a scan that ended (or was cancelled), and cache it.
    fn finish_scan(&mut self, mut a: ActiveScan, cancelled: bool) {
        // The account was removed while it was being scanned.
        if let Remote::Named(_) = &a.tree.root.remote
            && !self
                .sidebar
                .remotes
                .iter()
                .any(|r| r.remote == a.tree.root.remote)
        {
            self.recompute_space_rows(None);
            return;
        }
        a.tree.partial = cancelled;
        a.tree.duration_secs = a.started.elapsed().as_secs();
        a.tree.scanned_at = jiff::Timestamp::now();
        let name = account_name(&a.tree.root.remote);
        // Nothing could be read: that says nothing about the account's size, so it
        // replaces nothing (in memory or in the cache).
        if let Some(e) = a.root_error.take().filter(|_| !cancelled) {
            let kept = if self.scans.contains_key(&a.tree.root) {
                " — kept the previous scan"
            } else {
                ""
            };
            self.info(format!("Could not scan {name}: {e}{kept}"), MessageKind::Error);
            self.recompute_space_rows(None);
            return;
        }
        // A cancelled rescan must not replace a complete scan with a partial one.
        let keeps_old = cancelled && self.scans.get(&a.tree.root).is_some_and(|old| !old.partial);
        if keeps_old {
            self.info("Rescan cancelled — kept the previous scan", MessageKind::Info);
            self.recompute_space_rows(None);
            return;
        }
        if cancelled {
            self.info("Scan cancelled — partial results kept", MessageKind::Info);
        } else {
            let files = a.tree.node.files;
            let size = human_size(a.tree.node.bytes);
            match a.tree.unreadable {
                0 => self.info(format!("Scanned {name}: {size} in {files} files"), MessageKind::Success),
                n => self.info(
                    format!(
                        "Scanned {name}: {size} in {files} files — incomplete, {n} folder{} could not be read",
                        if n == 1 { "" } else { "s" }
                    ),
                    MessageKind::Error,
                ),
            }
            // Only changes made before the scan began are covered by it.
            let remote = &a.tree.root.remote;
            if self
                .changed_since_scan
                .get(remote)
                .is_some_and(|at| *at < a.started)
            {
                self.changed_since_scan.remove(remote);
            }
        }
        if let Some(dir) = self.scan_cache_dir.clone() {
            let tree = a.tree.clone();
            self.spawn(async move {
                let result = tokio::task::spawn_blocking(move || crate::scan::cache::save(&dir, &tree))
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|r| r.map_err(|e| e.to_string()));
                AppEvent::ScanSaved(result)
            });
        }
        self.scans.insert(a.tree.root.clone(), a.tree);
        self.recompute_space_rows(None);
    }

    /// Enter on an account that has no scan yet: scan it, or ask to replace a running one.
    fn request_scan(&mut self, root: Location) {
        match &self.scan {
            Some(a) if a.tree.root != root => {
                let from = account_name(&a.tree.root.remote);
                let to = account_name(&root.remote);
                self.overlay = Some(Overlay::Confirm {
                    title: "Replace scan".into(),
                    lines: vec![format!("Cancel the {from} scan and scan {to}?")],
                    commands: Vec::new(),
                    danger: false,
                    on_yes: PendingOp::ReplaceScan(root),
                });
            }
            Some(_) => {}
            None => self.start_scan(root),
        }
    }

    pub(super) fn space_action(&mut self, action: Action, count: u32) {
        if self.common_action(&action) {
            return;
        }
        let n = count as usize;
        let last = self.space.shown.len().saturating_sub(1);
        let half = (self.space_viewport / 2).max(1);
        let root = self.space_root();
        let has_tree = self.space_tree().is_some();
        match action {
            Action::Down => self.space.cursor = (self.space.cursor + n).min(last),
            Action::Up => self.space.cursor = self.space.cursor.saturating_sub(n),
            Action::Top => self.space.cursor = 0,
            Action::Bottom => self.space.cursor = last,
            Action::HalfDown => self.space.cursor = (self.space.cursor + half * n).min(last),
            Action::HalfUp => self.space.cursor = self.space.cursor.saturating_sub(half * n),
            Action::ToggleFocus => self.focus = Focus::Sidebar,
            Action::NextAccount => self.cycle_account(count as isize),
            Action::PrevAccount => self.cycle_account(-(count as isize)),
            Action::Open => match (root, has_tree) {
                (Some(root), false) => self.request_scan(root),
                (Some(_), true) => {
                    if let Some(row) = self.space.shown.get(self.space.cursor).filter(|r| r.is_dir) {
                        self.space.rel = join_rel(&self.space.rel, &row.name);
                        self.space.cursor = 0;
                        self.space.filter.clear();
                        self.recompute_space_rows(Some(String::new()));
                        self.space.cursor = 0;
                    }
                }
                (None, _) => {}
            },
            Action::Parent => {
                if let Some((parent, child)) = parent_rel(&self.space.rel) {
                    self.space.rel = parent;
                    self.space.filter.clear();
                    self.recompute_space_rows(Some(child));
                }
            }
            Action::Sort(key) => {
                self.space.sort = match key {
                    SortKey::Name => SpaceSort::Name,
                    SortKey::Count => SpaceSort::Count,
                    SortKey::Size | SortKey::Modified => SpaceSort::Size,
                };
                self.recompute_space_rows(None);
            }
            Action::Filter => {
                let input = LineInput::new(&self.space.filter, usize::MAX);
                self.overlay = Some(Overlay::Input {
                    title: "Filter".into(),
                    input,
                    purpose: InputPurpose::SpaceFilter,
                });
            }
            Action::Rescan => match (root, has_tree) {
                (Some(root), _) if self.scan.as_ref().is_some_and(|a| a.tree.root == root) => {
                    self.info("Already scanning this account", MessageKind::Info);
                }
                (Some(root), true) if self.scan.is_some() => self.request_scan(root),
                (Some(root), true) => {
                    let took = self.space_tree().map(|t| t.duration_secs).unwrap_or(0);
                    let name = account_name(&root.remote);
                    let took = if took < 60 {
                        format!("{took} s")
                    } else {
                        format!("{} min", took.div_ceil(60))
                    };
                    self.overlay = Some(Overlay::Confirm {
                        title: "Rescan".into(),
                        lines: vec![format!("Rescan {name}? Took {took} last time.")],
                        commands: Vec::new(),
                        danger: false,
                        on_yes: PendingOp::Rescan(root),
                    });
                }
                (Some(root), false) => self.request_scan(root),
                (None, _) => {}
            },
            Action::OpenInFiles => {
                if let Some(root) = root {
                    let rel = match self.space.shown.get(self.space.cursor) {
                        Some(r) if r.is_dir => join_rel(&self.space.rel, &r.name),
                        _ => self.space.rel.clone(),
                    };
                    let loc = if rel.is_empty() { root } else { root.join(&rel) };
                    self.set_tab(Tab::Files);
                    if loc != self.pane.loc {
                        self.navigate(loc, None);
                    }
                }
            }
            Action::CloseTab => self.set_tab(Tab::Files),
            Action::Escape => {
                let ours = self
                    .scan
                    .as_ref()
                    .is_some_and(|a| Some(&a.tree.root) == self.space.root.as_ref());
                if ours {
                    self.cancel_scan();
                } else if !self.space.filter.is_empty() {
                    self.space.filter.clear();
                    self.recompute_space_rows(None);
                } else {
                    self.set_tab(Tab::Files);
                }
            }
            _ => {}
        }
    }

    pub(super) fn set_space_filter(&mut self, value: &str) {
        self.space.filter = value.to_string();
        self.recompute_space_rows(None);
    }

    pub(super) fn run_scan_op(&mut self, op: PendingOp) {
        match op {
            PendingOp::Rescan(root) | PendingOp::ReplaceScan(root) => self.start_scan(root),
            _ => {}
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SpaceSort {
    #[default]
    Size,
    Name,
    Count,
}

/// One line of the Space list: a child folder, or all loose files together.
#[derive(Clone, Debug, PartialEq)]
pub struct SpaceRow {
    pub name: String,
    pub is_dir: bool,
    pub bytes: u64,
    pub files: u64,
}

/// `part` as a fraction of `whole`; 0 when there is nothing to divide.
pub fn share(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64
    }
}

/// Child folders of `node` in `sort` order (filtered by name), then the loose files.
pub fn rows(node: &DirNode, sort: SpaceSort, filter: &str) -> Vec<SpaceRow> {
    let needle = filter.to_lowercase();
    let mut out: Vec<SpaceRow> = node
        .children
        .iter()
        .filter(|(name, _)| needle.is_empty() || name.to_lowercase().contains(&needle))
        .map(|(name, n)| SpaceRow {
            name: name.clone(),
            is_dir: true,
            bytes: n.bytes,
            files: n.files,
        })
        .collect();
    // Each name is lower-cased once, not at every comparison: early in a scan most
    // folders still weigh nothing, and the name then decides nearly all of them.
    use std::cmp::Reverse;
    match sort {
        SpaceSort::Size => out.sort_by_cached_key(|r| (Reverse(r.bytes), r.name.to_lowercase())),
        SpaceSort::Count => out.sort_by_cached_key(|r| (Reverse(r.files), r.name.to_lowercase())),
        SpaceSort::Name => out.sort_by_cached_key(|r| r.name.to_lowercase()),
    }
    if node.loose_files > 0 {
        let n = node.loose_files;
        out.push(SpaceRow {
            name: format!("({n} {})", if n == 1 { "file" } else { "files" }),
            is_dir: false,
            bytes: node.loose_bytes,
            files: n,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node() -> DirNode {
        let mut n = DirNode::default();
        for (name, bytes, files) in [("Photos", 4100, 6210), ("Videos", 2600, 148), ("Books", 80, 900)] {
            n.children.insert(
                name.into(),
                DirNode {
                    bytes,
                    files,
                    ..Default::default()
                },
            );
        }
        n.loose_bytes = 50;
        n.loose_files = 3;
        n
    }

    fn names(rows: &[SpaceRow]) -> Vec<&str> {
        rows.iter().map(|r| r.name.as_str()).collect()
    }

    #[test]
    fn rows_sorted_with_loose_last() {
        let rows = rows(&node(), SpaceSort::Size, "");
        assert_eq!(names(&rows), ["Photos", "Videos", "Books", "(3 files)"]);
        assert!(rows[0].is_dir && !rows[3].is_dir);
        assert_eq!((rows[3].bytes, rows[3].files), (50, 3));
    }

    #[test]
    fn rows_sort_by_name_and_count_and_filter() {
        assert_eq!(
            names(&rows(&node(), SpaceSort::Name, "")),
            ["Books", "Photos", "Videos", "(3 files)"]
        );
        assert_eq!(
            names(&rows(&node(), SpaceSort::Count, "")),
            ["Photos", "Books", "Videos", "(3 files)"]
        );
        assert_eq!(
            names(&rows(&node(), SpaceSort::Size, "VID")),
            ["Videos", "(3 files)"]
        );
        let mut one = node();
        one.loose_files = 1;
        assert_eq!(rows(&one, SpaceSort::Size, "").last().unwrap().name, "(1 file)");
        one.loose_files = 0;
        assert_eq!(rows(&one, SpaceSort::Size, "").len(), 3);
    }

    #[test]
    fn zero_byte_folder_rows_have_zero_percent() {
        assert_eq!(share(0, 0), 0.0);
        assert_eq!(share(5, 0), 0.0);
        assert_eq!(share(1, 4), 0.25);
    }
}
