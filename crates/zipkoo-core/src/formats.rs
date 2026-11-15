//! Read-only formats beyond ZIP/7z/TAR/RAR: ISO 9660 disc images, Microsoft CAB, LZH/LHA, and
//! single compressed files (.gz, .xz, .bz2, .zst). Every format is read through `walk`, so
//! listing, preview, extraction and testing all share one code path per format.
use crate::error::{Error, Result};
use crate::format::Format;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

/// One item from an archive. `reader` is `None` when the caller only lists (no decompression).
pub(crate) struct Item<'a> {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub reader: Option<&'a mut dyn Read>,
}

/// Visit every item. Return `Ok(false)` from `visit` to stop early.
pub(crate) fn walk(
    path: &Path,
    format: Format,
    read: bool,
    visit: &mut dyn FnMut(Item) -> Result<bool>,
) -> Result<()> {
    match format {
        Format::Gz | Format::Xz | Format::Bz2 | Format::Zst => single(path, format, read, visit),
        Format::Iso => iso(path, read, visit),
        Format::Cab => cab(path, read, visit),
        Format::Lzh => lzh(path, read, visit),
        _ => Err(Error::UnsupportedFormat(format.label().into())),
    }
}

pub(crate) fn decoder(format: Format, f: BufReader<File>) -> Result<Box<dyn Read>> {
    Ok(match format {
        Format::Gz => Box::new(flate2::read::MultiGzDecoder::new(f)),
        Format::Xz => Box::new(liblzma::read::XzDecoder::new_multi_decoder(f)),
        Format::Bz2 => Box::new(bzip2::read::MultiBzDecoder::new(f)),
        Format::Zst => Box::new(zstd::stream::read::Decoder::new(f)?),
        _ => unreachable!(),
    })
}

/// "report.pdf.gz" holds "report.pdf".
fn inner_name(path: &Path, format: Format) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = format!(".{}", format.extension());
    name.strip_suffix(&ext)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{name}.out"))
}

/// Uncompressed size from the gzip trailer (exact below 4 GB; 0 when unknown).
fn gzip_size(path: &Path) -> u64 {
    use std::io::{Seek, SeekFrom};
    let Ok(mut f) = File::open(path) else {
        return 0;
    };
    let mut b = [0u8; 4];
    if f.seek(SeekFrom::End(-4)).is_ok() && f.read_exact(&mut b).is_ok() {
        u32::from_le_bytes(b) as u64
    } else {
        0
    }
}

fn single(
    path: &Path,
    format: Format,
    read: bool,
    visit: &mut dyn FnMut(Item) -> Result<bool>,
) -> Result<()> {
    let name = inner_name(path, format);
    let size = if format == Format::Gz {
        gzip_size(path)
    } else {
        0
    };
    if !read {
        visit(Item {
            name,
            is_dir: false,
            size,
            reader: None,
        })?;
        return Ok(());
    }
    let mut d = decoder(format, BufReader::new(File::open(path)?))?;
    visit(Item {
        name,
        is_dir: false,
        size,
        reader: Some(&mut d),
    })?;
    Ok(())
}

fn iso(path: &Path, read: bool, visit: &mut dyn FnMut(Item) -> Result<bool>) -> Result<()> {
    for e in crate::iso::list(path)? {
        let size = e.extents.iter().map(|x| x.1).sum();
        let go = if read && !e.is_dir {
            let mut r = crate::iso::open(path, &e)?;
            visit(Item {
                name: e.name,
                is_dir: false,
                size,
                reader: Some(&mut r),
            })?
        } else {
            visit(Item {
                name: e.name,
                is_dir: e.is_dir,
                size,
                reader: None,
            })?
        };
        if !go {
            break;
        }
    }
    Ok(())
}

fn cab(path: &Path, read: bool, visit: &mut dyn FnMut(Item) -> Result<bool>) -> Result<()> {
    let mut c = cab::Cabinet::new(BufReader::new(File::open(path)?))
        .map_err(|e| Error::Damaged(format!("CAB: {e}")))?;
    let files: Vec<(String, u64)> = c
        .folder_entries()
        .flat_map(|f| {
            f.file_entries()
                .map(|e| (e.name().to_string(), e.uncompressed_size() as u64))
                .collect::<Vec<_>>()
        })
        .collect();
    for (raw, size) in files {
        let name = raw.replace('\\', "/");
        let go = if read {
            let mut r = c.read_file(&raw)?;
            visit(Item {
                name,
                is_dir: false,
                size,
                reader: Some(&mut r),
            })?
        } else {
            visit(Item {
                name,
                is_dir: false,
                size,
                reader: None,
            })?
        };
        if !go {
            break;
        }
    }
    Ok(())
}

fn lzh(path: &Path, read: bool, visit: &mut dyn FnMut(Item) -> Result<bool>) -> Result<()> {
    let mut lha = delharc::LhaDecodeReader::new(BufReader::new(File::open(path)?))
        .map_err(|e| Error::Damaged(format!("LZH: {e}")))?;
    loop {
        let h = lha.header();
        let name = h.parse_pathname_to_str().replace('\\', "/");
        let is_dir = h.is_directory();
        let size = h.original_size;
        let go = if is_dir {
            visit(Item {
                name: format!("{}/", name.trim_end_matches('/')),
                is_dir: true,
                size: 0,
                reader: None,
            })?
        } else if read && lha.is_decoder_supported() {
            let mut buf = Vec::new();
            lha.read_to_end(&mut buf)
                .map_err(|e| Error::Damaged(format!("LZH: {e}")))?;
            if !lha.crc_check().is_ok() {
                return Err(Error::Damaged(format!("LZH: checksum failed for {name}")));
            }
            let mut cur = std::io::Cursor::new(buf);
            visit(Item {
                name,
                is_dir: false,
                size,
                reader: Some(&mut cur),
            })?
        } else {
            visit(Item {
                name,
                is_dir: false,
                size,
                reader: None,
            })?
        };
        if !go
            || !lha
                .next_file()
                .map_err(|e| Error::Damaged(format!("LZH: {e}")))?
        {
            break;
        }
    }
    Ok(())
}
