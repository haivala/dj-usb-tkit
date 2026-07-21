#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::{Mutex, OnceLock, mpsc};
use std::{fs, io::Write, path::Path, path::PathBuf, process::Command};

use backend::commands::BackendCommands;
use serde::Serialize;
use tauri::Emitter;
use tauri::Manager;
use tauri::webview::Color;
use tauri_plugin_dialog::{DialogExt, FilePath};

static LOG_DIR: OnceLock<PathBuf> = OnceLock::new();
static BACKEND_LOG_BUFFER: OnceLock<Mutex<Vec<BackendLogPayload>>> = OnceLock::new();
const BACKEND_LOG_EVENT_CHANNEL: &str = "backend:log";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BackendLogPayload {
    level: String,
    source: String,
    message: String,
    timestamp: String,
}

fn push_backend_log_entry(
    level: &str,
    source: &str,
    message: impl Into<String>,
) -> BackendLogPayload {
    let payload = BackendLogPayload {
        level: level.to_string(),
        source: source.to_string(),
        message: message.into(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    };
    let buf = BACKEND_LOG_BUFFER.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(mut guard) = buf.lock() {
        guard.push(payload.clone());
        if guard.len() > 2000 {
            let drop_count = guard.len() - 2000;
            guard.drain(0..drop_count);
        }
    }
    payload
}

fn emit_backend_log(app: &tauri::AppHandle, level: &str, source: &str, message: impl Into<String>) {
    let payload = push_backend_log_entry(level, source, message.into());
    let _ = app.emit(BACKEND_LOG_EVENT_CHANNEL, payload);
}

fn normalize_selected_path(value: &str) -> Option<PathBuf> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(url) = url::Url::parse(trimmed)
        && url.scheme() == "file"
        && let Ok(path) = url.to_file_path()
    {
        return Some(path);
    }
    Some(PathBuf::from(trimmed))
}

fn allow_asset_scope_path(app: &tauri::AppHandle, path: &Path) -> Result<(), String> {
    let scope = app.asset_protocol_scope();
    let candidate = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    scope.allow_directory(&candidate, true).map_err(|err| {
        format!(
            "allow asset scope failed for {}: {err}",
            candidate.display()
        )
    })
}

#[tauri::command]
async fn pick_source_folders(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    let (tx, rx) = mpsc::channel::<Option<Vec<FilePath>>>();

    app.dialog()
        .file()
        .set_title("Select media source folders")
        .pick_folders(move |folders| {
            let _ = tx.send(folders);
        });

    let selected = tauri::async_runtime::spawn_blocking(move || rx.recv())
        .await
        .map_err(|err| format!("dialog task failed: {err}"))?
        .map_err(|err| format!("dialog response failed: {err}"))?
        .unwrap_or_default();

    let paths = selected
        .into_iter()
        .map(|path| match path {
            FilePath::Path(path_buf) => path_buf.to_string_lossy().to_string(),
            FilePath::Url(url) => url.to_string(),
        })
        .collect::<Vec<_>>();

    for path in &paths {
        if let Some(pb) = normalize_selected_path(path) {
            let _ = allow_asset_scope_path(&app, &pb);
        }
    }

    Ok(paths)
}

#[tauri::command]
async fn pick_usb_folder(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let (tx, rx) = mpsc::channel::<Option<FilePath>>();

    app.dialog()
        .file()
        .set_title("Select USB root folder")
        .pick_folder(move |folder| {
            let _ = tx.send(folder);
        });

    let selected = tauri::async_runtime::spawn_blocking(move || rx.recv())
        .await
        .map_err(|err| format!("dialog task failed: {err}"))?
        .map_err(|err| format!("dialog response failed: {err}"))?;

    let path = selected.map(|path| match path {
        FilePath::Path(path_buf) => path_buf.to_string_lossy().to_string(),
        FilePath::Url(url) => url.to_string(),
    });

    if let Some(ref selected_path) = path
        && let Some(pb) = normalize_selected_path(selected_path)
    {
        let _ = allow_asset_scope_path(&app, &pb);
    }

    Ok(path)
}

