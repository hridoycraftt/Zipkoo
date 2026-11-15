//! Find identical files before packing: group by size, then a quick hash of the first 64 KB,
//! then a full BLAKE3 hash. Only same-size files are ever read.
use crate::error::Result;
use serde::Serialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub struct DupGroup {
    pub size: u64,
    pub files: Vec<PathBuf>,
    /// Space that could be saved by keeping one copy.
    pub wasted: u64,
}

fn hash(path: &Path, limit: Option<u64>) -> std::io::Result<[u8; 32]> {
    let mut h = blake3::Hasher::new();
    let f = File::open(path)?;
    let mut r: Box<dyn Read> = match limit {
        Some(l) => Box::new(f.take(l)),
        None => Box::new(f),
    };
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(*h.finalize().as_bytes())
}

pub fn find(inputs: &[PathBuf], min_size: u64) -> Result<Vec<DupGroup>> {
    let mut by_size: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    for input in inputs {
        for e in walkdir::WalkDir::new(input)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if e.file_type().is_file() {
                let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                if size >= min_size.max(1) {
                    by_size
                        .entry(size)
                        .or_default()
                        .push(e.path().to_path_buf());
                }
            }
        }
    }
    let mut groups = Vec::new();
    for (size, files) in by_size.into_iter().filter(|(_, v)| v.len() > 1) {
        let mut by_head: HashMap<[u8; 32], Vec<PathBuf>> = HashMap::new();
        for f in files {
            if let Ok(h) = hash(&f, Some(64 << 10)) {
                by_head.entry(h).or_default().push(f);
            }
        }
        for same_head in by_head.into_values().filter(|v| v.len() > 1) {
            let candidates = if size <= 64 << 10 {
                vec![same_head]
            } else {
                let mut by_full: HashMap<[u8; 32], Vec<PathBuf>> = HashMap::new();
                for f in same_head {
                    if let Ok(h) = hash(&f, None) {
                        by_full.entry(h).or_default().push(f);
                    }
                }
                by_full.into_values().filter(|v| v.len() > 1).collect()
            };
            for mut files in candidates {
                files.sort();
                groups.push(DupGroup {
                    size,
                    wasted: size * (files.len() as u64 - 1),
                    files,
                });
            }
        }
    }
    groups.sort_by(|a, b| b.wasted.cmp(&a.wasted));
    Ok(groups)
}
