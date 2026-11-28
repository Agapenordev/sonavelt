//! System tray icon with playback controls: freedesktop StatusNotifierItem on Linux, the
//! notification area on Windows.

use eframe::egui;
#[cfg(target_os = "linux")]
use ksni::blocking::TrayMethods;
use std::sync::mpsc::Sender;

// SeekTo only comes from media controls, which are Linux-only for now
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

#[cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))]
pub struct RwTray {
    tx: Sender<TrayCmd>,
    pub ctx: egui::Context,
    pub now: String,
    pub playing: bool,
}

#[cfg(any(target_os = "linux", windows))]
impl RwTray {
    fn send(&self, c: TrayCmd) {
        let _ = self.tx.send(c);
        self.ctx.request_repaint();
    }
}

#[cfg(target_os = "linux")]
pub type Handle = ksni::blocking::Handle<RwTray>;

/// The Windows tray runs on its own thread and picks up changes from here.
#[cfg(windows)]
pub struct Handle(std::sync::Arc<std::sync::Mutex<RwTray>>);

#[cfg(windows)]
impl Handle {
    pub fn update<R>(&self, f: impl FnOnce(&mut RwTray) -> R) -> Option<R> {
        self.0.lock().ok().map(|mut t| f(&mut t))
    }
}

// shortcut: no tray on macOS; closing the window quits there
#[cfg(not(any(target_os = "linux", windows)))]
pub struct Handle;

#[cfg(not(any(target_os = "linux", windows)))]
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

#[cfg(windows)]
pub fn spawn(tx: Sender<TrayCmd>, ctx: egui::Context) -> Option<Handle> {
    let state = std::sync::Arc::new(std::sync::Mutex::new(RwTray { tx, ctx, now: "Nothing playing".into(), playing: false }));
    let (ready_tx, ready) = std::sync::mpsc::channel();
    let shared = state.clone();
    std::thread::spawn(move || windows_tray::run(shared, ready_tx));
    ready.recv().unwrap_or(false).then_some(Handle(state))
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn spawn(_: Sender<TrayCmd>, _: egui::Context) -> Option<Handle> {
    None
}

#[cfg(windows)]
mod windows_tray {
    use super::{RwTray, TrayCmd};
    use std::sync::{Arc, Mutex, mpsc::Sender};
    use std::time::Duration;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    use windows_sys::Win32::UI::WindowsAndMessaging::{DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage};

    /// Windows delivers the tray's clicks as window messages to the thread that made it, so this
    /// thread owns the icon and pumps its messages, like ksni's thread on Linux.
    pub fn run(state: Arc<Mutex<RwTray>>, ready: Sender<bool>) {
        let now = MenuItem::new("Nothing playing", false, None);
        let toggle = MenuItem::new("Play", true, None);
        let next = MenuItem::new("Next", true, None);
        let prev = MenuItem::new("Previous", true, None);
        let show = MenuItem::new("Show Sonavelt", true, None);
        let quit = MenuItem::new("Quit", true, None);
        let menu = Menu::new();
        let tray = menu
            .append_items(&[&now, &PredefinedMenuItem::separator(), &toggle, &next, &prev, &PredefinedMenuItem::separator(), &show, &quit])
            .ok()
            .and_then(|_| tray_icon::Icon::from_rgba(crate::logo::rgba(32), 32, 32).ok())
            .and_then(|icon| {
                TrayIconBuilder::new()
                    .with_menu(Box::new(menu))
                    .with_icon(icon)
                    .with_tooltip("Sonavelt")
                    .build()
                    .map_err(|e| eprintln!("tray unavailable: {e}"))
                    .ok()
            });
        let _ = ready.send(tray.is_some());
        let Some(tray) = tray else { return };
        let send = |c: TrayCmd| {
            if let Ok(t) = state.lock() {
                t.send(c);
            }
        };
        let mut shown = (String::new(), false);
        loop {
            // SAFETY: a zeroed MSG is a valid out-parameter, and these calls only touch it
            unsafe {
                let mut msg: MSG = std::mem::zeroed();
                while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            while let Ok(e) = MenuEvent::receiver().try_recv() {
                let cmd = [(&toggle, TrayCmd::Toggle), (&next, TrayCmd::Next), (&prev, TrayCmd::Prev), (&show, TrayCmd::Show), (&quit, TrayCmd::Quit)]
                    .into_iter()
                    .find(|(item, _)| *item.id() == e.id);
                if let Some((_, c)) = cmd {
                    send(c);
                }
            }
            while let Ok(e) = TrayIconEvent::receiver().try_recv() {
                if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                    send(TrayCmd::Show);
                }
            }
            let current = state.lock().map(|t| (t.now.clone(), t.playing)).unwrap_or_default();
            if current != shown {
                now.set_text(&current.0);
                toggle.set_text(if current.1 { "Pause" } else { "Play" });
                // Windows cuts tray tooltips off at 128 characters
                let tip: String = format!("Sonavelt\n{}", current.0).chars().take(120).collect();
                let _ = tray.set_tooltip(Some(tip));
                shown = current;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
    }
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