#[tauri::command]
async fn allow_asset_paths(app: tauri::AppHandle, paths: Vec<String>) -> Result<usize, String> {
    let mut allowed = 0usize;
    for value in paths {
        let Some(path) = normalize_selected_path(&value) else {
            continue;
        };
        if allow_asset_scope_path(&app, &path).is_ok() {
            allowed += 1;
        }
    }
    Ok(allowed)
}

fn frontend_log_path() -> PathBuf {
    LOG_DIR
        .get()
        .cloned()
        .unwrap_or_else(|| PathBuf::from(".app-data"))
        .join("frontend-console.log")
}

fn quarantine_backend_db_files(data_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut moved = Vec::<PathBuf>::new();
    for suffix in ["", "-wal", "-shm"] {
        let src = data_dir.join(format!("backend.db{suffix}"));
        if !src.exists() {
            continue;
        }
        let dst = data_dir.join(format!("backend.db{suffix}.quarantine-{stamp}"));
        fs::rename(&src, &dst)
            .map_err(|err| format!("failed to quarantine {}: {err}", src.display()))?;
        moved.push(dst);
    }
    Ok(moved)
}

fn quarantine_frontend_state_files(data_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut moved = Vec::<PathBuf>::new();

    let candidates = [
        "local.db",
        "local.db-wal",
        "local.db-shm",
        "Local Storage",
        "Session Storage",
        "WebKit",
        "WebKitNetworkProcess",
        "Cache",
        "GPUCache",
    ];

    for name in candidates {
        let src = data_dir.join(name);
        if !src.exists() {
            continue;
        }
        let dst = data_dir.join(format!("{}.quarantine-{stamp}", name.replace('/', "_")));
        fs::rename(&src, &dst)
            .map_err(|err| format!("failed to quarantine {}: {err}", src.display()))?;
        moved.push(dst);
    }

    Ok(moved)
}

fn safe_reset_requested() -> bool {
    if std::env::var("DJ_USB_TKIT_SAFE_RESET")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        return true;
    }
    std::env::args().any(|arg| arg == "--safe-reset")
}

fn init_backend_commands_with_recovery(data_dir: &Path) -> Result<BackendCommands, String> {
    match BackendCommands::new(data_dir) {
        Ok(commands) => Ok(commands),
        Err(first_err) => {
            let moved = quarantine_backend_db_files(data_dir)?;
            if moved.is_empty() {
                return Err(format!("backend init failed: {}", first_err.message));
            }
            BackendCommands::new(data_dir).map_err(|retry_err| {
                format!(
                    "backend init failed after DB recovery (data dir: {}): {}",
                    data_dir.display(),
                    retry_err.message
                )
            })
        }
    }
}

