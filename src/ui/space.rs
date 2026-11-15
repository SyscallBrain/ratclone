//! The Space tab: a ranked list of folders with bars (spec §8).

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use super::{gauge, spinner};
use crate::app::App;
use crate::app::space::share;
use crate::format::{human_size, truncate_width};
use crate::location::Remote;
use crate::theme::Theme;

/// Colours rows cycle through, by rank. They only tell rows apart.
pub fn rank_color(t: &Theme, i: usize) -> ratatui::style::Color {
    let k = &t.tokens;
    [
        k.blue, k.magenta, k.cyan, k.green, k.yellow, k.orange, k.teal, k.red,
    ][i % 8]
}

/// `1 234 567`: spaces as thousands separators.
pub fn grouped(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

/// How long ago a scan finished: `now`, `5 min ago`, `3 h ago`, `2 days ago`.
pub fn ts_age(t: jiff::Timestamp) -> String {
    let s = jiff::Timestamp::now()
        .as_second()
        .saturating_sub(t.as_second())
        .max(0);
    match s {
        0..60 => "now".into(),
        60..3600 => format!("{} min ago", s / 60),
        3600..86400 => format!("{} h ago", s / 3600),
        _ => format!("{} days ago", s / 86400),
    }
}

fn account(r: &Remote) -> String {
    match r {
        Remote::Local => "local".into(),
        Remote::Named(n) => n.clone(),
    }
}

fn pad_left(s: &str, w: usize) -> String {
    format!("{}{s}", " ".repeat(w.saturating_sub(s.width())))
}

fn pad_right(s: &str, w: usize) -> String {
    format!("{s}{}", " ".repeat(w.saturating_sub(s.width())))
}

/// Left and right spans on one line; the right side is dropped if it does not fit.
fn spread(mut left: Vec<Span<'static>>, right: Vec<Span<'static>>, width: usize) -> Line<'static> {
    let lw: usize = left.iter().map(|s| s.content.width()).sum();
    let rw: usize = right.iter().map(|s| s.content.width()).sum();
    if lw + rw < width {
        left.push(Span::raw(" ".repeat(width - lw - rw)));
        left.extend(right);
    }
    Line::from(left)
}

