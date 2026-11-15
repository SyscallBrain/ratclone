//! Planning a paste: refusing impossible ones, finding name clashes, new names.

use std::collections::{HashMap, HashSet};

use super::register::Register;
use crate::backend::Entry;
use crate::location::Location;
use crate::transfer::spec::{TransferItem, TransferKind};

#[derive(Clone, Debug, PartialEq)]
pub enum PasteError {
    IntoItself(String),
    SameFolderMove,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    Replace,
    Skip,
    Rename,
}

/// Extensions kept together when making a new name.
const DOUBLE_EXTENSIONS: [&str; 4] = [".tar.gz", ".tar.xz", ".tar.bz2", ".tar.zst"];

/// Refuse pastes that cannot work: a folder into itself, or a move onto its own folder.
pub fn check_paste(reg: &Register, dst_dir: &Location) -> Result<(), PasteError> {
    for item in reg.items.iter().filter(|e| e.is_dir) {
        if dst_dir.is_within(&reg.src_dir.join(&item.name)) {
            return Err(PasteError::IntoItself(item.name.clone()));
        }
    }
    if reg.kind == TransferKind::Move && &reg.src_dir == dst_dir {
        return Err(PasteError::SameFolderMove);
    }
    Ok(())
}

/// Names (in `items` order) that already exist at the top of the destination.
/// Compared without case: Dropbox, OneDrive and others treat `Photo.jpg` and `photo.jpg` as one name.
pub fn conflicts(items: &[Entry], dst_entries: &[Entry]) -> Vec<String> {
    let existing: HashSet<String> = dst_entries.iter().map(|e| e.name.to_lowercase()).collect();
    items
        .iter()
        .filter(|e| existing.contains(&e.name.to_lowercase()))
        .map(|e| e.name.clone())
        .collect()
}

/// A file and a folder with the same name: one can't replace the other.
pub fn kind_clash(item: &Entry, dst_entries: &[Entry]) -> bool {
    let name = item.name.to_lowercase();
    dst_entries
        .iter()
        .any(|d| d.name.to_lowercase() == name && d.is_dir != item.is_dir)
}

/// `photo.jpg` → `photo (1).jpg`, `Docs` → `Docs (1)`, skipping names in `taken` (any case).
pub fn unique_name(name: &str, is_dir: bool, taken: &HashSet<String>) -> String {
    let taken: HashSet<String> = taken.iter().map(|t| t.to_lowercase()).collect();
    unique_name_lower(name, is_dir, &taken)
}

/// [`unique_name`] with `taken` already lower-cased.
pub fn unique_name_lower(name: &str, is_dir: bool, taken: &HashSet<String>) -> String {
    if !taken.contains(&name.to_lowercase()) {
        return name.to_string();
    }
    let split = if is_dir {
        None
    } else {
        DOUBLE_EXTENSIONS
            .iter()
            .find(|ext| name.len() > ext.len() && name.ends_with(*ext))
            .map(|ext| name.len() - ext.len())
            .or_else(|| name.rfind('.').filter(|&i| i > 0))
    };
    let (stem, ext) = match split {
        Some(i) => name.split_at(i),
        None => (name, ""),
    };
    (1..)
        .map(|n| format!("{stem} ({n}){ext}"))
        .find(|candidate| !taken.contains(&candidate.to_lowercase()))
        .unwrap()
}

