//! Modal dialogs: confirm, text input, paste conflicts, quit, help.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::app::overlay::{InputPurpose, LineInput, Overlay};
use crate::app::paste::{Resolution, unique_name_lower};
use crate::format::truncate_width;
use crate::input::keymap::{Binding, KeyContext, bindings, tokenize};
use crate::theme::Theme;

/// A centred box `w` columns wide and `h` rows tall, never larger than the screen.
fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(4)).max(10).min(area.width);
    let h = h.min(area.height.saturating_sub(2)).max(3).min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 3,
        width: w,
        height: h,
    }
}

/// Below this no dialog is readable: a short note says so instead.
const MIN_DIALOG_W: u16 = 24;
const MIN_DIALOG_H: u16 = 5;

fn too_small(frame: &mut Frame, area: Rect, t: &Theme) {
    frame.render_widget(Clear, area);
    let p = Paragraph::new(vec![
        Line::styled("Window too small", t.text()),
        Line::from(vec![
            Span::styled("Esc", t.key()),
            Span::styled(" close", t.dim()),
        ]),
    ])
    .wrap(Wrap { trim: true })
    .style(Style::default().bg(t.tokens.bg2));
    frame.render_widget(p, area);
}

fn frame_block<'a>(t: &Theme, title: &'a str, danger: bool) -> Block<'a> {
    let accent = if danger { t.tokens.red } else { t.tokens.blue };
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(accent).bg(t.tokens.bg2))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ))
        .padding(Padding::horizontal(1))
        .style(Style::default().fg(t.tokens.fg).bg(t.tokens.bg2))
}

/// `key label · key label` footer.
fn keys_line<'a>(t: &Theme, keys: &[(&'a str, &'a str)]) -> Line<'a> {
    let mut spans = Vec::new();
    for (i, (k, label)) in keys.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ·  ", t.dim()));
        }
        spans.push(Span::styled(*k, t.key()));
        spans.push(Span::styled(format!(" {label}"), t.secondary()));
    }
    Line::from(spans)
}

fn show(frame: &mut Frame, rect: Rect, block: Block, lines: Vec<Line>) {
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines).block(block).wrap(Wrap { trim: false }),
        rect,
    );
}

/// Like [`show`], centred and as tall as its lines once wrapped to the box's width,
/// so a long line never pushes the last ones (the keys) out of the box.
fn show_fitted(frame: &mut Frame, area: Rect, w: u16, block: Block, lines: Vec<Line>) {
    let width = centered(area, w, 3).width;
    // Two border columns and one column of padding on each side.
    let rows = Paragraph::new(lines.clone())
        .wrap(Wrap { trim: false })
        .line_count(width.saturating_sub(4));
    let rect = centered(area, w, (rows as u16).saturating_add(2));
    show(frame, rect, block, lines);
}

pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let Some(overlay) = &app.overlay else { return };
    // The settings and account dialogs say this themselves; the filter has no box.
    let boxed = !matches!(
        overlay,
        Overlay::Settings(_)
            | Overlay::Account(_)
            | Overlay::Input {
                purpose: InputPurpose::Filter,
                ..
            }
    );
    if boxed && (area.width < MIN_DIALOG_W || area.height < MIN_DIALOG_H) {
        return too_small(frame, area, t);
    }
    match overlay {
        Overlay::Input {
            purpose: InputPurpose::Filter,
            ..
        } => {}
        Overlay::Input { title, input, .. } => {
            // Box borders and padding take four columns.
            let inner_w = 56usize.min((area.width as usize).saturating_sub(4));
            let lines = vec![
                Line::raw(""),
                input_line(t, input, inner_w),
                Line::raw(""),
                keys_line(t, &[("Enter", "save"), ("Esc", "cancel")]),
            ];
            let rect = centered(area, inner_w as u16 + 4, lines.len() as u16 + 2);
            show(frame, rect, frame_block(t, title, false), lines);
        }
        Overlay::Confirm {
            title,
            lines: names,
            commands,
            danger,
            ..
        } => {
            let mut lines = vec![Line::raw("")];
            for n in names {
                let style = if n.starts_with('…') { t.dim() } else { t.text() };
                lines.push(Line::from(vec![
                    Span::styled("  ", t.dim()),
                    Span::styled(n.clone(), style),
                ]));
            }
            lines.push(Line::raw(""));
            for c in commands {
                lines.push(Line::from(vec![
                    Span::styled("$ ", t.dim()),
                    Span::styled(c.clone(), Style::default().fg(t.tokens.comment)),
                ]));
            }
            if !commands.is_empty() {
                lines.push(Line::raw(""));
            }
            let yes = if *danger { "delete" } else { "yes" };
            lines.push(keys_line(t, &[("y", yes), ("n", "cancel")]));
            let w = commands
                .iter()
                .map(|c| c.width() + 6)
                .chain(names.iter().map(|n| n.width() + 6))
                .max()
                .unwrap_or(0)
                .clamp(48, 90);
            show_fitted(frame, area, w as u16, frame_block(t, title, *danger), lines);
        }
        Overlay::Conflict {
            names,
            index,
            apply_all,
            default,
            pending,
            ..
        } => {
            let name = names.get(*index).cloned().unwrap_or_default();
            let is_dir = pending.register.items.iter().any(|e| e.name == name && e.is_dir);
            let renamed = unique_name_lower(&name, is_dir, &pending.taken_lower);
            let remaining = names.len() - index;
            let title = if names.len() == 1 {
                "Name already in use".to_string()
            } else {
                format!("Name already in use ({} of {})", index + 1, names.len())
            };
            let opt = |key: &'static str, label: String, r: Resolution| {
                let chosen = *default == r;
                let mark = if chosen { "› " } else { "  " };
                let style = if chosen {
                    t.text().add_modifier(Modifier::BOLD)
                } else {
                    t.secondary()
                };
                Line::from(vec![
                    Span::styled(mark, Style::default().fg(t.tokens.blue)),
                    Span::styled(key, t.key()),
                    Span::styled(format!("  {label}"), style),
                ])
            };
            let mut lines = vec![
                Line::raw(""),
                Line::from(vec![
                    Span::styled(
                        format!("“{}”", truncate_width(&name, 50)),
                        t.text().add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(" already exists in ", t.secondary()),
                ]),
                Line::styled(pending.dst_dir.display(), Style::default().fg(t.tokens.blue)),
                Line::raw(""),
                if app.conflict_clash() {
                    Line::styled(
                        "   r  replace — not possible, one is a file and the other a folder",
                        t.dim(),
                    )
                } else {
                    opt("r", "replace it".into(), Resolution::Replace)
                },
                opt("s", "skip this one".into(), Resolution::Skip),
                opt(
                    "n",
                    format!("keep both, name it “{}”", truncate_width(&renamed, 40)),
                    Resolution::Rename,
                ),
                Line::raw(""),
            ];
            if remaining > 1 {
                let state = if *apply_all { "on" } else { "off" };
                lines.push(Line::from(vec![
                    Span::styled("  a", t.key()),
                    Span::styled(
                        format!("  same choice for the remaining {remaining}: "),
                        t.secondary(),
                    ),
                    Span::styled(
                        state,
                        if *apply_all {
                            Style::default().fg(t.tokens.green)
                        } else {
                            t.dim()
                        },
                    ),
                ]));
                lines.push(Line::raw(""));
            }
            if is_dir {
                lines.push(Line::styled(
                    "Inside folders rclone skips identical files and overwrites different ones.",
                    t.dim(),
                ));
                lines.push(Line::raw(""));
            }
            let enter = match default {
                Resolution::Replace => "replace",
                Resolution::Skip => "skip",
                Resolution::Rename => "keep both",
            };
            lines.push(keys_line(t, &[("Enter", enter), ("Esc", "cancel paste")]));
            let rect = centered(area, 66, lines.len() as u16 + 2);
            show(frame, rect, frame_block(t, &title, false), lines);
        }
        Overlay::Quit { running, ops } => {
            let transfers = running.saturating_sub(*ops);
            let row = |k: &'static str, label: String| {
                Line::from(vec![
                    Span::styled(format!("  {k}"), t.key()),
                    Span::styled(format!("  {label}"), t.text()),
                ])
            };
            let mut lines = vec![Line::raw("")];
            if transfers > 0 {
                let what = if transfers == 1 {
                    "1 transfer is".to_string()
                } else {
                    format!("{transfers} transfers are")
                };
                lines.push(Line::styled(format!("{what} still running."), t.text()));
            }
            if *ops > 0 {
                let what = if *ops == 1 {
                    "1 file operation (delete, rename…) is".to_string()
                } else {
                    format!("{ops} file operations (delete, rename…) are")
                };
                lines.push(Line::styled(format!("{what} in progress."), t.text()));
                lines.push(Line::styled(
                    "The app stays open until they end, whichever you choose.",
                    t.dim(),
                ));
            }
            let them = if transfers == 1 { "it" } else { "them" };
            lines.push(Line::raw(""));
            if transfers > 0 {
                lines.push(row(
                    "b",
                    format!("quit and keep {them} running in the background"),
                ));
                lines.push(row("w", format!("wait for {them} to finish, then quit")));
                lines.push(row("c", format!("cancel {them} and quit")));
                lines.push(Line::raw(""));
                lines.push(Line::styled(
                    "Background transfers show up again the next time you open the app.",
                    t.dim(),
                ));
            } else {
                lines.push(row("b", "quit once they end".into()));
                lines.push(row("w", "quit once they end".into()));
            }
            lines.push(Line::raw(""));
            lines.push(keys_line(t, &[("Esc", "stay")]));
            let rect = centered(area, 72, lines.len() as u16 + 2);
            show(frame, rect, frame_block(t, "Quit", false), lines);
        }
        Overlay::Help { scroll } => draw_help(frame, area, t, *scroll),
        Overlay::Settings(dlg) => super::settings::draw(frame, area, app, dlg),
        Overlay::Account(w) => super::account::draw(frame, area, app, w),
    }
}

