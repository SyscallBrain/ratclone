//! The settings dialog (`,`): every setting previews live, `Enter` saves it to
//! `config.toml`, `Esc` puts everything back.

use crossterm::event::{KeyCode, KeyEvent};

use super::overlay::{LineInput, Overlay};
use super::{App, MessageKind};
use crate::backend::RemoteInfo;
use crate::config::{DefaultSort, IconStyle, OnQuit, SettingValues, SplitInfo};
use crate::input::keymap::SortKey;
use crate::location::{Location, Remote};
use crate::theme::Theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingId {
    Theme,
    Transparent,
    Icons,
    KeyHints,
    InfoPanel,
    SplitInfo,
    StartIn,
    Sort,
    Hidden,
    OnQuit,
    Bwlimit,
    ScanParallel,
}

impl SettingId {
    pub const ALL: [SettingId; 12] = [
        SettingId::Theme,
        SettingId::Transparent,
        SettingId::Icons,
        SettingId::KeyHints,
        SettingId::InfoPanel,
        SettingId::SplitInfo,
        SettingId::StartIn,
        SettingId::Sort,
        SettingId::Hidden,
        SettingId::OnQuit,
        SettingId::Bwlimit,
        SettingId::ScanParallel,
    ];

    pub fn section(self) -> &'static str {
        use SettingId::*;
        match self {
            Theme | Transparent | Icons | KeyHints | InfoPanel | SplitInfo => "APPEARANCE",
            StartIn | Sort | Hidden | OnQuit => "BEHAVIOUR",
            Bwlimit | ScanParallel => "TRANSFERS & SPACE",
        }
    }

    pub fn label(self) -> &'static str {
        use SettingId::*;
        match self {
            Theme => "Theme",
            Transparent => "Transparent background",
            Icons => "Icons",
            KeyHints => "Key hints",
            InfoPanel => "Info panel",
            SplitInfo => "Info panel with two panes",
            StartIn => "Start in",
            Sort => "Default sort",
            Hidden => "Hidden files",
            OnQuit => "On quit with transfers",
            Bwlimit => "Bandwidth limit",
            ScanParallel => "Scan in parallel",
        }
    }

    pub fn description(self) -> &'static str {
        use SettingId::*;
        match self {
            Theme => "Colours of the whole app. ← → to try them live.",
            Transparent => "Let the terminal's own background show through.",
            Icons => "Nerd Font icons need a patched font; ASCII works everywhere.",
            KeyHints => "The list of keys on the bottom line. ? always shows them all.",
            InfoPanel => "Storage, activity and selection at the bottom. Also i.",
            SplitInfo => "With two panes open: about the focused one, or hidden for more rows.",
            StartIn => "Where the Files tab opens when the app starts.",
            Sort => "How folders are sorted when the app starts.",
            Hidden => "Show files whose names start with a dot.",
            OnQuit => "What q does while copies or moves are still running.",
            Bwlimit => "Cap on transfer speed, applied by rclone when you save.",
            ScanParallel => "Folders listed at once by a Space scan. Auto: 4, or 2 on Mega.",
        }
    }
}

/// A theme the dialog can offer.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeChoice {
    pub name: String,
    /// A custom theme file that does not load.
    pub broken: bool,
}

/// What `Esc` restores.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionSnapshot {
    pub theme: Theme,
    pub icons: IconStyle,
    pub key_hints: bool,
    pub panel_visible: bool,
    pub show_hidden: bool,
    pub sort: SortKey,
    /// The second pane's own `show_hidden` and `sort`, when two are open.
    pub other_view: Option<(bool, SortKey)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SettingsDialog {
    pub cursor: usize,
    /// What is in `config.toml`.
    pub saved: SettingValues,
    /// What the dialog shows (and the app previews).
    pub values: SettingValues,
    pub before: SessionSnapshot,
    /// The custom bandwidth input, when open.
    pub input: Option<LineInput>,
    pub themes: Vec<ThemeChoice>,
    pending_g: bool,
}

