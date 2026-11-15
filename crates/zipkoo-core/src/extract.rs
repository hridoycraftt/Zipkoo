//! Safe Extract for every supported format.
use crate::error::{Error, Result};
use crate::format::Format;
use crate::inspect::{resolve, tar_reader};
use crate::progress::{Progress, Reporter};
use crate::safety::{guarded_copy, is_executable_name, safe_join, Budget};
use serde::Serialize;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

pub struct ExtractOptions<'a> {
    pub password: Option<&'a str>,
    pub overwrite: bool,
    /// Bomb guard: allowed output = archive size × ratio (at least `floor` bytes).
    pub max_ratio: u64,
    pub floor: u64,
    pub hard_cap: Option<u64>,
    /// When false, files that can run code are skipped and listed in the report.
    pub allow_executables: bool,
    /// Only extract these entry names. Empty means everything.
    pub only: Vec<String>,
}

impl Default for ExtractOptions<'_> {
    fn default() -> Self {
        Self {
            password: None,
            overwrite: false,
            max_ratio: 1000,
            floor: 4 << 30,
            hard_cap: None,
            allow_executables: true,
            only: Vec::new(),
        }
    }
}

#[derive(Debug, Default, Serialize)]
pub struct ExtractReport {
    pub dest: PathBuf,
    pub files: usize,
    pub bytes: u64,
    /// Entries left out, with the reason.
    pub skipped: Vec<(String, String)>,
}

struct Ctx<'o, 'r, 'a> {
    dest: &'o Path,
    opts: &'o ExtractOptions<'o>,
    budget: Budget,
    report: ExtractReport,
    rep: &'r mut Reporter<'a>,
}

impl Ctx<'_, '_, '_> {
    fn wanted(&self, name: &str) -> bool {
        self.opts.only.is_empty() || self.opts.only.iter().any(|o| o == name || name.starts_with(&format!("{}/", o.trim_end_matches('/'))))
    }

    /// Decide where an entry goes, or why it is skipped.
    fn target(&mut self, name: &str, is_dir: bool) -> Result<Option<PathBuf>> {
        if !self.wanted(name) {
            return Ok(None);
        }
        let out = match safe_join(self.dest, name) {
            Ok(p) => p,
            Err(_) => {
                self.report.skipped.push((name.into(), "unsafe path".into()));
                return Ok(None);
            }
        };
        if is_dir {
            fs::create_dir_all(&out)?;
            return Ok(None);
        }
        if !self.opts.allow_executables && is_executable_name(name) {
            self.report.skipped.push((name.into(), "program blocked".into()));
            return Ok(None);
        }
        if out.exists() && !self.opts.overwrite {
            self.report.skipped.push((name.into(), "already exists".into()));
            return Ok(None);
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        self.rep.set_current(name);
        Ok(Some(out))
    }

    fn write(&mut self, out: &Path, r: &mut dyn std::io::Read) -> Result<()> {
        let mut w = BufWriter::new(File::create(out)?);
        let res = guarded_copy(r, &mut w, &mut self.budget, self.rep);
        match res {
            Ok(n) => {
                w.flush()?;
                self.report.files += 1;
                self.report.bytes += n;
                Ok(())
            }
            Err(e) => {
                drop(w);
                let _ = fs::remove_file(out);
                Err(e)
            }
        }
    }
}

