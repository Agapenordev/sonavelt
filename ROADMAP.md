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

## Shipped in 0.1.1

- Automatic releases from CI, built against glibc 2.35 so downloads run on Ubuntu 22.04, Debian 12,
  Fedora 36 and newer

## Shipped in 0.2

- **Media keys and desktop integration (MPRIS).** Play, pause, skip and seek from media keys, the
  lock screen and desktop widgets, which also show the current song with its artwork.
- **Real tags for local files.** Title, artist and length come from the file's tags, with the
  `Artist - Title` filename as the fallback.
- **What's playing on the radio.** The song a station is playing shows under it, in the player and
  in media widgets, with a button to find it on YouTube.

## Next

- **Packages.** AUR, Flatpak, and a `.deb`/`.rpm`.
- **Albums and artists views** for local libraries, now that tags are read.

## Later

- **Gapless playback and crossfade** between songs.
- **Equalizer and volume normalization**, so loud and quiet songs sit at the same level.
- **Import a YouTube playlist** as a Sonavelt playlist, and export playlists to M3U.
- **Lyrics timing nudge** for files whose lyrics run early or late.
- **Light theme.**

## Maybe someday

- Windows and macOS builds.
- Last.fm / ListenBrainz scrobbling.
- Podcasts.

## Not planned

- Accounts, cloud sync or anything that needs a Sonavelt server. Your library stays a local file.
- Downloading for redistribution. The YouTube cache exists for replay and seeking only.
