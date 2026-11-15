//! Smart Update for ZIP archives: add only what changed, keep the rest byte-for-byte,
//! and keep a version history inside the archive.
//!
//! History lives in `.zipkoo/manifest.json`; with `keep_history`, replaced or removed files are
//! kept under `.zipkoo/history/v<N>/<path>` so older versions can still be extracted.
use crate::error::{Error, Result};
use crate::meta::{from_zip_time, zip_round, zip_time};
use crate::progress::{CountingReader, Progress, Reporter};
use crate::smart::is_precompressed;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::SystemTime;

pub const ZIPKOO_DIR: &str = ".zipkoo/";
pub const MANIFEST: &str = ".zipkoo/manifest.json";
pub const HISTORY_DIR: &str = ".zipkoo/history/";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Version {
    pub version: u32,
    /// Seconds since 1970 (UTC).
    pub time: u64,
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub removed: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Manifest {
    pub versions: Vec<Version>,
}

pub struct UpdateOptions<'a> {
    pub password: Option<&'a str>,
    /// Keep replaced and removed files under `.zipkoo/history/`.
    pub keep_history: bool,
    /// Drop files that are no longer in the inputs.
    pub remove_missing: bool,
    pub smart: bool,
}

impl Default for UpdateOptions<'_> {
    fn default() -> Self {
        Self {
            password: None,
            keep_history: true,
            remove_missing: false,
            smart: true,
        }
    }
}

#[derive(Debug, Default, Serialize)]
pub struct UpdateReport {
    pub version: u32,
    pub added: usize,
    pub changed: usize,
    pub removed: usize,
    pub unchanged: usize,
    /// Bytes read from disk and compressed this time (unchanged files are copied as-is).
    pub bytes_packed: u64,
}

struct Src {
    path: PathBuf,
    size: u64,
    mtime: Option<SystemTime>,
}

fn collect(inputs: &[PathBuf]) -> Result<BTreeMap<String, Option<Src>>> {
    // name -> Some(file) | None (directory)
    let mut out = BTreeMap::new();
    for input in inputs {
        let base = input.parent().unwrap_or(Path::new("")).to_path_buf();
        for e in walkdir::WalkDir::new(input).follow_links(false) {
            let e = e.map_err(|e| Error::Io(e.into()))?;
            if e.file_type().is_symlink() {
                continue;
            }
            let name = e
                .path()
                .strip_prefix(&base)
                .unwrap_or(e.path())
                .to_string_lossy()
                .replace('\\', "/");
            if name.is_empty() {
                continue;
            }
            if e.file_type().is_dir() {
                out.insert(format!("{name}/"), None);
            } else {
                let m = e.metadata().map_err(|e| Error::Io(e.into()))?;
                out.insert(
                    name,
                    Some(Src {
                        path: e.path().to_path_buf(),
                        size: m.len(),
                        mtime: m.modified().ok().and_then(zip_round),
                    }),
                );
            }
        }
    }
    Ok(out)
}

fn crc_of(path: &Path) -> std::io::Result<u32> {
    let mut h = crc32fast::Hasher::new();
    let mut f = BufReader::new(File::open(path)?);
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize())
}

pub fn read_manifest(archive: &Path) -> Result<Manifest> {
    let mut z = zip::ZipArchive::new(BufReader::new(File::open(archive)?))?;
    let Some(i) = z.index_for_name(MANIFEST) else {
        return Ok(Manifest::default());
    };
    let mut s = String::new();
    z.by_index(i)?.read_to_string(&mut s)?;
    serde_json::from_str(&s).map_err(|e| Error::Damaged(format!("history: {e}")))
}

