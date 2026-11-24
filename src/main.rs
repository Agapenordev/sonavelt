mod art;
mod audio;
mod library;
mod logo;
mod mpris;
mod sources;
mod theme;
mod tray;
mod updater;
mod viz;

use art::ArtCache;
use eframe::egui::{self, Align, Color32, CornerRadius, FontFamily, FontId, Key, Margin, Rect, RichText, Sense, Stroke, Ui, Vec2, pos2, vec2};
use egui_phosphor::regular as icon;
use library::{Background, Library, Palette, Source, Track};
use sources::Lyrics;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};
use theme::*;

const GENRES: &[&str] =
    &["lofi", "jazz", "house", "techno", "synthwave", "chillout", "hiphop", "rock", "classical", "ambient", "reggae", "kpop", "metal", "afrobeat"];
const ROW_H: f32 = 56.0;

fn main() -> eframe::Result {
    library::migrate_from_rustwave();
    install_desktop_entry();
    let mut app: Option<App> = None;
    loop {
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title("Sonavelt")
                .with_app_id("sonavelt")
                .with_icon(std::sync::Arc::new(egui::IconData { rgba: logo::rgba(256), width: 256, height: 256 }))
                .with_inner_size([1320.0, 860.0])
                .with_min_inner_size([980.0, 640.0]),
            ..Default::default()
        };
        let slot = &mut app;
        eframe::run_native(
            "sonavelt",
            options,
            Box::new(move |cc| {
                match slot {
                    Some(a) => a.attach(cc),
                    None => *slot = Some(App::new(cc)),
                }
                Ok(Box::new(Window(Option::as_mut(slot).unwrap())))
            }),
        )?;
        // Window closed. With a tray, keep playing in the background until "Show" or "Quit".
        let Some(a) = app.as_mut() else { break };
        if a.quitting || a.tray.is_none() || !a.lib.settings.close_to_tray || !a.background() {
            break;
        }
    }
    Ok(())
}

/// The eframe app for one window's lifetime; the `App` outlives it so playback survives closing.
struct Window<'a>(&'a mut App);

impl eframe::App for Window<'_> {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        eframe::App::logic(self.0, ctx, frame);
    }
    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        eframe::App::ui(self.0, ui, frame);
    }
}

enum Msg {
    Local(Vec<Track>),
    YouTube(Result<Vec<Track>, String>),
    Radio(Result<Vec<Track>, String>),
    Resolved(u64, Result<String, String>),
    Lyrics(String, Result<Lyrics, String>),
    Recs(Vec<Track>),
    RadioNow(u64, Option<String>),
    UpdateChecked(bool, Result<Option<updater::Release>, String>),
    UpdateInstalled(Result<String, String>),
}

#[derive(Clone, PartialEq)]
enum View {
    NowPlaying,
    Local,
    YouTube,
    Radio,
    Favorites,
    Recent,
    Queue,
    Playlist(String),
    Settings,
}

enum Update {
    Idle,
    Checking,
    UpToDate,
    Available(updater::Release),
    Installing(String),
    Ready(String),
    Failed(String),
}

#[derive(Clone, Copy, PartialEq)]
enum Repeat {
    Off,
    All,
    One,
}

enum RowAct {
    Play(usize),
    Fav(usize),
    Next(usize),
    Enqueue(usize),
    AddTo(usize, String),
    Remove(usize),
}

