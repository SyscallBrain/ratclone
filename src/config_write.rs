//! Saving settings to `config.toml` without disturbing the rest of the file.

use std::path::Path;

use toml_edit::{DocumentMut, value};

use crate::config::{DEFAULT_THEME, DefaultSort, IconStyle, OnQuit, SettingValues, SplitInfo};

/// Remove `key`, handing the comment lines above it to the next key (or the end
/// of the file), so removing a setting never deletes the user's notes.
fn remove_keeping_comments(doc: &mut DocumentMut, key: &str) {
    let keys: Vec<String> = doc.as_table().iter().map(|(k, _)| k.to_string()).collect();
    let Some(i) = keys.iter().position(|k| k == key) else {
        return;
    };
    let prefix = doc
        .as_table()
        .key(key)
        .and_then(|k| k.leaf_decor().prefix())
        .and_then(|p| p.as_str())
        .unwrap_or("")
        .to_string();
    doc.remove(key);
    if !prefix.contains('#') {
        return;
    }
    match keys.get(i + 1).and_then(|next| doc.as_table_mut().key_mut(next)) {
        Some(mut next) => {
            let old = next
                .leaf_decor()
                .prefix()
                .and_then(|p| p.as_str())
                .unwrap_or("")
                .to_string();
            next.leaf_decor_mut().set_prefix(format!("{prefix}{old}"));
        }
        None => {
            let old = doc.trailing().as_str().unwrap_or("").to_string();
            doc.set_trailing(format!("{prefix}{old}"));
        }
    }
}

/// Write the dialog's keys into `path`: comments, order and other keys are kept,
/// and a key at its default is removed.
pub fn save_settings(path: &Path, v: &SettingValues) -> std::io::Result<()> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("config.toml: {e}")))?;
    let mut set = |key: &str, item: Option<toml_edit::Item>| match item {
        Some(item) => {
            // Keep a trailing comment on an existing line.
            if let Some(old) = doc.get_mut(key).and_then(|i| i.as_value_mut()) {
                let decor = old.decor().clone();
                if let Ok(mut new) = item.into_value() {
                    *new.decor_mut() = decor;
                    *old = new;
                }
            } else {
                doc[key] = item;
            }
        }
        None => remove_keeping_comments(&mut doc, key),
    };
    set(
        "theme",
        (v.theme != DEFAULT_THEME).then(|| value(v.theme.as_str())),
    );
    set("transparent", v.transparent.map(value));
    set("icons", (v.icons != IconStyle::Nerd).then(|| value("ascii")));
    set("key_hints", (!v.key_hints).then(|| value(false)));
    set("info_panel", (!v.info_panel).then(|| value(false)));
    set(
        "info_panel_split",
        (v.split_info == SplitInfo::Hidden).then(|| value("hidden")),
    );
    set(
        "start_in",
        (v.start_in != "last").then(|| value(v.start_in.as_str())),
    );
    set(
        "sort",
        match v.sort {
            DefaultSort::Name => None,
            DefaultSort::Size => Some(value("size")),
            DefaultSort::Modified => Some(value("modified")),
        },
    );
    set("show_hidden", v.show_hidden.then(|| value(true)));
    set(
        "on_quit",
        match v.on_quit {
            OnQuit::Ask => None,
            OnQuit::Background => Some(value("background")),
            OnQuit::Wait => Some(value("wait")),
        },
    );
    set("bwlimit", (v.bwlimit != "off").then(|| value(v.bwlimit.as_str())));
    set("scan_concurrency", v.scan_concurrency.map(|n| value(n as i64)));
    // Write through a symlink (dotfile managers) and keep the file's permissions.
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = target.with_extension("toml.tmp");
    std::fs::write(&tmp, doc.to_string())?;
    if let Ok(meta) = std::fs::metadata(&target) {
        std::fs::set_permissions(&tmp, meta.permissions())?;
    }
    std::fs::rename(tmp, target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{IconStyle, SettingValues};

    fn defaults() -> SettingValues {
        SettingValues::from_config(&crate::config::Config::default())
    }

    #[test]
    fn save_preserves_comments_and_other_keys() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("config.toml");
        std::fs::write(
            &p,
            "# my comment\nrclone_path = \"/x\"\ntheme = \"nord\"  # pretty\n",
        )
        .unwrap();
        let mut v = defaults();
        v.theme = "dracula".into();
        save_settings(&p, &v).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("# my comment"), "{text}");
        assert!(text.contains("rclone_path = \"/x\""), "{text}");
        assert!(text.contains("theme = \"dracula\""), "{text}");
        assert!(!text.contains("key_hints"), "{text}");
    }

    #[test]
    fn symlinked_config_stays_a_symlink() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let real = d.path().join("dotfiles/config.toml");
        std::fs::create_dir_all(real.parent().unwrap()).unwrap();
        std::fs::write(&real, "# mine\n").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = d.path().join("config.toml");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let mut v = defaults();
        v.theme = "nord".into();
        save_settings(&link, &v).unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        let text = std::fs::read_to_string(&real).unwrap();
        assert!(
            text.contains("# mine") && text.contains("theme = \"nord\""),
            "{text}"
        );
        let mode = std::fs::metadata(&real).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn comment_above_a_removed_key_is_kept() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("config.toml");
        std::fs::write(&p, "# hints off, too busy\nkey_hints = false\n# my theme\ntheme = \"nord\"\n# last\nicons = \"ascii\"\n").unwrap();
        let mut v = defaults();
        v.theme = "nord".into();
        save_settings(&p, &v).unwrap(); // removes key_hints and icons
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("# hints off, too busy"), "{text}");
        assert!(text.contains("# my theme"), "{text}");
        assert!(text.contains("# last"), "{text}");
        assert!(!text.contains("key_hints") && !text.contains("icons"), "{text}");
    }

    #[test]
    fn defaults_are_removed() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("config.toml");
        std::fs::write(&p, "theme = \"nord\"\nkey_hints = false\n").unwrap();
        save_settings(&p, &defaults()).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(!text.contains("theme") && !text.contains("key_hints"), "{text}");
    }

    #[test]
    fn missing_file_is_created() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("sub/config.toml");
        let mut v = defaults();
        v.icons = IconStyle::Ascii;
        save_settings(&p, &v).unwrap();
        assert!(std::fs::read_to_string(&p).unwrap().contains("icons = \"ascii\""));
    }

    #[test]
    fn values_round_trip() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("config.toml");
        let v = SettingValues {
            theme: "nord".into(),
            transparent: Some(true),
            icons: IconStyle::Ascii,
            key_hints: false,
            info_panel: false,
            start_in: "personal".into(),
            sort: crate::config::DefaultSort::Size,
            show_hidden: true,
            on_quit: crate::config::OnQuit::Background,
            bwlimit: "10M".into(),
            scan_concurrency: Some(8),
            split_info: crate::config::SplitInfo::Hidden,
        };
        save_settings(&p, &v).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("info_panel_split = \"hidden\""), "{text}");
        let back = SettingValues::from_config(&crate::config::load(&p).unwrap());
        assert_eq!(back, v);
    }

    #[test]
    fn focused_split_info_writes_no_key() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("config.toml");
        std::fs::write(&p, "info_panel_split = \"hidden\"\n").unwrap();
        let mut v = SettingValues::from_config(&crate::config::load(&p).unwrap());
        assert_eq!(v.split_info, crate::config::SplitInfo::Hidden);
        v.split_info = crate::config::SplitInfo::Focused;
        save_settings(&p, &v).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(!text.contains("info_panel_split"), "{text}");
    }
}
