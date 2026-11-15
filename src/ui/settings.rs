//! The settings dialog (`,`), drawn over the app so theme changes show on the real thing.

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::app::settings::{SettingId, SettingsDialog, display_value};
use crate::format::truncate_width;

const MAX_W: u16 = 64;
const MAX_H: u16 = 22;
const MIN_W: u16 = 40;
const MIN_H: u16 = 12;
/// Column where values start, inside the border.
const VALUE_X: usize = 26;

pub fn draw(frame: &mut Frame, area: Rect, app: &App, dlg: &SettingsDialog) {
    let t = &app.theme;
    if area.width < MIN_W || area.height < MIN_H {
        frame.render_widget(Clear, area);
        let p = Paragraph::new(vec![
            Line::raw(""),
            Line::styled("Terminal too small for settings", t.text()),
            Line::from(vec![
                Span::styled("Esc", t.key()),
                Span::styled(" close", t.dim()),
            ]),
        ])
        .alignment(Alignment::Center)
        .style(Style::default().bg(t.tokens.bg2));
        frame.render_widget(p, area);
        return;
    }
    let w = MAX_W.min(area.width);
    let h = MAX_H.min(area.height);
    let rect = Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 3,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, rect);
    let footer = Line::from(vec![
        Span::styled(" ↵", t.key()),
        Span::styled(" save · ", t.dim()),
        Span::styled("Esc", t.key()),
        Span::styled(" cancel ", t.dim()),
    ]);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(t.tokens.blue).bg(t.tokens.bg2))
        .title(Span::styled(
            " Settings ",
            Style::default().fg(t.tokens.blue).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(footer.right_aligned())
        .style(Style::default().fg(t.tokens.fg).bg(t.tokens.bg2));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let iw = inner.width as usize;
    // Rows: sections and settings, then a rule and the description (2 rows at the bottom).
    let mut rows: Vec<Line> = Vec::new();
    let mut cursor_row = 0;
    let mut section = "";
    for (i, id) in SettingId::ALL.iter().enumerate() {
        if id.section() != section {
            if !section.is_empty() {
                rows.push(Line::raw(""));
            }
            section = id.section();
            rows.push(Line::styled(
                format!(" {section}"),
                t.dim().add_modifier(Modifier::BOLD),
            ));
        }
        let cur = i == dlg.cursor;
        if cur {
            cursor_row = rows.len();
        }
        let value = display_value(*id, &dlg.values, &dlg.themes, &app.sidebar.remotes);
        let changed = value != display_value(*id, &dlg.saved, &dlg.themes, &app.sidebar.remotes);
        let label = format!(" {} {}", if cur { "▸" } else { " " }, id.label());
        let pad = VALUE_X.saturating_sub(label.width());
        let mut value_style = if changed {
            Style::default().fg(t.tokens.yellow).add_modifier(Modifier::BOLD)
        } else {
            t.secondary()
        };
        let room = iw.saturating_sub(VALUE_X + 4);
        let value = truncate_width(&value, room);
        let mut spans = vec![
            Span::styled(
                label,
                if cur {
                    t.text().add_modifier(Modifier::BOLD)
                } else {
                    t.secondary()
                },
            ),
            Span::raw(" ".repeat(pad)),
        ];
        if cur {
            if !changed {
                value_style = Style::default().fg(t.tokens.blue);
            }
            spans.push(Span::styled("‹ ", Style::default().fg(t.tokens.blue)));
            spans.push(Span::styled(value, value_style));
            spans.push(Span::styled(" ›", Style::default().fg(t.tokens.blue)));
        } else {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(value, value_style));
        }
        let mut line = Line::from(spans);
        if cur {
            line = line.style(Style::default().bg(t.tokens.sel));
        }
        rows.push(line);
    }

    let list_h = inner.height.saturating_sub(2) as usize;
    // Keep the cursor visible when the dialog is shorter than the list.
    let offset = cursor_row.saturating_sub(list_h.saturating_sub(1));
    let visible: Vec<Line> = rows.into_iter().skip(offset).take(list_h).collect();
    let list = Rect {
        height: list_h as u16,
        ..inner
    };
    // Fill the cursor row's background across the full width.
    frame.render_widget(Paragraph::new(visible), list);
    let cur_y = (cursor_row - offset) as u16;
    if (cur_y as usize) < list_h {
        let buf = frame.buffer_mut();
        for x in inner.x..inner.x + inner.width {
            buf[(x, inner.y + cur_y)].set_bg(t.tokens.sel);
        }
    }

    let rule = Rect {
        y: inner.y + inner.height - 2,
        height: 1,
        ..inner
    };
    frame.render_widget(
        Paragraph::new(Line::styled("─".repeat(iw), Style::default().fg(t.tokens.hl))),
        rule,
    );
    let desc = Rect {
        y: rule.y + 1,
        ..rule
    };
    let line = match &dlg.input {
        Some(input) => {
            let (a, b) = input.value.split_at(
                input
                    .value
                    .char_indices()
                    .nth(input.cursor)
                    .map(|(i, _)| i)
                    .unwrap_or(input.value.len()),
            );
            Line::from(vec![
                Span::styled(" Limit: ", t.secondary()),
                Span::styled(a.to_string(), t.text()),
                Span::styled("█", Style::default().fg(t.tokens.blue)),
                Span::styled(b.to_string(), t.text()),
                Span::styled("   ↵ apply · Esc back", t.dim()),
            ])
        }
        None => Line::styled(
            format!(
                " {}",
                truncate_width(SettingId::ALL[dlg.cursor].description(), iw.saturating_sub(1))
            ),
            t.dim(),
        ),
    };
    frame.render_widget(Paragraph::new(line), desc);
}
