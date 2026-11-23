//! The info panel at the bottom: Storage · Activity · Selected (spec §6.2, §7).
//!
//! Every tab shares the frame and the first two columns; the third column
//! (and, in the Space tab, the whole body) depends on the tab.

use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use super::charts::{ascii_spark, braille_rows, stacked};
use super::icons::icon;
use super::text::remote_kind_label;
use crate::app::panel::PanelMode;
use crate::app::{App, Tab};
use crate::backend::About;
use crate::config::IconStyle;
use crate::format::{human_size, truncate_width};
use crate::location::Remote;
use crate::theme::Theme;
use crate::transfer::store::RecordState;

/// Width of the Storage column, including the left margin.
const STORAGE_W: u16 = 36;
/// Width of the Selected column.
const SELECTED_W: u16 = 34;
/// Below this width the Selected column is dropped.
const SELECTED_MIN_TOTAL: u16 = 100;

pub fn draw(frame: &mut Frame, area: Rect, app: &App, mode: PanelMode) {
    match mode {
        PanelMode::Hidden => {}
        PanelMode::Compact => draw_compact(frame, area, app),
        PanelMode::Expanded => draw_expanded(frame, area, app),
    }
}

// ---- shared pieces --------------------------------------------------------

/// `1 folder`, `12 345 folders`.
fn counted(n: u64, what: &str) -> String {
    format!(
        "{} {what}{}",
        super::space::grouped(n),
        if n == 1 { "" } else { "s" }
    )
}

fn account_label(r: &Remote) -> String {
    remote_label(r)
}

fn remote_label(r: &Remote) -> String {
    match r {
        Remote::Local => "local".into(),
        Remote::Named(n) => n.clone(),
    }
}

/// The account the panel describes (see [`App::panel_account`]).
fn account(app: &App) -> Option<(Remote, String)> {
    app.panel_account().map(|r| (r.remote.clone(), r.kind.clone()))
}

fn about_of<'a>(app: &'a App, remote: &Remote) -> Option<&'a (About, Instant)> {
    app.sidebar.about.get(remote)
}

/// `now`, `3 min ago`, `2 h ago`.
pub fn age(at: Instant) -> String {
    let s = at.elapsed().as_secs();
    if s < 60 {
        "now".into()
    } else if s < 3600 {
        format!("{} min ago", s / 60)
    } else {
        format!("{} h ago", s / 3600)
    }
}

/// Like `human_size`, without the decimal from 100 up (`800 MiB`, `4.0 GiB`).
pub fn short_size(bytes: u64) -> String {
    let s = human_size(bytes);
    match s.split_once(' ') {
        Some((n, unit)) if n.parse::<f64>().is_ok_and(|v| v >= 100.0) => {
            format!("{} {unit}", n.split('.').next().unwrap_or(n))
        }
        _ => s,
    }
}

/// `● Label value` legend items, laid out two per line.
fn legend(t: &Theme, dot: &'static str, items: &[(Color, &str, String)]) -> Vec<Line<'static>> {
    items
        .chunks(2)
        .map(|pair| {
            let mut spans = Vec::new();
            for (i, (c, label, value)) in pair.iter().enumerate() {
                if i > 0 {
                    spans.push(Span::raw("  "));
                }
                spans.push(Span::styled(format!("{dot} "), Style::default().fg(*c)));
                spans.push(Span::styled(format!("{label} "), t.dim()));
                spans.push(Span::styled(value.clone(), t.secondary()));
            }
            Line::from(spans)
        })
        .collect()
}

fn label(t: &Theme, s: &str) -> Span<'static> {
    Span::styled(s.to_string(), t.dim().add_modifier(Modifier::BOLD))
}

fn bullet(app: &App) -> &'static str {
    if app.icons == IconStyle::Ascii { "*" } else { "●" }
}

/// Storage parts known for `a`: (value, colour, label).
fn storage_parts(t: &Theme, a: &About) -> Vec<(u64, Color, &'static str)> {
    let mut parts = Vec::new();
    if let Some(u) = a.used {
        parts.push((u, t.tokens.blue, "Used"));
    }
    if let Some(v) = a.trashed {
        parts.push((v, t.tokens.orange, "Trash"));
    }
    if let Some(v) = a.other {
        parts.push((v, t.tokens.magenta, "Other"));
    }
    parts
}

// ---- expanded -------------------------------------------------------------

