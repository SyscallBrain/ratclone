//! Top bar, transfer bar and status line.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use super::{eta, gauge, spinner, text};
use crate::app::overlay::{InputPurpose, Overlay};
use crate::app::{App, Focus, MessageKind};
use crate::format::{human_size, truncate_width};
use crate::input::keymap::SortKey;
use crate::location::Remote;
use crate::transfer::spec::TransferKind;
use crate::transfer::store::RecordState;

fn width(spans: &[Span]) -> usize {
    spans.iter().map(|s| s.content.width()).sum()
}

/// Left and right groups on one line; spans that do not fit are dropped from the end of `left`.
fn split_line<'a>(mut left: Vec<Span<'a>>, right: Vec<Span<'a>>, total: usize) -> Line<'a> {
    let rw = width(&right);
    while width(&left) + rw > total && left.len() > 1 {
        left.pop();
    }
    let gap = total.saturating_sub(width(&left) + rw);
    left.push(Span::raw(" ".repeat(gap)));
    left.extend(right);
    Line::from(left)
}

/// First row of the Transfers tab: running count and history size.
pub fn transfers_title_line(app: &App, width: u16) -> Line<'static> {
    let t = &app.theme;
    let running = app.store.running().count();
    let left = vec![
        Span::styled(" Transfers", t.text().add_modifier(Modifier::BOLD)),
        Span::styled(
            format!("  {running} running · {} in history", app.store.records.len()),
            t.secondary(),
        ),
    ];
    split_line(left, vec![Span::styled(" T close ", t.dim())], width as usize)
}

/// First row of the Files tab: where we are, then filter, marks, sort, count and pending keys.
/// Describes `app.pane`; `active` is false for the unfocused one of two panes,
/// and `marker` (`1/2`, `2/2`) says which pane shows when only one fits.
pub fn location_line(app: &App, active: bool, marker: Option<&str>, cols: u16) -> Line<'static> {
    let t = &app.theme;
    let loc = &app.pane.loc;
    let remote = match &loc.remote {
        Remote::Local => "local".to_string(),
        Remote::Named(n) => n.clone(),
    };
    let remote_style = if active {
        Style::default().fg(t.tokens.blue).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(t.tokens.comment)
    };
    let mut left = vec![Span::styled(format!(" {remote}:"), remote_style)];
    let segments: Vec<&str> = loc.path.split('/').filter(|s| !s.is_empty()).collect();

    let mut right: Vec<Span> = Vec::new();
    if let Some(m) = marker {
        right.push(Span::styled(format!(" {m} "), t.dim()));
    }
    let pane = &app.pane;
    if !pane.filter.is_empty() {
        right.push(Span::styled(
            format!(" /{} ", pane.filter),
            Style::default().fg(t.tokens.yellow),
        ));
    }
    if !pane.marks.is_empty() {
        right.push(Span::styled(
            format!(" {} selected ", pane.marks.len()),
            Style::default().fg(t.tokens.magenta),
        ));
    }
    if pane.sort != SortKey::Name || pane.reverse {
        let key = match pane.sort {
            SortKey::Name => "name",
            SortKey::Size => "size",
            SortKey::Modified => "modified",
            SortKey::Count => "count",
        };
        right.push(Span::styled(
            format!(" {}{key} ", if pane.reverse { "↑" } else { "↓" }),
            t.dim(),
        ));
    }
    if pane.show_hidden {
        right.push(Span::styled(format!(" {} ", text::HIDDEN_ON), t.dim()));
    }
    if pane.listing == crate::app::pane::Listing::Ready {
        let n = pane.len();
        // Rows from the listing cache: say that fresh ones are coming.
        let refreshing = if pane.refreshing {
            format!("{} ", spinner(app.spinner))
        } else {
            String::new()
        };
        right.push(Span::styled(
            format!(" {refreshing}{n} {} ", if n == 1 { "item" } else { "items" }),
            t.dim(),
        ));
    }
    let keys = app.keys.pending_keys();
    if active && !keys.is_empty() {
        right.push(Span::styled(format!(" {keys} "), t.key()));
    }
    // Breadcrumb: drop leading folders (shown as "…") until it fits.
    let room = (cols as usize).saturating_sub(width(&left) + width(&right) + 1);
    let mut skip = 0;
    let crumb_width = |skip: usize| -> usize {
        let shown = &segments[skip..];
        shown.iter().map(|s| s.width() + 3).sum::<usize>() + if skip > 0 { 4 } else { 0 }
    };
    while skip + 1 < segments.len() && crumb_width(skip) > room {
        skip += 1;
    }
    if skip > 0 {
        left.push(Span::styled("  …", t.dim()));
    }
    for (i, seg) in segments.iter().enumerate().skip(skip) {
        let last = i + 1 == segments.len();
        let style = if last {
            t.text().add_modifier(Modifier::BOLD)
        } else {
            t.secondary()
        };
        left.push(Span::styled(
            if i == skip && skip == 0 { " " } else { " › " },
            t.dim(),
        ));
        let max = room.saturating_sub(3).max(1);
        left.push(Span::styled(truncate_width(seg, max), style));
    }
    split_line(left, right, cols as usize)
}

