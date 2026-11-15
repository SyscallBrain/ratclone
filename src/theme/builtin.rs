//! Built-in themes (token order: bg bg2 hl sel gutter comment fg fg2 blue cyan magenta green yellow orange red teal).

use super::{Theme, ThemeTokens, rgb};

fn tokens(hexes: [&str; 16]) -> ThemeTokens {
    ThemeTokens {
        bg: rgb(hexes[0]),
        bg2: rgb(hexes[1]),
        hl: rgb(hexes[2]),
        sel: rgb(hexes[3]),
        gutter: rgb(hexes[4]),
        comment: rgb(hexes[5]),
        fg: rgb(hexes[6]),
        fg2: rgb(hexes[7]),
        blue: rgb(hexes[8]),
        cyan: rgb(hexes[9]),
        magenta: rgb(hexes[10]),
        green: rgb(hexes[11]),
        yellow: rgb(hexes[12]),
        orange: rgb(hexes[13]),
        red: rgb(hexes[14]),
        teal: rgb(hexes[15]),
    }
}

macro_rules! theme {
    ($fn_name:ident, $name:expr, $($hex:expr),* $(,)?) => {
        /// Built-in theme constructor.
        pub fn $fn_name() -> Theme {
            Theme {
                name: $name,
                tokens: tokens([$($hex),*]),
                border: ratatui::widgets::BorderType::Rounded,
                transparent: false,
            }
        }
    };
}

theme!(
    tokyo_night,
    "tokyo-night",
    "#1a1b26",
    "#16161e",
    "#292e42",
    "#283457",
    "#3b4261",
    "#565f89",
    "#c0caf5",
    "#a9b1d6",
    "#7aa2f7",
    "#7dcfff",
    "#bb9af7",
    "#9ece6a",
    "#e0af68",
    "#ff9e64",
    "#f7768e",
    "#73daca"
);

theme!(
    tokyo_night_storm,
    "tokyo-night-storm",
    "#24283b",
    "#1f2335",
    "#292e42",
    "#2e3c64",
    "#3b4261",
    "#565f89",
    "#c0caf5",
    "#a9b1d6",
    "#7aa2f7",
    "#7dcfff",
    "#bb9af7",
    "#9ece6a",
    "#e0af68",
    "#ff9e64",
    "#f7768e",
    "#73daca"
);

theme!(
    tokyo_night_day,
    "tokyo-night-day",
    "#e1e2e7",
    "#d0d5e3",
    "#c4c8da",
    "#b7c1e3",
    "#a8aecb",
    "#6172b0",
    "#3760bf",
    "#4c5a8f",
    "#2e7de9",
    "#007197",
    "#9854f1",
    "#587539",
    "#8c6c3e",
    "#b15c00",
    "#f52a65",
    "#118c74"
);

theme!(
    catppuccin_mocha,
    "catppuccin-mocha",
    "#1e1e2e",
    "#181825",
    "#313244",
    "#45475a",
    "#45475a",
    "#7f849c",
    "#cdd6f4",
    "#bac2de",
    "#89b4fa",
    "#89dceb",
    "#cba6f7",
    "#a6e3a1",
    "#f9e2af",
    "#fab387",
    "#f38ba8",
    "#94e2d5"
);

theme!(
    gruvbox_dark,
    "gruvbox-dark",
    "#282828",
    "#1d2021",
    "#3c3836",
    "#504945",
    "#504945",
    "#928374",
    "#ebdbb2",
    "#d5c4a1",
    "#83a598",
    "#8ec07c",
    "#d3869b",
    "#b8bb26",
    "#fabd2f",
    "#fe8019",
    "#fb4934",
    "#8ec07c"
);

theme!(
    nord, "nord", "#2e3440", "#272c36", "#3b4252", "#434c5e", "#4c566a", "#7b88a1", "#eceff4", "#d8dee9",
    "#81a1c1", "#88c0d0", "#b48ead", "#a3be8c", "#ebcb8b", "#d08770", "#bf616a", "#8fbcbb"
);

theme!(
    dracula, "dracula", "#282a36", "#21222c", "#343746", "#44475a", "#44475a", "#6272a4", "#f8f8f2",
    "#e2e2dc", "#bd93f9", "#8be9fd", "#ff79c6", "#50fa7b", "#f1fa8c", "#ffb86c", "#ff5555", "#8be9fd"
);

theme!(
    kanagawa, "kanagawa", "#1f1f28", "#16161d", "#2a2a37", "#2d4f67", "#363646", "#727169", "#dcd7ba",
    "#c8c093", "#7e9cd8", "#7fb4ca", "#957fb8", "#98bb6c", "#e6c384", "#ffa066", "#e46876", "#7aa89f"
);

theme!(
    everforest,
    "everforest",
    "#2d353b",
    "#232a2e",
    "#343f44",
    "#475258",
    "#475258",
    "#859289",
    "#d3c6aa",
    "#9da9a0",
    "#7fbbb3",
    "#83c092",
    "#d699b6",
    "#a7c080",
    "#dbbc7f",
    "#e69875",
    "#e67e80",
    "#83c092"
);

theme!(
    tokyo_night_moon,
    "tokyo-night-moon",
    "#222436",
    "#1e2030",
    "#2f334d",
    "#2d3f76",
    "#3b4261",
    "#636da6",
    "#c8d3f5",
    "#828bb8",
    "#82aaff",
    "#86e1fc",
    "#c099ff",
    "#c3e88d",
    "#ffc777",
    "#ff966c",
    "#ff757f",
    "#4fd6be"
);

theme!(
    catppuccin_latte,
    "catppuccin-latte",
    "#eff1f5",
    "#e6e9ef",
    "#ccd0da",
    "#bcc0cc",
    "#acb0be",
    "#8c8fa1",
    "#4c4f69",
    "#5c5f77",
    "#1e66f5",
    "#04a5e5",
    "#8839ef",
    "#40a02b",
    "#df8e1d",
    "#fe640b",
    "#d20f39",
    "#179299"
);

/// All built-in themes, in picker order.
pub fn all() -> Vec<Theme> {
    vec![
        tokyo_night(),
        tokyo_night_storm(),
        tokyo_night_moon(),
        tokyo_night_day(),
        catppuccin_mocha(),
        catppuccin_latte(),
        gruvbox_dark(),
        nord(),
        dracula(),
        kanagawa(),
        everforest(),
    ]
}

/// Look up a built-in theme by name, falling back to tokyo-night.
pub fn by_name(name: &str) -> Theme {
    all()
        .into_iter()
        .find(|t| t.name == name)
        .unwrap_or_else(tokyo_night)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_tokyo_night() {
        assert_eq!(by_name("nope").name, "tokyo-night");
        assert_eq!(by_name("catppuccin-latte").name, "catppuccin-latte");
        let names: Vec<&str> = all().iter().map(|t| t.name).collect();
        for n in [
            "tokyo-night",
            "tokyo-night-storm",
            "tokyo-night-moon",
            "tokyo-night-day",
            "catppuccin-mocha",
            "catppuccin-latte",
            "gruvbox-dark",
            "nord",
            "dracula",
        ] {
            assert!(names.contains(&n), "{n}");
        }
    }
}
