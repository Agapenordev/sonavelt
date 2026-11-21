# Instructions for AI coding agents

Read this before changing anything. It is the short version of how this codebase works and what
a good change looks like here. Humans: [CONTRIBUTING.md](CONTRIBUTING.md) has the same rules
with more context.

## The project

Sonavelt is a native Linux music player in Rust: YouTube (through `yt-dlp`), internet radio
(radio-browser.info) and local files, with synced lyrics (LRCLIB). The UI is immediate-mode
[egui](https://github.com/emilk/egui) 0.36 on eframe with the OpenGL (`glow`) backend. Audio is
decoded by an `ffmpeg` child process and played through `cpal`.

## Commands

```bash
cargo build --release                        # build; the binary is target/release/sonavelt
cargo run --release                          # run it
cargo fmt --check                            # formatting (max_width 160, see rustfmt.toml)
cargo clippy --all-targets -- -D warnings    # lints, must be clean
cargo test                                   # fast offline tests
cargo test -- --ignored                      # live tests: network, sound device, yt-dlp, GitHub
```

CI runs fmt, clippy, `cargo test` and a release build on every pull request. Run all four
before you say a change is done. The ignored tests need `ffmpeg`, `yt-dlp` and a sound device;
run them when you touch `audio.rs`, `sources.rs` or `updater.rs`.

## Where things live

| File | What it owns |
|---|---|
| `src/main.rs` | `App` state, every view, the dock, widgets, styling, and the window loop in `main()` |
| `src/audio.rs` | `Engine`: ffmpeg reader thread, buffer, cpal callback, position, levels |
| `src/sources.rs` | Local scan, YouTube search/stream/download, radio directory, lyrics |
| `src/library.rs` | `Track`, `Library` (saved to `~/.config/sonavelt/library.json`), `Settings`, Smart Shuffle |
| `src/art.rs` | Artwork workers, disk cache, textures, 4×4 color grid, eviction |
| `src/viz.rs` | Aura and blurred-cover backgrounds, palettes, the playing equalizer, fallback covers |
| `src/theme.rs` | Colors and font families. All UI colors come from here |
| `src/logo.rs` | The logo, drawn from distance fields at any size |
| `src/tray.rs` | StatusNotifierItem tray (ksni) |
| `src/updater.rs` | GitHub release check, download, validation and swap |

## Rules that keep this codebase working

**Threads**
- Never block the UI thread. Network, process and disk work goes through `App::spawn`, which
  runs a closure on a thread and sends a `Msg` back; `tick()` applies it.
- `tick()` runs from `eframe::App::logic`, which also runs while the window is hidden and in
  `App::background()` after the window is closed. Keep playback logic there, not in `ui()`.

**Window lifetime**
- Closing the window doesn't end the app when a tray is present: `main()` loops, `App` outlives
  each window, and `attach()` rebinds everything tied to an egui context. Textures belong to
  one context: anything holding a `TextureHandle` must be recreated in `attach()`.
- Wayland can't hide windows (`ViewportCommand::Visible(false)` does nothing there). Don't build
  features on hiding.

**YouTube**
- Streaming uses the `visionos` client (`FAST_CLIENT`), which skips a 5-second ad wait. If you
  change yt-dlp arguments, re-run `cargo test -- --ignored stream` and check the time it prints.
- yt-dlp always runs with its working directory set to the cache folder; interrupted downloads
  leave fragment files behind otherwise.
- Only complete downloads become cache files (`*.part` is renamed on success).

**Data and compatibility**
- `Library` and `Settings` use `#[serde(default)]`. New fields need a default so existing
  libraries keep loading. Never rename or remove a saved field without a migration.
- `updater::REPO` and `updater::ASSET` are what every installed copy looks for. Changing them
  breaks updates for everyone already installed.

**Dependencies**
- Prefer what's already here (egui, ureq 2, image, serde, dirs). A new crate needs a reason the
  existing ones can't cover, stated in the PR.

## Style

- Match the surrounding code: naming, idioms, density.
- Comments explain *why*, in one line, only where the code can't. No comments that restate code.
- Keep diffs small and on-topic. No drive-by refactors, renames or reformatting in a feature PR.
- Put a test next to new non-trivial logic (parsing, ordering, state rules), in the same file's
  `#[cfg(test)] mod tests`. Live tests go behind `#[ignore]` with a comment saying how to run them.

## UI and copy

The look is deliberate. Keep it.

- Colors only from `theme.rs`. The accent is dynamic (`self.aura.accent`, from the cover art). Don't
  hard-code a second accent.
- Icons are Phosphor (`egui_phosphor::regular`), 16 to 19 px, colored with a text color. No emoji
  anywhere in the interface.
- Fonts: Instrument Serif (`serif()`) for big titles and lyrics, Bricolage Grotesque for the
  interface (`semibold()` for emphasis), DM Mono (`FontId::monospace`) for times and counts.
- Spacing on a 4 px grid. Hover and press feedback through `animate_bool_with_time`, about 0.15 s.
- Copy is plain and short: sentence case, no exclamation marks, no trailing period on labels,
  buttons and headings. Say what happens ("Pausing in 30 min"), not how great it is.
- No glows, gradients between hues, or looping animation that doesn't mean something (the playing
  equalizer and beat pulse are the exceptions, because they show the music).

For any visible change, run the app and look at it. Include a screenshot in the PR.

## Releasing (maintainers)

1. Bump `version` in `Cargo.toml`, commit.
2. `cargo build --release`, then tag `vX.Y.Z` and create a GitHub release with:
   - `sonavelt-X.Y.Z-x86_64-linux.tar.gz` (binary, README, LICENSE, icon)
   - `sonavelt-x86_64-linux` (the bare binary: the in-app updater downloads exactly this name)
   - `SHA256SUMS`
3. Installed copies offer the update on their next launch.

## Don't

- Commit `target/`, caches, or anything from `~/.config/sonavelt` / `~/.cache/sonavelt`.
- Put personal listening data in screenshots, tests or fixtures.
- Add telemetry, accounts or any server Sonavelt depends on (see ROADMAP "Not planned").
- Weaken the updater's checks: it must stay HTTPS-only, ELF-validated and atomically renamed.