pub fn extract(
    path: &Path,
    dest: &Path,
    opts: &ExtractOptions,
    on_progress: &mut dyn FnMut(&Progress),
    cancel: Option<&AtomicBool>,
) -> Result<ExtractReport> {
    let (real, format, archive_size) = resolve(path)?;
    let declared = match crate::inspect::inspect(path, opts.password) {
        Ok(i) if opts.only.is_empty() => i.total_size,
        Ok(i) => i.entries.iter().filter(|e| opts.only.iter().any(|o| e.name == *o || e.name.starts_with(&format!("{}/", o.trim_end_matches('/'))))).map(|e| e.size).sum(),
        Err(_) => 0,
    };
    let total = declared;
    fs::create_dir_all(dest)?;
    // Refuse up front when the declared output can't fit (keeps 64 MB of headroom).
    if let Ok(free) = fs4::available_space(dest) {
        if declared > free.saturating_sub(64 << 20) {
            return Err(Error::NoSpace { need: declared, free });
        }
    }
    let mut rep = Reporter::new(on_progress, cancel, total);
    let mut ctx = Ctx {
        dest,
        opts,
        budget: Budget::for_archive(archive_size, opts.max_ratio, opts.floor, declared, opts.hard_cap),
        report: ExtractReport { dest: dest.to_path_buf(), ..Default::default() },
        rep: &mut rep,
    };

    match format {
        Format::Zip => {
            let mut z = zip::ZipArchive::new(BufReader::new(File::open(&real)?))?;
            for i in 0..z.len() {
                let (name, is_dir, is_link, encrypted) = {
                    let f = z.by_index_raw(i)?;
                    (f.name()?.to_string(), f.is_dir(), f.is_symlink(), f.encrypted())
                };
                if is_link {
                    ctx.report.skipped.push((name, "link not followed".into()));
                    continue;
                }
                let Some(out) = ctx.target(&name, is_dir)? else { continue };
                if encrypted && opts.password.is_none() {
                    return Err(Error::PasswordRequired);
                }
                let mut f = match (encrypted, opts.password) {
                    (true, Some(pw)) => z.by_index_decrypt(i, pw.as_bytes())?,
                    _ => z.by_index(i)?,
                };
                // A wrong AES password fails when the data is read, so map that here.
                match ctx.write(&out, &mut f) {
                    Err(Error::Io(e)) if encrypted && e.to_string().to_lowercase().contains("password") => {
                        return Err(Error::WrongPassword)
                    }
                    other => other?,
                }
            }
        }
        Format::SevenZ => {
            let pw = opts.password.map(sevenz_rust2::Password::from).unwrap_or_else(sevenz_rust2::Password::empty);
            let mut r = sevenz_rust2::ArchiveReader::open(&real, pw)?;
            let mut err: Option<Error> = None;
            r.for_each_entries(|entry, reader| {
                let res = (|| -> Result<()> {
                    let Some(out) = ctx.target(&entry.name, entry.is_directory)? else {
                        // Drain so solid blocks stay in sync.
                        std::io::copy(reader, &mut std::io::sink())?;
                        return Ok(());
                    };
                    ctx.write(&out, reader)
                })();
                match res {
                    Ok(()) => Ok(true),
                    Err(e) => {
                        err = Some(e);
                        Ok(false)
                    }
                }
            })?;
            if let Some(e) = err {
                return Err(e);
            }
        }
        Format::Rar => {
            let mut a = match opts.password {
                Some(pw) => unrar::Archive::with_password(&real, pw),
                None => unrar::Archive::new(&real),
            }
            .open_for_processing()?;
            while let Some(h) = a.read_header()? {
                let e = h.entry();
                let name = e.filename.to_string_lossy().replace('\\', "/");
                let size = e.unpacked_size;
                match ctx.target(&name, e.is_directory())? {
                    Some(out) => {
                        // unrar streams to disk itself, so charge the declared size up front.
                        ctx.budget.charge(size)?;
                        a = h.extract_to(&out)?;
                        ctx.rep.add(size).map_err(|_| Error::Cancelled)?;
                        ctx.report.files += 1;
                        ctx.report.bytes += size;
                    }
                    None => a = h.skip()?,
                }
            }
        }
        _ => {
            let mut t = tar::Archive::new(tar_reader(&real, format)?);
            for e in t.entries()? {
                let mut e = e?;
                let name = e.path()?.to_string_lossy().into_owned();
                let kind = e.header().entry_type();
                if kind.is_symlink() || kind.is_hard_link() {
                    ctx.report.skipped.push((name, "link not followed".into()));
                    continue;
                }
                if !(kind.is_file() || kind.is_dir() || kind == tar::EntryType::Continuous || kind == tar::EntryType::GNUSparse) {
                    continue;
                }
                let Some(out) = ctx.target(&name, kind.is_dir())? else { continue };
                ctx.write(&out, &mut e)?;
                #[cfg(unix)]
                if let Ok(mode) = e.header().mode() {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = fs::set_permissions(&out, fs::Permissions::from_mode(mode & 0o755));
                }
            }
        }
    }
    let report = std::mem::take(&mut ctx.report);
    drop(ctx);
    rep.finish();
    Ok(report)
}

/// Where "Extract here" should put things. If everything sits in one top folder, extract next to
/// the archive (the folder is created by the archive itself); otherwise use a folder named after it.
/// Never reuses an existing path: adds " (2)", " (3)"...
pub fn suggest_dest(archive: &Path, info: &crate::inspect::ArchiveInfo) -> (PathBuf, PathBuf) {
    let parent = archive.parent().unwrap_or(Path::new(".")).to_path_buf();
    let name = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let name = crate::split::strip_part_suffix(&name).to_string();
    let stem = name
        .strip_suffix(&format!(".{}", info.format.extension()))
        .map(str::to_string)
        .unwrap_or_else(|| Path::new(&name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or(name.clone()));
    let tops: std::collections::BTreeSet<&str> = info
        .entries
        .iter()
        .map(|e| e.name.trim_start_matches("./").split('/').next().unwrap_or(""))
        .filter(|s| !s.is_empty())
        .collect();
    let single_folder = tops.len() == 1
        && info.entries.iter().any(|e| e.name.trim_start_matches("./").contains('/') || e.is_dir);
    if single_folder {
        let top = tops.into_iter().next().unwrap();
        if !parent.join(top).exists() {
            // Extract into the parent; the result is parent/top.
            return (parent.clone(), parent.join(top));
        }
    }
    let dest = crate::pack::unique_path(&parent.join(&stem));
    (dest.clone(), dest)
}
