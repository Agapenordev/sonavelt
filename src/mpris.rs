//! MPRIS over D-Bus: media keys, lock screens and desktop widgets control Sonavelt and show
//! what's playing. Commands arrive on the same channel as the tray's.

/// What the desktop should show. Compared each frame; a change notifies listeners.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Clone, Default, PartialEq)]
pub struct Now {
    /// "Playing", "Paused" or "Stopped"
    pub status: &'static str,
    pub id: u64,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub art: Option<String>,
    pub length: Option<f64>,
    pub volume: f64,
}

#[cfg(target_os = "linux")]
pub use dbus::Mpris;

#[cfg(target_os = "linux")]
mod dbus {
    use super::Now;
    use crate::tray::TrayCmd;
    use eframe::egui;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI64, Ordering::Relaxed};
    use std::sync::mpsc::Sender;
    use zbus::zvariant::{ObjectPath, OwnedValue, Value};

    const PATH: &str = "/org/mpris/MediaPlayer2";

    pub struct Mpris {
        conn: zbus::blocking::Connection,
        last: Now,
        position_us: Arc<AtomicI64>,
    }

    struct Root {
        tx: Sender<TrayCmd>,
        ctx: egui::Context,
    }

    struct Player {
        tx: Sender<TrayCmd>,
        ctx: egui::Context,
        now: Now,
        position_us: Arc<AtomicI64>,
    }

    fn send(tx: &Sender<TrayCmd>, ctx: &egui::Context, c: TrayCmd) {
        let _ = tx.send(c);
        ctx.request_repaint();
    }

    #[zbus::interface(name = "org.mpris.MediaPlayer2")]
    impl Root {
        fn raise(&self) {
            send(&self.tx, &self.ctx, TrayCmd::Show);
        }
        fn quit(&self) {
            send(&self.tx, &self.ctx, TrayCmd::Quit);
        }
        #[zbus(property)]
        fn can_quit(&self) -> bool {
            true
        }
        #[zbus(property)]
        fn can_raise(&self) -> bool {
            true
        }
        #[zbus(property)]
        fn has_track_list(&self) -> bool {
            false
        }
        #[zbus(property)]
        fn identity(&self) -> String {
            "Sonavelt".into()
        }
        #[zbus(property)]
        fn desktop_entry(&self) -> String {
            "sonavelt".into()
        }
        #[zbus(property)]
        fn supported_uri_schemes(&self) -> Vec<String> {
            vec![]
        }
        #[zbus(property)]
        fn supported_mime_types(&self) -> Vec<String> {
            vec![]
        }
    }

    #[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
    impl Player {
        fn play_pause(&self) {
            send(&self.tx, &self.ctx, TrayCmd::Toggle);
        }
        fn play(&self) {
            if self.now.status != "Playing" {
                send(&self.tx, &self.ctx, TrayCmd::Toggle);
            }
        }
        fn pause(&self) {
            if self.now.status == "Playing" {
                send(&self.tx, &self.ctx, TrayCmd::Toggle);
            }
        }
        // shortcut: Stop pauses rather than unloading the song, which is what people expect from a key press
        fn stop(&self) {
            self.pause();
        }
        fn next(&self) {
            send(&self.tx, &self.ctx, TrayCmd::Next);
        }
        fn previous(&self) {
            send(&self.tx, &self.ctx, TrayCmd::Prev);
        }
        fn seek(&self, offset_us: i64) {
            let to = self.position_us.load(Relaxed) + offset_us;
            send(&self.tx, &self.ctx, TrayCmd::SeekTo(to.max(0) as f64 / 1e6));
        }
        fn set_position(&self, track: ObjectPath<'_>, position_us: i64) {
            if track.as_str() == track_path(self.now.id).as_str() {
                send(&self.tx, &self.ctx, TrayCmd::SeekTo(position_us.max(0) as f64 / 1e6));
            }
        }
        fn open_uri(&self, _uri: String) {}

        #[zbus(property)]
        fn playback_status(&self) -> String {
            self.now.status.into()
        }
        #[zbus(property)]
        fn rate(&self) -> f64 {
            1.0
        }
        #[zbus(property)]
        fn minimum_rate(&self) -> f64 {
            1.0
        }
        #[zbus(property)]
        fn maximum_rate(&self) -> f64 {
            1.0
        }
        #[zbus(property)]
        fn metadata(&self) -> HashMap<String, OwnedValue> {
            metadata(&self.now)
        }
        // shortcut: read-only; widgets that set the volume get an error. Add a setter if anyone asks
        #[zbus(property)]
        fn volume(&self) -> f64 {
            self.now.volume
        }
        #[zbus(property(emits_changed_signal = "false"))]
        fn position(&self) -> i64 {
            self.position_us.load(Relaxed)
        }
        #[zbus(property)]
        fn can_go_next(&self) -> bool {
            true
        }
        #[zbus(property)]
        fn can_go_previous(&self) -> bool {
            true
        }
        #[zbus(property)]
        fn can_play(&self) -> bool {
            self.now.status != "Stopped"
        }
        #[zbus(property)]
        fn can_pause(&self) -> bool {
            self.now.status != "Stopped"
        }
        #[zbus(property)]
        fn can_seek(&self) -> bool {
            self.now.length.is_some()
        }
        #[zbus(property)]
        fn can_control(&self) -> bool {
            true
        }
    }

    fn track_path(id: u64) -> ObjectPath<'static> {
        ObjectPath::try_from(format!("/org/sonavelt/track/{id}")).expect("valid object path")
    }

    fn owned<'a>(v: impl Into<Value<'a>>) -> OwnedValue {
        OwnedValue::try_from(v.into()).expect("plain values never hold file descriptors")
    }

    fn metadata(n: &Now) -> HashMap<String, OwnedValue> {
        let mut m = HashMap::new();
        if n.status == "Stopped" {
            m.insert("mpris:trackid".into(), owned(ObjectPath::from_static_str_unchecked("/org/mpris/MediaPlayer2/TrackList/NoTrack")));
            return m;
        }
        m.insert("mpris:trackid".into(), owned(track_path(n.id)));
        m.insert("xesam:title".into(), owned(n.title.as_str()));
        m.insert("xesam:artist".into(), owned(vec![n.artist.as_str()]));
        if !n.album.is_empty() {
            m.insert("xesam:album".into(), owned(n.album.as_str()));
        }
        if let Some(art) = &n.art {
            m.insert("mpris:artUrl".into(), owned(art.as_str()));
        }
        if let Some(len) = n.length {
            m.insert("mpris:length".into(), owned((len * 1e6) as i64));
        }
        m
    }

    impl Mpris {
        /// Register on the session bus; `None` if there is no bus or another Sonavelt holds the name.
        pub fn spawn(tx: Sender<TrayCmd>, ctx: egui::Context) -> Option<Self> {
            let position_us = Arc::new(AtomicI64::new(0));
            let player = Player { tx: tx.clone(), ctx: ctx.clone(), now: Now { status: "Stopped", ..Default::default() }, position_us: position_us.clone() };
            let conn = zbus::blocking::connection::Builder::session()
                .and_then(|b| b.name("org.mpris.MediaPlayer2.sonavelt"))
                .and_then(|b| b.serve_at(PATH, Root { tx, ctx }))
                .and_then(|b| b.serve_at(PATH, player))
                .and_then(|b| b.build())
                .map_err(|e| eprintln!("media controls unavailable: {e}"))
                .ok()?;
            Some(Self { conn, last: Now { status: "Stopped", ..Default::default() }, position_us })
        }

        /// A new window means a new egui context to wake when a command arrives.
        pub fn set_ctx(&self, ctx: &egui::Context) {
            let server = self.conn.object_server();
            if let Ok(root) = server.interface::<_, Root>(PATH) {
                root.get_mut().ctx = ctx.clone();
            }
            if let Ok(player) = server.interface::<_, Player>(PATH) {
                player.get_mut().ctx = ctx.clone();
            }
        }

        pub fn update(&mut self, now: Now, position: f64) {
            self.position_us.store((position * 1e6) as i64, Relaxed);
            if now == self.last {
                return;
            }
            if let Ok(player) = self.conn.object_server().interface::<_, Player>(PATH) {
                player.get_mut().now = now.clone();
            }
            let changed: HashMap<&str, OwnedValue> = HashMap::from([
                ("PlaybackStatus", owned(now.status)),
                ("Metadata", owned(Value::from(metadata(&now)))),
                ("Volume", owned(now.volume)),
                ("CanSeek", owned(now.length.is_some())),
                ("CanPlay", owned(now.status != "Stopped")),
                ("CanPause", owned(now.status != "Stopped")),
            ]);
            let _ = self.conn.emit_signal(
                None::<()>,
                PATH,
                "org.freedesktop.DBus.Properties",
                "PropertiesChanged",
                &("org.mpris.MediaPlayer2.Player", changed, Vec::<&str>::new()),
            );
            self.last = now;
        }
    }
}

// shortcut: no media keys on Windows yet (System Media Transport Controls); add them with windows-rs
#[cfg(not(target_os = "linux"))]
pub struct Mpris;

#[cfg(not(target_os = "linux"))]
impl Mpris {
    pub fn spawn(_: std::sync::mpsc::Sender<crate::tray::TrayCmd>, _: eframe::egui::Context) -> Option<Self> {
        None
    }
    pub fn set_ctx(&self, _: &eframe::egui::Context) {}
    pub fn update(&mut self, _: Now, _: f64) {}
}