/// A one-line field `width` columns wide; a long value scrolls with the cursor.
fn input_line<'a>(t: &Theme, input: &LineInput, width: usize) -> Line<'a> {
    let (before, at, after) = crate::format::edit_window(&input.value, input.cursor, width);
    let used = before.width() + at.width() + after.width();
    let field = Style::default().fg(t.tokens.fg).bg(t.tokens.hl);
    Line::from(vec![
        Span::styled(before, field),
        Span::styled(at, Style::default().fg(t.tokens.bg).bg(t.tokens.fg)),
        Span::styled(after, field),
        Span::styled(" ".repeat(width.saturating_sub(used)), field),
    ])
}

/// `<C-d>` → `Ctrl-d`, `<Down>` → `↓`, `gg` → `gg`.
pub fn pretty_key(label: &str) -> String {
    tokenize(label)
        .iter()
        .map(|tok| match tok.as_str() {
            "<Down>" => "↓".to_string(),
            "<Up>" => "↑".to_string(),
            "<Left>" => "←".to_string(),
            "<Right>" => "→".to_string(),
            "<CR>" => "Enter".to_string(),
            "<BS>" => "Bksp".to_string(),
            "<Space>" => "Space".to_string(),
            "<Tab>" => "Tab".to_string(),
            "<Esc>" => "Esc".to_string(),
            "<S-Tab>" => "Shift-Tab".to_string(),
            t if t.starts_with("<F") => t[1..t.len() - 1].to_string(),
            t if t.starts_with("<C-") => format!("Ctrl-{}", &t[3..t.len() - 1]),
            t if t.starts_with("<A-") => format!("Alt-{}", &t[3..t.len() - 1]),
            t => t.to_string(),
        })
        .collect()
}

