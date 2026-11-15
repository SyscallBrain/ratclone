# Getting started

## The first run

```sh
ratclone
```

ratclone starts its own rclone in the background and lists the accounts
(rclone calls them *remotes*) from your rclone configuration, plus the local
disk. If you have no accounts yet, press `a` in the accounts list to
[add one](accounts.md), or run `rclone config` first.

To look around without touching anything:

```sh
ratclone --demo
```

## Moving around

The screen has an accounts list on the left and a file pane on the right.
`Tab` moves between them.

- `j` `k` move down and up, `l` or `Enter` opens, `h` or `Backspace` goes to
  the parent folder.
- `gg` and `G` jump to the first and last entry, `Ctrl-d` `Ctrl-u` move half a
  page. Counts work: `5j`.
- `/` filters the folder as you type, `n` `N` cycle the matches.
- `-` goes back to the previous folder.
- `?` shows every key.

## A first copy

1. Press `|` to open a second pane beside the first.
2. In one pane, go to the files you want to copy and mark them with `Space`
   (or `v` for a visual selection).
3. Press `>` (or `F5`) to copy them to the folder open in the other pane, or
   `F6` to move them.

Or copy and paste: `y` copies the marked files, `x` cuts them, and `p` pastes
them in the folder you are in.

The copy runs as an rclone job. Watch it in the info panel, or press `T` for
the [Transfers](transfers.md) tab.

## Quitting

`q` quits. If transfers are still running, ratclone asks whether to keep them
running in the background, wait for them, or cancel them. Background
transfers show up again the next time you open the app. The
[settings](settings.md) can make the choice for you.
