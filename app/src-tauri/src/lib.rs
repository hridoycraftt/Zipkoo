//! Tauri commands that expose zipkoo-core to the UI.
pub mod features;
pub mod shell;

use serde::{Deserialize, Serialize};
use shell::{Inbox, ShellAction};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};
use zipkoo_core::{
    ArchiveInfo, ExtractOptions, ExtractReport, Format, Level, PackOptions, PackReport, Progress,
};

#[derive(Default)]
pub struct Jobs(pub Mutex<HashMap<String, Arc<AtomicBool>>>);

/// Errors cross to the UI as plain sentences plus a machine code.
#[derive(Debug, Serialize)]
pub struct UiError {
    pub code: &'static str,
    pub message: String,
}

impl From<zipkoo_core::Error> for UiError {
    fn from(e: zipkoo_core::Error) -> Self {
        use zipkoo_core::Error as E;
        let code = match &e {
            E::PasswordRequired => "password_required",
            E::WrongPassword => "wrong_password",
            E::TooLarge { .. } => "too_large",
            E::NoSpace { .. } => "no_space",
            E::Cancelled => "cancelled",
            E::UnsupportedFormat(_) => "unsupported",
            E::Damaged(_) => "damaged",
            _ => "error",
        };
        UiError {
            code,
            message: e.to_string(),
        }
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
    /// Build a self-extracting program instead of a plain archive (ZIP payload).
    #[serde(default)]
    sfx: bool,
    /// Resize and re-encode photos into a temporary copy before packing.
    #[serde(default)]
    shrink_photos: bool,
    /// Write Self-heal repair data next to each output (percent).
    #[serde(default)]
    repair_percent: Option<u8>,
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
    #[serde(default)]
    strip: Option<String>,
    #[serde(default)]
    on_conflict: zipkoo_core::Conflict,
}

fn register(jobs: &Jobs, id: &str) -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    jobs.0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id.to_string(), flag.clone());
    flag
}

#[tauri::command]
async fn inspect_archive(path: PathBuf, password: Option<String>) -> Result<ArchiveInfo, UiError> {
    tauri::async_runtime::spawn_blocking(move || zipkoo_core::inspect(&path, password.as_deref()))
        .await
        .map_err(|e| UiError {
            code: "error",
            message: e.to_string(),
        })?
        .map_err(Into::into)
}

/// Raw bytes for Peek previews (images, text, PDF). Capped at 64 MB.
#[tauri::command]
async fn peek_file(
    path: PathBuf,
    name: String,
    password: Option<String>,
) -> Result<tauri::ipc::Response, UiError> {
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        zipkoo_core::peek(&path, &name, password.as_deref(), 64 << 20)
    })
    .await
    .map_err(|e| UiError {
        code: "error",
        message: e.to_string(),
    })??;
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
async fn pack_files(
    app: AppHandle,
    jobs: State<'_, Jobs>,
    job: String,
    req: PackRequest,
) -> Result<PackReport, UiError> {
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
            let _ = app.emit(
                "job-progress",
                JobEvent {
                    job: &job2,
                    progress: p,
                },
            );
        };
        // Optional photo shrinking into a temporary copy next to the output.
        let parent = req
            .output
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default();
        let shrink_dir = req
            .shrink_photos
            .then(|| zipkoo_core::unique_path(&parent.join(".zipkoo-shrink")));
        let inputs: Vec<PathBuf> = match &shrink_dir {
            Some(dir) => {
                let prepared =
                    zipkoo_core::optimize::optimize(&req.inputs, dir, &Default::default())
                        .and_then(|_| {
                            let mut v: Vec<PathBuf> = std::fs::read_dir(dir)?
                                .filter_map(|e| e.ok())
                                .map(|e| e.path())
                                .collect();
                            v.sort();
                            Ok(v)
                        });
                match prepared {
                    Ok(v) => v,
                    Err(e) => {
                        let _ = std::fs::remove_dir_all(dir);
                        return Err(e);
                    }
                }
            }
            None => req.inputs.clone(),
        };
        let res = if req.sfx {
            match sfx_stub(&app) {
                Some(stub) => zipkoo_core::sfx::build(
                    &stub,
                    &inputs,
                    &req.output,
                    &opts,
                    &mut emit,
                    Some(&cancel),
                ),
                None => Err(zipkoo_core::Error::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "The self-extracting stub is missing from this install.",
                ))),
            }
        } else {
            zipkoo_core::pack(&inputs, &req.output, &opts, &mut emit, Some(&cancel))
        };
        if let Some(dir) = &shrink_dir {
            let _ = std::fs::remove_dir_all(dir);
        }
        let report = res?;
        if let Some(p) = req.repair_percent {
            for out in &report.outputs {
                zipkoo_core::heal::create(out, p, &mut |_| {}, Some(&cancel))?;
            }
        }
        Ok::<_, zipkoo_core::Error>(report)
    })
    .await
    .map_err(|e| UiError {
        code: "error",
        message: e.to_string(),
    })?;
    jobs.0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&job);
    res.map_err(Into::into)
}

