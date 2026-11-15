//! Tauri commands for features 6–12: share, smart update, self-heal, watch folders, cloud,
//! search, duplicates, photo shrinking and batch convert.
use crate::{UiError, Jobs};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, State};
use zipkoo_core::{Format, PackOptions, Progress};

fn ui(e: impl std::fmt::Display) -> UiError {
    UiError { code: "error", message: e.to_string() }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn data_dir(app: &AppHandle) -> PathBuf {
    let d = app.path().app_data_dir().unwrap_or_else(|_| std::env::temp_dir().join("zipkoo"));
    let _ = std::fs::create_dir_all(&d);
    d
}

fn load<T: for<'de> Deserialize<'de> + Default>(p: &Path) -> T {
    std::fs::read(p).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save<T: Serialize>(p: &Path, v: &T) -> Result<(), UiError> {
    let tmp = p.with_extension("json.part");
    std::fs::write(&tmp, serde_json::to_vec_pretty(v).map_err(ui)?).map_err(ui)?;
    std::fs::rename(tmp, p).map_err(ui)
}

/// Run a blocking job with progress events and a cancel flag.
async fn job<T: Send + 'static>(
    app: AppHandle,
    jobs: &Jobs,
    id: String,
    f: impl FnOnce(&mut dyn FnMut(&Progress), &AtomicBool) -> zipkoo_core::Result<T> + Send + 'static,
) -> Result<T, UiError> {
    let cancel = Arc::new(AtomicBool::new(false));
    jobs.0.lock().unwrap_or_else(|e| e.into_inner()).insert(id.clone(), cancel.clone());
    let id2 = id.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        let mut emit = |p: &Progress| {
            let _ = app.emit("job-progress", serde_json::json!({ "job": id2, "progress": p }));
        };
        f(&mut emit, &cancel)
    })
    .await;
    jobs.0.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
    res.map_err(ui)?.map_err(Into::into)
}

// ---------- 12. Batch Convert ----------

#[derive(Serialize)]
pub struct ConvertResult {
    source: PathBuf,
    output: Option<PathBuf>,
    error: Option<String>,
    output_bytes: u64,
}

#[tauri::command]
pub async fn convert_archives(app: AppHandle, jobs: State<'_, Jobs>, job_id: String, archives: Vec<PathBuf>, format: Format, password: Option<String>) -> Result<Vec<ConvertResult>, UiError> {
    job(app, &jobs, job_id, move |emit, cancel| {
        let mut out = Vec::new();
        for a in archives {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let dst = zipkoo_core::convert::converted_path(&a, format);
            // A locked archive stays locked in its new format; TAR.ZST can't hold a password.
            let locked = match &password {
                Some(_) => zipkoo_core::inspect(&a, password.as_deref()).map(|i| i.encrypted).unwrap_or(true),
                None => false,
            };
            if locked && !format.supports_password() {
                out.push(ConvertResult { source: a, output: None, error: Some("This archive is locked, and TAR.ZST can't keep a password. Convert it to ZIP or 7z instead.".into()), output_bytes: 0 });
                continue;
            }
            let pack = PackOptions { format, password: if locked { password.clone() } else { None }, ..Default::default() };
            let opts = zipkoo_core::convert::ConvertOptions { password: password.as_deref(), pack };
            match zipkoo_core::convert::convert(&a, &dst, &opts, emit, Some(cancel)) {
                Ok(r) => out.push(ConvertResult { source: a, output: Some(dst), error: None, output_bytes: r.output_bytes }),
                Err(zipkoo_core::Error::Cancelled) => return Err(zipkoo_core::Error::Cancelled),
                Err(e) => out.push(ConvertResult { source: a, output: None, error: Some(e.to_string()), output_bytes: 0 }),
            }
        }
        Ok(out)
    })
    .await
}

// ---------- 7. Smart Update ----------

#[tauri::command]
pub async fn update_archive(app: AppHandle, jobs: State<'_, Jobs>, job_id: String, archive: PathBuf, inputs: Vec<PathBuf>, password: Option<String>, remove_missing: bool) -> Result<zipkoo_core::update::UpdateReport, UiError> {
    job(app, &jobs, job_id, move |emit, cancel| {
        let opts = zipkoo_core::update::UpdateOptions { password: password.as_deref(), keep_history: true, remove_missing, smart: true };
        zipkoo_core::update::update(&archive, &inputs, &opts, emit, Some(cancel))
    })
    .await
}

