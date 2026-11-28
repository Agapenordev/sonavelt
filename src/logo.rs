//! The Sonavelt mark, rasterized at any size: a velvet-red disc (a record, a small world)
//! crossed by one cream sound wave that reads as an S. Used for the window icon, the tray and
//! the sidebar, so it is drawn from distance fields rather than shipped as image files.

use std::f32::consts::PI;

const DISC_LIGHT: [f32; 3] = [176.0, 38.0, 74.0];
const DISC_DARK: [f32; 3] = [92.0, 14.0, 42.0];
const WAVE: [f32; 3] = [248.0, 234.0, 214.0];

/// RGBA pixels, `n` x `n`, straight (non-premultiplied) alpha.
pub fn rgba(n: usize) -> Vec<u8> {
    // one full sine period running top to bottom: upper half bulges left, lower half right,
    // leaning like an italic S, so the same stroke reads as the letter and as a sound wave
    let wave: Vec<(f32, f32)> = (0..=96)
        .map(|i| {
            let t = -0.56 + 1.12 * i as f32 / 96.0;
            let (x, y) = (0.27 * (t * PI / 0.56).sin(), t);
            let a: f32 = 0.38;
            (x * a.cos() - y * a.sin(), x * a.sin() + y * a.cos())
        })
        .collect();
    let px = 2.0 / n as f32; // one pixel in unit space, for anti-aliasing
    let mut out = Vec::with_capacity(n * n * 4);
    for j in 0..n {
        for i in 0..n {
            let (x, y) = ((i as f32 + 0.5) * px - 1.0, (j as f32 + 0.5) * px - 1.0);
            let r = (x * x + y * y).sqrt();
            let disc = ((0.94 - r) / px + 0.5).clamp(0.0, 1.0);
            // lit from the top left, like the sheen on velvet
            let k = ((x + y) * 0.5 + 0.5).clamp(0.0, 1.0);
            let mut c: [f32; 3] = std::array::from_fn(|ch| DISC_LIGHT[ch] * (1.0 - k) + DISC_DARK[ch] * k);
            let d = wave.windows(2).map(|s| seg_dist((x, y), s[0], s[1])).fold(f32::MAX, f32::min);
            let w = ((0.105 - d) / px + 0.5).clamp(0.0, 1.0);
            for ch in 0..3 {
                c[ch] = c[ch] * (1.0 - w) + WAVE[ch] * w;
            }
            out.extend([c[0] as u8, c[1] as u8, c[2] as u8, (disc * 255.0) as u8]);
        }
    }
    out
}

/// Same pixels as ARGB32 in network byte order, the tray protocol's format.
#[cfg(target_os = "linux")]
pub fn argb(n: usize) -> Vec<u8> {
    rgba(n).as_chunks::<4>().0.iter().flat_map(|p| [p[3], p[0], p[1], p[2]]).collect()
}

fn seg_dist(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    ((p.0 - a.0 - t * dx).powi(2) + (p.1 - a.1 - t * dy).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    #[test]
    fn mark_is_round_with_a_wave_through_it() {
        let n = 64;
        let px = super::rgba(n);
        let at = |x: usize, y: usize| &px[(y * n + x) * 4..(y * n + x) * 4 + 4];
        assert_eq!(at(0, 0)[3], 0, "corners are transparent");
        assert_eq!(at(n / 2, n / 2)[3], 255, "center is opaque");
        assert!(at(n / 2, n / 2)[0] > 200, "the wave crosses the center in cream");
    }
}
