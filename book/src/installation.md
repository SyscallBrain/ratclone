# Installation

ratclone runs on Linux, on x86_64 and aarch64. It needs:

- **rclone v1.68.0 or newer.** ratclone checks the version when it starts and
  says so if it is older.
  Browser login inside the app needs **v1.75.0 or newer**; older supported
  versions use the [authorization-token flow](accounts.md#browser-logins).
- A terminal with truecolor and a [Nerd Font](https://www.nerdfonts.com) for
  the icons. Without one, set `icons = "ascii"` in the
  [configuration](configuration.md) or in the settings (`,`).

## Getting a recent rclone

Fedora, openSUSE Tumbleweed and Arch Linux ship a recent enough rclone, and
the packages below install it for you. **Debian and Ubuntu ship rclone 1.60,
which is too old.** Install the current release from rclone's own site
instead, which puts it in `/usr/bin`:

```sh
sudo -v ; curl https://rclone.org/install.sh | sudo bash
```

rclone's [install page](https://rclone.org/install/) has the other ways,
including its own `.deb` and `.rpm` packages. Check with `rclone version`.

## Packages

Each release has packages and archives on the
[releases page](https://github.com/SyscallBrain/ratclone/releases/latest),
with `SHA256SUMS` listing the checksum of every file. x86_64 files are named
`amd64` or `x86_64`, aarch64 files `arm64` or `aarch64`. The binary inside is
static (musl), so it does not depend on your distribution's glibc.

| System | File | Install |
|--------|------|---------|
| Arch Linux | AUR | `paru -S ratclone-bin` (or `ratclone` to build from source) |
| Arch Linux | `ratclone-VERSION-1-x86_64.pkg.tar.zst` | `sudo pacman -U ./ratclone-*-x86_64.pkg.tar.zst` |
| Debian, Ubuntu | `ratclone_VERSION-1_amd64.deb` | `sudo apt install ./ratclone_*_amd64.deb` |
| Fedora, RHEL | `ratclone-VERSION-1.x86_64.rpm` | `sudo dnf install ./ratclone-*.x86_64.rpm` |
| openSUSE | `ratclone-VERSION-1.x86_64.rpm` | `sudo zypper install --allow-unsigned-rpm ./ratclone-*.x86_64.rpm` |
| Any Linux | `ratclone-VERSION-x86_64-unknown-linux-musl.tar.gz` | unpack it and put `ratclone` on your `PATH` |

The Arch and RPM packages depend on rclone and install it. The `.deb` only
suggests it, because the rclone in Debian and Ubuntu is too old: install it
as described above.

Check a download, and its build provenance (which proves it was built by
this repository's release workflow), with the
[GitHub CLI](https://cli.github.com):

```sh
sha256sum --check --ignore-missing SHA256SUMS
gh attestation verify ratclone_*_amd64.deb --repo SyscallBrain/ratclone
```

## From crates.io

With Rust 1.88 or newer:

```sh
cargo install --locked ratclone
```

## From source

```sh
git clone https://github.com/SyscallBrain/ratclone
cd ratclone
cargo build --release
./target/release/ratclone
```

## Try it first

```sh
ratclone --demo
```

Demo mode shows made-up accounts and files. Nothing touches rclone or your
accounts, so it is safe to try every key.
