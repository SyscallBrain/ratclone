//! The account wizard: type list, fields, rclone's questions, the browser login.

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::app::account::form::{self, Edit, Field, Widget};
use crate::app::account::{AccountWizard, Row, Step};
use crate::app::overlay::LineInput;
use crate::format::truncate_width;
use crate::theme::Theme;

const MAX_W: u16 = 72;
const MAX_H: u16 = 24;
const MIN_W: u16 = 40;
const MIN_H: u16 = 12;
/// Widest label column.
const MAX_LABEL: usize = 24;

pub fn draw(frame: &mut Frame, area: Rect, app: &App, w: &AccountWizard) {
    let t = &app.theme;
    if area.width < MIN_W || area.height < MIN_H {
        frame.render_widget(Clear, area);
        let p = Paragraph::new(vec![
            Line::raw(""),
            Line::styled("Terminal too small for this dialog", t.text()),
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
    let width = MAX_W.min(area.width);
    let height = wanted_height(w).min(MAX_H).min(area.height);
    let rect = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 3,
        width,
        height,
    };
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(t.tokens.blue).bg(t.tokens.bg2))
        .title(Span::styled(
            format!(" {} ", w.title()),
            Style::default().fg(t.tokens.blue).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(hints(t, &w.step).right_aligned())
        .style(Style::default().fg(t.tokens.fg).bg(t.tokens.bg2));
    let mut inner = block.inner(rect);
    frame.render_widget(block, rect);

    if let Some(warn) = &w.transfers_warning {
        let line = Line::styled(format!(" {warn}"), Style::default().fg(t.tokens.yellow));
        frame.render_widget(Paragraph::new(line), Rect { height: 1, ..inner });
        inner.y += 1;
        inner.height = inner.height.saturating_sub(1);
    }

    // Text that wraps keeps a one-column margin on both sides.
    let body = Rect {
        x: inner.x + 1,
        width: inner.width.saturating_sub(2),
        ..inner
    };
    let spin = super::spinner(app.spinner);
    match &w.step {
        Step::Loading => centred(
            frame,
            inner,
            vec![Line::styled(format!("{spin} Loading…"), t.dim())],
        ),
        Step::Sending => centred(
            frame,
            inner,
            vec![Line::styled(format!("{spin} Talking to rclone…"), t.dim())],
        ),
        Step::Type { query, cursor } => draw_types(frame, inner, t, w, query, *cursor),
        Step::Form {
            cursor,
            show_advanced,
            help_open,
        } => draw_form(frame, inner, t, w, *cursor, *show_advanced, *help_open),
        Step::Question { field, error, .. } => draw_question(frame, body, t, w, field, error.as_deref()),
        Step::WaitingBrowser { .. } => {
            let mut lines = vec![
                Line::styled(
                    format!("{spin} Waiting for you to authorise in the browser…"),
                    t.text(),
                ),
                Line::raw(""),
            ];
            if let Some(note) = &w.auto_note {
                lines.push(Line::styled(note.clone(), t.dim()));
            }
            lines.push(Line::from(vec![
                Span::styled("L", t.key()),
                Span::styled(" paste a token instead · ", t.dim()),
                Span::styled("Esc", t.key()),
                Span::styled(" cancel", t.dim()),
            ]));
            centred(frame, inner, lines);
        }
        Step::TokenPaste {
            command,
            input,
            error,
            ..
        } => {
            let mut lines = vec![
                Line::styled("Run this on a machine with a browser:", t.text()),
                Line::raw(""),
                Line::from(vec![
                    Span::styled(format!("  {command}"), Style::default().fg(t.tokens.blue)),
                    Span::styled("   ", t.dim()),
                    Span::styled("y", t.key()),
                    Span::styled(" copy", t.dim()),
                ]),
                Line::raw(""),
                Line::styled("Paste the result:", t.text()),
                masked_input(t, input),
            ];
            if let Some(e) = error {
                lines.push(Line::styled(e.clone(), Style::default().fg(t.tokens.red)));
            }
            lines.push(Line::raw(""));
            if let Some(note) = &w.auto_note {
                lines.push(Line::styled(format!("{note} (L to use a browser here)"), t.dim()));
            }
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body);
        }
        Step::DriveWarning { help, .. } => {
            let mut lines: Vec<Line> = help
                .lines()
                .map(|l| Line::styled(l.to_string(), Style::default().fg(t.tokens.yellow)))
                .collect();
            lines.push(Line::raw(""));
            lines.push(Line::from(vec![
                Span::styled("y", t.key()),
                Span::styled(" use the shared one anyway · ", t.dim()),
                Span::styled("n", t.key()),
                Span::styled(" enter my own client_id", t.dim()),
            ]));
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body);
        }
        Step::ConfirmCancel { .. } => {
            let text = if w.editing() {
                "Stop here? Changes already sent stay saved."
            } else {
                "Cancel? The account will not be created."
            };
            centred(
                frame,
                inner,
                vec![
                    Line::styled(text, t.text().add_modifier(Modifier::BOLD)),
                    Line::raw(""),
                    Line::from(vec![
                        Span::styled("y", t.key()),
                        Span::styled(" yes · ", t.dim()),
                        Span::styled("n", t.key()),
                        Span::styled(" no", t.dim()),
                    ]),
                ],
            );
        }
        Step::Failed { message, .. } => {
            let lines = vec![
                Line::raw(""),
                Line::styled(message.clone(), Style::default().fg(t.tokens.red)),
                Line::raw(""),
                Line::from(vec![Span::styled("↵", t.key()), Span::styled(" back", t.dim())]),
            ];
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body);
        }
    }
}

