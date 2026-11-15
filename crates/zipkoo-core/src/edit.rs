//! Edit an archive in place: add files into a folder, delete entries, rename entries.
//!
//! ZIP is edited by copying untouched entries byte-for-byte (no recompression, no decryption).
//! 7z and TAR.ZST are rebuilt: unpacked to a temporary folder on the same drive, changed there,
//! and packed again with the same format and password. Read-only formats (RAR, TAR.GZ…) are
//! refused with a clear message; convert them first. History (`.zipkoo/`) is kept, and repair
//! data is rebuilt so Self-heal stays valid.
use crate::error::{Error, Result};
use crate::extract::{extract, ExtractOptions};
use crate::format::Format;
use crate::meta::zip_time;
use crate::pack::{pack, unique_path, PackOptions};
use crate::progress::{CountingReader, Progress, Reporter};
use crate::smart::is_precompressed;
use serde::Serialize;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Change {
    /// Add files/folders under `folder` inside the archive ("" = top level, else "a/b/").
    Add {
        inputs: Vec<PathBuf>,
        folder: String,
        replace: bool,
    },
    /// Delete entries; a name ending in "/" deletes the whole folder.
    Delete { names: Vec<String> },
    /// Rename a file, or a folder when `from` ends in "/".
    Rename { from: String, to: String },
}

#[derive(Debug, Default, Serialize)]
pub struct EditReport {
    pub added: usize,
    pub replaced: usize,
    pub deleted: usize,
    pub renamed: usize,
    pub skipped: Vec<String>,
}

fn norm_folder(f: &str) -> String {
    let f = f.trim_matches('/').replace('\\', "/");
    if f.is_empty() {
        String::new()
    } else {
        format!("{f}/")
    }
}

fn valid_name(n: &str) -> bool {
    !n.is_empty()
        && !n.split('/').any(|c| c == ".." || c == ".")
        && !n.starts_with('/')
        && !n.contains('\\')
        && !n.starts_with(crate::update::ZIPKOO_DIR)
}

/// Archive-relative names for the files under `inputs`, placed in `folder`.
fn plan_inputs(inputs: &[PathBuf], folder: &str) -> Result<Vec<(String, Option<PathBuf>)>> {
    let mut v = Vec::new();
    for input in inputs {
        let base = input.parent().unwrap_or(Path::new("")).to_path_buf();
        for e in walkdir::WalkDir::new(input)
            .follow_links(false)
            .sort_by_file_name()
        {
            let e = e.map_err(|e| Error::Io(e.into()))?;
            if e.file_type().is_symlink() {
                continue;
            }
            let rel = e
                .path()
                .strip_prefix(&base)
                .unwrap_or(e.path())
                .to_string_lossy()
                .replace('\\', "/");
            if rel.is_empty() {
                continue;
            }
            if e.file_type().is_dir() {
                v.push((format!("{folder}{rel}/"), None));
            } else {
                v.push((format!("{folder}{rel}"), Some(e.path().to_path_buf())));
            }
        }
    }
    Ok(v)
}

fn renamed(name: &str, from: &str, to: &str) -> Option<String> {
    if from.ends_with('/') {
        name.strip_prefix(from)
            .map(|rest| format!("{}{rest}", norm_folder(to)))
    } else if name == from {
        Some(to.to_string())
    } else {
        None
    }
}

fn deleted(name: &str, names: &[String]) -> bool {
    names.iter().any(|n| {
        if n.ends_with('/') {
            name.starts_with(n.as_str())
        } else {
            name == n
        }
    })
}

pub fn edit(
    archive: &Path,
    change: &Change,
    password: Option<&str>,
    on_progress: &mut dyn FnMut(&Progress),
    cancel: Option<&AtomicBool>,
) -> Result<EditReport> {
    if let Change::Rename { to, .. } = change {
        if !valid_name(to.trim_end_matches('/')) {
            return Err(Error::UnsafePath(to.clone()));
        }
    }
    let info = crate::inspect::inspect(archive, password)?;
    if info.encrypted && password.is_none() {
        return Err(Error::PasswordRequired);
    }
    let report = match info.format {
        Format::Zip => edit_zip(archive, change, password, &info, on_progress, cancel)?,
        Format::SevenZ | Format::TarZst => edit_by_rebuild(
            archive,
            info.format,
            change,
            password,
            &info,
            on_progress,
            cancel,
        )?,
        other => return Err(Error::CreateNotSupported(other.label())),
    };
    // Repair data described the old bytes.
    let rec = crate::heal::recovery_path(archive);
    if rec.exists() {
        let pct = crate::heal::percent_of(&rec).unwrap_or(10);
        let _ = fs::remove_file(&rec);
        crate::heal::create(archive, pct, &mut |_| {}, None)?;
    }
    Ok(report)
}

