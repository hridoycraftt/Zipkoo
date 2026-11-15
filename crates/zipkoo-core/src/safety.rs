//! Safe Extract: path traversal, zip-bomb and executable checks.
use crate::error::{Error, Result};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Make one path component safe to create on Windows: no reserved device names (CON, NUL,
/// COM1, also with an extension like "nul.txt"), no trailing dots or spaces, no `<>:"|?*`.
pub fn windows_safe(component: &str) -> String {
    let mut s: String = component
        .chars()
        .map(|c| {
            if "<>:\"|?*".contains(c) || (c as u32) < 32 {
                '_'
            } else {
                c
            }
        })
        .collect();
    while s.ends_with('.') || s.ends_with(' ') {
        s.pop();
    }
    if s.is_empty() {
        s.push('_');
    }
    let stem = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    if RESERVED.contains(&stem.trim_end()) {
        s.insert(0, '_');
    }
    s
}

/// Join an untrusted entry name onto `dest`. Rejects `..`, absolute paths and drive prefixes,
/// treating both `/` and `\` as separators. On Windows, names are made creatable.
pub fn safe_join(dest: &Path, entry_name: &str) -> Result<PathBuf> {
    let normalized = entry_name.replace('\\', "/");
    let mut out = dest.to_path_buf();
    let mut pushed = false;
    for c in Path::new(&normalized).components() {
        match c {
            Component::Normal(p) => {
                let s = p.to_string_lossy();
                if cfg!(windows) {
                    // A drive letter ("C:") smuggled in as a component is never safe.
                    if s.len() == 2 && s.ends_with(':') {
                        return Err(Error::UnsafePath(entry_name.into()));
                    }
                    out.push(windows_safe(&s));
                } else {
                    out.push(p);
                }
                pushed = true;
            }
            Component::CurDir => {}
            _ => return Err(Error::UnsafePath(entry_name.into())),
        }
    }
    if !pushed {
        return Err(Error::UnsafePath(entry_name.into()));
    }
    Ok(out)
}

const EXECUTABLE: &[&str] = &[
    "exe", "msi", "bat", "cmd", "com", "scr", "ps1", "vbs", "vbe", "js", "jse", "wsf", "hta",
    "cpl", "lnk", "jar", "app", "command", "sh", "pkg", "dmg", "reg", "dll",
];

/// True when the file could run code when opened.
pub fn is_executable_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let last = lower.rsplit(['/', '\\']).next().unwrap_or(&lower);
    // Double extensions like "invoice.pdf.exe" are caught because we look at the last one.
    last.rsplit_once('.')
        .map(|(_, ext)| EXECUTABLE.contains(&ext))
        .unwrap_or(false)
        || lower.contains(".app/contents/macos/")
}

/// Shared budget for bytes written during one extraction.
pub struct Budget {
    pub limit: u64,
    pub used: u64,
}

impl Budget {
    /// Allow up to `ratio` times the archive size, at least `floor`, and at least what the archive
    /// honestly declares (plus 1 MB slack). Declared sizes are checked against free disk space before
    /// extraction starts, so an archive cannot claim terabytes and fill the disk; one that lies about
    /// its sizes is stopped here when its real output passes the limit.
    pub fn for_archive(
        archive_size: u64,
        ratio: u64,
        floor: u64,
        declared: u64,
        hard_cap: Option<u64>,
    ) -> Self {
        let mut limit = archive_size
            .saturating_mul(ratio)
            .max(floor)
            .max(declared.saturating_add(1 << 20));
        if let Some(cap) = hard_cap {
            limit = limit.min(cap);
        }
        Self { limit, used: 0 }
    }

    pub fn charge(&mut self, n: u64) -> Result<()> {
        self.used += n;
        if self.used > self.limit {
            return Err(Error::TooLarge { limit: self.limit });
        }
        Ok(())
    }
}

/// Copy with the bomb budget enforced on real bytes, not declared sizes.
pub fn guarded_copy(
    r: &mut dyn Read,
    w: &mut dyn Write,
    budget: &mut Budget,
    reporter: &mut crate::progress::Reporter,
) -> Result<u64> {
    let mut buf = vec![0u8; 1 << 16];
    let mut total = 0u64;
    loop {
        let n = match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.to_string().contains("zipkoo: cancelled") => return Err(Error::Cancelled),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        };
        budget.charge(n as u64)?;
        w.write_all(&buf[..n])?;
        reporter.add(n as u64).map_err(|_| Error::Cancelled)?;
        total += n as u64;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_traversal() {
        let d = Path::new("/tmp/out");
        assert!(safe_join(d, "../etc/passwd").is_err());
        assert!(safe_join(d, "/etc/passwd").is_err());
        assert!(safe_join(d, "a/../../b").is_err());
        assert!(safe_join(d, "..\\..\\win.ini").is_err());
        assert_eq!(
            safe_join(d, "a/b.txt").unwrap(),
            Path::new("/tmp/out/a/b.txt")
        );
        assert_eq!(
            safe_join(d, "./a\\b.txt").unwrap(),
            Path::new("/tmp/out/a/b.txt")
        );
    }

    #[test]
    fn windows_names() {
        assert_eq!(windows_safe("CON"), "_CON");
        assert_eq!(windows_safe("nul.txt"), "_nul.txt");
        assert_eq!(windows_safe("com1"), "_com1");
        assert_eq!(windows_safe("notes. "), "notes");
        assert_eq!(windows_safe("a:b?.txt"), "a_b_.txt");
        assert_eq!(windows_safe("console.log"), "console.log");
        assert_eq!(windows_safe("..."), "_");
    }

    #[test]
    fn budget_catches_lying_sizes() {
        // A 1 KB archive that declares 100 bytes but tries to write 2 MB.
        let mut b = Budget::for_archive(1024, 10, 0, 100, None);
        assert!(b.charge(1 << 20).is_ok());
        assert!(matches!(b.charge(1 << 20), Err(Error::TooLarge { .. })));
        // Honest big output is allowed.
        let mut b = Budget::for_archive(1024, 10, 0, 50 << 20, None);
        assert!(b.charge(50 << 20).is_ok());
    }

    #[test]
    fn flags_executables() {
        assert!(is_executable_name("invoice.pdf.exe"));
        assert!(is_executable_name("dir/Setup.MSI"));
        assert!(is_executable_name("Foo.app/Contents/MacOS/Foo"));
        assert!(!is_executable_name("photo.jpg"));
        assert!(!is_executable_name("README"));
    }
}
