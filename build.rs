//! Gives the Windows .exe its icon, drawn from the same code as the window and tray icons.

#[allow(dead_code)]
#[path = "src/logo.rs"]
mod logo;

fn main() {
    println!("cargo::rerun-if-changed=src/logo.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let ico = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("sonavelt.ico");
    std::fs::write(&ico, ico_bytes(&[16, 20, 24, 32, 40, 48, 64, 256])).unwrap();
    embed(&ico);
}

#[cfg(windows)]
fn embed(ico: &std::path::Path) {
    let mut res = winresource::WindowsResource::new();
    res.set_icon(ico.to_str().unwrap());
    res.compile().unwrap();
}

// shortcut: cross-compiling from Linux builds an .exe without an icon; release builds run on Windows
#[cfg(not(windows))]
fn embed(_: &std::path::Path) {}

/// An .ico with one uncompressed 32-bit image per size, which every Windows version reads.
fn ico_bytes(sizes: &[usize]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes.iter().map(|&n| dib(n)).collect();
    let mut out = Vec::new();
    for v in [0u16, 1, sizes.len() as u16] {
        out.extend(v.to_le_bytes());
    }
    let mut offset = 6 + 16 * sizes.len();
    for (&n, img) in sizes.iter().zip(&images) {
        let side = if n >= 256 { 0 } else { n as u8 }; // 0 means 256
        out.extend([side, side, 0, 0]);
        out.extend(1u16.to_le_bytes());
        out.extend(32u16.to_le_bytes());
        out.extend((img.len() as u32).to_le_bytes());
        out.extend((offset as u32).to_le_bytes());
        offset += img.len();
    }
    out.extend(images.concat());
    out
}

/// One icon image: a bitmap header, BGRA rows bottom-up, then an all-clear AND mask.
fn dib(n: usize) -> Vec<u8> {
    let rgba = logo::rgba(n);
    let mask_row = n.div_ceil(32) * 4;
    let mut out = Vec::new();
    for v in [40u32, n as u32, 2 * n as u32] {
        out.extend(v.to_le_bytes());
    }
    out.extend(1u16.to_le_bytes());
    out.extend(32u16.to_le_bytes());
    for v in [0u32, (n * n * 4 + mask_row * n) as u32, 0, 0, 0, 0] {
        out.extend(v.to_le_bytes());
    }
    for row in rgba.chunks(n * 4).rev() {
        for p in row.chunks(4) {
            out.extend([p[2], p[1], p[0], p[3]]);
        }
    }
    out.resize(out.len() + mask_row * n, 0);
    out
}

#[test]
fn ico_layout() {
    let ico = ico_bytes(&[16, 256]);
    assert_eq!(&ico[..6], &[0, 0, 1, 0, 2, 0]);
    let first = u32::from_le_bytes(ico[18..22].try_into().unwrap()) as usize;
    assert_eq!(first, 6 + 32);
    assert_eq!(ico[6 + 16], 0, "256 px is stored as 0");
    assert_eq!(ico.len(), 6 + 32 + (40 + 16 * 16 * 4 + 4 * 16) + (40 + 256 * 256 * 4 + 32 * 256));
}
