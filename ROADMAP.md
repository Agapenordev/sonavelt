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

## Shipped in 0.3

- **Home page.** Spotify-style quick tiles and shelves of picks: Made for you, a shelf per
  interest, Jump back in and Stations for you.
- **Recommendations that learn like YouTube's.** Interests from your songs and your stations'
  genres, weighted by listening time; skips, un-hearts, *Not interested*, *Don't recommend this
  artist* and ignored picks all teach it.

## Done, coming in the next release

- **Recommendations only learn from real listening.** A play counts after 30 seconds, and taste
  comes from listening time, so testing a song for a few seconds doesn't teach anything.
  *Remove* in Recently played erases a song from your history and your picks.
- **Cleaner interface.** Buttons and chips sized like controls instead of pills, no drop shadows
  on cards, a quiet *Live* label, plainer shelf titles and copy.
- **Radio that works.** Dead streams are left out of station lists, a saved station that moved is
  found again automatically, and stations get their real logos from their own websites.
- **Sound.** Even out loudness between songs, and Bass, Vocal and Bright equalizer presets.
- **Playlists in and out.** Save a pasted YouTube playlist as a Sonavelt playlist; export any
  playlist as M3U.
- **Lyrics timing nudge** for lyrics that run early or late, remembered per song.
- **Block genres.** Blocked genres are never recommended and drop out of radio lists.
- **Heard on radio.** Every song your stations played while you listened, to play or heart.
- **Private session**, and **why this pick?** on every recommendation.

## Next

- **Packages.** AUR, Flatpak, and a `.deb`/`.rpm`.
- **Albums and artists views** for local libraries, now that tags are read.

## Later

- **Gapless playback and crossfade** between songs.
- **Light theme.**

## Maybe someday

- Windows and macOS builds.
- Last.fm / ListenBrainz scrobbling.
- Podcasts.

## Not planned

- Accounts, cloud sync or anything that needs a Sonavelt server. Your library stays a local file.
- Downloading for redistribution. The YouTube cache exists for replay and seeking only.
