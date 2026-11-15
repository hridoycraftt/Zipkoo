//! Zipkoo self-extracting stub. Reads the ZIP payload appended to its own file and unpacks it.
//!
//! Double-click: asks where to unpack (and the password if needed), then opens the folder.
//! Scripted: `Name.exe --to <folder> [--password <pw>] [--overwrite] [--silent]`.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use tinyfiledialogs as dlg;

const MAGIC: &[u8; 8] = b"ZKSFX\x01\0\0";
const TITLE: &str = "Zipkoo";

struct Window<R> {
    inner: R,
    start: u64,
    len: u64,
    pos: u64,
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
        let t = match to {
            SeekFrom::Start(p) => p as i128,
            SeekFrom::End(d) => self.len as i128 + d as i128,
            SeekFrom::Current(d) => self.pos as i128 + d as i128,
        };
        if t < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek before start",
            ));
        }
        self.pos = t as u64;
        self.inner.seek(SeekFrom::Start(self.start + self.pos))?;
        Ok(self.pos)
    }
}

fn payload() -> io::Result<Window<File>> {
    let me = std::env::current_exe()?;
    let mut f = File::open(me)?;
    let len = f.metadata()?.len();
    let bad = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "This file is damaged or incomplete. Download it again.",
        )
    };
    if len < 16 {
        return Err(bad());
    }
    f.seek(SeekFrom::End(-16))?;
    let mut t = [0u8; 16];
    f.read_exact(&mut t)?;
    if &t[8..] != MAGIC {
        return Err(bad());
    }
    let start = u64::from_le_bytes(t[..8].try_into().unwrap());
    if start >= len - 16 {
        return Err(bad());
    }
    f.seek(SeekFrom::Start(start))?;
    Ok(Window {
        inner: f,
        start,
        len: len - 16 - start,
        pos: 0,
    })
}

/// Join an untrusted entry name onto `dest`, refusing anything that escapes it.
fn safe_join(dest: &Path, name: &str) -> Option<PathBuf> {
    let norm = name.replace('\\', "/");
    let mut out = dest.to_path_buf();
    let mut any = false;
    for c in Path::new(&norm).components() {
        match c {
            Component::Normal(p) => {
                if cfg!(windows) && p.to_string_lossy().contains(':') {
                    return None;
                }
                out.push(p);
                any = true;
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    any.then_some(out)
}

struct Args {
    to: Option<PathBuf>,
    password: Option<String>,
    overwrite: bool,
    silent: bool,
}

fn args() -> Args {
    let mut a = Args {
        to: None,
        password: None,
        overwrite: false,
        silent: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(x) = it.next() {
        match x.as_str() {
            "--to" => a.to = it.next().map(PathBuf::from),
            "--password" => a.password = it.next(),
            "--overwrite" => a.overwrite = true,
            "--silent" => a.silent = true,
            _ => {}
        }
    }
    a
}

fn human(n: u64) -> String {
    let u = ["B", "KB", "MB", "GB", "TB"];
    let (mut v, mut i) = (n as f64, 0);
    while v >= 1024.0 && i < 4 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", u[i])
    }
}

fn run(a: &Args) -> Result<(PathBuf, usize), String> {
    let mut zip = zip::ZipArchive::new(payload().map_err(|e| e.to_string())?)
        .map_err(|e| format!("This file is damaged: {e}"))?;
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let name = me
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Archive".into());
    let total: u64 = (0..zip.len())
        .filter_map(|i| zip.by_index_raw(i).ok().map(|f| f.size()))
        .sum();
    let encrypted =
        (0..zip.len()).any(|i| zip.by_index_raw(i).map(|f| f.encrypted()).unwrap_or(false));

    let dest = match &a.to {
        Some(d) => d.clone(),
        None if a.silent => me.parent().unwrap_or(Path::new(".")).join(&name),
        None => {
            let start = me
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            let msg = format!(
                "{name} holds {} files ({}). Choose where to unpack them.",
                zip.len(),
                human(total)
            );
            dlg::message_box_ok(TITLE, &msg, dlg::MessageBoxIcon::Info);
            let picked = dlg::select_folder_dialog("Unpack to", &start)
                .ok_or_else(|| "cancelled".to_string())?;
            PathBuf::from(picked).join(&name)
        }
    };

    let mut password = a.password.clone();
    if encrypted && password.is_none() {
        if a.silent {
            return Err("This archive is locked. Pass --password.".into());
        }
        password = dlg::password_box(TITLE, "This archive is locked. Enter its password.");
        if password.is_none() {
            return Err("cancelled".into());
        }
    }

    fs::create_dir_all(&dest).map_err(|e| format!("Can't create {}: {e}", dest.display()))?;
    let mut files = 0;
    let mut buf = vec![0u8; 1 << 16];
    for i in 0..zip.len() {
        let (entry_name, is_dir, is_link) = {
            let f = zip.by_index_raw(i).map_err(|e| e.to_string())?;
            (
                f.name().map(|n| n.to_string()).unwrap_or_default(),
                f.is_dir(),
                f.is_symlink(),
            )
        };
        let Some(out) = safe_join(&dest, &entry_name) else {
            continue;
        };
        if is_link {
            continue;
        }
        if is_dir {
            fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        if out.exists() && !a.overwrite {
            continue;
        }
        if let Some(p) = out.parent() {
            fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        let mut f = match &password {
            Some(pw) => zip.by_index_decrypt(i, pw.as_bytes()),
            None => zip.by_index(i),
        }
        .map_err(|e| match e {
            zip::result::ZipError::InvalidPassword => {
                "Wrong password. Check Caps Lock and try again.".to_string()
            }
            other => other.to_string(),
        })?;
        let mut w = BufWriter::new(File::create(&out).map_err(|e| e.to_string())?);
        let res = (|| -> io::Result<()> {
            loop {
                let n = f.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                w.write_all(&buf[..n])?;
            }
            w.flush()
        })();
        if let Err(e) = res {
            drop(w);
            let _ = fs::remove_file(&out);
            let lower = e.to_string().to_lowercase();
            return Err(if lower.contains("password") {
                "Wrong password. Check Caps Lock and try again.".into()
            } else {
                e.to_string()
            });
        }
        #[cfg(unix)]
        if let Some(mode) = f.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&out, fs::Permissions::from_mode(mode & 0o755));
        }
        files += 1;
    }
    Ok((dest, files))
}

fn reveal(dir: &Path) {
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(dir).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(dir).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
}

fn main() {
    let a = args();
    match run(&a) {
        Ok((dest, n)) => {
            if a.silent {
                println!("{n} files -> {}", dest.display());
            } else {
                dlg::message_box_ok(
                    TITLE,
                    &format!("Done. {n} files are in {}.", dest.display()),
                    dlg::MessageBoxIcon::Info,
                );
                reveal(&dest);
            }
        }
        Err(e) if e == "cancelled" => {}
        Err(e) => {
            if a.silent {
                eprintln!("{e}");
            } else {
                dlg::message_box_ok(TITLE, &e, dlg::MessageBoxIcon::Error);
            }
            std::process::exit(1);
        }
    }
}