struct App {
    ctx: egui::Context,
    engine: Result<audio::Engine, String>,
    lib: Library,
    art: ArtCache,
    aura: viz::Aura,
    level: f32,
    view: View,
    yt_fallback: bool,
    prefetched: HashSet<String>,
    /// Song the current station is playing, from its stream metadata.
    radio_now: Option<String>,
    radio_polled: Option<Instant>,
    view_since: f64,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    local: Vec<Track>,
    yt: Vec<Track>,
    radios: Vec<Track>,
    radio_label: String,
    busy: HashSet<&'static str>,
    yt_query: String,
    radio_query: String,
    filter: String,
    new_playlist: String,
    queue: Vec<Track>,
    qpos: usize,
    current: Option<Track>,
    play_gen: u64,
    buffering: bool,
    smart: bool,
    repeat: Repeat,
    recs: HashSet<String>,
    lyrics: HashMap<String, Result<Lyrics, String>>,
    lyric_line: Option<usize>,
    sleep_at: Option<Instant>,
    toast: Option<(String, f64)>,
    last_time: f64,
    marquee_phase: f32,
    logo: egui::TextureHandle,
    tray: Option<tray::Handle>,
    mpris: Option<mpris::Mpris>,
    tray_rx: Receiver<tray::TrayCmd>,
    tray_state: (String, bool),
    quitting: bool,
    picker: Option<String>,
    pick_query: String,
    cache_size: Option<u64>,
    update: Update,
    /// Our own path, read at startup: after an update swaps the file, the running process's
    /// link to itself points at the deleted original.
    exe: std::path::PathBuf,
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        let ctx = cc.egui_ctx.clone();
        style(&ctx);
        let (tx, rx) = channel();
        let mut lib = Library::load();
        // saved stations keep the logo they had when saved; curated ones get today's link
        let curated = sources::curated_radios();
        for t in [&mut lib.favorites, &mut lib.recent].into_iter().chain(lib.playlists.values_mut()).flatten() {
            if let Some(c) = curated.iter().find(|c| c.source == t.source) {
                t.art = c.art.clone();
            }
        }
        let engine = audio::Engine::new();
        if let Ok(e) = &engine {
            e.set_volume(lib.volume);
        }
        ctx.set_zoom_factor(lib.settings.ui_scale);
        let (tray_tx, tray_rx) = channel();
        let mpris = mpris::Mpris::spawn(tray_tx.clone(), ctx.clone());
        let tray = tray::spawn(tray_tx, ctx.clone());
        let mut app = Self {
            marquee_phase: 0.0,
            logo: logo_texture(&ctx),
            tray,
            mpris,
            tray_rx,
            tray_state: (String::new(), false),
            quitting: false,
            picker: None,
            pick_query: String::new(),
            cache_size: None,
            update: Update::Idle,
            exe: updater::current_exe(),
            art: ArtCache::new(&ctx),
            ctx,
            engine,
            aura: viz::Aura::new(),
            level: 0.0,
            view: View::NowPlaying,
            yt_fallback: false,
            prefetched: HashSet::new(),
            radio_now: None,
            radio_polled: None,
            view_since: 0.0,
            tx,
            rx,
            local: vec![],
            yt: vec![],
            radios: sources::curated_radios(),
            radio_label: "Our picks".into(),
            busy: HashSet::new(),
            yt_query: String::new(),
            radio_query: String::new(),
            filter: String::new(),
            new_playlist: String::new(),
            queue: vec![],
            qpos: 0,
            current: None,
            play_gen: 0,
            buffering: false,
            smart: false,
            repeat: Repeat::Off,
            recs: HashSet::new(),
            lyrics: HashMap::new(),
            lyric_line: None,
            sleep_at: None,
            toast: None,
            last_time: 0.0,
            lib,
        };
        app.scan();
        app.check_update(true);
        // resume where you left off: last song loaded, paused
        if let Some(t) = app.lib.recent.first().cloned() {
            app.queue = vec![t.clone()];
            app.current = Some(t);
            app.fetch_lyrics();
        }
        app
    }

    fn spawn(&mut self, tag: &'static str, f: impl FnOnce() -> Msg + Send + 'static) {
        self.busy.insert(tag);
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(f());
            ctx.request_repaint();
        });
    }

    fn toast(&mut self, s: impl Into<String>) {
        self.toast = Some((s.into(), self.ctx.input(|i| i.time)));
    }

    fn scan(&mut self) {
        let dir = self.lib.music_dir.clone();
        self.spawn("local", move || Msg::Local(sources::scan_folder(&dir)));
    }

    fn search_youtube(&mut self) {
        let q = self.yt_query.trim().to_string();
        if !q.is_empty() {
            self.spawn("yt", move || Msg::YouTube(sources::youtube_search(&q, 24)));
        }
    }

    fn search_radio(&mut self, query: String, by_tag: bool, label: String) {
        self.radio_label = label;
        self.spawn("radio", move || Msg::Radio(sources::radio_search(&query, by_tag)));
    }

    fn fetch_lyrics(&mut self) {
        let Some(t) = self.current.clone() else { return };
        if t.is_radio() || self.lyrics.contains_key(&t.key()) {
            return;
        }
        self.lyrics.insert(t.key(), Err("Looking for lyrics".into()));
        self.spawn("lyrics", move || Msg::Lyrics(t.key(), sources::lyrics(&t)));
    }

    // ---------- playback ----------

    /// Play a whole list (library, favorites, playlist) starting at `i`.
    fn play_list(&mut self, mut list: Vec<Track>, i: usize) {
        self.recs.clear();
        if self.smart && !list[i].is_radio() {
            let first = list.remove(i);
            self.lib.smart_shuffle(&mut list);
            list.insert(0, first);
            self.queue = list;
            self.qpos = 0;
            self.fetch_recs();
        } else {
            self.queue = list;
            self.qpos = i;
        }
        self.start();
    }

    fn shuffle_play(&mut self, mut list: Vec<Track>) {
        if list.is_empty() {
            return;
        }
        self.lib.smart_shuffle(&mut list);
        self.queue = list;
        self.qpos = 0;
        self.start();
    }

    /// Play one track now (search results, stations) without replacing the queue.
    fn play_now(&mut self, t: Track) {
        if self.current.is_none() || self.queue.is_empty() {
            self.queue = vec![t];
            self.qpos = 0;
        } else {
            self.qpos = (self.qpos + 1).min(self.queue.len());
            self.queue.insert(self.qpos, t);
        }
        self.start();
    }

    /// Smart shuffle extra: sprinkle in YouTube songs related to the queue's artists.
    fn fetch_recs(&mut self) {
        let artists: Vec<String> = self.queue.iter().filter(|t| !t.is_radio()).map(|t| t.artist.clone()).take(20).collect();
        if artists.is_empty() {
            return;
        }
        let pick = artists[rand::random_range(0..artists.len())].clone();
        self.spawn("recs", move || {
            let found = sources::youtube_search(&format!("songs similar to {pick}"), 12).unwrap_or_default();
            Msg::Recs(found.into_iter().filter(|t| t.duration.is_some_and(|d| (90.0..480.0).contains(&d))).take(5).collect())
        });
    }

    fn start(&mut self) {
        if let Some(t) = self.queue.get(self.qpos).cloned() {
            self.play_track(t);
        }
    }

    fn play_track(&mut self, t: Track) {
        let Ok(engine) = &mut self.engine else { return };
        self.play_gen += 1;
        self.lyric_line = None;
        self.buffering = false;
        match &t.source {
            Source::Local(p) => engine.play(&p.display().to_string(), 0.0),
            Source::Radio(u) => engine.play(u, 0.0),
            // already downloaded: instant. Otherwise stream while yt-dlp downloads.
            Source::YouTube(id) => match sources::youtube_cached(id) {
                Some(path) => engine.play(&path, 0.0),
                None => match sources::youtube_stream(id) {
                    Ok(pipe) => engine.play_reader(pipe),
                    Err(e) => {
                        engine.stop();
                        self.toast(e);
                    }
                },
            },
        }
        self.yt_fallback = false;
        self.radio_now = None;
        self.radio_polled = None;
        self.lib.played(&t);
        self.current = Some(t);
        self.fetch_lyrics();
        self.prefetch_next();
    }

    /// Download the next YouTube song in the background so it starts instantly.
    fn prefetch_next(&mut self) {
        let Some(Source::YouTube(id)) = self.queue.get(self.qpos + 1).map(|t| t.source.clone()) else { return };
        if self.prefetched.insert(id.clone()) && sources::youtube_cached(&id).is_none() {
            std::thread::spawn(move || sources::youtube_audio(&id));
        }
    }

    /// The stream failed (YouTube refuses one now and then): retry once as a full download.
    fn youtube_fallback(&mut self) -> bool {
        let Some(Source::YouTube(id)) = self.current.as_ref().map(|t| t.source.clone()) else { return false };
        if self.yt_fallback {
            return false;
        }
        self.yt_fallback = true;
        self.buffering = true;
        let generation = self.play_gen;
        self.spawn("resolve", move || Msg::Resolved(generation, sources::youtube_audio(&id)));
        true
    }

    fn next(&mut self, auto: bool) {
        if auto && self.repeat == Repeat::One {
            self.start();
        } else if self.qpos + 1 < self.queue.len() {
            self.qpos += 1;
            self.start();
        } else if self.repeat == Repeat::All && !self.queue.is_empty() {
            self.qpos = 0;
            self.start();
        } else if let Ok(e) = &mut self.engine {
            e.stop();
        }
    }

    fn prev(&mut self) {
        if self.position() > 3.0 || self.qpos == 0 {
            self.seek(0.0);
        } else {
            self.qpos -= 1;
            self.start();
        }
    }

    fn toggle_play(&mut self) {
        match &self.engine {
            Ok(e) if e.active() => e.set_paused(!e.paused()),
            _ => {
                if let Some(t) = self.current.clone() {
                    self.play_track(t);
                }
            }
        }
    }

    fn seek(&mut self, t: f64) {
        let Some(cur) = self.current.clone() else { return };
        let Ok(e) = &mut self.engine else { return };
        if e.seekable() {
            if !cur.is_radio() {
                e.seek(t);
            }
        } else if let Source::YouTube(id) = &cur.source {
            // streamed songs become seekable once the download lands in the cache
            match sources::youtube_cached(id) {
                Some(path) => e.play(&path, t.max(0.0)),
                None => self.toast("Still downloading, seeking works in a moment"),
            }
        }
    }

    /// Waiting for the first audio: a download, a stream starting up, a radio connecting.
    fn loading(&self) -> bool {
        self.buffering || (self.playing() && self.position() < 0.05)
    }

    fn playing(&self) -> bool {
        self.engine.as_ref().is_ok_and(|e| e.active() && !e.paused())
    }

    fn position(&self) -> f64 {
        self.engine.as_ref().map(|e| e.position()).unwrap_or(0.0)
    }

    fn duration(&self) -> Option<f64> {
        self.engine.as_ref().ok().and_then(|e| e.duration()).or(self.current.as_ref().and_then(|t| t.duration))
    }

    fn set_view(&mut self, v: View) {
        if self.view != v {
            self.view = v;
            self.view_since = self.ctx.input(|i| i.time);
            self.filter.clear();
        }
    }

    fn apply(&mut self, act: RowAct, list: &[Track]) {
        match act {
            RowAct::Play(i) if self.view == View::Queue => {
                self.qpos = i;
                self.start();
            }
            RowAct::Play(i) if matches!(self.view, View::YouTube | View::Radio | View::NowPlaying) => self.play_now(list[i].clone()),
            RowAct::Play(i) => self.play_list(list.to_vec(), i),
            RowAct::Fav(i) => self.lib.toggle_fav(&list[i]),
            RowAct::Next(i) => {
                let at = if self.current.is_some() { (self.qpos + 1).min(self.queue.len()) } else { 0 };
                self.queue.insert(at, list[i].clone());
                self.toast(format!("Playing next: {}", list[i].title));
            }
            RowAct::Enqueue(i) => {
                self.queue.push(list[i].clone());
                self.toast(format!("Added to queue: {}", list[i].title));
            }
            RowAct::AddTo(i, name) => {
                self.lib.playlists.entry(name.clone()).or_default().push(list[i].clone());
                self.lib.save();
                self.toast(format!("Added to {name}"));
            }
            RowAct::Remove(i) => match self.view.clone() {
                View::Queue => {
                    self.queue.remove(i);
                    if i < self.qpos {
                        self.qpos -= 1;
                    }
                }
                View::Playlist(name) => {
                    if let Some(p) = self.lib.playlists.get_mut(&name) {
                        p.remove(i);
                    }
                    self.lib.save();
                }
                View::Favorites => self.lib.toggle_fav(&list[i]),
                _ => {}
            },
        }
    }

    // ---------- per-frame logic ----------

    fn tick(&mut self, ctx: &egui::Context) {
        self.art.poll(ctx);
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Local(v) => {
                    self.busy.remove("local");
                    self.local = v;
                }
                Msg::YouTube(r) => {
                    self.busy.remove("yt");
                    match r {
                        Ok(v) => self.yt = v,
                        Err(e) => self.toast(e),
                    }
                }
                Msg::Radio(r) => {
                    self.busy.remove("radio");
                    match r {
                        Ok(v) => self.radios = v,
                        Err(e) => self.toast(format!("Radio directory unreachable: {e}")),
                    }
                }
                Msg::Resolved(generation, r) => {
                    self.busy.remove("resolve");
                    if generation == self.play_gen {
                        self.buffering = false;
                        match (r, &mut self.engine) {
                            (Ok(path), Ok(e)) => e.play(&path, 0.0),
                            (Err(e), _) => self.toast(format!("YouTube: {e}")),
                            _ => {}
                        }
                    }
                }
                Msg::Lyrics(k, r) => {
                    self.busy.remove("lyrics");
                    self.lyrics.insert(k, r);
                }
                Msg::UpdateChecked(quiet, r) => {
                    self.busy.remove("update");
                    self.update = match r {
                        Ok(Some(rel)) => {
                            self.toast(format!("Sonavelt {} is available. Update it from Settings", rel.version));
                            Update::Available(rel)
                        }
                        Ok(None) => Update::UpToDate,
                        Err(_) if quiet => Update::Idle, // offline at launch: say nothing
                        Err(e) => Update::Failed(e),
                    };
                }
                Msg::UpdateInstalled(r) => {
                    self.busy.remove("update");
                    self.update = match r {
                        Ok(v) => Update::Ready(v),
                        Err(e) => Update::Failed(e),
                    };
                }
                Msg::RadioNow(generation, now) => {
                    self.busy.remove("icy");
                    // keep the last known song when one poll comes back empty
                    if generation == self.play_gen && now.is_some() {
                        self.radio_now = now;
                    }
                }
                Msg::Recs(v) => {
                    self.busy.remove("recs");
                    let mut at = self.qpos + 2;
                    for t in v {
                        if self.queue.iter().any(|q| q.source == t.source) {
                            continue;
                        }
                        self.recs.insert(t.key());
                        self.queue.insert(at.min(self.queue.len()), t);
                        at += 3;
                    }
                }
            }
        }

        let now = ctx.input(|i| i.time);
        let dt = ((now - self.last_time) as f32).clamp(0.0, 0.1);
        self.last_time = now;

        if let Ok(e) = &mut self.engine {
            if let Some(err) = e.take_error() {
                e.stop();
                if !self.youtube_fallback() {
                    self.toast(format!("Can't play: {err}"));
                }
            } else if e.ended() {
                self.next(true);
            }
        }
        if self.sleep_at.is_some_and(|t| Instant::now() >= t) {
            self.sleep_at = None;
            if let Ok(e) = &self.engine {
                e.set_paused(true);
            }
            self.toast("Sleep timer paused playback");
        }

        let playing = self.playing();
        let rms = self.engine.as_ref().ok().filter(|_| playing).map(|e| e.levels().iter().sum::<f32>() / 2.0).unwrap_or(0.0);
        let target = if self.lib.settings.beat { ((20.0 * rms.max(1e-5).log10() + 36.0) / 30.0).clamp(0.0, 1.0) } else { 0.0 };
        self.level += (target - self.level) * (1.0 - (-dt / 0.08).exp());

        let cur = self.current.clone();
        let palette = self.lib.settings.palette;
        let (grid, accent) = match cur.as_ref().and_then(|t| self.art.get(t)) {
            Some(a) if palette == Palette::Artwork => (a.grid, a.accent),
            _ => viz::palette(palette),
        };
        self.aura.update(&grid, accent, dt);
        self.tray_tick();
        self.poll_radio_now();
        self.mpris_tick();
        // the aura drifts continuously; when idle it only needs a few frames a second
        if playing || self.buffering {
            ctx.request_repaint();
        } else {
            let ms = if self.lib.settings.motion { 66 } else { 500 };
            ctx.request_repaint_after(std::time::Duration::from_millis(ms));
        }
    }

    fn keys(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let pressed = |k| ctx.input(|i| i.key_pressed(k));
        if pressed(Key::Space) {
            self.toggle_play();
        }
        if pressed(Key::ArrowRight) {
            self.seek(self.position() + 5.0);
        }
        if pressed(Key::ArrowLeft) {
            self.seek(self.position() - 5.0);
        }
        if pressed(Key::N) {
            self.next(false);
        }
        if pressed(Key::P) {
            self.prev();
        }
        if pressed(Key::L) {
            self.set_view(View::NowPlaying);
        }
        if pressed(Key::F)
            && let Some(t) = self.current.clone()
        {
            self.lib.toggle_fav(&t);
        }
        for (k, d) in [(Key::ArrowUp, 0.05), (Key::ArrowDown, -0.05)] {
            if pressed(k) {
                self.lib.volume = (self.lib.volume + d).clamp(0.0, 1.0);
                if let Ok(e) = &self.engine {
                    e.set_volume(self.lib.volume);
                }
            }
        }
    }

    // ---------- UI ----------

    fn nav(&mut self, ui: &mut Ui) {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ui.add(egui::Image::new((self.logo.id(), Vec2::splat(30.0))));
            ui.label(RichText::new("Sonavelt").font(FontId::new(30.0, serif())).color(TEXT));
        });
        ui.add_space(28.0);
        let accent = self.aura.accent;
        let live = self.playing();
        for (ic, label, v) in [
            (icon::VINYL_RECORD, "Now playing", View::NowPlaying),
            (icon::YOUTUBE_LOGO, "YouTube", View::YouTube),
            (icon::RADIO, "Radio", View::Radio),
            (icon::FOLDER_SIMPLE, "Your files", View::Local),
        ] {
            let dot = (live && v == View::NowPlaying).then_some(accent);
            if nav_item(ui, ic, label, None, self.view == v, dot) {
                self.set_view(v);
            }
        }
        ui.add_space(24.0);
        section(ui, "Library");
        let counts = [Some(self.lib.favorites.len()), None, Some(self.queue.len())];
        for ((ic, label, v), count) in
            [(icon::HEART, "Favorites", View::Favorites), (icon::CLOCK_COUNTER_CLOCKWISE, "Recently played", View::Recent), (icon::QUEUE, "Queue", View::Queue)]
                .into_iter()
                .zip(counts)
        {
            if nav_item(ui, ic, label, count, self.view == v, None) {
                self.set_view(v);
            }
        }
        ui.add_space(24.0);
        section(ui, "Playlists");
        egui::ScrollArea::vertical().max_height(ui.available_height() - 52.0).show(ui, |ui| {
            let names: Vec<(String, usize)> = self.lib.playlists.iter().map(|(k, v)| (k.clone(), v.len())).collect();
            for (name, n) in names {
                let v = View::Playlist(name.clone());
                if nav_item(ui, icon::PLAYLIST, &name, Some(n), self.view == v, None) {
                    self.set_view(v);
                }
            }
        });
        ui.with_layout(egui::Layout::bottom_up(Align::Min), |ui| {
            let dot = matches!(self.update, Update::Available(_) | Update::Ready(_)).then_some(self.aura.accent);
            if nav_item(ui, icon::GEAR_SIX, "Settings", None, self.view == View::Settings, dot) {
                self.set_view(View::Settings);
            }
            ui.add_space(8.0);
            let r = ui.add(input(&mut self.new_playlist, &format!("{}  New playlist", icon::PLUS), f32::INFINITY, 14.0));
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && !self.new_playlist.trim().is_empty() {
                let name = std::mem::take(&mut self.new_playlist).trim().to_string();
                self.lib.playlists.entry(name.clone()).or_default();
                self.lib.save();
                self.set_view(View::Playlist(name));
            }
        });
    }

    fn dock(&mut self, ui: &mut Ui) {
        let full = ui.max_rect();
        let p = ui.painter();
        p.add(egui::Shadow { offset: [0, 10], blur: 40, spread: 0, color: black(90) }.as_shape(full, 22));
        p.rect_filled(full, 22, black(150));
        p.rect_stroke(full, 22, Stroke::new(1.0, HAIR), egui::StrokeKind::Inside);
        let inner = full.shrink2(vec2(16.0, 12.0));
        let left = Rect::from_min_size(inner.min, vec2(300.0, inner.height()));
        let right = Rect::from_min_max(pos2(inner.right() - 280.0, inner.top()), inner.max);
        let center = Rect::from_min_max(pos2(left.right() + 24.0, inner.top() - 3.0), pos2(right.left() - 24.0, inner.bottom()));
        let accent = self.aura.accent;

        ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| {
            ui.horizontal_centered(|ui| {
                let Some(t) = self.current.clone() else {
                    ui.label(RichText::new("Nothing playing").color(MUTED));
                    return;
                };
                let (r, resp) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::click());
                art_img(ui.painter(), &mut self.art, &t, r, 10);
                if resp.on_hover_text("Open Now playing").clicked() {
                    self.set_view(View::NowPlaying);
                }
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.set_width(176.0);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.add_space(9.0);
                    ui.add(egui::Label::new(RichText::new(&t.title).font(FontId::new(14.0, semibold())).color(TEXT)).truncate());
                    let sub = if t.is_radio() { self.radio_now.as_deref().unwrap_or(&t.artist) } else { &t.artist };
                    ui.add(egui::Label::new(RichText::new(sub).size(13.0).color(TEXT2)).truncate());
                });
                let fav = self.lib.is_fav(&t);
                if heart_btn(ui, fav).on_hover_text("Favorite  F").clicked() {
                    self.lib.toggle_fav(&t);
                }
            });
        });

        ui.scope_builder(egui::UiBuilder::new().max_rect(center), |ui| {
            ui.vertical_centered(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    let w = 4.0 * 32.0 + 44.0 + 4.0 * 12.0;
                    ui.add_space((ui.available_width() - w) / 2.0);
                    if icon_btn(ui, icon::SHUFFLE, self.smart.then_some(accent), 32.0)
                        .on_hover_text("Smart Shuffle: leans on your favorites and mixes in recommendations")
                        .clicked()
                    {
                        self.smart = !self.smart;
                        if self.smart && !self.queue.is_empty() {
                            let mut rest = self.queue.split_off(self.qpos + 1);
                            self.lib.smart_shuffle(&mut rest);
                            self.queue.extend(rest);
                            self.fetch_recs();
                            self.toast("Smart Shuffle on");
                        }
                    }
                    if icon_btn(ui, icon::SKIP_BACK, None, 32.0).on_hover_text("Previous  P").clicked() {
                        self.prev();
                    }
                    if play_btn(ui, self.playing(), self.loading()).clicked() {
                        self.toggle_play();
                    }
                    if icon_btn(ui, icon::SKIP_FORWARD, None, 32.0).on_hover_text("Next  N").clicked() {
                        self.next(false);
                    }
                    let (ic, tip) = match self.repeat {
                        Repeat::Off => (icon::REPEAT, "Repeat off"),
                        Repeat::All => (icon::REPEAT, "Repeat all"),
                        Repeat::One => (icon::REPEAT_ONCE, "Repeat one"),
                    };
                    if icon_btn(ui, ic, (self.repeat != Repeat::Off).then_some(accent), 32.0).on_hover_text(tip).clicked() {
                        self.repeat = match self.repeat {
                            Repeat::Off => Repeat::All,
                            Repeat::All => Repeat::One,
                            Repeat::One => Repeat::Off,
                        };
                    }
                });
                let (pos, dur) = (self.position(), self.duration());
                let live = self.current.as_ref().is_some_and(|t| t.is_radio());
                ui.horizontal(|ui| {
                    let mono = |s: String| RichText::new(s).font(FontId::monospace(11.0)).color(MUTED);
                    let left = if self.buffering { "--:--".into() } else { fmt_time(pos) };
                    ui.add_sized([40.0, 16.0], egui::Label::new(mono(left)));
                    let w = ui.available_width() - 48.0;
                    if live {
                        let (r, _) = ui.allocate_exact_size(vec2(w, 16.0), Sense::hover());
                        let p = ui.painter();
                        p.rect_filled(Rect::from_center_size(r.center(), vec2(r.width(), 3.0)), 2, white(30));
                        let tag = Rect::from_center_size(r.center(), vec2(60.0, 18.0));
                        p.rect_filled(tag, 9, accent);
                        p.text(tag.center(), egui::Align2::CENTER_CENTER, "LIVE", FontId::monospace(10.0), INK);
                    } else if let Some(f) = slider(ui, w, dur.map(|d| (pos / d) as f32).unwrap_or(0.0), false, accent)
                        && let Some(d) = dur
                    {
                        self.seek(f as f64 * d);
                    }
                    let right = if self.buffering { "loading".into() } else { dur.map(fmt_time).unwrap_or("--:--".into()) };
                    ui.add_sized([40.0, 16.0], egui::Label::new(mono(right)));
                });
            });
        });

        ui.scope_builder(egui::UiBuilder::new().max_rect(right), |ui| {
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if let Some(v) = slider(ui, 92.0, self.lib.volume, true, TEXT) {
                    self.lib.volume = v;
                    if let Ok(e) = &self.engine {
                        e.set_volume(v);
                    }
                }
                if ui.input(|i| i.pointer.any_released()) {
                    self.lib.save();
                }
                ui.add_space(4.0);
                ui.label(RichText::new(if self.lib.volume == 0.0 { icon::SPEAKER_X } else { icon::SPEAKER_HIGH }).size(18.0).color(TEXT2));
                ui.add_space(8.0);
                let tip = self
                    .sleep_at
                    .map(|t| format!("Pausing in {} min", t.saturating_duration_since(Instant::now()).as_secs() / 60 + 1))
                    .unwrap_or("Sleep timer".into());
                let resp = icon_btn(ui, icon::MOON, self.sleep_at.is_some().then_some(accent), 32.0).on_hover_text(tip);
                egui::Popup::menu(&resp).show(|ui| {
                    ui.label(RichText::new("Pause after").size(12.0).color(MUTED));
                    for m in [15, 30, 45, 60, 90] {
                        if ui.button(format!("{m} min")).clicked() {
                            self.sleep_at = Some(Instant::now() + Duration::from_secs(m * 60));
                            self.toast(format!("Pausing in {m} min"));
                        }
                    }
                    if self.sleep_at.is_some() && ui.button("Turn off").clicked() {
                        self.sleep_at = None;
                    }
                });
                if icon_btn(ui, icon::QUEUE, (self.view == View::Queue).then_some(accent), 32.0).on_hover_text("Queue").clicked() {
                    self.set_view(View::Queue);
                }
            });
        });
    }

    fn lyrics(&mut self, ui: &mut Ui) {
        let Some(t) = self.current.clone() else { return };
        let pos = self.position() + 0.3;
        let mut seek = None;
        match self.lyrics.get(&t.key()) {
            None | Some(Err(_)) => {}
            Some(Ok(l)) => {
                let cur = l.synced.then(|| l.lines.iter().rposition(|(ts, _)| *ts <= pos)).flatten();
                let changed = cur != self.lyric_line;
                self.lyric_line = cur;
                egui::ScrollArea::vertical().auto_shrink(false).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 14.0;
                    ui.add_space(ui.available_height() * 0.35);
                    for (i, (ts, line)) in l.lines.iter().enumerate() {
                        if line.is_empty() {
                            ui.add_space(12.0);
                            continue;
                        }
                        let col = match cur {
                            Some(c) if c == i => TEXT,
                            Some(c) if i < c => white(70),
                            _ if l.synced => white(110),
                            _ => TEXT2,
                        };
                        let rich = RichText::new(line).font(FontId::new(if l.synced { 32.0 } else { 26.0 }, serif())).color(col);
                        let r = ui.add(egui::Label::new(rich).wrap().sense(if l.synced { Sense::click() } else { Sense::hover() }));
                        if l.synced && r.clicked() {
                            seek = Some(*ts);
                        }
                        if l.synced && r.hovered() && Some(i) != cur {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if Some(i) == cur && changed {
                            r.scroll_to_me_animation(Some(Align::Center), egui::style::ScrollAnimation::duration(0.5));
                        }
                    }
                    ui.add_space(ui.available_height() * 0.4 + 240.0);
                });
            }
        }
        if let Some(ts) = seek {
            self.seek(ts);
        }
    }

    fn header(&self, ui: &mut Ui, title: &str, meta: &str) {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(RichText::new(title).font(FontId::new(56.0, serif())).color(TEXT));
        ui.label(RichText::new(meta).font(FontId::monospace(12.0)).color(TEXT2));
        ui.add_space(24.0);
    }

    fn track_source(&self, which: &View) -> Vec<Track> {
        match which {
            View::Local => &self.local,
            View::YouTube => &self.yt,
            View::Radio => &self.radios,
            View::Favorites => &self.lib.favorites,
            View::Recent | View::NowPlaying => &self.lib.recent,
            View::Settings => &self.queue[..0],
            View::Queue => &self.queue,
            View::Playlist(n) => self.lib.playlists.get(n).map(Vec::as_slice).unwrap_or(&[]),
        }
        .to_vec()
    }

    fn list(&mut self, ui: &mut Ui, which: View) {
        let list = self.track_source(&which);
        if list.is_empty() {
            let msg = match which {
                View::Local => "No audio files in this folder yet",
                View::Favorites => "Tap the heart on any song to keep it here",
                View::Recent => "Songs you play will show up here",
                View::Queue => "The queue is empty",
                View::Playlist(_) => "Empty for now. Use Add songs, or right-click any song elsewhere",
                _ => "Nothing here yet",
            };
            ui.add_space(24.0);
            ui.label(RichText::new(msg).font(FontId::new(24.0, serif())).color(MUTED));
            return;
        }
        let f = self.filter.to_lowercase();
        let idx: Vec<usize> =
            (0..list.len()).filter(|&i| f.is_empty() || format!("{} {}", list[i].title, list[i].artist).to_lowercase().contains(&f)).collect();
        let removable = matches!(which, View::Queue | View::Playlist(_) | View::Favorites);
        let playing_idx = (which == View::Queue && self.current.is_some()).then_some(self.qpos);
        let names: Vec<String> = self.lib.playlists.keys().cloned().collect();
        let row = RowCtx {
            lib: &self.lib,
            current: self.current.as_ref(),
            playing_idx,
            recs: &self.recs,
            names: &names,
            removable,
            playing: self.playing(),
            level: self.level,
            accent: self.aura.accent,
        };
        let act = if matches!(which, View::YouTube | View::Radio) {
            card_grid(ui, &list, &row, &mut self.art, which == View::Radio)
        } else {
            track_list(ui, &list, &idx, &row, &mut self.art)
        };
        if let Some(act) = act {
            self.apply(act, &list);
        }
    }

    fn play_buttons(&mut self, ui: &mut Ui, list: Vec<Track>) -> egui::Response {
        let accent = self.aura.accent;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            if pill(ui, &format!("{}  Play", icon::PLAY), accent, INK).clicked() && !list.is_empty() {
                self.play_list(list.clone(), 0);
            }
            if pill(ui, &format!("{}  Shuffle", icon::SHUFFLE), white(26), TEXT).clicked() {
                self.shuffle_play(list);
            }
        })
        .response
    }

    fn now_playing(&mut self, ui: &mut Ui) {
        let avail = ui.available_rect_before_wrap();
        let size = (avail.height() * 0.6).min(avail.width() * 0.4).clamp(220.0, 440.0);
        let cover = Rect::from_min_size(avail.min + vec2(0.0, 8.0), Vec2::splat(size));
        let cur = self.current.clone();
        let p = ui.painter().clone();
        p.add(egui::Shadow { offset: [0, 24], blur: 64, spread: 0, color: black(140) }.as_shape(cover, 18));
        match &cur {
            Some(t) => match self.art.get_hi(t) {
                Some(a) => draw_art(&p, a, cover, 18),
                None => viz::fallback_cover(&p, cover, &t.key(), &t.title, 18),
            },
            None => {
                p.rect_filled(cover, 18, white(10));
                p.text(cover.center(), egui::Align2::CENTER_CENTER, icon::VINYL_RECORD, FontId::proportional(size * 0.22), white(60));
            }
        }
        // beat: a thin accent ring that breathes with the level
        if cur.is_some() && self.playing() {
            p.rect_stroke(
                cover.expand(6.0 + 4.0 * self.level),
                22,
                Stroke::new(1.5, self.aura.accent.gamma_multiply(0.25 + 0.5 * self.level)),
                egui::StrokeKind::Outside,
            );
        }

        let info = Rect::from_min_max(pos2(cover.left(), cover.bottom() + 28.0), pos2(cover.right(), avail.bottom()));
        ui.scope_builder(egui::UiBuilder::new().max_rect(info), |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            match &cur {
                Some(t) => {
                    let mut job = egui::text::LayoutJob::single_section(t.title.clone(), egui::TextFormat::simple(FontId::new(40.0, serif()), TEXT));
                    job.wrap = egui::text::TextWrapping { max_width: info.width(), max_rows: 3, break_anywhere: false, overflow_character: Some('…') };
                    ui.add(egui::Label::new(job));
                    ui.label(RichText::new(&t.artist).size(16.0).color(TEXT2));
                    if let Some(now) = self.radio_now.clone().filter(|_| t.is_radio()) {
                        ui.add_space(16.0);
                        ui.label(RichText::new("On air").font(FontId::monospace(12.0)).color(MUTED));
                        ui.add(egui::Label::new(RichText::new(&now).font(FontId::new(18.0, semibold())).color(TEXT)).wrap());
                        ui.add_space(8.0);
                        if pill(ui, &format!("{}  Find on YouTube", icon::YOUTUBE_LOGO), white(26), TEXT).clicked() {
                            self.yt_query = now;
                            self.search_youtube();
                            self.set_view(View::YouTube);
                        }
                    }
                }
                None => {
                    ui.label(RichText::new("Nothing playing").font(FontId::new(40.0, serif())).color(TEXT));
                    ui.label(RichText::new("Find something on YouTube, the radio or in your files").size(16.0).color(TEXT2));
                }
            }
        });

        let col = Rect::from_min_max(pos2(cover.right() + 64.0, avail.top()), avail.max);
        ui.scope_builder(egui::UiBuilder::new().max_rect(col), |ui| {
            if cur.is_none() {
                ui.label(RichText::new("Recently played").font(FontId::new(15.0, semibold())).color(TEXT2));
                ui.add_space(12.0);
                self.list(ui, View::NowPlaying);
                return;
            }
            // no lyrics (radio, not found, still searching): no tabs, the kinetic type fills the column
            let has_lyrics = cur.as_ref().is_some_and(|t| matches!(self.lyrics.get(&t.key()), Some(Ok(_))));
            if !has_lyrics {
                self.marquee(ui);
                return;
            }
            self.lyrics(ui);
        });
    }

    fn central(&mut self, ui: &mut Ui) {
        let now = ui.input(|i| i.time);
        let a = ease(((now - self.view_since) / 0.35).clamp(0.0, 1.0) as f32);
        if a < 1.0 {
            ui.ctx().request_repaint();
        }
        ui.set_opacity(a);
        ui.add_space((1.0 - a) * 10.0);

        match self.view.clone() {
            View::NowPlaying => self.now_playing(ui),
            View::Settings => self.settings(ui),
            View::Local => {
                self.header(ui, "Your files", &format!("{} tracks · {}", self.local.len(), self.lib.music_dir));
                ui.horizontal(|ui| {
                    self.play_buttons(ui, self.local.clone());
                    ui.add_space(12.0);
                    let r = ui.add(input(&mut self.lib.music_dir, &format!("{}  Music folder", icon::FOLDER_SIMPLE), 280.0, 14.0));
                    if ui.button("Rescan").clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))) {
                        self.lib.save();
                        self.scan();
                    }
                    if self.busy.contains("local") {
                        ui.spinner();
                    }
                    filter_box(ui, &mut self.filter);
                });
                ui.add_space(20.0);
                self.list(ui, View::Local);
            }
            View::YouTube => {
                self.header(ui, "YouTube", "Any song, live set or mix. Paste a playlist link to load it all");
                ui.horizontal(|ui| {
                    let r = ui.add(input(&mut self.yt_query, &format!("{}  What do you want to hear", icon::MAGNIFYING_GLASS), 520.0, 17.0));
                    if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        self.search_youtube();
                    }
                    if self.busy.contains("yt") || self.busy.contains("resolve") {
                        ui.spinner();
                    }
                });
                ui.add_space(28.0);
                if self.yt.is_empty() {
                    ui.label(RichText::new("Search to fill this page").font(FontId::new(24.0, serif())).color(MUTED));
                } else {
                    self.list(ui, View::YouTube);
                }
            }
            View::Radio => {
                self.header(ui, "Radio", &format!("{} · {} stations", self.radio_label, self.radios.len()));
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                    if chip(ui, "Our picks", self.radio_label == "Our picks") {
                        self.radios = sources::curated_radios();
                        self.radio_label = "Our picks".into();
                    }
                    if chip(ui, "Most loved", self.radio_label == "Most loved") {
                        self.search_radio(String::new(), false, "Most loved".into());
                    }
                    for g in GENRES {
                        if chip(ui, g, self.radio_label == *g) {
                            self.search_radio(g.to_string(), true, g.to_string());
                        }
                    }
                    let r = ui.add(input(&mut self.radio_query, &format!("{}  Station", icon::MAGNIFYING_GLASS), 180.0, 13.0));
                    if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && !self.radio_query.trim().is_empty() {
                        let q = self.radio_query.trim().to_string();
                        self.search_radio(q.clone(), false, format!("“{q}”"));
                    }
                    if self.busy.contains("radio") {
                        ui.spinner();
                    }
                });
                ui.add_space(28.0);
                self.list(ui, View::Radio);
            }
            View::Favorites => {
                self.header(ui, "Favorites", &format!("{} songs", self.lib.favorites.len()));
                ui.horizontal(|ui| {
                    self.play_buttons(ui, self.lib.favorites.clone());
                    filter_box(ui, &mut self.filter);
                });
                ui.add_space(20.0);
                self.list(ui, View::Favorites);
            }
            View::Recent => {
                self.header(ui, "Recently played", "Your last 50");
                self.list(ui, View::Recent);
            }
            View::Queue => {
                let meta = if self.smart { format!("{} tracks · smart shuffle on", self.queue.len()) } else { format!("{} tracks", self.queue.len()) };
                self.header(ui, "Queue", &meta);
                ui.horizontal(|ui| {
                    if ui.button("Clear upcoming").clicked() {
                        self.queue.truncate(self.qpos + 1);
                    }
                    if self.busy.contains("recs") {
                        ui.spinner();
                        ui.label(RichText::new("Finding songs you might like").color(TEXT2));
                    }
                });
                ui.add_space(20.0);
                self.list(ui, View::Queue);
            }
            View::Playlist(name) => {
                let list = self.lib.playlists.get(&name).cloned().unwrap_or_default();
                self.header(ui, &name, &format!("Playlist · {} tracks", list.len()));
                ui.horizontal(|ui| {
                    self.play_buttons(ui, list.clone());
                    if pill(ui, &format!("{}  Add songs", icon::PLUS), white(26), TEXT).clicked() {
                        self.picker = Some(name.clone());
                        self.pick_query.clear();
                    }
                    ui.add_space(4.0);
                    if icon_btn(ui, icon::TRASH, None, 40.0).on_hover_text("Delete playlist").clicked() {
                        self.lib.playlists.remove(&name);
                        self.lib.save();
                        self.set_view(View::Favorites);
                    }
                });
                ui.add_space(20.0);
                self.list(ui, View::Playlist(name));
            }
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.tick(ctx);
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.keys(&ctx);
        self.paint_background(ui);
        self.picker_modal(&ctx);

        if let Err(e) = &self.engine {
            egui::Panel::top("err").frame(egui::Frame::new().fill(black(160)).inner_margin(Margin::symmetric(16, 8))).show(ui, |ui| {
                ui.label(RichText::new(format!("No audio output: {e}")).color(Color32::from_rgb(255, 120, 120)));
            });
        }
        egui::Panel::bottom("dock")
            .exact_size(104.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().inner_margin(Margin { left: 16, right: 16, top: 0, bottom: 16 }))
            .show(ui, |ui| self.dock(ui));
        egui::Panel::left("nav")
            .exact_size(232.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().inner_margin(Margin { left: 20, right: 12, top: 24, bottom: 16 }))
            .show(ui, |ui| self.nav(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(Margin { left: 32, right: 40, top: 32, bottom: 8 }))
            .show(ui, |ui| self.central(ui));

        if let Some((msg, t0)) = self.toast.clone() {
            let age = ctx.input(|i| i.time) - t0;
            if age > 3.0 {
                self.toast = None;
            } else {
                let a = ease((age / 0.2).min(1.0).min((3.0 - age) / 0.2) as f32);
                egui::Area::new("toast".into()).anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -124.0 + (1.0 - a) * 8.0)).show(&ctx, |ui| {
                    ui.set_opacity(a);
                    egui::Frame::new()
                        .fill(Color32::from_rgb(30, 30, 34))
                        .corner_radius(20)
                        .inner_margin(Margin::symmetric(18, 10))
                        .stroke(Stroke::new(1.0, HAIR))
                        .show(ui, |ui| {
                            ui.label(RichText::new(msg).color(TEXT));
                        });
                });
                ctx.request_repaint();
            }
        }
    }
}

