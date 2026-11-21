## What this changes

<!-- One or two sentences. Link the issue: Closes #123 -->

## Why

<!-- The problem it solves, or the reason for the approach you chose -->

## How I tested it

<!-- What you ran and what you checked by hand. Mention the live tests if you ran them. -->

## Screenshots

<!-- Before and after, for anything visible. Delete this section otherwise. -->

## Checklist

- [ ] `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` and `cargo build --release` pass
- [ ] Ran the app and tried the change
- [ ] New logic has tests; ran `cargo test -- --ignored` if I touched audio, sources or the updater
- [ ] Follows the style and UI rules in [AGENTS.md](../AGENTS.md) (theme colors, Phosphor icons, no emoji, plain copy)
- [ ] Saved data still loads (new fields have defaults; no renamed or removed fields)
- [ ] No new dependency, or the reason is explained above