#[tauri::command]
pub async fn archive_history(archive: PathBuf) -> Result<zipkoo_core::update::Manifest, UiError> {
    tauri::async_runtime::spawn_blocking(move || zipkoo_core::update::read_manifest(&archive)).await.map_err(ui)?.map_err(Into::into)
}

// ---------- 8. Self-heal ----------

#[derive(Serialize)]
pub struct HealStatus {
    has_repair_data: bool,
    health: Option<zipkoo_core::heal::Health>,
}

#[tauri::command]
pub fn heal_status(archive: PathBuf) -> HealStatus {
    HealStatus { has_repair_data: zipkoo_core::heal::recovery_path(&archive).is_file(), health: None }
}

#[tauri::command]
pub async fn heal_create(app: AppHandle, jobs: State<'_, Jobs>, job_id: String, archive: PathBuf, percent: u8) -> Result<zipkoo_core::heal::RecoveryReport, UiError> {
    job(app, &jobs, job_id, move |emit, cancel| zipkoo_core::heal::create(&archive, percent, emit, Some(cancel))).await
}

#[tauri::command]
pub async fn heal_check(app: AppHandle, jobs: State<'_, Jobs>, job_id: String, archive: PathBuf, fix: bool) -> Result<zipkoo_core::heal::Health, UiError> {
    job(app, &jobs, job_id, move |emit, cancel| zipkoo_core::heal::check(&archive, fix, emit, Some(cancel))).await
}

// ---------- 11. Duplicates ----------

#[tauri::command]
pub async fn find_duplicates(inputs: Vec<PathBuf>) -> Result<Vec<zipkoo_core::dupes::DupGroup>, UiError> {
    tauri::async_runtime::spawn_blocking(move || zipkoo_core::dupes::find(&inputs, 1)).await.map_err(ui)?.map_err(Into::into)
}

// ---------- 10. Cloud ----------

#[tauri::command]
pub fn cloud_folders() -> Vec<zipkoo_core::cloud::CloudFolder> {
    zipkoo_core::cloud::detect()
}

#[derive(Serialize)]
pub struct FolderItem {
    path: PathBuf,
    size: u64,
    modified: u64,
}

/// Archives inside a folder (e.g. a cloud folder), newest first.
#[tauri::command]
pub async fn list_archives(folder: PathBuf) -> Vec<FolderItem> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut v: Vec<FolderItem> = zipkoo_core::index::find_archives(&folder, 4)
            .into_iter()
            .filter_map(|p| {
                let m = std::fs::metadata(&p).ok()?;
                let modified = m.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs();
                Some(FolderItem { path: p, size: m.len(), modified })
            })
            .collect();
        v.sort_by(|a, b| b.modified.cmp(&a.modified));
        v
    })
    .await
    .unwrap_or_default()
}

// ---------- 11. Search across archives ----------

#[derive(Default)]
pub struct SearchIndex(pub Mutex<Option<zipkoo_core::index::Index>>);

fn library_roots(extra: &[PathBuf]) -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default();
    let mut v: Vec<PathBuf> = ["Downloads", "Documents", "Desktop"].iter().map(|d| home.join(d)).filter(|p| p.is_dir()).collect();
    v.extend(zipkoo_core::cloud::detect().into_iter().map(|c| c.path));
    v.extend(extra.iter().cloned());
    v
}

#[derive(Serialize)]
pub struct IndexStats {
    archives: u64,
    files: u64,
    indexed_now: usize,
}

