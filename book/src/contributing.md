# Contributing

Bug reports, ideas and pull requests are welcome. Questions go to
[Discussions](https://github.com/SyscallBrain/ratclone/discussions); security
problems to a
[private report](https://github.com/SyscallBrain/ratclone/security/advisories/new).
[CONTRIBUTING.md](https://github.com/SyscallBrain/ratclone/blob/main/CONTRIBUTING.md)
has the full guide.

## Building and testing

```sh
git clone https://github.com/SyscallBrain/ratclone
cd ratclone
cargo run -- --demo
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

The tests that talk to a real `rclone rcd` skip themselves when rclone is not
installed. UI changes update the [insta](https://insta.rs) snapshots in
`tests/snapshots/`; review them with `cargo insta review`.

## This guide

The guide is an [mdBook](https://rust-lang.github.io/mdBook/) in `book/`.
Preview it with:

```sh
mdbook serve book --open
```

## Project layout

| Path | What |
|---|---|
| `src/app/` | the application state and what each key does |
| `src/ui/` | drawing, one module per part of the screen |
| `src/input/` | key parsing and the key map |
| `src/backend/` | the rclone backend (`rc.rs`) and the demo one (`fake.rs`) |
| `src/rc/` | the rc API client over the unix socket |
| `src/daemon.rs`, `src/reaper.rs` | starting rclone, and stopping it when background transfers end |
| `src/transfer/` | transfer specs and the history |
| `src/scan/` | the Space scans |
| `src/theme/` | built-in and custom themes |
| `tests/` | integration and snapshot tests |
