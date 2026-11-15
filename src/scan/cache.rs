//! Scan results saved on disk (`$XDG_CACHE_HOME/ratclone/scans/`).

use std::path::Path;

use serde::{Deserialize, Serialize};

use std::collections::BTreeMap;

use super::tree::{Category, DirNode, ScanTree};
use crate::location::{Location, Remote};

/// 2: folders as a flat list (1 nested them, and deep trees could not be read back).
const VERSION: u32 = 2;

/// Deepest tree read back. Real folder trees stay far below; a deeper file is treated
/// as damaged rather than rebuilt, since the tree itself is dropped, cloned and
/// compared recursively and must fit a thread's stack (checked at 2 MiB, debug build).
pub const MAX_DEPTH: usize = 1024;

/// One folder without its children; they point back to it by index.
#[derive(Serialize, Deserialize)]
struct FlatDir {
    /// Index of the parent folder in the list; `None` only for the root (first).
    parent: Option<usize>,
    name: String,
    bytes: u64,
    files: u64,
    dirs: u64,
    by_type: BTreeMap<Category, u64>,
    largest: Vec<(String, u64)>,
    loose_bytes: u64,
    loose_files: u64,
    #[serde(default)]
    loose_largest: Vec<(String, u64)>,
}

impl FlatDir {
    fn of(parent: Option<usize>, name: &str, n: &DirNode) -> FlatDir {
        FlatDir {
            parent,
            name: name.to_string(),
            bytes: n.bytes,
            files: n.files,
            dirs: n.dirs,
            by_type: n.by_type.clone(),
            largest: n.largest.clone(),
            loose_bytes: n.loose_bytes,
            loose_files: n.loose_files,
            loose_largest: n.loose_largest.clone(),
        }
    }

    fn into_node(self) -> (Option<usize>, String, DirNode) {
        let node = DirNode {
            bytes: self.bytes,
            files: self.files,
            dirs: self.dirs,
            by_type: self.by_type,
            largest: self.largest,
            loose_bytes: self.loose_bytes,
            loose_files: self.loose_files,
            loose_largest: self.loose_largest,
            children: BTreeMap::new(),
        };
        (self.parent, self.name, node)
    }
}

#[derive(Serialize, Deserialize)]
struct Flat {
    version: u32,
    root: Location,
    unreadable: u64,
    listed: u64,
    scanned_at: jiff::Timestamp,
    duration_secs: u64,
    partial: bool,
    /// Parents before their children, the root first.
    dirs: Vec<FlatDir>,
}

/// Just the version, to tell formats apart before reading the rest.
#[derive(Deserialize)]
struct Version {
    version: u32,
}

/// The tree as a list, walked without recursion (however deep it is).
fn flatten(tree: &ScanTree) -> Flat {
    let mut dirs = vec![FlatDir::of(None, "", &tree.node)];
    let mut stack: Vec<(usize, &DirNode)> = vec![(0, &tree.node)];
    while let Some((at, node)) = stack.pop() {
        for (name, child) in &node.children {
            dirs.push(FlatDir::of(Some(at), name, child));
            stack.push((dirs.len() - 1, child));
        }
    }
    Flat {
        version: VERSION,
        root: tree.root.clone(),
        unreadable: tree.unreadable,
        listed: tree.listed,
        scanned_at: tree.scanned_at,
        duration_secs: tree.duration_secs,
        partial: tree.partial,
        dirs,
    }
}

/// Rebuild the tree; `None` when the list does not describe one (or is too deep).
fn unflatten(flat: Flat) -> Option<ScanTree> {
    let mut nodes = Vec::with_capacity(flat.dirs.len());
    let mut depth = Vec::with_capacity(flat.dirs.len());
    for (i, d) in flat.dirs.into_iter().enumerate() {
        let (parent, name, node) = d.into_node();
        // Only the first is the root, and every parent comes before its children.
        let d = match parent {
            None if i == 0 => 0,
            Some(p) if p < i => depth[p] + 1,
            _ => return None,
        };
        if d > MAX_DEPTH {
            return None;
        }
        depth.push(d);
        nodes.push(Some((parent, name, node)));
    }
    // Children come after their parent: going backwards, each folder is complete
    // when it is moved into its parent.
    for i in (1..nodes.len()).rev() {
        let (parent, name, node) = nodes[i].take()?;
        let p = parent?;
        nodes[p].as_mut()?.2.children.insert(name, node);
    }
    let (_, _, node) = nodes.into_iter().next()??;
    Some(ScanTree {
        root: flat.root,
        node,
        unreadable: flat.unreadable,
        listed: flat.listed,
        scanned_at: flat.scanned_at,
        duration_secs: flat.duration_secs,
        partial: flat.partial,
    })
}

/// File name for the scan of `root`: the account name, or a hash of a local path.
pub fn key_file(root: &Location) -> String {
    match &root.remote {
        Remote::Named(n) => format!("{n}.json"),
        Remote::Local => {
            // FNV-1a: stable across runs and Rust versions, unlike the std hasher.
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            for b in root.path.bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
            format!("local-{h:016x}.json")
        }
    }
}