fn configure_desktop_analysis_runtime(_app: &tauri::AppHandle) -> Result<(), String> {
    if std::env::var_os("DJTKIT_ENABLE_ESSENTIA_JS").is_none() {
        // SAFETY: set once at startup before backend worker threads are spawned.
        unsafe { std::env::set_var("DJTKIT_ENABLE_ESSENTIA_JS", "1") };
    }

    let essentia_enabled = std::env::var("DJTKIT_ENABLE_ESSENTIA_JS")
        .ok()
        .map(|v| !(v == "0" || v.eq_ignore_ascii_case("false")))
        .unwrap_or(true);
    if !essentia_enabled {
        emit_backend_log(
            _app,
            "info",
            "startup",
            "analysis runtime setup skipped: DJTKIT_ENABLE_ESSENTIA_JS=0",
        );
        return Ok(());
    }
    let is_file = |p: &Path| p.is_file();
    let desktop_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.to_path_buf());
    let resource_dir = _app.path().resource_dir().ok();
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));

    let mut resolved_node = std::env::var_os("DJTKIT_ESSENTIA_NODE")
        .map(|v| PathBuf::from(v.to_string_lossy().to_string()));
    if resolved_node.is_none() {
        let mut runtime_candidates = Vec::<PathBuf>::new();
        if let Some(dir) = resource_dir.as_ref() {
            runtime_candidates.push(dir.join("bin").join("node"));
            runtime_candidates.push(dir.join("bin").join("node.exe"));
            runtime_candidates.push(dir.join("node"));
            runtime_candidates.push(dir.join("node.exe"));
            runtime_candidates.push(dir.join("_up_").join("runtime").join("bin").join("node"));
            runtime_candidates.push(
                dir.join("_up_")
                    .join("runtime")
                    .join("bin")
                    .join("node.exe"),
            );
            runtime_candidates.push(
                dir.join("DJ_USB_Tkit")
                    .join("_up_")
                    .join("runtime")
                    .join("bin")
                    .join("node"),
            );
            runtime_candidates.push(
                dir.join("DJ_USB_Tkit")
                    .join("_up_")
                    .join("runtime")
                    .join("bin")
                    .join("node.exe"),
            );
        }
        if let Some(dir) = exe_dir.as_ref() {
            runtime_candidates.push(dir.join("node"));
            runtime_candidates.push(dir.join("node.exe"));
        }
        if let Some(dir) = desktop_root.as_ref() {
            runtime_candidates.push(dir.join("runtime").join("bin").join("node"));
            runtime_candidates.push(dir.join("runtime").join("bin").join("node.exe"));
        }

        if let Some(runtime) = runtime_candidates
            .into_iter()
            .find(|p| is_file(p.as_path()))
        {
            // SAFETY: set once at startup before backend worker threads are spawned.
            unsafe {
                std::env::set_var(
                    "DJTKIT_ESSENTIA_NODE",
                    runtime.to_string_lossy().to_string(),
                )
            };
            resolved_node = Some(runtime);
        }

        // Fall back to system PATH if no bundled node was found.
        if resolved_node.is_none()
            && let Ok(output) = std::process::Command::new("which").arg("node").output()
            && output.status.success()
        {
            let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path_str.is_empty() {
                let path = PathBuf::from(&path_str);
                if path.is_file() {
                    resolved_node = Some(path);
                }
            }
        }
    }

    let mut resolved_runner = std::env::var_os("DJTKIT_ESSENTIA_RUNNER")
        .map(|v| PathBuf::from(v.to_string_lossy().to_string()));
    if resolved_runner.is_none() {
        let mut runner_candidates = Vec::<PathBuf>::new();
        if let Some(dir) = resource_dir.as_ref() {
            runner_candidates.push(dir.join("scripts").join("essentia_runner.cjs"));
            runner_candidates.push(dir.join("essentia_runner.cjs"));
            runner_candidates.push(dir.join("_up_").join("scripts").join("essentia_runner.cjs"));
            runner_candidates.push(
                dir.join("DJ_USB_Tkit")
                    .join("_up_")
                    .join("scripts")
                    .join("essentia_runner.cjs"),
            );
        }
        if let Some(dir) = desktop_root.as_ref() {
            runner_candidates.push(dir.join("scripts").join("essentia_runner.cjs"));
        }
        if let Some(runner) = runner_candidates.into_iter().find(|p| is_file(p.as_path())) {
            // SAFETY: set once at startup before backend worker threads are spawned.
            unsafe {
                std::env::set_var(
                    "DJTKIT_ESSENTIA_RUNNER",
                    runner.to_string_lossy().to_string(),
                )
            };
            resolved_runner = Some(runner);
        }
    }

    let mut resolved_modules = std::env::var_os("DJTKIT_ESSENTIA_NODE_MODULES")
        .map(|v| PathBuf::from(v.to_string_lossy().to_string()));
    if resolved_modules.is_none()
        && let Ok(app_data) = _app.path().app_data_dir()
    {
        let modules_dir = app_data.join("essentia").join("node_modules");
        if modules_dir.join("essentia.js").is_dir() {
            // SAFETY: set once at startup before backend worker threads are spawned.
            unsafe {
                std::env::set_var(
                    "DJTKIT_ESSENTIA_NODE_MODULES",
                    modules_dir.to_string_lossy().to_string(),
                )
            };
            resolved_modules = Some(modules_dir);
        }
    }

    emit_backend_log(
        _app,
        "info",
        "startup",
        format!(
            "analysis runtime resolved: node={} runner={} modules={}",
            resolved_node
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "not found".to_string()),
            resolved_runner
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "not found".to_string()),
            resolved_modules
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "not found".to_string())
        ),
    );

    let node_bin = std::env::var("DJTKIT_ESSENTIA_NODE").unwrap_or_else(|_| "node".to_string());
    let runner_script = std::env::var("DJTKIT_ESSENTIA_RUNNER")
        .unwrap_or_else(|_| "../desktop/scripts/essentia_runner.cjs".to_string());
    let runner_path = PathBuf::from(&runner_script);
    if !runner_path.is_file() {
        emit_backend_log(
            _app,
            "warn",
            "startup",
            format!(
                "analysis runtime unavailable: runner missing at {}; essentia disabled",
                runner_path.display()
            ),
        );
        return Ok(());
    }

    let probe = match Command::new(&node_bin)
        .arg(runner_path.to_string_lossy().to_string())
        .arg(r#"{"sampleRate":44100}"#)
        .output()
    {
        Ok(output) => output,
        Err(err) => {
            emit_backend_log(
                _app,
                "warn",
                "startup",
                format!(
                    "analysis runtime unavailable: failed to start node '{}' with runner '{}': {err}; essentia disabled",
                    node_bin,
                    runner_path.display()
                ),
            );
            return Ok(());
        }
    };

    if !probe.status.success() {
        emit_backend_log(
            _app,
            "warn",
            "startup",
            format!(
                "analysis runtime unavailable: node '{}' failed to execute runner '{}' (status: {}); essentia disabled",
                node_bin,
                runner_path.display(),
                probe.status
            ),
        );
        return Ok(());
    }

    let probe_stdout = String::from_utf8_lossy(&probe.stdout);
    if !(probe_stdout.contains("\"ok\":false") && probe_stdout.contains("missing pcm payload")) {
        emit_backend_log(
            _app,
            "warn",
            "startup",
            format!(
                "analysis runtime unavailable: unexpected runner probe output from '{}': {}; essentia disabled",
                runner_path.display(),
                probe_stdout.trim()
            ),
        );
        return Ok(());
    }

    Ok(())
}