// ---------- widgets ----------

struct RowCtx<'a> {
    lib: &'a Library,
    current: Option<&'a Track>,
    playing_idx: Option<usize>,
    recs: &'a HashSet<String>,
    names: &'a [String],
    removable: bool,
    playing: bool,
    level: f32,
    accent: Color32,
}

impl RowCtx<'_> {
    fn is_current(&self, i: usize, t: &Track) -> bool {
        match self.playing_idx {
            Some(p) => p == i,
            None => self.current.is_some_and(|cur| cur.source == t.source),
        }
    }
}

fn art_img(p: &egui::Painter, cache: &mut ArtCache, t: &Track, r: Rect, radius: u8) {
    match cache.get(t) {
        Some(a) => draw_art(p, a, r, radius),
        None => viz::fallback_cover(p, r, &t.key(), &t.title, radius),
    }
}

fn draw_art(p: &egui::Painter, a: &art::Art, r: Rect, radius: u8) {
    if a.small {
        // a tiny favicon stretched looks smeared: center it on a tile of its own color
        p.rect_filled(r, radius, a.grid[5].gamma_multiply(0.6));
        let side = (r.width() * 0.42).min(a.tex.size_vec2().x * 3.0);
        let icon = Rect::from_center_size(r.center(), Vec2::splat(side));
        p.add(egui::epaint::RectShape::filled(icon, (side * 0.18) as u8, Color32::WHITE).with_texture(a.tex.id(), art::cover_uv(a.aspect, icon)));
    } else {
        p.add(egui::epaint::RectShape::filled(r, radius, Color32::WHITE).with_texture(a.tex.id(), art::cover_uv(a.aspect, r)));
    }
}

