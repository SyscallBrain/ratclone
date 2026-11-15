//! Rendering. Layout, top to bottom:
//!
//! ```text
//!  top bar      app chip · tabs · host
//!  (each tab's first row: location / title)
//!  sidebar │ pane
//!  transfer bar (only while something is moving or failed)
//!  status line  message or key hints
//! ```
//!
//! Hierarchy comes from colour and spacing; the only line is the one-column
//! gutter between the sidebar and the pane.

pub mod account;
pub mod charts;
pub mod icons;
pub mod overlays;
mod pane;
pub mod panel;
pub mod settings;
mod sidebar;
pub mod space;
mod statusbar;
pub mod tabs;
pub mod text;
pub mod transfers;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::Block;

use crate::app::App;
use crate::app::panel::{PANEL_ROWS, PanelMode};

/// Width of the accounts list.
pub const SIDEBAR_WIDTH: u16 = 18;
/// Below this width (beside the accounts list) two panes show one at a time.
pub const SPLIT_MIN_WIDTH: u16 = 80;
/// Below this terminal width the accounts list is hidden.
pub const SIDEBAR_MIN_TOTAL: u16 = 70;

pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn spinner(frame: usize) -> &'static str {
    SPINNER[frame % SPINNER.len()]
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    frame.render_widget(Block::default().style(app.theme.base()), area);

    let mode = crate::app::panel::panel_mode(app.panel_visible, area.width, area.height);
    let panel_rows = match mode {
        PanelMode::Expanded => PANEL_ROWS,
        PanelMode::Compact => 1,
        PanelMode::Hidden => 0,
    };
    // The expanded panel's Activity column replaces the transfer bar.
    let transfer_rows = if mode == PanelMode::Expanded {
        0
    } else {
        statusbar::transfer_bar_height(app)
    };
    let [top, main, panel_area, transfers, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(panel_rows),
        Constraint::Length(transfer_rows),
        Constraint::Length(1),
    ])
    .areas(area);

    tabs::draw(frame, top, app);
    if area.width >= SIDEBAR_MIN_TOTAL {
        let [side, gutter, pane_area] = Layout::horizontal([
            Constraint::Length(SIDEBAR_WIDTH),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .areas(main);
        sidebar::draw(frame, side, app);
        draw_gutter(frame, gutter, app);
        draw_main(frame, pane_area, app);
    } else if app.focus == crate::app::Focus::Sidebar {
        // No room for both: the accounts list, while it has the keys, takes the place
        // of the files, so what the keys act on is always on screen.
        sidebar::draw(frame, main, app);
    } else {
        draw_main(frame, main, app);
    }
    panel::draw(frame, panel_area, app, mode);
    if transfer_rows > 0 {
        statusbar::draw_transfers(frame, transfers, app);
    }
    statusbar::draw_status(frame, status, app);
    // The help scrolls by keys: they need to know how far it can go at this size.
    if let Some(crate::app::overlay::Overlay::Help { scroll }) = &mut app.overlay {
        let (max, page) = overlays::help_extent(area, &app.theme);
        *scroll = (*scroll).min(max);
        app.help_extent = (max, page);
    }
    overlays::draw(frame, area, app);
}

fn draw_main(frame: &mut Frame, area: Rect, app: &mut App) {
    if area.height == 0 {
        return;
    }
    if app.tab == crate::app::Tab::Space {
        return space::draw(frame, area, app);
    }
    if app.tab == crate::app::Tab::Files {
        return draw_files(frame, area, app);
    }
    let [title, rest] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
    match app.tab {
        crate::app::Tab::Files => unreachable!(),
        crate::app::Tab::Transfers => {
            let line = statusbar::transfers_title_line(app, title.width);
            frame.render_widget(ratatui::widgets::Paragraph::new(line), title);
            transfers::draw(frame, rest, app);
        }
        crate::app::Tab::Space => unreachable!(),
    }
}

/// The Files tab: one pane, or two side by side when there is room.
fn draw_files(frame: &mut Frame, area: Rect, app: &mut App) {
    use crate::app::Side;
    use pane::Slot;
    if !app.is_split() {
        return draw_pane_column(frame, area, app, Slot::Focused, None);
    }
    if area.width < SPLIT_MIN_WIDTH {
        let marker = if app.side == Side::Left { "1/2" } else { "2/2" };
        return draw_pane_column(frame, area, app, Slot::Focused, Some(marker));
    }
    let [left, gutter, right] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(1), Constraint::Fill(1)]).areas(area);
    let (l, r) = match app.side {
        Side::Left => (Slot::Focused, Slot::Other),
        Side::Right => (Slot::Other, Slot::Focused),
    };
    draw_pane_column(frame, left, app, l, None);
    draw_gutter(frame, gutter, app);
    draw_pane_column(frame, right, app, r, None);
}

