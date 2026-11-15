//! Custom theme loading from ~/.config/ratclone/themes/<name>.toml.

use std::collections::BTreeSet;
use std::sync::Mutex;

use color_eyre::{Result, eyre::eyre};

use super::{Theme, ThemeTokens, parse_hex};

const KEYS: [&str; 16] = [
    "bg", "bg2", "hl", "sel", "gutter", "comment", "fg", "fg2", "blue", "cyan", "magenta", "green", "yellow",
    "orange", "red", "teal",
];

/// `name` as a `'static` string, the same one for every call with that name.
/// A [`Theme`] is `Copy` and holds its name by reference; the settings dialog loads
/// a custom theme at every step through the list, which must not leak each time.
fn interned(name: &str) -> &'static str {
    static NAMES: Mutex<BTreeSet<&'static str>> = Mutex::new(BTreeSet::new());
    let mut names = NAMES.lock().unwrap_or_else(|e| e.into_inner());
    match names.get(name) {
        Some(known) => known,
        None => {
            let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
            names.insert(leaked);
            leaked
        }
    }
}

/// Parse a custom theme from TOML text.
///
/// Expects a `[tokens]` table with all 16 keys as `"#rrggbb"`.
/// A missing key is an error naming the key.
pub fn from_toml(name: &str, text: &str) -> Result<Theme> {
    let value: toml::Value = toml::from_str(text)?;
    let tokens = value
        .get("tokens")
        .and_then(|t| t.as_table())
        .ok_or_else(|| eyre!("theme {name}: missing [tokens]"))?;
    let mut colors = Vec::with_capacity(KEYS.len());
    for key in KEYS {
        let hex = tokens
            .get(key)
            .and_then(|v| v.as_str())
            .ok_or_else(|| eyre!("theme {name}: missing key {key}"))?;
        let color =
            parse_hex(hex).ok_or_else(|| eyre!("theme {name}: {key} = {hex:?} is not a #rrggbb colour"))?;
        colors.push(color);
    }
    Ok(Theme {
        name: interned(name),
        tokens: ThemeTokens {
            bg: colors[0],
            bg2: colors[1],
            hl: colors[2],
            sel: colors[3],
            gutter: colors[4],
            comment: colors[5],
            fg: colors[6],
            fg2: colors[7],
            blue: colors[8],
            cyan: colors[9],
            magenta: colors[10],
            green: colors[11],
            yellow: colors[12],
            orange: colors[13],
            red: colors[14],
            teal: colors[15],
        },
        border: ratatui::widgets::BorderType::Rounded,
        transparent: false,
    })
}

/// Load a custom theme file. Returns an error string for the message line on failure.
pub fn load_custom(name: &str) -> Result<Theme> {
    let dir = crate::paths::config_file()
        .parent()
        .map(|p| p.join("themes"))
        .unwrap_or_default();
    load_custom_from(&dir, name)
}

/// Load `<dir>/<name>.toml`.
pub fn load_custom_from(dir: &std::path::Path, name: &str) -> Result<Theme> {
    let text = std::fs::read_to_string(dir.join(format!("{name}.toml")))?;
    from_toml(name, &text)
}

/// Names of the `*.toml` files in `dir`, sorted.
pub fn custom_theme_names(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            (p.extension()? == "toml").then(|| p.file_stem()?.to_str().map(str::to_string))?
        })
        .collect();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(bg: &str) -> String {
        let mut t = String::from("[tokens]\n");
        for key in KEYS {
            let v = if key == "bg" { bg } else { "#ffffff" };
            t.push_str(&format!("{key} = \"{v}\"\n"));
        }
        t
    }

    #[test]
    fn a_theme_name_is_kept_once_however_often_it_is_loaded() {
        let a = from_toml("mine", &text("#000000")).unwrap();
        let b = from_toml("mine", &text("#111111")).unwrap();
        assert_eq!(a.name, "mine");
        assert!(
            std::ptr::eq(a.name, b.name),
            "one copy of the name, not one per load"
        );
    }

    #[test]
    fn a_bad_colour_is_an_error_naming_the_key() {
        assert!(from_toml("ok", &text("#1a1b26")).is_ok());
        assert!(from_toml("ok", &text("1A1B26")).is_ok());
        for bad in ["#fff", "#ééé", "#12345", "#1234567", "#gggggg", "", "#ab€"] {
            let e = from_toml("bad", &text(bad))
                .err()
                .unwrap_or_else(|| panic!("{bad:?} accepted"));
            assert!(e.to_string().contains("bad: bg"), "{e}");
        }
    }
}