/// Bindings grouped by section, with every key for the same description joined.
fn help_rows(list: Vec<Binding>) -> Vec<(&'static str, Vec<(String, &'static str)>)> {
    let mut groups: Vec<(&'static str, Vec<(String, &'static str)>)> = Vec::new();
    for (group, keys, desc, _) in list {
        if !groups.iter().any(|(g, _)| *g == group) {
            groups.push((group, Vec::new()));
        }
        let rows = &mut groups.iter_mut().find(|(g, _)| *g == group).unwrap().1;
        match rows.iter_mut().find(|(_, d)| *d == desc) {
            Some((k, _)) => {
                k.push(' ');
                k.push_str(&pretty_key(keys));
            }
            None => rows.push((pretty_key(keys), desc)),
        }
    }
    groups
}

const ACCOUNTS_HELP: &str = "Accounts";

/// Groups of the help screen: (title, rows of key and description).
type HelpGroups = Vec<(&'static str, Vec<(String, &'static str)>)>;

/// Everything the help screen lists: the file keys, plus those that only work in the
/// accounts list, the Transfers tab or the Space tab (saying where).
fn help_groups() -> (HelpGroups, HelpGroups) {
    let mut pane = help_rows(bindings(KeyContext::Pane));
    for (g, rows) in help_rows(bindings(KeyContext::Transfers)) {
        if g == "Transfers" {
            let mut rows = rows;
            rows.retain(|(_, d)| !d.contains("close"));
            if let Some((_, existing)) = pane.iter_mut().find(|(name, _)| *name == "Transfers") {
                existing.extend(rows);
            }
        }
    }
    // The help opens over the files, but the account keys only work from the
    // accounts list: show them too, saying how to get there.
    for (g, rows) in help_rows(bindings(KeyContext::Sidebar)) {
        if g == "Accounts" {
            pane.push((ACCOUNTS_HELP, rows));
        }
    }
    // Space-tab keys the file list does not have (or that do something else there).
    let known: Vec<(String, &'static str)> = pane.iter().flat_map(|(_, r)| r.iter().cloned()).collect();
    let space: Vec<(String, &'static str)> = help_rows(bindings(KeyContext::Space))
        .into_iter()
        .filter(|(g, _)| *g != "Tabs")
        .flat_map(|(_, rows)| rows)
        .filter(|row| !known.contains(row))
        .collect();
    pane.push((SPACE_HELP, space));
    pane.into_iter()
        .partition(|(g, _)| *g == "Navigation" || *g == ACCOUNTS_HELP)
}

const SPACE_HELP: &str = "Space tab (U)";

fn help_column(t: &Theme, groups: &[(&'static str, Vec<(String, &'static str)>)]) -> Vec<Line<'static>> {
    // Keys sit left-aligned, indented under their section title, as wide as the widest key of
    // the column, so titles, keys and descriptions each start on one edge.
    let key_w = groups
        .iter()
        .flat_map(|(_, rows)| rows.iter().map(|(k, _)| k.width()))
        .max()
        .unwrap_or(0)
        .min(14);
    let mut lines = Vec::new();
    for (g, rows) in groups {
        lines.push(Line::styled(
            g.to_string(),
            Style::default().fg(t.tokens.blue).add_modifier(Modifier::BOLD),
        ));
        for (k, d) in rows {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("{k:<key_w$}  ", k = truncate_width(k, key_w)), t.key()),
                Span::styled(d.to_string(), t.secondary()),
            ]));
        }
        lines.push(Line::raw(""));
    }
    lines
}

/// Inner width from which the help shows two columns side by side.
const HELP_TWO_COLUMNS: u16 = 76;
const HELP_W: u16 = 84;

/// How the help fits `area`: its box, the columns' lines, and how far it scrolls.
struct HelpLayout {
    rect: Rect,
    left: Vec<Line<'static>>,
    right: Option<Vec<Line<'static>>>,
    max_scroll: usize,
}