#[tauri::command]
fn clear_frontend_log() -> Result<String, String> {
    let path = frontend_log_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("create log dir failed: {err}"))?;
    }
    fs::write(&path, b"").map_err(|err| format!("clear log failed: {err}"))?;
    Ok(path.to_string_lossy().to_string())
}

const VALID_LOG_LEVELS: &[&str] = &["log", "info", "warn", "error", "debug", "trace"];

#[tauri::command]
fn append_frontend_log(level: String, message: String) -> Result<(), String> {
    let sanitized_level = if VALID_LOG_LEVELS.contains(&level.as_str()) {
        level
    } else {
        "info".to_string()
    };
    let sanitized_message = message.replace('\n', "\\n").replace('\r', "\\r");

    let path = frontend_log_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("create log dir failed: {err}"))?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|err| format!("open log failed: {err}"))?;
    let now = chrono::Utc::now().to_rfc3339();
    writeln!(file, "[{now}] [{sanitized_level}] {sanitized_message}")
        .map_err(|err| format!("append log failed: {err}"))?;
    Ok(())
}

#[tauri::command]
fn get_backend_log_buffer() -> Result<Vec<BackendLogPayload>, String> {
    let buf = BACKEND_LOG_BUFFER.get_or_init(|| Mutex::new(Vec::new()));
    let guard = buf
        .lock()
        .map_err(|err| format!("backend log buffer lock failed: {err}"))?;
    Ok(guard.clone())
}

