//! Where music comes from: local folder, YouTube (via yt-dlp), internet radio (radio-browser.info),
//! and lyrics (lrclib.net). All functions block; call them from a worker thread.

use crate::library::{Source, Track};
use serde_json::Value;
use std::path::Path;
use std::process::Command;

const UA: &str = "sonavelt/0.1 (https://github.com/)";
const AUDIO_EXT: &[&str] = &["mp3", "flac", "ogg", "opus", "m4a", "aac", "wav", "wma", "webm"];

pub fn scan_folder(dir: &str) -> Vec<Track> {
    let mut v: Vec<Track> = walkdir::WalkDir::new(shellexpand(dir))
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| {
            e.path().extension().and_then(|x| x.to_str()).is_some_and(|x| AUDIO_EXT.contains(&x.to_lowercase().as_str()))
        })
        .map(|e| {
            let stem = e.path().file_stem().unwrap_or_default().to_string_lossy().to_string();
            // shortcut: artist/title from "Artist - Title" filenames, read real tags (lofty) if names are messy
            let (artist, title) = match stem.split_once(" - ") {
                Some((a, t)) => (a.trim().to_string(), t.trim().to_string()),
                None => (parent_name(e.path()), stem),
            };
            Track { title, artist, source: Source::Local(e.path().to_path_buf()), duration: None, art: None }
        })
        .collect();
    v.sort_by(|a, b| (&a.artist, &a.title).cmp(&(&b.artist, &b.title)));
    v
}

fn parent_name(p: &Path) -> String {
    p.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

fn shellexpand(dir: &str) -> String {
    match dir.strip_prefix("~/") {
        Some(rest) => dirs::home_dir().unwrap_or_default().join(rest).display().to_string(),
        None => dir.to_string(),
    }
}

/// Search YouTube, or list a pasted video/playlist URL.
pub fn youtube_search(query: &str, n: usize) -> Result<Vec<Track>, String> {
    let target = if query.starts_with("http") { query.to_string() } else { format!("ytsearch{n}:{query}") };
    let out = Command::new("yt-dlp")
        .args(["--flat-playlist", "-j", "--no-warnings", &target])
        .output()
        .map_err(|e| format!("yt-dlp not found: {e}"))?;
    if !out.status.success() && out.stdout.is_empty() {
        return Err(String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("YouTube search failed").to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|j| {
            Some(Track {
                title: crate::library::plain_text(j["title"].as_str()?),
                artist: crate::library::plain_text(j["channel"].as_str().or(j["uploader"].as_str()).unwrap_or("YouTube")),
                source: Source::YouTube(j["id"].as_str()?.to_string()),
                duration: j["duration"].as_f64(),
                art: None,
            })
        })
        .collect())
}

fn yt_dir() -> std::path::PathBuf {
    dirs::cache_dir().unwrap_or_default().join("sonavelt").join("youtube")
}

/// Path of a fully downloaded video, if we have one.
pub fn youtube_cached(id: &str) -> Option<String> {
    std::fs::read_dir(yt_dir())
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.file_stem().is_some_and(|s| s == id) && p.extension().is_some_and(|x| x != "part"))
        .map(|p| p.display().to_string())
}

/// The visionos client skips the ~5 s "watch the ad" wait other YouTube clients get.
const FAST_CLIENT: &str = "youtube:player_client=visionos";
/// Prefer one plain HTTPS file over HLS fragments: starts faster and leaves nothing behind.
const BEST_AUDIO: &str = "bestaudio[protocol^=http]/bestaudio/best";

