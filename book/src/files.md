# Files and paths

ratclone follows the XDG base directories: each path below moves with
`XDG_CONFIG_HOME`, `XDG_STATE_HOME`, `XDG_CACHE_HOME` or `XDG_RUNTIME_DIR`
when it is set.

| Path | What |
|---|---|
| `~/.config/ratclone/config.toml` | [settings](configuration.md) |
| `~/.config/ratclone/themes/` | [custom themes](themes.md) |
| `~/.local/state/ratclone/transfers.json` | transfer history |
| `~/.local/state/ratclone/ui.json` | where the Files tab was when you last quit |
| `~/.local/state/ratclone/rcd.log` | the log of ratclone's rclone, rotated at 10 MiB |
| `~/.local/state/ratclone/lists/` | file lists for multi-file pastes |
| `~/.cache/ratclone/scans/` | [Space](space.md) scans |
| `$XDG_RUNTIME_DIR/ratclone/rc.sock` | the socket ratclone and its rclone talk through |

Your accounts stay in rclone's own configuration file, usually
`~/.config/rclone/rclone.conf`; `rclone config file` prints where it is.
ratclone does not keep a copy of them.
