# Troubleshooting

## "rclone not found"

ratclone runs `rclone` from your `PATH`. Install it (see
[Installation](installation.md)), or point ratclone at it with `rclone_path`
in the [configuration](configuration.md) or `--rclone PATH`.

## "rclone … is too old"

ratclone needs rclone v1.68.0 or newer, and Debian and Ubuntu ship 1.60.
Install the current release from [rclone.org](https://rclone.org/install/):

```sh
sudo -v ; curl https://rclone.org/install.sh | sudo bash
```

If you have several rclones, `rclone_path` chooses which one ratclone runs.

## "rclone rcd did not start" or "cannot reach rclone"

The background rclone failed to start or stopped answering. Its log is in
`~/.local/state/ratclone/rcd.log`; the last lines usually say why. A stale
socket from a crash is cleaned up on the next start. If `XDG_RUNTIME_DIR` is
not set (some `su` and container sessions), the socket goes to
`/tmp/ratclone-UID/` instead.

## A copy or a listing fails

The error in the app is rclone's own. Try the same thing with the `rclone`
command shown for the transfer, and check `rcd.log`. Expired logins are a
common cause: `A` in the accounts list signs in again.

## Icons show as boxes or question marks

The icons need a [Nerd Font](https://www.nerdfonts.com) in your terminal.
Install one, or switch to ASCII icons: **Icons** in the settings (`,`), or
`icons = "ascii"` in `config.toml`.

## No colours, or wrong colours

ratclone draws in truecolor. Most terminals support it; inside tmux, add
`set -as terminal-features ",*:RGB"` to `~/.tmux.conf`. When the `NO_COLOR`
environment variable is set, colours are turned off.

## "Window too small"

The terminal is too small to draw the app: make it bigger. Below 80 columns
the accounts list is hidden to leave room for the files.

## The browser does not open for a login

Over SSH there is no browser, so ratclone shows an `rclone authorize` command
to run on a machine that has one. If it guessed wrong, `L` switches between
the two ways. See [Browser logins](accounts.md#browser-logins).

## Still stuck?

Ask in [Discussions](https://github.com/SyscallBrain/ratclone/discussions), or
[open an issue](https://github.com/SyscallBrain/ratclone/issues/new/choose)
with your ratclone and rclone versions and the last lines of `rcd.log`.
