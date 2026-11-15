//! The file list.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};
use unicode_width::UnicodeWidthStr;

use super::icons::icon;
use super::{spinner, text};
use crate::app::pane::Listing;
use crate::app::{App, Focus};
use crate::backend::Entry;
use crate::format::{human_size, short_time_in, truncate_width};
use crate::transfer::spec::TransferKind;

const SIZE_W: usize = 9;
const DATE_W: usize = 10;
/// Columns before the name: padding, mark, icon, space.
const LEAD_W: usize = 4;

struct Columns {
    name: usize,
    size: bool,
    date: bool,
}

fn columns(width: usize) -> Columns {
    let date = width >= 50;
    let size = width >= 34;
    let mut used = LEAD_W + 1; // + scrollbar column
    if size {
        used += 2 + SIZE_W;
    }
    if date {
        used += 2 + DATE_W;
    }
    Columns {
        name: width.saturating_sub(used).max(4),
        size,
        date,
    }
}

fn pad_right(s: &str, w: usize) -> String {
    let s = truncate_width(s, w);
    let fill = w.saturating_sub(s.width());
    format!("{s}{}", " ".repeat(fill))
}

fn pad_left(s: &str, w: usize) -> String {
    let fill = w.saturating_sub(s.width());
    format!("{}{s}", " ".repeat(fill))
}

/// Which of the two panes to draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Focused,
    Other,
}

/// Draw `app.pane`. `active` is false for the unfocused one of two panes (drawn
/// while it stands in as `app.pane`): its cursor row never shows as focused.
pub fn draw(frame: &mut Frame, area: Rect, app: &mut App, active: bool) {
    if area.height == 0 {
        return;
    }
    let t = app.theme;
    let cols = columns(area.width as usize);

    // Column header.
    let mut header = vec![
        Span::raw(" ".repeat(LEAD_W)),
        Span::raw(pad_right(text::COL_NAME, cols.name)),
    ];
    if cols.size {
        header.push(Span::raw(format!("  {}", pad_left(text::COL_SIZE, SIZE_W))));
    }
    if cols.date {
        header.push(Span::raw(format!("  {}", pad_left(text::COL_MODIFIED, DATE_W))));
    }
    let header_area = Rect { height: 1, ..area };
    frame.render_widget(Paragraph::new(Line::from(header)).style(t.dim()), header_area);

    let list = Rect {
        y: area.y + 1,
        height: area.height - 1,
        ..area
    };
    let rows = list.height as usize;
    app.pane.viewport_height = rows.max(1);

    let state_line =
        |msg: Line<'static>| Paragraph::new(vec![Line::raw(""), msg]).alignment(Alignment::Center);
    match &app.pane.listing {
        Listing::Loading => {
            let msg = Line::from(vec![
                Span::styled(
                    format!("{} ", spinner(app.spinner)),
                    Style::default().fg(t.tokens.blue),
                ),
                Span::styled(text::LOADING, t.dim()),
            ]);
            frame.render_widget(state_line(msg), list);
            return;
        }
        Listing::Failed(err) => {
            let p = Paragraph::new(vec![
                Line::raw(""),
                Line::styled(format!("✗ {err}"), Style::default().fg(t.tokens.red)),
                Line::styled(text::RETRY, t.dim()),
            ])
            .alignment(Alignment::Center)
            .wrap(ratatui::widgets::Wrap { trim: true });
            frame.render_widget(p, list);
            return;
        }
        Listing::Ready if app.pane.is_empty() => {
            let msg = if app.pane.filter.is_empty() {
                text::EMPTY
            } else {
                text::NO_MATCH
            };
            frame.render_widget(state_line(Line::styled(msg, t.dim())), list);
            return;
        }
        Listing::Ready => {}
    }

    app.pane.ensure_visible(rows);
    let pane = &app.pane;
    let focused = active && app.focus == Focus::Pane;
    let now = jiff::Timestamp::now();
    let visual = pane
        .visual_anchor
        .map(|a| (a.min(pane.cursor), a.max(pane.cursor)));
    let register = app.register.as_ref();

    let end = (pane.offset + rows).min(pane.len());
    for (i, e) in (pane.offset..end).filter_map(|i| pane.entry(i).map(|e| (i, e))) {
        let in_register = register.filter(|r| r.contains(&pane.loc, &e.name));
        let mut name_style = name_style(&t, e);
        if let Some(r) = in_register {
            let fg = if r.kind == TransferKind::Copy {
                t.tokens.yellow
            } else {
                t.tokens.orange
            };
            name_style = name_style.fg(fg).add_modifier(Modifier::ITALIC);
        }
        let row_bg = if i == pane.cursor {
            Some(if focused { t.tokens.sel } else { t.tokens.hl })
        } else if visual.is_some_and(|(lo, hi)| (lo..=hi).contains(&i)) {
            Some(t.tokens.hl)
        } else {
            None
        };
        let row = Rect {
            y: list.y + (i - pane.offset) as u16,
            height: 1,
            ..list
        };
        // Cells are written straight into the buffer and padding is skipped
        // rather than drawn as spaces: laying out text is most of a frame's cost.
        let buf = frame.buffer_mut();
        if let Some(c) = row_bg {
            buf.set_style(row, Style::default().bg(c));
        }
        let mut pen = Pen { x: row.x, row };
        pen.skip(1);
        if pane.marks.contains(&e.name) {
            pen.put(buf, "●", Style::default().fg(t.tokens.magenta));
        } else {
            pen.skip(1);
        }
        pen.put(buf, icon(e, app.icons), icon_style(&t, e));
        pen.skip(1);
        pen.put_left(buf, &truncate_width(&e.name, cols.name), cols.name, name_style);
        if cols.size {
            let size = match (e.is_dir, e.size) {
                (true, _) => String::new(),
                (false, Some(s)) => human_size(s),
                (false, None) => "–".into(),
            };
            pen.skip(2);
            pen.put_right(buf, &size, SIZE_W, t.secondary());
        }
        if cols.date {
            let date = e
                .mod_time
                .map(|m| short_time_in(m, now, &app.tz))
                .unwrap_or_default();
            pen.skip(2);
            pen.put_right(buf, &date, DATE_W, t.dim());
        }
    }

    if pane.len() > rows {
        let mut state = ScrollbarState::new(pane.len().saturating_sub(rows)).position(pane.offset);
        let bar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some(" "))
            .thumb_symbol("▐")
            .thumb_style(Style::default().fg(t.tokens.comment));
        frame.render_stateful_widget(bar, list, &mut state);
    }
}

