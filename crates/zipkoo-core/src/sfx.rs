//! Self-extracting archives: a small stub program with a ZIP payload appended.
//!
//! Layout: `[stub exe][zip payload][payload offset: u64 LE][MAGIC: 8 bytes]`.
//! The stub (crates/zipkoo-sfx) reads the trailer from its own file and unpacks the payload.
use crate::error::{Error, Result};
use crate::format::Format;
use crate::pack::{pack, PackOptions, PackReport};
use crate::progress::Progress;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

pub const MAGIC: &[u8; 8] = b"ZKSFX\x01\0\0";
pub const TRAILER_LEN: u64 = 16;
/// Windows refuses to start programs of 4 GB or more.
pub const MAX_EXE: u64 = (4 << 30) - (1 << 20);

/// Build `output` (an .exe on Windows) from `stub` plus a ZIP of `inputs`.
/// Uses ZIP so the payload can be password-protected with AES-256.
pub fn build(
    stub: &Path,
    inputs: &[PathBuf],
    output: &Path,
    opts: &PackOptions,
    on_progress: &mut dyn FnMut(&Progress),
    cancel: Option<&AtomicBool>,
) -> Result<PackReport> {
    if !stub.is_file() {
        return Err(Error::Io(io::Error::new(io::ErrorKind::NotFound, format!("SFX stub not found: {}", stub.display()))));
    }
    // Quick check before spending time packing: stored size can't be below the input once
    // nothing compresses, so only refuse up front when even a 4x squeeze couldn't fit.
    let input: u64 = inputs.iter().map(|p| dir_size(p)).sum();
    if input / 4 > MAX_EXE {
        return Err(Error::SfxTooBig);
    }
    let payload = output.with_extension("sfx-payload.zip");
    let zip_opts = PackOptions { format: Format::Zip, split_size: None, hide_names: false, ..opts.clone() };
    let mut report = pack(inputs, &payload, &zip_opts, on_progress, cancel)?;
    let res = (|| -> Result<u64> {
        let total = fs::metadata(stub)?.len() + fs::metadata(&payload)?.len() + TRAILER_LEN;
        if total > MAX_EXE {
            return Err(Error::SfxTooBig);
        }
        let tmp = output.with_extension("exe.part");
        {
            let mut out = BufWriter::new(File::create(&tmp)?);
            let stub_len = io::copy(&mut BufReader::new(File::open(stub)?), &mut out)?;
            io::copy(&mut BufReader::new(File::open(&payload)?), &mut out)?;
            out.write_all(&stub_len.to_le_bytes())?;
            out.write_all(MAGIC)?;
            out.flush()?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755))?;
        }
        fs::rename(&tmp, output)?;
        Ok(fs::metadata(output)?.len())
    })();
    let _ = fs::remove_file(&payload);
    report.output_bytes = res?;
    report.outputs = vec![output.to_path_buf()];
    Ok(report)
}

/// Find the payload inside a self-extracting file: returns (offset, length) of the ZIP bytes.
pub fn locate_payload(file: &mut File) -> io::Result<Option<(u64, u64)>> {
    let len = file.metadata()?.len();
    if len < TRAILER_LEN {
        return Ok(None);
    }
    file.seek(SeekFrom::End(-(TRAILER_LEN as i64)))?;
    let mut t = [0u8; 16];
    file.read_exact(&mut t)?;
    if &t[8..] != MAGIC {
        return Ok(None);
    }
    let offset = u64::from_le_bytes(t[..8].try_into().unwrap());
    if offset >= len - TRAILER_LEN {
        return Ok(None);
    }
    Ok(Some((offset, len - TRAILER_LEN - offset)))
}

/// A Read + Seek window over part of a file.
pub struct Window<R> {
    inner: R,
    start: u64,
    len: u64,
    pos: u64,
}

impl<R: Read + Seek> Window<R> {
    pub fn new(mut inner: R, start: u64, len: u64) -> io::Result<Self> {
        inner.seek(SeekFrom::Start(start))?;
        Ok(Self { inner, start, len, pos: 0 })
    }
}

impl<R: Read + Seek> Read for Window<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.len.saturating_sub(self.pos);
        if left == 0 {
            return Ok(0);
        }
        let n = (buf.len() as u64).min(left) as usize;
        let n = self.inner.read(&mut buf[..n])?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl<R: Read + Seek> Seek for Window<R> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let target = match to {
            SeekFrom::Start(p) => p as i128,
            SeekFrom::End(d) => self.len as i128 + d as i128,
            SeekFrom::Current(d) => self.pos as i128 + d as i128,
        };
        if target < 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "seek before start"));
        }
        self.pos = target as u64;
        self.inner.seek(SeekFrom::Start(self.start + self.pos))?;
        Ok(self.pos)
    }
}

fn dir_size(p: &Path) -> u64 {
    walkdir::WalkDir::new(p)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok().map(|m| m.len()))
        .sum()
}
