//! The aggregated folder tree a scan builds (no per-file list is kept).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::backend::Entry;
use crate::location::Location;

/// Largest files remembered per folder.
pub const LARGEST_KEPT: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Category {
    Images,
    Video,
    Audio,
    Documents,
    Archives,
    Code,
    Other,
}

impl Category {
    pub const ALL: [Category; 7] = [
        Category::Images,
        Category::Video,
        Category::Audio,
        Category::Documents,
        Category::Archives,
        Category::Code,
        Category::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::Images => "Images",
            Category::Video => "Video",
            Category::Audio => "Audio",
            Category::Documents => "Documents",
            Category::Archives => "Archives",
            Category::Code => "Code",
            Category::Other => "Other",
        }
    }
}

/// What kind of file this is: the mime type first, the extension as fallback.
pub fn category(name: &str, mime: &str) -> Category {
    let m = mime.to_ascii_lowercase();
    if m.starts_with("image/") {
        return Category::Images;
    }
    if m.starts_with("video/") {
        return Category::Video;
    }
    if m.starts_with("audio/") {
        return Category::Audio;
    }
    if m.starts_with("text/") || m == "application/pdf" {
        return Category::Documents;
    }
    if let Some(app) = m.strip_prefix("application/")
        && [
            "zip",
            "x-tar",
            "gzip",
            "x-7z-compressed",
            "x-rar-compressed",
            "x-xz",
            "zstd",
        ]
        .contains(&app)
    {
        return Category::Archives;
    }
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "tiff" | "tif" | "bmp" | "svg" | "raw" | "cr2"
        | "nef" => Category::Images,
        "mp4" | "mkv" | "mov" | "avi" | "webm" | "m4v" => Category::Video,
        "mp3" | "flac" | "wav" | "ogg" | "m4a" | "aac" | "opus" => Category::Audio,
        "pdf" | "doc" | "docx" | "odt" | "txt" | "md" | "rtf" | "xls" | "xlsx" | "ods" | "ppt" | "pptx"
        | "epub" => Category::Documents,
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" | "rar" | "zst" | "iso" => Category::Archives,
        "rs" | "py" | "js" | "ts" | "go" | "c" | "h" | "cpp" | "java" | "sh" | "json" | "toml" | "yaml"
        | "yml" | "html" | "css" => Category::Code,
        _ => Category::Other,
    }
}

/// One folder with everything below it added up.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DirNode {
    pub bytes: u64,
    pub files: u64,
    pub dirs: u64,
    pub by_type: BTreeMap<Category, u64>,
    /// Largest files below this folder, paths relative to it, largest first.
    pub largest: Vec<(String, u64)>,
    /// Files directly in this folder.
    pub loose_bytes: u64,
    pub loose_files: u64,
    /// Largest files directly in this folder, largest first.
    #[serde(default)]
    pub loose_largest: Vec<(String, u64)>,
    pub children: BTreeMap<String, DirNode>,
}

/// Keep `list` as the [`LARGEST_KEPT`] biggest entries; the path is only built when it gets in.
fn push_largest(list: &mut Vec<(String, u64)>, size: u64, path: impl FnOnce() -> String) {
    if list.len() < LARGEST_KEPT || list.last().is_some_and(|l| size > l.1) {
        let at = list.partition_point(|l| l.1 >= size);
        list.insert(at, (path(), size));
        list.truncate(LARGEST_KEPT);
    }
}

impl DirNode {
    fn add_file(&mut self, below: &str, name: &str, size: u64, cat: Category) {
        self.bytes += size;
        self.files += 1;
        *self.by_type.entry(cat).or_default() += size;
        push_largest(&mut self.largest, size, || {
            if below.is_empty() {
                name.to_string()
            } else {
                format!("{below}/{name}")
            }
        });
    }
}

/// The result of scanning one root, filled in as folders are listed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScanTree {
    pub root: Location,
    pub node: DirNode,
    /// Folders that could not be listed.
    pub unreadable: u64,
    /// Folders listed so far.
    pub listed: u64,
    pub scanned_at: jiff::Timestamp,
    pub duration_secs: u64,
    /// The scan was cancelled before it finished.
    pub partial: bool,
}

fn segments(rel: &str) -> impl Iterator<Item = &str> {
    rel.split('/').filter(|s| !s.is_empty())
}

impl ScanTree {
    pub fn new(root: Location) -> ScanTree {
        ScanTree {
            root,
            node: DirNode::default(),
            unreadable: 0,
            listed: 0,
            scanned_at: jiff::Timestamp::now(),
            duration_secs: 0,
            partial: false,
        }
    }

    pub fn node(&self, rel: &str) -> Option<&DirNode> {
        let mut n = &self.node;
        for s in segments(rel) {
            n = n.children.get(s)?;
        }
        Some(n)
    }

