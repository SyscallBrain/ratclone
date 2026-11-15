//! Human-friendly formatting helpers.

use jiff::Timestamp;
use jiff::tz::TimeZone;
use unicode_width::UnicodeWidthChar;

/// Binary-unit size: `"0 B"`, `"1023 B"`, `"4.0 MiB"`.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut v = bytes as f64 / 1024.0;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    format!("{v:.1} {}", UNITS[unit])
}

/// Truncate to at most `width` terminal columns, ending with `…` when cut.
pub fn truncate_width(s: &str, width: usize) -> String {
    let total: usize = s.chars().map(|c| c.width().unwrap_or(0)).sum();
    if total <= width {
        return s.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > width - 1 {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

/// Truncate to at most `width` columns by cutting out the middle, so the start and
/// the end (a file's extension) stay: `"a very long…name.tar.gz"`.
pub fn truncate_middle(s: &str, width: usize) -> String {
    let w = |c: char| c.width().unwrap_or(0);
    let total: usize = s.chars().map(w).sum();
    if total <= width {
        return s.to_string();
    }
    if width < 5 {
        return truncate_width(s, width);
    }
    let room = width - 1;
    let tail_w = room * 2 / 5;
    let mut tail: Vec<char> = Vec::new();
    let mut used = 0;
    for c in s.chars().rev() {
        if used + w(c) > tail_w {
            break;
        }
        tail.push(c);
        used += w(c);
    }
    let head_w = room - used;
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        if used + w(c) > head_w {
            break;
        }
        out.push(c);
        used += w(c);
    }
    out.push('…');
    out.extend(tail.into_iter().rev());
    out
}

/// `"14:05"` today, `"Aug 01"` this year, `"2024-08-01"` otherwise.
pub fn short_time_in(t: Timestamp, now: Timestamp, tz: &TimeZone) -> String {
    let t = t.to_zoned(tz.clone());
    let now = now.to_zoned(tz.clone());
    if t.date() == now.date() {
        t.strftime("%H:%M").to_string()
    } else if t.year() == now.year() {
        t.strftime("%b %d").to_string()
    } else {
        t.strftime("%Y-%m-%d").to_string()
    }
}

/// The part of `value` to show while editing it in `room` columns, so the cursor
/// (`cursor` chars in) is always in sight: `(before, at, after)`, where `at` is the
/// character under the cursor (a space at the end) and a cut side ends in `…`.
/// Widths are display columns; the three together never exceed `room` (but the
/// cursor always shows, even in less than one column).
pub fn edit_window(value: &str, cursor: usize, room: usize) -> (String, String, String) {
    let cw = |c: &char| c.width().unwrap_or(0);
    let chars: Vec<char> = value.chars().collect();
    let cursor = cursor.min(chars.len());
    let at = chars.get(cursor).map_or_else(|| " ".to_string(), char::to_string);
    let at_w = chars.get(cursor).map_or(1, cw);
    let before_all = &chars[..cursor];
    let after_all = chars.get(cursor + 1..).unwrap_or(&[]);
    let before_w: usize = before_all.iter().map(cw).sum();
    let after_w: usize = after_all.iter().map(cw).sum();
    if before_w + at_w + after_w <= room {
        return (before_all.iter().collect(), at, after_all.iter().collect());
    }
    let budget = room.saturating_sub(at_w);
    if budget < 2 {
        // No room for anything but the cursor itself.
        return (String::new(), at, String::new());
    }
    // Some of what follows stays in sight, but the side being typed gets the most.
    let before_budget = budget - after_w.min(budget / 3);
    let before = keep(before_all.iter().rev(), before_w, before_budget, cw);
    let before: String = if before.len() < before_all.len() {
        std::iter::once('…').chain(before.into_iter().rev()).collect()
    } else {
        before.into_iter().rev().collect()
    };
    let used = before.chars().map(|c| cw(&c)).sum::<usize>();
    let after_budget = budget.saturating_sub(used);
    let kept = keep(after_all.iter(), after_w, after_budget, cw);
    let mut after: String = kept.iter().collect();
    if kept.len() < after_all.len() {
        after.push('…');
    }
    (before, at, after)
}

/// Chars from `it` (of total width `total`) that fit in `budget` columns, leaving one
/// for an ellipsis when not all of them fit.
fn keep<'a>(
    it: impl Iterator<Item = &'a char>,
    total: usize,
    budget: usize,
    cw: impl Fn(&char) -> usize,
) -> Vec<char> {
    let budget = if total <= budget {
        budget
    } else {
        budget.saturating_sub(1)
    };
    let mut w = 0;
    let mut out = Vec::new();
    for c in it {
        if w + cw(c) > budget {
            break;
        }
        w += cw(c);
        out.push(*c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(value: &str, cursor: usize, room: usize) -> String {
        let (b, a, f) = edit_window(value, cursor, room);
        let total = format!("{b}{a}{f}");
        assert!(
            unicode_width::UnicodeWidthStr::width(total.as_str()) <= room,
            "{total:?} over {room}"
        );
        format!("{b}[{a}]{f}")
    }

    #[test]
    fn edit_window_keeps_the_cursor_in_sight() {
        assert_eq!(window("abc", 3, 10), "abc[ ]");
        assert_eq!(window("abc", 1, 10), "a[b]c");
        let long = "x".repeat(90);
        assert_eq!(window(&long, 90, 10), format!("…{}[ ]", "x".repeat(8)));
        let named = format!("{}END.txt", "x".repeat(210));
        let shown = window(&named, named.chars().count(), 56);
        assert!(shown.ends_with("END.txt[ ]"), "{shown}");
        // In the middle: both sides cut, both still showing something.
        let shown = window(&long, 45, 20);
        assert!(
            shown.starts_with('…') && shown.ends_with('…') && shown.contains("[x]"),
            "{shown}"
        );
        // At the start of a long value: only the end is cut.
        assert_eq!(window(&long, 0, 6), "[x]xxxx…");
        // Wide characters count as two columns.
        let wide = "日本語のファイル名".repeat(4);
        let shown = window(&wide, wide.chars().count(), 12);
        assert!(shown.starts_with('…'), "{shown}");
        assert_eq!(window("abc", 1, 2), "[b]");
    }

    #[test]
    fn sizes() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(4_200_000), "4.0 MiB");
        assert_eq!(human_size(16_106_127_360), "15.0 GiB");
    }

    #[test]
    fn truncate_by_display_width() {
        assert_eq!(truncate_width("abcdef", 4), "abc…");
        assert_eq!(truncate_width("日本語テキスト", 5), "日本…");
        assert_eq!(truncate_width("abc", 10), "abc");
        assert_eq!(truncate_width("abc", 0), "");
    }

    #[test]
    fn truncate_in_the_middle() {
        assert_eq!(truncate_middle("short.txt", 20), "short.txt");
        let long = "a very long file name that will certainly not fit.tar.gz";
        let cut = truncate_middle(long, 30);
        assert_eq!(cut, "a very long file n… fit.tar.gz");
        assert!(cut.chars().count() <= 30);
        assert_eq!(truncate_middle("日本語のファイル名.txt", 12), "日本語….txt");
        assert_eq!(truncate_middle("abcdef", 4), "abc…");
    }

    fn st(t: Timestamp, now: Timestamp) -> String {
        short_time_in(t, now, &TimeZone::UTC)
    }

    #[test]
    fn times() {
        let now: Timestamp = "2026-11-14T15:00:00Z".parse().unwrap();
        assert_eq!(st("2026-11-14T14:05:00Z".parse().unwrap(), now), "14:05");
        assert_eq!(st("2026-08-01T10:00:00Z".parse().unwrap(), now), "Aug 01");
        assert_eq!(st("2024-08-01T10:00:00Z".parse().unwrap(), now), "2024-08-01");
    }
}
