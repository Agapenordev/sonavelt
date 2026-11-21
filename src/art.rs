//! Artwork: YouTube thumbnails, cover art embedded in local files, radio station logos.
//! A worker pool fetches, decodes and keeps a small disk cache; the UI thread turns results
//! into textures and a 4x4 color grid that drives the aura background. Thumbnails are small
//! and least-recently-seen ones are dropped, so memory stays flat however much you browse.

use crate::library::{Source, Track};
use eframe::egui::{self, Color32, Rect, TextureHandle, pos2};
use std::collections::HashMap;
use std::process::Command;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

pub struct Art {
    pub tex: TextureHandle,
    /// Tiny source image (most favicons): draw it centered on a tile instead of stretched.
    pub small: bool,
    /// 12x12 copy; stretched with bilinear filtering it reads as a heavy blur.
    pub blur: TextureHandle,
    pub aspect: f32,
    pub grid: [Color32; 16],
    pub accent: Color32,
}

enum Slot {
    Loading,
    Ready(Art, u64),
    Failed,
}

type Job = (String, Track, u32);
type Done = (String, Option<image::RgbaImage>);

const THUMB: u32 = 256;
const HI: u32 = 640;
const MAX_TEXTURES: usize = 160;
/// Bump to invalidate every cached image (older versions cached wrong fallback logos).
const ART_DIR: &str = "art-v3";

pub struct ArtCache {
    map: HashMap<String, Slot>,
    jobs: Sender<Job>,
    done: Receiver<Done>,
    frame: u64,
}

impl ArtCache {
    pub fn new(ctx: &egui::Context) -> Self {
        let (jobs, job_rx) = channel::<Job>();
        let (done_tx, done) = channel::<Done>();
        let job_rx = Arc::new(Mutex::new(job_rx));
        for old in ["art", "art-v2"] {
            let _ = std::fs::remove_dir_all(dirs::cache_dir().unwrap_or_default().join("sonavelt").join(old));
        }
        // network-bound, so more workers than cores: one slow logo server can't stall the grid
        for _ in 0..12 {
            let (rx, tx, ctx) = (job_rx.clone(), done_tx.clone(), ctx.clone());
            std::thread::Builder::new()
                .stack_size(512 * 1024)
                .spawn(move || {
                    loop {
                        let Ok((key, track, size)) = rx.lock().unwrap().recv() else { return };
                        let _ = tx.send((key.clone(), load(&key, &track, size)));
                        ctx.request_repaint();
                    }
                })
                .expect("spawn art worker");
        }
        Self { map: HashMap::new(), jobs, done, frame: 0 }
    }

    /// Thumbnail artwork for a track, queuing a fetch the first time it's asked for.
    pub fn get(&mut self, t: &Track) -> Option<&Art> {
        self.get_sized(t, t.key(), THUMB)
    }

    /// Sharp artwork for big displays; the thumbnail stands in until it arrives.
    pub fn get_hi(&mut self, t: &Track) -> Option<&Art> {
        if matches!(self.map.get(&format!("{}#hi", t.key())), Some(Slot::Ready(..))) {
            return self.get_sized(t, format!("{}#hi", t.key()), HI);
        }
        self.get_sized(t, format!("{}#hi", t.key()), HI);
        self.get(t)
    }

    fn get_sized(&mut self, t: &Track, key: String, size: u32) -> Option<&Art> {
        let frame = self.frame;
        match self.map.entry(key) {
            std::collections::hash_map::Entry::Vacant(v) => {
                let _ = self.jobs.send((v.key().clone(), t.clone(), size));
                v.insert(Slot::Loading);
                None
            }
            std::collections::hash_map::Entry::Occupied(o) => match o.into_mut() {
                Slot::Ready(a, used) => {
                    *used = frame;
                    Some(a)
                }
                _ => None,
            },
        }
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        self.frame += 1;
        let ready = self.map.values().filter(|s| matches!(s, Slot::Ready(..))).count();
        if ready > MAX_TEXTURES {
            // forget the least recently drawn; they reload from the disk cache if seen again
            let mut used: Vec<(u64, String)> = self.map.iter().filter_map(|(k, s)| if let Slot::Ready(_, u) = s { Some((*u, k.clone())) } else { None }).collect();
            used.sort();
            for (_, k) in used.into_iter().take(ready - MAX_TEXTURES * 3 / 4) {
                self.map.remove(&k);
            }
        }
        while let Ok((key, img)) = self.done.try_recv() {
            let slot = match img {
                Some(img) => {
                    let size = [img.width() as usize, img.height() as usize];
                    let tex = ctx.load_texture(&key, egui::ColorImage::from_rgba_unmultiplied(size, &img), egui::TextureOptions::LINEAR);
                    let tiny = image::imageops::resize(&img, 12, 12, image::imageops::FilterType::Triangle);
                    let blur = ctx.load_texture(format!("{key}#blur"), egui::ColorImage::from_rgba_unmultiplied([12, 12], &tiny), egui::TextureOptions::LINEAR);
                    let small = image::imageops::resize(&img, 4, 4, image::imageops::FilterType::Triangle);
                    let grid: [Color32; 16] = std::array::from_fn(|i| {
                        let p = small.get_pixel((i % 4) as u32, (i / 4) as u32);
                        Color32::from_rgb(p[0], p[1], p[2])
                    });
                    let small = size[0].max(size[1]) < 120;
                    Slot::Ready(Art { tex, small, blur, aspect: size[0] as f32 / size[1] as f32, accent: accent_of(&grid), grid }, self.frame)
                }
                None => Slot::Failed,
            };
            self.map.insert(key, slot);
        }
    }
}

