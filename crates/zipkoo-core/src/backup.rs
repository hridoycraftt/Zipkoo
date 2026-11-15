//! Watch Folder snapshots: pack a folder into a dated archive, skip when nothing changed,
//! and keep only the newest N snapshots.
use crate::error::{Error, Result};
use crate::format::Format;
use crate::pack::{pack, PackOptions, PackReport};
use crate::progress::Progress;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupOptions {
    pub format: Format,
    /// Keep this many snapshots; older ones are deleted. 0 keeps all.
    pub keep: usize,
    pub password: Option<String>,
    /// Write a repair file (Self-heal) with this much redundancy.
    pub recovery_percent: Option<u8>,
}

impl Default for BackupOptions {
    fn default() -> Self {
        Self { format: Format::Zip, keep: 10, password: None, recovery_percent: None }
    }
}

#[derive(Debug, Serialize)]
pub struct Snapshot {
    pub path: PathBuf,
    pub report: PackReport,
    pub pruned: Vec<PathBuf>,
}

/// A cheap fingerprint of a folder: every file's path, size and modified time.
/// Sizes and times come from `fs::metadata` on each file, not from the directory listing:
/// on Windows (NTFS) the listing's cached values can lag behind until a file is reopened,
/// which made unchanged folders look changed. Folder times are left out (they move when
/// anything inside is touched, including by antivirus or indexers).
pub fn fingerprint(dir: &Path) -> String {
    let mut h = blake3::Hasher::new();
    for e in walkdir::WalkDir::new(dir).sort_by_file_name().into_iter().filter_map(|e| e.ok()) {
        let rel = e.path().strip_prefix(dir).unwrap_or(e.path()).to_string_lossy().replace('\\', "/");
        h.update(rel.as_bytes());
        h.update(&[0]);
        if e.file_type().is_file() {
            if let Ok(m) = fs::metadata(e.path()) {
                h.update(&m.len().to_le_bytes());
                if let Ok(t) = m.modified().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).map_err(std::io::Error::other)) {
                    h.update(&t.as_nanos().to_le_bytes());
                }
            }
        }
    }
    h.finalize().to_hex().to_string()
}

fn state_file(dest: &Path, name: &str) -> PathBuf {
    dest.join(format!(".zipkoo-watch-{name}.json"))
}

fn stamp() -> String {
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    format!("{:04}-{:02}-{:02} {:02}-{:02}-{:02}", now.year(), now.month() as u8, now.day(), now.hour(), now.minute(), now.second())
}

/// Existing snapshots of `name` in `dest`, oldest first.
pub fn snapshots(dest: &Path, name: &str, format: Format) -> Vec<PathBuf> {
    let prefix = format!("{name} ");
    let suffix = format!(".{}", format.extension());
    let mut v: Vec<PathBuf> = fs::read_dir(dest)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).map(|n| n.starts_with(&prefix) && n.ends_with(&suffix)).unwrap_or(false))
        .collect();
    v.sort();
    v
}

/// Make a snapshot if `src` changed since the last one. Returns None when nothing changed.
pub fn snapshot(
    src: &Path,
    dest: &Path,
    opts: &BackupOptions,
    on_progress: &mut dyn FnMut(&Progress),
    cancel: Option<&AtomicBool>,
) -> Result<Option<Snapshot>> {
    if !src.is_dir() {
        return Err(Error::Io(std::io::Error::new(std::io::ErrorKind::NotFound, format!("Folder not found: {}", src.display()))));
    }
    fs::create_dir_all(dest)?;
    let name = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Folder".into());
    let fp = fingerprint(src);
    let state = state_file(dest, &name);
    if fs::read_to_string(&state).ok().as_deref() == Some(fp.as_str()) && !snapshots(dest, &name, opts.format).is_empty() {
        return Ok(None);
    }
    let out = crate::pack::unique_path(&dest.join(format!("{name} {}.{}", stamp(), opts.format.extension())));
    let po = PackOptions { format: opts.format, password: opts.password.clone(), ..Default::default() };
    let report = pack(&[src.to_path_buf()], &out, &po, on_progress, cancel)?;
    if let Some(p) = opts.recovery_percent {
        crate::heal::create(&out, p, &mut |_| {}, cancel)?;
    }
    fs::write(&state, &fp)?;
    let mut pruned = Vec::new();
    if opts.keep > 0 {
        let all = snapshots(dest, &name, opts.format);
        if all.len() > opts.keep {
            for old in &all[..all.len() - opts.keep] {
                if fs::remove_file(old).is_ok() {
                    let _ = fs::remove_file(crate::heal::recovery_path(old));
                    pruned.push(old.clone());
                }
            }
        }
    }
    Ok(Some(Snapshot { path: out, report, pruned }))
}
