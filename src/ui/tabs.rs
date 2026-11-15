//! The top bar: app chip, tab chips and where the app runs.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::app::{App, Tab};
use crate::config::IconStyle;

/// `user@host`, plus ` · ssh` inside an SSH session.
pub fn host_label() -> String {
    let user = std::env::var("USER").unwrap_or_default();
    let host = std::fs::read_to_string("/etc/hostname")
        .map(|h| h.trim().to_string())
        .unwrap_or_default();
    let mut s = match (user.is_empty(), host.is_empty()) {
        (false, false) => format!("{user}@{host}"),
        (true, false) => host,
        (false, true) => user,
        (true, true) => String::new(),
    };
    if crate::paths::over_ssh() {
        s.push_str(" · ssh");
    }
    s
}

fn tab_name(tab: Tab) -> &'static str {
    match tab {
        Tab::Files => "Files",
        Tab::Space => "Space",
        Tab::Transfers => "Transfers",
    }
}

/// The app's name in the top bar, with the rodent of the logo when icons are on.
pub fn app_chip(icons: IconStyle) -> &'static str {
    match icons {
        IconStyle::Nerd => " \u{f1327} ratclone ",
        IconStyle::Ascii => " ratclone ",
    }
}

pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let chip = app_chip(app.icons);
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(
            chip,
            Style::default()
                .fg(t.tokens.bg)
                .bg(t.tokens.blue)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
    ];
    let names = area.width >= 60;
    for (i, tab) in Tab::ALL.into_iter().enumerate() {
        let n = i + 1;
        // A scan runs in the background: its tab shows a spinner wherever you are.
        let busy = if tab == Tab::Space && app.scan.is_some() {
            format!("{} ", super::spinner(app.spinner))
        } else {
            String::new()
        };
        let label = if names {
            format!(" {n} {} {busy}", tab_name(tab))
        } else {
            format!(" {n} {busy}")
        };
        if tab == app.tab {
            spans.push(Span::styled(
                label,
                Style::default()
                    .fg(t.tokens.fg)
                    .bg(t.tokens.hl)
                    .add_modifier(Modifier::BOLD),
            ));
        } else if names {
            spans.push(Span::styled(format!(" {n}"), t.dim()));
            spans.push(Span::styled(format!(" {} ", tab_name(tab)), t.secondary()));
            if !busy.is_empty() {
                spans.push(Span::styled(busy, Style::default().fg(t.tokens.blue)));
            }
        } else {
            spans.push(Span::styled(label, t.dim()));
        }
        spans.push(Span::raw(" "));
    }
    let host = app.host.clone();
    let used: usize = spans.iter().map(|s| s.content.width()).sum();
    let room = (area.width as usize).saturating_sub(used);
    if !host.is_empty() && host.width() + 2 <= room {
        spans.push(Span::raw(" ".repeat(room - host.width() - 2)));
        spans.push(Span::styled(host, t.dim()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)).style(t.topbar()), area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_chip_shows_a_rodent() {
        assert_eq!(app_chip(IconStyle::Nerd), " \u{f1327} ratclone ");
        assert_eq!(app_chip(IconStyle::Ascii), " ratclone ");
    }
}