/// A pane's location line and its file list.
fn draw_pane_column(frame: &mut Frame, area: Rect, app: &mut App, slot: pane::Slot, marker: Option<&str>) {
    let [title, rest] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
    let mut draw = |app: &mut App, active: bool| {
        let line = statusbar::location_line(app, active, marker, title.width);
        frame.render_widget(ratatui::widgets::Paragraph::new(line), title);
        pane::draw(frame, rest, app, active);
    };
    match slot {
        pane::Slot::Focused => draw(app, true),
        pane::Slot::Other => {
            app.with_other(|a| draw(a, false));
        }
    }
}

fn draw_gutter(frame: &mut Frame, area: Rect, app: &App) {
    let style = Style::default().fg(app.theme.tokens.gutter);
    let buf = frame.buffer_mut();
    for y in area.top()..area.bottom() {
        buf[(area.x, y)].set_symbol("│").set_style(style);
    }
}

/// The first row to show of a list of `len` rows so that `cursor` is inside a
/// viewport of `height` rows, given the `offset` shown last time: the list only
/// scrolls when the cursor would leave the viewport, and never past its end.
pub fn scroll_offset(offset: usize, cursor: usize, len: usize, height: usize) -> usize {
    let height = height.max(1);
    let offset = if cursor < offset {
        cursor
    } else if cursor >= offset + height {
        cursor + 1 - height
    } else {
        offset
    };
    offset.min(len.saturating_sub(height))
}

/// `0:41`, `12:03`, `1:02:03`.
pub fn eta(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// A bar of `width` cells, filled to `ratio` with eighth-block precision.
pub fn gauge(ratio: f64, width: usize) -> (String, String) {
    let ratio = ratio.clamp(0.0, 1.0);
    let eighths = (ratio * width as f64 * 8.0).round() as usize;
    let full = eighths / 8;
    let mut filled = "█".repeat(full);
    let partial = eighths % 8;
    if partial > 0 && full < width {
        filled.push(['▏', '▎', '▍', '▌', '▋', '▊', '▉'][partial - 1]);
    }
    let used = filled.chars().count();
    (filled, "░".repeat(width.saturating_sub(used)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_scrolls_only_when_the_cursor_would_leave_it() {
        // Going down: the cursor reaches the last row, then the list follows.
        assert_eq!(scroll_offset(0, 9, 100, 10), 0);
        assert_eq!(scroll_offset(0, 10, 100, 10), 1);
        // Going back up: the rows stay put until the cursor reaches the first one.
        assert_eq!(scroll_offset(1, 5, 100, 10), 1);
        assert_eq!(scroll_offset(1, 1, 100, 10), 1);
        assert_eq!(scroll_offset(1, 0, 100, 10), 0);
        // A list that got shorter, or a taller window: never past the end.
        assert_eq!(scroll_offset(50, 12, 15, 10), 5);
        assert_eq!(scroll_offset(3, 2, 4, 10), 0);
        assert_eq!(scroll_offset(0, 0, 0, 0), 0);
    }

    #[test]
    fn eta_format() {
        assert_eq!(eta(41), "0:41");
        assert_eq!(eta(723), "12:03");
        assert_eq!(eta(3723), "1:02:03");
    }

    #[test]
    fn gauge_cells() {
        assert_eq!(gauge(0.0, 4), (String::new(), "░░░░".into()));
        assert_eq!(gauge(1.0, 4), ("████".into(), String::new()));
        let (a, b) = gauge(0.78, 10);
        assert_eq!(a.chars().count() + b.chars().count(), 10);
        assert!(a.starts_with("███████"));
    }
}
