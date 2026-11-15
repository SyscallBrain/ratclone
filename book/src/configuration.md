# Configuration

Settings live in `~/.config/ratclone/config.toml` (or
`$XDG_CONFIG_HOME/ratclone/config.toml`). The file is optional: every key has
a default. The [settings](settings.md) screen (`,`) writes it for you and
keeps your comments.

```toml
theme = "tokyo-night"
transparent = false
icons = "nerd"
key_hints = true
info_panel = true
info_panel_split = "focused"
start_in = "last"
sort = "name"
show_hidden = false
on_quit = "ask"
bwlimit = "off"
scan_concurrency = 4
rclone_path = "/usr/bin/rclone"
```

| Key | Values | Default |
|---|---|---|
| `theme` | a built-in theme or the name of a file in `themes/`; see [Themes](themes.md) | `"tokyo-night"` |
| `transparent` | `true`: use the terminal's background; `false`: paint the theme's | `false` |
| `icons` | `"nerd"` (needs a Nerd Font) or `"ascii"` | `"nerd"` |
| `key_hints` | show the keys on the bottom line | `true` |
| `info_panel` | show the info panel at start-up | `true` |
| `info_panel_split` | with two panes: `"focused"` (the focused pane's selection) or `"hidden"` | `"focused"` |
| `start_in` | `"last"` (where you were), `"home"` (the home folder) or an account name | `"last"` |
| `sort` | `"name"`, `"size"` or `"modified"` | `"name"` |
| `show_hidden` | show files whose name starts with `.` | `false` |
| `on_quit` | with transfers running: `"ask"`, `"background"` (keep them running) or `"wait"` | `"ask"` |
| `bwlimit` | rclone's bandwidth syntax: `"10M"`, `"750k"`, `"10M:1M"` (upload:download) or `"off"` | `"off"` |
| `scan_concurrency` | folders listed at once by a [Space](space.md) scan, 1 to 16 | 4, or 2 on Mega |
| `rclone_path` | the rclone binary to run | `rclone` on your `PATH` |

An unknown key or a wrong value is reported when ratclone starts, naming the
problem, rather than ignored.

The `--rclone` [option](cli.md) overrides `rclone_path` for one run.
