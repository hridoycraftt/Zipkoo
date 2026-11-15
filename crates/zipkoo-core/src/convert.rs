//! Batch Convert: re-pack an archive into another format (e.g. RAR → 7z, ZIP → TAR.ZST).
use crate::error::{Error, Result};
use crate::extract::{extract, ExtractOptions};
use crate::format::Format;
use crate::pack::{pack, unique_path, PackOptions, PackReport};
use crate::progress::Progress;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

pub struct ConvertOptions<'a> {
    /// Password of the source archive, if it is locked.
    pub password: Option<&'a str>,
    /// Settings for the new archive (format, level, password, split...).
    pub pack: PackOptions,
}

/// Where a converted copy goes: same folder, same name, new extension, never overwriting.
pub fn converted_path(src: &Path, to: Format) -> PathBuf {
    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = crate::split::strip_part_suffix(&name).to_string();
    let stem = Format::from_name(&name)
        .and_then(|f| {
            name.strip_suffix(&format!(".{}", f.extension()))
                .map(str::to_string)
        })
        .unwrap_or_else(|| {
            Path::new(&name)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or(name.clone())
        });
    unique_path(&src.with_file_name(format!("{stem}.{}", to.extension())))
}

/// Unpack to a temporary folder next to the output (same drive, so no cross-drive copy), then pack.
pub fn convert(
    src: &Path,
    out: &Path,
    opts: &ConvertOptions,
    on_progress: &mut dyn FnMut(&Progress),
    cancel: Option<&AtomicBool>,
) -> Result<PackReport> {
    if !opts.pack.format.can_create() {
        return Err(Error::CreateNotSupported(opts.pack.format.label()));
    }
    let parent = out.parent().unwrap_or(Path::new("."));
    let work = unique_path(&parent.join(".zipkoo-convert"));
    let res = (|| {
        let eo = ExtractOptions {
            password: opts.password,
            ..Default::default()
        };
        extract(src, &work, &eo, on_progress, cancel)?;
        let mut inputs: Vec<PathBuf> = fs::read_dir(&work)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .collect();
        inputs.sort();
        if inputs.is_empty() {
            return Err(Error::NoInputs);
        }
        pack(&inputs, out, &opts.pack, on_progress, cancel)
    })();
    let _ = fs::remove_dir_all(&work);
    res
}
