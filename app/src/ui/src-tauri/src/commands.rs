//! The commands the frontend calls (`invoke`). They check their arguments:
//! the webview is the less trusted side, and opening URLs or paths is
//! limited to what the screens need.

use std::path::Path;
use std::sync::Arc;

use ipc::ErrorInfo;
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, State, WebviewWindow};

use crate::connection::{Ipc, Status};
use crate::service;
use crate::update::{self, Phase, Updates};
use crate::windows;

/// Sends a request to the core -> its result, or the core's ErrorInfo
/// (codes "ipc.*" when the connection fails)
#[tauri::command]
pub async fn ipc_request(
    window: WebviewWindow,
    ipc: State<'_, Arc<Ipc>>,
    method: String,
    params: Option<Value>,
) -> Result<Value, ErrorInfo> {
    let result = ipc.request(window.label(), &method, params).await;
    // Only the connection's failures: the core's own errors are in its log
    if let Err(e) = &result
        && e.code.starts_with("ipc.")
    {
        tracing::warn!("[{}] {method} failed: {}", window.label(), e.code);
    }
    result
}

#[tauri::command]
pub fn ipc_status(ipc: State<'_, Arc<Ipc>>) -> Status {
    ipc.status()
}

#[derive(Serialize)]
pub struct UiInfo {
    /// "main" or "flyout"
    window: String,
    version: &'static str,
    /// The window has the Mica backdrop: the page keeps its background
    /// translucent
    mica: bool,
    /// The licence texts next to the programs (installer and portable zip,
    /// installer/build-release.ps1); None in a development build
    licenses_dir: Option<String>,
}

#[tauri::command]
pub fn ui_info(window: WebviewWindow) -> UiInfo {
    UiInfo {
        window: window.label().into(),
        version: env!("CARGO_PKG_VERSION"),
        mica: windows::mica_supported(),
        licenses_dir: licenses_dir(),
    }
}

fn licenses_dir() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.join("licenses");
    dir.is_dir().then(|| dir.display().to_string())
}

// Async: a synchronous command runs on the main thread, and creating a
// webview window from there deadlocks on Windows
#[tauri::command]
pub async fn show_main(app: AppHandle, route: Option<String>) -> Result<(), String> {
    windows::show_main(&app, route).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn show_flyout(app: AppHandle) -> Result<(), String> {
    windows::show_flyout(&app).map_err(|e| e.to_string())
}

/// The flyout's page height (logical pixels): the flyout window fits it
#[tauri::command]
pub async fn fit_flyout(window: WebviewWindow, height: f64) -> Result<(), String> {
    windows::fit_flyout(&window, height).map_err(|e| e.to_string())
}

/// The first-run setup opened or closed: the main window fits it
#[tauri::command]
pub async fn setup_window(window: WebviewWindow, active: bool) -> Result<(), String> {
    windows::setup_window(&window, active).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_theme(app: AppHandle, theme: String) {
    windows::set_theme(&app, &theme);
}

#[derive(Serialize)]
pub struct Accent {
    light: String,
    dark: String,
}

/// Windows' accent colours (read again when the window gets the focus, so a
/// change in Settings shows); None keeps the UI's default
#[tauri::command]
pub fn accent_colors() -> Option<Accent> {
    win_sys::accent::accent_colors().map(|a| Accent { light: a.light, dark: a.dark })
}

/// Opens a web page in the user's browser (https only)
#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("only https URLs are opened".into());
    }
    tauri_plugin_opener::open_url(url, None::<&str>).map_err(|e| e.to_string())
}

/// Opens a folder in Explorer (the settings folder), creating it first if it
/// is not there yet, as v1's Open Config Folder did (the firmware files go
/// there before anything else wrote to it). Folders only: a file path would
/// run the file's program.
#[tauri::command]
pub fn open_folder(path: String) -> Result<(), String> {
    let p = Path::new(&path);
    if !p.is_absolute() {
        return Err(format!("{path} is not a folder"));
    }
    if !p.exists() {
        std::fs::create_dir_all(p).map_err(|e| format!("{path}: {e}"))?;
    }
    if !p.is_dir() {
        return Err(format!("{path} is not a folder"));
    }
    tauri_plugin_opener::open_path(p, None::<&str>).map_err(|e| e.to_string())
}