#[tauri::command]
async fn extract_archive(
    app: AppHandle,
    jobs: State<'_, Jobs>,
    job: String,
    req: ExtractRequest,
) -> Result<ExtractReport, UiError> {
    let cancel = register(&jobs, &job);
    let job2 = job.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        let opts = ExtractOptions {
            password: req.password.as_deref(),
            overwrite: req.overwrite,
            allow_executables: req.allow_programs,
            only: req.only,
            strip: req.strip,
            on_conflict: req.on_conflict,
            ..Default::default()
        };
        let mut emit = |p: &Progress| {
            let _ = app.emit(
                "job-progress",
                JobEvent {
                    job: &job2,
                    progress: p,
                },
            );
        };
        zipkoo_core::extract(&req.archive, &req.dest, &opts, &mut emit, Some(&cancel))
    })
    .await
    .map_err(|e| UiError {
        code: "error",
        message: e.to_string(),
    })?;
    jobs.0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&job);
    res.map_err(Into::into)
}

#[tauri::command]
fn cancel_job(jobs: State<'_, Jobs>, job: String) {
    if let Some(f) = jobs.0.lock().unwrap_or_else(|e| e.into_inner()).get(&job) {
        f.store(true, Ordering::Relaxed);
    }
}

#[tauri::command]
async fn hash_file(path: PathBuf) -> Result<String, UiError> {
    tauri::async_runtime::spawn_blocking(move || zipkoo_core::hash::sha256_file(&path))
        .await
        .map_err(|e| UiError {
            code: "error",
            message: e.to_string(),
        })?
        .map_err(|e| UiError {
            code: "error",
            message: e.to_string(),
        })
}

/// Where the SFX stub lives: bundled as a resource on Windows, next to the binary in dev builds.
fn sfx_stub(app: &AppHandle) -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "zipkoo-sfx.exe"
    } else {
        "zipkoo-sfx"
    };
    let mut candidates = Vec::new();
    if let Ok(p) = app
        .path()
        .resolve(name, tauri::path::BaseDirectory::Resource)
    {
        candidates.push(p);
    }
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|p| p.to_path_buf()))
    {
        candidates.push(dir.join(name));
        candidates.push(dir.join("../release").join(name));
    }
    if let Some(p) = std::env::var_os("ZIPKOO_SFX_STUB") {
        candidates.insert(0, PathBuf::from(p));
    }
    candidates.into_iter().find(|p| p.is_file())
}

#[tauri::command]
fn sfx_available(app: AppHandle) -> bool {
    // Self-extracting programs are a Windows convention; macOS blocks unsigned downloaded apps.
    (cfg!(windows) || std::env::var_os("ZIPKOO_SFX_STUB").is_some()) && sfx_stub(&app).is_some()
}

/// A free output path next to the first input: "Photos.zip", or "Photos (2).zip" if taken.
#[tauri::command]
fn suggest_pack_output(inputs: Vec<PathBuf>, ext: String) -> Option<PathBuf> {
    let first = inputs.first()?;
    let parent = first.parent()?.to_path_buf();
    let name = if inputs.len() == 1 {
        let n = first.file_name()?.to_string_lossy().into_owned();
        if first.is_dir() {
            n
        } else {
            first.file_stem()?.to_string_lossy().into_owned()
        }
    } else {
        parent
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Archive".into())
    };
    Some(zipkoo_core::unique_path(
        &parent.join(format!("{name}.{ext}")),
    ))
}