pub fn update(
    archive: &Path,
    inputs: &[PathBuf],
    opts: &UpdateOptions,
    on_progress: &mut dyn FnMut(&Progress),
    cancel: Option<&AtomicBool>,
) -> Result<UpdateReport> {
    use zip::write::SimpleFileOptions;
    if let Some(pw) = opts.password {
        // Never re-encrypt new files with a mistyped password.
        crate::inspect::inspect(archive, Some(pw))?;
    }
    let mut old = zip::ZipArchive::new(BufReader::new(File::open(archive)?))?;
    let mut manifest = read_manifest(archive)?;
    let wanted = collect(inputs)?;

    // What is in the archive now (ignoring our own history folder).
    struct Old {
        idx: usize,
        size: u64,
        crc: u32,
        mtime: Option<SystemTime>,
        encrypted: bool,
    }
    let mut current: BTreeMap<String, Old> = BTreeMap::new();
    for i in 0..old.len() {
        let f = old.by_index_raw(i)?;
        let name = crate::names::zip_name(&f);
        if name.starts_with(".zipkoo/") {
            continue;
        }
        current.insert(
            name,
            Old {
                idx: i,
                size: f.size(),
                crc: f.crc32(),
                mtime: f.last_modified().and_then(from_zip_time),
                encrypted: f.encrypted(),
            },
        );
    }
    if current.values().any(|o| o.encrypted) && opts.password.is_none() {
        return Err(Error::PasswordRequired);
    }

    let next_version = manifest.versions.last().map(|v| v.version + 1).unwrap_or(1);
    let mut v = Version {
        version: next_version,
        time: SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        ..Default::default()
    };

    // Decide per file.
    let mut to_pack: Vec<(&String, &Src)> = Vec::new();
    let mut keep: BTreeSet<String> = BTreeSet::new();
    for (name, src) in &wanted {
        let Some(src) = src else {
            if current.contains_key(name) {
                keep.insert(name.clone());
            } else {
                v.added.push(name.clone());
            }
            continue;
        };
        match current.get(name) {
            None => {
                v.added.push(name.clone());
                to_pack.push((name, src));
            }
            Some(o) => {
                let same = o.size == src.size
                    && (o.mtime.is_some() && o.mtime == src.mtime
                        || crc_of(&src.path).map(|c| c == o.crc).unwrap_or(false));
                if same {
                    keep.insert(name.clone());
                } else {
                    v.changed.push(name.clone());
                    to_pack.push((name, src));
                }
            }
        }
    }
    let removed: Vec<String> = current
        .keys()
        .filter(|n| !wanted.contains_key(*n))
        .cloned()
        .collect();
    if opts.remove_missing {
        v.removed = removed.clone();
    } else {
        keep.extend(removed);
    }

    let total: u64 = to_pack.iter().map(|(_, s)| s.size).sum();
    let mut rep = Reporter::new(on_progress, cancel, total);
    let tmp = archive.with_extension("zip.part");
    let res = (|| -> Result<UpdateReport> {
        let mut zw = zip::ZipWriter::new(BufWriter::new(File::create(&tmp)?));
        // 1. Unchanged entries and old history: copied raw, no recompression, no decryption.
        for i in 0..old.len() {
            let name = crate::names::zip_name(&old.by_index_raw(i)?);
            if name == MANIFEST {
                continue;
            }
            if name.starts_with(HISTORY_DIR) || keep.contains(&name) {
                zw.raw_copy_file(old.by_index_raw(i)?)?;
            }
        }
        // 2. Replaced or removed files move into history.
        if opts.keep_history {
            for name in v.changed.iter().chain(v.removed.iter()) {
                if let Some(o) = current.get(name) {
                    let hist = format!("{HISTORY_DIR}v{}/{name}", next_version - 1);
                    zw.raw_copy_file_rename(old.by_index_raw(o.idx)?, hist)?;
                }
            }
        }
        // 3. New folders and new or changed files.
        let mut packed = 0u64;
        for name in &v.added {
            if name.ends_with('/') {
                let mut o = SimpleFileOptions::default();
                if let Some(pw) = opts.password {
                    o = o.with_aes_encryption(zip::AesMode::Aes256, pw);
                }
                zw.add_directory(name.as_str(), o)?;
            }
        }
        for (name, src) in &to_pack {
            let mut o = SimpleFileOptions::default().large_file(src.size >= u32::MAX as u64);
            o = if opts.smart && is_precompressed(&src.path) {
                o.compression_method(zip::CompressionMethod::Stored)
            } else {
                o.compression_method(zip::CompressionMethod::Deflated)
            };
            if let Some(t) = src.mtime.and_then(zip_time) {
                o = o.last_modified_time(t);
            }
            if let Some(pw) = opts.password {
                o = o.with_aes_encryption(zip::AesMode::Aes256, pw);
            }
            rep.set_current(name);
            zw.start_file(name.as_str(), o)?;
            let mut r = CountingReader {
                inner: BufReader::new(File::open(&src.path)?),
                reporter: &mut rep,
            };
            packed += std::io::copy(&mut r, &mut zw)?;
        }
        // 4. Manifest.
        let report = UpdateReport {
            version: next_version,
            added: v.added.iter().filter(|n| !n.ends_with('/')).count(),
            changed: v.changed.len(),
            removed: v.removed.len(),
            unchanged: keep.iter().filter(|n| !n.ends_with('/')).count(),
            bytes_packed: packed,
        };
        manifest.versions.push(v.clone());
        zw.start_file(MANIFEST, SimpleFileOptions::default())?;
        zw.write_all(serde_json::to_string_pretty(&manifest).unwrap().as_bytes())?;
        zw.finish()?.flush()?;
        Ok(report)
    })();
    match res {
        Ok(r) => {
            drop(old);
            fs::rename(&tmp, archive)?;
            // Repair data describes the old bytes; rebuild it so Self-heal never "repairs"
            // the updated archive back to the old one.
            let rec = crate::heal::recovery_path(archive);
            if rec.exists() {
                let pct = crate::heal::percent_of(&rec).unwrap_or(10);
                let _ = fs::remove_file(&rec);
                crate::heal::create(archive, pct, &mut |_| {}, None)?;
            }
            rep.finish();
            Ok(r)
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(match e {
                Error::Io(io) if io.to_string().contains("zipkoo: cancelled") => Error::Cancelled,
                other => other,
            })
        }
    }
}
