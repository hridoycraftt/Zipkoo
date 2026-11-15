//! Virus scan using the scanner already on the computer: Microsoft Defender on Windows
//! (MpCmdRun.exe), ClamAV (clamscan) elsewhere. Nothing is bundled or downloaded.
use crate::error::{Error, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct ScanResult {
    /// "Microsoft Defender" or "ClamAV".
    pub engine: String,
    pub clean: bool,
    /// What the scanner reported, e.g. "invoice.exe: Win.Trojan.Agent".
    pub threats: Vec<String>,
}

#[derive(Debug, Clone)]
enum Engine {
    Defender(PathBuf),
    Clam(PathBuf),
}

fn defender() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    // The newest platform copy is preferred; Program Files has a stable fallback.
    let mut candidates = Vec::new();
    if let Some(pd) = std::env::var_os("ProgramData") {
        let plat = PathBuf::from(pd).join("Microsoft/Windows Defender/Platform");
        if let Ok(rd) = std::fs::read_dir(&plat) {
            let mut dirs: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
            dirs.sort();
            candidates.extend(dirs.into_iter().rev().map(|d| d.join("MpCmdRun.exe")));
        }
    }
    if let Some(pf) = std::env::var_os("ProgramFiles") {
        candidates.push(PathBuf::from(pf).join("Windows Defender/MpCmdRun.exe"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

fn clamscan() -> Option<PathBuf> {
    // ZIPKOO_CLAMSCAN lets tests and admins point at a specific clamscan-compatible binary.
    if let Some(p) = std::env::var_os("ZIPKOO_CLAMSCAN")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
    {
        return Some(p);
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.extend(
        ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"]
            .iter()
            .map(PathBuf::from),
    );
    let exe = if cfg!(windows) {
        "clamscan.exe"
    } else {
        "clamscan"
    };
    dirs.into_iter().map(|d| d.join(exe)).find(|p| p.is_file())
}

fn engine() -> Option<Engine> {
    defender()
        .map(Engine::Defender)
        .or_else(|| clamscan().map(Engine::Clam))
}

/// Name of the scanner that would be used, if any.
pub fn available() -> Option<String> {
    engine().map(|e| match e {
        Engine::Defender(_) => "Microsoft Defender".to_string(),
        Engine::Clam(_) => "ClamAV".to_string(),
    })
}

/// Scan a file or folder. Errors when no scanner is installed or it fails to run.
pub fn scan(path: &Path) -> Result<ScanResult> {
    let not_found = || {
        Error::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "No virus scanner found. Windows has Microsoft Defender built in; on Mac or Linux install ClamAV."))
    };
    match engine().ok_or_else(not_found)? {
        Engine::Defender(exe) => {
            let mut cmd = Command::new(exe);
            cmd.args(["-Scan", "-ScanType", "3", "-File"])
                .arg(path)
                .arg("-DisableRemediation");
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x0800_0000); // no console window
            }
            let out = cmd.output()?;
            let text = String::from_utf8_lossy(&out.stdout);
            let threats: Vec<String> = text
                .lines()
                .filter(|l| l.contains("Threat ") || l.trim_start().starts_with("Threat"))
                .map(|l| l.trim().to_string())
                .collect();
            match out.status.code() {
                Some(0) => Ok(ScanResult {
                    engine: "Microsoft Defender".into(),
                    clean: true,
                    threats: vec![],
                }),
                Some(2) => Ok(ScanResult {
                    engine: "Microsoft Defender".into(),
                    clean: false,
                    threats: if threats.is_empty() {
                        vec!["Threat found".into()]
                    } else {
                        threats
                    },
                }),
                c => Err(Error::Io(std::io::Error::other(format!(
                    "Microsoft Defender couldn't scan (code {c:?}): {}",
                    text.lines().last().unwrap_or("")
                )))),
            }
        }
        Engine::Clam(exe) => {
            let out = Command::new(exe)
                .args(["--no-summary", "--infected", "-r"])
                .arg(path)
                .output()?;
            let text = String::from_utf8_lossy(&out.stdout);
            let threats: Vec<String> = text
                .lines()
                .filter(|l| l.ends_with(" FOUND"))
                .map(|l| {
                    let l = l.trim_end_matches(" FOUND");
                    match l.rsplit_once(": ") {
                        Some((file, name)) => format!(
                            "{}: {name}",
                            Path::new(file)
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| file.to_string())
                        ),
                        None => l.to_string(),
                    }
                })
                .collect();
            match out.status.code() {
                Some(0) => Ok(ScanResult {
                    engine: "ClamAV".into(),
                    clean: true,
                    threats: vec![],
                }),
                Some(1) => Ok(ScanResult {
                    engine: "ClamAV".into(),
                    clean: false,
                    threats,
                }),
                c => Err(Error::Io(std::io::Error::other(format!(
                    "ClamAV couldn't scan (code {c:?}): {}",
                    String::from_utf8_lossy(&out.stderr)
                        .lines()
                        .next()
                        .unwrap_or("")
                )))),
            }
        }
    }
}
