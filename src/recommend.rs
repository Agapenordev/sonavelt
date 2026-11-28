//! Home page recommendations, modeled on how YouTube builds a home feed.
//!
//! 1. Interests. Your listening splits into separate interests instead of one blended taste:
//!    the songs you play (labeled by their style, e.g. "Slowed + reverb") and the genre families
//!    of the stations you play (Dance & electronic, Chill & ambient, ...). Each is weighted by
//!    listening time, with play counts standing in where time hasn't been recorded yet.
//! 2. Seeds per interest. Songs qualify only if you favorited them, put them in a playlist or
//!    really listened (skips subtracted); un-hearted and hidden songs never count. Genre
//!    interests are seeded with songs your own stations played: the ones heard while you
//!    listened, plus what those stations are playing right now, looked up on YouTube.
//! 3. Candidates: YouTube's own Mix for every seed, fetched in parallel, first 15 songs only
//!    (after that YouTube drifts toward generic hits).
//! 4. Ranking within each interest: songs several strong seeds agree on come first, earlier
//!    positions count more. Songs you know, hid or un-hearted are dropped, blocked artists too;
//!    artists of un-hearted songs are pushed down; picks you were shown but never played fade
//!    each time, artist by artist; at most two songs per artist.
//! 5. Shelves: Made for you mixes every interest by weight, then one shelf per interest.
//!
//! Results are cached, so Home opens instantly and refreshes in the background.

use crate::library::{Library, Source, Track, artist_key, by_artist, in_genre};
use crate::sources;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

const SONG_SEEDS: usize = 5;
const GENRE_SEEDS: usize = 3;
const MIX_LEN: usize = 15;
const MAX_AGE_SECS: u64 = 6 * 3600;
/// Seconds one play stands for in older history, recorded before listening time was.
const PLAY_SECS: f64 = 180.0;
/// A station needs this much listening before its genre becomes an interest.
const STATION_SECS: f64 = 300.0;

/// Genre families: radio-browser tags that belong together.
const FAMILIES: &[(&str, &[&str])] = &[
    (
        "Dance & electronic",
        &[
            "dance",
            "electronic",
            "electronica",
            "electro",
            "house",
            "deep house",
            "edm",
            "techno",
            "trance",
            "bass",
            "club",
            "mashup",
            "#mashup",
            "remix",
            "dj",
        ],
    ),
    (
        "Chill & ambient",
        &["chillout", "chill", "ambient", "downtempo", "lounge", "sensual vocals", "beach music", "atmospheric", "relax", "relaxing", "easy listening"],
    ),
    ("Lofi", &["lofi", "lo-fi", "lofi hip hop", "study", "beats"]),
    ("Jazz", &["jazz", "smooth jazz", "swing", "bossa nova", "soul"]),
    ("Hip hop", &["hiphop", "hip-hop", "hip hop", "rap", "trap", "rnb", "r&b"]),
    ("Rock", &["rock", "alternative", "indie", "metal", "punk", "grunge"]),
    ("Synthwave", &["synthwave", "retrowave", "darksynth", "chillsynth", "outrun", "80s"]),
    ("Classical", &["classical", "orchestra", "piano", "baroque", "opera"]),
];

