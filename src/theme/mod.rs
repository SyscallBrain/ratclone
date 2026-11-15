//! Theme tokens, built-in themes, and style helpers (ported from virsh-tui).

pub mod builtin;
pub mod load;

use ratatui::style::{Color, Modifier, Style};

/// The 16 theme tokens in binding order:
/// bg bg2 hl sel gutter comment fg fg2 blue cyan magenta green yellow orange red teal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeTokens {
    pub bg: Color,
    pub bg2: Color,
    pub hl: Color,
    pub sel: Color,
    pub gutter: Color,
    pub comment: Color,
    pub fg: Color,
    pub fg2: Color,
    pub blue: Color,
    pub cyan: Color,
    pub magenta: Color,
    pub green: Color,
    pub yellow: Color,
    pub orange: Color,
    pub red: Color,
    pub teal: Color,
}

impl ThemeTokens {
    /// Token hex values in binding order.
    pub fn hex_list(&self) -> [String; 16] {
        [
            color_hex(self.bg),
            color_hex(self.bg2),
            color_hex(self.hl),
            color_hex(self.sel),
            color_hex(self.gutter),
            color_hex(self.comment),
            color_hex(self.fg),
            color_hex(self.fg2),
            color_hex(self.blue),
            color_hex(self.cyan),
            color_hex(self.magenta),
            color_hex(self.green),
            color_hex(self.yellow),
            color_hex(self.orange),
            color_hex(self.red),
            color_hex(self.teal),
        ]
    }
}

/// A named theme: tokens plus metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub name: &'static str,
    pub tokens: ThemeTokens,
    pub border: ratatui::widgets::BorderType,
    pub transparent: bool,
}

impl Theme {
    /// Effective background (`Reset` when transparent).
    pub fn bg(&self) -> Color {
        if self.transparent {
            Color::Reset
        } else {
            self.tokens.bg
        }
    }

    /// Base style of a whole area (text colour on the theme background). Use it
    /// as the style of paragraphs/blocks; spans use the fg-only helpers below so
    /// they inherit the surface they sit on (chips, bars, selected rows).
    pub fn base(&self) -> Style {
        Style::default().fg(self.tokens.fg).bg(self.bg())
    }

    /// Primary text colour (inherits the background).
    pub fn text(&self) -> Style {
        Style::default().fg(self.tokens.fg)
    }

    /// Dim text (labels, hints); inherits the background.
    pub fn dim(&self) -> Style {
        Style::default().fg(self.tokens.comment)
    }

    /// Secondary text (values); inherits the background.
    pub fn secondary(&self) -> Style {
        Style::default().fg(self.tokens.fg2)
    }

    /// Key in a hint (orange bold); inherits the background.
    pub fn key(&self) -> Style {
        Style::default()
            .fg(self.tokens.orange)
            .add_modifier(Modifier::BOLD)
    }

    pub fn accent(&self) -> Style {
        Style::default().fg(self.tokens.blue)
    }

    pub fn topbar(&self) -> Style {
        Style::default().fg(self.tokens.fg).bg(self.tokens.bg2)
    }

    pub fn panel_border(&self, focused: bool) -> Style {
        Style::default()
            .fg(if focused {
                self.tokens.blue
            } else {
                self.tokens.gutter
            })
            .bg(self.bg())
    }

    pub fn panel_title(&self, focused: bool) -> Style {
        Style::default()
            .fg(if focused {
                self.tokens.blue
            } else {
                self.tokens.fg2
            })
            .bg(self.bg())
            .add_modifier(Modifier::BOLD)
    }

    pub fn row_selected(&self, focused: bool) -> Style {
        Style::default()
            .fg(self.tokens.fg)
            .bg(if focused { self.tokens.sel } else { self.tokens.hl })
    }
}

fn color_hex(c: Color) -> String {
    match c {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => String::from("?"),
    }
}

/// A built-in `#rrggbb`: fixed text, every one built by the tests of [`builtin`].
fn rgb(hex: &str) -> Color {
    parse_hex(hex).unwrap_or_else(|| panic!("bad built-in colour {hex}"))
}

/// `#rrggbb` (the `#` optional) as a colour; anything else is `None`.
pub fn parse_hex(hex: &str) -> Option<Color> {
    let h = hex.trim().strip_prefix('#').unwrap_or(hex.trim());
    if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some(Color::Rgb(byte(0)?, byte(2)?, byte(4)?))
}