fn draw_expanded(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 {
        return;
    }
    if app.tab == Tab::Space && draw_space(frame, area, app) {
        return;
    }
    let t = &app.theme;
    let sep = Rect { height: 1, ..area };
    frame.render_widget(Paragraph::new(separator(app, area.width)), sep);
    let body = Rect {
        y: area.y + 1,
        height: area.height - 1,
        ..area
    };
    let with_selected = area.width >= SELECTED_MIN_TOTAL;
    let storage = Rect {
        x: body.x + 2,
        width: STORAGE_W - 4,
        ..body
    };
    let act_x = body.x + STORAGE_W + 2;
    let act_end = if with_selected {
        body.x + body.width - SELECTED_W - 3
    } else {
        body.x + body.width - 2
    };
    let activity = Rect {
        x: act_x,
        width: act_end.saturating_sub(act_x),
        ..body
    };
    let divider = |frame: &mut Frame, x: u16| {
        let buf = frame.buffer_mut();
        for y in body.y..body.y + body.height {
            buf[(x, y)]
                .set_symbol("│")
                .set_style(Style::default().fg(t.tokens.hl));
        }
    };
    divider(frame, body.x + STORAGE_W - 1);
    frame.render_widget(Paragraph::new(storage_lines(app, storage.width)), storage);
    frame.render_widget(Paragraph::new(activity_lines(app, activity.width)), activity);
    if with_selected {
        divider(frame, body.x + body.width - SELECTED_W - 1);
        let sel = Rect {
            x: body.x + body.width - SELECTED_W + 2,
            width: SELECTED_W - 3,
            ..body
        };
        let lines = match app.tab {
            Tab::Transfers => transfer_lines(app, sel.width),
            Tab::Files => selected_lines(app, sel.width),
            // Nothing selected in Space yet (not scanned, or still empty).
            Tab::Space => vec![Line::from(label(t, "SELECTED"))],
        };
        frame.render_widget(Paragraph::new(lines), sel);
    }
}

fn separator(app: &App, width: u16) -> Line<'static> {
    let t = &app.theme;
    let mut left = vec![Span::styled("──", Style::default().fg(t.tokens.gutter))];
    let mut right_text = String::from(" i hide · R refresh ");
    let scanning = app
        .scan
        .as_ref()
        .filter(|a| app.tab == Tab::Space && Some(&a.tree.root) == app.space_root().as_ref());
    if let Some(a) = scanning {
        left.push(Span::styled(
            format!(" {} ", super::spinner(app.spinner)),
            Style::default().fg(t.tokens.blue),
        ));
        left.push(Span::styled(
            format!("scanning {} ", remote_label(&a.tree.root.remote)),
            Style::default().fg(t.tokens.blue).add_modifier(Modifier::BOLD),
        ));
        left.push(Span::styled(
            format!(
                "· {} · {} ",
                counted(a.tree.listed, "folder"),
                counted(a.tree.node.files, "file")
            ),
            t.dim(),
        ));
    } else if let Some((remote, kind)) = account(app) {
        left.push(Span::styled(
            format!(" {} ", remote_label(&remote)),
            Style::default().fg(t.tokens.blue).add_modifier(Modifier::BOLD),
        ));
        left.push(Span::styled(format!("{} ", remote_kind_label(&kind)), t.dim()));
        if let Some((_, at)) = about_of(app, &remote) {
            right_text = format!(" i hide · R refresh · {} ", age(*at));
        }
        // What can be done with the account under the cursor.
        if app.focus == crate::app::Focus::Sidebar && matches!(remote, crate::location::Remote::Named(_)) {
            right_text = format!(" a add · e edit · dd remove ·{right_text}");
        }
    }
    let used: usize = left.iter().map(|s| s.content.width()).sum();
    let right_w = right_text.width() + 2;
    let total = width as usize;
    if used + right_w <= total {
        left.push(Span::styled(
            "─".repeat(total - used - right_w),
            Style::default().fg(t.tokens.gutter),
        ));
        left.push(Span::styled(right_text, t.dim()));
        left.push(Span::styled("──", Style::default().fg(t.tokens.gutter)));
    } else if used < total {
        left.push(Span::styled(
            "─".repeat(total - used),
            Style::default().fg(t.tokens.gutter),
        ));
    }
    Line::from(left)
}

