//! User configuration (`~/.config/ratclone/config.toml`).

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DefaultSort {
    #[default]
    Name,
    Size,
    Modified,
}

/// What `q` does while transfers are running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OnQuit {
    #[default]
    Ask,
    Background,
    Wait,
}

/// What the info panel shows while two file panes are open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SplitInfo {
    /// The focused pane's selection.
    #[default]
    Focused,
    /// Nothing: the panel is hidden to give the panes more rows.
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IconStyle {
    #[default]
    Nerd,
    Ascii,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub theme: Option<String>,
    pub icons: Option<IconStyle>,
    pub show_hidden: bool,
    pub rclone_path: Option<PathBuf>,
    /// Folders listed at once by a Space scan.
    pub scan_concurrency: Option<usize>,
    /// Override the theme's own transparency.
    pub transparent: Option<bool>,
    /// Show the key hints on the bottom line.
    pub key_hints: Option<bool>,
    /// Show the info panel at start-up.
    pub info_panel: Option<bool>,
    /// What the info panel shows while two panes are open.
    pub info_panel_split: Option<SplitInfo>,
    /// `last`, `home` or a remote name.
    pub start_in: Option<String>,
    pub sort: Option<DefaultSort>,
    pub on_quit: Option<OnQuit>,
    /// rclone bandwidth syntax (`10M`, `750k`, `off`).
    pub bwlimit: Option<String>,
}

/// Every setting the settings dialog edits, resolved to a value.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingValues {
    pub theme: String,
    pub transparent: Option<bool>,
    pub icons: IconStyle,
    pub key_hints: bool,
    pub info_panel: bool,
    pub start_in: String,
    pub sort: DefaultSort,
    pub show_hidden: bool,
    pub on_quit: OnQuit,
    /// `"off"` when there is no limit.
    pub bwlimit: String,
    /// `None` = automatic.
    pub scan_concurrency: Option<usize>,
    pub split_info: SplitInfo,
}

pub const DEFAULT_THEME: &str = "tokyo-night";

impl SettingValues {
    pub fn from_config(c: &Config) -> SettingValues {
        SettingValues {
            theme: c.theme.clone().unwrap_or_else(|| DEFAULT_THEME.into()),
            transparent: c.transparent,
            icons: c.icons.unwrap_or_default(),
            key_hints: c.key_hints.unwrap_or(true),
            info_panel: c.info_panel.unwrap_or(true),
            start_in: c.start_in.clone().unwrap_or_else(|| "last".into()),
            sort: c.sort.unwrap_or_default(),
            show_hidden: c.show_hidden,
            on_quit: c.on_quit.unwrap_or_default(),
            bwlimit: c.bwlimit.clone().unwrap_or_else(|| "off".into()),
            scan_concurrency: c.scan_concurrency,
            split_info: c.info_panel_split.unwrap_or_default(),
        }
    }
}

/// A missing file means defaults; a malformed one is an error to show the user.
pub fn load(path: &Path) -> color_eyre::Result<Config> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(toml::from_str(&text)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_defaults() {
        let d = tempfile::tempdir().unwrap();
        assert!(load(&d.path().join("none.toml")).unwrap().theme.is_none());
        let p = d.path().join("c.toml");
        std::fs::write(&p, "theme = \"nord\"\nicons = \"ascii\"\nshow_hidden = true\n").unwrap();
        let c = load(&p).unwrap();
        assert_eq!(c.theme.as_deref(), Some("nord"));
        assert_eq!(c.icons, Some(IconStyle::Ascii));
        assert!(c.show_hidden);
        std::fs::write(&p, "colour = 1\n").unwrap();
        assert!(load(&p).is_err());
    }
}
