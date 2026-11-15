//! The transfers view (`T`): every transfer, and the details of the selected one.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use super::{eta, gauge, spinner};
use crate::app::App;
use crate::command::{Op, equivalent};
use crate::format::{human_size, short_time_in, truncate_middle, truncate_width};
use crate::theme::Theme;
use crate::transfer::spec::{FileProgress, TransferItem, TransferKind};
use crate::transfer::store::{RecordState, TransferRecord};

pub const RESUME_NOTE: &str = "Resume re-runs the transfer; files already copied are skipped.";

fn state_look(t: &Theme, s: RecordState, frame: usize) -> (&'static str, Color, &'static str) {
    match s {
        RecordState::Starting => (spinner(frame), t.tokens.blue, "starting"),
        RecordState::Running => (spinner(frame), t.tokens.blue, "running"),
        RecordState::Done => ("✓", t.tokens.green, "done"),
        RecordState::Failed => ("✗", t.tokens.red, "failed"),
        RecordState::Stopped => ("■", t.tokens.yellow, "stopped"),
        RecordState::Interrupted => ("⚠", t.tokens.orange, "interrupted"),
    }
}

/// Pad by display width (wide characters count double), truncating with `…`.
fn pad_right(s: &str, w: usize) -> String {
    let s = truncate_width(s, w);
    let fill = w.saturating_sub(s.width());
    format!("{s}{}", " ".repeat(fill))
}

fn pad_left(s: &str, w: usize) -> String {
    format!("{}{s}", " ".repeat(w.saturating_sub(s.width())))
}

pub fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    // Where the list is scrolled to is kept between frames, as in the file list.
    let detail_h = 9.min(area.height / 2);
    let len = app.store.records.len();
    let cursor = app.transfers.cursor.min(len.saturating_sub(1));
    let list_rows = area.height.saturating_sub(detail_h + 1).max(1) as usize;
    app.transfers.offset = super::scroll_offset(app.transfers.offset, cursor, len, list_rows);
    let app = &*app;
    let t = &app.theme;
    let order = app.transfer_records();
    if order.is_empty() {
        let p = Paragraph::new(vec![
            Line::raw(""),
            Line::raw(""),
            Line::styled("No transfers yet", t.dim()),
            Line::styled("copy with y or cut with x, then paste with p", t.dim()),
        ])
        .alignment(Alignment::Center);
        frame.render_widget(p, area);
        return;
    }
    let [list_area, rule, detail] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(detail_h),
    ])
    .areas(area);

    let now = jiff::Timestamp::now();
    let w = list_area.width as usize;
    let right_w = 34usize;
    // Lead: pad + icon + " copy " (8 cells); trail: 1 cell.
    let title_w = w.saturating_sub(right_w + 9);
    let rows = list_area.height as usize;
    let offset = app.transfers.offset;
    let mut lines = Vec::new();
    for (i, &r) in order.iter().enumerate().skip(offset).take(rows) {
        let id = &r.spec.id;
        let (icon, color, _) = state_look(t, r.state, app.spinner);
        let selected = i == cursor;
        let bg = |s: Style| if selected { s.bg(t.tokens.sel) } else { s };
        let p = app.progress.get(id);
        let right = match (r.state, p) {
            (RecordState::Running, _) if app.is_stopping(*id) => "stopping…".to_string(),
            (RecordState::Running, Some(p)) if p.total_bytes > 0 => {
                let ratio = p.bytes as f64 / p.total_bytes as f64;
                let speed = if p.speed > 0.0 {
                    format!("{}/s", human_size(p.speed as u64))
                } else {
                    String::new()
                };
                format!(
                    "{:>3.0}%  {:>11}  {:>7}",
                    ratio * 100.0,
                    speed,
                    p.eta.map(eta).unwrap_or_default()
                )
            }
            (RecordState::Running | RecordState::Starting, _) => "starting…".to_string(),
            _ => {
                let size = if r.total_bytes > 0 {
                    human_size(r.total_bytes)
                } else {
                    String::new()
                };
                let when = short_time_in(r.finished.unwrap_or(r.started), now, &app.tz);
                format!("{size:>11}  {when:>10}")
            }
        };
        let verb = if r.spec.kind == TransferKind::Move {
            "move"
        } else {
            "copy"
        };
        let mut line = Line::from(vec![
            Span::styled(" ", bg(Style::default())),
            Span::styled(icon, bg(Style::default().fg(color))),
            Span::styled(format!(" {verb} "), bg(t.dim())),
        ]);
        // The destination and the item count stay whole; the first name gives way.
        let more = r.spec.more();
        let dest = format!(" → {}", r.spec.dst_dir.display());
        let dest = truncate_width(&dest, title_w.saturating_sub(more.width() + 6));
        let name_w = title_w.saturating_sub(more.width() + dest.width());
        let name = truncate_middle(&r.spec.first_name(), name_w);
        let fill = title_w.saturating_sub(name.width() + more.width() + dest.width());
        line.spans.extend([
            Span::styled(
                name,
                bg(if selected {
                    t.text().add_modifier(Modifier::BOLD)
                } else {
                    t.text()
                }),
            ),
            Span::styled(more, bg(t.dim())),
            Span::styled(format!("{dest}{}", " ".repeat(fill)), bg(t.secondary())),
        ]);
        line.spans.extend([
            Span::styled(pad_left(&right, right_w), bg(t.secondary())),
            Span::styled(" ", bg(Style::default())),
        ]);
        lines.push(line);
    }
    frame.render_widget(Paragraph::new(lines), list_area);
    frame.render_widget(
        Paragraph::new(Line::styled("─".repeat(rule.width as usize), t.dim())),
        rule,
    );

    if let Some(&r) = order.get(cursor) {
        draw_detail(frame, detail, app, r);
    }
}