fn edit_zip(
    archive: &Path,
    change: &Change,
    password: Option<&str>,
    info: &crate::inspect::ArchiveInfo,
    on_progress: &mut dyn FnMut(&Progress),
    cancel: Option<&AtomicBool>,
) -> Result<EditReport> {
    use zip::write::SimpleFileOptions;
    let mut r = EditReport::default();
    let mut old = crate::inspect::open_zip(archive)?;
    let existing: std::collections::BTreeSet<String> =
        info.entries.iter().map(|e| e.name.clone()).collect();
    let adds = match change {
        Change::Add { inputs, folder, .. } => plan_inputs(inputs, &norm_folder(folder))?,
        _ => Vec::new(),
    };
    if let Change::Rename { from, to } = change {
        let target_exists = existing.iter().any(|n| {
            renamed(n, from, to)
                .map(|t| existing.contains(&t) && &t != n)
                .unwrap_or(false)
        });
        if target_exists {
            return Err(Error::UnsafePath(format!(
                "“{to}” already exists in the archive"
            )));
        }
    }
    let total: u64 = adds
        .iter()
        .filter_map(|(_, p)| p.as_ref())
        .filter_map(|p| fs::metadata(p).ok())
        .map(|m| m.len())
        .sum();
    let mut rep = Reporter::new(on_progress, cancel, total);
    let tmp = archive.with_extension("zip.part");
    let res = (|| -> Result<()> {
        let mut zw = zip::ZipWriter::new(BufWriter::new(File::create(&tmp)?));
        let replace = matches!(change, Change::Add { replace: true, .. });
        let adding: std::collections::BTreeSet<&str> =
            adds.iter().map(|(n, _)| n.as_str()).collect();
        for i in 0..old.len() {
            let name = crate::names::zip_name(&old.by_index_raw(i)?);
            match change {
                Change::Delete { names }
                    if deleted(&name, names) && !name.starts_with(crate::update::ZIPKOO_DIR) =>
                {
                    r.deleted += 1;
                    continue;
                }
                Change::Rename { from, to } if !name.starts_with(crate::update::ZIPKOO_DIR) => {
                    if let Some(new) = renamed(&name, from, to) {
                        zw.raw_copy_file_rename(old.by_index_raw(i)?, new)?;
                        r.renamed += 1;
                        continue;
                    }
                }
                Change::Add { .. } if adding.contains(name.as_str()) => {
                    if name.ends_with('/') {
                        // Folder already there: keep the existing entry, don't add it twice.
                    } else if replace {
                        r.replaced += 1;
                        continue;
                    } else {
                        r.skipped.push(name.clone());
                    }
                }
                _ => {}
            }
            zw.raw_copy_file(old.by_index_raw(i)?)?;
        }
        for (name, src) in &adds {
            let exists = existing.contains(name);
            if exists && (name.ends_with('/') || !replace) {
                continue;
            }
            let mut o = SimpleFileOptions::default();
            if let Some(pw) = password.filter(|_| info.encrypted) {
                o = o.with_aes_encryption(zip::AesMode::Aes256, pw);
            }
            match src {
                None => zw.add_directory(name.as_str(), o)?,
                Some(p) => {
                    let meta = fs::metadata(p)?;
                    o = o
                        .large_file(meta.len() >= u32::MAX as u64)
                        .compression_method(if is_precompressed(p) {
                            zip::CompressionMethod::Stored
                        } else {
                            zip::CompressionMethod::Deflated
                        });
                    if let Some(t) = meta.modified().ok().and_then(zip_time) {
                        o = o.last_modified_time(t);
                    }
                    rep.set_current(name);
                    zw.start_file(name.as_str(), o)?;
                    let mut rd = CountingReader {
                        inner: BufReader::new(File::open(p)?),
                        reporter: &mut rep,
                    };
                    std::io::copy(&mut rd, &mut zw)?;
                    if !exists {
                        r.added += 1;
                    }
                }
            }
        }
        zw.finish()?.flush()?;
        Ok(())
    })();
    drop(old);
    match res {
        Ok(()) => {
            fs::rename(&tmp, archive)?;
            rep.finish();
            Ok(r)
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(match e {
                Error::Io(io) if io.to_string().contains("zipkoo: cancelled") => Error::Cancelled,
                other => other,
            })
        }
    }
}

