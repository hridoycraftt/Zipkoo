//! Folder access. The Mac App Store build runs in Apple's sandbox: Zipkoo may only touch files
//! and folders the person picked (or dropped / opened from Finder). Before writing next to an
//! archive, the app checks it can; if not, it asks the person to choose that folder once.
//! In the App Store build that choice is saved as a security-scoped bookmark, so it still holds
//! after a restart (watch folders, recents, "Extract here" in the same place).
//! Other builds have no sandbox: the check still catches read-only folders, and nothing is saved.
use crate::UiError;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

/// Can Zipkoo write in `dir` (or read it, when `write` is false)? `dir` may not exist yet (a new
/// "Extract here" folder): then its nearest existing parent is what counts. Returns `None` when
/// access is fine, or the folder the person needs to choose.
#[tauri::command]
pub fn folder_access(dir: PathBuf, write: bool) -> Option<PathBuf> {
    let existing = nearest_existing(&dir)?;
    if probe(&existing, write) {
        None
    } else {
        Some(existing)
    }
}

fn nearest_existing(dir: &Path) -> Option<PathBuf> {
    dir.ancestors()
        .find(|d| !d.as_os_str().is_empty() && d.is_dir())
        .map(Path::to_path_buf)
}

fn probe(dir: &Path, write: bool) -> bool {
    if !write {
        return std::fs::read_dir(dir).is_ok();
    }
    // The only reliable test is to try: permissions bits don't show sandbox rules.
    let p = dir.join(format!(".zipkoo-access-{}", std::process::id()));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&p)
    {
        Ok(_) => {
            let _ = std::fs::remove_file(&p);
            true
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = std::fs::remove_file(&p);
            true
        }
        Err(_) => false,
    }
}

/// The person picked `dir` in a folder dialog: keep the access (App Store build on Mac only).
#[tauri::command]
pub fn folder_grant(app: AppHandle, dir: PathBuf) -> Result<(), UiError> {
    imp::grant(&app, &dir)
}

/// Re-open the folders granted in earlier runs. Called once at startup.
pub fn restore(app: &AppHandle) {
    imp::restore(app);
}

#[allow(dead_code)]
fn store_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|d| d.join("folder-access.json"))
}

#[cfg(all(target_os = "macos", feature = "appstore"))]
mod imp {
    use super::*;
    use base64::Engine as _;
    use objc2::runtime::Bool;
    use objc2_foundation::{
        NSData, NSString, NSURLBookmarkCreationOptions, NSURLBookmarkResolutionOptions, NSURL,
    };
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    // Folders opened this run; kept so their access lasts as long as the app runs.
    static OPEN: Mutex<Vec<objc2::rc::Retained<NSURL>>> = Mutex::new(Vec::new());

    fn load(app: &AppHandle) -> BTreeMap<String, String> {
        store_path(app)
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save(app: &AppHandle, map: &BTreeMap<String, String>) -> std::io::Result<()> {
        let p = store_path(app).ok_or_else(|| std::io::Error::other("no app data folder"))?;
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(p, serde_json::to_vec(map).unwrap_or_default())
    }

    fn bookmark(dir: &Path) -> Option<Vec<u8>> {
        let url = NSURL::fileURLWithPath(&NSString::from_str(&dir.to_string_lossy()));
        url.bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
            NSURLBookmarkCreationOptions::WithSecurityScope,
            None,
            None,
        )
        .ok()
        .map(|d| d.to_vec())
    }

    /// Resolve a saved bookmark and start using it. Returns the folder's current path and
    /// whether the bookmark should be saved again (macOS marks it stale after a move).
    fn open(data: &[u8]) -> Option<(PathBuf, bool)> {
        let data = NSData::with_bytes(data);
        let mut stale = Bool::NO;
        // SAFETY: `stale` is a valid, writable Bool for the duration of the call.
        let url = unsafe {
            NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error(
                &data,
                NSURLBookmarkResolutionOptions::WithSecurityScope,
                None,
                &mut stale,
            )
        }
        .ok()?;
        // SAFETY: plain Objective-C call on a valid NSURL.
        if !unsafe { url.startAccessingSecurityScopedResource() } {
            return None;
        }
        let path = PathBuf::from(url.path()?.to_string());
        OPEN.lock().unwrap_or_else(|e| e.into_inner()).push(url);
        Some((path, stale.as_bool()))
    }

    pub fn grant(app: &AppHandle, dir: &Path) -> Result<(), UiError> {
        let data = bookmark(dir).ok_or_else(|| UiError {
            code: "no_access",
            message: format!("Couldn't keep access to {}", dir.display()),
        })?;
        let mut map = load(app);
        map.insert(
            dir.to_string_lossy().into_owned(),
            base64::engine::general_purpose::STANDARD.encode(&data),
        );
        save(app, &map).map_err(|e| UiError {
            code: "error",
            message: e.to_string(),
        })?;
        let _ = open(&data);
        Ok(())
    }

    pub fn restore(app: &AppHandle) {
        let map = load(app);
        let mut fresh = BTreeMap::new();
        for (_, b64) in map {
            let Ok(data) = base64::engine::general_purpose::STANDARD.decode(b64.as_bytes()) else {
                continue;
            };
            // Folders that were deleted, or whose access was revoked, are dropped.
            if let Some((path, stale)) = open(&data) {
                let data = if stale {
                    bookmark(&path).unwrap_or(data)
                } else {
                    data
                };
                fresh.insert(
                    path.to_string_lossy().into_owned(),
                    base64::engine::general_purpose::STANDARD.encode(&data),
                );
            }
        }
        let _ = save(app, &fresh);
    }
}

#[cfg(not(all(target_os = "macos", feature = "appstore")))]
mod imp {
    use super::*;
    pub fn grant(_app: &AppHandle, _dir: &Path) -> Result<(), UiError> {
        Ok(())
    }
    pub fn restore(_app: &AppHandle) {}
}

#[cfg(test)]
mod tests {
    #[test]
    fn probe_write_and_read() {
        let t = std::env::temp_dir().join(format!("zk-probe-{}", std::process::id()));
        std::fs::create_dir_all(&t).unwrap();
        assert!(super::probe(&t, true));
        assert!(super::probe(&t, false));
        assert_eq!(
            std::fs::read_dir(&t).unwrap().count(),
            0,
            "probe file left behind"
        );
        assert!(!super::probe(&t.join("missing"), true));
        // A folder that doesn't exist yet is judged by its parent.
        assert_eq!(super::folder_access(t.join("new/deeper"), true), None);
        std::fs::remove_dir_all(&t).unwrap();
    }
}
