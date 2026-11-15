//! Minimal read-only ISO 9660 reader with Joliet long names (no external dependency).
//! Directory records point at contiguous extents, so files are read with a seek + take.
use crate::error::{Error, Result};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const SECTOR: u64 = 2048;

pub(crate) struct IsoEntry {
    pub name: String,
    pub is_dir: bool,
    /// (byte offset, length) of each extent; large files can span several.
    pub extents: Vec<(u64, u64)>,
}

fn damaged(m: &str) -> Error {
    Error::Damaged(format!("ISO: {m}"))
}

fn read_sector(f: &mut File, lba: u64, buf: &mut [u8]) -> Result<()> {
    f.seek(SeekFrom::Start(lba * SECTOR))?;
    f.read_exact(buf).map_err(|_| damaged("unexpected end of image"))
}

/// Root directory (lba, size) and whether names are Joliet (UCS-2 big-endian).
fn root(f: &mut File) -> Result<(u64, u64, bool)> {
    let mut primary = None;
    let mut joliet = None;
    let mut vd = vec![0u8; SECTOR as usize];
    for lba in 16..64 {
        read_sector(f, lba, &mut vd)?;
        if &vd[1..6] != b"CD001" {
            return Err(damaged("not an ISO 9660 image"));
        }
        let rec = &vd[156..190];
        let at = (u32::from_le_bytes(rec[2..6].try_into().unwrap()) as u64, u32::from_le_bytes(rec[10..14].try_into().unwrap()) as u64);
        match vd[0] {
            1 => primary = Some(at),
            2 if vd[88] == 0x25 && vd[89] == 0x2F && matches!(vd[90], 0x40 | 0x43 | 0x45) => joliet = Some(at),
            255 => break,
            _ => {}
        }
    }
    match (joliet, primary) {
        (Some((l, s)), _) => Ok((l, s, true)),
        (None, Some((l, s))) => Ok((l, s, false)),
        _ => Err(damaged("no volume descriptor")),
    }
}

fn decode_name(raw: &[u8], joliet: bool) -> String {
    let s = if joliet {
        let units: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(raw).into_owned()
    };
    // Drop the ";1" version and a trailing dot left by names without an extension.
    let s = s.split(';').next().unwrap_or(&s).to_string();
    s.strip_suffix('.').map(str::to_string).unwrap_or(s)
}

/// Every file and folder in the image, depth first.
pub(crate) fn list(path: &Path) -> Result<Vec<IsoEntry>> {
    let mut f = File::open(path)?;
    let (lba, size, joliet) = root(&mut f)?;
    let mut out: Vec<IsoEntry> = Vec::new();
    let mut stack = vec![(lba, size, String::new())];
    let mut seen = std::collections::HashSet::new();
    while let Some((lba, size, prefix)) = stack.pop() {
        if !seen.insert(lba) || size > 64 << 20 {
            continue; // loops or absurd directory sizes in a damaged image
        }
        let mut dir = vec![0u8; size as usize];
        f.seek(SeekFrom::Start(lba * SECTOR))?;
        f.read_exact(&mut dir).map_err(|_| damaged("directory runs past the end"))?;
        let mut i = 0usize;
        while i < dir.len() {
            let len = dir[i] as usize;
            if len == 0 {
                // Records never cross sectors: skip to the next one.
                i = (i / SECTOR as usize + 1) * SECTOR as usize;
                continue;
            }
            if i + len > dir.len() || len < 34 {
                break;
            }
            let r = &dir[i..i + len];
            i += len;
            let name_len = r[32] as usize;
            if 33 + name_len > r.len() {
                continue;
            }
            let raw = &r[33..33 + name_len];
            if raw == [0] || raw == [1] {
                continue; // "." and ".."
            }
            let flags = r[25];
            let ext_lba = u32::from_le_bytes(r[2..6].try_into().unwrap()) as u64;
            let data_len = u32::from_le_bytes(r[10..14].try_into().unwrap()) as u64;
            let name = decode_name(raw, joliet);
            if flags & 0x02 != 0 {
                let full = format!("{prefix}{name}/");
                out.push(IsoEntry { name: full.clone(), is_dir: true, extents: vec![] });
                stack.push((ext_lba, data_len, full));
            } else {
                let full = format!("{prefix}{name}");
                let extent = (ext_lba * SECTOR, data_len);
                // Multi-extent files (> 4 GB) repeat the same name with flag 0x80 on all but the last.
                match out.last_mut() {
                    Some(prev) if !prev.is_dir && prev.name == full && prev.extents.len() < 64 => prev.extents.push(extent),
                    _ => out.push(IsoEntry { name: full, is_dir: false, extents: vec![extent] }),
                }
            }
        }
    }
    Ok(out)
}

/// A reader over a file's extents.
pub(crate) fn open(path: &Path, e: &IsoEntry) -> Result<Box<dyn Read>> {
    let mut parts: Vec<Box<dyn Read>> = Vec::new();
    for &(off, len) in &e.extents {
        let mut f = File::open(path)?;
        f.seek(SeekFrom::Start(off))?;
        parts.push(Box::new(f.take(len)));
    }
    Ok(parts.into_iter().reduce(|a, b| Box::new(a.chain(b))).unwrap_or_else(|| Box::new(std::io::empty())))
}
