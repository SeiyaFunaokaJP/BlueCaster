//! What the screens ask of the program around them: its version, opening
//! pages and folders, saving a file, the in-app update, the service add-on;
//! and the connection of ui-host as the store's transport.

use std::path::Path;
use std::sync::Arc;

use gpui_kit::{App, AppContext as _, ClipboardItem, Entity, Global, Task};
use ipc::ErrorInfo;
use serde_json::Value;
use tokio::sync::oneshot;
use ui_host::service::{Outcome, Route};
use ui_host::update::Phase;

use crate::app::{self, NoticeKind};
use crate::core::{ConnStatus, Transport};

/// The UI's own version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The in-app update (ui_host::update::Phase)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdatePhase {
    Unavailable {
        reason: UpdateUnavailable,
    },
    Idle,
    Downloading {
        version: String,
        downloaded: u64,
        total: Option<u64>,
    },
    /// Downloaded and verified; installed as soon as nothing streams
    Waiting {
        version: String,
    },
    Installing {
        version: String,
    },
    Failed {
        code: String,
        detail: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateUnavailable {
    /// Nothing installed to update
    Portable,
    /// Built without the updater key
    Unsigned,
}

/// The service add-on (ui_host::service::Route)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceState {
    NotAdded,
    Running {
        version: String,
    },
    Stopped {
        version: String,
    },
    NeedsUpdate {
        version: String,
    },
    /// Newer than the app: the app needs the update
    AppOlder {
        version: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceAction {
    Install,
    Update,
    Remove,
}

/// How a set-up ended
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceOutcome {
    Done,
    /// The user answered No on Windows' prompt
    Declined,
    /// The set-up tool's exit code
    Failed {
        code: u32,
    },
    NotStarted {
        detail: String,
    },
}

/// How a set-up ended, and the state after it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceSetupResult {
    pub outcome: ServiceOutcome,
    pub service: ServiceState,
}

/// What changes while the program runs; views observe it (`host::state(cx)`)
pub struct HostState {
    pub update: UpdatePhase,
}

struct Globals {
    state: Entity<HostState>,
    /// None with the demo core: nothing to update, no service to set up
    host: Option<Arc<ui_host::Host>>,
}
impl Global for Globals {}

pub fn init(host: Option<Arc<ui_host::Host>>, cx: &mut App) {
    let update = match &host {
        Some(h) => h.update_phase().into(),
        None => UpdatePhase::Unavailable { reason: UpdateUnavailable::Portable },
    };
    let state = cx.new(|_| HostState { update });
    cx.set_global(Globals { state, host });
}

pub fn state(cx: &App) -> Entity<HostState> {
    cx.global::<Globals>().state.clone()
}

/// The connection of ui-host (None with the demo core)
pub fn backend(cx: &App) -> Option<Arc<ui_host::Host>> {
    cx.global::<Globals>().host.clone()
}

/// The update's phase changed (the connection's event)
pub fn on_update_phase(phase: Phase, cx: &mut App) {
    state(cx).update(cx, |s, cx| {
        s.update = phase.into();
        cx.notify();
    });
}

impl From<Phase> for UpdatePhase {
    fn from(p: Phase) -> UpdatePhase {
        match p {
            Phase::Unavailable { reason } => UpdatePhase::Unavailable {
                reason: if reason == "unsigned" { UpdateUnavailable::Unsigned } else { UpdateUnavailable::Portable },
            },
            Phase::Idle => UpdatePhase::Idle,
            Phase::Downloading { version, downloaded, total } => {
                UpdatePhase::Downloading { version, downloaded, total }
            }
            Phase::Waiting { version } => UpdatePhase::Waiting { version },
            Phase::Installing { version } => UpdatePhase::Installing { version },
            Phase::Failed { code, detail } => UpdatePhase::Failed { code, detail },
        }
    }
}

impl From<Route> for ServiceState {
    fn from(r: Route) -> ServiceState {
        match r {
            Route::NotAdded => ServiceState::NotAdded,
            Route::Running { version } => ServiceState::Running { version },
            Route::Stopped { version } => ServiceState::Stopped { version },
            Route::NeedsUpdate { version } => ServiceState::NeedsUpdate { version },
            Route::AppOlder { version } => ServiceState::AppOlder { version },
        }
    }
}

impl From<ui_host::Status> for ConnStatus {
    fn from(s: ui_host::Status) -> ConnStatus {
        use ui_host::Status;
        match s {
            Status::Connecting => ConnStatus::Connecting,
            Status::Connected { server } => ConnStatus::Connected { server },
            Status::CoreMissing { detail } => ConnStatus::CoreMissing { detail },
            Status::Rejected { reason } => ConnStatus::Rejected { reason },
            Status::ServiceNeedsUpdate { version, app } => ConnStatus::ServiceNeedsUpdate { version, app },
            Status::ServiceNewer { version, app } => ConnStatus::ServiceNewer { version, app },
            Status::ServiceStopped => ConnStatus::ServiceStopped,
            Status::ServiceUntrusted { reason } => ConnStatus::ServiceUntrusted { reason },
        }
    }
}

/// The real core, through the connection of ui-host
pub struct Connection(pub Arc<ui_host::Host>);

impl Transport for Connection {
    fn status(&self) -> ConnStatus {
        self.0.status().into()
    }

    fn call(&self, method: String, params: Option<Value>) -> oneshot::Receiver<Result<Value, ErrorInfo>> {
        // One subscription for the process: both windows share the store
        self.0.request("ui", method, params)
    }
}

/// The windows have the Mica backdrop (Windows 11); also what Quick
/// Settings need
pub fn mica() -> bool {
    crate::windows::mica_supported()
}

/// The licence texts shipped with the app (None in a development build)
pub fn licenses_dir() -> Option<String> {
    ui_host::shell::licenses_dir().map(|p| p.display().to_string())
}

fn told(result: Result<(), String>, cx: &mut App) {
    if let Err(e) = result {
        app::notify(e, NoticeKind::Error, cx);
    }
}

/// Runs a shell call on a thread of its own and shows its error as a
/// notice. The shell can run a message loop inside the call
/// (SHOpenFolderAndSelectItems does), which on the window's thread hands
/// the window a message while a click handler still holds the app: a panic
/// that aborts the process (discussion #584).
fn on_shell_thread(call: impl FnOnce() -> Result<(), String> + Send + 'static, cx: &mut App) {
    let (tx, rx) = oneshot::channel();
    let spawned = std::thread::Builder::new().name("shell-open".into()).spawn(move || {
        let _ = tx.send(call());
    });
    if let Err(e) = spawned {
        app::notify(e.to_string(), NoticeKind::Error, cx);
        return;
    }
    cx.spawn(async move |cx: &mut gpui_kit::AsyncApp| {
        if let Ok(Err(e)) = rx.await {
            cx.update(|cx| app::notify(e, NoticeKind::Error, cx));
        }
    })
    .detach();
}

/// Opens a web page in the user's browser (https only)
pub fn open_url(url: &str, cx: &mut App) {
    let url = url.to_owned();
    on_shell_thread(move || ui_host::shell::open_url(&url), cx);
}

/// Opens a folder in Explorer, creating it first if it is not there yet
pub fn open_folder(path: &str, cx: &mut App) {
    let path = path.to_owned();
    on_shell_thread(move || ui_host::shell::open_folder(Path::new(&path)), cx);
}

/// Shows a file in Explorer, selected
pub fn reveal_path(path: &str, cx: &mut App) {
    let path = path.to_owned();
    on_shell_thread(move || ui_host::shell::reveal_path(Path::new(&path)), cx);
}

/// Saves `text` to a .txt file picked in the Save As dialog, `name`
/// suggested -> false when cancelled; the error's text when it failed
pub fn save_text_file(name: &str, text: String, cx: &App) -> Task<Result<bool, String>> {
    let rx = ui_host::shell::save_text_file(crate::windows::main_hwnd(cx), name.to_owned(), text);
    cx.background_spawn(async move { rx.await.unwrap_or_else(|_| Err("the dialog did not open".into())) })
}

/// Puts `text` on the clipboard
pub fn copy_text(text: impl Into<String>, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(text.into()));
}