fn context_menu(resp: &egui::Response, i: usize, fav: bool, c: &RowCtx, act: &mut Option<RowAct>) {
    resp.context_menu(|ui| {
        if ui.button(format!("{}  Play", icon::PLAY)).clicked() {
            *act = Some(RowAct::Play(i));
        }
        if ui.button(format!("{}  Play next", icon::SKIP_FORWARD)).clicked() {
            *act = Some(RowAct::Next(i));
        }
        if ui.button(format!("{}  Add to queue", icon::PLUS)).clicked() {
            *act = Some(RowAct::Enqueue(i));
        }
        if ui.button(format!("{}  {}", icon::HEART, if fav { "Remove from favorites" } else { "Add to favorites" })).clicked() {
            *act = Some(RowAct::Fav(i));
        }
        ui.menu_button(format!("{}  Add to playlist", icon::PLAYLIST), |ui| {
            if c.names.is_empty() {
                ui.label(RichText::new("Create one in the sidebar").color(MUTED));
            }
            for n in c.names {
                if ui.button(n).clicked() {
                    *act = Some(RowAct::AddTo(i, n.clone()));
                }
            }
        });
        if c.removable && ui.button(format!("{}  Remove", icon::X)).clicked() {
            *act = Some(RowAct::Remove(i));
        }
    });
}

