# Themes

ratclone has eleven built-in themes:

- `tokyo-night` (default), `tokyo-night-storm`, `tokyo-night-moon`,
  `tokyo-night-day`
- `catppuccin-mocha`, `catppuccin-latte`
- `gruvbox-dark`, `nord`, `dracula`, `kanagawa`, `everforest`

The quickest way to choose is the [settings](settings.md) screen (`,`): `←`
`→` on **Theme** restyles the whole app as you go, `Enter` keeps it and `Esc`
puts the old one back. Or set `theme` in the
[configuration](configuration.md).

## Your own theme

A custom theme is a file in `~/.config/ratclone/themes/`, named after the
theme: `~/.config/ratclone/themes/mytheme.toml` is the theme `mytheme`. It
shows up in the settings next to the built-in ones.

The file has a `[tokens]` table with sixteen colours, all of them required,
as `"#rrggbb"`:

```toml
[tokens]
bg      = "#1a1b26"  # background
bg2     = "#16161e"  # dialogs
hl      = "#292e42"  # highlights, the cursor in an unfocused pane
sel     = "#283457"  # the cursor row
gutter  = "#3b4261"  # dim lines
comment = "#565f89"  # secondary text
fg      = "#c0caf5"  # text
fg2     = "#a9b1d6"  # softer text
blue    = "#7aa2f7"
cyan    = "#7dcfff"
magenta = "#bb9af7"
green   = "#9ece6a"
yellow  = "#e0af68"
orange  = "#ff9e64"
red     = "#f7768e"
teal    = "#73daca"
```

A missing key or a malformed colour is reported with the key's name.

## Transparency

`transparent = true` leaves the background to your terminal, for a terminal
with its own background image or opacity. `false` always paints the theme's
background, which is also what happens without the key.