/// Downloads the release of `manifest_url` (UpdateInfo.manifest_url) and
/// installs it once nothing streams; the app restarts
pub fn update_install(manifest_url: &str, cx: &mut App) {
    let Some(host) = backend(cx) else { return };
    told(host.update_install(manifest_url.to_owned()), cx);
}

/// The service add-on's state
pub fn service_state(cx: &App) -> Task<ServiceState> {
    let Some(host) = backend(cx) else { return Task::ready(ServiceState::NotAdded) };
    let rx = host.service_state();
    cx.background_spawn(async move {
        match rx.await {
            Ok(Ok(route)) => route.into(),
            _ => ServiceState::NotAdded,
        }
    })
}

/// Adds, updates or removes the service add-on (Windows asks for an
/// administrator) and waits for it; `remove_data` only with Remove
pub fn service_setup(action: ServiceAction, remove_data: bool, cx: &App) -> Task<ServiceSetupResult> {
    let not_started = |detail: String| ServiceSetupResult {
        outcome: ServiceOutcome::NotStarted { detail },
        service: ServiceState::NotAdded,
    };
    let Some(host) = backend(cx) else { return Task::ready(not_started(String::new())) };
    let action = match action {
        ServiceAction::Install => "install",
        ServiceAction::Update => "update",
        ServiceAction::Remove => "remove",
    };
    let rx = host.service_setup(action.to_owned(), remove_data);
    cx.background_spawn(async move {
        match rx.await {
            Ok(Ok(r)) => ServiceSetupResult {
                outcome: match r.outcome {
                    Outcome::Done => ServiceOutcome::Done,
                    Outcome::Declined => ServiceOutcome::Declined,
                    Outcome::Failed { code } => ServiceOutcome::Failed { code },
                    Outcome::NotStarted { detail } => ServiceOutcome::NotStarted { detail },
                },
                service: r.service.into(),
            },
            Ok(Err(detail)) => not_started(detail),
            Err(_) => not_started(String::new()),
        }
    })
}