pub fn save(dir: &Path, tree: &ScanTree) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(key_file(&tree.root));
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(&flatten(tree))?)?;
    std::fs::rename(tmp, path)
}

/// The saved scan of `root`; anything missing, unreadable or from another version is `None`.
pub fn load(dir: &Path, root: &Location) -> Option<ScanTree> {
    let text = std::fs::read(dir.join(key_file(root))).ok()?;
    let version: Version = serde_json::from_slice(&text).ok()?;
    if version.version != VERSION {
        return None;
    }
    let tree = unflatten(serde_json::from_slice(&text).ok()?)?;
    (&tree.root == root).then_some(tree)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Entry;
    use crate::location::Remote;
    use crate::scan::tree::DirNode;

    fn tree() -> ScanTree {
        let mut t = ScanTree::new(Location::new(Remote::Named("personal".into()), ""));
        t.add_listing(
            "",
            &[Entry {
                name: "a.jpg".into(),
                size: Some(10),
                mod_time: None,
                is_dir: false,
                mime: String::new(),
            }],
        );
        t.partial = true;
        t
    }

    #[test]
    fn cache_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let t = tree();
        save(d.path(), &t).unwrap();
        assert_eq!(load(d.path(), &t.root), Some(t));
        let other = Location::new(Remote::Named("work".into()), "");
        assert_eq!(load(d.path(), &other), None);
    }

    #[test]
    fn corrupt_cache_is_ignored() {
        let d = tempfile::tempdir().unwrap();
        let t = tree();
        let file = d.path().join(key_file(&t.root));
        std::fs::write(&file, "{").unwrap();
        assert_eq!(load(d.path(), &t.root), None);
        save(d.path(), &t).unwrap();
        let text = std::fs::read_to_string(&file)
            .unwrap()
            .replace("\"version\":2", "\"version\":0");
        std::fs::write(&file, text).unwrap();
        assert_eq!(load(d.path(), &t.root), None);
    }

    #[test]
    fn local_keys_differ_by_path() {
        let a = key_file(&Location::new(Remote::Local, "home/a"));
        let b = key_file(&Location::new(Remote::Local, "home/b"));
        assert_ne!(a, b);
        assert!(a.starts_with("local-") && a.ends_with(".json"));
        assert_eq!(key_file(&Location::new(Remote::Named("g".into()), "")), "g.json");
    }

    /// A chain of `depth` folders below the root, each with one file (built directly:
    /// listing it folder by folder would take long at these depths).
    fn deep(depth: usize) -> ScanTree {
        let mut node = DirNode::default();
        for i in (0..=depth).rev() {
            let mut parent = DirNode {
                bytes: node.bytes + i as u64 + 1,
                files: node.files + 1,
                dirs: node.dirs + u64::from(i < depth),
                loose_bytes: i as u64 + 1,
                loose_files: 1,
                loose_largest: vec![(format!("f{i}.bin"), i as u64 + 1)],
                ..DirNode::default()
            };
            if i < depth {
                parent.children.insert(format!("d{i}"), node);
            }
            node = parent;
        }
        let mut t = ScanTree::new(Location::new(Remote::Named("deep".into()), ""));
        t.node = node;
        t
    }

    #[test]
    fn deep_trees_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        for depth in [70, 1000] {
            let t = deep(depth);
            save(d.path(), &t).unwrap();
            let back = load(d.path(), &t.root).unwrap_or_else(|| panic!("depth {depth} not read back"));
            assert_eq!(back, t);
        }
    }

    #[test]
    fn depth_limit_is_enforced_on_load() {
        let d = tempfile::tempdir().unwrap();
        let t = deep(MAX_DEPTH);
        save(d.path(), &t).unwrap();
        assert_eq!(
            load(d.path(), &t.root).as_ref(),
            Some(&t),
            "the supported depth reads back"
        );
        let t = deep(MAX_DEPTH + 1);
        save(d.path(), &t).unwrap();
        assert_eq!(load(d.path(), &t.root), None);
    }

    #[test]
    fn malformed_lists_are_refused() {
        let d = tempfile::tempdir().unwrap();
        let t = tree();
        save(d.path(), &t).unwrap();
        let file = d.path().join(key_file(&t.root));
        let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        // A child pointing at itself (or forward) is not a tree.
        v["dirs"].as_array_mut().unwrap().push(serde_json::json!({
            "parent": 1, "name": "x", "bytes": 0, "files": 0, "dirs": 0, "by_type": {},
            "largest": [], "loose_bytes": 0, "loose_files": 0
        }));
        std::fs::write(&file, serde_json::to_vec(&v).unwrap()).unwrap();
        assert_eq!(load(d.path(), &t.root), None);
        // An old nested (version 1) file is ignored, then replaced by the next save.
        std::fs::write(&file, r#"{"version":1,"tree":{}}"#).unwrap();
        assert_eq!(load(d.path(), &t.root), None);
    }
}
