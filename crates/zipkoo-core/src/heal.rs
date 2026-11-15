//! Self-heal: Reed-Solomon repair data stored next to an archive (`name.zip.zkrec`).
//!
//! The archive is cut into equal blocks. Blocks are grouped by stride (group g holds blocks
//! g, g+G, g+2G, ...), so one long damaged stretch — a bad sector run, a cut-off download —
//! touches each group only a little. Each group of `k` data blocks gets `m` parity blocks;
//! up to `m` bad blocks per group can be rebuilt. Every block has a BLAKE3 hash to find damage.
//!
//! File layout: header | data-block hashes | parity-block hashes | parity blocks (group-major).
use crate::error::{Error, Result};
use crate::progress::{Progress, Reporter};
use reed_solomon_erasure::galois_8::ReedSolomon;
use serde::Serialize;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

const MAGIC: &[u8; 8] = b"ZKREC\x01\0\0";
const HEADER_LEN: u64 = 8 + 8 + 4 + 2 + 2 + 4;
const K: usize = 20;
const MAX_BLOCKS: u64 = 1 << 15;

#[derive(Debug, Clone, Copy)]
struct Layout {
    file_size: u64,
    block: u64,
    k: usize,
    m: usize,
    groups: u64,
}

impl Layout {
    fn plan(file_size: u64, percent: u8) -> Self {
        let percent = percent.clamp(1, 50) as usize;
        // Keep the hash table small (<= 32k blocks) and blocks 4 KB aligned.
        let raw = file_size.div_ceil(MAX_BLOCKS).max(4096);
        let block = raw.div_ceil(4096) * 4096;
        let blocks = file_size.div_ceil(block).max(1);
        let k = K.min(blocks as usize).max(1);
        let m = (k * percent).div_ceil(100).max(1);
        let groups = blocks.div_ceil(k as u64);
        Self { file_size, block, k, m, groups }
    }
    fn blocks(&self) -> u64 {
        self.groups * self.k as u64
    }
    /// Block index of member j in group g.
    fn member(&self, g: u64, j: usize) -> u64 {
        g + j as u64 * self.groups
    }
    fn hashes_at(&self) -> u64 {
        HEADER_LEN
    }
    fn parity_hashes_at(&self) -> u64 {
        HEADER_LEN + self.blocks() * 32
    }
    fn parity_at(&self) -> u64 {
        self.parity_hashes_at() + self.groups * self.m as u64 * 32
    }
    fn parity_block_at(&self, g: u64, p: usize) -> u64 {
        self.parity_at() + (g * self.m as u64 + p as u64) * self.block
    }
}

pub fn recovery_path(archive: &Path) -> PathBuf {
    let mut s = archive.as_os_str().to_owned();
    s.push(".zkrec");
    PathBuf::from(s)
}

