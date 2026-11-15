//! Tauri commands that expose zipkoo-core to the UI.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};
use zipkoo_core::{ArchiveInfo, ExtractOptions, ExtractReport, Format, Level, PackOptions, PackReport, Progress};

#[derive(Default)]
struct Jobs(Mutex<HashMap<String, Arc<AtomicBool>>>);

/// Errors cross to the UI as plain sentences plus a machine code.
#[derive(Debug, Serialize)]
struct UiError {
    code: &'static str,
    message: String,
}

impl From<zipkoo_core::Error> for UiError {
    fn from(e: zipkoo_core::Error) -> Self {
        use zipkoo_core::Error as E;
        let code = match &e {
            E::PasswordRequired => "password_required",
            E::WrongPassword => "wrong_password",
            E::TooLarge { .. } => "too_large",
            E::Cancelled => "cancelled",
            E::UnsupportedFormat(_) => "unsupported",
            E::Damaged(_) => "damaged",
            _ => "error",
        };
        UiError { code, message: e.to_string() }
    }
}

#[derive(Clone, Serialize)]
struct JobEvent<'a> {
    job: &'a str,
    progress: &'a Progress,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackRequest {
    inputs: Vec<PathBuf>,
    output: PathBuf,
    format: Format,
    level: Level,
    smart: bool,
    password: Option<String>,
    hide_names: bool,
    split_size: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExtractRequest {
    archive: PathBuf,
    dest: PathBuf,
    password: Option<String>,
    overwrite: bool,
    allow_programs: bool,
    only: Vec<String>,
}

fn register(jobs: &Jobs, id: &str) -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    jobs.0.lock().unwrap().insert(id.to_string(), flag.clone());
    flag
}

#[tauri::command]
async fn inspect_archive(path: PathBuf, password: Option<String>) -> Result<ArchiveInfo, UiError> {
    tauri::async_runtime::spawn_blocking(move || zipkoo_core::inspect(&path, password.as_deref()))
        .await
        .map_err(|e| UiError { code: "error", message: e.to_string() })?
        .map_err(Into::into)
}

/// Raw bytes for Peek previews (images, text, PDF). Capped at 64 MB.
#[tauri::command]
async fn peek_file(path: PathBuf, name: String, password: Option<String>) -> Result<tauri::ipc::Response, UiError> {
    let bytes = tauri::async_runtime::spawn_blocking(move || zipkoo_core::peek(&path, &name, password.as_deref(), 64 << 20))
        .await
        .map_err(|e| UiError { code: "error", message: e.to_string() })??;
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
async fn pack_files(app: AppHandle, jobs: State<'_, Jobs>, job: String, req: PackRequest) -> Result<PackReport, UiError> {
    let cancel = register(&jobs, &job);
    let job2 = job.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        let opts = PackOptions {
            format: req.format,
            level: req.level,
            smart: req.smart,
            password: req.password.filter(|p| !p.is_empty()),
            hide_names: req.hide_names,
            split_size: req.split_size,
            ..Default::default()
        };
        let mut emit = |p: &Progress| {
            let _ = app.emit("job-progress", JobEvent { job: &job2, progress: p });
        };
        zipkoo_core::pack(&req.inputs, &req.output, &opts, &mut emit, Some(&cancel))
    })
    .await
    .map_err(|e| UiError { code: "error", message: e.to_string() })?;
    jobs.0.lock().unwrap().remove(&job);
    res.map_err(Into::into)
}

#[tauri::command]
async fn extract_archive(app: AppHandle, jobs: State<'_, Jobs>, job: String, req: ExtractRequest) -> Result<ExtractReport, UiError> {
    let cancel = register(&jobs, &job);
    let job2 = job.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        let opts = ExtractOptions {
            password: req.password.as_deref(),
            overwrite: req.overwrite,
            allow_executables: req.allow_programs,
            only: req.only,
            ..Default::default()
        };
        let mut emit = |p: &Progress| {
            let _ = app.emit("job-progress", JobEvent { job: &job2, progress: p });
        };
        zipkoo_core::extract(&req.archive, &req.dest, &opts, &mut emit, Some(&cancel))
    })
    .await
    .map_err(|e| UiError { code: "error", message: e.to_string() })?;
    jobs.0.lock().unwrap().remove(&job);
    res.map_err(Into::into)
}

#[tauri::command]
fn cancel_job(jobs: State<'_, Jobs>, job: String) {
    if let Some(f) = jobs.0.lock().unwrap().get(&job) {
        f.store(true, Ordering::Relaxed);
    }
}

#[tauri::command]
async fn hash_file(path: PathBuf) -> Result<String, UiError> {
    tauri::async_runtime::spawn_blocking(move || zipkoo_core::hash::sha256_file(&path))
        .await
        .map_err(|e| UiError { code: "error", message: e.to_string() })?
        .map_err(|e| UiError { code: "error", message: e.to_string() })
}

/// Paths the app was launched with ("Open with Zipkoo", file associations).
#[tauri::command]
fn launch_paths() -> Vec<String> {
    std::env::args().skip(1).filter(|a| !a.starts_with('-')).collect()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Jobs::default())
        .setup(|app| {
            #[cfg(debug_assertions)]
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_title("Zipkoo (dev)");
            }
            let _ = app;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            inspect_archive,
            peek_file,
            pack_files,
            extract_archive,
            cancel_job,
            hash_file,
            launch_paths
        ])
        .run(tauri::generate_context!())
        .expect("error while running Zipkoo");
}
