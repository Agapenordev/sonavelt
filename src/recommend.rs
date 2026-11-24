//! Home page recommendations.
//!
//! 1. Seeds: the songs that say the most about your taste, scored by how often and how recently
//!    you played them, plus favorites and playlists, one per artist so one doesn't dominate.
//! 2. YouTube's own Mix for each seed (fetched in parallel): YouTube's recommender does the
//!    heavy lifting; we combine and filter its answers.
//! 3. Ranking: a song that turns up in several seeds' mixes beats one that turns up once, earlier
//!    in a mix beats later, songs you already know are dropped, and no artist gets more than two
//!    spots.
//!
//! Results are cached, so Home opens instantly and refreshes in the background.

use crate::library::{Library, Source, Track};
use crate::sources;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

const SEEDS: usize = 4;
const MIX_LEN: usize = 30;
const MAX_AGE_SECS: u64 = 6 * 3600;

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
}

/// What the recommender needs from the library, copied so it can run on a worker thread.
pub struct Taste {
    seeds: Vec<Track>,
    known: HashSet<String>,
    radio_tags: Vec<String>,
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn cache_path() -> std::path::PathBuf {
    dirs::cache_dir().unwrap_or_default().join("sonavelt").join("home.json")
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

    /// "updated 2 h ago"
    pub fn age(&self) -> String {
        match now_secs().saturating_sub(self.made_at) / 60 {
            0 => "updated just now".into(),
            m if m < 60 => format!("updated {m} min ago"),
            m => format!("updated {} h ago", m / 60),
        }
    }
}

/// Lowercased artist without the channel suffixes YouTube adds, for "same artist" checks.
fn artist_key(a: &str) -> String {
    let a = a.to_lowercase();
    a.trim_end_matches(" - topic").trim_end_matches("vevo").trim_end_matches(" official").trim().to_string()
}

pub fn taste(lib: &Library) -> Taste {
    let mut score: HashMap<String, (Track, f64)> = HashMap::new();
    let mut add = |t: &Track, s: f64| {
        if !t.is_radio() {
            score.entry(t.key()).or_insert_with(|| (t.clone(), 0.0)).1 += s;
        }
    };
    for (i, t) in lib.recent.iter().enumerate() {
        add(t, 2.0 * 0.9f64.powi(i as i32)); // recent taste counts most
    }
    for t in &lib.favorites {
        add(t, 1.5);
    }
    for t in lib.playlists.values().flatten() {
        add(t, 0.5);
    }
    for (t, s) in score.values_mut() {
        *s += (1.0 + *lib.plays.get(&t.key()).unwrap_or(&0) as f64).ln();
    }
    let mut ranked: Vec<(Track, f64)> = score.into_values().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut artists = HashSet::new();
    let seeds = ranked.iter().filter(|(t, _)| artists.insert(artist_key(&t.artist))).take(SEEDS).map(|(t, _)| t.clone()).collect();

    let known = lib.recent.iter().chain(&lib.favorites).chain(lib.playlists.values().flatten()).map(Track::key).collect();

    // radio-browser tags of the stations you play ("US · chillout, lofi" -> chillout, lofi)
    let mut tags: HashMap<String, usize> = HashMap::new();
    for t in lib.recent.iter().chain(&lib.favorites).filter(|t| t.is_radio()) {
        if let Some((_, list)) = t.artist.rsplit_once(" · ") {
            for tag in list.split(',').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty() && s.len() < 20) {
                *tags.entry(tag).or_default() += 1;
            }
        }
    }
    let mut radio_tags: Vec<(String, usize)> = tags.into_iter().collect();
    radio_tags.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    Taste { seeds, known, radio_tags: radio_tags.into_iter().map(|(t, _)| t).take(2).collect() }
}

/// Songs only, no hour-long compilations or 20-second clips.
fn song_length(t: &Track) -> bool {
    t.duration.is_some_and(|d| (60.0..=600.0).contains(&d))
}

/// Combine several mixes into one ranked list of new songs.
pub fn rank(mixes: &[Vec<Track>], known: &HashSet<String>) -> Vec<Track> {
    let mut score: HashMap<String, (Track, f64)> = HashMap::new();
    for mix in mixes {
        for (pos, t) in mix.iter().enumerate() {
            if known.contains(&t.key()) || !song_length(t) {
                continue;
            }
            let s = 1.0 + 0.5 * (1.0 - pos as f64 / mix.len() as f64);
            score.entry(t.key()).or_insert_with(|| (t.clone(), 0.0)).1 += s;
        }
    }
    let mut ranked: Vec<(Track, f64)> = score.into_values().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.title.cmp(&b.0.title)));
    let mut per_artist: HashMap<String, usize> = HashMap::new();
    ranked
        .into_iter()
        .filter(|(t, _)| {
            let n = per_artist.entry(artist_key(&t.artist)).or_default();
            *n += 1;
            *n <= 2
        })
        .map(|(t, _)| t)
        .take(24)
        .collect()
}