/// Disk cache first, then the network; the decoded, resized image is cached as PNG.
fn load(key: &str, t: &Track, size: u32) -> Option<image::RgbaImage> {
    let hash = key.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    let dir = dirs::cache_dir().unwrap_or_default().join("sonavelt").join(ART_DIR);
    let file = dir.join(format!("{hash:016x}.png"));
    if let Ok(img) = image::open(&file) {
        return Some(img.to_rgba8());
    }
    let img = fetch(t, size).and_then(|b| image::load_from_memory(&b).ok())?;
    let img = crop_letterbox(img);
    let img = if img.width().max(img.height()) > size { img.resize(size, size, image::imageops::FilterType::Triangle) } else { img }.to_rgba8();
    let _ = std::fs::create_dir_all(&dir);
    let _ = img.save(&file);
    Some(img)
}

fn fetch(t: &Track, size: u32) -> Option<Vec<u8>> {
    let get = |url: &str| -> Option<Vec<u8>> {
        let resp = ureq::get(url).set("User-Agent", "Mozilla/5.0 sonavelt/0.1").timeout(std::time::Duration::from_secs(6)).call().ok()?;
        let mut v = Vec::new();
        std::io::Read::read_to_end(&mut std::io::Read::take(resp.into_reader(), 8 << 20), &mut v).ok()?;
        // some servers answer 200 with an HTML error page
        image::guess_format(&v).is_ok().then_some(v)
    };
    match &t.source {
        Source::YouTube(id) if size > THUMB => get(&format!("https://i.ytimg.com/vi/{id}/maxresdefault.jpg")).or_else(|| get(&format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"))),
        Source::YouTube(id) => get(&format!("https://i.ytimg.com/vi/{id}/mqdefault.jpg")),
        // stations' own logos are often dead links; fall back to the site's icon
        Source::Radio(stream) => t.art.as_deref().and_then(get).or_else(|| {
            let page = t.art.as_deref().unwrap_or(stream);
            let host = page.split("://").nth(1)?.split(['/', ':', '?']).next()?;
            // a shared image host's icon (Google, Wikimedia, a CDN) isn't the station's
            const SHARED: &[&str] = &["google.", "gstatic.", "wikimedia.", "wikipedia.", "tunein.", "brightspotcdn.", "cloudfront.", "akamai", "imgur.", "githubusercontent.", "duckduckgo.", "bing."];
            (!SHARED.iter().any(|s| host.contains(s))).then(|| get(&format!("https://www.google.com/s2/favicons?domain={host}&sz=128")))?
        }),
        Source::Local(p) => {
            let out = Command::new("ffmpeg").args(["-v", "error", "-i"]).arg(p).args(["-an", "-frames:v", "1", "-f", "image2pipe", "-vcodec", "mjpeg", "-"]).output().ok()?;
            (!out.stdout.is_empty()).then_some(out.stdout)
        }
    }
}

/// YouTube's hqdefault pads 16:9 frames with black bars; cut them off.
fn crop_letterbox(i: image::DynamicImage) -> image::DynamicImage {
    let (w, h) = (i.width(), i.height());
    if w * 3 == h * 4 {
        let rgb = i.to_rgb8();
        let bar = (h - w * 9 / 16) / 2;
        let dark = (0..w).step_by(8).all(|x| rgb.get_pixel(x, 2).0.iter().all(|&c| c < 20));
        if dark && bar > 0 {
            return i.crop_imm(0, bar, w, h - 2 * bar);
        }
    }
    i
}

/// The most colorful cell of the grid, brightened so it reads on dark backgrounds.
fn accent_of(grid: &[Color32; 16]) -> Color32 {
    let best = grid
        .iter()
        .map(|&c| egui::ecolor::Hsva::from(c))
        .max_by(|a, b| (a.s * a.v).total_cmp(&(b.s * b.v)))
        .unwrap();
    if best.s < 0.18 {
        return crate::theme::FALLBACK_ACCENT;
    }
    egui::ecolor::Hsva::new(best.h, best.s.clamp(0.45, 0.7), 0.95, 1.0).into()
}

/// UV rect that crops the texture to fill `target` (CSS object-fit: cover).
pub fn cover_uv(aspect: f32, target: Rect) -> Rect {
    let t = target.width() / target.height();
    if aspect > t {
        let w = t / aspect;
        Rect::from_min_max(pos2((1.0 - w) / 2.0, 0.0), pos2((1.0 + w) / 2.0, 1.0))
    } else {
        let h = aspect / t;
        Rect::from_min_max(pos2(0.0, (1.0 - h) / 2.0), pos2(1.0, (1.0 + h) / 2.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_uv_crops_wide_image_to_square() {
        let uv = cover_uv(16.0 / 9.0, Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(100.0, 100.0)));
        assert!((uv.width() - 9.0 / 16.0).abs() < 1e-4 && uv.height() == 1.0);
    }
}