/// Index archives in Downloads, Documents, Desktop, cloud folders and the given extra paths
/// (recent archives and folders). Only changed archives are read again.
#[tauri::command]
pub async fn index_refresh(app: AppHandle, extra: Vec<PathBuf>) -> Result<IndexStats, UiError> {
    let db = data_dir(&app).join("search.db");
    tauri::async_runtime::spawn_blocking(move || {
        // Refresh on its own connection so searching stays responsive while this runs
        // (SQLite in WAL mode allows a reader alongside the writer).
        let mut ix = zipkoo_core::index::Index::open(&db)?;
        {
            let state = app.state::<SearchIndex>();
            let mut g = state.0.lock().unwrap_or_else(|e| e.into_inner());
            if g.is_none() {
                *g = zipkoo_core::index::Index::open(&db).ok();
            }
        }
        let mut archives = Vec::new();
        for root in library_roots(&extra) {
            if root.is_file() {
                archives.push(root);
            } else {
                archives.extend(zipkoo_core::index::find_archives(&root, 5));
            }
        }
        let r = ix.refresh(&archives)?;
        ix.prune()?;
        let (a, f) = ix.stats()?;
        Ok::<_, zipkoo_core::Error>(IndexStats { archives: a, files: f, indexed_now: r.indexed })
    })
    .await
    .map_err(ui)?
    .map_err(Into::into)
}

#[tauri::command]
pub async fn search_archives(app: AppHandle, query: String) -> Result<Vec<zipkoo_core::index::Hit>, UiError> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<SearchIndex>();
        let g = state.0.lock().unwrap_or_else(|e| e.into_inner());
        match g.as_ref() {
            Some(ix) => ix.search(&query, 300).map_err(Into::into),
            None => Ok(Vec::new()),
        }
    })
    .await
    .map_err(ui)?
}

// ---------- 6. Secure Share ----------

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ShareRecord {
    pub url: String,
    pub id: String,
    pub delete_token: String,
    pub expires_at: u64,
    pub max_downloads: Option<u32>,
    pub password: bool,
    pub name: String,
    pub size: u64,
    pub created: u64,
    pub server: String,
}

fn shares_file(app: &AppHandle) -> PathBuf {
    data_dir(app).join("shares.json")
}

#[tauri::command]
pub async fn share_file(app: AppHandle, jobs: State<'_, Jobs>, job_id: String, path: PathBuf, server: String, hours: u32, downloads: Option<u32>, password: Option<String>) -> Result<ShareRecord, UiError> {
    if server.trim().is_empty() {
        return Err(UiError { code: "no_server", message: "Add your share server address first.".into() });
    }
    let file = shares_file(&app);
    let app2 = app.clone();
    let rec = job(app, &jobs, job_id, move |emit, cancel| {
        let size = std::fs::metadata(&path)?.len();
        // Encrypting and uploading both count, so progress goes to 2× the size.
        let mut done = 0u64;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut last = std::time::Instant::now();
        let mut cb = |n: u64| {
            done += n;
            if last.elapsed().as_millis() > 80 {
                last = std::time::Instant::now();
                emit(&Progress { done_bytes: done, total_bytes: size * 2, current: name.clone() });
            }
        };
        if cancel.load(Ordering::Relaxed) {
            return Err(zipkoo_core::Error::Cancelled);
        }
        let opts = zipkoo_core::share::ShareOptions { server: &server, expires_hours: hours, max_downloads: downloads, password: password.as_deref().filter(|p| !p.is_empty()) };
        let s = zipkoo_core::share::upload_cancellable(&path, &opts, &mut cb, Some(cancel))?;
        Ok(ShareRecord { url: s.url, id: s.id, delete_token: s.delete_token, expires_at: s.expires_at, max_downloads: s.max_downloads, password: s.password, name, size, created: now(), server: server.clone() })
    })
    .await?;
    let mut all: Vec<ShareRecord> = load(&file);
    all.insert(0, rec.clone());
    save(&file, &all)?;
    let _ = app2.emit("shares-updated", ());
    Ok(rec)
}

#[tauri::command]
pub fn shares_list(app: AppHandle) -> Vec<ShareRecord> {
    let mut v: Vec<ShareRecord> = load(&shares_file(&app));
    // Drop links that expired more than a week ago.
    v.retain(|s| s.expires_at + 7 * 86400 > now());
    v
}

