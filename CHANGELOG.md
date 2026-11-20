# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Fixed

- Keep background transfers running with rclone 1.68.0, whose `job/list`
  response does not include the newer `runningIds` field.
- Offer authorization-token login on rclone versions older than 1.75.0,
  which cannot cancel an in-app browser login cleanly.
- Refuse to rename a folder onto a name the filesystem treats as an existing
  folder of another case, as on FAT, exFAT or SMB drives on Linux. Changing
  only the case of a folder's name (`Photos` to `photos`) deleted every file in
  it, and renaming `a` to `b` merged it into an existing `B`.

## [0.1.1] - 2026-11-15

### Added

- Packages for every release: `.deb` (Debian, Ubuntu), `.rpm` (Fedora,
  openSUSE, RHEL) and Arch `.pkg.tar.zst` for x86_64 and aarch64, a static
  (musl) binary that runs on any Linux, `SHA256SUMS` and signed build
  provenance (`gh attestation verify`).
- `ratclone-bin` in the AUR, with the release binaries.
- A user guide at <https://syscallbrain.github.io/ratclone/>, and a
  presentation at the top of the README.

## [0.1.0] - 2026-11-15

First release.

### Added

- Browse rclone accounts and the local disk with Vim keys, in one pane or two
  side by side (`|`).
- Copy, move, rename and delete, with a question when a name is already
  taken: replace, skip or keep both, once or for all.
- Transfers run as rclone jobs on a private `rclone rcd`, keep going while you
  browse and, if you choose, after you quit; a small helper records how they
  end and stops the rcd when they are done.
- Every operation shows the equivalent `rclone` command.
- An info panel for the selected account and a Space tab that shows where its space goes.
- Accounts can be added from the app.
- Settings screen, eleven built-in themes plus custom ones, Nerd Font or ASCII icons.
- `--demo` mode with example accounts.
- Published on [crates.io](https://crates.io/crates/ratclone) and in the
  [AUR](https://aur.archlinux.org/packages/ratclone).

[Unreleased]: https://github.com/SyscallBrain/ratclone/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/SyscallBrain/ratclone/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/SyscallBrain/ratclone/releases/tag/v0.1.0