#[derive(Clone, Serialize, Deserialize)]
pub struct Shelf {
    pub title: String,
    pub subtitle: String,
    pub tracks: Vec<Track>,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Home {
    pub made_at: u64,
    pub shelves: Vec<Shelf>,
    /// Built from your listening (false: the starter page for a new library).
    #[serde(default)]
    pub seeded: bool,
    /// Why each pick is there, by track key ("Because you play Runaway").
    #[serde(default)]
    pub why: HashMap<String, String>,
}

struct Interest {
    title: String,
    subtitle: String,
    /// Share of your listening, 0..1.
    weight: f64,
    /// Seeds with their strength (0.5 to 1).
    seeds: Vec<(Track, f64)>,
    /// Genre interests: songs heard on these stations ("Artist - Song"), newest first.
    songs: Vec<String>,
    /// Genre interests: stream URLs of your stations, most listened first.
    stations: Vec<String>,
}

/// What the recommender needs from the library, copied so it can run on a worker thread.
pub struct Taste {
    interests: Vec<Interest>,
    /// Songs never to recommend: already yours, hidden or un-hearted.
    known: HashSet<String>,
    /// The same, as `song` keys: another upload of a song you know is not new.
    known_songs: HashSet<String>,
    /// Your songs are slowed or sped-up versions, so those versions are welcome in picks.
    variants: bool,
    /// Artists you asked not to see.
    blocked: Vec<String>,
    /// Genres you blocked.
    genres: Vec<String>,
    /// Artists of songs you un-hearted: still possible, but pushed down.
    cooled: Vec<String>,
    /// Times each pick was shown on Home.
    shown: HashMap<String, u32>,
    /// Artists you've actually played: showing them again is never "ignored".
    heard: HashSet<String>,
    radio_tag: Option<String>,
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn cache_path() -> std::path::PathBuf {
    // v5: picks carry their reasons; older caches have none
    dirs::cache_dir().unwrap_or_default().join("sonavelt").join("home-v5.json")
}

impl Home {
    pub fn load() -> Self {
        std::fs::read_to_string(cache_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let p = cache_path();
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        let _ = std::fs::write(p, serde_json::to_vec(self).unwrap());
    }

    pub fn stale(&self) -> bool {
        self.shelves.is_empty() || now_secs().saturating_sub(self.made_at) > MAX_AGE_SECS
    }

    pub fn newer_than(&self, secs: u64) -> bool {
        now_secs().saturating_sub(self.made_at) < secs
    }

    /// "updated 2 h ago"
    pub fn age(&self) -> String {
        match now_secs().saturating_sub(self.made_at) / 60 {
            0 => "updated just now".into(),
            m if m < 60 => format!("updated {m} min ago"),
            m => format!("updated {} h ago", m / 60),
        }
    }
}

/// Who a song is really by, for "one per artist" rules: the "Artist - Song" part of the title
/// when there is one (reupload and lyric channels hide the artist), else the channel.
fn who(t: &Track) -> String {
    match t.title.split_once(" - ") {
        Some((a, _)) if !a.trim().is_empty() && a.len() < 40 => artist_key(a),
        _ => artist_key(&t.artist),
    }
}

/// Words in titles that name an upload, not a song.
const NOISE: &[&str] = &["official", "video", "audio", "lyrics", "lyric", "visualizer", "hd", "4k", "mv", "slowed", "reverb", "nightcore", "8d"];

/// One key for every upload of a song: the artist and the title without "(Official Video)",
/// "[Lyrics]" or "slowed + reverb".
fn song(t: &Track) -> String {
    let mut depth = 0;
    let bare: String = t
        .title
        .chars()
        .filter(|&c| match c {
            '(' | '[' => {
                depth += 1;
                false
            }
            ')' | ']' => {
                depth -= 1;
                false
            }
            _ => depth <= 0,
        })
        .collect();
    let name = bare.split_once(" - ").map_or(bare.as_str(), |(_, s)| s).to_lowercase().replace("sped up", "");
    let words: Vec<&str> = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty() && !NOISE.contains(w)).collect();
    format!("{}|{}", who(t), words.join(" "))
}

/// Slowed, sped-up and other remakes of a song.
fn variant(t: &Track) -> bool {
    let title = t.title.to_lowercase();
    ["slowed", "reverb", "sped up", "nightcore", "8d", "karaoke", "instrumental"].iter().any(|w| title.contains(w))
}

/// Seconds you really listened. Older history has only play counts, and back then a play was
/// counted on start, so one or two plays may have been a quick test: those count from three up.
fn listened(lib: &Library, t: &Track) -> f64 {
    match lib.listened.get(&t.key()) {
        Some(secs) => *secs,
        None => match *lib.plays.get(&t.key()).unwrap_or(&0) {
            n if n >= 3 => n as f64 * PLAY_SECS,
            _ => 0.0,
        },
    }
}

/// Name a song interest after the style its seeds share, the way YouTube labels a mix.
fn style_of(seeds: &[(Track, f64)]) -> &'static str {
    let share = |words: &[&str]| seeds.iter().filter(|(t, _)| words.iter().any(|w| t.title.to_lowercase().contains(w))).count() * 2 > seeds.len();
    if share(&["slowed", "reverb"]) {
        "Slowed + reverb"
    } else if share(&["sped up", "nightcore"]) {
        "Sped up"
    } else if share(&["lofi", "lo-fi"]) {
        "Lofi"
    } else if share(&["remix", "mashup"]) {
        "Remixes"
    } else {
        "Like what you play"
    }
}

pub fn taste(lib: &Library) -> Taste {
    let fb = &lib.feedback;
    let in_playlist: HashSet<String> = lib.playlists.values().flatten().map(Track::key).collect();

    // songs: what counts is how much you listened, like watch time on YouTube. Starting a song
    // (to test it, by accident) says nothing; recency only scales real listening.
    let mut recency: HashMap<String, (Track, f64)> = HashMap::new();
    for (i, t) in lib.recent.iter().enumerate() {
        recency.entry(t.key()).or_insert_with(|| (t.clone(), 0.9f64.powi(i as i32)));
    }
    for t in lib.favorites.iter().chain(lib.playlists.values().flatten()) {
        recency.entry(t.key()).or_insert_with(|| (t.clone(), 0.0));
    }
    let mut songs: Vec<(Track, f64)> = recency
        .into_values()
        .filter(|(t, _)| !t.is_radio() && !lib.dismissed(t))
        .filter_map(|(t, recent)| {
            let len = t.duration.unwrap_or(200.0).max(60.0);
            let skips = *fb.skips.get(&t.key()).unwrap_or(&0) as f64;
            // full listens, each skip taking one away
            let listens = ((listened(lib, &t) - skips * len) / len).max(0.0);
            let (fav, pl) = (lib.is_fav(&t), in_playlist.contains(&t.key()));
            let s = 2.0 * (1.0 + listens).ln() * (0.5 + 0.5 * recent) + if fav { 2.5 } else { 0.0 } + if pl { 1.0 } else { 0.0 };
            // about one whole listen, a heart or a playlist: anything less is not a taste yet
            (fav || pl || listens >= 0.9).then_some((t, s))
        })
        .collect();
    songs.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.title.cmp(&b.0.title)));
    let mut artists = HashSet::new();
    let picked: Vec<(Track, f64)> = songs.into_iter().filter(|(t, _)| artists.insert(who(t))).take(SONG_SEEDS).collect();
    let top = picked.first().map_or(1.0, |(_, s)| *s);
    let song_seeds: Vec<(Track, f64)> = picked.into_iter().map(|(t, s)| (t, 0.5 + 0.5 * s / top)).collect();
    let song_time: f64 = song_seeds.iter().map(|(t, _)| listened(lib, t).max(PLAY_SECS)).sum();

    // stations: listening time per genre family, from their radio-browser tags
    let mut family_time = vec![0.0; FAMILIES.len()];
    let mut family_stations: Vec<Vec<(f64, Track)>> = vec![vec![]; FAMILIES.len()];
    let mut tag_time: HashMap<String, f64> = HashMap::new();
    let mut seen_stations = HashSet::new();
    for t in lib.recent.iter().chain(&lib.favorites).filter(|t| t.is_radio() && !lib.dismissed(t) && seen_stations.insert(t.key())) {
        let secs = listened(lib, t);
        if secs < STATION_SECS {
            continue; // tuned in for a moment: not an interest
        }
        let Some((_, list)) = t.artist.rsplit_once(" · ") else { continue };
        let tags: Vec<String> = list.split(',').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect();
        for tag in &tags {
            *tag_time.entry(tag.clone()).or_default() += secs;
        }
        for (f, (_, family_tags)) in FAMILIES.iter().enumerate() {
            if tags.iter().any(|tag| family_tags.contains(&tag.as_str())) {
                family_time[f] += secs;
                family_stations[f].push((secs, t.clone()));
            }
        }
    }

    let mut interests = vec![];
    if !song_seeds.is_empty() {
        let lead = &song_seeds[0].0;
        interests.push(Interest {
            title: style_of(&song_seeds).into(),
            subtitle: format!("Because you play {}", lead.title.split(" (").next().unwrap_or(&lead.title)),
            weight: song_time,
            seeds: song_seeds,
            songs: vec![],
            stations: vec![],
        });
    }
    for (f, (name, _)) in FAMILIES.iter().enumerate() {
        if family_time[f] > 0.0 {
            let mut stations = std::mem::take(&mut family_stations[f]);
            stations.sort_by(|a, b| b.0.total_cmp(&a.0));
            let keys: HashSet<String> = stations.iter().map(|(_, t)| t.key()).collect();
            let mut seen = HashSet::new();
            let songs =
                lib.radio_songs.iter().filter(|(k, s)| keys.contains(k) && seen.insert(s.to_lowercase())).map(|(_, s)| s.clone()).take(GENRE_SEEDS).collect();
            let stations = stations
                .into_iter()
                .filter_map(|(_, t)| match t.source {
                    Source::Radio(url) => Some(url),
                    _ => None,
                })
                .take(GENRE_SEEDS)
                .collect();
            interests.push(Interest {
                title: (*name).into(),
                subtitle: format!("From the songs on your {} stations", name.to_lowercase()),
                weight: family_time[f],
                seeds: vec![],
                songs,
                stations,
            });
        }
    }
    interests.sort_by(|a, b| b.weight.total_cmp(&a.weight));
    interests.truncate(4);
    let total: f64 = interests.iter().map(|i| i.weight).sum();
    for i in &mut interests {
        i.weight /= total.max(1.0);
    }

    let known: HashSet<String> = lib
        .recent
        .iter()
        .chain(&lib.favorites)
        .chain(lib.playlists.values().flatten())
        .chain(&fb.unliked)
        .map(Track::key)
        .chain(fb.hidden.iter().cloned())
        .collect();
    let loved: HashSet<String> = lib.favorites.iter().map(|t| artist_key(&t.artist)).collect();
    let cooled = fb.unliked.iter().map(|t| artist_key(&t.artist)).filter(|a| !loved.contains(a)).collect();
    let known_songs = lib.recent.iter().chain(&lib.favorites).chain(lib.playlists.values().flatten()).chain(&fb.unliked).map(song).collect();
    let variants = interests.first().is_some_and(|i| matches!(i.title.as_str(), "Slowed + reverb" | "Sped up"));
    let heard = lib.recent.iter().chain(&lib.favorites).map(who).collect();
    let radio_tag = tag_time.into_iter().max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0))).map(|(t, _)| t);
    Taste {
        interests,
        known,
        known_songs,
        variants,
        blocked: fb.hidden_artists.clone(),
        genres: fb.hidden_genres.clone(),
        cooled,
        shown: fb.impressions.clone(),
        heard,
        radio_tag,
    }
}