/// Start streaming a video's audio right away. yt-dlp downloads at full speed into memory and
/// the cache file; the player reads from memory as bytes arrive, so playback starts within a
/// couple of seconds and the finished file lands in the cache for seeking and replay.
pub fn youtube_stream(id: &str) -> Result<Box<dyn std::io::Read + Send>, String> {
    let _ = std::fs::create_dir_all(yt_dir());
    let mut child = Command::new("yt-dlp")
        .args(["-f", BEST_AUDIO, "--no-warnings", "--no-playlist", "--no-progress", "-q", "--extractor-args", FAST_CLIENT, "-o", "-"])
        .arg(format!("https://www.youtube.com/watch?v={id}"))
        .current_dir(yt_dir()) // an interrupted download leaves fragment files in the working directory
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("yt-dlp not found: {e}"))?;
    let mut out = child.stdout.take().unwrap();
    let _ = std::fs::create_dir_all(yt_dir());
    let done = yt_dir().join(format!("{id}.audio"));
    let part = done.with_extension("part");
    let buf = std::sync::Arc::new(StreamBuf::default());
    let writer = buf.clone();
    std::thread::spawn(move || {
        let mut file = std::fs::File::create(&part).ok();
        let mut chunk = [0u8; 64 * 1024];
        let ok = loop {
            if writer.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                break false;
            }
            match std::io::Read::read(&mut out, &mut chunk) {
                Ok(0) => break child.wait().is_ok_and(|s| s.success()),
                Ok(n) => {
                    if let Some(f) = &mut file {
                        if std::io::Write::write_all(f, &chunk[..n]).is_err() {
                            file = None;
                        }
                    }
                    writer.data.lock().unwrap().0.extend_from_slice(&chunk[..n]);
                    writer.cv.notify_all();
                }
                Err(_) => break false,
            }
        };
        let _ = child.kill();
        let _ = child.wait();
        // only a complete download becomes a cache entry
        if !(ok && file.is_some() && std::fs::rename(&part, &done).is_ok()) {
            let _ = std::fs::remove_file(&part);
        }
        writer.data.lock().unwrap().1 = true;
        writer.cv.notify_all();
    });
    Ok(Box::new(StreamReader { buf, pos: 0 }))
}

#[derive(Default)]
struct StreamBuf {
    data: std::sync::Mutex<(Vec<u8>, bool)>, // bytes so far, finished
    cv: std::sync::Condvar,
    cancelled: std::sync::atomic::AtomicBool,
}

struct StreamReader {
    buf: std::sync::Arc<StreamBuf>,
    pos: usize,
}