/// Rows the box needs for this step (borders included), before the screen limit.
fn wanted_height(w: &AccountWizard) -> u16 {
    let warn = u16::from(w.transfers_warning.is_some());
    let help_rows = |help: &str| help.lines().count() as u16 + help.len() as u16 / 60;
    let h = match &w.step {
        Step::Type { .. } => MAX_H,
        Step::Form {
            show_advanced,
            help_open: false,
            ..
        } => w.rows(*show_advanced).len() as u16 + 6,
        Step::Form { .. } => MAX_H,
        Step::Question { field, .. } => help_rows(&field.option.help) + 8,
        Step::DriveWarning { help, .. } => help_rows(help) + 6,
        Step::TokenPaste { .. } => 14,
        _ => 10,
    };
    h.clamp(MIN_H - 2, MAX_H) + warn
}

fn hints(t: &Theme, step: &Step) -> Line<'static> {
    let parts: &[(&str, &str)] = match step {
        Step::Type { .. } => &[("↵", "next"), ("Esc", "cancel")],
        Step::Form { .. } => &[
            ("^s", "save"),
            ("Tab", "advanced"),
            ("F1", "help"),
            ("Esc", "back"),
        ],
        Step::Question { .. } | Step::TokenPaste { .. } => &[("↵", "answer"), ("Esc", "cancel")],
        Step::WaitingBrowser { .. } | Step::Loading | Step::Sending | Step::DriveWarning { .. } => {
            &[("Esc", "cancel")]
        }
        Step::ConfirmCancel { .. } | Step::Failed { .. } => &[],
    };
    let mut spans = vec![Span::raw(" ")];
    for (i, (k, d)) in parts.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", t.dim()));
        }
        spans.push(Span::styled(k.to_string(), t.key()));
        spans.push(Span::styled(format!(" {d}"), t.dim()));
    }
    spans.push(Span::raw(" "));
    Line::from(spans)
}

fn centred(frame: &mut Frame, area: Rect, lines: Vec<Line>) {
    let top = area.height.saturating_sub(lines.len() as u16) / 2;
    let r = Rect {
        y: area.y + top,
        height: area.height - top,
        ..area
    };
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), r);
}