fn help_layout(area: Rect, t: &Theme) -> HelpLayout {
    let (left, right) = help_groups();
    let (left, right) = (help_column(t, &left), help_column(t, &right));
    let probe = centered(area, HELP_W, u16::MAX);
    let two = probe.width.saturating_sub(4) >= HELP_TWO_COLUMNS;
    let (left, right) = if two {
        (left, Some(right))
    } else {
        let mut all = left;
        all.extend(right);
        (all, None)
    };
    let total = left.len().max(right.as_ref().map_or(0, Vec::len));
    let rect = centered(area, HELP_W, (total as u16).saturating_add(3));
    // Borders and the hint line take 2 rows.
    let visible = rect.height.saturating_sub(2) as usize;
    HelpLayout {
        rect,
        left,
        right,
        max_scroll: total.saturating_sub(visible),
    }
}

/// How far the help can scroll in `area`, and how many rows a page is.
pub fn help_extent(area: Rect, t: &Theme) -> (usize, usize) {
    let l = help_layout(area, t);
    (l.max_scroll, (l.rect.height.saturating_sub(2) as usize).max(1))
}

fn draw_help(frame: &mut Frame, area: Rect, t: &Theme, scroll: usize) {
    let l = help_layout(area, t);
    let scroll = scroll.min(l.max_scroll);
    frame.render_widget(Clear, l.rect);
    let mut hint = vec![];
    if l.max_scroll > 0 {
        hint.push(Span::styled(" j/k ", t.key()));
        hint.push(Span::styled("scroll ", t.dim()));
        hint.push(Span::styled(format!("{}/{} ", scroll, l.max_scroll), t.dim()));
    }
    hint.push(Span::styled(" Esc ", t.key()));
    hint.push(Span::styled("closes ", t.dim()));
    let block = frame_block(t, "Keys", false).title_bottom(Line::from(hint).right_aligned());
    let inner = block.inner(l.rect);
    frame.render_widget(block, l.rect);
    let skip = |lines: Vec<Line<'static>>| -> Vec<Line<'static>> { lines.into_iter().skip(scroll).collect() };
    match l.right {
        Some(right) => {
            let half = inner.width / 2;
            let left_area = Rect { width: half, ..inner };
            let right_area = Rect {
                x: inner.x + half,
                width: inner.width - half,
                ..inner
            };
            frame.render_widget(Paragraph::new(skip(l.left)), left_area);
            frame.render_widget(Paragraph::new(skip(right)), right_area);
        }
        None => frame.render_widget(Paragraph::new(skip(l.left)), inner),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretty_keys() {
        assert_eq!(pretty_key("<C-d>"), "Ctrl-d");
        assert_eq!(pretty_key("<A-2>"), "Alt-2");
        assert_eq!(pretty_key("<Down>"), "↓");
        assert_eq!(pretty_key("gg"), "gg");
        assert_eq!(pretty_key("<Space>"), "Space");
        assert_eq!(pretty_key("<F5>"), "F5");
        assert_eq!(pretty_key("<S-Tab>"), "Shift-Tab");
    }

    /// Every key of every context is on the help screen, with its description — bar the
    /// accounts-list and Transfers variants of keys the help already shows for the files.
    #[test]
    fn help_lists_every_binding() {
        let variants = [
            "first account",
            "last account",
            "open account",
            "switch to files",
            "clear",
            "close",
        ];
        let (left, right) = help_groups();
        let rows: Vec<(String, &str)> = left.into_iter().chain(right).flat_map(|(_, r)| r).collect();
        let mut missing = Vec::new();
        for ctx in [
            KeyContext::Pane,
            KeyContext::Sidebar,
            KeyContext::Transfers,
            KeyContext::Space,
        ] {
            for (_, keys, desc, _) in bindings(ctx) {
                let key = pretty_key(keys);
                let shown = rows
                    .iter()
                    .any(|(k, d)| *d == desc && k.split(' ').any(|t| t == key));
                if !shown && !variants.contains(&desc) {
                    missing.push(format!("{ctx:?}: {key} {desc}"));
                }
            }
        }
        assert!(missing.is_empty(), "missing from help: {missing:#?}");
    }
}