fn track_list(ui: &mut Ui, list: &[Track], idx: &[usize], c: &RowCtx, cache: &mut ArtCache) -> Option<RowAct> {
    let mut act = None;
    let t_now = ui.input(|i| i.time) as f32;
    egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, ROW_H, idx.len(), |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for &i in &idx[range] {
            let t = &list[i];
            let is_cur = c.is_current(i, t);
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
            let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.15);
            let p = ui.painter();
            if is_cur {
                p.rect_filled(r, 12, SELECTED);
            } else if hover > 0.0 {
                p.rect_filled(r, 12, white((13.0 * hover) as u8));
            }
            let num = r.left_center() + vec2(26.0, 0.0);
            if is_cur {
                viz::eq(p, num - vec2(4.0, 0.0), t_now, c.level, c.playing, c.accent);
            } else if hover > 0.5 {
                p.text(num, egui::Align2::RIGHT_CENTER, icon::PLAY, FontId::proportional(15.0), TEXT);
            } else {
                p.text(num, egui::Align2::RIGHT_CENTER, format!("{}", i + 1), FontId::monospace(12.0), MUTED);
            }
            let thumb = Rect::from_min_size(r.min + vec2(44.0, 8.0), Vec2::splat(40.0));
            art_img(p, cache, t, thumb, 8);
            let text_w = r.width() - 240.0;
            let title_col = if is_cur { c.accent } else { TEXT };
            text(p, thumb.right_top() + vec2(14.0, 1.0), &t.title, 15.0, title_col, text_w, semibold());
            let sub = if c.recs.contains(&t.key()) { format!("For you · {}", t.artist) } else { t.artist.clone() };
            text(p, thumb.right_top() + vec2(14.0, 21.0), &sub, 13.0, TEXT2, text_w, FontFamily::Proportional);
            if let Some(d) = t.duration {
                p.text(pos2(r.right() - 88.0, r.center().y), egui::Align2::RIGHT_CENTER, fmt_time(d), FontId::monospace(12.0), MUTED);
            }
            let fav = c.lib.is_fav(t);
            let heart = Rect::from_center_size(pos2(r.right() - 56.0, r.center().y), Vec2::splat(28.0));
            let plus = Rect::from_center_size(pos2(r.right() - 24.0, r.center().y), Vec2::splat(28.0));
            let pointer = ui.input(|i| i.pointer.hover_pos()).unwrap_or_default();
            let hl = |rect: Rect| if rect.contains(pointer) { TEXT } else { TEXT2 };
            if fav || hover > 0.0 {
                heart_icon(p, heart.center(), 17.0, fav, hl(heart).gamma_multiply(hover));
            }
            if hover > 0.0 {
                p.text(plus.center(), egui::Align2::CENTER_CENTER, icon::PLUS, FontId::proportional(17.0), hl(plus).gamma_multiply(hover));
            }
            if resp.clicked() {
                let at = resp.interact_pointer_pos().unwrap_or_default();
                act = Some(if heart.contains(at) {
                    RowAct::Fav(i)
                } else if plus.contains(at) {
                    RowAct::Enqueue(i)
                } else {
                    RowAct::Play(i)
                });
            }
            let resp = if heart.contains(pointer) {
                resp.on_hover_text(if fav { "Remove from favorites" } else { "Add to favorites" })
            } else if plus.contains(pointer) {
                resp.on_hover_text("Add to queue")
            } else {
                resp
            };
            context_menu(&resp, i, fav, c, &mut act);
        }
    });
    act
}

/// Artwork cards: 16:9 for YouTube videos, square for radio stations.
fn card_grid(ui: &mut Ui, list: &[Track], c: &RowCtx, cache: &mut ArtCache, square: bool) -> Option<RowAct> {
    let mut act = None;
    let gap = 20.0;
    let width = ui.available_width() - 12.0;
    let target = if square { 176.0 } else { 248.0 };
    let cols = ((width + gap) / (target + gap)).floor().max(1.0) as usize;
    let cw = (width - gap * (cols - 1) as f32) / cols as f32;
    let ih = if square { cw } else { cw * 9.0 / 16.0 };
    let ch = ih + 64.0;
    let rows = list.len().div_ceil(cols);
    egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, ch + gap, rows, |ui, range| {
        for row in range {
            let (band, _) = ui.allocate_exact_size(vec2(width, ch + gap), Sense::hover());
            for col in 0..cols {
                let i = row * cols + col;
                let Some(t) = list.get(i) else { break };
                let card = Rect::from_min_size(band.min + vec2(col as f32 * (cw + gap), 0.0), vec2(cw, ch));
                let resp = ui.interact(card, ui.id().with(("card", i)), Sense::click());
                let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.18);
                let is_cur = c.is_current(i, t);
                let img = Rect::from_min_size(card.min, vec2(cw, ih));
                let p = ui.painter();
                p.add(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: black((40.0 + 40.0 * hover) as u8) }.as_shape(img, 14));
                art_img(p, cache, t, img, 14);
                if hover > 0.0 {
                    p.rect_filled(img, 14, black((70.0 * hover) as u8));
                    let btn = img.right_bottom() - vec2(30.0, 30.0) + vec2(0.0, 6.0 * (1.0 - hover));
                    p.circle_filled(btn, 22.0, c.accent.gamma_multiply(hover));
                    p.text(btn + vec2(1.0, 0.0), egui::Align2::CENTER_CENTER, icon::PLAY, FontId::proportional(18.0), INK.gamma_multiply(hover));
                }
                if is_cur {
                    p.rect_stroke(img, 14, Stroke::new(2.0, c.accent), egui::StrokeKind::Inside);
                    let tag = Rect::from_min_size(img.min + vec2(10.0, 10.0), vec2(28.0, 24.0));
                    p.rect_filled(tag, 8, black(170));
                    viz::eq(p, tag.center() + vec2(1.0, 0.0), ui.input(|i| i.time) as f32, c.level, c.playing, c.accent);
                }
                if let Some(d) = t.duration {
                    let s = fmt_time(d);
                    let tag = Rect::from_min_size(img.min + vec2(10.0, ih - 30.0), vec2(8.0 + 7.0 * s.len() as f32, 20.0));
                    p.rect_filled(tag, 6, black(170));
                    p.text(tag.center(), egui::Align2::CENTER_CENTER, s, FontId::monospace(11.0), TEXT);
                }
                let fav = c.lib.is_fav(t);
                let heart = Rect::from_center_size(img.right_top() + vec2(-24.0, 24.0), Vec2::splat(32.0));
                if fav || hover > 0.0 {
                    let a = if fav { 1.0 } else { hover };
                    p.circle_filled(heart.center(), 16.0, black((150.0 * a) as u8));
                    heart_icon(p, heart.center(), 16.0, fav, TEXT.gamma_multiply(a));
                }
                text(p, img.left_bottom() + vec2(2.0, 12.0), &t.title, 15.0, if is_cur { c.accent } else { TEXT }, cw - 4.0, semibold());
                text(p, img.left_bottom() + vec2(2.0, 33.0), &t.artist, 13.0, TEXT2, cw - 4.0, FontFamily::Proportional);
                if resp.clicked() {
                    let at = resp.interact_pointer_pos().unwrap_or_default();
                    act = Some(if heart.contains(at) { RowAct::Fav(i) } else { RowAct::Play(i) });
                }
                context_menu(&resp, i, fav, c, &mut act);
            }
        }
    });
    act
}

fn text(p: &egui::Painter, pos: egui::Pos2, s: &str, size: f32, color: Color32, max_w: f32, family: FontFamily) {
    let mut job = egui::text::LayoutJob::single_section(s.to_string(), egui::TextFormat::simple(FontId::new(size, family), color));
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_w.max(20.0));
    p.galley(pos, p.layout_job(job), color);
}

