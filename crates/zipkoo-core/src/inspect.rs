//! List archive contents and Peek at one file without extracting.
use crate::error::{Error, Result};
use crate::format::Format;
use crate::safety::is_executable_name;
use crate::split;
use serde::Serialize;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub name: String,
    pub size: u64,
    /// Packed size when the format records it per file.
    pub packed: Option<u64>,
    pub is_dir: bool,
    pub encrypted: bool,
    pub executable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArchiveInfo {
    pub path: PathBuf,
    pub format: Format,
    pub archive_size: u64,
    pub total_size: u64,
    pub file_count: usize,
    pub encrypted: bool,
    pub executables: Vec<String>,
    pub entries: Vec<Entry>,
}

/// Resolve split parts to a single readable file.
pub(crate) fn resolve(path: &Path) -> Result<(PathBuf, Format, u64)> {
    let (real, size) = if split::is_first_part(path) {
        let total: u64 = split::parts_of(path)?
            .iter()
            .map(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0))
            .sum();
        (split::join_to_temp(path)?, total)
    } else {
        (path.to_path_buf(), std::fs::metadata(path)?.len())
    };
    let fmt = Format::detect(&real)
        .ok_or_else(|| Error::UnsupportedFormat(path.display().to_string()))?;
    Ok((real, fmt, size))
}

pub(crate) fn tar_reader(path: &Path, fmt: Format) -> Result<Box<dyn Read>> {
    let f = BufReader::new(File::open(path)?);
    Ok(match fmt {
        Format::Tar => Box::new(f),
        Format::TarGz => Box::new(flate2::read::MultiGzDecoder::new(f)),
        Format::TarZst => Box::new(zstd::stream::read::Decoder::new(f)?),
        Format::TarBz2 => Box::new(bzip2::read::MultiBzDecoder::new(f)),
        Format::TarXz => Box::new(liblzma::read::XzDecoder::new_multi_decoder(f)),
        _ => unreachable!(),
    })
}

pub fn inspect(path: &Path, password: Option<&str>) -> Result<ArchiveInfo> {
    let (real, format, archive_size) = resolve(path)?;
    let mut entries = Vec::new();
    match format {
        Format::Zip => {
            let mut z = zip::ZipArchive::new(BufReader::new(File::open(&real)?))?;
            for i in 0..z.len() {
                let f = z.by_index_raw(i)?;
                entries.push(Entry {
                    name: f.name()?.to_string(),
                    size: f.size(),
                    packed: Some(f.compressed_size()),
                    is_dir: f.is_dir(),
                    encrypted: f.encrypted(),
                    executable: false,
                });
            }
        }
        Format::SevenZ => {
            let pw = password.map(sevenz_rust2::Password::from).unwrap_or_else(sevenz_rust2::Password::empty);
            let a = sevenz_rust2::Archive::open_with_password(&real, &pw)?;
            let encrypted = a.blocks.iter().any(|b| b.coders.iter().any(|c| c.encoder_method_id() == sevenz_rust2::EncoderMethod::ID_AES256_SHA256));
            for e in &a.files {
                entries.push(Entry {
                    name: e.name.clone(),
                    size: e.size,
                    packed: None,
                    is_dir: e.is_directory,
                    encrypted,
                    executable: false,
                });
            }
        }
        Format::Rar => {
            let mut a = match password {
                Some(pw) => unrar::Archive::with_password(&real, pw),
                None => unrar::Archive::new(&real),
            }
            .open_for_listing()?;
            for h in &mut a {
                let h = h?;
                entries.push(Entry {
                    name: h.filename.to_string_lossy().replace('\\', "/"),
                    size: h.unpacked_size,
                    packed: None,
                    is_dir: h.is_directory(),
                    encrypted: h.is_encrypted(),
                    executable: false,
                });
            }
        }
        _ => {
            let mut t = tar::Archive::new(tar_reader(&real, format)?);
            for e in t.entries()? {
                let e = e?;
                let h = e.header();
                entries.push(Entry {
                    name: e.path()?.to_string_lossy().into_owned(),
                    size: h.size()?,
                    packed: None,
                    is_dir: h.entry_type().is_dir(),
                    encrypted: false,
                    executable: h.mode().map(|m| m & 0o111 != 0 && !h.entry_type().is_dir()).unwrap_or(false),
                });
            }
        }
    }
    for e in &mut entries {
        e.executable = e.executable || (!e.is_dir && is_executable_name(&e.name));
    }
    let total_size = entries.iter().map(|e| e.size).sum();
    let file_count = entries.iter().filter(|e| !e.is_dir).count();
    let encrypted = entries.iter().any(|e| e.encrypted);
    let executables = entries.iter().filter(|e| e.executable).map(|e| e.name.clone()).collect();
    Ok(ArchiveInfo { path: path.to_path_buf(), format, archive_size, total_size, file_count, encrypted, executables, entries })
}

/// Read one file from the archive into memory (for previews). Refuses files over `max_bytes`.
pub fn peek(path: &Path, name: &str, password: Option<&str>, max_bytes: u64) -> Result<Vec<u8>> {
    let (real, format, _) = resolve(path)?;
    let too_big = || Error::TooLarge { limit: max_bytes };
    match format {
        Format::Zip => {
            let mut z = zip::ZipArchive::new(BufReader::new(File::open(&real)?))?;
            let idx = z.index_for_name(name).ok_or_else(|| Error::EntryNotFound(name.into()))?;
            let mut f = match password {
                Some(pw) => z.by_index_decrypt(idx, pw.as_bytes())?,
                None => z.by_index(idx)?,
            };
            if f.size() > max_bytes {
                return Err(too_big());
            }
            let mut out = Vec::with_capacity(f.size() as usize);
            f.by_ref().take(max_bytes + 1).read_to_end(&mut out)?;
            if out.len() as u64 > max_bytes {
                return Err(too_big());
            }
            Ok(out)
        }
        Format::SevenZ => {
            let pw = password.map(sevenz_rust2::Password::from).unwrap_or_else(sevenz_rust2::Password::empty);
            let mut r = sevenz_rust2::ArchiveReader::open(&real, pw)?;
            let entry = r.archive().files.iter().find(|e| e.name == name).ok_or_else(|| Error::EntryNotFound(name.into()))?;
            if entry.size > max_bytes {
                return Err(too_big());
            }
            Ok(r.read_file(name)?)
        }
        Format::Rar => {
            let mut a = match password {
                Some(pw) => unrar::Archive::with_password(&real, pw),
                None => unrar::Archive::new(&real),
            }
            .open_for_processing()?;
            while let Some(h) = a.read_header()? {
                let n = h.entry().filename.to_string_lossy().replace('\\', "/");
                if n == name {
                    if h.entry().unpacked_size > max_bytes {
                        return Err(too_big());
                    }
                    let (data, _) = h.read()?;
                    return Ok(data);
                }
                a = h.skip()?;
            }
            Err(Error::EntryNotFound(name.into()))
        }
        _ => {
            let mut t = tar::Archive::new(tar_reader(&real, format)?);
            for e in t.entries()? {
                let mut e = e?;
                if e.path()?.to_string_lossy() == name {
                    if e.header().size()? > max_bytes {
                        return Err(too_big());
                    }
                    let mut out = Vec::new();
                    e.read_to_end(&mut out)?;
                    return Ok(out);
                }
            }
            Err(Error::EntryNotFound(name.into()))
        }
    }
}