pub fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    if area.height == 0 {
        return;
    }
    let t = app.theme;
    let header = Rect { height: 1, ..area };
    let list = Rect {
        y: area.y + 2,
        height: area.height.saturating_sub(2),
        ..area
    };
    app.space_viewport = list.height.max(1) as usize;
    // Where the list is scrolled to is kept between frames, as in the file list.
    let len = app.space.shown.len();
    app.space.offset = super::scroll_offset(
        app.space.offset,
        app.space.cursor.min(len.saturating_sub(1)),
        len,
        list.height as usize,
    );
    let Some(root) = app.space_root() else {
        return;
    };
    let name = account(&root.remote);
    let mut left = vec![Span::styled(
        format!(" {name}:"),
        Style::default().fg(t.tokens.blue).add_modifier(Modifier::BOLD),
    )];
    let shown_path = match &root.remote {
        Remote::Local => format!(
            "/{}",
            [root.path.as_str(), app.space.rel.as_str()]
                .iter()
                .filter(|s| !s.is_empty())
                .cloned()
                .collect::<Vec<_>>()
                .join("/")
        ),
        _ => format!("/{}", app.space.rel),
    };
    left.push(Span::styled(shown_path, t.text().add_modifier(Modifier::BOLD)));
    let scanning = app.scan.as_ref().filter(|a| a.tree.root == root);
    let w = area.width as usize;

    let Some(tree) = app.space_tree() else {
        frame.render_widget(Paragraph::new(spread(left, Vec::new(), w)), header);
        let mut lines = vec![
            Line::raw(""),
            Line::raw(""),
            Line::styled(format!("{name} has not been scanned"), t.text()),
        ];
        if let Some(used) = app.sidebar.about.get(&root.remote).and_then(|(a, _)| a.used) {
            lines.push(Line::styled(format!("~{} to read", human_size(used)), t.dim()));
        }
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![
            Span::styled("Enter", t.key()),
            Span::styled(" scan · may take minutes on large accounts", t.dim()),
        ]));
        frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), list);
        return;
    };

    // Header: totals, or progress while scanning.
    let right = match scanning {
        Some(a) => {
            let found = a.tree.node.bytes;
            let secs = a.started.elapsed().as_secs();
            let spin = vec![Span::styled(
                format!("{} ", spinner(app.spinner)),
                Style::default().fg(t.tokens.blue),
            )];
            let current = vec![
                Span::styled(truncate_width(&format!("/{}", a.current), 24), t.secondary()),
                Span::raw("  "),
            ];
            let mut bar = Vec::new();
            let mut pct = Vec::new();
            if let Some(expected) = a.expected_bytes.filter(|e| *e > 0) {
                let ratio = (found as f64 / expected as f64).min(0.99);
                let (fill, rest) = gauge(ratio, 12);
                bar.push(Span::styled(fill, Style::default().fg(t.tokens.blue)));
                bar.push(Span::styled(rest, Style::default().fg(t.tokens.hl)));
                bar.push(Span::raw(" "));
                pct.push(Span::styled(format!("~{:.0}%  ", ratio * 100.0), t.secondary()));
            }
            let counts = vec![Span::styled(
                format!(
                    "{} files · {} · {}:{:02}  ",
                    grouped(a.tree.node.files),
                    human_size(found),
                    secs / 60,
                    secs % 60
                ),
                t.dim(),
            )];
            let esc = vec![Span::styled("Esc", t.key()), Span::styled(" cancel ", t.dim())];
            // Narrow terminals drop the details first; spinner, % and Esc always stay.
            let room = w.saturating_sub(left.iter().map(|s| s.content.width()).sum::<usize>() + 1);
            let variants = [
                vec![&spin, &current, &bar, &pct, &counts, &esc],
                vec![&spin, &bar, &pct, &counts, &esc],
                vec![&spin, &pct, &counts, &esc],
                vec![&spin, &pct, &esc],
            ];
            let pick = variants
                .iter()
                .find(|v| {
                    v.iter()
                        .flat_map(|p| p.iter())
                        .map(|s| s.content.width())
                        .sum::<usize>()
                        <= room
                })
                .unwrap_or(&variants[3]);
            pick.iter().flat_map(|p| p.iter().cloned()).collect()
        }
        None => {
            let warn = Style::default().fg(t.tokens.yellow);
            let mut r = vec![Span::styled(
                format!(
                    "{} · {} files · scanned {}",
                    human_size(tree.node.bytes),
                    grouped(tree.node.files),
                    ts_age(tree.scanned_at)
                ),
                t.dim(),
            )];
            if tree.partial {
                r.push(Span::styled(" · partial", warn));
            }
            if tree.unreadable > 0 {
                let n = tree.unreadable;
                r.push(Span::styled(
                    format!(" · {n} {} unreadable", if n == 1 { "folder" } else { "folders" }),
                    warn,
                ));
            }
            if app.changed_since_scan.contains_key(&root.remote) {
                r.push(Span::styled(" · changed since scan", warn));
            }
            r.push(Span::raw(" "));
            r
        }
    };
    frame.render_widget(Paragraph::new(spread(left, right, w)), header);

    let Some(node) = tree.node(&app.space.rel) else {
        return;
    };
    let rows = &app.space.shown;
    if rows.is_empty() {
        let msg = if app.space.filter.is_empty() {
            if scanning.is_some() {
                "Reading…"
            } else {
                "Empty folder"
            }
        } else {
            "No folder matches the filter"
        };
        frame.render_widget(
            Paragraph::new(vec![Line::raw(""), Line::styled(msg, t.dim())]).alignment(Alignment::Center),
            list,
        );
        return;
    }
    let name_w = (w / 5).clamp(12, 28);
    let bar_w = w.saturating_sub(name_w + 40).clamp(10, 40);
    let biggest = rows.iter().map(|r| r.bytes).max().unwrap_or(0);
    let focused = app.focus == crate::app::Focus::Pane;
    let height = list.height as usize;
    let cursor = app.space.cursor.min(rows.len() - 1);
    let offset = app.space.offset;
    let mut lines = Vec::new();
    for (i, r) in rows.iter().enumerate().skip(offset).take(height) {
        let cur = i == cursor;
        let color = rank_color(&t, i);
        let name_style = if r.is_dir {
            Style::default().fg(t.tokens.blue).add_modifier(Modifier::BOLD)
        } else {
            t.secondary()
        };
        let track = if cur { t.tokens.gutter } else { t.tokens.hl };
        let (fill, _) = gauge(share(r.bytes, biggest), bar_w);
        let rest = bar_w.saturating_sub(fill.chars().count());
        let mut spans = vec![
            Span::raw(" "),
            Span::styled("▍", Style::default().fg(color)),
            Span::raw(" "),
            Span::styled(pad_right(&truncate_width(&r.name, name_w), name_w), name_style),
            Span::styled(pad_left(&human_size(r.bytes), 11), t.text()),
            Span::raw("  "),
            Span::styled(fill, Style::default().fg(color)),
            Span::styled("·".repeat(rest), Style::default().fg(track)),
            Span::styled(
                pad_left(&format!("{:.0}%", share(r.bytes, node.bytes) * 100.0), 6),
                t.secondary(),
            ),
        ];
        let used: usize = spans.iter().map(|s| s.content.width()).sum();
        let count = format!(
            "{} {} ",
            grouped(r.files),
            if r.files == 1 { "file" } else { "files" }
        );
        if used + count.width() < w {
            spans.push(Span::styled(pad_left(&count, w - used), t.dim()));
        }
        let mut line = Line::from(spans);
        if cur {
            let bg = if focused { t.tokens.sel } else { t.tokens.hl };
            line = line.style(Style::default().bg(bg));
        }
        lines.push(line);
    }
    frame.render_widget(Paragraph::new(lines), list);
}
