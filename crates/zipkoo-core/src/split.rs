//! Split an archive into fixed-size parts (name.7z.001, .002, ...) and join them back.
//! The raw split layout is the same one 7-Zip uses, so other tools can open the parts too.
use crate::error::{Error, Result};
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

pub fn strip_part_suffix(name: &str) -> &str {
    if let Some((base, num)) = name.rsplit_once('.') {
        if num.len() == 3 && num.bytes().all(|b| b.is_ascii_digit()) {
            return base;
        }
    }
    name
}

pub fn is_first_part(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some("001")
}

fn part_path(base: &Path, n: usize) -> PathBuf {
    let mut s = base.as_os_str().to_owned();
    s.push(format!(".{n:03}"));
    PathBuf::from(s)
}

/// Split `path` into parts of `part_size` bytes and remove the original.
pub fn split_file(path: &Path, part_size: u64) -> Result<Vec<PathBuf>> {
    let len = fs::metadata(path)?.len();
    if part_size == 0 || len <= part_size {
        return Ok(vec![path.to_path_buf()]);
    }
    let mut src = BufReader::new(File::open(path)?);
    let mut parts = Vec::new();
    let mut n = 1;
    loop {
        let p = part_path(path, n);
        let mut out = BufWriter::new(File::create(&p)?);
        let copied = io::copy(&mut (&mut src).take(part_size), &mut out)?;
        out.flush()?;
        if copied == 0 {
            drop(out);
            fs::remove_file(&p)?;
            break;
        }
        parts.push(p);
        n += 1;
    }
    fs::remove_file(path)?;
    Ok(parts)
}

/// All parts starting from `first` (.001), in order.
pub fn parts_of(first: &Path) -> Result<Vec<PathBuf>> {
    let base = first.with_extension("");
    let mut parts = Vec::new();
    for n in 1.. {
        let p = part_path(&base, n);
        if !p.exists() {
            break;
        }
        parts.push(p);
    }
    if parts.is_empty() {
        return Err(Error::MissingPart(first.to_path_buf()));
    }
    Ok(parts)
}

/// A temporary folder that is deleted when dropped.
pub struct TempDir(pub PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn unique_temp_dir() -> std::io::Result<TempDir> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("zipkoo-join-{}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed), nanos));
    fs::create_dir_all(&dir)?;
    Ok(TempDir(dir))
}

/// Join parts into one temporary file. The file keeps the inner archive name so format
/// detection still works; the returned guard deletes it when the caller is done.
pub fn join_to_temp(first: &Path) -> Result<(PathBuf, TempDir)> {
    let parts = parts_of(first)?;
    let inner = first.with_extension("");
    let name = inner.file_name().unwrap_or_default();
    let guard = unique_temp_dir()?;
    let dir = guard.0.clone();
    let out_path = dir.join(name);
    let mut out = BufWriter::new(File::create(&out_path)?);
    for p in parts {
        io::copy(&mut BufReader::new(File::open(p)?), &mut out)?;
    }
    out.flush()?;
    Ok((out_path, guard))
}