/// Songs only, no hour-long compilations or 20-second clips.
fn song_length(t: &Track) -> bool {
    t.duration.is_some_and(|d| (60.0..=600.0).contains(&d))
}

/// Combine one interest's weighted mixes into a ranked list of new songs, each with the reason
/// it was picked (the seed that pointed to it most strongly).
fn rank(mixes: &[(f64, String, Vec<Track>)], taste: &Taste) -> Vec<(Track, String)> {
    // unfamiliar artists: the most times any of their songs was shown to you without a play
    let mut artist_shown: HashMap<String, u32> = HashMap::new();
    for t in mixes.iter().flat_map(|(_, _, m)| m) {
        let a = who(t);
        if !taste.heard.contains(&a) {
            let n = artist_shown.entry(a).or_default();
            *n = (*n).max(*taste.shown.get(&t.key()).unwrap_or(&0));
        }
    }

    // scored per song, so its uploads add up and show once, as the artist's own upload if possible
    let mut score: HashMap<String, (Track, f64)> = HashMap::new();
    let mut why: HashMap<String, (f64, &str)> = HashMap::new();
    let better = |t: &Track| (artist_key(&t.artist) == who(t), !variant(t));
    for (weight, reason, mix) in mixes {
        for (pos, t) in mix.iter().take(MIX_LEN).enumerate() {
            let key = song(t);
            if taste.known.contains(&t.key())
                || taste.known_songs.contains(&key)
                || !song_length(t)
                || taste.blocked.iter().any(|a| by_artist(t, a))
                || taste.genres.iter().any(|g| in_genre(t, g))
            {
                continue;
            }
            let cooled = if taste.cooled.iter().any(|a| by_artist(t, a)) { 0.3 } else { 1.0 };
            // shown and passed over: the song fades fast, an unfamiliar artist a little slower
            let ignored = 0.6f64.powi(*taste.shown.get(&t.key()).unwrap_or(&0) as i32) * 0.8f64.powi(*artist_shown.get(&who(t)).unwrap_or(&0) as i32);
            let remake = if variant(t) && !taste.variants { 0.4 } else { 1.0 };
            let s = weight * cooled * ignored * remake / (1.0 + 0.12 * pos as f64);
            let entry = score.entry(key.clone()).or_insert_with(|| (t.clone(), 0.0));
            if better(t) > better(&entry.0) {
                entry.0 = t.clone();
            }
            entry.1 += s;
            let best = why.entry(key).or_insert((0.0, reason));
            if s > best.0 {
                *best = (s, reason);
            }
        }
    }
    let mut ranked: Vec<(Track, f64)> = score.into_values().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.title.cmp(&b.0.title)));
    // artists you already play get one slot, so most picks are artists new to you
    let mut per_artist: HashMap<String, usize> = HashMap::new();
    ranked
        .into_iter()
        .filter(|(t, _)| {
            let a = who(t);
            let cap = if taste.heard.contains(&a) { 1 } else { 2 };
            let n = per_artist.entry(a).or_default();
            *n += 1;
            *n <= cap
        })
        .map(|(t, _)| {
            let reason = why.get(&song(&t)).map_or("", |(_, r)| r).to_string();
            (t, reason)
        })
        .take(28)
        .collect()
}