/// Build the Home shelves. Blocking and networked: run it on a worker thread.
pub fn build(taste: Taste) -> Home {
    let mut shelves = Vec::new();
    let seeded = !taste.seeds.is_empty();
    if !seeded {
        // nothing played yet: something to start with
        if let Ok(popular) = sources::youtube_search("top hits this week", 20) {
            let tracks: Vec<Track> = popular.into_iter().filter(song_length).collect();
            shelves.push(Shelf { title: "Popular right now".into(), subtitle: "Play a few songs and this page learns your taste".into(), tracks });
        }
    } else {
        let mixes: Vec<(Track, Vec<Track>)> = std::thread::scope(|s| {
            let jobs: Vec<_> = taste.seeds.iter().map(|seed| s.spawn(move || (seed.clone(), mix_for(seed)))).collect();
            jobs.into_iter().filter_map(|j| j.join().ok()).filter(|(_, m)| !m.is_empty()).collect()
        });
        let all: Vec<Vec<Track>> = mixes.iter().map(|(_, m)| m.clone()).collect();
        let mut used = taste.known.clone();
        let made = rank(&all, &taste.known);
        used.extend(made.iter().map(Track::key));
        if !made.is_empty() {
            shelves.push(Shelf {
                title: "Made for you".into(),
                subtitle: "New songs picked from what you play".into(),
                tracks: made.into_iter().take(16).collect(),
            });
        }
        for (seed, mix) in &mixes {
            let more: Vec<Track> = mix.iter().filter(|t| song_length(t) && used.insert(t.key())).take(12).cloned().collect();
            if more.len() >= 4 {
                shelves.push(Shelf { title: format!("More like {}", seed.artist), subtitle: format!("Because you played {}", seed.title), tracks: more });
            }
        }
    }
    let stations = taste.radio_tags.first().and_then(|tag| {
        let found = sources::radio_search(tag, true).ok()?;
        Some(Shelf { title: "Stations for you".into(), subtitle: format!("Because you listen to {tag}"), tracks: found.into_iter().take(12).collect() })
    });
    shelves.push(stations.unwrap_or_else(|| Shelf {
        title: "Stations to try".into(),
        subtitle: "Hand-picked internet radio".into(),
        tracks: sources::curated_radios(),
    }));
    Home { made_at: now_secs(), shelves, seeded }
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

    fn yt(id: &str, artist: &str, secs: f64) -> Track {
        Track { title: id.into(), artist: artist.into(), source: Source::YouTube(id.into()), duration: Some(secs), art: None }
    }

    #[test]
    fn seeds_favor_recent_and_loved_one_per_artist() {
        let mut lib = Library {
            recent: vec![yt("a1", "Ann", 200.0), yt("a2", "Ann", 200.0), yt("b1", "Bob", 200.0), yt("c1", "Cat", 200.0)],
            favorites: vec![yt("d1", "Dan", 200.0)],
            ..Default::default()
        };
        lib.plays.insert(yt("c1", "Cat", 200.0).key(), 40);
        let t = taste(&lib);
        let ids: Vec<&str> = t.seeds.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(ids.len(), 4);
        assert!(ids.contains(&"a1") && !ids.contains(&"a2"), "one per artist: {ids:?}");
        assert_eq!(ids[0], "c1", "40 plays outweighs recency: {ids:?}");
        assert!(t.known.contains(&yt("d1", "Dan", 1.0).key()));
    }

    #[test]
    fn rank_rewards_agreement_and_drops_known() {
        let known: HashSet<String> = [yt("old", "X", 200.0).key()].into();
        let mixes = vec![
            vec![yt("old", "X", 200.0), yt("both", "Y", 200.0), yt("one", "Z", 200.0), yt("long", "L", 3600.0)],
            vec![yt("both", "Y", 200.0), yt("two", "W", 200.0)],
        ];
        let r: Vec<String> = rank(&mixes, &known).into_iter().map(|t| t.title).collect();
        assert_eq!(r[0], "both", "in two mixes ranks first: {r:?}");
        assert!(!r.contains(&"old".to_string()) && !r.contains(&"long".to_string()), "{r:?}");
    }

    #[test]
    fn rank_caps_each_artist_at_two() {
        let mix: Vec<Track> = (0..5).map(|i| yt(&format!("s{i}"), "Same Artist - Topic", 200.0)).collect();
        assert_eq!(rank(&[mix], &HashSet::new()).len(), 2);
    }
}
