//! Shrink photos before sharing: resize to a maximum edge and re-encode JPEGs.
//! Originals are never touched; results go to `out_dir`. A result that would be larger is
//! replaced by a plain copy of the original. Videos are copied as-is (no video encoder bundled).
use crate::error::Result;
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageReader};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy)]
pub struct OptimizeOptions {
    /// Longest edge in pixels; larger photos are scaled down.
    pub max_edge: u32,
    /// JPEG quality 1–100.
    pub quality: u8,
}

impl Default for OptimizeOptions {
    fn default() -> Self {
        Self { max_edge: 2560, quality: 82 }
    }
}

#[derive(Debug, Default, Serialize)]
pub struct OptimizeReport {
    pub out_dir: PathBuf,
    pub images: usize,
    pub shrunk: usize,
    pub copied: usize,
    pub before: u64,
    pub after: u64,
}

fn kind(p: &Path) -> Option<&'static str> {
    match p.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => Some("jpeg"),
        "png" => Some("png"),
        "webp" => Some("webp"),
        _ => None,
    }
}

fn shrink(src: &Path, dst: &Path, k: &str, o: &OptimizeOptions) -> image::ImageResult<()> {
    let img = ImageReader::open(src)?.with_guessed_format()?.decode()?;
    let img = if img.width().max(img.height()) > o.max_edge { img.resize(o.max_edge, o.max_edge, image::imageops::FilterType::Lanczos3) } else { img };
    match k {
        "png" => img.save_with_format(dst, image::ImageFormat::Png),
        // JPEG and WebP photos become JPEG (the bundled WebP encoder is lossless only).
        _ => {
            let rgb = DynamicImage::ImageRgb8(img.to_rgb8());
            let mut f = std::io::BufWriter::new(fs::File::create(dst)?);
            JpegEncoder::new_with_quality(&mut f, o.quality.clamp(1, 100)).encode_image(&rgb)
        }
    }
}

/// Optimize every photo under `inputs` into `out_dir`, keeping the folder structure.
/// Non-image files are copied so `out_dir` is a complete, ready-to-pack copy.
pub fn optimize(inputs: &[PathBuf], out_dir: &Path, o: &OptimizeOptions) -> Result<OptimizeReport> {
    let mut r = OptimizeReport { out_dir: out_dir.to_path_buf(), ..Default::default() };
    for input in inputs {
        let base = input.parent().unwrap_or(Path::new("")).to_path_buf();
        for e in walkdir::WalkDir::new(input).follow_links(false).into_iter().filter_map(|e| e.ok()) {
            let rel = e.path().strip_prefix(&base).unwrap_or(e.path());
            let dst = out_dir.join(rel);
            if e.file_type().is_dir() {
                fs::create_dir_all(&dst)?;
                continue;
            }
            if !e.file_type().is_file() {
                continue;
            }
            if let Some(p) = dst.parent() {
                fs::create_dir_all(p)?;
            }
            let before = e.metadata().map(|m| m.len()).unwrap_or(0);
            r.before += before;
            let Some(k) = kind(e.path()) else {
                fs::copy(e.path(), &dst)?;
                r.after += before;
                r.copied += 1;
                continue;
            };
            r.images += 1;
            let dst = if k == "webp" { dst.with_extension("jpg") } else { dst };
            let ok = shrink(e.path(), &dst, k, o).is_ok();
            let after = fs::metadata(&dst).map(|m| m.len()).unwrap_or(u64::MAX);
            if ok && after < before {
                r.shrunk += 1;
                r.after += after;
            } else {
                let _ = fs::remove_file(&dst);
                let plain = out_dir.join(rel);
                fs::copy(e.path(), &plain)?;
                r.after += before;
            }
        }
    }
    Ok(r)
}
