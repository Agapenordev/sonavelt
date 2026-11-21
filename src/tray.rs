//! System tray icon (freedesktop StatusNotifierItem) with playback controls.

use eframe::egui;
use ksni::blocking::TrayMethods;
use std::sync::mpsc::Sender;

pub enum TrayCmd {
    Toggle,
    Next,
    Prev,
    Show,
    Quit,
}

pub struct RwTray {
    tx: Sender<TrayCmd>,
    pub ctx: egui::Context,
    pub now: String,
    pub playing: bool,
}

impl RwTray {
    fn send(&self, c: TrayCmd) {
        let _ = self.tx.send(c);
        self.ctx.request_repaint();
    }
}

pub type Handle = ksni::blocking::Handle<RwTray>;

/// Start the tray; `None` when no tray host is running (no status bar with a tray).
pub fn spawn(tx: Sender<TrayCmd>, ctx: egui::Context) -> Option<Handle> {
    RwTray { tx, ctx, now: "Nothing playing".into(), playing: false }.spawn().map_err(|e| eprintln!("tray unavailable: {e}")).ok()
}

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