#[tauri::command]
pub async fn share_delete(app: AppHandle, id: String) -> Result<(), UiError> {
    let file = shares_file(&app);
    let mut all: Vec<ShareRecord> = load(&file);
    if let Some(s) = all.iter().find(|s| s.id == id).cloned() {
        let r = tauri::async_runtime::spawn_blocking(move || zipkoo_core::share::delete(&s.server, &s.id, &s.delete_token)).await.map_err(ui)?;
        // Already gone on the server is fine.
        if let Err(e) = r {
            if !matches!(e, zipkoo_core::Error::EntryNotFound(_)) {
                return Err(e.into());
            }
        }
    }
    all.retain(|s| s.id != id);
    save(&file, &all)
}

#[tauri::command]
pub async fn share_download(app: AppHandle, jobs: State<'_, Jobs>, job_id: String, link: String, dest: PathBuf, password: Option<String>) -> Result<PathBuf, UiError> {
    job(app, &jobs, job_id, move |emit, cancel| {
        let meta = zipkoo_core::share::check(&link, password.as_deref())?;
        let mut done = 0u64;
        let mut last = std::time::Instant::now();
        let mut cb = |n: u64| {
            done += n;
            if last.elapsed().as_millis() > 80 {
                last = std::time::Instant::now();
                emit(&Progress { done_bytes: done, total_bytes: meta.size, current: meta.name.clone() });
            }
        };
        zipkoo_core::share::download_cancellable(&link, &dest, password.as_deref(), &mut cb, Some(cancel))
    })
    .await
}

// ---------- 9. Watch Folders ----------

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Watch {
    pub id: String,
    pub folder: PathBuf,
    pub dest: PathBuf,
    /// Minimum minutes between snapshots.
    pub interval_minutes: u32,
    pub keep: usize,
    pub repair: bool,
    pub paused: bool,
    pub last_run: Option<u64>,
    pub last_result: Option<String>,
    pub last_ok: bool,
}

#[derive(Default)]
pub struct Watches {
    pub running: Mutex<std::collections::HashSet<String>>,
}

fn watches_file(app: &AppHandle) -> PathBuf {
    data_dir(app).join("watches.json")
}

#[tauri::command]
pub fn watches_list(app: AppHandle) -> Vec<Watch> {
    load(&watches_file(&app))
}

#[tauri::command]
pub fn watch_save(app: AppHandle, watch: Watch) -> Result<Vec<Watch>, UiError> {
    if !watch.folder.is_dir() {
        return Err(ui("Pick a folder to watch."));
    }
    if watch.dest.starts_with(&watch.folder) {
        return Err(ui("Save backups outside the folder you're watching, or each backup would include the last one."));
    }
    let file = watches_file(&app);
    let mut all: Vec<Watch> = load(&file);
    let mut w = watch;
    if w.id.is_empty() {
        w.id = format!("w{}", now());
    }
    w.interval_minutes = w.interval_minutes.max(5);
    match all.iter_mut().find(|x| x.id == w.id) {
        Some(x) => *x = w,
        None => all.push(w),
    }
    save(&file, &all)?;
    Ok(all)
}

#[tauri::command]
pub fn watch_remove(app: AppHandle, id: String) -> Result<Vec<Watch>, UiError> {
    let file = watches_file(&app);
    let mut all: Vec<Watch> = load(&file);
    all.retain(|w| w.id != id);
    save(&file, &all)?;
    Ok(all)
}

/// Snapshot one watch now (also used by the scheduler). A panic inside is caught so the
/// scheduler keeps running and the watch isn't stuck as "running" forever.
pub fn run_watch(app: &AppHandle, id: &str, force: bool) {
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_watch_inner(app, id, force)));
    if r.is_err() {
        app.state::<Watches>().running.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
        let file = watches_file(app);
        let mut all: Vec<Watch> = load(&file);
        if let Some(x) = all.iter_mut().find(|x| x.id == id) {
            x.last_run = Some(now());
            x.last_result = Some("The backup stopped because of an internal error.".into());
            x.last_ok = false;
        }
        let _ = save(&file, &all);
        let _ = app.emit("watches-updated", ());
    }
}

