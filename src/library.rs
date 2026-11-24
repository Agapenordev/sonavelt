//! Tracks, persisted user library (favorites, playlists, play counts) and smart shuffle.

use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Source {
    Local(PathBuf),
    YouTube(String),
    Radio(String),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Track {
    pub title: String,
    pub artist: String,
    pub source: Source,
    #[serde(default)]
    pub duration: Option<f64>,
    /// Artwork URL when the source provides one (radio favicons).
    #[serde(default)]
    pub art: Option<String>,
}

/// Small-capital look-alikes for A..Z (`x` has none, so a NUL placeholder keeps the order).
const SMALL_CAPS: [char; 26] =
    ['ᴀ', 'ʙ', 'ᴄ', 'ᴅ', 'ᴇ', 'ꜰ', 'ɢ', 'ʜ', 'ɪ', 'ᴊ', 'ᴋ', 'ʟ', 'ᴍ', 'ɴ', 'ᴏ', 'ᴘ', 'ǫ', 'ʀ', 'ꜱ', 'ᴛ', 'ᴜ', 'ᴠ', 'ᴡ', '\0', 'ʏ', 'ᴢ'];

/// Map "fancy" Unicode letters (𝘴𝘭𝘰𝘸𝘦𝘥, 𝐁𝐎𝐋𝐃 …) that YouTube titles love back to plain ASCII,
/// since no bundled font has glyphs for them.
pub fn plain_text(s: &str) -> String {
    s.chars()
        .map(|c| match c as u32 {
            cp @ 0x1D400..=0x1D6A3 => {
                let i = (cp - 0x1D400) % 52;
                char::from(if i < 26 { b'A' + i as u8 } else { b'a' + (i - 26) as u8 })
            }
            cp @ 0x1D7CE..=0x1D7FF => char::from(b'0' + ((cp - 0x1D7CE) % 10) as u8),
            cp @ 0xFF01..=0xFF5E => char::from_u32(cp - 0xFEE0).unwrap_or(c), // fullwidth Ａ
            _ => SMALL_CAPS.iter().position(|&s| s == c).map_or(c, |i| char::from(b'A' + i as u8)),
        })
        .collect()
}

impl Track {
    pub fn key(&self) -> String {
        match &self.source {
            Source::Local(p) => format!("l:{}", p.display()),
            Source::YouTube(id) => format!("y:{id}"),
            Source::Radio(u) => format!("r:{u}"),
        }
    }
    pub fn is_radio(&self) -> bool {
        matches!(self.source, Source::Radio(_))
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub enum Background {
    #[default]
    Aura,
    BlurredCover,
    Solid,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub enum Palette {
    #[default]
    Artwork,
    Midnight,
    Ember,
    Forest,
    Rose,
    Mono,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub background: Background,
    pub palette: Palette,
    pub brightness: f32,
    pub motion: bool,
    pub beat: bool,
    pub ui_scale: f32,
    pub close_to_tray: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { background: Background::Aura, palette: Palette::Artwork, brightness: 0.55, motion: true, beat: true, ui_scale: 1.0, close_to_tray: true }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Library {
    pub settings: Settings,
    pub music_dir: String,
    pub favorites: Vec<Track>,
    pub playlists: BTreeMap<String, Vec<Track>>,
    pub plays: HashMap<String, u32>,
    pub recent: Vec<Track>,
    pub volume: f32,
    pub feedback: Feedback,
    /// Seconds actually listened, per song or station: the strongest taste signal.
    pub listened: HashMap<String, f64>,
    /// Songs your stations played while you listened, newest first: (station key, "Artist - Song").
    pub radio_songs: Vec<(String, String)>,
}

/// What you've told Sonavelt you don't want, so recommendations stop offering it.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Feedback {
    /// Songs you un-hearted.
    pub unliked: Vec<Track>,
    /// Songs marked "Not interested" (track keys).
    pub hidden: Vec<String>,
    /// Artists marked "Don't recommend" (see `artist_key`).
    pub hidden_artists: Vec<String>,
    /// Times a song was skipped in its first 30 seconds.
    pub skips: HashMap<String, u32>,
    /// Times a pick was shown on Home. Picks you keep passing over fade, like on YouTube.
    pub impressions: HashMap<String, u32>,
}

/// Lowercased artist without the channel suffixes YouTube adds ("Samara Officiel", "X - Topic",
/// "XVEVO"), so a channel and the name in a title compare equal.
pub fn artist_key(a: &str) -> String {
    let mut a = a.to_lowercase().trim().to_string();
    for suffix in [" - topic", "vevo", " officiel", " official", " music", " tv"] {
        if let Some(rest) = a.strip_suffix(suffix) {
            a = rest.trim().to_string();
        }
    }
    a
}

impl Default for Library {
    fn default() -> Self {
        let music_dir = dirs::audio_dir().unwrap_or_default().display().to_string();
        Self {
            settings: Settings::default(),
            music_dir,
            favorites: vec![],
            playlists: BTreeMap::new(),
            plays: HashMap::new(),
            recent: vec![],
            volume: 0.7,
            feedback: Feedback::default(),
            listened: HashMap::new(),
            radio_songs: vec![],
        }
    }
}

fn path() -> PathBuf {
    dirs::config_dir().unwrap_or_default().join("sonavelt").join("library.json")
}

/// The app used to be called rustwave: carry its library and downloads over on first launch.
pub fn migrate_from_rustwave() {
    for base in [dirs::config_dir(), dirs::cache_dir()].into_iter().flatten() {
        let (old, new) = (base.join("rustwave"), base.join("sonavelt"));
        if old.exists() && !new.exists() {
            let _ = std::fs::rename(old, new);
        }
    }
}

impl Library {
    pub fn load() -> Self {
        let mut lib: Self = std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let lists = [&mut lib.favorites, &mut lib.recent].into_iter().chain(lib.playlists.values_mut());
        for t in lists.flatten() {
            t.title = plain_text(&t.title);
            t.artist = plain_text(t.artist.trim_start_matches("· ")); // radio entries saved with no country
        }
        lib
    }

    pub fn save(&self) {
        let p = path();
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        // write-then-rename so a crash mid-write never corrupts the library
        let tmp = p.with_extension("json.tmp");
        if std::fs::write(&tmp, serde_json::to_vec_pretty(self).unwrap()).is_ok() {
            let _ = std::fs::rename(tmp, p);
        }
    }

    pub fn is_fav(&self, t: &Track) -> bool {
        self.favorites.iter().any(|f| f.source == t.source)
    }

    pub fn toggle_fav(&mut self, t: &Track) {
        let was_fav = self.is_fav(t);
        let fb = &mut self.feedback;
        if was_fav {
            self.favorites.retain(|f| f.source != t.source);
            // un-hearting is a signal too: stop building on this song
            if !fb.unliked.iter().any(|u| u.source == t.source) {
                fb.unliked.push(t.clone());
            }
        } else {
            self.favorites.insert(0, t.clone());
            fb.unliked.retain(|u| u.source != t.source);
            fb.hidden.retain(|k| *k != t.key());
            fb.hidden_artists.retain(|a| *a != artist_key(&t.artist));
        }
        self.save();
    }

    pub fn hide(&mut self, t: &Track) {
        self.feedback.hidden.push(t.key());
        self.save();
    }

    pub fn hide_artist(&mut self, t: &Track) {
        self.feedback.hidden_artists.push(artist_key(&t.artist));
        self.save();
    }

    pub fn skipped(&mut self, t: &Track) {
        *self.feedback.skips.entry(t.key()).or_default() += 1;
        self.save();
    }

    /// Marked "Not interested", un-hearted, or by an artist you asked not to see.
    pub fn dismissed(&self, t: &Track) -> bool {
        let fb = &self.feedback;
        fb.hidden.contains(&t.key()) || fb.unliked.iter().any(|u| u.source == t.source) || fb.hidden_artists.iter().any(|a| by_artist(t, a))
    }

    pub fn played(&mut self, t: &Track) {
        *self.plays.entry(t.key()).or_default() += 1;
        self.recent.retain(|r| r.source != t.source);
        self.recent.insert(0, t.clone());
        self.recent.truncate(50);
        self.save();
    }

    /// Shuffle weighted toward favorites and often-played songs, away from just-played ones,
    /// and never the same artist twice in a row when it can be avoided.
    pub fn smart_shuffle(&self, tracks: &mut Vec<Track>) {
        let mut rng = rand::rng();
        let mut keyed: Vec<(f64, Track)> = tracks
            .drain(..)
            .map(|t| {
                let mut w = 1.0 + (1.0 + *self.plays.get(&t.key()).unwrap_or(&0) as f64).ln();
                if self.is_fav(&t) {
                    w += 2.0;
                }
                if self.recent.iter().take(10).any(|r| r.source == t.source) {
                    w *= 0.25;
                }
                // Efraimidis–Spirakis weighted random order
                (-rng.random::<f64>().max(1e-12).ln() / w, t)
            })
            .collect();
        keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
        *tracks = keyed.into_iter().map(|(_, t)| t).collect();
        for i in 1..tracks.len() {
            if tracks[i].artist == tracks[i - 1].artist
                && let Some(j) = (i + 1..tracks.len()).find(|&j| tracks[j].artist != tracks[i - 1].artist)
            {
                tracks.swap(i, j);
            }
        }
    }
}

/// Is `t` by the artist with key `a`? Checks the channel and the "Artist - Song" title, since
/// lyric and fan channels upload under their own name.
pub fn by_artist(t: &Track, a: &str) -> bool {
    if a.len() < 3 {
        return false;
    }
    let title = t.title.to_lowercase();
    // "Artist - Song", "Song - Artist" (common on lyric channels), and features
    artist_key(&t.artist) == a
        || title.starts_with(&format!("{a} "))
        || ["- ", ", ", "& ", "ft. ", "feat. ", "x "].iter().any(|p| title.contains(&format!("{p}{a}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(artist: &str, n: u32) -> Track {
        Track { title: format!("{artist}{n}"), artist: artist.into(), source: Source::YouTube(format!("{artist}{n}")), duration: None, art: None }
    }

    #[test]
    fn unhearting_and_hiding_dismiss() {
        let mut lib = Library::default();
        let song =
            |title: &str, artist: &str| Track { title: title.into(), artist: artist.into(), source: Source::YouTube(title.into()), duration: None, art: None };
        // toggle_fav and hide save to disk, so set the feedback directly
        let s = song("NEW SAMARA - YA 7OUMA", "Samara Officiel");
        lib.feedback.unliked.push(s.clone());
        assert!(lib.dismissed(&s));
        lib.feedback.hidden_artists.push(artist_key("Samara Officiel"));
        assert_eq!(artist_key("Samara Officiel"), "samara");
        assert!(lib.dismissed(&song("Samara - Yammi", "7clouds")), "artist named in a lyric channel's title");
        assert!(!lib.dismissed(&song("Lana Del Rey - Video Games", "Lana Del Rey")));
        lib.feedback.hidden_artists.push(artist_key("Billie Eilish"));
        assert!(lib.dismissed(&song("lovely - billie eilish ft. khalid (slowed n reverb)", "starclouds")), "song - artist titles");
        assert!(!by_artist(&song("Samaradise - X", "Other"), "samara"), "word boundary");
    }

    #[test]
    fn fancy_letters_become_ascii() {
        assert_eq!(plain_text("𝘴𝘭𝘰𝘸𝘦𝘥 + 𝐁𝐎𝐋𝐃 𝟐"), "slowed + BOLD 2");
        assert_eq!(plain_text("ʏ ᴀ ᴇ ꜱ ꜱ ɪ ꜰ ʏ · Ｌｏｆｉ x"), "Y A E S S I F Y · Lofi x");
    }

    #[test]
    fn smart_shuffle_keeps_all_and_spreads_artists() {
        let lib = Library::default();
        let mut v: Vec<Track> = (0..3).map(|n| t("a", n)).chain((0..3).map(|n| t("b", n))).collect();
        lib.smart_shuffle(&mut v);
        assert_eq!(v.len(), 6);
        assert!(v.windows(2).all(|w| w[0].artist != w[1].artist), "{v:?}");
    }
}
