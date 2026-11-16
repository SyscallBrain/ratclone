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

CI and releases require the real integration suite against both rclone 1.68.0
and the current official release, plus a focused browser-login cancellation
test on rclone 1.75.0 (the minimum for that feature). The existing
`fmt, clippy, test` check name is unchanged. To reproduce a run without
replacing your installed rclone:

```sh
test_dir=$(mktemp -d)
bash scripts/install-test-rclone.sh 1.68.0 "$test_dir" # or latest
PATH="$test_dir:$PATH" RATCLONE_TEST_RCLONE_VERSION=$(cat "$test_dir/version") cargo test --locked
rm -rf "$test_dir"
```

The installer verifies the archive against the official `SHA256SUMS` and checks
the binary's version. Setting `RATCLONE_TEST_RCLONE_VERSION` makes a missing or
different rclone fail the run; the OAuth test also fails if its localhost port
is unavailable. Tests use temporary rclone configurations and local files,
without your cloud credentials or accounts. These checks cover the rc API and
local backend; provider-specific behavior still depends on rclone and the provider.
Test daemons run with a disposable HOME and a cleared environment, so
environment-defined remotes and provider credentials cannot override fixtures.
The minimum-version suite verifies that older rclone rejects browser login
before starting a listener and offers token authorization instead. The
1.75.0 and current-version checks exercise real listener startup and cancellation.

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