fn run_watch_inner(app: &AppHandle, id: &str, force: bool) {
    let state = app.state::<Watches>();
    if !state.running.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_string()) {
        return;
    }
    let file = watches_file(app);
    let all: Vec<Watch> = load(&file);
    let Some(w) = all.into_iter().find(|w| w.id == id) else {
        state.running.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
        return;
    };
    let due = force || w.last_run.map(|t| now() >= t + w.interval_minutes as u64 * 60).unwrap_or(true);
    if !due || (w.paused && !force) {
        state.running.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
        return;
    }
    let opts = zipkoo_core::backup::BackupOptions { keep: w.keep, recovery_percent: w.repair.then_some(10), ..Default::default() };
    let res = zipkoo_core::backup::snapshot(&w.folder, &w.dest, &opts, &mut |_| {}, None);
    let (msg, ok) = match &res {
        Ok(Some(s)) => (format!("Saved {}", s.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()), true),
        Ok(None) => ("No changes since the last backup".to_string(), true),
        Err(e) => (e.to_string(), false),
    };
    let mut all: Vec<Watch> = load(&file);
    if let Some(x) = all.iter_mut().find(|x| x.id == id) {
        x.last_run = Some(now());
        x.last_result = Some(msg);
        x.last_ok = ok;
    }
    let _ = save(&file, &all);
    state.running.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
    let _ = app.emit("watches-updated", ());
}

#[tauri::command]
pub async fn watch_run_now(app: AppHandle, id: String) -> Vec<Watch> {
    let a = app.clone();
    let _ = tauri::async_runtime::spawn_blocking(move || run_watch(&a, &id, true)).await;
    load(&watches_file(&app))
}

/// Check every watch once a minute; each runs when its interval has passed and the folder changed.
pub fn start_scheduler(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
        let all: Vec<Watch> = load(&watches_file(&app));
        for w in all.iter().filter(|w| !w.paused) {
            run_watch(&app, &w.id, false);
        }
    });
}

// ---------- Editing, opening and dragging files inside an archive ----------

#[tauri::command]
pub async fn edit_archive(app: AppHandle, jobs: State<'_, Jobs>, job_id: String, archive: PathBuf, change: zipkoo_core::edit::Change, password: Option<String>) -> Result<zipkoo_core::edit::EditReport, UiError> {
    job(app, &jobs, job_id, move |emit, cancel| zipkoo_core::edit::edit(&archive, &change, password.as_deref(), emit, Some(cancel))).await
}

