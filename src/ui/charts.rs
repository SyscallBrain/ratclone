//! Small text charts: braille sparklines and stacked bars.

use ratatui::style::{Color, Style};
use ratatui::text::Span;

/// Dot bits of a braille cell, bottom to top, for the left and right columns.
const LEFT: [u32; 4] = [0x40, 0x04, 0x02, 0x01];
const RIGHT: [u32; 4] = [0x80, 0x20, 0x10, 0x08];

/// A sparkline `rows` high and `cells` wide, top row first. Two samples per
/// cell, the latest `2 * cells` samples, scaled to their maximum.
pub fn braille_rows(samples: &[f64], rows: usize, cells: usize) -> Vec<String> {
    let want = cells * 2;
    let start = samples.len().saturating_sub(want);
    let mut shown: Vec<f64> = vec![0.0; want.saturating_sub(samples.len() - start)];
    shown.extend_from_slice(&samples[start..]);
    let max = shown.iter().copied().fold(0.0, f64::max);
    let levels = rows * 4;
    let heights: Vec<usize> = shown
        .iter()
        .map(|v| {
            if max <= 0.0 {
                0
            } else {
                ((v / max) * levels as f64).round() as usize
            }
        })
        .collect();
    (0..rows)
        .map(|r| {
            let base = (rows - 1 - r) * 4;
            heights
                .chunks(2)
                .map(|pair| {
                    let mut code = 0;
                    for d in 0..4 {
                        if pair[0] > base + d {
                            code |= LEFT[d];
                        }
                        if pair.get(1).is_some_and(|&h| h > base + d) {
                            code |= RIGHT[d];
                        }
                    }
                    char::from_u32(0x2800 + code).unwrap_or(' ')
                })
                .collect()
        })
        .collect()
}

/// One-row sparkline in block characters (for terminals without braille).
pub fn ascii_spark(samples: &[f64], cells: usize) -> String {
    const BLOCKS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let start = samples.len().saturating_sub(cells);
    let shown = &samples[start..];
    let max = shown.iter().copied().fold(0.0, f64::max);
    let mut out = " ".repeat(cells - shown.len());
    out.extend(shown.iter().map(|v| {
        if max <= 0.0 {
            ' '
        } else {
            BLOCKS[((v / max) * 8.0).round() as usize]
        }
    }));
    out
}

/// A bar `width` cells wide made of `parts` (value, colour) out of `total`,
/// the rest drawn as a track. Every non-zero part gets at least one cell.
pub fn stacked(parts: &[(u64, Color)], total: u64, width: usize, track: Color) -> Vec<Span<'static>> {
    let mut cells: Vec<usize> = parts
        .iter()
        .map(|&(v, _)| {
            if total == 0 || v == 0 {
                0
            } else {
                (((v as f64 / total as f64) * width as f64).round() as usize).max(1)
            }
        })
        .collect();
    // Too many cells after rounding up: take them back from the largest part.
    while cells.iter().sum::<usize>() > width {
        if let Some(i) = (0..cells.len()).max_by_key(|&i| cells[i]) {
            cells[i] -= 1;
        }
    }
    let mut spans: Vec<Span<'static>> = parts
        .iter()
        .zip(&cells)
        .filter(|(_, n)| **n > 0)
        .map(|(&(_, c), &n)| Span::styled("█".repeat(n), Style::default().fg(c)))
        .collect();
    let rest = width - cells.iter().sum::<usize>();
    if rest > 0 {
        spans.push(Span::styled("█".repeat(rest), Style::default().fg(track)));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn braille_full_and_empty() {
        let rows = braille_rows(&[1.0; 4], 2, 2);
        assert_eq!(rows, vec!["⣿⣿".to_string(), "⣿⣿".to_string()]);
        let rows = braille_rows(&[0.0; 4], 3, 2);
        assert!(rows.iter().all(|r| r == "⠀⠀"));
    }

    #[test]
    fn braille_half_height_fills_bottom_row_only() {
        let rows = braille_rows(&[0.5, 0.5, 1.0, 1.0], 2, 2);
        assert_eq!(rows[0].chars().next(), Some('⠀'));
        assert_eq!(rows[1].chars().next(), Some('⣿'));
    }

    #[test]
    fn braille_uses_the_latest_samples() {
        // 6 samples, 2 cells: only the last 4 count.
        let rows = braille_rows(&[9.0, 9.0, 0.0, 0.0, 1.0, 1.0], 1, 2);
        assert_eq!(rows[0], "⠀⣿");
    }

    #[test]
    fn ascii_spark_levels() {
        assert_eq!(ascii_spark(&[0.0, 0.5, 1.0], 3), " ▄█");
        assert_eq!(ascii_spark(&[0.0, 0.0], 2), "  ");
    }

    #[test]
    fn stacked_widths_sum_and_tiny_parts_show() {
        let spans = stacked(&[(62, Color::Blue), (1, Color::Red)], 100, 30, Color::DarkGray);
        let total: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        assert_eq!(total, 30);
        assert!(
            spans
                .iter()
                .any(|s| s.style.fg == Some(Color::Red) && !s.content.is_empty())
        );
        let empty = stacked(&[], 0, 10, Color::DarkGray);
        assert_eq!(empty.iter().map(|s| s.content.chars().count()).sum::<usize>(), 10);
    }
}
