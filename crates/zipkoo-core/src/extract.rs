//! Safe Extract for every supported format.
use crate::error::{Error, Result};
use crate::format::Format;
use crate::inspect::{resolve, tar_reader};
use crate::progress::{Progress, Reporter};
use crate::safety::{guarded_copy, is_executable_name, safe_join, Budget};
use serde::Serialize;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
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
    /// Remove this leading folder (e.g. "Project/") from every entry name, so a single top folder
    /// can be unpacked straight into a new folder instead of nesting it one level deeper.
    pub strip: Option<String>,
    /// What to do when a file already exists at the destination.
    pub on_conflict: Conflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Conflict {
    /// Leave the existing file and report the entry as skipped.
    #[default]
    Skip,
    Overwrite,
    /// Keep both: the new file gets " (2)", " (3)"...
    Rename,
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
            strip: None,
            on_conflict: Conflict::Skip,
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
    /// Windows "Mark of the Web" copied from a downloaded archive to every file it produces.
    motw: Option<Vec<u8>>,
    dest: &'o Path,
    opts: &'o ExtractOptions<'o>,
    budget: Budget,
    report: ExtractReport,
    rep: &'r mut Reporter<'a>,
}

impl Ctx<'_, '_, '_> {
    fn wanted(&self, name: &str) -> bool {
        // Smart Update's history stays inside the archive unless asked for by name.
        if name.starts_with(crate::update::ZIPKOO_DIR) {
            return self.opts.only.iter().any(|o| name.starts_with(o.as_str()));
        }
        self.opts.only.is_empty() || self.opts.only.iter().any(|o| o == name || name.starts_with(&format!("{}/", o.trim_end_matches('/'))))
    }

    /// Decide where an entry goes, or why it is skipped.
    fn target(&mut self, name: &str, is_dir: bool) -> Result<Option<PathBuf>> {
        if !self.wanted(name) {
            return Ok(None);
        }
        let stripped;
        let name = match &self.opts.strip {
            Some(prefix) => {
                let n = name.trim_start_matches("./");
                match n.strip_prefix(prefix.as_str()) {
                    Some(rest) if !rest.is_empty() => {
                        stripped = rest.to_string();
                        stripped.as_str()
                    }
                    // The top folder itself: just make sure the destination exists.
                    _ if n.trim_end_matches('/') == prefix.trim_end_matches('/') => {
                        fs::create_dir_all(self.dest)?;
                        return Ok(None);
                    }
                    _ => name,
                }
            }
            None => name,
        };
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
        let out = if out.exists() {
            match (self.opts.overwrite, self.opts.on_conflict) {
                (true, _) | (_, Conflict::Overwrite) => out,
                (_, Conflict::Rename) => crate::pack::unique_path(&out),
                (_, Conflict::Skip) => {
                    self.report.skipped.push((name.into(), "already exists".into()));
                    return Ok(None);
                }
            }
        } else {
            out
        };
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
                drop(w);
                if let Some(zone) = &self.motw {
                    crate::meta::motw_write(out, zone);
                }
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
    let (real, format, archive_size, _guard) = resolve(path)?;
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
        motw: crate::meta::motw_read(path),
        dest,
        opts,
        budget: Budget::for_archive(archive_size, opts.max_ratio, opts.floor, declared, opts.hard_cap),
        report: ExtractReport { dest: dest.to_path_buf(), ..Default::default() },
        rep: &mut rep,
    };

    match format {
        Format::Zip => {
            let mut z = crate::inspect::open_zip(&real)?;
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
                let mtime = f.last_modified().and_then(crate::meta::from_zip_time);
                // A wrong AES password fails when the data is read, so map that here.
                match ctx.write(&out, &mut f) {
                    Err(Error::Io(e)) if encrypted && e.to_string().to_lowercase().contains("password") => {
                        return Err(Error::WrongPassword)
                    }
                    other => other?,
                }
                drop(f);
                if let Some(t) = mtime {
                    crate::meta::set_mtime(&out, t);
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
                    ctx.write(&out, reader)?;
                    if entry.has_last_modified_date {
                        crate::meta::set_mtime(&out, entry.last_modified_date.into());
                    }
                    Ok(())
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
                if let Ok(m) = e.header().mtime() {
                    crate::meta::set_mtime(&out, std::time::UNIX_EPOCH + std::time::Duration::from_secs(m));
                }
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
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ExtractTarget {
    /// Folder to extract into.
    pub dest: PathBuf,
    /// Folder the user sees afterwards.
    pub result: PathBuf,
    /// Leading folder to drop from entry names (see `ExtractOptions::strip`).
    pub strip: Option<String>,
}

pub fn suggest_dest(archive: &Path, info: &crate::inspect::ArchiveInfo) -> ExtractTarget {
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
        .filter(|e| !e.name.starts_with(crate::update::ZIPKOO_DIR))
        .map(|e| e.name.trim_start_matches("./").split('/').next().unwrap_or(""))
        .filter(|s| !s.is_empty())
        .collect();
    let single_folder = tops.len() == 1
        && info.entries.iter().any(|e| e.name.trim_start_matches("./").contains('/') || e.is_dir);
    if single_folder {
        let top = tops.into_iter().next().unwrap();
        let wanted = parent.join(top);
        if !wanted.exists() {
            // Extract into the parent; the result is parent/top.
            return ExtractTarget { dest: parent.clone(), result: wanted, strip: None };
        }
        // "Project" is taken: unpack its contents into "Project (2)" (not "Project (2)/Project").
        let dest = crate::pack::unique_path(&wanted);
        return ExtractTarget { dest: dest.clone(), result: dest, strip: Some(format!("{top}/")) };
    }
    let dest = crate::pack::unique_path(&parent.join(&stem));
    ExtractTarget { dest: dest.clone(), result: dest, strip: None }
}

/// Files this extraction would collide with (existing files at the destination), so the UI can
/// ask once: skip, keep both, or replace. Uses the same naming rules as `extract`.
pub fn conflicts(path: &Path, dest: &Path, opts: &ExtractOptions) -> Result<Vec<String>> {
    let info = crate::inspect::inspect(path, opts.password)?;
    let mut out = Vec::new();
    for e in info.entries.iter().filter(|e| !e.is_dir) {
        let name = e.name.trim_start_matches("./");
        if name.starts_with(crate::update::ZIPKOO_DIR) && opts.only.is_empty() {
            continue;
        }
        if !opts.only.is_empty() && !opts.only.iter().any(|o| name == o || name.starts_with(&format!("{}/", o.trim_end_matches('/')))) {
            continue;
        }
        let rel = match &opts.strip {
            Some(p) => match name.strip_prefix(p.as_str()) {
                Some(r) if !r.is_empty() => r,
                _ => continue,
            },
            None => name,
        };
        if let Ok(target) = safe_join(dest, rel) {
            if target.is_file() {
                out.push(rel.to_string());
            }
        }
    }
    Ok(out)
}

#[derive(Debug, Default, Serialize)]
pub struct TestReport {
    pub files: usize,
    pub bytes: u64,
    /// Entries that failed their checksum or couldn't be read, with the reason.
    pub errors: Vec<(String, String)>,
}

impl TestReport {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Read every file in the archive and check its checksum (CRC-32 for ZIP/7z/RAR, the zstd/xz
/// frame checks for compressed TARs) without writing anything.
pub fn test(path: &Path, password: Option<&str>, on_progress: &mut dyn FnMut(&Progress), cancel: Option<&AtomicBool>) -> Result<TestReport> {
    let (real, format, _, _guard) = resolve(path)?;
    let total = crate::inspect::inspect(path, password).map(|i| i.total_size).unwrap_or(0);
    let mut rep = Reporter::new(on_progress, cancel, total);
    let mut r = TestReport::default();
    let sink = |rd: &mut dyn std::io::Read, rep: &mut Reporter| -> std::io::Result<u64> {
        let mut buf = vec![0u8; 1 << 16];
        let mut n = 0u64;
        loop {
            let k = rd.read(&mut buf)?;
            if k == 0 {
                return Ok(n);
            }
            n += k as u64;
            rep.add(k as u64)?;
        }
    };
    match format {
        Format::Zip => {
            let mut z = crate::inspect::open_zip(&real)?;
            for i in 0..z.len() {
                let (name, is_dir, enc) = {
                    let f = z.by_index_raw(i)?;
                    (f.name()?.to_string(), f.is_dir(), f.encrypted())
                };
                if is_dir {
                    continue;
                }
                if enc && password.is_none() {
                    return Err(Error::PasswordRequired);
                }
                rep.set_current(&name);
                let res = match (enc, password) {
                    (true, Some(pw)) => z.by_index_decrypt(i, pw.as_bytes()),
                    _ => z.by_index(i),
                };
                match res {
                    Ok(mut f) => match sink(&mut f, &mut rep) {
                        Ok(n) => {
                            r.files += 1;
                            r.bytes += n;
                        }
                        Err(e) if e.to_string().contains("zipkoo: cancelled") => return Err(Error::Cancelled),
                        Err(e) => r.errors.push((name, e.to_string())),
                    },
                    Err(zip::result::ZipError::InvalidPassword) => return Err(Error::WrongPassword),
                    Err(e) => r.errors.push((name, e.to_string())),
                }
            }
        }
        Format::SevenZ => {
            let pw = password.map(sevenz_rust2::Password::from).unwrap_or_else(sevenz_rust2::Password::empty);
            let mut ar = sevenz_rust2::ArchiveReader::open(&real, pw)?;
            let mut errors = Vec::new();
            let res = ar.for_each_entries(|entry, reader| {
                if entry.is_directory {
                    return Ok(true);
                }
                rep.set_current(&entry.name);
                match sink(reader, &mut rep) {
                    Ok(n) => {
                        r.files += 1;
                        r.bytes += n;
                        Ok(true)
                    }
                    Err(e) => {
                        errors.push((entry.name.clone(), e.to_string()));
                        // A broken solid block can't be read past; stop here.
                        Ok(false)
                    }
                }
            });
            if let Err(e) = res {
                let e: Error = e.into();
                match e {
                    Error::PasswordRequired | Error::WrongPassword | Error::Cancelled => return Err(e),
                    other => errors.push(("(archive)".into(), other.to_string())),
                }
            }
            if errors.iter().any(|(_, m)| m.contains("zipkoo: cancelled")) {
                return Err(Error::Cancelled);
            }
            r.errors.extend(errors);
        }
        Format::Rar => {
            let mut a = match password {
                Some(pw) => unrar::Archive::with_password(&real, pw),
                None => unrar::Archive::new(&real),
            }
            .open_for_processing()?;
            while let Some(h) = a.read_header()? {
                let name = h.entry().filename.to_string_lossy().replace('\\', "/");
                let size = h.entry().unpacked_size;
                let is_file = h.entry().is_file();
                rep.set_current(&name);
                a = if is_file {
                    match h.test() {
                        Ok(next) => {
                            r.files += 1;
                            r.bytes += size;
                            rep.add(size).map_err(|_| Error::Cancelled)?;
                            next
                        }
                        Err(e) => {
                            let e: Error = e.into();
                            if matches!(e, Error::WrongPassword | Error::PasswordRequired) {
                                return Err(e);
                            }
                            r.errors.push((name, e.to_string()));
                            break;
                        }
                    }
                } else {
                    h.skip()?
                };
            }
        }
        _ => {
            let mut t = tar::Archive::new(crate::inspect::tar_reader(&real, format)?);
            {
                let entries = match t.entries() {
                    Ok(e) => e,
                    Err(e) => {
                        r.errors.push(("(archive)".into(), e.to_string()));
                        return Ok(r);
                    }
                };
                for e in entries {
                    let mut e = match e {
                        Ok(e) => e,
                        Err(err) => {
                            r.errors.push(("(archive)".into(), err.to_string()));
                            break;
                        }
                    };
                    if !e.header().entry_type().is_file() {
                        continue;
                    }
                    let name = e.path().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
                    rep.set_current(&name);
                    match sink(&mut e, &mut rep) {
                        Ok(n) => {
                            r.files += 1;
                            r.bytes += n;
                        }
                        Err(err) if err.to_string().contains("zipkoo: cancelled") => return Err(Error::Cancelled),
                        Err(err) => {
                            r.errors.push((name, err.to_string()));
                            break;
                        }
                    }
                }
            }
            // The compressed stream's own checksum (zstd/xz/gzip) is only checked at its end,
            // past the last file, so read whatever is left.
            if r.errors.is_empty() {
                if let Err(err) = std::io::copy(&mut t.into_inner(), &mut std::io::sink()) {
                    r.errors.push(("(archive)".into(), err.to_string()));
                }
            }
        }
    }
    rep.finish();
    Ok(r)
}