/// Read block `i` of the archive; bytes past the end (or missing because the file is short) are zeros.
fn read_block(f: &mut File, l: &Layout, i: u64, buf: &mut [u8]) -> std::io::Result<()> {
    buf.fill(0);
    let start = i * l.block;
    if start >= l.file_size {
        return Ok(());
    }
    f.seek(SeekFrom::Start(start))?;
    let want = (l.file_size - start).min(l.block) as usize;
    let mut got = 0;
    while got < want {
        let n = f.read(&mut buf[got..want])?;
        if n == 0 {
            break;
        }
        got += n;
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct RecoveryReport {
    pub path: PathBuf,
    pub size: u64,
    pub percent: u8,
}

/// Write `archive.zkrec` with about `percent`% repair data.
pub fn create(archive: &Path, percent: u8, on_progress: &mut dyn FnMut(&Progress), cancel: Option<&AtomicBool>) -> Result<RecoveryReport> {
    let size = std::fs::metadata(archive)?.len();
    let l = Layout::plan(size, percent);
    let rs = ReedSolomon::new(l.k, l.m).map_err(|e| Error::Damaged(format!("{e:?}")))?;
    let mut f = File::open(archive)?;
    let out_path = recovery_path(archive);
    let tmp = out_path.with_extension("zkrec.part");
    let mut rep = Reporter::new(on_progress, cancel, size);
    rep.set_current(&archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    let res = (|| -> Result<()> {
        let mut out = BufWriter::new(File::create(&tmp)?);
        out.write_all(MAGIC)?;
        out.write_all(&l.file_size.to_le_bytes())?;
        out.write_all(&(l.block as u32).to_le_bytes())?;
        out.write_all(&(l.k as u16).to_le_bytes())?;
        out.write_all(&(l.m as u16).to_le_bytes())?;
        out.write_all(&(l.groups as u32).to_le_bytes())?;
        // Reserve the hash tables; parity follows. Filled group by group.
        out.write_all(&vec![0u8; (l.blocks() * 32 + l.groups * l.m as u64 * 32) as usize])?;
        out.flush()?;
        let mut out = out.into_inner().map_err(|e| e.into_error())?;
        let mut shards = vec![vec![0u8; l.block as usize]; l.k + l.m];
        for g in 0..l.groups {
            for j in 0..l.k {
                let i = l.member(g, j);
                read_block(&mut f, &l, i, &mut shards[j])?;
                out.seek(SeekFrom::Start(l.hashes_at() + i * 32))?;
                out.write_all(blake3::hash(&shards[j]).as_bytes())?;
                rep.add(l.block.min(l.file_size.saturating_sub(i * l.block))).map_err(|_| Error::Cancelled)?;
            }
            rs.encode(&mut shards).map_err(|e| Error::Damaged(format!("{e:?}")))?;
            for p in 0..l.m {
                out.seek(SeekFrom::Start(l.parity_hashes_at() + (g * l.m as u64 + p as u64) * 32))?;
                out.write_all(blake3::hash(&shards[l.k + p]).as_bytes())?;
                out.seek(SeekFrom::Start(l.parity_block_at(g, p)))?;
                out.write_all(&shards[l.k + p])?;
            }
        }
        out.flush()?;
        Ok(())
    })();
    if let Err(e) = res {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, &out_path)?;
    rep.finish();
    Ok(RecoveryReport { size: std::fs::metadata(&out_path)?.len(), path: out_path, percent })
}

fn read_layout(rec: &mut File) -> Result<Layout> {
    let mut h = [0u8; HEADER_LEN as usize];
    rec.seek(SeekFrom::Start(0))?;
    rec.read_exact(&mut h).map_err(|_| Error::Damaged("the repair file is incomplete".into()))?;
    if &h[..8] != MAGIC {
        return Err(Error::Damaged("not a Zipkoo repair file".into()));
    }
    let file_size = u64::from_le_bytes(h[8..16].try_into().unwrap());
    let block = u32::from_le_bytes(h[16..20].try_into().unwrap()) as u64;
    let k = u16::from_le_bytes(h[20..22].try_into().unwrap()) as usize;
    let m = u16::from_le_bytes(h[22..24].try_into().unwrap()) as usize;
    let groups = u32::from_le_bytes(h[24..28].try_into().unwrap()) as u64;
    if block == 0 || k == 0 || m == 0 || k + m > 256 {
        return Err(Error::Damaged("the repair file header is invalid".into()));
    }
    Ok(Layout { file_size, block, k, m, groups })
}

/// Redundancy (percent) of an existing repair file, so it can be rebuilt with the same setting.
pub fn percent_of(rec: &Path) -> Result<u8> {
    let l = read_layout(&mut File::open(rec)?)?;
    Ok(((l.m * 100) / l.k).clamp(1, 50) as u8)
}

#[derive(Debug, Default, Serialize, PartialEq)]
pub struct Health {
    /// The archive is larger than when the repair data was made: it was changed on purpose
    /// (e.g. updated), so the repair data no longer applies and nothing is repaired.
    pub outdated: bool,
    pub blocks: u64,
    pub damaged: u64,
    /// Damaged blocks in groups that have more damage than parity can fix.
    pub unrecoverable: u64,
    pub size_ok: bool,
    pub repaired: u64,
}

impl Health {
    pub fn healthy(&self) -> bool {
        self.damaged == 0 && self.size_ok
    }
    pub fn repairable(&self) -> bool {
        self.unrecoverable == 0 && !self.outdated
    }
}

/// Check the archive against its repair file; with `fix`, rebuild damaged blocks in place.
pub fn check(archive: &Path, fix: bool, on_progress: &mut dyn FnMut(&Progress), cancel: Option<&AtomicBool>) -> Result<Health> {
    let rec_path = recovery_path(archive);
    let mut rec = BufReader::new(File::open(&rec_path)?).into_inner();
    let l = read_layout(&mut rec)?;
    let rs = ReedSolomon::new(l.k, l.m).map_err(|e| Error::Damaged(format!("{e:?}")))?;
    let actual = std::fs::metadata(archive).map(|m| m.len()).unwrap_or(0);
    let mut f = if fix { OpenOptions::new().read(true).write(true).open(archive)? } else { File::open(archive)? };
    let mut health = Health { blocks: l.file_size.div_ceil(l.block), size_ok: actual == l.file_size, ..Default::default() };
    if actual > l.file_size {
        // Growth is not damage. Repairing would cut the new data off.
        health.outdated = true;
        return Ok(health);
    }
    let mut rep = Reporter::new(on_progress, cancel, l.file_size);
    let mut buf = vec![0u8; l.block as usize];
    let mut hash = [0u8; 32];
    for g in 0..l.groups {
        let mut bad = Vec::new();
        let mut shards: Vec<Option<Vec<u8>>> = Vec::with_capacity(l.k + l.m);
        for j in 0..l.k {
            let i = l.member(g, j);
            read_block(&mut f, &l, i, &mut buf)?;
            rec.seek(SeekFrom::Start(l.hashes_at() + i * 32))?;
            rec.read_exact(&mut hash)?;
            if blake3::hash(&buf).as_bytes() == &hash {
                shards.push(Some(buf.clone()));
            } else {
                bad.push(j);
                shards.push(None);
            }
            rep.add(l.block).map_err(|_| Error::Cancelled)?;
        }
        if bad.is_empty() {
            continue;
        }
        let real_bad = bad.iter().filter(|j| l.member(g, **j) * l.block < l.file_size).count() as u64;
        health.damaged += real_bad;
        let mut good_parity = 0;
        for p in 0..l.m {
            rec.seek(SeekFrom::Start(l.parity_block_at(g, p)))?;
            let mut pb = vec![0u8; l.block as usize];
            let ok = rec.read_exact(&mut pb).is_ok() && {
                rec.seek(SeekFrom::Start(l.parity_hashes_at() + (g * l.m as u64 + p as u64) * 32))?;
                rec.read_exact(&mut hash).is_ok() && blake3::hash(&pb).as_bytes() == &hash
            };
            if ok {
                good_parity += 1;
                shards.push(Some(pb));
            } else {
                shards.push(None);
            }
        }
        if bad.len() > good_parity {
            health.unrecoverable += real_bad;
            continue;
        }
        if fix {
            rs.reconstruct_data(&mut shards).map_err(|e| Error::Damaged(format!("{e:?}")))?;
            for j in bad {
                let i = l.member(g, j);
                let start = i * l.block;
                if start >= l.file_size {
                    continue;
                }
                let len = (l.file_size - start).min(l.block) as usize;
                f.seek(SeekFrom::Start(start))?;
                f.write_all(&shards[j].as_ref().unwrap()[..len])?;
                health.repaired += 1;
            }
        }
    }
    if fix && health.unrecoverable == 0 {
        f.set_len(l.file_size)?;
        f.flush()?;
        health.size_ok = true;
    }
    rep.finish();
    Ok(health)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_bounds() {
        for size in [0u64, 1, 4096, 1 << 20, 3 << 30, 50 << 30] {
            let l = Layout::plan(size, 10);
            assert!(l.blocks() * l.block >= size, "{size}");
            assert!(l.k + l.m <= 256);
            assert!(l.blocks() <= MAX_BLOCKS + K as u64 * 2, "{size}: {}", l.blocks());
        }
    }
}