/// Made for you: picks from every interest in proportion to its weight, interleaved.
fn blend(ranked: &[(f64, Vec<Track>)], n: usize) -> Vec<Track> {
    let mut quotas: Vec<usize> = ranked.iter().map(|(w, _)| ((w * n as f64).round() as usize).max(2)).collect();
    let mut cursors = vec![0; ranked.len()];
    let (mut out, mut seen) = (vec![], HashSet::new());
    while out.len() < n && quotas.iter().any(|&q| q > 0) {
        for (i, (_, list)) in ranked.iter().enumerate() {
            if quotas[i] == 0 {
                continue;
            }
            match list[cursors[i]..].iter().position(|t| !seen.contains(&t.key())) {
                Some(skip) => {
                    let t = &list[cursors[i] + skip];
                    cursors[i] += skip + 1;
                    seen.insert(t.key());
                    out.push(t.clone());
                    quotas[i] -= 1;
                }
                None => quotas[i] = 0,
            }
        }
    }
    out.truncate(n);
    out
}

/// Build the Home shelves. Blocking and networked: run it on a worker thread.
pub fn build(taste: Taste) -> Home {
    let mut shelves = Vec::new();
    let mut home_why = HashMap::new();
    let seeded = !taste.interests.is_empty();
    if !seeded {
        // nothing to go on yet: something to start with
        if let Ok(popular) = sources::youtube_search("top hits this week", 20) {
            let tracks: Vec<Track> = popular
                .into_iter()
                .filter(|t| song_length(t) && !taste.blocked.iter().any(|a| by_artist(t, a)) && !taste.genres.iter().any(|g| in_genre(t, g)))
                .collect();
            shelves.push(Shelf {
                title: "Popular right now".into(),
                subtitle: "Heart a few songs or listen a while and this page learns your taste".into(),
                tracks,
            });
        }
    } else {
        // every interest's seeds and mixes, all in parallel
        let ranked: Vec<(f64, Vec<(Track, String)>)> = std::thread::scope(|s| {
            let jobs: Vec<_> = taste
                .interests
                .iter()
                .map(|interest| {
                    let taste = &taste;
                    s.spawn(move || {
                        let short = |t: &Track| t.title.split(" (").next().unwrap_or(&t.title).trim().to_string();
                        let mut seeds: Vec<(Track, f64, String)> =
                            interest.seeds.iter().map(|(t, w)| (t.clone(), *w, format!("Because you play {}", short(t)))).collect();
                        if !interest.stations.is_empty() {
                            let family = interest.title.to_lowercase();
                            seeds.extend(station_seeds(interest).into_iter().map(|t| {
                                let reason = format!("Because your {family} stations played {}", short(&t));
                                (t, 0.8, reason)
                            }));
                        }
                        let mixes: Vec<(f64, String, Vec<Track>)> = std::thread::scope(|s2| {
                            let jobs: Vec<_> = seeds.iter().map(|(t, w, why)| s2.spawn(move || (*w, why.clone(), mix_for(t)))).collect();
                            jobs.into_iter().filter_map(|j| j.join().ok()).collect()
                        });
                        (interest.weight, rank(&mixes, taste))
                    })
                })
                .collect();
            jobs.into_iter().map(|j| j.join().unwrap_or_default()).collect()
        });

        let why: HashMap<String, String> = ranked.iter().flat_map(|(_, l)| l).filter(|(_, r)| !r.is_empty()).map(|(t, r)| (t.key(), r.clone())).collect();
        let ranked: Vec<(f64, Vec<Track>)> = ranked.into_iter().map(|(w, l)| (w, l.into_iter().map(|(t, _)| t).collect())).collect();
        home_why = why;
        let made = blend(&ranked, 16);
        let mut used: HashSet<String> = made.iter().map(Track::key).collect();
        if !made.is_empty() {
            shelves.push(Shelf { title: "Made for you".into(), subtitle: "A mix of everything you listen to".into(), tracks: made });
        }
        for (interest, (_, list)) in taste.interests.iter().zip(&ranked) {
            let rest: Vec<Track> = list.iter().filter(|t| used.insert(t.key())).take(12).cloned().collect();
            if rest.len() >= 6 {
                shelves.push(Shelf { title: interest.title.clone(), subtitle: interest.subtitle.clone(), tracks: rest });
            }
        }
    }
    let stations = taste.radio_tag.as_ref().and_then(|tag| {
        let found = sources::radio_search(tag, true).ok()?;
        Some(Shelf { title: "Stations".into(), subtitle: format!("Because you listen to {tag}"), tracks: found.into_iter().take(12).collect() })
    });
    shelves.push(stations.unwrap_or_else(|| Shelf {
        title: "Stations to try".into(),
        subtitle: "Hand-picked internet radio".into(),
        tracks: sources::curated_radios(),
    }));
    Home { made_at: now_secs(), shelves, seeded, why: home_why }
}