fn edit_by_rebuild(
    archive: &Path,
    format: Format,
    change: &Change,
    password: Option<&str>,
    info: &crate::inspect::ArchiveInfo,
    on_progress: &mut dyn FnMut(&Progress),
    cancel: Option<&AtomicBool>,
) -> Result<EditReport> {
    let parent = archive.parent().unwrap_or(Path::new("."));
    let work = unique_path(&parent.join(".zipkoo-edit"));
    let mut r = EditReport::default();
    let res = (|| -> Result<PathBuf> {
        let eo = ExtractOptions {
            password,
            ..Default::default()
        };
        extract(archive, &work, &eo, on_progress, cancel)?;
        match change {
            Change::Delete { names } => {
                for n in names {
                    let p = crate::safety::safe_join(&work, n.trim_end_matches('/'))?;
                    if p.is_dir() {
                        fs::remove_dir_all(&p)?;
                        r.deleted += info
                            .entries
                            .iter()
                            .filter(|e| e.name.starts_with(n.as_str()))
                            .count();
                    } else if p.exists() {
                        fs::remove_file(&p)?;
                        r.deleted += 1;
                    }
                }
            }
            Change::Rename { from, to } => {
                let a = crate::safety::safe_join(&work, from.trim_end_matches('/'))?;
                let b = crate::safety::safe_join(&work, to.trim_end_matches('/'))?;
                if b.exists() {
                    return Err(Error::UnsafePath(format!(
                        "“{to}” already exists in the archive"
                    )));
                }
                if let Some(p) = b.parent() {
                    fs::create_dir_all(p)?;
                }
                fs::rename(&a, &b)?;
                r.renamed += 1;
            }
            Change::Add {
                inputs,
                folder,
                replace,
            } => {
                let dest = if norm_folder(folder).is_empty() {
                    work.clone()
                } else {
                    crate::safety::safe_join(&work, folder.trim_end_matches('/'))?
                };
                fs::create_dir_all(&dest)?;
                for (name, src) in plan_inputs(inputs, "")? {
                    let out = crate::safety::safe_join(&dest, name.trim_end_matches('/'))?;
                    match src {
                        None => fs::create_dir_all(&out)?,
                        Some(p) => {
                            if out.exists() && !replace {
                                r.skipped.push(name);
                                continue;
                            }
                            if out.exists() {
                                r.replaced += 1;
                            } else {
                                r.added += 1;
                            }
                            if let Some(pp) = out.parent() {
                                fs::create_dir_all(pp)?;
                            }
                            fs::copy(&p, &out)?;
                        }
                    }
                }
            }
        }
        let mut inputs: Vec<PathBuf> = fs::read_dir(&work)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .collect();
        inputs.sort();
        // 7z with hidden names can't be listed without the password.
        let hide_names = format == Format::SevenZ
            && password.is_some()
            && crate::inspect::inspect(archive, None).is_err();
        let po = PackOptions {
            format,
            password: password.filter(|_| info.encrypted).map(String::from),
            hide_names,
            ..Default::default()
        };
        let out = archive.with_extension(format!("{}.new", format.extension().replace('.', "-")));
        if inputs.is_empty() {
            // Everything was deleted: write an empty archive of the same kind.
            let empty = work.join(".zipkoo-empty");
            fs::create_dir_all(&empty)?;
            pack(&[empty.clone()], &out, &po, &mut |_| {}, cancel)?;
        } else {
            pack(&inputs, &out, &po, on_progress, cancel)?;
        }
        Ok(out)
    })();
    let _ = fs::remove_dir_all(&work);
    let out = res?;
    fs::rename(&out, archive)?;
    Ok(r)
}
