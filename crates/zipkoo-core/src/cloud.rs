//! Cloud folders: find the Google Drive, Dropbox, OneDrive and iCloud folders that their desktop
//! apps keep on this computer. Zipkoo opens and saves archives there like any other folder; the
//! provider's app syncs them. (Direct API access would need an OAuth app per provider.)
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CloudFolder {
    pub provider: &'static str,
    /// Name shown to people, e.g. "Google Drive (rina@…)".
    pub label: String,
    pub path: PathBuf,
}

fn push(v: &mut Vec<CloudFolder>, provider: &'static str, label: String, path: PathBuf) {
    if path.is_dir() && !v.iter().any(|c| c.path == path) {
        v.push(CloudFolder { provider, label, path });
    }
}

/// Dropbox writes its folders to info.json: {"personal": {"path": ...}, "business": {...}}.
fn dropbox_info(file: &Path, v: &mut Vec<CloudFolder>) {
    let Ok(s) = std::fs::read_to_string(file) else { return };
    let Ok(j) = serde_json::from_str::<serde_json::Value>(&s) else { return };
    for (kind, label) in [("personal", "Dropbox"), ("business", "Dropbox (Business)")] {
        if let Some(p) = j[kind]["path"].as_str() {
            push(v, "dropbox", label.into(), PathBuf::from(p));
        }
    }
}

pub fn detect_in(home: &Path, env: &dyn Fn(&str) -> Option<String>) -> Vec<CloudFolder> {
    let mut v = Vec::new();
    // macOS File Provider locations: ~/Library/CloudStorage/GoogleDrive-<account>, OneDrive-<org>, Dropbox…
    if let Ok(rd) = std::fs::read_dir(home.join("Library/CloudStorage")) {
        let mut found: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        found.sort();
        for p in found {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let (provider, base) = if name.starts_with("GoogleDrive") {
                ("google-drive", "Google Drive")
            } else if name.starts_with("OneDrive") {
                ("onedrive", "OneDrive")
            } else if name.starts_with("Dropbox") {
                ("dropbox", "Dropbox")
            } else if name.starts_with("Box") {
                ("box", "Box")
            } else {
                continue;
            };
            let account = name.split_once('-').map(|(_, a)| a.to_string());
            let label = match account {
                Some(a) if !a.is_empty() => format!("{base} ({a})"),
                _ => base.to_string(),
            };
            // Google Drive keeps the user's files in "My Drive".
            let path = if provider == "google-drive" && p.join("My Drive").is_dir() { p.join("My Drive") } else { p };
            push(&mut v, provider, label, path);
        }
    }
    push(&mut v, "icloud", "iCloud Drive".into(), home.join("Library/Mobile Documents/com~apple~CloudDocs"));

    // Windows: OneDrive sets environment variables; Dropbox writes info.json.
    for (var, label) in [("OneDriveConsumer", "OneDrive"), ("OneDriveCommercial", "OneDrive (Work)"), ("OneDrive", "OneDrive")] {
        if let Some(p) = env(var) {
            push(&mut v, "onedrive", label.into(), PathBuf::from(p));
        }
    }
    for var in ["LOCALAPPDATA", "APPDATA"] {
        if let Some(base) = env(var) {
            dropbox_info(&Path::new(&base).join("Dropbox/info.json"), &mut v);
        }
    }
    dropbox_info(&home.join(".dropbox/info.json"), &mut v);
    // Google Drive for desktop on Windows mounts a drive letter with "My Drive".
    if cfg!(windows) {
        for letter in b'D'..=b'Z' {
            let p = PathBuf::from(format!("{}:\\My Drive", letter as char));
            push(&mut v, "google-drive", "Google Drive".into(), p);
        }
    }
    // Common default folders (Linux, older installs).
    push(&mut v, "dropbox", "Dropbox".into(), home.join("Dropbox"));
    push(&mut v, "onedrive", "OneDrive".into(), home.join("OneDrive"));
    push(&mut v, "google-drive", "Google Drive".into(), home.join("Google Drive"));
    v
}

pub fn detect() -> Vec<CloudFolder> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default();
    detect_in(&home, &|k| std::env::var(k).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_mac_windows_and_linux_layouts() {
        let t = std::env::temp_dir().join(format!("zipkoo-cloud-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        let cs = t.join("Library/CloudStorage");
        std::fs::create_dir_all(cs.join("GoogleDrive-rina@example.com/My Drive")).unwrap();
        std::fs::create_dir_all(cs.join("OneDrive-Contoso")).unwrap();
        std::fs::create_dir_all(cs.join("SomethingElse")).unwrap();
        std::fs::create_dir_all(t.join("Library/Mobile Documents/com~apple~CloudDocs")).unwrap();
        let od = t.join("OneDrive - Personal");
        std::fs::create_dir_all(&od).unwrap();
        let dbx = t.join("Dropbox (Home)");
        std::fs::create_dir_all(&dbx).unwrap();
        std::fs::create_dir_all(t.join("AppData/Local/Dropbox")).unwrap();
        std::fs::write(t.join("AppData/Local/Dropbox/info.json"), format!(r#"{{"personal":{{"path":{:?}}}}}"#, dbx.to_string_lossy())).unwrap();

        let od2 = od.clone();
        let local = t.join("AppData/Local");
        let env = move |k: &str| match k {
            "OneDrive" => Some(od2.to_string_lossy().into_owned()),
            "LOCALAPPDATA" => Some(local.to_string_lossy().into_owned()),
            _ => None,
        };
        let v = detect_in(&t, &env);
        let labels: Vec<&str> = v.iter().map(|c| c.label.as_str()).collect();
        assert!(labels.contains(&"Google Drive (rina@example.com)"), "{labels:?}");
        assert!(v.iter().any(|c| c.path.ends_with("My Drive")));
        assert!(labels.contains(&"OneDrive (Contoso)"));
        assert!(labels.contains(&"iCloud Drive"));
        assert!(v.iter().any(|c| c.provider == "onedrive" && c.path == od));
        assert!(v.iter().any(|c| c.provider == "dropbox" && c.path == dbx));
        assert!(!labels.iter().any(|l| l.contains("SomethingElse")));
        let _ = std::fs::remove_dir_all(&t);
    }
}