/// Seed songs for a genre interest: songs heard on its stations, topped up with what those
/// stations are playing right now, each looked up on YouTube.
fn station_seeds(interest: &Interest) -> Vec<Track> {
    let mut titles = interest.songs.clone();
    if titles.len() < GENRE_SEEDS {
        let live: Vec<String> = std::thread::scope(|s| {
            let jobs: Vec<_> = interest.stations.iter().map(|url| s.spawn(move || sources::radio_now(url))).collect();
            jobs.into_iter().filter_map(|j| j.join().ok().flatten()).collect()
        });
        for t in live {
            if !titles.iter().any(|x| x.eq_ignore_ascii_case(&t)) {
                titles.push(t);
            }
        }
    }
    std::thread::scope(|s| {
        let jobs: Vec<_> =
            titles.iter().take(GENRE_SEEDS).map(|title| s.spawn(move || sources::youtube_search(title, 3).ok()?.into_iter().find(song_length))).collect();
        jobs.into_iter().filter_map(|j| j.join().ok().flatten()).collect()
    })
}

/// A seed's mix; songs from your files are looked up on YouTube first.
fn mix_for(seed: &Track) -> Vec<Track> {
    let id = match &seed.source {
        Source::YouTube(id) => Some(id.clone()),
        _ => sources::youtube_search(&format!("{} {}", seed.artist, seed.title), 1).ok().and_then(|r| match r.first().map(|t| &t.source) {
            Some(Source::YouTube(id)) => Some(id.clone()),
            _ => None,
        }),
    };
    id.and_then(|id| sources::youtube_mix(&id, MIX_LEN).ok()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Feedback;

    fn yt(id: &str, artist: &str, secs: f64) -> Track {
        Track { title: id.into(), artist: artist.into(), source: Source::YouTube(id.into()), duration: Some(secs), art: None }
    }

    fn radio(name: &str, tags: &str) -> Track {
        Track { title: name.into(), artist: format!("DE · {tags}"), source: Source::Radio(format!("http://{name}")), duration: None, art: None }
    }

    fn song_seeds(lib: &Library) -> Vec<String> {
        taste(lib).interests.into_iter().flat_map(|i| i.seeds).map(|(t, _)| t.title).collect()
    }

    fn taste_with(known: &[&str], blocked: &[&str], cooled: &[&str], shown: &[(&str, u32)]) -> Taste {
        Taste {
            interests: vec![],
            known: known.iter().map(|k| yt(k, "", 0.0).key()).collect(),
            known_songs: HashSet::new(),
            variants: false,
            blocked: blocked.iter().map(|s| s.to_string()).collect(),
            genres: vec![],
            cooled: cooled.iter().map(|s| s.to_string()).collect(),
            shown: shown.iter().map(|(k, n)| (yt(k, "", 0.0).key(), *n)).collect(),
            heard: HashSet::new(),
            radio_tag: None,
        }
    }

    #[test]
    fn song_seeds_must_be_earned_and_feedback_is_respected() {
        let mut lib = Library {
            recent: vec![
                yt("once", "Samara Officiel", 200.0),
                yt("a1", "Ann", 200.0),
                yt("a2", "Ann", 200.0),
                yt("b1", "Bob", 200.0),
                yt("skip", "Sid", 200.0),
            ],
            favorites: vec![yt("d1", "Dan", 200.0)],
            ..Default::default()
        };
        lib.plays = HashMap::from([
            (yt("once", "", 0.0).key(), 1),
            (yt("a1", "", 0.0).key(), 5),
            (yt("a2", "", 0.0).key(), 3),
            (yt("b1", "", 0.0).key(), 40),
            (yt("skip", "", 0.0).key(), 4),
        ]);
        lib.feedback = Feedback { skips: HashMap::from([(yt("skip", "", 0.0).key(), 4)]), ..Default::default() };
        let s = song_seeds(&lib);
        assert_eq!(s[0], "b1", "most listened leads: {s:?}");
        assert!(s.contains(&"d1".into()), "favorites qualify: {s:?}");
        assert!(s.contains(&"a1".into()) && !s.contains(&"a2".into()), "one per artist: {s:?}");
        assert!(!s.contains(&"once".into()), "one play isn't a taste: {s:?}");
        assert!(!s.contains(&"skip".into()), "every play skipped: {s:?}");

        lib.feedback.unliked.push(yt("d1", "Dan", 200.0));
        lib.favorites.clear();
        assert!(!song_seeds(&lib).contains(&"d1".into()), "un-hearted songs stop being seeds");
    }

    #[test]
    fn a_test_listen_is_not_a_taste() {
        let tried = yt("tried", "Tess", 200.0);
        let loved = yt("loved", "Lou", 200.0);
        let station = radio("dropin", "dance");
        let mut lib = Library { recent: vec![tried.clone(), loved.clone(), station.clone()], ..Default::default() };
        lib.listened = HashMap::from([(tried.key(), 20.0), (loved.key(), 210.0), (station.key(), 40.0)]);
        lib.plays = HashMap::from([(tried.key(), 2)]); // old-style counts from starting it twice
        let t = taste(&lib);
        let seeds: Vec<String> = t.interests.iter().flat_map(|i| &i.seeds).map(|(s, _)| s.title.clone()).collect();
        assert_eq!(seeds, ["loved"], "20 s is a test, one whole listen is a taste");
        assert!(t.interests.iter().all(|i| i.title != "Dance & electronic"), "40 s of a station isn't an interest");
    }

    #[test]
    fn radio_listening_becomes_genre_interests_weighted_by_time() {
        let dance = radio("dancewave", "dance, electronica");
        let chill = radio("chillzone", "chillout, ambient");
        let song = yt("Lana Del Rey - Song (slowed + reverb)", "x", 200.0);
        let mut lib = Library { recent: vec![dance.clone(), chill.clone(), song.clone()], ..Default::default() };
        lib.listened = HashMap::from([(dance.key(), 5000.0), (chill.key(), 2000.0), (song.key(), 900.0)]);
        lib.radio_songs =
            vec![(dance.key(), "Daft Punk - One More Time".into()), (chill.key(), "Bonobo - Kerala".into()), (dance.key(), "daft punk - one more time".into())];
        let t = taste(&lib);
        let titles: Vec<&str> = t.interests.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["Dance & electronic", "Chill & ambient", "Slowed + reverb"]);
        assert_eq!(t.interests[0].songs, ["Daft Punk - One More Time"], "heard songs feed their own family, once");
        assert_eq!(t.interests[0].stations, ["http://dancewave"]);
        assert!((t.interests.iter().map(|i| i.weight).sum::<f64>() - 1.0).abs() < 1e-9);
        assert_eq!(t.radio_tag.as_deref(), Some("dance"));
    }

    #[test]
    fn one_seed_per_real_artist_across_channels() {
        let mut lib = Library {
            favorites: vec![
                yt("Lana Del Rey - Summertime Sadness (Lyrics)", "flyartz", 200.0),
                yt("Lana Del Rey - Summertime Sadness (Official Video)", "Lana Del Rey", 200.0),
                yt("AURORA - Runaway", "AURORA", 200.0),
            ],
            ..Default::default()
        };
        lib.recent = lib.favorites.clone();
        assert_eq!(song_seeds(&lib).len(), 2);
    }

    #[test]
    fn rank_rewards_agreement_and_drops_known_and_blocked() {
        let mixes = vec![
            (
                1.0,
                String::new(),
                vec![yt("old", "X", 200.0), yt("both", "Y", 200.0), yt("one", "Zed", 200.0), yt("long", "L", 3600.0), yt("Samara - Yammi", "7clouds", 200.0)],
            ),
            (1.0, String::new(), vec![yt("both", "Y", 200.0), yt("two", "W", 200.0)]),
        ];
        let r: Vec<String> = rank(&mixes, &taste_with(&["old"], &["samara"], &[], &[])).into_iter().map(|(t, _)| t.title).collect();
        assert_eq!(r[0], "both", "in two mixes ranks first: {r:?}");
        for gone in ["old", "long", "Samara - Yammi"] {
            assert!(!r.contains(&gone.to_string()), "{gone} should be dropped: {r:?}");
        }
    }

    #[test]
    fn rank_weights_seeds_cools_unliked_and_fades_ignored() {
        let mixes = vec![
            (1.0, String::new(), vec![yt("strong", "Ann", 200.0)]),
            (0.5, String::new(), vec![yt("weak", "Bob", 200.0)]),
            (1.0, String::new(), vec![yt("cool", "Cat Officiel", 200.0)]),
            (1.0, String::new(), vec![yt("ignored", "Billie Eilish", 200.0)]),
        ];
        let r: Vec<String> = rank(&mixes, &taste_with(&[], &[], &["cat"], &[("ignored", 3)])).into_iter().map(|(t, _)| t.title).collect();
        assert_eq!(r, ["strong", "weak", "cool", "ignored"]);
    }

    #[test]
    fn each_pick_keeps_the_seed_that_chose_it() {
        let mixes = vec![
            (0.9, "Because you play Runaway".to_string(), vec![yt("pick", "Ann", 200.0)]),
            (0.7, "Because you play Daylight".to_string(), vec![yt("x", "Bob", 200.0), yt("pick", "Ann", 200.0)]),
        ];
        let r = rank(&mixes, &taste_with(&[], &[], &[], &[]));
        let pick = r.iter().find(|(t, _)| t.title == "pick").unwrap();
        assert_eq!(pick.1, "Because you play Runaway", "first in a 0.9 mix beats second in a 0.7 mix");
    }

    #[test]
    fn rank_caps_each_artist_at_two() {
        let mix: Vec<Track> = (0..5).map(|i| yt(&format!("s{i}"), "Same Artist - Topic", 200.0)).collect();
        assert_eq!(rank(&[(1.0, String::new(), mix)], &taste_with(&[], &[], &[], &[])).len(), 2);
    }

    #[test]
    fn other_uploads_of_a_song_count_as_that_song() {
        let up = |title: &str, channel: &str| Track { source: Source::YouTube(format!("{title}{channel}")), ..yt(title, channel, 200.0) };
        let mut taste = taste_with(&[], &[], &[], &[]);
        taste.known_songs.insert(song(&up("David Kushner - Daylight (Official Music Video)", "David Kushner")));
        taste.heard.insert("david kushner".into());
        let mix = vec![
            up("David Kushner - Daylight [Lyrics]", "7clouds"),
            up("David Kushner - California (slowed + reverb)", "Maria Males"),
            up("David Kushner - California (Official Audio)", "David Kushner"),
            up("David Kushner - Hero", "David Kushner"),
            up("Gigi Perez - Sailor Song (sped up)", "Some Edits"),
            up("Temper City - Self Aware", "Temper City"),
        ];
        let r: Vec<(String, String)> = rank(&[(1.0, String::new(), mix)], &taste).into_iter().map(|(t, _)| (t.title, t.artist)).collect();
        let titles: Vec<&str> = r.iter().map(|(t, _)| t.as_str()).collect();
        assert!(!titles.iter().any(|t| t.contains("Daylight")), "a known song in another upload: {titles:?}");
        assert_eq!(r[0], ("David Kushner - California (Official Audio)".into(), "David Kushner".into()), "two uploads add up, the artist's own shows");
        assert!(!titles.contains(&"David Kushner - Hero"), "one pick per artist you already play: {titles:?}");
        assert_eq!(titles.last(), Some(&"Gigi Perez - Sailor Song (sped up)"), "remakes sink: {titles:?}");
    }

    #[test]
    fn blend_takes_from_every_interest_by_weight() {
        let list = |p: &str| (0..10).map(|i| yt(&format!("{p}{i}"), &format!("{p}{i}"), 200.0)).collect::<Vec<_>>();
        let made = blend(&[(0.75, list("dance")), (0.25, list("pop"))], 8);
        let dance = made.iter().filter(|t| t.title.starts_with("dance")).count();
        assert_eq!(made.len(), 8);
        assert!((5..=6).contains(&dance), "about three quarters dance: {dance}");
    }
}