pub fn transfer_bar_height(app: &App) -> u16 {
    let any = app.store.records.iter().any(|r| {
        matches!(
            r.state,
            RecordState::Starting | RecordState::Running | RecordState::Failed | RecordState::Interrupted
        )
    });
    u16::from(any)
}

pub fn draw_transfers(frame: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let running: Vec<_> = app.store.running().collect();
    let problems = app
        .store
        .records
        .iter()
        .filter(|r| matches!(r.state, RecordState::Failed | RecordState::Interrupted))
        .count();
    let mut left: Vec<Span> = vec![Span::raw(" ")];
    if !running.is_empty() {
        let (mut bytes, mut total, mut speed) = (0u64, 0u64, 0f64);
        let mut eta_secs: Option<u64> = None;
        for r in &running {
            if let Some(p) = app.progress.get(&r.spec.id) {
                bytes += p.bytes;
                total += p.total_bytes;
                speed += p.speed;
                eta_secs = eta_secs.max(p.eta);
            }
        }
        left.push(Span::styled(
            format!("{} ", spinner(app.spinner)),
            Style::default().fg(t.tokens.blue),
        ));
        if let [one] = running.as_slice() {
            let verb = if one.spec.kind == TransferKind::Move {
                "Moving"
            } else {
                "Copying"
            };
            left.push(Span::styled(format!("{verb} "), t.dim()));
            left.push(Span::styled(one.spec.title_fit(40), t.text()));
        } else {
            left.push(Span::styled(format!("{} transfers", running.len()), t.text()));
            if total > 0 {
                left.push(Span::styled(
                    format!("  {} / {}", human_size(bytes), human_size(total)),
                    t.secondary(),
                ));
            }
        }
        if total > 0 {
            let ratio = bytes as f64 / total as f64;
            let (done, rest) = gauge(ratio, 16);
            left.push(Span::raw("  "));
            left.push(Span::styled(done, Style::default().fg(t.tokens.blue)));
            left.push(Span::styled(rest, Style::default().fg(t.tokens.gutter)));
            left.push(Span::styled(format!(" {:>3.0}%", ratio * 100.0), t.text()));
            if speed > 0.0 {
                left.push(Span::styled(
                    format!("  {}/s", human_size(speed as u64)),
                    t.secondary(),
                ));
            }
            if let Some(e) = eta_secs {
                left.push(Span::styled(format!("  ETA {}", eta(e)), t.dim()));
            }
        } else {
            left.push(Span::styled("  starting…", t.dim()));
        }
    }
    let mut right = Vec::new();
    if problems > 0 {
        right.push(Span::styled(
            format!(
                "✗ {problems} {} ",
                if problems == 1 { "problem" } else { "problems" }
            ),
            Style::default().fg(t.tokens.red),
        ));
        right.push(Span::styled("T ", t.key()));
    }
    let line = split_line(left, right, area.width as usize);
    frame.render_widget(Paragraph::new(line).style(t.topbar()), area);
}

