//! Salvage a damaged ZIP without repair data. The central directory (the index at the end) is
//! ignored; the file is scanned for local file headers ("PK\x03\x04") and every entry whose data
//! decompresses and passes its CRC-32 is written to a new archive. Works when the end was cut off
//! or the index is broken, and skips past damaged entries. Stored and Deflate entries (almost all
//! real ZIPs) are recovered, including ones that use data descriptors.
use crate::error::{Error, Result};
use crate::progress::{Progress, Reporter};
use serde::Serialize;
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

#[derive(Debug, Default, Serialize)]
pub struct SalvageReport {
    pub output: PathBuf,
    pub recovered: usize,
    pub bytes: u64,
    /// Entries found but not recoverable, with the reason.
    pub lost: Vec<(String, String)>,
}

const SIG: &[u8; 4] = b"PK\x03\x04";

fn u16le(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}
fn u32le(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// Inflate raw deflate data from `data`; returns (plain bytes, compressed bytes consumed).
fn inflate(data: &[u8], limit: u64) -> Option<(Vec<u8>, usize)> {
    let mut d = flate2::Decompress::new(false);
    let mut out = Vec::with_capacity(data.len().min(1 << 20) * 2);
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let before_in = d.total_in() as usize;
        let before_out = d.total_out();
        let status = d
            .decompress(&data[before_in..], &mut buf, flate2::FlushDecompress::None)
            .ok()?;
        let produced = (d.total_out() - before_out) as usize;
        out.extend_from_slice(&buf[..produced]);
        if out.len() as u64 > limit {
            return None;
        }
        match status {
            flate2::Status::StreamEnd => return Some((out, d.total_in() as usize)),
            _ if d.total_in() as usize == before_in && produced == 0 => return None,
            _ => {}
        }
    }
}

pub fn salvage(
    path: &Path,
    out: &Path,
    on_progress: &mut dyn FnMut(&Progress),
    cancel: Option<&AtomicBool>,
) -> Result<SalvageReport> {
    let mut data = Vec::new();
    File::open(path)?.read_to_end(&mut data)?;
    if data.len() as u64 > 8 << 30 {
        return Err(Error::TooLarge { limit: 8 << 30 });
    }
    let mut rep = Reporter::new(on_progress, cancel, data.len() as u64);
    let mut r = SalvageReport {
        output: out.to_path_buf(),
        ..Default::default()
    };
    let tmp = out.with_extension("zip.part");
    let mut zw = zip::ZipWriter::new(BufWriter::new(File::create(&tmp)?));
    let mut seen = std::collections::HashSet::new();
    let mut i = 0usize;
    let res = (|| -> Result<()> {
        while i + 30 <= data.len() {
            if &data[i..i + 4] != SIG {
                i += 1;
                continue;
            }
            rep.state.done_bytes = i as u64;
            rep.add(0).map_err(|_| Error::Cancelled)?;
            let h = &data[i..];
            let flags = u16le(h, 6);
            let method = u16le(h, 8);
            let crc = u32le(h, 14);
            let csize = u32le(h, 18) as usize;
            let usize_ = u32le(h, 22) as u64;
            let nlen = u16le(h, 26) as usize;
            let xlen = u16le(h, 28) as usize;
            let start = i + 30 + nlen + xlen;
            if start > data.len() || nlen == 0 {
                i += 4;
                continue;
            }
            let raw_name = &h[30..30 + nlen];
            let name = if flags & 0x0800 != 0 {
                String::from_utf8_lossy(raw_name).into_owned()
            } else {
                crate::names::decode_legacy(raw_name)
            };
            if flags & 0x1 != 0 {
                r.lost.push((name, "encrypted".into()));
                i += 4;
                continue;
            }
            let descriptor = flags & 0x8 != 0;
            // Data: use the header's size when known, else let the deflate stream end itself.
            let (plain, consumed) = match method {
                0 if !descriptor || csize > 0 => {
                    let end = start.checked_add(csize).filter(|&e| e <= data.len());
                    match end {
                        Some(e) => (data[start..e].to_vec(), csize),
                        None => {
                            r.lost.push((name, "cut off".into()));
                            break;
                        }
                    }
                }
                8 => {
                    let slice = if !descriptor && csize > 0 {
                        &data[start..(start + csize).min(data.len())]
                    } else {
                        &data[start..]
                    };
                    match inflate(slice, 16 << 30) {
                        Some(x) => x,
                        None => {
                            r.lost.push((name, "damaged data".into()));
                            i += 4;
                            continue;
                        }
                    }
                }
                m => {
                    r.lost.push((
                        name,
                        format!("compression method {m} not supported for salvage"),
                    ));
                    i += 4;
                    continue;
                }
            };
            // CRC from the header, or from the data descriptor that follows.
            let mut want_crc = crc;
            let mut next = start + consumed;
            if descriptor {
                let mut d = next;
                if d + 4 <= data.len() && &data[d..d + 4] == b"PK\x07\x08" {
                    d += 4;
                }
                if d + 12 <= data.len() {
                    want_crc = u32le(&data, d);
                    next = d + 12;
                }
            }
            let ok =
                crc32fast::hash(&plain) == want_crc && (descriptor || usize_ == plain.len() as u64);
            if !ok {
                r.lost.push((name, "checksum mismatch".into()));
                i += 4;
                continue;
            }
            if name.ends_with('/') || !seen.insert(name.clone()) {
                i = next.max(i + 4);
                continue;
            }
            let o = zip::write::SimpleFileOptions::default()
                .compression_method(if method == 0 {
                    zip::CompressionMethod::Stored
                } else {
                    zip::CompressionMethod::Deflated
                })
                .large_file(plain.len() as u64 >= u32::MAX as u64);
            zw.start_file(name.as_str(), o)?;
            zw.write_all(&plain)?;
            r.recovered += 1;
            r.bytes += plain.len() as u64;
            i = next.max(i + 4);
        }
        zw.finish()?.flush()?;
        Ok(())
    })();
    if let Err(e) = res {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if r.recovered == 0 {
        let _ = std::fs::remove_file(&tmp);
        return Err(Error::Damaged(
            "nothing could be recovered from this file".into(),
        ));
    }
    std::fs::rename(&tmp, out)?;
    rep.finish();
    Ok(r)
}

/// "Photos.zip" → "Photos (recovered).zip", never overwriting.
pub fn recovered_path(path: &Path) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Archive".into());
    crate::pack::unique_path(&path.with_file_name(format!("{stem} (recovered).zip")))
}