/// The text with a block cursor at the input's cursor, in at most `room` columns:
/// a long value scrolls so the cursor stays in sight.
fn with_cursor(t: &Theme, shown: &str, cursor: usize, style: Style, room: usize) -> Vec<Span<'static>> {
    let chars: Vec<char> = shown.chars().collect();
    let cursor = cursor.min(chars.len());
    let fits = shown.width() < room;
    let (before, after): (String, String) = if fits {
        (chars[..cursor].iter().collect(), chars[cursor..].iter().collect())
    } else {
        // Keep what ends at the cursor, after an ellipsis.
        let mut start = cursor;
        let mut w = 0;
        while start > 0 {
            let cw = chars[start - 1].to_string().width();
            if w + cw + 2 > room {
                break;
            }
            w += cw;
            start -= 1;
        }
        let mut before: String = chars[start..cursor].iter().collect();
        if start > 0 {
            before.insert(0, '…');
        }
        let mut after = String::new();
        for c in &chars[cursor..] {
            if before.width() + after.width() + c.to_string().width() + 1 >= room {
                break;
            }
            after.push(*c);
        }
        (before, after)
    };
    let (a, b) = (before.as_str(), after.as_str());
    vec![
        Span::styled(a.to_string(), style),
        Span::styled("█", Style::default().fg(t.tokens.blue)),
        Span::styled(b.to_string(), style),
    ]
}

fn masked_input(t: &Theme, input: &LineInput) -> Line<'static> {
    let n = input.value.chars().count();
    let mut spans = vec![Span::raw("   ")];
    if n == 0 {
        spans.push(Span::styled("█", Style::default().fg(t.tokens.blue)));
    } else {
        spans.push(Span::styled(format!("••••• ({n} characters)"), t.text()));
        spans.push(Span::styled("█", Style::default().fg(t.tokens.blue)));
    }
    Line::from(spans)
}

/// The description area under a rule: the last `h` rows of `inner`.
fn bottom(frame: &mut Frame, inner: Rect, t: &Theme, h: u16, lines: Vec<Line>) -> Rect {
    let h = h.min(inner.height.saturating_sub(1));
    let rule = Rect {
        y: inner.y + inner.height - h - 1,
        height: 1,
        ..inner
    };
    frame.render_widget(
        Paragraph::new(Line::styled(
            "─".repeat(inner.width as usize),
            Style::default().fg(t.tokens.hl),
        )),
        rule,
    );
    let desc = Rect {
        y: rule.y + 1,
        height: h,
        ..inner
    };
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), desc);
    Rect {
        height: rule.y - inner.y,
        ..inner
    }
}

/// Paint the cursor row's background across the width.
fn highlight(frame: &mut Frame, t: &Theme, area: Rect, row: usize) {
    if row >= area.height as usize {
        return;
    }
    let buf = frame.buffer_mut();
    for x in area.x..area.x + area.width {
        buf[(x, area.y + row as u16)].set_bg(t.tokens.sel);
    }
}