fn storage_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let t = &app.theme;
    let mut lines = vec![Line::from(label(t, "STORAGE"))];
    let Some((remote, _)) = account(app) else {
        return lines;
    };
    let Some((a, _)) = about_of(app, &remote) else {
        let msg = if app.sidebar.about_failed.contains_key(&remote) {
            "Usage unknown"
        } else {
            "Loading usage…"
        };
        lines.push(Line::styled(msg, t.dim()));
        return lines;
    };
    let bar_w = (width as usize).min(30);
    match (a.used, a.total) {
        (Some(used), Some(total)) if total > 0 => {
            let pct = (used as f64 / total as f64 * 100.0).round() as u64;
            lines.push(Line::from(vec![
                Span::styled(format!("{pct}%"), t.text().add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!(" {} of {}", human_size(used), human_size(total)),
                    t.secondary(),
                ),
            ]));
            lines.push(Line::raw(""));
            let parts: Vec<(u64, Color)> = storage_parts(t, a).iter().map(|p| (p.0, p.1)).collect();
            lines.push(Line::from(stacked(&parts, total, bar_w, t.tokens.hl)));
            lines.push(Line::raw(""));
        }
        (Some(used), _) => {
            lines.push(Line::from(vec![
                Span::styled(format!("Used {}", human_size(used)), t.secondary()),
                Span::styled(" · no quota reported", t.dim()),
            ]));
            lines.push(Line::raw(""));
        }
        _ => {
            lines.push(Line::styled("Usage unknown", t.dim()));
            return lines;
        }
    }
    let mut items: Vec<(Color, &str, String)> = storage_parts(t, a)
        .into_iter()
        .map(|(v, c, l)| (c, l, short_size(v)))
        .collect();
    if let Some(free) = a.free {
        items.push((t.tokens.gutter, "Free", short_size(free)));
    }
    lines.extend(legend(t, bullet(app), &items));
    lines
}

fn activity_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let t = &app.theme;
    let w = width as usize;
    let mut lines = Vec::new();
    let head = "ACTIVITY";
    lines.push(Line::from(vec![
        label(t, head),
        Span::raw(" ".repeat(w.saturating_sub(head.len() + 3))),
        Span::styled("60s", t.dim()),
    ]));
    let running = app.store.running().count();
    let speed = app.speed.latest();
    if speed <= 0.0 && running == 0 {
        lines.push(Line::styled("idle", t.dim()));
    } else {
        lines.push(Line::styled(
            format!("↑ {}/s", human_size(speed as u64)),
            Style::default().fg(t.tokens.teal).add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(limit) = &app.bwlimit_shown
        && let Some(last) = lines.last_mut()
    {
        last.spans
            .push(Span::styled(format!(" · limit {limit}"), t.dim()));
    }
    let samples = app.speed.samples();
    if app.icons == IconStyle::Ascii {
        lines.push(Line::raw(""));
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            ascii_spark(&samples, w),
            Style::default().fg(t.tokens.teal),
        ));
    } else {
        for (i, row) in braille_rows(&samples, 3, w).into_iter().enumerate() {
            let c = if i == 0 { t.tokens.cyan } else { t.tokens.teal };
            lines.push(Line::styled(row, Style::default().fg(c)));
        }
    }
    lines.push(Line::raw(""));
    let today = jiff::Timestamp::now().to_zoned_datetime(app.tz.clone()).date();
    let done: Vec<_> = app
        .store
        .records
        .iter()
        .filter(|r| r.state == RecordState::Done)
        .filter(|r| {
            r.finished
                .is_some_and(|f| f.to_zoned_datetime(app.tz.clone()).date() == today)
        })
        .collect();
    let bytes: u64 = done.iter().map(|r| r.bytes).sum();
    let mut footer = format!("{running} active · {} done today", done.len());
    if bytes > 0 {
        footer.push_str(&format!(" · {}", human_size(bytes)));
    }
    lines.push(Line::styled(truncate_width(&footer, w), t.dim()));
    lines
}

fn row(t: &Theme, key: &str, value: String, width: u16) -> Line<'static> {
    let v = truncate_width(&value, (width as usize).saturating_sub(10));
    Line::from(vec![
        Span::styled(format!("{key:<10}"), t.dim()),
        Span::styled(v, t.secondary()),
    ])
}

