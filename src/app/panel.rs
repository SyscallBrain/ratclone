//! The info panel at the bottom: when it shows, and the data only it needs.

use std::collections::VecDeque;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Rows of the expanded panel: one separator plus seven of content.
pub const PANEL_ROWS: u16 = 8;
/// Seconds of transfer speed kept for the sparkline.
pub const SPEED_SAMPLES: usize = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelMode {
    Expanded,
    Compact,
    Hidden,
}

/// How the panel fits a terminal of `width`×`height` (spec §6.3).
pub fn panel_mode(visible: bool, width: u16, height: u16) -> PanelMode {
    if !visible || height < 16 {
        PanelMode::Hidden
    } else if height < 24 || width < 70 {
        PanelMode::Compact
    } else {
        PanelMode::Expanded
    }
}

impl super::App {
    /// The account the info panel describes. In the sidebar and in Space, the one
    /// under the sidebar cursor; otherwise the focused pane's, so the panel matches
    /// the files on screen even after the cursor wandered in the sidebar.
    pub fn panel_account(&self) -> Option<&crate::backend::RemoteInfo> {
        if self.focus == super::Focus::Sidebar || self.tab == super::Tab::Space {
            return self.sidebar.current();
        }
        self.sidebar
            .remotes
            .iter()
            .find(|r| r.remote == self.pane.loc.remote)
            .or_else(|| self.sidebar.current())
    }
}

/// Total transfer speed, one sample per second.
#[derive(Clone, Debug, Default)]
pub struct SpeedHistory {
    samples: VecDeque<f64>,
}

impl SpeedHistory {
    pub fn push(&mut self, bytes_per_sec: f64) {
        if self.samples.len() == SPEED_SAMPLES {
            self.samples.pop_front();
        }
        self.samples.push_back(bytes_per_sec.max(0.0));
    }

    /// Oldest first, always [`SPEED_SAMPLES`] long (zeros before the first sample).
    pub fn samples(&self) -> Vec<f64> {
        let mut out = vec![0.0; SPEED_SAMPLES - self.samples.len()];
        out.extend(self.samples.iter().copied());
        out
    }

    pub fn latest(&self) -> f64 {
        self.samples.back().copied().unwrap_or(0.0)
    }
}

/// Small UI preferences remembered between runs (never in `config.toml`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiState {
    /// Written by older versions; read once to migrate to the `info_panel` setting.
    #[serde(skip_serializing)]
    pub panel_hidden: bool,
    /// Where the Files tab was when the app last quit.
    pub last_location: Option<crate::location::Location>,
}

impl UiState {
    /// A missing or unreadable file means defaults.
    pub fn load(path: &Path) -> UiState {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_follow_size_and_toggle() {
        assert_eq!(panel_mode(true, 110, 30), PanelMode::Expanded);
        assert_eq!(panel_mode(true, 110, 23), PanelMode::Compact);
        assert_eq!(panel_mode(true, 110, 15), PanelMode::Hidden);
        assert_eq!(panel_mode(true, 69, 40), PanelMode::Compact);
        assert_eq!(panel_mode(false, 110, 40), PanelMode::Hidden);
    }

    #[test]
    fn speed_history_keeps_last_60() {
        let mut h = SpeedHistory::default();
        for i in 0..70 {
            h.push(i as f64);
        }
        let s = h.samples();
        assert_eq!(s.len(), 60);
        assert_eq!(s[0], 10.0);
        assert_eq!(h.latest(), 69.0);
        assert_eq!(SpeedHistory::default().samples(), vec![0.0; 60]);
    }

    #[test]
    fn ui_state_roundtrip_and_corrupt_file() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("ui.json");
        assert!(!UiState::load(&p).panel_hidden);
        std::fs::write(&p, r#"{"panel_hidden":true}"#).unwrap();
        assert!(UiState::load(&p).panel_hidden);
        let loc = crate::location::Location::new(crate::location::Remote::Local, "home");
        UiState {
            panel_hidden: false,
            last_location: Some(loc.clone()),
        }
        .save(&p)
        .unwrap();
        assert_eq!(UiState::load(&p).last_location, Some(loc));
        std::fs::write(&p, "{not json").unwrap();
        assert!(!UiState::load(&p).panel_hidden);
    }
}
