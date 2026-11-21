//! Aura theme: near-black ink under a color field taken from the current artwork,
//! white text at a few opacities, and an accent picked from the cover.

use eframe::egui::{Color32, FontFamily};

pub const INK: Color32 = Color32::from_rgb(9, 9, 11);
pub const MENU: Color32 = Color32::from_rgb(24, 24, 28);
pub const FALLBACK_ACCENT: Color32 = Color32::from_rgb(236, 226, 208);

pub const fn white(a: u8) -> Color32 {
    Color32::from_rgba_premultiplied(a, a, a, a)
}
pub const fn black(a: u8) -> Color32 {
    Color32::from_rgba_premultiplied(0, 0, 0, a)
}

pub const TEXT: Color32 = white(242);
pub const TEXT2: Color32 = white(165);
pub const MUTED: Color32 = white(110);
pub const HAIR: Color32 = white(22);
pub const SELECTED: Color32 = white(26);

pub const HEART_RED: Color32 = Color32::from_rgb(255, 59, 92);

pub fn filled() -> FontFamily {
    FontFamily::Name("filled".into())
}
pub fn serif() -> FontFamily {
    FontFamily::Name("serif".into())
}
pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}