/// Turn the conflict choices into transfer items; renamed items never collide with each
/// other, and "replace" across a file/folder clash becomes "keep both".
pub fn build_items(
    items: &[Entry],
    chosen: &HashMap<String, Resolution>,
    dst_entries: &[Entry],
) -> Vec<TransferItem> {
    // Lower-cased once: per item, both lookups are then constant time.
    let mut taken: HashSet<String> = HashSet::with_capacity(dst_entries.len() + items.len());
    // Lower-cased name → (a file has it, a folder has it).
    let mut kinds: HashMap<String, (bool, bool)> = HashMap::with_capacity(dst_entries.len());
    for d in dst_entries {
        let lower = d.name.to_lowercase();
        let k = kinds.entry(lower.clone()).or_default();
        if d.is_dir {
            k.1 = true;
        } else {
            k.0 = true;
        }
        taken.insert(lower);
    }
    let clash = |e: &Entry| {
        kinds
            .get(&e.name.to_lowercase())
            .is_some_and(|&(file, dir)| if e.is_dir { file } else { dir })
    };
    // First the items that keep their name reserve it, so a later "keep both" can't pick
    // it (`x.txt` renamed to `x (1).txt` while `x (1).txt` is pasted as is). Two kept
    // items with one name (any case) can't share it either: the later one is renamed.
    let mut plan: Vec<Option<(&Entry, bool, bool)>> = Vec::with_capacity(items.len());
    let mut kept: HashSet<String> = HashSet::with_capacity(items.len());
    for e in items {
        let choice = chosen.get(&e.name);
        let keep = match choice {
            Some(Resolution::Skip) => {
                plan.push(None);
                continue;
            }
            Some(Resolution::Replace) => !clash(e),
            Some(Resolution::Rename) => false,
            None => true,
        };
        let keep = keep && kept.insert(e.name.to_lowercase());
        let replace = keep && choice == Some(&Resolution::Replace);
        plan.push(Some((e, keep, replace)));
    }
    taken.extend(kept);
    let mut out = Vec::with_capacity(items.len());
    for (e, keep, replace) in plan.into_iter().flatten() {
        let dst_name = if keep {
            e.name.clone()
        } else {
            let name = unique_name_lower(&e.name, e.is_dir, &taken);
            taken.insert(name.to_lowercase());
            name
        };
        out.push(TransferItem {
            replace,
            name: e.name.clone(),
            is_dir: e.is_dir,
            dst_name,
            size: e.size,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::Remote;

    fn e(name: &str, is_dir: bool) -> Entry {
        Entry {
            name: name.into(),
            size: None,
            mod_time: None,
            is_dir,
            mime: String::new(),
        }
    }
    fn g(p: &str) -> Location {
        Location::new(Remote::Named("g".into()), p)
    }
    fn taken(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn unique_names() {
        assert_eq!(
            unique_name("photo.jpg", false, &taken(&["photo.jpg"])),
            "photo (1).jpg"
        );
        assert_eq!(
            unique_name("photo.jpg", false, &taken(&["photo.jpg", "photo (1).jpg"])),
            "photo (2).jpg"
        );
        assert_eq!(unique_name(".bashrc", false, &taken(&[".bashrc"])), ".bashrc (1)");
        assert_eq!(unique_name("Docs", true, &taken(&["Docs"])), "Docs (1)");
        assert_eq!(unique_name("v1.2", true, &taken(&["v1.2"])), "v1.2 (1)");
        assert_eq!(
            unique_name("archive.tar.gz", false, &taken(&["archive.tar.gz"])),
            "archive (1).tar.gz"
        );
        assert_eq!(unique_name("a.b.c", false, &taken(&["a.b.c"])), "a.b (1).c");
        assert_eq!(unique_name("free.txt", false, &taken(&[])), "free.txt");
    }

    #[test]
    fn refuses_into_itself_and_same_folder_move() {
        let reg = |kind| Register::new(kind, g(""), vec![e("Photos", true), e("a.txt", false)]);
        assert_eq!(
            check_paste(&reg(TransferKind::Copy), &g("Photos/2025")),
            Err(PasteError::IntoItself("Photos".into()))
        );
        assert_eq!(
            check_paste(&reg(TransferKind::Copy), &g("Photos")),
            Err(PasteError::IntoItself("Photos".into()))
        );
        assert_eq!(
            check_paste(&reg(TransferKind::Move), &g("")),
            Err(PasteError::SameFolderMove)
        );
        assert_eq!(check_paste(&reg(TransferKind::Copy), &g("")), Ok(()));
        assert_eq!(check_paste(&reg(TransferKind::Copy), &g("Photos2")), Ok(()));
        let other = Location::new(Remote::Named("t".into()), "Photos");
        assert_eq!(check_paste(&reg(TransferKind::Move), &other), Ok(()));
    }

    #[test]
    fn finds_top_level_conflicts() {
        let items = [e("a.txt", false), e("Docs", true), e("new", false)];
        let dst = [e("Docs", true), e("a.txt", false), e("other", false)];
        assert_eq!(conflicts(&items, &dst), ["a.txt", "Docs"]);
    }

    #[test]
    fn builds_items_from_resolutions() {
        let items = [
            e("a.txt", false),
            e("Docs", true),
            e("b.txt", false),
            e("c", false),
        ];
        let chosen = HashMap::from([
            ("a.txt".to_string(), Resolution::Replace),
            ("Docs".to_string(), Resolution::Rename),
            ("b.txt".to_string(), Resolution::Skip),
        ]);
        let dst = [e("a.txt", false), e("Docs", true), e("b.txt", false)];
        let built = build_items(&items, &chosen, &dst);
        let got: Vec<(&str, bool, &str)> = built
            .iter()
            .map(|i| (i.name.as_str(), i.is_dir, i.dst_name.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("a.txt", false, "a.txt"),
                ("Docs", true, "Docs (1)"),
                ("c", false, "c")
            ]
        );
        let replaced: Vec<bool> = built.iter().map(|i| i.replace).collect();
        assert_eq!(
            replaced,
            [true, false, false],
            "only the replaced item overwrites"
        );
    }

    #[test]
    fn names_clash_regardless_of_case() {
        assert_eq!(
            conflicts(&[e("Photo.JPG", false)], &[e("photo.jpg", false)]),
            ["Photo.JPG"]
        );
        assert_eq!(
            unique_name("Photo.jpg", false, &taken(&["photo.jpg", "PHOTO (1).jpg"])),
            "Photo (2).jpg"
        );
    }

    #[test]
    fn a_file_never_replaces_a_folder() {
        let items = [e("Docs", false)];
        let chosen = HashMap::from([("Docs".to_string(), Resolution::Replace)]);
        let built = build_items(&items, &chosen, &[e("Docs", true)]);
        assert_eq!(built[0].dst_name, "Docs (1)");
        assert!(!built[0].replace, "kept both: nothing is overwritten");
        assert!(kind_clash(&e("Docs", false), &[e("docs", true)]));
        assert!(!kind_clash(&e("Docs", true), &[e("Docs", true)]));
    }

    #[test]
    fn renames_do_not_collide_with_each_other() {
        let items = [e("x.txt", false), e("x (1).txt", false)];
        let chosen = HashMap::from([
            ("x.txt".to_string(), Resolution::Rename),
            ("x (1).txt".to_string(), Resolution::Rename),
        ]);
        let built = build_items(&items, &chosen, &[e("x.txt", false), e("x (1).txt", false)]);
        let names: Vec<&str> = built.iter().map(|i| i.dst_name.as_str()).collect();
        assert_eq!(names, ["x (2).txt", "x (1) (1).txt"]);
    }

    fn dst_names(built: &[TransferItem]) -> Vec<&str> {
        built.iter().map(|i| i.dst_name.as_str()).collect()
    }

    #[test]
    fn a_rename_never_takes_the_name_of_an_item_kept_as_is() {
        let dst = [e("x.txt", false)];
        let chosen = HashMap::from([("x.txt".to_string(), Resolution::Rename)]);
        // Both orders: the kept item may come before or after the renamed one.
        let built = build_items(&[e("x.txt", false), e("x (1).txt", false)], &chosen, &dst);
        assert_eq!(dst_names(&built), ["x (2).txt", "x (1).txt"]);
        let built = build_items(&[e("x (1).txt", false), e("x.txt", false)], &chosen, &dst);
        assert_eq!(dst_names(&built), ["x (1).txt", "x (2).txt"]);
        // A replaced item keeps its name too.
        let dst = [e("x.txt", false), e("x (1).txt", false)];
        let chosen = HashMap::from([
            ("x.txt".to_string(), Resolution::Rename),
            ("x (1).txt".to_string(), Resolution::Replace),
        ]);
        let built = build_items(&[e("x.txt", false), e("x (1).txt", false)], &chosen, &dst);
        assert_eq!(dst_names(&built), ["x (2).txt", "x (1).txt"]);
        // A skipped item reserves nothing.
        let dst = [e("x.txt", false)];
        let chosen = HashMap::from([
            ("x.txt".to_string(), Resolution::Rename),
            ("x (1).txt".to_string(), Resolution::Skip),
        ]);
        let built = build_items(&[e("x.txt", false), e("x (1).txt", false)], &chosen, &dst);
        assert_eq!(dst_names(&built), ["x (1).txt"]);
    }

    #[test]
    fn two_kept_items_never_share_a_destination() {
        let built = build_items(&[e("A.txt", false), e("a.txt", false)], &HashMap::new(), &[]);
        assert_eq!(dst_names(&built), ["A.txt", "a (1).txt"]);
    }
}