impl std::io::Read for StreamReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let mut data = self.buf.data.lock().unwrap();
        while self.pos >= data.0.len() && !data.1 {
            data = self.buf.cv.wait(data).unwrap();
        }
        let n = (data.0.len() - self.pos).min(out.len());
        out[..n].copy_from_slice(&data.0[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

impl Drop for StreamReader {
    fn drop(&mut self) {
        // skipped before the download finished: stop it rather than fetch a song nobody hears
        self.buf.cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Download a video's audio into the cache (used to prefetch the next song) and return the path.
pub fn youtube_audio(id: &str) -> Result<String, String> {
    let dir = yt_dir();
    if let Some(p) = youtube_cached(id) {
        return Ok(p);
    }
    let _ = std::fs::create_dir_all(&dir);
    let template = dir.join("%(id)s.%(ext)s").display().to_string();
    let mut err = String::from("couldn't download video");
    // YouTube hands out the odd 403 at random; a second attempt usually gets through
    for attempt in 0..3 {
        let mut cmd = Command::new("yt-dlp");
        cmd.current_dir(&dir);
        cmd.args(["-f", BEST_AUDIO, "--no-warnings", "--no-playlist", "--no-progress", "--retries", "5", "-o", &template, "--print", "after_move:filepath"]);
        if attempt == 0 {
            cmd.args(["--extractor-args", FAST_CLIENT]); // fast path first, YouTube's default clients after
        }
        let out = cmd
            .arg(format!("https://www.youtube.com/watch?v={id}"))
            .output()
            .map_err(|e| format!("yt-dlp not found: {e}"))?;
        if let Some(path) = String::from_utf8_lossy(&out.stdout).lines().last().filter(|l| Path::new(l).exists()) {
            return Ok(path.to_string());
        }
        if let Some(line) = String::from_utf8_lossy(&out.stderr).lines().last() {
            err = line.trim_start_matches("ERROR: ").to_string();
        }
    }
    Err(err)
}

/// Hand-picked stations that are reliably great.
pub fn curated_radios() -> Vec<Track> {
    [
        ("Groove Salad", "SomaFM · ambient downtempo", "https://ice1.somafm.com/groovesalad-128-mp3", "https://somafm.com/img3/groovesalad-400.jpg"),
        ("Drone Zone", "SomaFM · atmospheric", "https://ice1.somafm.com/dronezone-128-mp3", "https://somafm.com/img3/dronezone-400.jpg"),
        ("Secret Agent", "SomaFM · spy lounge", "https://ice1.somafm.com/secretagent-128-mp3", "https://somafm.com/img3/secretagent-400.jpg"),
        ("DEF CON Radio", "SomaFM · hacker beats", "https://ice1.somafm.com/defcon-128-mp3", "https://somafm.com/img3/defcon-400.jpg"),
        ("Lush", "SomaFM · sensual vocals", "https://ice1.somafm.com/lush-128-mp3", "https://somafm.com/img3/lush-400.jpg"),
        ("Radio Paradise", "eclectic rock & more", "https://stream.radioparadise.com/mp3-192", "https://radioparadise.com/apple-touch-icon.png"),
        ("FIP", "Radio France · jazz/world/everything", "https://icecast.radiofrance.fr/fip-midfi.mp3", "https://www.fip.fr/dist/favicons/logo-120.png"),
        ("FIP Groove", "Radio France · funk & soul", "https://icecast.radiofrance.fr/fipgroove-midfi.mp3", "https://www.fip.fr/dist/favicons/logo-120.png"),
        ("KEXP 90.3", "Seattle · indie", "https://kexp-mp3-128.streamguys1.com/kexp128.mp3", "https://www.kexp.org/apple-touch-icon.png"),
        ("NTS 1", "London · underground", "https://stream-relay-geo.ntslive.net/stream", "https://www.nts.live/apple-touch-icon.png"),
        ("Nightride FM", "synthwave", "https://stream.nightride.fm/nightride.mp3", "https://nightride.fm/thumbnail.png"),
        ("Chillsynth", "chill synthwave", "https://stream.nightride.fm/chillsynth.mp3", "https://nightride.fm/thumbnail.png"),
        ("Jazz24", "straight-ahead jazz", "https://live.wostreaming.net/direct/ppm-jazz24mp3-ibc1", "https://www.jazz24.org/apple-touch-icon.png"),
        ("BBC Radio 1", "UK · pop & dance", "http://as-hls-ww-live.akamaized.net/pool_904/live/ww/bbc_radio_one/bbc_radio_one.isml/bbc_radio_one-audio%3d96000.norewind.m3u8", "https://cdn-radiotime-logos.tunein.com/s24939q.png"),
    ]
    .into_iter()
    .map(|(name, desc, url, art)| Track { title: name.into(), artist: desc.into(), source: Source::Radio(url.into()), duration: None, art: Some(art.into()) })
    .collect()
}

/// Search radio-browser.info by tag (genre) or name; empty query = most voted worldwide.
pub fn radio_search(query: &str, by_tag: bool) -> Result<Vec<Track>, String> {
    let base = "https://de1.api.radio-browser.info/json/stations";
    let req = if query.is_empty() {
        ureq::get(&format!("{base}/topvote/80")).query("hidebroken", "true")
    } else {
        ureq::get(&format!("{base}/search"))
            .query(if by_tag { "tag" } else { "name" }, query)
            .query("order", "votes")
            .query("reverse", "true")
            .query("hidebroken", "true")
            .query("limit", "80")
    };
    let list: Vec<Value> = req.set("User-Agent", UA).call().map_err(|e| e.to_string())?.into_json().map_err(|e| e.to_string())?;
    Ok(list
        .iter()
        .filter_map(|s| {
            let url = s["url_resolved"].as_str().filter(|u| !u.is_empty())?;
            let tags: Vec<&str> = s["tags"].as_str().unwrap_or("").split(',').filter(|t| !t.is_empty()).take(3).collect();
            let country = s["countrycode"].as_str().unwrap_or("");
            Some(Track {
                title: s["name"].as_str()?.trim().to_string(),
                artist: [country.to_string(), tags.join(", ")].into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" · "),
                source: Source::Radio(url.to_string()),
                duration: None,
                // no logo listed: the station website's icon is the next best thing
                art: s["favicon"].as_str().filter(|f| f.starts_with("http")).map(str::to_string).or_else(|| {
                    let host = s["homepage"].as_str()?.split("://").nth(1)?.split(['/', ':', '?']).next()?;
                    Some(format!("https://www.google.com/s2/favicons?domain={host}&sz=128"))
                }),
            })
        })
        .collect())
}

pub struct Lyrics {
    pub synced: bool,
    pub lines: Vec<(f64, String)>,
}

pub fn lyrics(t: &Track) -> Result<Lyrics, String> {
    let (artist, title) = clean_title(t);
    let mut req = ureq::get("https://lrclib.net/api/search").set("User-Agent", UA);
    req = if artist.is_empty() { req.query("q", &title) } else { req.query("track_name", &title).query("artist_name", &artist) };
    let mut hits: Vec<Value> = req.call().map_err(|e| e.to_string())?.into_json().map_err(|e| e.to_string())?;
    if hits.is_empty() && !artist.is_empty() {
        let q = format!("{artist} {title}");
        hits = ureq::get("https://lrclib.net/api/search").set("User-Agent", UA).query("q", &q).call().map_err(|e| e.to_string())?.into_json().map_err(|e| e.to_string())?;
    }
    if let Some(synced) = hits.iter().find_map(|h| h["syncedLyrics"].as_str()) {
        return Ok(Lyrics { synced: true, lines: parse_lrc(synced) });
    }
    hits.iter()
        .find_map(|h| h["plainLyrics"].as_str())
        .map(|p| Lyrics { synced: false, lines: p.lines().map(|l| (0.0, l.to_string())).collect() })
        .ok_or_else(|| "No lyrics found".into())
}

/// "Artist - Song (Official Video) [4K]" -> ("Artist", "Song")
fn clean_title(t: &Track) -> (String, String) {
    let mut s = String::new();
    let mut depth = 0;
    for c in t.title.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            _ if depth == 0 => s.push(c),
            _ => {}
        }
    }
    let s = s.replace("Official Video", "").replace("Lyrics", "");
    match s.split_once(" - ") {
        Some((a, b)) => (a.trim().into(), b.trim().into()),
        None if matches!(t.source, Source::YouTube(_)) => (String::new(), s.trim().into()),
        None => (t.artist.clone(), s.trim().into()),
    }
}

fn parse_lrc(s: &str) -> Vec<(f64, String)> {
    s.lines()
        .filter_map(|l| {
            let (stamp, text) = l.strip_prefix('[')?.split_once(']')?;
            let (m, sec) = stamp.split_once(':')?;
            Some((m.parse::<f64>().ok()? * 60.0 + sec.parse::<f64>().ok()?, text.trim().to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrc_and_title_cleaning() {
        assert_eq!(parse_lrc("[01:02.50] hello\n[ti:x]")[0], (62.5, "hello".to_string()));
        let t = Track { title: "Daft Punk - One More Time (Official Video)".into(), artist: "x".into(), source: Source::YouTube("i".into()), duration: None, art: None };
        assert_eq!(clean_title(&t), ("Daft Punk".into(), "One More Time".into()));
    }
}

#[cfg(test)]
mod net_tests {
    // Hits the network; run with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn youtube_download_and_decode() {
        let p = super::youtube_audio("FGBhQbmPwH8").unwrap();
        let out = std::process::Command::new("ffmpeg").args(["-v", "error", "-i", &p, "-t", "1", "-f", "f32le", "-"]).output().unwrap();
        assert!(out.stdout.len() > 100_000, "{}", String::from_utf8_lossy(&out.stderr));
    }
}