    /// The listing of folder `rel` (`""` is the root): its files count for it
    /// and every folder above it; its folders become children to be listed.
    pub fn add_listing(&mut self, rel: &str, entries: &[Entry]) {
        self.listed += 1;
        let path: Vec<&str> = segments(rel).collect();
        let new_dirs = entries.iter().filter(|e| e.is_dir).count() as u64;
        // Classified once, not once per ancestor.
        let files: Vec<(&str, u64, Category)> = entries
            .iter()
            .filter(|e| !e.is_dir)
            .map(|e| (e.name.as_str(), e.size.unwrap_or(0), category(&e.name, &e.mime)))
            .collect();
        // Walk down, adding to every ancestor (and to the folder itself last).
        let mut n = &mut self.node;
        for depth in 0..=path.len() {
            let below = path[depth..].join("/");
            n.dirs += new_dirs;
            for &(name, size, cat) in &files {
                n.add_file(&below, name, size, cat);
            }
            if depth == path.len() {
                for e in entries {
                    if e.is_dir {
                        n.children.entry(e.name.clone()).or_default();
                    } else {
                        let size = e.size.unwrap_or(0);
                        n.loose_bytes += size;
                        n.loose_files += 1;
                        push_largest(&mut n.loose_largest, size, || e.name.clone());
                    }
                }
                break;
            }
            n = n.children.entry(path[depth].to_string()).or_default();
        }
    }

    pub fn add_unreadable(&mut self, rel: &str) {
        self.unreadable += 1;
        let mut n = &mut self.node;
        for s in segments(rel) {
            n = n.children.entry(s.to_string()).or_default();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::Remote;

    fn f(name: &str, size: u64) -> Entry {
        Entry {
            name: name.into(),
            size: Some(size),
            mod_time: None,
            is_dir: false,
            mime: String::new(),
        }
    }

    fn d(name: &str) -> Entry {
        Entry {
            name: name.into(),
            size: None,
            mod_time: None,
            is_dir: true,
            mime: "inode/directory".into(),
        }
    }

    fn root() -> Location {
        Location::new(Remote::Named("g".into()), "")
    }

    #[test]
    fn rollup_sizes_counts_types_and_largest() {
        let mut t = ScanTree::new(root());
        t.add_listing("", &[d("Photos"), f("a.txt", 10)]);
        t.add_listing("Photos", &[d("2025"), f("x.jpg", 100)]);
        t.add_listing("Photos/2025", &[f("y.mp4", 1000)]);
        let r = t.node("").unwrap();
        assert_eq!((r.bytes, r.files, r.dirs), (1110, 3, 2));
        assert_eq!((r.loose_bytes, r.loose_files), (10, 1));
        assert_eq!(r.by_type[&Category::Video], 1000);
        assert_eq!(r.largest[0], ("Photos/2025/y.mp4".to_string(), 1000));
        let photos = t.node("Photos").unwrap();
        assert_eq!(photos.largest[0], ("2025/y.mp4".to_string(), 1000));
        assert_eq!(photos.bytes, 1100);
        assert_eq!(photos.dirs, 1);
        assert_eq!(t.listed, 3);
    }

    #[test]
    fn loose_largest_only_counts_files_directly_inside() {
        let mut t = ScanTree::new(root());
        t.add_listing("", &[d("Big"), f("small.txt", 5), f("mid.txt", 50)]);
        t.add_listing("Big", &[f("huge.bin", 9000)]);
        let r = t.node("").unwrap();
        assert_eq!(
            r.loose_largest,
            vec![("mid.txt".to_string(), 50), ("small.txt".to_string(), 5)]
        );
        assert_eq!(r.largest[0].0, "Big/huge.bin");
    }

    #[test]
    fn largest_is_capped() {
        let mut t = ScanTree::new(root());
        let files: Vec<Entry> = (0..25).map(|i| f(&format!("{i}.bin"), i)).collect();
        t.add_listing("", &files);
        let l = &t.node("").unwrap().largest;
        assert_eq!(l.len(), 10);
        assert_eq!(l[0].1, 24);
        assert_eq!(l[9].1, 15);
    }

    #[test]
    fn unreadable_is_counted() {
        let mut t = ScanTree::new(root());
        t.add_listing("", &[d("Secret")]);
        t.add_unreadable("Secret");
        assert_eq!(t.unreadable, 1);
        assert!(t.node("Secret").is_some());
        assert!(t.node("Nope").is_none());
    }

    #[test]
    fn categories() {
        assert_eq!(category("a.JPG", ""), Category::Images);
        assert_eq!(category("x", "video/mp4"), Category::Video);
        assert_eq!(category("r.pdf", ""), Category::Documents);
        assert_eq!(category("b.tar.gz", ""), Category::Archives);
        assert_eq!(category("m.rs", ""), Category::Code);
        assert_eq!(category("noext", ""), Category::Other);
        assert_eq!(category("noext", "application/octet-stream"), Category::Other);
    }
}
