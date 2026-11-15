//! Smart Compress: skip work on files that are already compressed.
use std::path::Path;

const PRECOMPRESSED: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "heic", "heif", "avif", "mp4", "mov", "mkv", "avi", "webm",
    "m4v", "mp3", "aac", "m4a", "ogg", "opus", "flac", "zip", "7z", "rar", "gz", "tgz", "bz2", "xz",
    "zst", "lz4", "br", "docx", "xlsx", "pptx", "odt", "ods", "epub", "jar", "apk", "dmg", "pdf",
];

pub fn is_precompressed(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| PRECOMPRESSED.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Default worker threads: all cores, at least 1.
pub fn threads() -> u32 {
    std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(1)
}
