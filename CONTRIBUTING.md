# Contributing to ratclone

Thanks for helping. Bug reports, ideas and pull requests are all welcome.
Questions go to [Discussions](https://github.com/SyscallBrain/ratclone/discussions).
For security problems, please follow [SECURITY.md](SECURITY.md) instead of
opening an issue. For anything bigger than a small fix, please open an issue
first so we can agree on the approach before you spend time on it.
Everyone taking part is expected to follow the
[Code of Conduct](CODE_OF_CONDUCT.md).

## Getting started

```sh
git clone https://github.com/SyscallBrain/ratclone
cd ratclone
cargo run -- --demo        # example accounts, nothing touches yours
```

You need Rust 1.88 or newer. rclone v1.68.0 or newer is only needed to try
the real backend and for the tests that talk to `rclone rcd`; without it
those tests skip themselves.

## Before you open a pull request

Run the same checks as CI:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

- UI changes update [insta](https://insta.rs) snapshots in `tests/snapshots/`.
  Review them with `cargo insta review` and commit the accepted ones.
- If you change behaviour, keys or settings, update the user guide in
  `book/src/` (preview with `mdbook serve book --open`) and, for the main
  keys and features, the README.
- Add a line under `[Unreleased]` in [CHANGELOG.md](CHANGELOG.md).

## Code guidelines

- Talk to rclone through the rc API, never by building shell commands.
- Never touch a user's real accounts in tests: use the demo backend or a
  local temporary directory.
- Errors become messages, not panics.
- Code, UI text, docs and commit messages are in English.
