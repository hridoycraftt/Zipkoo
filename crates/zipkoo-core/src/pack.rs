//! Create archives: ZIP, 7z and TAR.ZST, with Smart Compress, AES-256 and splitting.
use crate::error::{Error, Result};
use crate::format::Format;
use crate::progress::{CountingReader, Progress, Reporter};
use crate::smart::{is_precompressed, threads};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use walkdir::WalkDir;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Fast,
    #[default]
    Normal,
    Max,
}

impl Level {
    fn deflate(self) -> i64 {
        match self {
            Level::Fast => 1,
            Level::Normal => 6,
            Level::Max => 9,
        }
    }
    fn lzma(self) -> u32 {
        match self {
            Level::Fast => 1,
            Level::Normal => 6,
            Level::Max => 9,
        }
    }
    fn zstd(self) -> i32 {
        match self {
            Level::Fast => 1,
            Level::Normal => 6,
            Level::Max => 19,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PackOptions {
    pub format: Format,
    pub level: Level,
    /// Store already-compressed files (JPG, MP4, ZIP...) instead of squeezing them again.
    pub smart: bool,
    pub password: Option<String>,
    /// 7z only: also hide file names until the password is entered.
    pub hide_names: bool,
    /// Split the result into parts of this many bytes.
    pub split_size: Option<u64>,
    pub threads: u32,
}

impl Default for PackOptions {
    fn default() -> Self {
        Self {
            format: Format::Zip,
            level: Level::Normal,
            smart: true,
            password: None,
            hide_names: false,
            split_size: None,
            threads: threads(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct PackReport {
    pub outputs: Vec<PathBuf>,
    pub files: usize,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub stored_as_is: usize,
}

struct Item {
    src: PathBuf,
    name: String,
    is_dir: bool,
    size: u64,
}

fn collect(inputs: &[PathBuf]) -> Result<Vec<Item>> {
    let mut items = Vec::new();
    for input in inputs {
        let base = input.parent().unwrap_or(Path::new("")).to_path_buf();
        for e in WalkDir::new(input).follow_links(false).sort_by_file_name() {
            let e = e.map_err(|e| Error::Io(e.into()))?;
            let ft = e.file_type();
            if ft.is_symlink() {
                continue;
            }
            let rel = e.path().strip_prefix(&base).unwrap_or(e.path());
            let name = rel.to_string_lossy().replace('\\', "/");
            if name.is_empty() {
                continue;
            }
            items.push(Item {
                src: e.path().to_path_buf(),
                name,
                is_dir: ft.is_dir(),
                size: if ft.is_file() {
                    e.metadata().map(|m| m.len()).unwrap_or(0)
                } else {
                    0
                },
            });
        }
    }
    if items.iter().all(|i| i.is_dir) && items.is_empty() {
        return Err(Error::NoInputs);
    }
    Ok(items)
}

pub fn pack(
    inputs: &[PathBuf],
    output: &Path,
    opts: &PackOptions,
    on_progress: &mut dyn FnMut(&Progress),
    cancel: Option<&AtomicBool>,
) -> Result<PackReport> {
    if inputs.is_empty() {
        return Err(Error::NoInputs);
    }
    if !opts.format.can_create() {
        return Err(Error::CreateNotSupported(opts.format.label()));
    }
    if opts.password.is_some() && !opts.format.supports_password() {
        return Err(Error::EncryptionNotSupported(opts.format.label()));
    }
    let items = collect(inputs)?;
    let input_bytes: u64 = items.iter().map(|i| i.size).sum();
    let mut rep = Reporter::new(on_progress, cancel, input_bytes);
    // Write to a temp name first so a cancelled job never leaves a half archive behind.
    let tmp = output.with_extension(format!(
        "{}.part",
        output.extension().and_then(|e| e.to_str()).unwrap_or("tmp")
    ));
    let res = write_archive(&items, &tmp, opts, &mut rep);
    let stored_as_is = match res {
        Ok(n) => n,
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            return Err(map_cancel(e));
        }
    };
    fs::rename(&tmp, output)?;
    rep.finish();
    let output_bytes = fs::metadata(output)?.len();
    let outputs = match opts.split_size {
        Some(s) => crate::split::split_file(output, s)?,
        None => vec![output.to_path_buf()],
    };
    Ok(PackReport {
        outputs,
        files: items.iter().filter(|i| !i.is_dir).count(),
        input_bytes,
        output_bytes,
        stored_as_is,
    })
}

fn map_cancel(e: Error) -> Error {
    match e {
        Error::Io(io) if io.to_string().contains("zipkoo: cancelled") => Error::Cancelled,
        Error::Damaged(m) if m.contains("zipkoo: cancelled") => Error::Cancelled,
        other => other,
    }
}

fn write_archive(
    items: &[Item],
    out: &Path,
    opts: &PackOptions,
    rep: &mut Reporter,
) -> Result<usize> {
    let mut stored = 0;
    match opts.format {
        Format::Zip => {
            use zip::write::SimpleFileOptions;
            use zip::CompressionMethod;
            let mut zw = zip::ZipWriter::new(BufWriter::new(File::create(out)?));
            for it in items {
                let mut o = SimpleFileOptions::default();
                if let Some(pw) = opts.password.as_deref() {
                    o = o.with_aes_encryption(zip::AesMode::Aes256, pw);
                }
                if it.is_dir {
                    zw.add_directory(it.name.as_str(), o)?;
                    continue;
                }
                let store = opts.smart && is_precompressed(&it.src);
                if store {
                    stored += 1;
                    o = o.compression_method(CompressionMethod::Stored);
                } else {
                    o = o
                        .compression_method(CompressionMethod::Deflated)
                        .compression_level(Some(opts.level.deflate()));
                }
                o = o.large_file(it.size >= u32::MAX as u64);
                if let Some(t) = fs::metadata(&it.src)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(crate::meta::zip_time)
                {
                    o = o.last_modified_time(t);
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(m) = fs::metadata(&it.src) {
                        o = o.unix_permissions(m.permissions().mode());
                    }
                }
                rep.set_current(&it.name);
                zw.start_file(it.name.as_str(), o)?;
                let mut r = CountingReader {
                    inner: BufReader::new(File::open(&it.src)?),
                    reporter: rep,
                };
                std::io::copy(&mut r, &mut zw)?;
            }
            zw.finish()?.flush()?;
        }
        Format::SevenZ => {
            use sevenz_rust2::{
                encoder_options::{AesEncoderOptions, Lzma2Options},
                ArchiveEntry, ArchiveWriter, EncoderMethod, Password,
            };
            let mut w = ArchiveWriter::create(out)?;
            let aes = opts
                .password
                .as_deref()
                .map(|p| AesEncoderOptions::new(Password::from(p)));
            if aes.is_some() && opts.hide_names {
                w.set_encrypt_header(true);
            }
            let lz = Lzma2Options::from_level_mt(opts.level.lzma(), opts.threads.max(1), 16 << 20);
            for it in items {
                let entry = ArchiveEntry::from_path(&it.src, it.name.clone());
                if it.is_dir {
                    w.push_archive_entry::<File>(entry, None)?;
                    continue;
                }
                let store = opts.smart && is_precompressed(&it.src);
                if store {
                    stored += 1;
                }
                let mut methods = Vec::new();
                if let Some(a) = &aes {
                    methods.push(a.clone().into());
                }
                methods.push(if store {
                    EncoderMethod::COPY.into()
                } else {
                    lz.clone().into()
                });
                w.set_content_methods(methods);
                rep.set_current(&it.name);
                let r = CountingReader {
                    inner: BufReader::new(File::open(&it.src)?),
                    reporter: &mut *rep,
                };
                w.push_archive_entry(entry, Some(r))?;
            }
            w.finish()?;
        }
        Format::TarZst => {
            let f = BufWriter::new(File::create(out)?);
            let mut enc = zstd::stream::write::Encoder::new(f, opts.level.zstd())?;
            enc.multithread(opts.threads.max(1))?;
            enc.include_checksum(true)?;
            let mut tb = tar::Builder::new(enc);
            tb.follow_symlinks(false);
            for it in items {
                if it.is_dir {
                    tb.append_dir(&it.name, &it.src)?;
                    continue;
                }
                rep.set_current(&it.name);
                let file = File::open(&it.src)?;
                let mut header = tar::Header::new_gnu();
                let meta = file.metadata()?;
                header.set_metadata(&meta);
                // The header promises `size` bytes. If the file grows or shrinks while we read it,
                // write exactly that many (cut or zero-padded) so later entries stay intact.
                let size = meta.len();
                let exact = std::io::Read::take(
                    std::io::Read::chain(std::io::Read::take(file, size), std::io::repeat(0)),
                    size,
                );
                let mut r = CountingReader {
                    inner: exact,
                    reporter: rep,
                };
                tb.append_data(&mut header, &it.name, &mut r)?;
            }
            let enc = tb.into_inner()?;
            enc.finish()?.flush()?;
        }
        f => return Err(Error::CreateNotSupported(f.label())),
    }
    Ok(stored)
}

/// `path` if free, else "name (2).ext", "name (3).ext"... Handles double extensions like .tar.zst.
pub fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let parent = path.parent().unwrap_or(Path::new(""));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (stem, ext) = match name.find(".tar.") {
        Some(i) => (name[..i].to_string(), name[i..].to_string()),
        None => match name.rfind('.') {
            Some(i) if i > 0 && !path.is_dir() => (name[..i].to_string(), name[i..].to_string()),
            _ => (name.clone(), String::new()),
        },
    };
    for n in 2.. {
        let p = parent.join(format!("{stem} ({n}){ext}"));
        if !p.exists() {
            return p;
        }
    }
    unreachable!()
}