pub const BWLIMITS: [&str; 6] = ["off", "1M", "5M", "10M", "50M", "custom…"];
pub const SCAN_PARALLEL: [Option<usize>; 6] = [None, Some(1), Some(2), Some(4), Some(8), Some(16)];

fn title_case(name: &str) -> String {
    name.split('-')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().chain(c).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn remote_names(remotes: &[RemoteInfo]) -> Vec<String> {
    remotes
        .iter()
        .filter_map(|r| match &r.remote {
            Remote::Named(n) => Some(n.clone()),
            Remote::Local => None,
        })
        .collect()
}

/// The value of `id` as the dialog shows it.
pub fn display_value(
    id: SettingId,
    v: &SettingValues,
    themes: &[ThemeChoice],
    _remotes: &[RemoteInfo],
) -> String {
    match id {
        SettingId::Theme => {
            let broken = themes.iter().any(|t| t.name == v.theme && t.broken);
            let name = title_case(&v.theme);
            if broken { format!("{name} (broken)") } else { name }
        }
        SettingId::Transparent => match v.transparent {
            None => "theme default".into(),
            Some(true) => "on".into(),
            Some(false) => "off".into(),
        },
        SettingId::Icons => match v.icons {
            IconStyle::Nerd => "Nerd Font".into(),
            IconStyle::Ascii => "ASCII".into(),
        },
        SettingId::KeyHints => if v.key_hints { "shown" } else { "hidden" }.into(),
        SettingId::InfoPanel => if v.info_panel { "shown" } else { "hidden" }.into(),
        SettingId::SplitInfo => match v.split_info {
            SplitInfo::Focused => "focused pane".into(),
            SplitInfo::Hidden => "hidden".into(),
        },
        SettingId::StartIn => match v.start_in.as_str() {
            "last" => "last location".into(),
            "home" => "home folder".into(),
            other => format!("{other}:/"),
        },
        SettingId::Sort => match v.sort {
            DefaultSort::Name => "name".into(),
            DefaultSort::Size => "size".into(),
            DefaultSort::Modified => "modified".into(),
        },
        SettingId::Hidden => if v.show_hidden { "on" } else { "off" }.into(),
        SettingId::OnQuit => match v.on_quit {
            OnQuit::Ask => "ask".into(),
            OnQuit::Background => "keep running".into(),
            OnQuit::Wait => "wait".into(),
        },
        SettingId::Bwlimit => v.bwlimit.clone(),
        SettingId::ScanParallel => match v.scan_concurrency {
            None => "auto (4, Mega 2)".into(),
            Some(n) => n.to_string(),
        },
    }
}

/// rclone's bandwidth syntax: `off`, or `RATE[:RATE]` with RATE like `10M`, `750k`, `2.5M`.
pub fn valid_bwlimit(s: &str) -> bool {
    fn rate(r: &str) -> bool {
        let num = r.trim_end_matches(['b', 'B', 'k', 'K', 'M', 'G', 'T', 'P']);
        let unit_len = r.len() - num.len();
        unit_len <= 1 && !num.is_empty() && num.parse::<f64>().is_ok_and(|v| v >= 0.0)
    }
    if s == "off" {
        return true;
    }
    let mut parts = s.split(':');
    let up = parts.next().unwrap_or("");
    let down = parts.next();
    parts.next().is_none() && rate(up) && down.is_none_or(rate)
}

/// Where to open at start-up; `None` means the home folder.
pub fn resolve_start(start_in: &str, last: Option<&Location>, remotes: &[String]) -> Option<Location> {
    let known = |r: &Remote| match r {
        Remote::Local => true,
        Remote::Named(n) => remotes.contains(n),
    };
    match start_in {
        "home" => None,
        "last" => last.filter(|l| known(&l.remote)).cloned(),
        name => {
            let remote = Remote::Named(name.to_string());
            known(&remote).then(|| Location::new(remote, ""))
        }
    }
}

/// The next (or previous) item of `list` after `current`, wrapping; the first if absent.
fn step<T: Clone + PartialEq>(list: &[T], current: &T, forward: bool) -> T {
    let n = list.len();
    let i = list.iter().position(|x| x == current);
    let j = match (i, forward) {
        (None, _) => 0,
        (Some(i), true) => (i + 1) % n,
        (Some(i), false) => (i + n - 1) % n,
    };
    list[j].clone()
}

pub fn sort_key(s: DefaultSort) -> SortKey {
    match s {
        DefaultSort::Name => SortKey::Name,
        DefaultSort::Size => SortKey::Size,
        DefaultSort::Modified => SortKey::Modified,
    }
}

impl App {
    /// The theme `name` (built-in, else a custom file), with the transparency override.
    pub fn resolve_theme(&self, name: &str, transparent: Option<bool>) -> Option<Theme> {
        let builtin = crate::theme::builtin::all().into_iter().find(|t| t.name == name);
        let mut theme = match builtin {
            Some(t) => t,
            None => crate::theme::load::load_custom_from(&self.themes_dir, name).ok()?,
        };
        if let Some(t) = transparent {
            theme.transparent = t;
        }
        Some(theme)
    }

    pub fn theme_choices(&self) -> Vec<ThemeChoice> {
        let mut out: Vec<ThemeChoice> = crate::theme::builtin::all()
            .into_iter()
            .map(|t| ThemeChoice {
                name: t.name.to_string(),
                broken: false,
            })
            .collect();
        for name in crate::theme::load::custom_theme_names(&self.themes_dir) {
            if out.iter().any(|t| t.name == name) {
                continue;
            }
            let broken = crate::theme::load::load_custom_from(&self.themes_dir, &name).is_err();
            out.push(ThemeChoice { name, broken });
        }
        out
    }

    pub fn open_settings(&mut self) {
        let before = SessionSnapshot {
            theme: self.theme,
            icons: self.icons,
            key_hints: self.key_hints,
            panel_visible: self.panel_visible,
            show_hidden: self.pane.show_hidden,
            sort: self.pane.sort,
            other_view: self.other.as_ref().map(|o| (o.show_hidden, o.sort)),
        };
        self.overlay = Some(Overlay::Settings(Box::new(SettingsDialog {
            cursor: 0,
            saved: self.settings.clone(),
            values: self.settings.clone(),
            before,
            input: None,
            themes: self.theme_choices(),
            pending_g: false,
        })));
    }

    /// Show `v` now (everything but the bandwidth limit, which waits for `Enter`).
    pub fn apply_settings(&mut self, v: &SettingValues) {
        if let Some(theme) = self.resolve_theme(&v.theme, v.transparent) {
            self.theme = theme;
        }
        self.icons = v.icons;
        self.key_hints = v.key_hints;
        self.panel_visible = v.info_panel;
        for pane in self.panes_mut() {
            set_hidden(pane, v.show_hidden);
            set_sort(pane, sort_key(v.sort));
        }
        self.on_quit = v.on_quit;
        self.scan_concurrency = v.scan_concurrency;
    }

    fn restore_session(&mut self, s: &SessionSnapshot) {
        self.theme = s.theme;
        self.icons = s.icons;
        self.key_hints = s.key_hints;
        self.panel_visible = s.panel_visible;
        set_hidden(&mut self.pane, s.show_hidden);
        set_sort(&mut self.pane, s.sort);
        if let (Some(other), Some((hidden, sort))) = (self.other.as_mut(), s.other_view) {
            set_hidden(other, hidden);
            set_sort(other, sort);
        }
        self.on_quit = self.settings.on_quit;
        self.scan_concurrency = self.settings.scan_concurrency;
    }

    /// Write `values` to `config.toml`; on failure they stay applied for this session.
    pub fn save_settings(&mut self, mut values: SettingValues) {
        // A theme that does not load is never written: the next start would quietly drop it.
        let broken = self
            .theme_choices()
            .into_iter()
            .any(|t| t.broken && t.name == values.theme);
        let broken_note = broken.then(|| {
            let name = std::mem::replace(&mut values.theme, self.settings.theme.clone());
            if let Some(theme) = self.resolve_theme(&values.theme, values.transparent) {
                self.theme = theme;
            }
            format!(" — theme “{name}” does not load, kept “{}”", values.theme)
        });
        let bw_changed = values.bwlimit != self.settings.bwlimit;
        let result = match &self.config_path {
            Some(path) => crate::config_write::save_settings(path, &values),
            None => Ok(()),
        };
        if bw_changed {
            self.send_bwlimit(values.bwlimit.clone());
        }
        let note = broken_note.unwrap_or_default();
        let saved_to_disk = self.config_path.is_some();
        self.settings = values;
        match result {
            Ok(()) if !saved_to_disk => self.info(
                format!("Settings applied (demo: not saved){note}"),
                MessageKind::Info,
            ),
            Ok(()) if broken => self.info(format!("Settings saved{note}"), MessageKind::Error),
            Ok(()) => self.info("Settings saved", MessageKind::Success),
            Err(e) => {
                // Parse errors span lines with a snippet; the message bar has one.
                let e = e.to_string();
                let first = e.lines().next().unwrap_or_default().trim_end_matches(':');
                self.info(
                    format!("Settings applied but not saved: {first}{note}"),
                    MessageKind::Error,
                );
            }
        }
    }

    /// Ask rclone for its current limit (it may have one the config does not set).
    pub fn query_bwlimit(&mut self) {
        let backend = self.backend.clone();
        self.spawn(async move { super::AppEvent::BwLimit(backend.get_bwlimit().await) });
    }

    /// Send the bandwidth limit to rclone; the answer arrives as `AppEvent::BwLimit`.
    pub fn send_bwlimit(&mut self, rate: String) {
        let backend = self.backend.clone();
        self.spawn(async move {
            let result = backend.set_bwlimit(&rate).await;
            super::AppEvent::BwLimit(result)
        });
    }

    pub(super) fn settings_key(&mut self, key: KeyEvent) {
        let Some(Overlay::Settings(dlg)) = self.overlay.as_mut() else {
            return;
        };
        let last = SettingId::ALL.len() - 1;
        if let Some(input) = dlg.input.as_mut() {
            match input.handle(key) {
                super::overlay::InputOutcome::Submit => {
                    let value = input.value.trim().to_string();
                    if valid_bwlimit(&value) {
                        dlg.values.bwlimit = value;
                        dlg.input = None;
                    } else {
                        self.info(
                            format!("“{value}” is an invalid bandwidth limit (e.g. 10M, 750k, 10M:1M)"),
                            MessageKind::Error,
                        );
                    }
                }
                super::overlay::InputOutcome::Cancel => dlg.input = None,
                _ => {}
            }
            return;
        }
        let g = std::mem::take(&mut dlg.pending_g);
        let id = SettingId::ALL[dlg.cursor];
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => dlg.cursor = (dlg.cursor + 1).min(last),
            KeyCode::Char('k') | KeyCode::Up => dlg.cursor = dlg.cursor.saturating_sub(1),
            KeyCode::Char('G') => dlg.cursor = last,
            KeyCode::Char('g') if g => dlg.cursor = 0,
            KeyCode::Char('g') => dlg.pending_g = true,
            KeyCode::Char('l') | KeyCode::Right | KeyCode::Char(' ') => self.cycle_setting(id, true),
            KeyCode::Char('h') | KeyCode::Left => self.cycle_setting(id, false),
            KeyCode::Enter if id == SettingId::Bwlimit && dlg.values.bwlimit == "custom…" => {
                dlg.input = Some(LineInput::new("", 0));
            }
            KeyCode::Enter => {
                let mut values = dlg.values.clone();
                if values.bwlimit == "custom…" {
                    values.bwlimit = dlg.saved.bwlimit.clone();
                }
                self.overlay = None;
                self.save_settings(values);
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                let before = dlg.before.clone();
                self.overlay = None;
                self.restore_session(&before);
            }
            _ => {}
        }
    }

    fn cycle_setting(&mut self, id: SettingId, forward: bool) {
        let remotes = remote_names(&self.sidebar.remotes);
        let Some(Overlay::Settings(dlg)) = self.overlay.as_mut() else {
            return;
        };
        let v = &mut dlg.values;
        match id {
            SettingId::Theme => {
                let names: Vec<String> = dlg.themes.iter().map(|t| t.name.clone()).collect();
                v.theme = step(&names, &v.theme, forward);
            }
            SettingId::Transparent => {
                v.transparent = step(&[None, Some(true), Some(false)], &v.transparent, forward)
            }
            SettingId::Icons => v.icons = step(&[IconStyle::Nerd, IconStyle::Ascii], &v.icons, forward),
            SettingId::KeyHints => v.key_hints = !v.key_hints,
            SettingId::InfoPanel => v.info_panel = !v.info_panel,
            SettingId::SplitInfo => {
                v.split_info = step(&[SplitInfo::Focused, SplitInfo::Hidden], &v.split_info, forward)
            }
            SettingId::StartIn => {
                let mut opts = vec!["last".to_string(), "home".to_string()];
                opts.extend(remotes);
                v.start_in = step(&opts, &v.start_in, forward);
            }
            SettingId::Sort => {
                v.sort = step(
                    &[DefaultSort::Name, DefaultSort::Size, DefaultSort::Modified],
                    &v.sort,
                    forward,
                )
            }
            SettingId::Hidden => v.show_hidden = !v.show_hidden,
            SettingId::OnQuit => {
                v.on_quit = step(
                    &[OnQuit::Ask, OnQuit::Background, OnQuit::Wait],
                    &v.on_quit,
                    forward,
                )
            }
            SettingId::Bwlimit => {
                let opts: Vec<String> = BWLIMITS.iter().map(|s| s.to_string()).collect();
                // A custom value sits where "custom…" is.
                let current = if opts.contains(&v.bwlimit) {
                    v.bwlimit.clone()
                } else {
                    "custom…".into()
                };
                v.bwlimit = step(&opts, &current, forward);
            }
            SettingId::ScanParallel => {
                v.scan_concurrency = step(&SCAN_PARALLEL, &v.scan_concurrency, forward)
            }
        }
        let values = dlg.values.clone();
        self.apply_one(id, &values);
    }

    /// Show the change to one setting only: the session view (sort, dotfiles,
    /// panel) set with the quick keys stays as it is for the others.
    fn apply_one(&mut self, id: SettingId, v: &SettingValues) {
        match id {
            SettingId::Theme | SettingId::Transparent => {
                if let Some(theme) = self.resolve_theme(&v.theme, v.transparent) {
                    self.theme = theme;
                }
            }
            SettingId::Icons => self.icons = v.icons,
            SettingId::KeyHints => self.key_hints = v.key_hints,
            SettingId::InfoPanel => self.panel_visible = v.info_panel,
            SettingId::Hidden => {
                for pane in self.panes_mut() {
                    set_hidden(pane, v.show_hidden);
                }
            }
            SettingId::Sort => {
                for pane in self.panes_mut() {
                    pane.set_sort(sort_key(v.sort));
                }
            }
            SettingId::OnQuit => self.on_quit = v.on_quit,
            // Takes effect the next time the second pane opens.
            SettingId::SplitInfo => {}
            SettingId::ScanParallel => self.scan_concurrency = v.scan_concurrency,
            // Applied by rclone on save; start-in matters at the next start.
            SettingId::Bwlimit | SettingId::StartIn => {}
        }
    }
}

fn set_hidden(pane: &mut super::pane::PaneState, on: bool) {
    if pane.show_hidden != on {
        pane.toggle_hidden();
    }
}

fn set_sort(pane: &mut super::pane::PaneState, key: SortKey) {
    if pane.sort != key {
        pane.set_sort(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bandwidth_syntax() {
        for ok in ["off", "10M", "750k", "2.5M", "10M:1M", "0", "1G"] {
            assert!(valid_bwlimit(ok), "{ok}");
        }
        for bad in ["fast", "", "10MM", "1:2:3", "-1M", "M"] {
            assert!(!valid_bwlimit(bad), "{bad}");
        }
    }
}