/// Shows a file selected in Explorer (an HCI capture)
#[tauri::command]
pub fn reveal_path(path: String) -> Result<(), String> {
    let p = Path::new(&path);
    if !p.is_absolute() || !p.exists() {
        return Err(format!("{path} does not exist"));
    }
    tauri_plugin_opener::reveal_item_in_dir(p).map_err(|e| e.to_string())
}

/// Longest text [`save_text_file`] writes
const SAVE_TEXT_MAX: usize = 1 << 20;

/// Writes `text` to a .txt file the user picks in Windows' Save As dialog
/// (`name`: the file name suggested; an exported EQ) -> false when the
/// dialog was cancelled. Only where the user chose: the page names no path.
#[tauri::command]
pub async fn save_text_file(window: WebviewWindow, name: String, text: String) -> Result<bool, String> {
    if text.len() > SAVE_TEXT_MAX || name.is_empty() || name.contains(['\\', '/', ':']) {
        return Err("not a file name, or too long".into());
    }
    let owner = window.hwnd().map(|h| h.0 as isize).unwrap_or(0);
    let (tx, rx) = tokio::sync::oneshot::channel();
    // The dialog blocks: a thread of its own, the window's event loop runs on
    std::thread::spawn(move || {
        let _ = tx.send(win_sys::file_dialog::save_file(owner, &name, "txt"));
    });
    let picked = rx.await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
    let Some(path) = picked else { return Ok(false) };
    std::fs::write(&path, &text).map_err(|e| e.to_string())?;
    tracing::info!("[{}] text file saved ({} bytes)", window.label(), text.len());
    Ok(true)
}

/// The in-app update's state (also sent as `update://phase`). It updates
/// the user's own install, also while the window uses the service add-on:
/// the service has its own copy, which then needs an update (service.rs).
#[tauri::command]
pub fn update_status(updates: State<'_, Arc<Updates>>) -> Phase {
    updates.phase()
}

/// The service add-on's state (read-only queries)
#[tauri::command]
pub async fn service_state() -> Result<service::Route, String> {
    tauri::async_runtime::spawn_blocking(service::current).await.map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct SetupResult {
    #[serde(flatten)]
    outcome: service::Outcome,
    service: service::Route,
}

/// Adds ("install"), updates ("update") or removes ("remove", with
/// `remove_data` its data too) the service add-on: the per-user install's
/// set-up tool runs as administrator (Windows' prompt) and is waited for.
/// Then the state is read again.
#[tauri::command]
pub async fn service_setup(ipc: State<'_, Arc<Ipc>>, action: String, remove_data: bool) -> Result<SetupResult, String> {
    let action = service::Action::parse(&action, remove_data)?;
    let before = tauri::async_runtime::spawn_blocking(service::current).await.map_err(|e| e.to_string())?;
    action.allowed(&before)?;
    let ipc = ipc.inner().clone();
    // The core that stops for the set-up says Bye: the window stays
    ipc.set_switching(true);
    let outcome = tauri::async_runtime::spawn_blocking(move || service::run_setup(action))
        .await
        .unwrap_or_else(|e| service::Outcome::NotStarted { detail: e.to_string() });
    let route = tauri::async_runtime::spawn_blocking(service::current).await.map_err(|e| e.to_string())?;
    // Added (running or still starting) or gone: the other core serves next
    let added = route != service::Route::NotAdded;
    let switched = outcome == service::Outcome::Done && ipc.service() != added;
    if switched && !ipc.service() {
        // Added: the user's own core hands the adapter to the service and
        // exits; the window connects to the service next
        let _ = ipc.request("main", "app.quit", None).await;
    } else if !switched {
        // Nothing to connect to again: a later Bye ends the window as usual
        ipc.set_switching(false);
    }
    Ok(SetupResult { outcome, service: route })
}

/// Downloads the update of `manifest_url` (UpdateInfo.manifest_url) and
/// installs it as soon as nothing streams. Only this repository's release
/// manifests are accepted.
#[tauri::command]
pub fn update_install(app: AppHandle, manifest_url: String) -> Result<(), String> {
    update::start(&app, manifest_url)
}
