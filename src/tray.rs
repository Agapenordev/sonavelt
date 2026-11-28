//! System tray icon (freedesktop StatusNotifierItem) with playback controls.

use eframe::egui;
#[cfg(target_os = "linux")]
use ksni::blocking::TrayMethods;
use std::sync::mpsc::Sender;

// on Windows nothing sends these yet: no tray or media keys there
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub enum TrayCmd {
    Toggle,
    Next,
    Prev,
    Show,
    Quit,
    /// Jump to this many seconds (from media controls).
    SeekTo(f64),
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub struct RwTray {
    tx: Sender<TrayCmd>,
    pub ctx: egui::Context,
    pub now: String,
    pub playing: bool,
}

#[cfg(target_os = "linux")]
impl RwTray {
    fn send(&self, c: TrayCmd) {
        let _ = self.tx.send(c);
        self.ctx.request_repaint();
    }
}

#[cfg(target_os = "linux")]
pub type Handle = ksni::blocking::Handle<RwTray>;

// shortcut: no tray on Windows yet (Shell_NotifyIcon); closing the window quits there
#[cfg(not(target_os = "linux"))]
pub struct Handle;

#[cfg(not(target_os = "linux"))]
impl Handle {
    pub fn update<R>(&self, _: impl FnOnce(&mut RwTray) -> R) -> Option<R> {
        None
    }
}

/// Start the tray; `None` when no tray host is running (no status bar with a tray).
#[cfg(target_os = "linux")]
pub fn spawn(tx: Sender<TrayCmd>, ctx: egui::Context) -> Option<Handle> {
    RwTray { tx, ctx, now: "Nothing playing".into(), playing: false }.spawn().map_err(|e| eprintln!("tray unavailable: {e}")).ok()
}

#[cfg(not(target_os = "linux"))]
pub fn spawn(_: Sender<TrayCmd>, _: egui::Context) -> Option<Handle> {
    None
}

#[cfg(target_os = "linux")]
impl ksni::Tray for RwTray {
    fn id(&self) -> String {
        "sonavelt".into()
    }
    fn title(&self) -> String {
        "Sonavelt".into()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        [22, 32, 48, 64].map(|n| ksni::Icon { width: n as i32, height: n as i32, data: crate::logo::argb(n) }).into()
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip { title: "Sonavelt".into(), description: self.now.clone(), ..Default::default() }
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayCmd::Show);
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        let item = |label: &str, cmd: fn() -> TrayCmd| -> ksni::MenuItem<Self> {
            StandardItem { label: label.into(), activate: Box::new(move |t: &mut Self| t.send(cmd())), ..Default::default() }.into()
        };
        vec![
            StandardItem { label: self.now.clone(), enabled: false, ..Default::default() }.into(),
            ksni::MenuItem::Separator,
            item(if self.playing { "Pause" } else { "Play" }, || TrayCmd::Toggle),
            item("Next", || TrayCmd::Next),
            item("Previous", || TrayCmd::Prev),
            ksni::MenuItem::Separator,
            item("Show Sonavelt", || TrayCmd::Show),
            item("Quit", || TrayCmd::Quit),
        ]
    }
}
