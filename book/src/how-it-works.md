# How it works

## Its own rclone

ratclone starts its own `rclone rcd`, rclone's remote-control server, on a
private unix socket (`$XDG_RUNTIME_DIR/ratclone/rc.sock`, in a directory only
you can open) and talks to it through the
[rc API](https://rclone.org/rc/), which is only available through that socket.
Browser logins may temporarily open a localhost port for OAuth. ratclone never
builds shell commands.

Listing, renaming, deleting and account changes are rc calls. Copies and moves
are rclone *jobs*, so they keep going while you browse.

## Transfers outlive the app

Because transfers are jobs of the background rclone, they do not depend on the
app. When you quit with transfers running, you choose to keep them running,
wait, or cancel them (or set **On quit with transfers** once in the
[settings](settings.md)). If the terminal is closed or an SSH session drops,
the app leaves and the transfers carry on.

When transfers are left running, a small helper (ratclone itself, started
with a hidden option) watches them, records how each one ends in the transfer
history, and stops the rclone once they are done. Open the app again and they
are all there, running or with how they ended.

## One at a time

Only one ratclone runs at a time, since it owns the rclone and the transfer
history. Starting a second one while the first is open says so.

## Listings

Folders you visited recently open at once with the rows seen last time, while
a fresh listing is fetched; a spinner next to the item count shows it is on
its way. Until it arrives you can move around and filter, but not change
anything, so you never act on a stale list.

## Demo mode

`--demo` replaces rclone with made-up accounts and files held in memory.
Nothing is started and nothing is written to your accounts, which is why the
screenshots and the presentation come from it.