/// "Extract here": skip the extra folder when the archive already has one top folder.
#[tauri::command]
async fn suggest_extract_dest(
    path: PathBuf,
    password: Option<String>,
) -> Result<zipkoo_core::ExtractTarget, UiError> {
    tauri::async_runtime::spawn_blocking(move || {
        let info = zipkoo_core::inspect(&path, password.as_deref())?;
        Ok::<_, zipkoo_core::Error>(zipkoo_core::suggest_dest(&path, &info))
    })
    .await
    .map_err(|e| UiError {
        code: "error",
        message: e.to_string(),
    })?
    .map_err(Into::into)
}

#[derive(Serialize)]
struct ShellStatus {
    supported: bool,
    installed: bool,
}

#[tauri::command]
fn shell_status() -> ShellStatus {
    ShellStatus {
        supported: shell::supported(),
        installed: shell::is_installed(),
    }
}

#[tauri::command]
fn shell_set(enabled: bool) -> Result<ShellStatus, UiError> {
    let r = if enabled {
        shell::install()
    } else {
        shell::uninstall()
    };
    r.map_err(|e| UiError {
        code: "error",
        message: e.to_string(),
    })?;
    Ok(shell_status())
}

/// Clicks from the right-click menu that are ready for the UI.
#[derive(Default)]
struct Pending(Mutex<Vec<ShellAction>>);

#[tauri::command]
fn take_shell_actions(pending: State<'_, Pending>) -> Vec<ShellAction> {
    std::mem::take(&mut *pending.0.lock().unwrap_or_else(|e| e.into_inner()))
}

fn receive(app: &AppHandle, args: Vec<String>) {
    if let Some(a) = shell::parse_args(args) {
        app.state::<Inbox>().push(a);
    }
}

fn bring_to_front(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        // Must be first: a second launch (a right-click on another file) hands its args to us.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            receive(app, argv.into_iter().skip(1).collect());
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_drag::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init());
    #[cfg(feature = "direct")]
    let builder = builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init());
    builder
        .manage(Jobs::default())
        .manage(Inbox::default())
        .manage(Pending::default())
        .manage(features::SearchIndex::default())
        .manage(features::Watches::default())
        .setup(|app| {
            #[cfg(debug_assertions)]
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_title("Zipkoo (dev)");
            }
            receive(app.handle(), std::env::args().skip(1).collect());
            features::start_scheduler(app.handle().clone());
            // Move merged clicks to the UI once their merge window closes.
            let handle = app.handle().clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_millis(120));
                let ready = handle.state::<Inbox>().take_ready();
                if !ready.is_empty() {
                    handle
                        .state::<Pending>()
                        .0
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .extend(ready);
                    bring_to_front(&handle);
                    let _ = handle.emit("shell-actions", ());
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            inspect_archive,
            peek_file,
            pack_files,
            extract_archive,
            cancel_job,
            hash_file,
            sfx_available,
            suggest_pack_output,
            suggest_extract_dest,
            shell_status,
            shell_set,
            take_shell_actions,
            features::convert_archives,
            features::update_archive,
            features::archive_history,
            features::heal_status,
            features::heal_create,
            features::heal_check,
            features::find_duplicates,
            features::cloud_folders,
            features::list_archives,
            features::index_refresh,
            features::search_archives,
            features::share_file,
            features::shares_list,
            features::share_delete,
            features::share_download,
            features::watches_list,
            features::watch_save,
            features::watch_remove,
            features::watch_run_now,
            features::edit_archive,
            features::open_entry,
            features::prepare_drag,
            features::extract_conflicts,
            features::test_archive,
            features::salvage_zip,
            features::app_info,
            features::trash_path,
            features::passwords_list,
            features::password_add,
            features::password_remove,
            features::password_find,
            features::scan_available,
            features::virus_scan
        ])
        .build(tauri::generate_context!())
        .expect("error while building Zipkoo")
        .run(|_app, _event| {
            // Finder opening a file with Zipkoo (double-click on a .zip, "Open With").
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Opened { urls } = &_event {
                let paths: Vec<String> = urls
                    .iter()
                    .filter_map(|u| u.to_file_path().ok())
                    .map(|p| p.display().to_string())
                    .collect();
                receive(_app, paths);
            }
        });
}
