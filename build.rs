//! Embeds the icon and version information into Spacelens.exe.

#[path = "src/icon.rs"]
mod icon;

use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/icon.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let ico = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("spacelens.ico");
    write_ico(&ico, &[16, 20, 24, 32, 40, 48, 64, 256]);

    let mut res = winresource::WindowsResource::new();
    res.set_icon(ico.to_str().unwrap());
    res.set("FileDescription", "Spacelens - Disk Space Analyzer");
    res.set("ProductName", "Spacelens");
    res.set("OriginalFilename", "Spacelens.exe");
    res.compile().expect("failed to embed Windows resources");
}

/// Writes a multi-size .ico whose entries are PNG-encoded (supported since Windows Vista).
fn write_ico(path: &Path, sizes: &[u32]) {
    let images: Vec<(u32, Vec<u8>)> = sizes
        .iter()
        .map(|&s| {
            let mut bytes = Vec::new();
            {
                let mut enc = png::Encoder::new(&mut bytes, s, s);
                enc.set_color(png::ColorType::Rgba);
                enc.set_depth(png::BitDepth::Eight);
                enc.write_header().unwrap().write_image_data(&icon::rgba(s)).unwrap();
            }
            (s, bytes)
        })
        .collect();

    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (s, bytes) in &images {
        let dim = if *s >= 256 { 0 } else { *s as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += bytes.len() as u32;
    }
    for (_, bytes) in &images {
        out.extend_from_slice(bytes);
    }
    std::fs::write(path, out).unwrap();
}