fn section(ui: &mut Ui, s: &str) {
    ui.label(RichText::new(s).size(12.0).color(MUTED));
    ui.add_space(6.0);
}

fn nav_item(ui: &mut Ui, ic: &str, label: &str, count: Option<usize>, selected: bool, dot: Option<Color32>) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click());
    let hov = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.15);
    let sel = ui.ctx().animate_bool_with_time(resp.id.with("sel"), selected, 0.2);
    let p = ui.painter();
    p.rect_filled(r, 12, white((13.0 * hov + 14.0 * sel) as u8));
    let col = lerp(TEXT2, TEXT, sel.max(hov));
    p.text(r.left_center() + vec2(12.0, 0.0), egui::Align2::LEFT_CENTER, ic, FontId::proportional(18.0), col);
    let right_pad = if count.is_some() || dot.is_some() { 44.0 } else { 12.0 };
    let family = if selected { semibold() } else { FontFamily::Proportional };
    text(p, r.left_center() + vec2(42.0, -10.0), label, 15.0, col, r.width() - 42.0 - right_pad, family);
    if let Some(c) = dot {
        p.circle_filled(r.right_center() - vec2(16.0, 0.0), 4.0, c);
    } else if let Some(n) = count.filter(|&n| n > 0) {
        p.text(r.right_center() - vec2(12.0, 0.0), egui::Align2::RIGHT_CENTER, n.to_string(), FontId::monospace(11.0), MUTED);
    }
    resp.clicked()
}

fn pill(ui: &mut Ui, label: &str, fill: Color32, color: Color32) -> egui::Response {
    let font = FontId::new(15.0, semibold());
    let galley = ui.painter().layout_no_wrap(label.to_string(), font.clone(), color);
    let (r, resp) = ui.allocate_exact_size(vec2(galley.size().x + 40.0, 44.0), Sense::click());
    let h = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.15);
    let r2 = if resp.is_pointer_button_down_on() { r.shrink(1.0) } else { r };
    let p = ui.painter();
    p.rect_filled(r2, 22, fill);
    p.rect_filled(r2, 22, white((16.0 * h) as u8));
    p.text(r2.center(), egui::Align2::CENTER_CENTER, label, font, color);
    resp
}

fn chip(ui: &mut Ui, label: &str, selected: bool) -> bool {
    let font = FontId::proportional(14.0);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font.clone(), TEXT);
    let (r, resp) = ui.allocate_exact_size(vec2(galley.size().x + 28.0, 34.0), Sense::click());
    let hov = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.15);
    let p = ui.painter();
    if selected {
        p.rect_filled(r, 17, TEXT);
    } else {
        p.rect_filled(r, 17, white((16.0 + 14.0 * hov) as u8));
    }
    p.text(r.center(), egui::Align2::CENTER_CENTER, label, font, if selected { INK } else { TEXT });
    resp.clicked()
}

/// Favorites get a filled red heart so they stand out on any background; others an outline.
fn heart_icon(p: &egui::Painter, center: egui::Pos2, size: f32, fav: bool, idle: Color32) {
    if fav {
        p.text(center, egui::Align2::CENTER_CENTER, egui_phosphor::fill::HEART, FontId::new(size, filled()), HEART_RED);
    } else {
        p.text(center, egui::Align2::CENTER_CENTER, icon::HEART, FontId::proportional(size), idle);
    }
}

fn heart_btn(ui: &mut Ui, fav: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::click());
    let h = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.15);
    let pop = ui.ctx().animate_bool_with_time(resp.id.with("fav"), fav, 0.2);
    let p = ui.painter();
    p.circle_filled(r.center(), 16.0, white((18.0 * h) as u8));
    // a small pop when it turns red
    let size = 19.0 * (1.0 + 0.25 * (pop * std::f32::consts::PI).sin());
    heart_icon(p, r.center(), size, fav, lerp(TEXT2, TEXT, h));
    resp
}

fn icon_btn(ui: &mut Ui, ic: &str, active: Option<Color32>, size: f32) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let h = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.15);
    let p = ui.painter();
    p.circle_filled(r.center(), size / 2.0, white((18.0 * h) as u8));
    let col = active.unwrap_or(lerp(TEXT2, TEXT, h));
    let press = if resp.is_pointer_button_down_on() { 0.94 } else { 1.0 };
    p.text(r.center(), egui::Align2::CENTER_CENTER, ic, FontId::proportional(19.0 * press), col);
    resp
}

fn play_btn(ui: &mut Ui, playing: bool, loading: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::click());
    let h = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.15);
    let press = if resp.is_pointer_button_down_on() { 0.95 } else { 1.0 };
    let p = ui.painter();
    p.circle_filled(r.center(), (21.0 + 1.0 * h) * press, TEXT);
    if loading {
        let t = ui.input(|i| i.time) as f32;
        let pts: Vec<egui::Pos2> = (0..20).map(|i| r.center() + Vec2::angled(t * 5.0 + i as f32 * 0.22) * 8.0).collect();
        p.add(egui::Shape::line(pts, Stroke::new(2.0, INK)));
    } else {
        let ic = if playing { icon::PAUSE } else { icon::PLAY };
        let nudge = if playing { 0.0 } else { 1.0 }; // optical centering of the triangle
        p.text(r.center() + vec2(nudge, 0.0), egui::Align2::CENTER_CENTER, ic, FontId::proportional(20.0), INK);
    }
    resp.on_hover_text("Play / pause  Space")
}

/// Thin slider. Reports while dragging if `live`, otherwise only on click / release
/// (used for seeking, which restarts the decoder).
fn slider(ui: &mut Ui, w: f32, value: f32, live: bool, fill_col: Color32) -> Option<f32> {
    let (r, resp) = ui.allocate_exact_size(vec2(w, 16.0), Sense::click_and_drag());
    let h = ui.ctx().animate_bool_with_time(resp.id, resp.hovered() || resp.dragged(), 0.15);
    let pointer = resp.interact_pointer_pos().map(|p| ((p.x - r.left()) / r.width()).clamp(0.0, 1.0));
    let shown = if resp.dragged() { pointer.unwrap_or(value) } else { value.clamp(0.0, 1.0) };
    let track = Rect::from_center_size(r.center(), vec2(r.width(), 3.0 + 2.0 * h));
    let p = ui.painter();
    p.rect_filled(track, 3, white(34));
    let fill = Rect::from_min_max(track.min, pos2(track.left() + track.width() * shown, track.bottom()));
    p.rect_filled(fill, 3, fill_col);
    if h > 0.0 {
        p.circle_filled(pos2(fill.right(), track.center().y), 6.0 * h, TEXT);
    }
    if live && resp.dragged() || resp.clicked() || resp.drag_stopped() { pointer } else { None }
}

fn input<'a>(s: &'a mut String, hint: &str, w: f32, size: f32) -> egui::TextEdit<'a> {
    let pad = (size * 0.7) as i8;
    egui::TextEdit::singleline(s)
        .hint_text(RichText::new(hint).color(MUTED))
        .font(FontId::proportional(size))
        .desired_width(w)
        .margin(Margin::symmetric(pad + 4, pad))
}

fn filter_box(ui: &mut Ui, s: &mut String) {
    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
        ui.add(input(s, &format!("{}  Filter", icon::MAGNIFYING_GLASS), 220.0, 14.0));
    });
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgba_premultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

fn logo_texture(ctx: &egui::Context) -> egui::TextureHandle {
    let n = 96;
    ctx.load_texture("logo", egui::ColorImage::from_rgba_unmultiplied([n, n], &logo::rgba(n)), egui::TextureOptions::LINEAR)
}

/// Wayland takes window icons from a desktop entry, not from the app: install one (and the
/// icon) the first time, which also puts Sonavelt in the app launcher.
fn install_desktop_entry() {
    let (Some(data), Ok(exe)) = (dirs::data_dir(), std::env::current_exe()) else { return };
    let icon = data.join("icons/hicolor/256x256/apps/sonavelt.png");
    if !icon.exists() {
        let _ = std::fs::create_dir_all(icon.parent().unwrap());
        let _ = image::save_buffer(&icon, &logo::rgba(256), 256, 256, image::ExtendedColorType::Rgba8);
    }
    let entry = data.join("applications/sonavelt.desktop");
    let body = format!(
        "[Desktop Entry]\nType=Application\nName=Sonavelt\nComment=Music from YouTube, radio and your files\nExec={}\nIcon=sonavelt\nCategories=Audio;Music;Player;\nStartupWMClass=sonavelt\n",
        exe.display()
    );
    if std::fs::read_to_string(&entry).ok().as_deref() != Some(body.as_str()) {
        let _ = std::fs::create_dir_all(entry.parent().unwrap());
        let _ = std::fs::write(&entry, body);
    }
}

fn ease(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

fn fmt_time(s: f64) -> String {
    let s = s.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn style(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for (name, bytes) in [
        ("bricolage", &include_bytes!("../assets/fonts/Bricolage-Regular.ttf")[..]),
        ("bricolage-sb", &include_bytes!("../assets/fonts/Bricolage-SemiBold.ttf")[..]),
        ("instrument", &include_bytes!("../assets/fonts/InstrumentSerif.ttf")[..]),
        ("dmmono", &include_bytes!("../assets/fonts/DMMono.ttf")[..]),
    ] {
        fonts.font_data.insert(name.into(), egui::FontData::from_static(bytes).into());
    }
    let fallback = fonts.families[&FontFamily::Proportional].clone();
    fonts.families.get_mut(&FontFamily::Proportional).unwrap().insert(0, "bricolage".into());
    fonts.families.get_mut(&FontFamily::Monospace).unwrap().insert(0, "dmmono".into());
    fonts.families.insert(semibold(), [vec!["bricolage-sb".into()], fallback.clone()].concat());
    fonts.families.insert(serif(), [vec!["instrument".into()], fallback].concat());
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    fonts.font_data.insert("phosphor-fill".into(), egui::FontData::from_static(egui_phosphor::Variant::Fill.font_bytes()).into());
    fonts.families.insert(filled(), vec!["phosphor-fill".into()]);
    fonts.families.get_mut(&semibold()).unwrap().insert(1, "phosphor".into());
    ctx.set_fonts(fonts);

    ctx.options_mut(|o| o.theme_preference = egui::ThemePreference::Dark);
    let mut v = egui::Visuals::dark();
    v.panel_fill = Color32::TRANSPARENT;
    v.window_fill = MENU;
    v.extreme_bg_color = white(16);
    v.faint_bg_color = white(8);
    v.selection.bg_fill = white(60);
    v.selection.stroke = Stroke::new(1.0, white(90));
    v.hyperlink_color = TEXT;
    v.window_corner_radius = CornerRadius::same(14);
    v.menu_corner_radius = CornerRadius::same(14);
    v.window_stroke = Stroke::new(1.0, HAIR);
    v.window_shadow = egui::Shadow { offset: [0, 12], blur: 32, spread: 0, color: black(120) };
    v.popup_shadow = v.window_shadow;
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open, &mut v.widgets.noninteractive] {
        w.corner_radius = CornerRadius::same(12);
        w.bg_stroke = Stroke::NONE;
        w.expansion = 0.0;
    }
    v.widgets.noninteractive.fg_stroke.color = TEXT2;
    v.widgets.inactive.weak_bg_fill = white(20);
    v.widgets.inactive.bg_fill = white(20);
    v.widgets.inactive.fg_stroke.color = TEXT;
    v.widgets.hovered.weak_bg_fill = white(34);
    v.widgets.hovered.bg_fill = white(34);
    v.widgets.hovered.fg_stroke.color = TEXT;
    v.widgets.active.weak_bg_fill = white(44);
    v.widgets.active.fg_stroke.color = TEXT;
    v.widgets.open.weak_bg_fill = white(28);
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = vec2(8.0, 8.0);
        s.spacing.button_padding = vec2(16.0, 10.0);
        s.spacing.interact_size.y = 36.0;
        s.spacing.menu_margin = Margin::same(8);
        s.animation_time = 0.18;
        for (ts, size) in [
            (egui::TextStyle::Body, 14.0),
            (egui::TextStyle::Button, 14.0),
            (egui::TextStyle::Heading, 28.0),
            (egui::TextStyle::Small, 12.0),
            (egui::TextStyle::Monospace, 12.0),
        ] {
            if let Some(f) = s.text_styles.get_mut(&ts) {
                f.size = size;
            }
        }
    });
}

