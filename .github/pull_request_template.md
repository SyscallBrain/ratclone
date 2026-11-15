## What and why

<!-- What does this change, and what problem does it solve?
     If it fixes an issue, write "Closes #123". -->

## Checklist

- [ ] `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass
- [ ] Tests cover the change (snapshots reviewed with `cargo insta review` if the UI changed)
- [ ] The README is updated if keys, options or behaviour changed
- [ ] The change has a line under `[Unreleased]` in CHANGELOG.md