fn draw_types(
    frame: &mut Frame,
    inner: Rect,
    t: &Theme,
    w: &AccountWizard,
    query: &LineInput,
    cursor: usize,
) {
    let list = w.filtered(&query.value);
    let desc = list
        .get(cursor)
        .map(|p| format!(" {}", p.description))
        .unwrap_or_else(|| " No type matches".into());
    let top = bottom(frame, inner, t, 1, vec![Line::styled(desc, t.dim())]);
    let mut q = vec![Span::styled(" Type  ", t.secondary())];
    q.extend(with_cursor(
        t,
        &query.value,
        query.cursor,
        t.text(),
        top.width as usize - 8,
    ));
    frame.render_widget(Paragraph::new(Line::from(q)), Rect { height: 1, ..top });
    let area = Rect {
        y: top.y + 1,
        height: top.height.saturating_sub(1),
        ..top
    };
    let h = area.height as usize;
    let offset = cursor.saturating_sub(h.saturating_sub(1));
    let iw = area.width as usize;
    let lines: Vec<Line> = list
        .iter()
        .enumerate()
        .skip(offset)
        .take(h)
        .map(|(i, p)| {
            let cur = i == cursor;
            let name = format!(" {} {:<14}", if cur { "▸" } else { " " }, p.name);
            let room = iw.saturating_sub(name.width() + 1);
            Line::from(vec![
                Span::styled(
                    name,
                    if cur {
                        t.text().add_modifier(Modifier::BOLD)
                    } else {
                        t.text()
                    },
                ),
                Span::styled(
                    format!(" {}", truncate_width(&p.description, room)),
                    t.secondary(),
                ),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
    highlight(frame, t, area, cursor - offset);
}

fn label_of(w: &AccountWizard, row: Row) -> String {
    match row {
        Row::Name => "Name".into(),
        Row::Field(i) => w.fields[i].option.name.clone(),
        Row::Advanced => String::new(),
    }
}

/// The value part of a field row.
fn field_value(t: &Theme, f: &Field, cur: bool, editing: bool, room: usize) -> Vec<Span<'static>> {
    let changed = editing && f.edit != Edit::Unchanged;
    let base = if changed {
        Style::default().fg(t.tokens.yellow).add_modifier(Modifier::BOLD)
    } else {
        t.text()
    };
    match f.widget {
        Widget::Toggle | Widget::Choice(_) => {
            let v = f.current();
            // What a value means: for Drive's shared drives the value is an opaque id.
            let meaning = match f.widget {
                Widget::Choice(_) => f
                    .option
                    .examples
                    .iter()
                    .find(|(value, _)| *value == v)
                    .map(|(_, help)| help.lines().next().unwrap_or("").to_string())
                    .filter(|h| !h.is_empty() && *h != v),
                _ => None,
            };
            if cur {
                let s = if changed {
                    base
                } else {
                    Style::default().fg(t.tokens.blue)
                };
                let mut spans = vec![
                    Span::styled("‹ ", Style::default().fg(t.tokens.blue)),
                    Span::styled(v, s),
                    Span::styled(" ›", Style::default().fg(t.tokens.blue)),
                ];
                if let Some(m) = meaning {
                    spans.push(Span::styled(format!("  {m}"), t.dim()));
                }
                spans
            } else {
                vec![Span::styled(v, if changed { base } else { t.secondary() })]
            }
        }
        _ => {
            let shown = f.shown_value();
            if shown.is_empty() {
                let placeholder = if f.edit == Edit::Reset {
                    format!("{} (default)", f.option.default)
                } else {
                    f.option.default.clone()
                };
                let mut v = Vec::new();
                if cur {
                    v.push(Span::styled("█", Style::default().fg(t.tokens.blue)));
                }
                v.push(Span::styled(placeholder, t.dim()));
                v
            } else if cur && (f.widget != Widget::Password || f.reveal) {
                with_cursor(t, &shown, f.input.cursor, base, room)
            } else if cur {
                vec![
                    Span::styled(shown, base),
                    Span::styled("█", Style::default().fg(t.tokens.blue)),
                ]
            } else {
                vec![Span::styled(shown, base)]
            }
        }
    }
}

fn draw_form(
    frame: &mut Frame,
    inner: Rect,
    t: &Theme,
    w: &AccountWizard,
    cursor: usize,
    show_advanced: bool,
    help_open: bool,
) {
    let rows = w.rows(show_advanced);
    let row = rows.get(cursor).copied();
    let help = match row {
        Some(Row::Field(i)) => w.fields[i].option.help.clone(),
        Some(Row::Name) => "The account's name, used as name: in paths.".into(),
        _ => String::new(),
    };
    if help_open {
        let lines: Vec<Line> = help
            .lines()
            .map(|l| Line::styled(l.to_string(), t.text()))
            .collect();
        let body = Rect {
            x: inner.x + 1,
            width: inner.width.saturating_sub(2),
            ..inner
        };
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((w.help_scroll, 0)),
            body,
        );
        return;
    }
    let error = match row {
        Some(Row::Name) => w.name_error.clone().or_else(|| w.form_error.clone()),
        _ => w.form_error.clone(),
    };
    let suggestions = match row {
        Some(Row::Field(i)) => match &w.fields[i].widget {
            Widget::Text { suggestions } if !suggestions.is_empty() => Some(suggestions.join(", ")),
            _ => None,
        },
        _ => None,
    };
    let desc = match (error, suggestions) {
        (Some(e), _) => vec![Line::styled(format!(" {e}"), Style::default().fg(t.tokens.red))],
        (None, Some(ex)) => vec![
            Line::styled(
                format!(
                    " {}",
                    truncate_width(
                        form::first_paragraph(&help).lines().next().unwrap_or(""),
                        inner.width as usize - 1
                    )
                ),
                t.dim(),
            ),
            Line::styled(
                format!(" Examples: {}", truncate_width(&ex, inner.width as usize - 12)),
                t.dim(),
            ),
        ],
        (None, None) => vec![Line::styled(
            format!(" {}", form::first_paragraph(&help)),
            t.dim(),
        )],
    };
    let area = bottom(frame, inner, t, 2, desc);
    let label_w = rows
        .iter()
        .map(|r| label_of(w, *r).width())
        .max()
        .unwrap_or(0)
        .min(MAX_LABEL);
    let h = area.height as usize;
    let offset = cursor.saturating_sub(h.saturating_sub(1));
    let iw = area.width as usize;
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(offset)
        .take(h)
        .map(|(i, r)| {
            let cur = i == cursor;
            let marker = if cur { "▸" } else { " " };
            if *r == Row::Advanced {
                let n = w.advanced_count();
                let s = if n == 1 { "" } else { "s" };
                return Line::styled(
                    format!(" {marker} ⋯ {n} advanced option{s} (Tab to show)"),
                    t.dim(),
                );
            }
            let label = truncate_width(&label_of(w, *r), label_w);
            let pad = label_w.saturating_sub(label.width());
            let mut spans = vec![Span::styled(
                format!(" {marker} {label}{}  ", " ".repeat(pad)),
                if cur {
                    t.text().add_modifier(Modifier::BOLD)
                } else {
                    t.secondary()
                },
            )];
            let room = iw.saturating_sub(spans[0].content.width());
            match r {
                Row::Name => {
                    let style = if w.name_error.is_some() {
                        Style::default().fg(t.tokens.red)
                    } else {
                        t.text()
                    };
                    if cur {
                        spans.extend(with_cursor(t, &w.name.value, w.name.cursor, style, room));
                    } else {
                        spans.push(Span::styled(w.name.value.clone(), style));
                    }
                }
                Row::Field(f) => spans.extend(field_value(t, &w.fields[*f], cur, w.editing(), room)),
                Row::Advanced => {}
            }
            let line = Line::from(spans);
            // Keep long values inside the box.
            if line.width() > iw {
                let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                return Line::styled(truncate_width(&text, iw), t.text());
            }
            line
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
    highlight(frame, t, area, cursor - offset);
}

fn draw_question(
    frame: &mut Frame,
    inner: Rect,
    t: &Theme,
    w: &AccountWizard,
    field: &Field,
    error: Option<&str>,
) {
    // The answer row stays at the bottom; the help fills what is left above it.
    let mut tail: Vec<Line> = Vec::new();
    if let Some(e) = error {
        tail.push(Line::styled(e.to_string(), Style::default().fg(t.tokens.red)));
    }
    let label = format!("▸ {}  ", field.option.name);
    let room = (inner.width as usize).saturating_sub(label.width());
    let mut row = vec![Span::styled(label, t.text().add_modifier(Modifier::BOLD))];
    row.extend(field_value(t, field, true, false, room));
    tail.push(Line::from(row));
    if let Some(note) = &w.auto_note {
        tail.push(Line::styled(note.clone(), t.dim()));
    }
    let tail_h = (tail.len() as u16).min(inner.height);
    let help_h = inner.height.saturating_sub(tail_h + 1);
    let help: Vec<Line> = field
        .option
        .help
        .lines()
        .map(|l| Line::styled(l.to_string(), t.text()))
        .collect();
    frame.render_widget(
        Paragraph::new(help).wrap(Wrap { trim: false }),
        Rect {
            height: help_h,
            ..inner
        },
    );
    frame.render_widget(
        Paragraph::new(tail).wrap(Wrap { trim: false }),
        Rect {
            y: inner.y + inner.height - tail_h,
            height: tail_h,
            ..inner
        },
    );
}