// ---------- settings, kinetic type, picker, tray ----------

impl App {
    fn paint_background(&mut self, ui: &mut Ui) {
        let s = self.lib.settings.clone();
        let t = if s.motion { ui.input(|i| i.time) as f32 } else { 0.0 };
        let gain = s.brightness + 0.12 * self.level;
        let r = ui.max_rect();
        let p = ui.painter();
        match s.background {
            Background::Solid => {
                p.rect_filled(r, 0, INK);
            }
            Background::BlurredCover => match self.current.clone().and_then(|c| self.art.get(&c).map(|a| a.blur.clone())) {
                Some(tex) => viz::blurred_cover(p, r, &tex, t, gain),
                None => self.aura.paint(p, r, t, gain),
            },
            Background::Aura => self.aura.paint(p, r, t, gain),
        }
    }

    /// Kinetic type for songs without lyrics: the title and artist in huge serif rows that
    /// slide in alternating directions, quickening with the music.
    fn marquee(&mut self, ui: &mut Ui) {
        let Some(t) = self.current.clone() else { return };
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        // moves only while music plays; the per-frame repaint then comes from `tick`
        if self.lib.settings.motion && self.playing() {
            self.marquee_phase += dt * (1.0 + 1.2 * self.level);
        }
        let area = ui.available_rect_before_wrap();
        ui.allocate_rect(area, Sense::hover());
        let r = Rect::from_min_max(area.min + vec2(0.0, 16.0), area.max - vec2(0.0, 16.0));
        let p = ui.painter().with_clip_rect(r);
        let size = (r.height() / 4.2).clamp(48.0, 150.0);
        let rows = ((r.height() / (size * 0.92)).floor() as usize).max(1);
        let gap = (r.height() - rows as f32 * size * 0.92) / 2.0;
        for i in 0..rows {
            let second = if t.is_radio() { self.radio_now.as_ref().unwrap_or(&t.artist) } else { &t.artist };
            let text = if i % 2 == 0 { &t.title } else { second };
            let col = if i == rows / 2 { self.aura.accent.gamma_multiply(0.45 + 0.4 * self.level) } else { white((16.0 + 22.0 * self.level) as u8) };
            let g = p.layout_no_wrap(format!("{text}  —  "), FontId::new(size, serif()), col);
            let w = g.size().x.max(1.0);
            let dir = if i % 2 == 0 { -1.0 } else { 1.0 };
            let speed = 26.0 + 14.0 * (i % 3) as f32;
            let offset = (self.marquee_phase * speed * dir).rem_euclid(w);
            let y = r.top() + gap + i as f32 * size * 0.92;
            let mut x = r.left() - offset - 40.0;
            while x < r.right() + 40.0 {
                p.galley(pos2(x, y), g.clone(), col);
                x += w;
            }
        }
    }

    fn check_update(&mut self, quiet: bool) {
        self.update = Update::Checking;
        self.spawn("update", move || Msg::UpdateChecked(quiet, updater::check()));
    }

    fn install_update(&mut self) {
        let Update::Available(rel) = &self.update else { return };
        let (rel, exe) = (rel.clone(), self.exe.clone());
        self.update = Update::Installing(rel.version.clone());
        self.spawn("update", move || Msg::UpdateInstalled(updater::install(&rel, &exe).map(|_| rel.version)));
    }

    /// Start the freshly installed build, then quit this one.
    fn restart(&mut self, ctx: &egui::Context) {
        self.lib.save();
        match std::process::Command::new(&self.exe).spawn() {
            Ok(_) => {
                self.quitting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Err(e) => self.update = Update::Failed(format!("Couldn't restart: {e}")),
        }
    }

    /// Keep media keys, lock screens and desktop widgets in step with what's playing.
    fn mpris_tick(&mut self) {
        if self.mpris.is_none() {
            return;
        }
        let status = match (&self.engine, &self.current) {
            (Ok(e), Some(_)) if e.active() && !e.paused() => "Playing",
            (_, Some(_)) => "Paused", // a loaded song is resumable, so media keys can start it
            _ => "Stopped",
        };
        let now = match &self.current {
            None => mpris::Now { status, volume: self.lib.volume as f64, ..Default::default() },
            Some(t) => {
                let (title, artist, album) = match (&t.source, &self.radio_now) {
                    (Source::Radio(_), Some(song)) => match song.split_once(" - ") {
                        Some((a, s)) => (s.to_string(), a.to_string(), t.title.clone()),
                        None => (song.clone(), t.title.clone(), t.title.clone()),
                    },
                    (Source::Radio(_), None) => (t.title.clone(), t.artist.clone(), String::new()),
                    _ => (t.title.clone(), t.artist.clone(), String::new()),
                };
                let art = match &t.source {
                    Source::YouTube(id) => Some(format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg")),
                    Source::Radio(_) => t.art.clone(),
                    Source::Local(_) => None,
                };
                let length = if t.is_radio() { None } else { self.duration() };
                mpris::Now { status, id: self.play_gen, title, artist, album, art, length, volume: self.lib.volume as f64 }
            }
        };
        let position = self.position();
        if let Some(m) = &mut self.mpris {
            m.update(now, position);
        }
    }

    /// While a station plays, ask it what song is on about every 20 seconds.
    fn poll_radio_now(&mut self) {
        let Some(Source::Radio(url)) = self.current.as_ref().map(|t| t.source.clone()) else { return };
        if !self.playing() || self.busy.contains("icy") || self.radio_polled.is_some_and(|t| t.elapsed() < Duration::from_secs(20)) {
            return;
        }
        self.radio_polled = Some(Instant::now());
        let generation = self.play_gen;
        self.spawn("icy", move || Msg::RadioNow(generation, sources::radio_now(&url)));
    }

    fn tray_tick(&mut self) {
        while let Ok(cmd) = self.tray_rx.try_recv() {
            self.tray_cmd(cmd);
        }
        let now = match (&self.current, &self.radio_now) {
            (Some(t), Some(song)) if t.is_radio() => format!("{song} — {}", t.title),
            (Some(t), _) => format!("{} — {}", t.title, t.artist),
            _ => "Nothing playing".into(),
        };
        let state = (now, self.playing());
        if state != self.tray_state {
            if let Some(h) = &self.tray {
                let (n, p) = state.clone();
                h.update(|t| {
                    t.now = n;
                    t.playing = p;
                });
            }
            self.tray_state = state;
        }
    }

    /// A new window opened on the existing state: rebind everything tied to the old context.
    fn attach(&mut self, cc: &eframe::CreationContext) {
        self.ctx = cc.egui_ctx.clone();
        style(&self.ctx);
        self.ctx.set_zoom_factor(self.lib.settings.ui_scale);
        self.art = ArtCache::new(&self.ctx); // textures belonged to the old renderer
        self.logo = logo_texture(&self.ctx);
        self.view_since = 0.0;
        self.last_time = 0.0;
        if let Some(h) = &self.tray {
            let ctx = self.ctx.clone();
            h.update(|t| t.ctx = ctx);
        }
        if let Some(m) = &self.mpris {
            m.set_ctx(&self.ctx);
        }
    }

    /// Runs while the window is closed: keeps the queue moving and answers the tray.
    /// Returns true when the window should reopen.
    fn background(&mut self) -> bool {
        loop {
            match self.tray_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(tray::TrayCmd::Show) => return true,
                Ok(tray::TrayCmd::Quit) | Err(RecvTimeoutError::Disconnected) => return false,
                Ok(cmd) => self.tray_cmd(cmd),
                Err(RecvTimeoutError::Timeout) => {}
            }
            let ctx = self.ctx.clone();
            self.tick(&ctx);
        }
    }

    fn tray_cmd(&mut self, cmd: tray::TrayCmd) {
        match cmd {
            tray::TrayCmd::Toggle => self.toggle_play(),
            tray::TrayCmd::Next => self.next(false),
            tray::TrayCmd::Prev => self.prev(),
            tray::TrayCmd::SeekTo(t) => self.seek(t),
            tray::TrayCmd::Show => self.ctx.send_viewport_cmd(egui::ViewportCommand::Focus),
            tray::TrayCmd::Quit => {
                self.quitting = true;
                self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn picker_modal(&mut self, ctx: &egui::Context) {
        let Some(name) = self.picker.clone() else { return };
        let in_list: HashSet<String> = self.lib.playlists.get(&name).map(|v| v.iter().map(Track::key).collect()).unwrap_or_default();
        let mut seen = HashSet::new();
        let q = self.pick_query.to_lowercase();
        let pool: Vec<Track> = [&self.yt, &self.lib.favorites, &self.lib.recent, &self.local]
            .into_iter()
            .flatten()
            .filter(|t| !t.is_radio() && seen.insert(t.key()))
            .filter(|t| q.is_empty() || format!("{} {}", t.title, t.artist).to_lowercase().contains(&q))
            .take(300)
            .cloned()
            .collect();
        let mut add = None;
        let resp = egui::Modal::new(egui::Id::new("picker"))
            .frame(egui::Frame::new().fill(MENU).corner_radius(20).inner_margin(24).stroke(Stroke::new(1.0, HAIR)))
            .show(ctx, |ui| {
                ui.set_width(600.0);
                ui.label(RichText::new(format!("Add to {name}")).font(FontId::new(34.0, serif())).color(TEXT));
                ui.label(RichText::new("From your favorites, history, files and YouTube").color(TEXT2));
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    let r = ui.add(input(&mut self.pick_query, &format!("{}  Filter, or search YouTube", icon::MAGNIFYING_GLASS), 420.0, 15.0));
                    let go = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    if (ui.button(format!("{}  YouTube", icon::YOUTUBE_LOGO)).clicked() || go) && !self.pick_query.trim().is_empty() {
                        self.yt_query = self.pick_query.clone();
                        self.search_youtube();
                        self.pick_query.clear();
                    }
                    if self.busy.contains("yt") {
                        ui.spinner();
                    }
                });
                ui.add_space(12.0);
                egui::ScrollArea::vertical().max_height(440.0).auto_shrink([false, true]).show(ui, |ui| {
                    if pool.is_empty() {
                        ui.label(RichText::new("Nothing matches. Press Enter to search YouTube").color(MUTED));
                    }
                    for t in &pool {
                        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::click());
                        let added = in_list.contains(&t.key());
                        let h = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.15);
                        let p = ui.painter();
                        p.rect_filled(r, 10, white((12.0 * h) as u8));
                        art_img(p, &mut self.art, t, Rect::from_min_size(r.min + vec2(8.0, 8.0), Vec2::splat(36.0)), 6);
                        text(p, r.min + vec2(56.0, 8.0), &t.title, 14.0, TEXT, r.width() - 120.0, semibold());
                        text(p, r.min + vec2(56.0, 27.0), &t.artist, 12.0, TEXT2, r.width() - 120.0, FontFamily::Proportional);
                        let (ic, col) = if added { (icon::CHECK, self.aura.accent) } else { (icon::PLUS, lerp(TEXT2, TEXT, h)) };
                        p.text(r.right_center() - vec2(24.0, 0.0), egui::Align2::CENTER_CENTER, ic, FontId::proportional(18.0), col);
                        if resp.clicked() && !added {
                            add = Some(t.clone());
                        }
                    }
                });
                ui.add_space(16.0);
                ui.with_layout(egui::Layout::right_to_left(Align::Min), |ui| pill(ui, "Done", TEXT, INK).clicked()).inner
            });
        if let Some(t) = add {
            self.lib.playlists.entry(name).or_default().push(t);
            self.lib.save();
        }
        if resp.inner || resp.should_close() {
            self.picker = None;
        }
    }