/// Columns the filter text keeps before the key hint beside it is dropped.
const FILTER_MIN_ROOM: usize = 24;

pub fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    // Live filter input.
    if let Some(Overlay::Input {
        input,
        purpose: InputPurpose::Filter,
        ..
    }) = &app.overlay
    {
        let total = area.width as usize;
        let hint = format!("{} ", text::FILTER_HINT);
        // The hint only when it leaves the filter a useful width.
        let hint_w = hint.width();
        let with_hint = total >= hint_w + 2 + FILTER_MIN_ROOM;
        let room = total.saturating_sub(2 + if with_hint { hint_w + 1 } else { 0 });
        let (before, at, after) = crate::format::edit_window(&input.value, input.cursor, room);
        let left = vec![
            Span::styled(
                " /",
                Style::default().fg(t.tokens.yellow).add_modifier(Modifier::BOLD),
            ),
            Span::styled(before, t.text()),
            Span::styled(at, Style::default().fg(t.tokens.bg).bg(t.tokens.fg)),
            Span::styled(after, t.text()),
        ];
        let right = if with_hint {
            vec![Span::styled(hint, t.dim())]
        } else {
            Vec::new()
        };
        frame.render_widget(Paragraph::new(split_line(left, right, area.width as usize)), area);
        return;
    }
    if let Some(m) = &app.message {
        let (icon, color) = match m.kind {
            MessageKind::Info => ("•", t.tokens.blue),
            MessageKind::Success => ("✓", t.tokens.green),
            MessageKind::Error => ("✗", t.tokens.red),
        };
        let line = Line::from(vec![
            Span::styled(
                format!(" {icon} "),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                truncate_width(&m.text, area.width.saturating_sub(4) as usize),
                t.text(),
            ),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        return;
    }
    let mut left: Vec<Span> = vec![Span::raw(" ")];
    if let Some(r) = app
        .register
        .as_ref()
        .filter(|_| app.tab != crate::app::Tab::Transfers)
    {
        let verb = if r.kind == TransferKind::Copy {
            "copied"
        } else {
            "cut"
        };
        let color = if r.kind == TransferKind::Copy {
            t.tokens.yellow
        } else {
            t.tokens.orange
        };
        let n = r.items.len();
        left.push(Span::styled(
            format!("{n} {} {verb}", if n == 1 { "item" } else { "items" }),
            Style::default().fg(color),
        ));
        left.push(Span::styled("  p", t.key()));
        left.push(Span::styled(" paste  ", t.dim()));
        left.push(Span::styled("Esc", t.key()));
        left.push(Span::styled(" clear", t.dim()));
    } else if !app.key_hints {
        // Hints turned off in the settings: only a quiet way back to them.
        let right = vec![Span::styled("?", t.key()), Span::styled(" help ", t.dim())];
        let line = split_line(left, right, area.width as usize);
        frame.render_widget(Paragraph::new(line), area);
        return;
    } else {
        let hints = if app.focus == Focus::Sidebar {
            text::SIDEBAR_HINTS
        } else if app.tab == crate::app::Tab::Transfers {
            text::TRANSFERS_HINTS
        } else if app.tab == crate::app::Tab::Space {
            if app.space_tree().is_some() {
                text::SPACE_HINTS
            } else {
                text::SPACE_EMPTY_HINTS
            }
        } else {
            text::PANE_HINTS
        };
        for (key, label) in hints {
            left.push(Span::styled(key.to_string(), t.key()));
            left.push(Span::styled(format!(" {label}  "), t.dim()));
        }
    }
    let line = split_line(left, Vec::new(), area.width as usize);
    frame.render_widget(Paragraph::new(line), area);
}
