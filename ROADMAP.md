# Roadmap

Where Sonavelt is headed. Nothing here has a date. Items move up when they're wanted, so if
one matters to you, open an issue or add a thumbs-up to an existing one. Want to build one? Read
[CONTRIBUTING.md](CONTRIBUTING.md) and comment on the issue first, so work isn't doubled.

## Shipped in 0.1

- YouTube search and playback, streamed while it downloads, cached for instant replay
- Internet radio: curated picks, most loved, 14 genres, search by name
- Local files, with embedded cover art
- Favorites, playlists with an Add songs picker, queue, recently played, Smart Shuffle
- Time-synced lyrics; kinetic type for songs without them
- Aura, Blurred cover and Solid backgrounds; color moods; interface size
- System tray, close to tray, sleep timer, keyboard shortcuts
- In-app updates from GitHub releases

## Next

- **Media keys and desktop integration (MPRIS).** Play, pause and skip from keyboard media keys,
  the lock screen and desktop widgets, and show the current song in your bar.
- **Real tags for local files.** Read title, artist and album from the file instead of guessing
  from `Artist - Title` filenames.
- **What's playing on the radio.** Show the current song from the station's stream metadata, and
  let you heart it or search it on YouTube.
- **Prebuilt binaries for older distros.** Build releases in CI on an older glibc, so the download
  runs on Ubuntu 24.04 and similar.
- **Packages.** AUR, Flatpak, and a `.deb`/`.rpm`.

## Later

- **Gapless playback and crossfade** between songs.
- **Equalizer and volume normalization**, so loud and quiet songs sit at the same level.
- **Import a YouTube playlist** as a Sonavelt playlist, and export playlists to M3U.
- **Lyrics timing nudge** for files whose lyrics run early or late.
- **Albums and artists views** for local libraries.
- **Light theme.**

## Maybe someday

- Windows and macOS builds.
- Last.fm / ListenBrainz scrobbling.
- Podcasts.

## Not planned

- Accounts, cloud sync or anything that needs a Sonavelt server. Your library stays a local file.
- Downloading for redistribution. The YouTube cache exists for replay and seeking only.
