//! Thumbnails for image clips (`images/<id>.thumb.png`). The image itself is
//! stored with the clip's raw clipboard data. Cliplog 0.1 stored a full PNG at
//! `images/<id>.png`; those are still read for old clips.

use base64::Engine;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{imageops, ExtendedColorType, ImageEncoder, RgbaImage};
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};

/// Thumbnails aren't made for images bigger than this (raw RGBA, ~8K x 4K).
pub const MAX_IMAGE_BYTES: usize = 128 * 1024 * 1024;
const THUMB_W: u32 = 400;
const THUMB_H: u32 = 160;

pub fn legacy_path(dir: &Path, id: u64) -> PathBuf {
    dir.join(format!("{id}.png"))
}

fn thumb_path(dir: &Path, id: u64) -> PathBuf {
    dir.join(format!("{id}.thumb.png"))
}

fn write_png(path: &Path, img: &RgbaImage) -> Result<(), String> {
    let file = BufWriter::new(File::create(path).map_err(|e| e.to_string())?);
    PngEncoder::new_with_quality(file, CompressionType::Fast, FilterType::Adaptive)
        .write_image(img.as_raw(), img.width(), img.height(), ExtendedColorType::Rgba8)
        .map_err(|e| e.to_string())
}

pub fn save_thumbnail(dir: &Path, id: u64, width: u32, height: u32, rgba: Vec<u8>) -> Result<(), String> {
    let img = RgbaImage::from_raw(width, height, rgba).ok_or("invalid image data")?;
    let thumb = if width > THUMB_W || height > THUMB_H {
        let scale = (THUMB_W as f64 / width as f64).min(THUMB_H as f64 / height as f64);
        let (w, h) = ((width as f64 * scale).max(1.0), (height as f64 * scale).max(1.0));
        imageops::thumbnail(&img, w as u32, h as u32)
    } else {
        img
    };
    write_png(&thumb_path(dir, id), &thumb)
}

/// Loads a full image saved by Cliplog 0.1. Returns (width, height, RGBA bytes).
pub fn load_legacy(dir: &Path, id: u64) -> Result<(u32, u32, Vec<u8>), String> {
    let img = image::open(legacy_path(dir, id)).map_err(|e| e.to_string())?.into_rgba8();
    Ok((img.width(), img.height(), img.into_raw()))
}

pub fn thumbnail_data_url(dir: &Path, id: u64) -> Option<String> {
    let bytes = fs::read(thumb_path(dir, id)).ok()?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    Some(format!("data:image/png;base64,{b64}"))
}

/// Deletes files and folders (named `<id>` or `<id>.<ext>`) whose clip is no
/// longer in the history.
pub fn prune(dir: &Path, keep: impl Fn(u64) -> bool) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let id = name.to_str().and_then(|n| n.split('.').next()).and_then(|n| n.parse().ok());
        if id.is_some_and(|id| !keep(id)) {
            let path = entry.path();
            let _ = if path.is_dir() { fs::remove_dir_all(path) } else { fs::remove_file(path) };
        }
    }
}