fn draw_detail(frame: &mut Frame, area: Rect, app: &App, r: &TransferRecord) {
    let t = &app.theme;
    let (icon, color, label) = state_look(t, r.state, app.spinner);
    let label_style = t.dim();
    let mut lines = vec![Line::from(vec![
        Span::styled(
            format!(" {icon} {label}  "),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("{} → ", r.spec.src_dir.display()), t.secondary()),
        Span::styled(r.spec.dst_dir.display(), Style::default().fg(t.tokens.blue)),
    ])];
    let mut tail = Vec::new();
    if let Some(err) = &r.error {
        tail.push(Line::from(vec![
            Span::styled("   error  ", label_style),
            Span::styled(err.clone(), Style::default().fg(t.tokens.red)),
        ]));
    }
    let cmds = equivalent(&Op::Transfer(&r.spec));
    if let Some(first) = cmds.first() {
        let more = match cmds.len() {
            1 => String::new(),
            n => format!("  (+{} more)", n - 1),
        };
        let cmd_w = (area.width as usize).saturating_sub(5 + more.width());
        tail.push(Line::from(vec![
            Span::styled("   $ ", label_style),
            Span::styled(truncate_middle(first, cmd_w), t.dim()),
            Span::styled(more, label_style),
        ]));
    }
    if matches!(
        r.state,
        RecordState::Stopped | RecordState::Failed | RecordState::Interrupted
    ) {
        tail.push(Line::styled(format!("   {RESUME_NOTE}"), t.dim()));
    }
    let room = (area.height as usize).saturating_sub(lines.len() + tail.len());
    lines.extend(item_lines(app, r, area.width as usize, room));
    lines.extend(tail);
    frame.render_widget(Paragraph::new(lines), area);
}

/// Where one item of a transfer stands.
enum ItemState<'a> {
    Done,
    Moving(&'a FileProgress),
    Other,
}

/// The file rclone reports as moving for `item`, if any. rclone names files by their
/// path, so a file matches by its last part and a folder by a leading part.
fn moving<'a>(item: &TransferItem, files: &'a [FileProgress]) -> Option<&'a FileProgress> {
    let names = [item.name.as_str(), item.dst_name.as_str()];
    files.iter().find(|f| {
        names.iter().any(|n| {
            if item.is_dir {
                f.name.starts_with(&format!("{n}/")) || f.name.contains(&format!("/{n}/"))
            } else {
                f.name == *n || f.name.ends_with(&format!("/{n}"))
            }
        })
    })
}

/// One line per item: state, name, and a gauge while it moves. When they do not fit
/// in `room` lines, finished items go first, then the rest is counted.
fn item_lines<'a>(app: &App, r: &TransferRecord, width: usize, room: usize) -> Vec<Line<'a>> {
    let t = &app.theme;
    if room == 0 {
        return Vec::new();
    }
    let done = r.done_items(&app.backend.job_units(&r.spec));
    let files: &[FileProgress] = match (r.state, app.progress.get(&r.spec.id)) {
        (RecordState::Running, Some(p)) => &p.files,
        _ => &[],
    };
    let states: Vec<ItemState> = r
        .spec
        .items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            if r.state == RecordState::Done || done[i] {
                ItemState::Done
            } else if let Some(f) = moving(item, files) {
                ItemState::Moving(f)
            } else {
                ItemState::Other
            }
        })
        .collect();
    let mut shown: Vec<usize> = (0..states.len()).collect();
    if shown.len() > room {
        let mut extra = shown.len() - room;
        shown.retain(|&i| {
            let drop = extra > 0 && matches!(states[i], ItemState::Done);
            extra -= drop as usize;
            !drop
        });
    }
    if shown.len() > room {
        shown.truncate(room - 1);
    }
    let hidden = states.len() - shown.len();

    // "   ⠧ name ……… ████████████  50%    42 B "
    let right_w = 30;
    let name_w = width.saturating_sub(5 + right_w + 1);
    let mut lines = Vec::new();
    for i in shown {
        let item = &r.spec.items[i];
        let size = item.size.map(human_size).unwrap_or_default();
        let (icon, icon_style, right) = match &states[i] {
            ItemState::Done => ("✓", Style::default().fg(t.tokens.green), Vec::new()),
            ItemState::Moving(f) => {
                let (full, rest) = gauge(f.percentage as f64 / 100.0, 12);
                (
                    spinner(app.spinner),
                    Style::default().fg(t.tokens.blue),
                    vec![
                        Span::styled(full, Style::default().fg(t.tokens.blue)),
                        Span::styled(rest, Style::default().fg(t.tokens.gutter)),
                        Span::styled(format!(" {:>3}%", f.percentage), t.secondary()),
                    ],
                )
            }
            ItemState::Other => ("·", t.dim(), Vec::new()),
        };
        let right_used: usize = right.iter().map(|s| s.content.width()).sum();
        let mut spans = vec![
            Span::styled(format!("   {icon} "), icon_style),
            Span::styled(
                pad_right(&truncate_middle(&item.label(), name_w), name_w),
                t.text(),
            ),
            Span::raw(" ".repeat(right_w.saturating_sub(right_used + 12))),
        ];
        spans.extend(right);
        spans.push(Span::styled(pad_left(&size, 12), t.secondary()));
        lines.push(Line::from(spans));
    }
    if hidden > 0 {
        lines.push(Line::styled(format!("     + {hidden} more"), t.dim()));
    }
    lines
}