/// Writes a row left to right, clipped to the row.
struct Pen {
    x: u16,
    row: Rect,
}

impl Pen {
    fn skip(&mut self, cells: usize) {
        self.x = self.x.saturating_add(cells as u16);
    }

    fn put(&mut self, buf: &mut Buffer, s: &str, style: Style) {
        let room = self.row.right().saturating_sub(self.x) as usize;
        if room > 0 {
            buf.set_stringn(self.x, self.row.y, s, room, style);
        }
        self.skip(s.width());
    }

    /// `s` then blanks up to `w` cells (`s` is already cut to fit).
    fn put_left(&mut self, buf: &mut Buffer, s: &str, w: usize, style: Style) {
        let start = self.x;
        self.put(buf, s, style);
        self.x = self.x.max(start.saturating_add(w as u16));
    }

    /// Blanks, then `s` ending at `w` cells (a longer `s` overflows, as with padding).
    fn put_right(&mut self, buf: &mut Buffer, s: &str, w: usize, style: Style) {
        self.skip(w.saturating_sub(s.width()));
        self.put(buf, s, style);
    }
}

fn name_style(t: &crate::theme::Theme, e: &Entry) -> Style {
    if e.is_dir {
        Style::default().fg(t.tokens.blue).add_modifier(Modifier::BOLD)
    } else if e.name.starts_with('.') {
        t.dim()
    } else {
        t.text()
    }
}

fn icon_style(t: &crate::theme::Theme, e: &Entry) -> Style {
    if e.is_dir {
        Style::default().fg(t.tokens.blue)
    } else {
        Style::default().fg(t.tokens.fg2)
    }
}