#[tauri::command]
async fn show_window(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("main") {
        w.show().map_err(|e| format!("show failed: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
async fn set_theme_background(app: tauri::AppHandle, dark: bool) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("main") {
        let color = if dark {
            Color(12, 10, 18, 255) // #0c0a12
        } else {
            Color(240, 237, 245, 255) // #f0edf5
        };
        w.set_background_color(Some(color))
            .map_err(|e| format!("set background failed: {e}"))?;
    }
    Ok(())
}

/// Install the project-wide backend log sink so `backend::logging::emit`
/// from anywhere in the lib forwards into the UI Event Log.
fn install_backend_log_sink(app: tauri::AppHandle) {
    backend::logging::set_sink(Box::new(move |level, source, message| {
        emit_backend_log(&app, level.as_str(), source, message);
    }));
}

/// Install a panic hook that pushes the panic info to the Event Log before
/// re-running the default panic handler (so the process still aborts/logs
/// to stderr as usual). Without this, panics in worker threads vanish from
/// the UI.
fn install_backend_panic_hook(app: tauri::AppHandle) {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "unknown location".to_string());
        let payload_str = if let Some(s) = info.payload().downcast_ref::<&'static str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "non-string panic payload".to_string()
        };
        emit_backend_log(
            &app,
            "error",
            "panic",
            format!("panic at {location}: {payload_str}"),
        );
        prev(info);
    }));
}

