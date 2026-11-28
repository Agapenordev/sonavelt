//! Backgrounds (aura color field, blurred cover), theme palettes,
//! the small "playing" equalizer and fallback covers.

use crate::library::Palette;
use crate::theme::*;
use eframe::egui::{self, Color32, FontId, Painter, Pos2, Rect, Rgba, pos2, vec2};

const N: usize = 18; // mesh resolution

/// 16-cell color grid and accent for a palette preset.
pub fn palette(p: Palette) -> ([Color32; 16], Color32) {
    let (h1, h2, s, accent) = match p {
        Palette::Artwork | Palette::Midnight => (0.64, 0.75, 0.55, Color32::from_rgb(150, 180, 255)),
        Palette::Ember => (0.03, 0.08, 0.6, Color32::from_rgb(255, 178, 122)),
        Palette::Forest => (0.38, 0.47, 0.5, Color32::from_rgb(155, 227, 176)),
        Palette::Rose => (0.92, 0.83, 0.5, Color32::from_rgb(255, 158, 196)),
        Palette::Mono => (0.0, 0.0, 0.0, Color32::from_rgb(232, 232, 232)),
    };
    let grid = std::array::from_fn(|i| {
        let k = ((i * 7 + 3) % 16) as f32 / 16.0; // scrambled, so the field isn't a plain gradient
        let h = if i % 3 == 0 { h2 } else { h1 };
        egui::ecolor::Hsva::new(h, s * (0.6 + 0.4 * k), 0.14 + 0.12 * k, 1.0).into()
    });
    (grid, accent)
}

pub struct Aura {
    cur: [Rgba; 16],
    pub accent: Color32,
}

impl Aura {
    pub fn new() -> Self {
        let (grid, accent) = palette(Palette::Midnight);
        Self { cur: grid.map(Rgba::from), accent }
    }

    /// Ease toward new colors so track and theme changes cross-fade.
    pub fn update(&mut self, grid: &[Color32; 16], accent: Color32, dt: f32) {
        let k = 1.0 - (-dt * 1.8).exp();
        for (c, g) in self.cur.iter_mut().zip(grid) {
            *c = *c * (1.0 - k) + Rgba::from(*g) * k;
        }
        self.accent = (Rgba::from(self.accent) * (1.0 - k) + Rgba::from(accent) * k).into();
    }

    /// About the color along the top edge, where the window's title bar sits.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn top(&self, gain: f32) -> Color32 {
        let c = (self.cur[0] + self.cur[1] + self.cur[2] + self.cur[3]) * (gain / 4.0);
        Rgba::from_rgb(c.r(), c.g(), c.b()).into()
    }

    /// `t` drives the drift (pass a constant to freeze it), `gain` is the overall brightness.
    pub fn paint(&self, p: &Painter, r: Rect, t: f32, gain: f32) {
        let sample = |u: f32, v: f32| -> Rgba {
            let (x, y) = ((u * 3.0).clamp(0.0, 3.0), (v * 3.0).clamp(0.0, 3.0));
            let (x0, y0) = ((x.floor() as usize).min(2), (y.floor() as usize).min(2));
            let s = |f: f32| f * f * (3.0 - 2.0 * f);
            let (fx, fy) = (s(x - x0 as f32), s(y - y0 as f32));
            let g = |xx: usize, yy: usize| self.cur[yy * 4 + xx];
            let top = g(x0, y0) * (1.0 - fx) + g(x0 + 1, y0) * fx;
            let bot = g(x0, y0 + 1) * (1.0 - fx) + g(x0 + 1, y0 + 1) * fx;
            top * (1.0 - fy) + bot * fy
        };
        let mut mesh = egui::Mesh::default();
        for j in 0..=N {
            for i in 0..=N {
                let (u, v) = (i as f32 / N as f32, j as f32 / N as f32);
                let su = u + 0.08 * (t * 0.07 + v * 3.1).sin() + 0.04 * (t * 0.11 + u * 5.0).cos();
                let sv = v + 0.08 * (t * 0.05 + u * 2.7).cos();
                let shade = 1.0 - 0.55 * v.powf(1.6); // darker toward the dock
                let c = sample(su, sv) * (gain * shade);
                mesh.colored_vertex(pos2(r.left() + u * r.width(), r.top() + v * r.height()), Rgba::from_rgb(c.r(), c.g(), c.b()).into());
            }
        }
        grid_triangles(&mut mesh, N as u32);
        p.rect_filled(r, 0, INK);
        p.add(mesh);
    }
}

fn grid_triangles(mesh: &mut egui::Mesh, n: u32) {
    for j in 0..n {
        for i in 0..n {
            let a = j * (n + 1) + i;
            mesh.add_triangle(a, a + 1, a + n + 2);
            mesh.add_triangle(a, a + n + 2, a + n + 1);
        }
    }
}

/// The artwork stretched from a tiny texture: bilinear filtering does the blur for free.
pub fn blurred_cover(p: &Painter, r: Rect, tex: &egui::TextureHandle, t: f32, gain: f32) {
    let drift = vec2((t * 0.05).sin(), (t * 0.04).cos()) * r.width() * 0.03;
    let side = r.width().max(r.height()) * 1.25;
    let img = Rect::from_center_size(r.center() + drift, vec2(side, side));
    p.rect_filled(r, 0, INK);
    p.image(tex.id(), img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    let dim = ((1.0 - gain * 0.75).clamp(0.0, 1.0) * 255.0) as u8;
    p.rect_filled(r, 0, black(dim));
    // same bottom shade as the aura, so the dock always sits on dark
    let mut mesh = egui::Mesh::default();
    for (pos, a) in [(r.left_top(), 0), (r.right_top(), 0), (r.right_bottom(), 170), (r.left_bottom(), 170)] {
        mesh.colored_vertex(pos, black(a));
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.add(mesh);
}

/// Three bouncing bars marking the playing row.
pub fn eq(p: &Painter, center: Pos2, t: f32, level: f32, playing: bool, color: Color32) {
    for i in 0..3 {
        let f = i as f32;
        let h = if playing { 4.0 + 10.0 * (0.35 + 0.65 * level) * ((t * (6.0 + f * 1.9) + f * 2.0).sin().abs()) } else { 4.0 };
        let x = center.x - 5.0 + f * 5.0;
        p.rect_filled(Rect::from_min_max(pos2(x - 1.5, center.y + 7.0 - h), pos2(x + 1.5, center.y + 7.0)), 1, color);
    }
}

/// Cover used before artwork arrives or when there is none.
pub fn fallback_cover(p: &Painter, r: Rect, key: &str, title: &str, radius: u8) {
    let h = key.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    let hue = (h % 360) as f32 / 360.0;
    let c = |s: f32, v: f32| -> Color32 { egui::ecolor::Hsva::new(hue, s, v, 1.0).into() };
    p.rect_filled(r, radius, c(0.35, 0.22));
    p.rect_filled(r.shrink(r.width() * 0.18), r.width() * 0.4, c(0.4, 0.32));
    let initial = title.chars().find(|c| c.is_alphanumeric()).unwrap_or('#').to_uppercase().to_string();
    p.text(r.center() + vec2(0.0, r.height() * 0.02), egui::Align2::CENTER_CENTER, initial, FontId::new(r.height() * 0.46, serif()), white(200));
}
