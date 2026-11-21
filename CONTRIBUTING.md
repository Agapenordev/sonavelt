# Contributing to Sonavelt

Thanks for wanting to help. Bug reports, ideas, code, docs and screenshots are all welcome.

## Before you start

- **Bugs:** open an issue with the *Bug report* form. Include what you did, what happened, and
  your distro.
- **Ideas:** check the [roadmap](ROADMAP.md) and existing issues, then open a *Feature request*.
- **Code:** for anything bigger than a small fix, comment on the issue (or open one) first and
  say how you plan to do it. It saves everyone from a PR that goes in a different direction.

Issues labeled `good first issue` are a good way in.

## Set up

You need a recent stable Rust (1.88 or newer), the ALSA headers, and the tools Sonavelt uses at
runtime:

```bash
# Arch:          sudo pacman -S rust alsa-lib ffmpeg yt-dlp deno
# Fedora:        sudo dnf install rust cargo alsa-lib-devel ffmpeg yt-dlp
# Debian/Ubuntu: sudo apt install cargo libasound2-dev ffmpeg   (plus a recent yt-dlp)
# openSUSE:      sudo zypper install rust alsa-devel ffmpeg yt-dlp

git clone https://github.com/Agapenordev/sonavelt
cd sonavelt
cargo run --release
```

A quick tour of the code is in [AGENTS.md](AGENTS.md#where-things-live). It's written for AI
agents but it's the best map for people too.

## Making a change

1. Fork, then branch from `main` with a short descriptive name: `fix/radio-reconnect`,
   `feat/mpris`, `docs/install-fedora`.
2. Keep the change focused on one thing. Unrelated cleanups go in their own PR.
3. Follow the code style and UI rules in [AGENTS.md](AGENTS.md#style). In short: match the
   surrounding code, comment only the *why*, colors from `theme.rs`, Phosphor icons, no emoji, plain copy.
4. Add or update tests for new logic.
5. Check everything passes:

   ```bash
   cargo fmt --check
   cargo clippy --all-targets -- -D warnings
   cargo test
   cargo build --release
   ```

   If you touched audio, YouTube, radio, lyrics or the updater, also run the live tests:
   `cargo test -- --ignored`.
6. For anything you can see, run the app and take a before and after screenshot.

### Commit messages

Write the subject in the imperative, about 50 characters, no trailing period:
`Fix radio reconnect after network drop`. Add a body when the *why* isn't obvious from the diff.

## Pull requests

- Fill in the PR template: what changed, why, how you tested it, screenshots for UI changes.
- Link the issue (`Closes #12`).
- CI runs formatting, clippy, tests and a release build on every PR, and it must pass.
- Keep PRs small enough to review in one sitting. Big features can land in steps.
- A maintainer will review, usually with a few comments. Push fixes to the same branch; there's no
  need to squash, we squash on merge.

### Using AI tools

AI-assisted contributions are fine. Point your agent at [AGENTS.md](AGENTS.md). You're
still responsible for the result: read every line, run it, and test it as you would your own code.
Say in the PR if an AI wrote a large part of it.

## Things that won't be merged

- New dependencies without a clear reason the existing ones can't cover.
- Changes to the look that go against the design rules, without discussing it first.
- Telemetry, accounts, or anything that needs a server.
- Changes to saved data that break existing libraries without a migration.

## Releases

Maintainers release by bumping `version` in `Cargo.toml` on `main`. GitHub Actions builds and
publishes it, and installed copies offer the update in Settings. Contributors never need to bump
the version or build release files.

## License

By contributing you agree that your work is released under the project's [MIT license](LICENSE).