fn main() {
    #[cfg(target_os = "linux")]
    {
        // Suppress a noisy GTK/GIO warning emitted by some folder listings:
        // "GLib-GIO-CRITICAL ... g_file_info_get_size: should not be reached"
        // This is a native dialog warning and does not affect app behavior.
        glib::log_set_default_handler(|domain, level, message| {
            let is_known_noise = domain == Some("GLib-GIO")
                && message.contains("g_file_info_get_size")
                && message.contains("should not be reached");
            if is_known_noise {
                return;
            }
            glib::log_default_handler(domain, level, Some(message));
        });

        // AppImage and some Wayland stacks can fail GBM buffer allocation in WebKit.
        // Keep user overrides intact; only apply this workaround for AppImage.
        if std::env::var_os("APPIMAGE").is_some()
            && std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none()
        {
            // SAFETY: set once during startup before worker threads are spawned.
            unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let app_handle = app.handle().clone();

            // Route every backend log line and every panic into the UI Event
            // Log via the existing `backend:log` event channel.
            install_backend_log_sink(app_handle.clone());
            install_backend_panic_hook(app_handle.clone());

            configure_desktop_analysis_runtime(&app_handle)?;
            let fallback = std::env::current_dir()
                .map(|p| p.join(".app-data"))
                .unwrap_or_else(|_| PathBuf::from(".app-data"));
            let data_dir = app.path().app_data_dir().unwrap_or(fallback);
            emit_backend_log(
                &app_handle,
                "info",
                "startup",
                format!("app data dir: {}", data_dir.display()),
            );

            if safe_reset_requested() {
                let mut quarantined = quarantine_backend_db_files(&data_dir)?;
                let mut frontend_quarantined = quarantine_frontend_state_files(&data_dir)?;
                quarantined.append(&mut frontend_quarantined);
                if quarantined.is_empty() {
                    emit_backend_log(
                        &app_handle,
                        "warn",
                        "startup",
                        "safe reset requested, but no state files were found to quarantine",
                    );
                } else {
                    emit_backend_log(
                        &app_handle,
                        "warn",
                        "startup",
                        format!(
                            "safe reset quarantined: {}",
                            quarantined
                                .iter()
                                .map(|p| p.to_string_lossy().to_string())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    );
                }
            }

            // Store data dir for frontend log path resolution
            let _ = LOG_DIR.set(data_dir.clone());

            let commands = init_backend_commands_with_recovery(&data_dir)?;
            emit_backend_log(
                &app_handle,
                "info",
                "startup",
                "startup: backend initialized",
            );
            backend::tauri_commands::start_playback_event_pump(
                app.handle().clone(),
                commands.clone(),
            );
            emit_backend_log(
                &app_handle,
                "info",
                "startup",
                "startup: playback event pump started",
            );
            app.manage(commands);
            app.manage(backend::tauri_commands::EssentiaDownloadCancel(
                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            ));
            emit_backend_log(
                &app_handle,
                "info",
                "startup",
                "startup: backend state managed",
            );

            // Set webview background to dark purple to prevent white flash.
            // Window starts hidden (config), we show it after a brief delay
            // so the dark CSS has time to paint before the window appears.
            if let Some(window) = app.get_webview_window("main") {
                // Must match CSS --bg (#0c0a12) in styles.css
                let _ = window.set_background_color(Some(Color(12, 10, 18, 255)));
                emit_backend_log(
                    &app_handle,
                    "info",
                    "startup",
                    "startup: window background set",
                );

                // Set window icon from embedded PNG
                let icon_bytes = include_bytes!("../icons/icon.png");
                match tauri::image::Image::from_bytes(icon_bytes) {
                    Ok(icon) => {
                        if let Err(e) = window.set_icon(icon) {
                            emit_backend_log(
                                &app_handle,
                                "warn",
                                "startup",
                                format!("set_icon failed: {e}"),
                            );
                        } else {
                            emit_backend_log(
                                &app_handle,
                                "info",
                                "startup",
                                "startup: window icon set",
                            );
                        }
                    }
                    Err(e) => emit_backend_log(
                        &app_handle,
                        "warn",
                        "startup",
                        format!("icon decode failed: {e}"),
                    ),
                }
            }
            emit_backend_log(&app_handle, "info", "startup", "startup: setup completed");

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            backend::tauri_commands::scan_library,
            backend::tauri_commands::scan_master_db,
            backend::tauri_commands::search_tracks,
            backend::tauri_commands::list_tracks,
            backend::tauri_commands::browse_source_files,
            backend::tauri_commands::check_source_roots,
            backend::tauri_commands::materialize_source_track,
            backend::tauri_commands::remove_tracks_by_source_roots,
            backend::tauri_commands::relocate_source_root,
            backend::tauri_commands::get_system_parallelism,
            backend::tauri_commands::get_tracks_by_ids_with_previews,
            backend::tauri_commands::resolve_playback_source,
            backend::tauri_commands::create_playlist,
            backend::tauri_commands::rename_playlist,
            backend::tauri_commands::delete_playlist,
            backend::tauri_commands::list_playlists,
            backend::tauri_commands::get_playlist_tracks,
            backend::tauri_commands::add_tracks_to_playlist,
            backend::tauri_commands::remove_tracks_from_playlist,
            backend::tauri_commands::get_frontend_settings,
            backend::tauri_commands::set_frontend_setting,
            backend::tauri_commands::validate_usb_root,
            backend::tauri_commands::fetch_usb_playlists,
            backend::tauri_commands::fetch_usb_histories,
            backend::tauri_commands::get_usb_player_menu_config,
            backend::tauri_commands::update_usb_player_menu_config,
            backend::tauri_commands::sync_usb_player_menu_edb_to_pdb,
            backend::tauri_commands::remove_usb_playlist,
            backend::tauri_commands::inspect_usb_track,
            backend::tauri_commands::analyze_new_tracks,
            backend::tauri_commands::analyze_track_piece,
            backend::tauri_commands::export_to_usb,
            backend::tauri_commands::run_usb_diagnostics,
            backend::tauri_commands::run_usb_parity_report,
            backend::tauri_commands::repair_usb_diagnostics,
            backend::tauri_commands::detect_external_master_db,
            backend::tauri_commands::initialize_usb,
            backend::tauri_commands::download_essentia,
            backend::tauri_commands::cancel_essentia_download,
            backend::tauri_commands::remove_essentia,
            backend::tauri_commands::play_track_native,
            backend::tauri_commands::stop_playback_native,
            backend::tauri_commands::get_playback_status_native,
            backend::tauri_commands::playback_preflight_native,
            clear_frontend_log,
            append_frontend_log,
            get_backend_log_buffer,
            pick_source_folders,
            pick_usb_folder,
            allow_asset_paths,
            show_window,
            set_theme_background
        ])
        .run(tauri::generate_context!())
        .expect("failed to run tauri application");
}