/// A fresh, private temporary folder for files taken out of an archive to open or drag.
fn scratch(app: &AppHandle, kind: &str) -> Result<PathBuf, UiError> {
    let base = app.path().app_cache_dir().unwrap_or_else(|_| std::env::temp_dir().join("zipkoo")).join(kind);
    // Clean up what earlier sessions left behind (older than a day).
    if let Ok(rd) = std::fs::read_dir(&base) {
        for e in rd.flatten() {
            let old = e.metadata().and_then(|m| m.modified()).map(|t| t.elapsed().map(|d| d.as_secs() > 86_400).unwrap_or(false)).unwrap_or(false);
            if old {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
    let dir = base.join(format!("{}-{}", now(), rand_tag()));
    std::fs::create_dir_all(&dir).map_err(ui)?;
    Ok(dir)
}

fn rand_tag() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    format!("{:x}", h.finish())
}

/// Take one file out to a private temp folder and return its path (the UI opens it with the
/// default app). Programs are never started automatically: the UI asks first.
#[tauri::command]
pub async fn open_entry(app: AppHandle, archive: PathBuf, name: String, password: Option<String>) -> Result<PathBuf, UiError> {
    let dir = scratch(&app, "open")?;
    tauri::async_runtime::spawn_blocking(move || {
        let opts = zipkoo_core::ExtractOptions { password: password.as_deref(), only: vec![name.clone()], strip: name.rsplit_once('/').map(|(p, _)| format!("{p}/")), ..Default::default() };
        let r = zipkoo_core::extract(&archive, &dir, &opts, &mut |_| {}, None)?;
        let leaf = name.rsplit('/').next().unwrap_or(&name).to_string();
        let out = dir.join(zipkoo_core::safety::windows_safe(&leaf));
        let out = if out.exists() { out } else { dir.join(&leaf) };
        if r.files == 0 || !out.exists() {
            return Err(zipkoo_core::Error::EntryNotFound(name));
        }
        Ok(out)
    })
    .await
    .map_err(ui)?
    .map_err(Into::into)
}

#[derive(Serialize)]
pub struct DragPayload {
    items: Vec<PathBuf>,
    icon: PathBuf,
}

/// Extract the dragged entries (files or whole folders) to a temp folder so the OS can copy them
/// wherever they are dropped.
#[tauri::command]
pub async fn prepare_drag(app: AppHandle, archive: PathBuf, names: Vec<String>, password: Option<String>) -> Result<DragPayload, UiError> {
    let dir = scratch(&app, "drag")?;
    let icon = dir.join(".zipkoo-drag.png");
    std::fs::write(&icon, include_bytes!("../icons/32x32.png")).map_err(ui)?;
    tauri::async_runtime::spawn_blocking(move || {
        // Items keep their names; their parent folder inside the archive is dropped.
        let mut items = Vec::new();
        for n in &names {
            let parent = n.trim_end_matches('/').rsplit_once('/').map(|(p, _)| format!("{p}/"));
            let opts = zipkoo_core::ExtractOptions { password: password.as_deref(), only: vec![n.clone()], strip: parent, on_conflict: zipkoo_core::Conflict::Rename, ..Default::default() };
            zipkoo_core::extract(&archive, &dir, &opts, &mut |_| {}, None)?;
            let leaf = n.trim_end_matches('/').rsplit('/').next().unwrap_or(n);
            let p = dir.join(leaf);
            if p.exists() {
                items.push(p);
            }
        }
        if items.is_empty() {
            return Err(zipkoo_core::Error::EntryNotFound(names.join(", ")));
        }
        Ok(DragPayload { items, icon })
    })
    .await
    .map_err(ui)?
    .map_err(Into::into)
}

// ---------- Conflicts and Test ----------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictQuery {
    archive: PathBuf,
    dest: PathBuf,
    password: Option<String>,
    #[serde(default)]
    only: Vec<String>,
    #[serde(default)]
    strip: Option<String>,
}

#[tauri::command]
pub async fn extract_conflicts(q: ConflictQuery) -> Result<Vec<String>, UiError> {
    tauri::async_runtime::spawn_blocking(move || {
        let opts = zipkoo_core::ExtractOptions { password: q.password.as_deref(), only: q.only, strip: q.strip, ..Default::default() };
        zipkoo_core::conflicts(&q.archive, &q.dest, &opts)
    })
    .await
    .map_err(ui)?
    .map_err(Into::into)
}

#[tauri::command]
pub async fn test_archive(app: AppHandle, jobs: State<'_, Jobs>, job_id: String, archive: PathBuf, password: Option<String>) -> Result<zipkoo_core::TestReport, UiError> {
    job(app, &jobs, job_id, move |emit, cancel| zipkoo_core::test(&archive, password.as_deref(), emit, Some(cancel))).await
}

// ---------- Settings support: app info, trash ----------

#[derive(Serialize)]
pub struct AppInfo {
    version: String,
    /// "direct" (our download, self-updating) or "appstore".
    flavor: &'static str,
    /// Updates are signed; without a public key configured the app never tries to update.
    updater_ready: bool,
    os: &'static str,
}

#[tauri::command]
pub fn app_info(app: AppHandle) -> AppInfo {
    let flavor = if cfg!(feature = "appstore") { "appstore" } else { "direct" };
    let pubkey = app.config().plugins.0.get("updater").and_then(|u| u.get("pubkey")).and_then(|k| k.as_str()).unwrap_or("");
    AppInfo {
        version: app.package_info().version.to_string(),
        flavor,
        updater_ready: flavor == "direct" && !pubkey.trim().is_empty(),
        os: std::env::consts::OS,
    }
}

/// Move a file to the Recycle Bin / Trash (used for "delete the archive after extracting").
#[tauri::command]
pub async fn trash_path(path: PathBuf) -> Result<(), UiError> {
    tauri::async_runtime::spawn_blocking(move || trash::delete(&path)).await.map_err(ui)?.map_err(ui)
}