fn selected_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let t = &app.theme;
    let pane = &app.pane;
    let mut lines = vec![Line::from(label(t, "SELECTED"))];
    if !pane.marks.is_empty() {
        let (count, size) = pane.marked_summary();
        lines.push(Line::from(vec![
            Span::styled(format!("{} ", bullet(app)), Style::default().fg(t.tokens.magenta)),
            Span::styled(
                format!("{count} marked · {}", human_size(size)),
                t.text().add_modifier(Modifier::BOLD),
            ),
        ]));
        return lines;
    }
    let Some(e) = pane.current() else {
        return lines;
    };
    let icon_color = if e.is_dir { t.tokens.blue } else { t.tokens.fg2 };
    lines.push(Line::from(vec![
        Span::styled(
            format!("{} ", icon(e, app.icons)),
            Style::default().fg(icon_color),
        ),
        Span::styled(
            truncate_width(&e.name, (width as usize).saturating_sub(2)),
            t.text().add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::raw(""));
    if e.is_dir {
        let items = scanned_files(app, e)
            .map(|n| format!("{} files", super::space::grouped(n)))
            .unwrap_or_else(|| "—".into());
        lines.push(row(t, "Items", items, width));
    } else {
        let size = e.size.map(human_size).unwrap_or_else(|| "—".into());
        lines.push(row(t, "Size", size, width));
    }
    if !e.mime.is_empty() && !e.is_dir {
        lines.push(row(t, "Type", e.mime.clone(), width));
    }
    if let Some(m) = e.mod_time {
        let s = m
            .to_zoned_datetime(app.tz.clone())
            .strftime("%Y-%m-%d %H:%M")
            .to_string();
        lines.push(row(t, "Modified", s, width));
    }
    if let Some(size) = e.size.filter(|_| !e.is_dir) {
        let total = pane.total_bytes();
        if total > 0 {
            let folder = if pane.loc.path.is_empty() {
                format!("{}:", remote_label(&pane.loc.remote))
            } else {
                format!("{}/", pane.loc.name())
            };
            let pct = size as f64 / total as f64 * 100.0;
            lines.push(row(t, "Folder", format!("{pct:.1}% of {folder}"), width));
        }
    }
    lines
}

fn transfer_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let t = &app.theme;
    let mut lines = vec![Line::from(label(t, "SELECTED"))];
    let order = app.transfer_records();
    let Some(r) = order.get(app.transfers.cursor.min(order.len().saturating_sub(1))) else {
        return lines;
    };
    lines.push(Line::styled(
        r.spec.title_fit(width as usize),
        t.text().add_modifier(Modifier::BOLD),
    ));
    let kind = match r.spec.kind {
        crate::transfer::spec::TransferKind::Copy => "copy",
        crate::transfer::spec::TransferKind::Move => "move",
    };
    lines.push(row(t, "Kind", kind.into(), width));
    lines.push(row(t, "From", r.spec.src_dir.display(), width));
    lines.push(row(t, "To", r.spec.dst_dir.display(), width));
    let total = if r.total_bytes > 0 {
        human_size(r.total_bytes)
    } else {
        "?".into()
    };
    lines.push(row(
        t,
        "Bytes",
        format!("{} / {total}", human_size(r.bytes)),
        width,
    ));
    let errors = app
        .progress
        .get(&r.spec.id)
        .map(|p| p.errors)
        .unwrap_or(u64::from(r.error.is_some()));
    if errors > 0 {
        lines.push(Line::from(vec![
            Span::styled(format!("{:<10}", "Errors"), t.dim()),
            Span::styled(errors.to_string(), Style::default().fg(t.tokens.red)),
        ]));
    } else {
        let started = crate::format::short_time_in(r.started, jiff::Timestamp::now(), &app.tz);
        lines.push(row(t, "Started", started, width));
    }
    lines
}

// ---- compact --------------------------------------------------------------

fn draw_compact(frame: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let mut spans = vec![Span::raw(" ")];
    if let Some((remote, _)) = account(app) {
        spans.push(Span::styled(
            remote_label(&remote),
            Style::default().fg(t.tokens.blue).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
        if let Some((a, _)) = about_of(app, &remote)
            && let (Some(used), Some(total)) = (a.used, a.total)
            && total > 0
        {
            let parts: Vec<(u64, Color)> = storage_parts(t, a).iter().map(|p| (p.0, p.1)).collect();
            spans.extend(stacked(&parts, total, 16, t.tokens.hl));
            let pct = (used as f64 / total as f64 * 100.0).round() as u64;
            spans.push(Span::styled(
                format!(" {pct}% · {}/{}", human_size(used), human_size(total)),
                t.secondary(),
            ));
        }
    }
    let speed = format!("↑{}/s ", human_size(app.speed.latest() as u64));
    let spark_w = 12usize;
    let spark = if app.icons == IconStyle::Ascii {
        ascii_spark(&app.speed.samples(), spark_w)
    } else {
        braille_rows(&app.speed.samples(), 1, spark_w).remove(0)
    };
    let used: usize = spans.iter().map(|s| s.content.width()).sum();
    let right_w = spark_w + 1 + speed.width();
    if used + right_w < area.width as usize {
        spans.push(Span::raw(" ".repeat(area.width as usize - used - right_w)));
        spans.push(Span::styled(spark, Style::default().fg(t.tokens.teal)));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(speed, Style::default().fg(t.tokens.teal)));
    }
    let style = Style::default().bg(t.tokens.bg2);
    frame.render_widget(Paragraph::new(Line::from(spans)).style(style), area);
}

/// Files under the Files-tab folder `e`, when a scan of its account covers it.
fn scanned_files(app: &App, e: &crate::backend::Entry) -> Option<u64> {
    let loc = app.pane.loc.join(&e.name);
    app.scans.values().find_map(|tree| {
        if tree.root.remote != loc.remote {
            return None;
        }
        let rel = if tree.root.path.is_empty() {
            loc.path.as_str()
        } else {
            loc.path.strip_prefix(&tree.root.path)?.strip_prefix('/')?
        };
        tree.node(rel).map(|n| n.files)
    })
}

// ---- space tab -------------------------------------------------------------

/// The panel for the Space tab's selected row; `false` when there is nothing to show.
fn draw_space(frame: &mut Frame, area: Rect, app: &App) -> bool {
    let t = &app.theme;
    let (Some(tree), Some(row)) = (app.space_tree(), app.space.shown.get(app.space.cursor)) else {
        return false;
    };
    let Some(here) = tree.node(&app.space.rel) else {
        return false;
    };
    let rel = if row.is_dir {
        if app.space.rel.is_empty() {
            row.name.clone()
        } else {
            format!("{}/{}", app.space.rel, row.name)
        }
    } else {
        app.space.rel.clone()
    };
    let node = if row.is_dir { tree.node(&rel) } else { None };
    let w = area.width as usize;

    // Separator: the row's name, then the keys.
    let gutter = Style::default().fg(t.tokens.gutter);
    let mut sep = vec![Span::styled("──", gutter)];
    match app.scan.as_ref().filter(|a| a.tree.root == tree.root) {
        // While scanning, the title says so (spec §8.4).
        Some(a) => {
            sep.push(Span::styled(
                format!(" {} ", super::spinner(app.spinner)),
                Style::default().fg(t.tokens.blue),
            ));
            sep.push(Span::styled(
                format!("scanning {} ", account_label(&tree.root.remote)),
                Style::default().fg(t.tokens.blue).add_modifier(Modifier::BOLD),
            ));
            sep.push(Span::styled(
                format!(
                    "· {} · {} ",
                    counted(a.tree.listed, "folder"),
                    counted(a.tree.node.files, "file")
                ),
                t.dim(),
            ));
        }
        None => {
            sep.push(Span::styled(
                format!(" {} ", truncate_width(&row.name, 40)),
                Style::default().fg(t.tokens.blue).add_modifier(Modifier::BOLD),
            ));
            sep.push(Span::styled("selected ", t.dim()));
        }
    }
    let keys = " l open · h up · R rescan ";
    let used: usize = sep.iter().map(|s| s.content.width()).sum();
    if used + keys.len() + 2 <= w {
        sep.push(Span::styled("─".repeat(w - used - keys.len() - 2), gutter));
        sep.push(Span::styled(keys, t.dim()));
        sep.push(Span::styled("──", gutter));
    }
    frame.render_widget(Paragraph::new(Line::from(sep)), Rect { height: 1, ..area });

    let body = Rect {
        x: area.x + 2,
        y: area.y + 1,
        width: area.width.saturating_sub(4),
        height: area.height.saturating_sub(1),
    };
    let account = remote_label(&tree.root.remote);
    let share = super::super::app::space::share(row.bytes, tree.node.bytes) * 100.0;
    let mut summary = vec![
        Span::styled(
            format!("{} ", truncate_width(&row.name, 40)),
            Style::default().fg(t.tokens.blue).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                "{} · {} {}",
                human_size(row.bytes),
                super::space::grouped(row.files),
                if row.files == 1 { "file" } else { "files" }
            ),
            t.secondary(),
        ),
    ];
    if let Some(n) = node {
        let d = n.dirs;
        summary.push(Span::styled(
            format!(
                " · {} {}",
                super::space::grouped(d),
                if d == 1 { "folder" } else { "folders" }
            ),
            t.secondary(),
        ));
    }
    summary.push(Span::styled(format!(" · {share:.0}% of {account}:/"), t.dim()));
    let mut left = vec![Line::from(summary), Line::raw("")];
    let half = (body.width / 2).max(20);

    // BY TYPE (folders only: loose files have no breakdown of their own).
    if let Some(n) = node.filter(|n| n.bytes > 0) {
        left.push(Line::from(label(t, "BY TYPE")));
        let mut cats: Vec<(crate::scan::tree::Category, u64)> = n
            .by_type
            .iter()
            .map(|(c, v)| (*c, *v))
            .filter(|(_, v)| *v > 0)
            .collect();
        cats.sort_by_key(|c| std::cmp::Reverse(c.1));
        let parts: Vec<(u64, Color)> = cats.iter().map(|(c, v)| (*v, category_color(t, *c))).collect();
        let bar_w = (half as usize).saturating_sub(4).min(44);
        left.push(Line::from(stacked(&parts, n.bytes, bar_w, t.tokens.hl)));
        let mut items: Vec<(Color, &str, String)> = cats
            .iter()
            .take(3)
            .map(|(c, v)| (category_color(t, *c), c.label(), short_size(*v)))
            .collect();
        let rest: u64 = cats.iter().skip(3).map(|(_, v)| v).sum();
        if rest > 0 {
            items.push((t.tokens.comment, "Other", short_size(rest)));
        }
        let mut legend_line = Vec::new();
        for (i, (c, l, v)) in items.iter().enumerate() {
            if i > 0 {
                legend_line.push(Span::raw("  "));
            }
            legend_line.push(Span::styled(format!("{} ", bullet(app)), Style::default().fg(*c)));
            legend_line.push(Span::styled(format!("{l} "), t.dim()));
            legend_line.push(Span::styled(v.clone(), t.secondary()));
        }
        left.push(Line::from(legend_line));
    }
    frame.render_widget(Paragraph::new(left), body);

    // LARGEST FILES, on the right half.
    let largest: Vec<(String, u64)> = match node {
        Some(n) => n.largest.clone(),
        None => here.loose_largest.clone(),
    };
    if !largest.is_empty() && body.height > 2 {
        let right = Rect {
            x: body.x + half + 2,
            y: body.y + 2,
            width: body.width.saturating_sub(half + 2),
            height: body.height - 2,
        };
        let rw = right.width as usize;
        let mut lines = vec![Line::from(label(t, "LARGEST FILES"))];
        for (path, size) in largest.iter().take(right.height.saturating_sub(1) as usize) {
            let size = human_size(*size);
            let room = rw.saturating_sub(size.width() + 2);
            let shown = truncate_left(path, room);
            let gap = rw.saturating_sub(shown.width() + size.width());
            lines.push(Line::from(vec![
                Span::styled(shown, t.secondary()),
                Span::raw(" ".repeat(gap)),
                Span::styled(size, t.text()),
            ]));
        }
        frame.render_widget(Paragraph::new(lines), right);
    }
    true
}

/// Keep the end of a path: `…/2025/video.mp4`.
fn truncate_left(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    let mut out: Vec<char> = Vec::new();
    let mut w = 1;
    for c in s.chars().rev() {
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if w + cw > width {
            break;
        }
        w += cw;
        out.push(c);
    }
    out.push('…');
    out.iter().rev().collect()
}

pub fn category_color(t: &Theme, c: crate::scan::tree::Category) -> Color {
    use crate::scan::tree::Category::*;
    let k = &t.tokens;
    match c {
        Images => k.yellow,
        Video => k.magenta,
        Audio => k.cyan,
        Documents => k.green,
        Archives => k.orange,
        Code => k.teal,
        Other => k.comment,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn left_truncation_keeps_the_end() {
        assert_eq!(truncate_left("2025/video_beach.mp4", 50), "2025/video_beach.mp4");
        assert_eq!(truncate_left("2025/video_beach.mp4", 10), "…beach.mp4");
    }

    #[test]
    fn counted_plurals() {
        assert_eq!(counted(1, "folder"), "1 folder");
        assert_eq!(counted(12_345, "file"), "12 345 files");
    }

    #[test]
    fn short_sizes() {
        assert_eq!(short_size(800 * 1024 * 1024), "800 MiB");
        assert_eq!(short_size(4 * 1024 * 1024 * 1024), "4.0 GiB");
        assert_eq!(short_size(12), "12 B");
    }
}