    fn settings(&mut self, ui: &mut Ui) {
        self.header(ui, "Settings", "Make it yours");
        let before = self.lib.settings.clone();
        let accent = self.aura.accent;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.set_max_width(ui.available_width().min(880.0));
            group(ui, "Look");
            let s = &mut self.lib.settings;
            row(ui, "Background", "What fills the window behind everything", |ui| {
                segmented(ui, &mut s.background, &[(Background::Aura, "Aura"), (Background::BlurredCover, "Blurred cover"), (Background::Solid, "Solid")]);
            });
            row(ui, "Colors", "Follow each song's artwork, or keep one mood", |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (pal, label) in [
                        (Palette::Artwork, "Artwork"),
                        (Palette::Midnight, "Midnight"),
                        (Palette::Ember, "Ember"),
                        (Palette::Forest, "Forest"),
                        (Palette::Rose, "Rose"),
                        (Palette::Mono, "Mono"),
                    ] {
                        let dot = if pal == Palette::Artwork { accent } else { viz::palette(pal).1 };
                        if swatch(ui, label, dot, s.palette == pal) {
                            s.palette = pal;
                        }
                    }
                });
            });
            row(ui, "Brightness", "How strongly the background glows", |ui| {
                if let Some(v) = slider(ui, 220.0, (s.brightness - 0.2) / 0.8, true, accent) {
                    s.brightness = 0.2 + v * 0.8;
                }
            });
            row(ui, "Moving background", "Let the colors drift slowly", |ui| toggle(ui, &mut s.motion));
            row(ui, "React to the beat", "Background and cover pulse with the music", |ui| toggle(ui, &mut s.beat));
            row(ui, "Interface size", "Scale everything up or down", |ui| {
                segmented(ui, &mut s.ui_scale, &[(0.9, "90%"), (1.0, "100%"), (1.1, "110%"), (1.25, "125%")]);
            });

            group(ui, "Playback");
            let tray = self.tray.is_some();
            let s = &mut self.lib.settings;
            let desc = if tray { "Closing the window keeps the music going. Quit from the tray" } else { "No tray found on your desktop, so closing quits" };
            row(ui, "Close to tray", desc, |ui| toggle(ui, &mut s.close_to_tray));
            let mut rescan = false;
            row(ui, "Music folder", "Where Your files looks for songs", |ui| {
                let r = ui.add(input(&mut self.lib.music_dir, &format!("{}  Folder", icon::FOLDER_SIMPLE), 260.0, 14.0));
                rescan = ui.button("Rescan").clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)));
            });
            if rescan {
                self.lib.save();
                self.scan();
                self.toast("Rescanning your music folder");
            }

            group(ui, "Updates");
            let accent = self.aura.accent;
            let mut action = None;
            let desc = match &self.update {
                Update::Idle => format!("You're on {}", updater::VERSION),
                Update::Checking => "Checking GitHub for a new release".into(),
                Update::UpToDate => format!("{} is the latest version", updater::VERSION),
                Update::Available(r) => format!("{} is out. You're on {}", r.version, updater::VERSION),
                Update::Installing(v) => format!("Downloading {v}"),
                Update::Ready(v) => format!("{v} is installed. Restart to start using it"),
                Update::Failed(e) => e.clone(),
            };
            row(ui, "Sonavelt", &desc, |ui| match &self.update {
                Update::Checking | Update::Installing(_) => {
                    ui.spinner();
                }
                Update::Available(r) => {
                    if pill(ui, &format!("{}  Update to {}", icon::DOWNLOAD_SIMPLE, r.version), accent, INK).clicked() {
                        action = Some(1);
                    }
                }
                Update::Ready(_) => {
                    if pill(ui, &format!("{}  Restart now", icon::ARROW_CLOCKWISE), accent, INK).clicked() {
                        action = Some(2);
                    }
                }
                _ => {
                    if ui.button("Check for updates").clicked() {
                        action = Some(0);
                    }
                }
            });
            match action {
                Some(0) => self.check_update(false),
                Some(1) => self.install_update(),
                Some(2) => self.restart(ui.ctx()),
                _ => {}
            }

            group(ui, "Storage");
            let dir = dirs::cache_dir().unwrap_or_default().join("sonavelt").join("youtube");
            let size = *self.cache_size.get_or_insert_with(|| {
                walkdir::WalkDir::new(&dir).into_iter().filter_map(Result::ok).filter_map(|e| e.metadata().ok()).filter(|m| m.is_file()).map(|m| m.len()).sum()
            });
            let mut clear = false;
            row(ui, "YouTube downloads", &format!("{:.0} MB of songs saved for instant replay", size as f64 / 1e6), |ui| {
                clear = ui.button("Clear").clicked();
            });
            if clear {
                let _ = std::fs::remove_dir_all(&dir);
                self.cache_size = None;
                self.toast("Cleared YouTube downloads");
            }

            group(ui, "Shortcuts");
            for (keys, what) in [
                ("Space", "Play or pause"),
                ("← →", "Back or forward 5 seconds"),
                ("↑ ↓", "Volume"),
                ("N  P", "Next, previous"),
                ("L", "Lyrics"),
                ("F", "Favorite the current song"),
            ] {
                ui.horizontal(|ui| {
                    ui.add_sized([120.0, 28.0], egui::Label::new(RichText::new(keys).font(FontId::monospace(12.0)).color(TEXT)).halign(Align::Min));
                    ui.label(RichText::new(what).color(TEXT2));
                });
            }
            ui.add_space(40.0);
        });
        if self.lib.settings != before {
            if self.lib.settings.ui_scale != before.ui_scale {
                ui.ctx().set_zoom_factor(self.lib.settings.ui_scale);
            }
            if !ui.input(|i| i.pointer.any_down()) {
                self.lib.save();
            }
        }
        if ui.input(|i| i.pointer.any_released()) {
            self.lib.save();
        }
    }
}

fn group(ui: &mut Ui, title: &str) {
    ui.add_space(24.0);
    ui.label(RichText::new(title).font(FontId::new(30.0, serif())).color(TEXT));
    ui.add_space(4.0);
}

/// One settings line: title and description on the left, the control on the right.
fn row(ui: &mut Ui, title: &str, desc: &str, control: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        // fixed-width label column so every control starts on the same line
        let label_w = (ui.available_width() * 0.42).clamp(220.0, 360.0);
        ui.allocate_ui_with_layout(vec2(label_w, 56.0), egui::Layout::top_down(Align::Min), |ui| {
            ui.set_min_width(label_w);
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add_space(8.0);
            ui.add(egui::Label::new(RichText::new(title).font(FontId::new(15.0, semibold())).color(TEXT)).truncate());
            ui.add(egui::Label::new(RichText::new(desc).size(13.0).color(TEXT2)).wrap());
        });
        ui.add_space(16.0);
        let w = ui.available_width();
        ui.vertical(|ui| {
            ui.set_max_width(w);
            ui.add_space(12.0);
            control(ui);
        });
    });
    let r = ui.min_rect();
    ui.painter().hline(r.x_range(), ui.cursor().top() + 4.0, Stroke::new(1.0, HAIR));
    ui.add_space(8.0);
}

fn segmented<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) {
    let font = FontId::proportional(14.0);
    let widths: Vec<f32> = options.iter().map(|(_, l)| ui.painter().layout_no_wrap(l.to_string(), font.clone(), TEXT).size().x + 28.0).collect();
    let (r, _) = ui.allocate_exact_size(vec2(widths.iter().sum::<f32>() + 8.0, 40.0), Sense::hover());
    ui.painter().rect_filled(r, 20, white(18));
    let mut x = r.left() + 4.0;
    for ((v, label), w) in options.iter().zip(widths) {
        let cell = Rect::from_min_size(pos2(x, r.top() + 4.0), vec2(w, 32.0));
        let resp = ui.interact(cell, ui.id().with(label), Sense::click());
        let selected = *v == *value;
        let sel = ui.ctx().animate_bool_with_time(resp.id, selected, 0.18);
        let p = ui.painter();
        if sel > 0.0 {
            p.rect_filled(cell, 16, TEXT.gamma_multiply(sel));
        } else if resp.hovered() {
            p.rect_filled(cell, 16, white(14));
        }
        p.text(cell.center(), egui::Align2::CENTER_CENTER, *label, font.clone(), lerp(TEXT, INK, sel));
        if resp.clicked() {
            *value = *v;
        }
        x += w;
    }
}

fn swatch(ui: &mut Ui, label: &str, dot: Color32, selected: bool) -> bool {
    let font = FontId::proportional(14.0);
    let w = ui.painter().layout_no_wrap(label.to_string(), font.clone(), TEXT).size().x + 48.0;
    let (r, resp) = ui.allocate_exact_size(vec2(w, 36.0), Sense::click());
    let h = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.15);
    let p = ui.painter();
    p.rect_filled(r, 18, white((18.0 + 12.0 * h) as u8));
    if selected {
        p.rect_stroke(r, 18, Stroke::new(1.5, TEXT), egui::StrokeKind::Inside);
    }
    p.circle_filled(r.left_center() + vec2(18.0, 0.0), 6.0, dot);
    p.text(r.left_center() + vec2(32.0, 0.0), egui::Align2::LEFT_CENTER, label, font, if selected { TEXT } else { TEXT2 });
    resp.clicked()
}

fn toggle(ui: &mut Ui, on: &mut bool) {
    let (r, resp) = ui.allocate_exact_size(vec2(48.0, 28.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
    }
    let t = ui.ctx().animate_bool_with_time(resp.id, *on, 0.18);
    let p = ui.painter();
    p.rect_filled(r, 14, lerp(white(40), TEXT, t));
    let knob = pos2(r.left() + 14.0 + t * 20.0, r.center().y);
    p.circle_filled(knob, 10.0, lerp(TEXT, INK, t));
}
