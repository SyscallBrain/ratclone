//! The accounts list.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use super::icons::remote_icon;
use super::{spinner, text};
use crate::app::{App, Focus};
use crate::format::truncate_width;
use crate::location::Remote;
use crate::theme::Theme;

/// Colour of a usage percentage: calm, then warning, then alarm.
pub fn usage_color(theme: &Theme, ratio: f64) -> Color {
    if ratio >= 0.9 {
        theme.tokens.red
    } else if ratio >= 0.75 {
        theme.tokens.yellow
    } else {
        theme.tokens.green
    }
}

fn remote_name(r: &Remote) -> &str {
    match r {
        Remote::Local => "local",
        Remote::Named(n) => n,
    }
}

/// First account shown: the cursor sits on the last line once it would go below it,
/// and the list never scrolls past its end.
fn sidebar_offset(cursor: usize, len: usize, rows: usize) -> usize {
    cursor
        .saturating_sub(rows.saturating_sub(1))
        .min(len.saturating_sub(rows))
}

pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let focused = app.focus == Focus::Sidebar;
    let width = area.width as usize;
    let mut lines = vec![Line::from(Span::styled(
        format!(" {}", text::ACCOUNTS),
        Style::default()
            .fg(if focused { t.tokens.blue } else { t.tokens.comment })
            .add_modifier(Modifier::BOLD),
    ))];
    // Scroll so the cursor's row is always on screen (the header takes one line).
    let rows = (area.height as usize).saturating_sub(1).max(1);
    let len = app.sidebar.remotes.len();
    let cursor = app.sidebar.cursor.min(len.saturating_sub(1));
    let offset = sidebar_offset(cursor, len, rows);
    for (i, info) in app.sidebar.remotes.iter().enumerate().skip(offset).take(rows) {
        let is_cursor = i == app.sidebar.cursor;
        // The account the main area shows: the Space tab follows the sidebar cursor.
        let is_open = if app.tab == crate::app::Tab::Space {
            is_cursor
        } else {
            info.remote == app.pane.loc.remote
        };
        let row_bg = if focused && is_cursor {
            Some(t.tokens.sel)
        } else {
            None
        };
        let with_bg = |s: Style| match row_bg {
            Some(bg) => s.bg(bg),
            None => s,
        };

        let pct = match (
            app.sidebar.about.get(&info.remote),
            app.sidebar.about_failed.get(&info.remote),
        ) {
            (Some((a, _)), _) => match a.used_ratio() {
                Some(r) => Span::styled(
                    format!("{:>3.0}%", (r * 100.0).min(100.0)),
                    with_bg(Style::default().fg(usage_color(t, r))),
                ),
                None => Span::styled("   –", with_bg(t.dim())),
            },
            (None, Some(_)) => Span::styled("   ?", with_bg(t.dim())),
            (None, None) => Span::styled(format!("   {}", spinner(app.spinner)), with_bg(t.dim())),
        };

        let bar = if is_open { "▌" } else { " " };
        let icon = remote_icon(&info.kind, app.icons);
        let name_width = width.saturating_sub(4 + 4 + 1);
        let name = truncate_width(remote_name(&info.remote), name_width);
        let pad = name_width.saturating_sub(name.width());
        let name_style = if is_open {
            Style::default().fg(t.tokens.blue).add_modifier(Modifier::BOLD)
        } else if is_cursor && focused {
            t.text().add_modifier(Modifier::BOLD)
        } else {
            t.secondary()
        };
        let icon_style = if is_open {
            Style::default().fg(t.tokens.blue)
        } else {
            t.dim()
        };
        lines.push(Line::from(vec![
            Span::styled(bar, with_bg(Style::default().fg(t.tokens.blue))),
            Span::styled(" ", with_bg(Style::default())),
            Span::styled(icon, with_bg(icon_style)),
            Span::styled(" ", with_bg(Style::default())),
            Span::styled(name, with_bg(name_style)),
            Span::styled(" ".repeat(pad), with_bg(Style::default())),
            pct,
            Span::styled(" ", with_bg(Style::default())),
        ]));
    }
    frame.render_widget(Paragraph::new(lines).style(t.base()), area);
}

#[cfg(test)]
mod tests {
    use super::sidebar_offset;

    #[test]
    fn scrolls_only_when_needed() {
        assert_eq!(sidebar_offset(0, 30, 13), 0);
        assert_eq!(sidebar_offset(12, 30, 13), 0);
        assert_eq!(sidebar_offset(13, 30, 13), 1);
        assert_eq!(sidebar_offset(29, 30, 13), 17);
        assert_eq!(sidebar_offset(3, 5, 13), 0, "short lists never scroll");
        assert_eq!(sidebar_offset(40, 30, 13), 17, "a stale cursor stays in range");
    }
}
