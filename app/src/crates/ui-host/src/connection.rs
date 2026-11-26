//! The connection to bluecaster-core.exe: the service's pipe when a core
//! runs as a service (checked first: [`open_service`]), else the user's
//! Named Pipe; Hello, every
//! server message forwarded to the window as [`HostEvent::Message`],
//! requests matched to their responses by id, and reconnecting when the pipe
//! breaks (the core restarted or crashed).
//!
//! The host keeps no state of the core beyond what the transport needs:
//! the requests waiting for their answer and the diagnostics subscriptions
//! of each window (the core has one subscription per connection; both
//! windows share this connection). Snapshots, events and revisions are the
//! window's.
//!
//! The host runs on a tokio runtime of its own ([`start`]); the window's
//! toolkit needs no tokio context. Events go out on an unbounded channel,
//! answers to requests on oneshot channels: their receivers work on any
//! executor.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use ipc::{ClientMessage, ErrorInfo, LogLevel, Method, Msg, PROTOCOL_VERSION};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};
use tokio::sync::{mpsc, oneshot};

use crate::APP_VERSION;
use crate::service::Route;
use crate::update::{Phase, Updates};

/// All pipe instances busy: try again shortly
const ERROR_PIPE_BUSY: i32 = 231;
/// app.run_as_admin: the user may take a while on the UAC prompt, and the
/// call itself may take minutes (a driver change)
const ADMIN_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// How long the core may take to create its pipe after we started it
const CORE_START_WAIT: Duration = Duration::from_secs(8);
/// Starting the core again after it did not come up
const CORE_RESPAWN_INTERVAL: Duration = Duration::from_secs(30);
/// Reading the service add-on's state again while it is stopped or needs
/// an update
const SERVICE_RECHECK: Duration = Duration::from_secs(2);

/// What the host tells the window
#[derive(Debug, Clone, PartialEq)]
pub enum HostEvent {
    /// Every server message, as the JSON the core sent. Read as plain JSON
    /// (not `ipc::ServerMessage`): a newer minor version of the core may
    /// send event kinds this build does not know; the window ignores them
    /// instead of the connection failing here.
    Message(Value),
    /// The connection's [`Status`] changed
    Status(Status),
    /// The in-app update's [`Phase`] changed (also its download progress)
    UpdatePhase(Phase),
    /// The core said Bye (it is shutting down on purpose: tray Exit,
    /// app.quit) or closed this user's UI (service mode): the window
    /// process ends
    Exit,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Status {
    /// Looking for the core's pipe (also while it starts)
    Connecting,
    Connected {
        server: String,
    },
    /// bluecaster-core.exe is not next to bluecaster.exe, or did not start
    CoreMissing {
        detail: String,
    },
    /// The core speaks another protocol version
    Rejected {
        reason: String,
    },
    /// The service add-on is another version than this window (or its
    /// registration is wrong): no core runs for the window until it is
    /// updated (service.rs)
    ServiceNeedsUpdate {
        version: String,
        app: String,
    },
    /// The service add-on is newer than this window: this app needs the
    /// update
    ServiceNewer {
        version: String,
        app: String,
    },
    /// The service add-on is added but stopped
    ServiceStopped,
    /// The service's pipe is not the service's (ipc::service_state::
    /// check_service_pipe says why): nothing it sends is used
    ServiceUntrusted {
        reason: String,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
struct Subscription {
    logs: Option<LogLevel>,
    link: bool,
    #[serde(default)]
    receiver: bool,
}

impl Subscription {
    fn method(self) -> Method {
        Method::DiagSubscribe { logs: self.logs, link: self.link, receiver: Some(self.receiver) }
    }
}

type Pending = oneshot::Sender<Result<Value, ErrorInfo>>;

/// The host: the connection to the core and what goes with it (the
/// in-app update, the service add-on's set-up). One per process, from
/// [`start`].
pub struct Host {
    /// The host's runtime ([`RUNTIME`])
    pub(crate) runtime: tokio::runtime::Handle,
    events: mpsc::UnboundedSender<HostEvent>,
    pub(crate) updates: Updates,
    status: Mutex<Status>,
    out: Mutex<Option<mpsc::UnboundedSender<ClientMessage>>>,
    pending: Mutex<HashMap<u64, Pending>>,
    next_id: AtomicU64,
    /// diag.subscribe per window label; the core gets their union
    subscriptions: Mutex<HashMap<String, Subscription>>,
    /// An update stops the core (update.rs): its Bye does not end the UI,
    /// and it is not started again
    updating: AtomicBool,
    /// Connected to the core that runs as a Windows service
    service: AtomicBool,
    /// The service add-on is being added, updated or removed: the core
    /// that stops for it says Bye, and the window connects again (to the
    /// service or the user's own core) instead of ending. Cleared by the
    /// next Welcome, or when the set-up did not change anything.
    switching: AtomicBool,
}

/// A poisoned lock only means another thread panicked while holding it;
/// the maps stay usable.
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn error(code: &str, key: &str, args: &[String]) -> ErrorInfo {
    ErrorInfo { code: code.into(), message: Msg { key: key.into(), args: args.to_vec() } }
}

/// How long to wait for the answer. Most calls are quick; scans, the
/// stack's start and driver changes take seconds, and a driver change waits
/// for the user to answer the UAC prompt.
fn timeout_for(method: &str, params: &Value) -> Duration {
    match method {
        // The inquiries, the name requests after the last one (~7 s), the
        // LE Audio scan (~6 s) and the stack's start
        "device.scan" => {
            let seconds = params.get("seconds").and_then(Value::as_u64).unwrap_or(10);
            Duration::from_secs(30.max(seconds + 25))
        }
        "adapter.assign_winusb" | "adapter.restore_driver" => Duration::from_secs(120),
        // VB-Audio's pack fetched (60 s limit), the administrator prompt,
        // VB-Audio's setup, then up to 10 s for the output to come or go
        // (issue #86: with 5 s the UI showed a failure while it installed)
        "audio.install_vb_cable" | "audio.uninstall_cable" => Duration::from_secs(300),
        // Up to two files, each fetched with a 30 s limit, then the
        // adapters are listed again
        "adapter.download_firmware" => Duration::from_secs(90),
        // Up to 10 s for the stack with keyboards and mice, then up to
        // 10 s for the self-test through Windows
        "hid.enable" => Duration::from_secs(45),
        "stream.start"
        // May start or restart the stack first
        | "hid.disable"
        | "hid.add"
        | "hid.connect"
        | "stream.start_custom"
        | "adapter.restart"
        | "adapter.list"
        | "adapter.firmware_files"
        | "audio.devices"
        | "audio.input_devices"
        | "audio.apps"
        | "diag.info"
        | "diag.capture_start"
        | "diag.capture_stop"
        | "app.check_update"
        | "state.snapshot"
        | "settings.set"
        | "receiver.discover"
        | "receiver.connect"
        | "receiver.disconnect" => Duration::from_secs(30),
        _ => Duration::from_secs(5),
    }
}

/// The host's runtime, for the life of the process (a runtime cannot be
/// dropped from inside one of its own tasks, where the last `Arc<Host>`
/// may go)
static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// Starts the host: its runtime and the connection task (runs for the
/// life of the process). Call once; the receiver gets every [`HostEvent`].
///
/// # Panics
///
/// When the runtime's threads cannot be created.
pub fn start() -> (Arc<Host>, mpsc::UnboundedReceiver<HostEvent>) {
    let runtime = RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("ui-host")
            .enable_all()
            .build()
            .expect("the host's runtime")
    });
    let (events, rx) = mpsc::unbounded_channel();
    let host = Host::new(runtime.handle().clone(), events);
    let h = host.clone();
    runtime.spawn(async move { run(h).await });
    (host, rx)
}

impl Host {
    fn new(runtime: tokio::runtime::Handle, events: mpsc::UnboundedSender<HostEvent>) -> Arc<Host> {
        Arc::new(Host {
            runtime,
            updates: Updates::new(),
            events,
            status: Mutex::new(Status::Connecting),
            out: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            subscriptions: Mutex::new(HashMap::new()),
            updating: AtomicBool::new(false),
            service: AtomicBool::new(false),
            switching: AtomicBool::new(false),
        })
    }

    pub(crate) fn emit(&self, event: HostEvent) {
        let _ = self.events.send(event);
    }

    pub(crate) fn set_switching(&self, on: bool) {
        self.switching.store(on, Ordering::Release);
    }

    fn switching(&self) -> bool {
        self.switching.load(Ordering::Acquire)
    }

    pub(crate) fn set_updating(&self, on: bool) {
        self.updating.store(on, Ordering::Release);
    }

    fn updating(&self) -> bool {
        self.updating.load(Ordering::Acquire)
    }

    /// The core runs as a Windows service (its pipe is the one connected):
    /// Windows starts and stops it, so the in-app update cannot stop it
    pub fn service(&self) -> bool {
        self.service.load(Ordering::Acquire)
    }

    pub fn status(&self) -> Status {
        lock(&self.status).clone()
    }

    fn set_status(&self, status: Status) {
        let mut s = lock(&self.status);
        if *s != status {
            match &status {
                Status::Connected { server } => tracing::info!("core connected ({server})"),
                Status::Connecting => tracing::info!("connecting to the core"),
                other => tracing::warn!("core: {other:?}"),
            }
            *s = status.clone();
            drop(s);
            self.emit(HostEvent::Status(status));
        }
    }

    /// Sends a request to the core -> its result, or the core's ErrorInfo
    /// (codes "ipc.*" when the connection fails). `window`: the label of
    /// the window asking ("main", "flyout": for diag.subscribe). Runs on
    /// the host's runtime; awaiting the receiver needs no tokio context
    /// (it fails only if the runtime is gone).
    pub fn request(
        self: &Arc<Self>,
        window: &str,
        method: String,
        params: Option<Value>,
    ) -> oneshot::Receiver<Result<Value, ErrorInfo>> {
        let (tx, rx) = oneshot::channel();
        let host = self.clone();
        let window = window.to_string();
        self.runtime.spawn(async move {
            let result = host.call(&window, &method, params).await;
            // Only the connection's failures: the core's own errors are in its log
            if let Err(e) = &result
                && e.code.starts_with("ipc.")
            {
                tracing::warn!("[{window}] {method} failed: {}", e.code);
            }
            let _ = tx.send(result);
        });
        rx
    }

    /// Sends a request and waits for its answer (on the host's runtime).
    /// `window`: the label of the window asking (for diag.subscribe).
    pub(crate) async fn call(&self, window: &str, method: &str, params: Option<Value>) -> Result<Value, ErrorInfo> {
        let params = params.filter(|p| !p.is_null());
        // The protocol's own types check the method and its parameters, so
        // a typo in the window is reported here and not by the core
        let mut wire = serde_json::json!({ "method": method });
        if let Some(p) = &params {
            wire["params"] = p.clone();
        }
        let mut call: Method = serde_json::from_value(wire)
            .map_err(|e| error("ipc.bad_request", "ipc.error.bad_request", &[method.into(), e.to_string()]))?;
        if let Method::DiagSubscribe { logs, link, receiver } = call {
            let merged = {
                let mut subs = lock(&self.subscriptions);
                subs.insert(window.into(), Subscription { logs, link, receiver: receiver.unwrap_or(false) });
                merge(subs.values())
            };
            call = merged.method();
        }
        let timeout = timeout_for(method, params.as_ref().unwrap_or(&Value::Null));
        // Service mode: a request that needs an administrator runs again
        // through the core's elevated helper (the UAC prompt)
        let retry = (!matches!(call, Method::AppRunAsAdmin { .. })).then(|| call.clone());
        match self.send(call, timeout, method).await {
            Err(e) if e.code == ipc::access::NEEDS_ADMIN && retry.is_some() => {
                let call = Method::AppRunAsAdmin { call: Box::new(retry.unwrap_or(Method::StateSnapshot)) };
                self.send(call, ADMIN_TIMEOUT, "app.run_as_admin").await
            }
            other => other,
        }
    }

    async fn send(&self, call: Method, timeout: Duration, method: &str) -> Result<Value, ErrorInfo> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        lock(&self.pending).insert(id, tx);
        let sent =
            lock(&self.out).as_ref().map(|out| out.send(ClientMessage::Request { id, call }).is_ok()).unwrap_or(false);
        if !sent {
            lock(&self.pending).remove(&id);
            return Err(error("ipc.not_connected", "ipc.error.not_connected", &[]));
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(result)) => result,
            // The sender was dropped: the connection broke
            Ok(Err(_)) => Err(error("ipc.disconnected", "ipc.error.disconnected", &[])),
            Err(_) => {
                lock(&self.pending).remove(&id);
                Err(error("ipc.timeout", "ipc.error.timeout", &[method.into()]))
            }
        }
    }

    /// Forgets a window's subscription (window closed for good)
    pub fn drop_window(&self, window: &str) {
        let merged = {
            let mut subs = lock(&self.subscriptions);
            if subs.remove(window).is_none() {
                return;
            }
            merge(subs.values())
        };
        self.send_subscription(merged);
    }

    fn send_subscription(&self, sub: Subscription) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        if let Some(out) = lock(&self.out).as_ref() {
            let _ = out.send(ClientMessage::Request { id, call: sub.method() });
        }
    }

    fn on_response(&self, msg: &Value) {
        #[derive(Deserialize)]
        struct Response {
            id: u64,
            result: Option<Value>,
            error: Option<ErrorInfo>,
        }
        let Ok(r) = serde_json::from_value::<Response>(msg.clone()) else { return };
        let Some(p) = lock(&self.pending).remove(&r.id) else { return };
        let result = match r.error {
            Some(e) => Err(e),
            None => Ok(r.result.unwrap_or(Value::Null)),
        };
        let _ = p.send(result);
    }

    fn fail_pending(&self) {
        // Dropping the senders wakes every waiter with "disconnected"
        lock(&self.pending).clear();
    }
}

fn merge<'a>(subs: impl Iterator<Item = &'a Subscription>) -> Subscription {
    subs.fold(Subscription::default(), |acc, s| Subscription {
        // LogLevel orders from Error (fewest lines) to Trace (most): the
        // most detailed level any window wants
        logs: acc.logs.max(s.logs),
        link: acc.link || s.link,
        receiver: acc.receiver || s.receiver,
    })
}

enum Ended {
    /// The pipe broke or the core closed it
    Broken,
    /// The core said Bye: it is shutting down on purpose (tray Exit,
    /// app.quit), so the UI ends too instead of starting it again
    Bye,
    Rejected,
}

async fn run(host: Arc<Host>) {
    let name = match win_sys::pipe::current_user_sid() {
        Ok(sid) => ipc::pipe_name(&sid),
        Err(e) => {
            host.set_status(Status::CoreMissing { detail: format!("user SID: {e}") });
            return;
        }
    };
    loop {
        host.set_status(Status::Connecting);
        let pipe = connect(&host, &name).await;
        let ended = session(&host, pipe).await;
        *lock(&host.out) = None;
        host.fail_pending();
        match ended {
            // The update's installer runs next and starts the UI again
            Ended::Bye if host.updating() => tokio::time::sleep(Duration::from_millis(300)).await,
            // The service add-on came or went: the other core serves next
            Ended::Bye if host.switching() => tokio::time::sleep(Duration::from_millis(300)).await,
            Ended::Bye => {
                host.emit(HostEvent::Exit);
                return;
            }
            // Another major version: retrying soon will not help, but the
            // user may update the core while the window stays open
            Ended::Rejected => tokio::time::sleep(Duration::from_secs(10)).await,
            Ended::Broken => tokio::time::sleep(Duration::from_millis(300)).await,
        }
    }
}

/// Opens the pipe, starting the core when it does not exist. Only the new
/// name: a 2.0.x core that still runs (only the old name) is not connected
/// to; the core started here asks it to exit and takes over (resident.rs).
async fn connect(host: &Host, name: &str) -> NamedPipeClient {
    let mut spawned: Option<Instant> = None;
    // The last start failed (its reason is the status already)
    let mut spawn_failed = false;
    loop {
        // The service add-on decides which core serves the window. Read
        // again each round: an update, a start or a removal shows at once.
        match crate::service::current() {
            Route::NotAdded => {}
            Route::Running { .. } => {
                match open_service() {
                    Service::Pipe(pipe) => {
                        host.service.store(true, Ordering::Release);
                        return pipe;
                    }
                    // Starting: its pipe comes shortly. Never a core of
                    // our own beside the service.
                    Service::None | Service::Busy => tokio::time::sleep(Duration::from_millis(200)).await,
                    Service::Untrusted(reason) => {
                        host.set_status(Status::ServiceUntrusted { reason });
                        tokio::time::sleep(Duration::from_secs(10)).await;
                    }
                }
                continue;
            }
            Route::Stopped { .. } => {
                host.set_status(Status::ServiceStopped);
                tokio::time::sleep(SERVICE_RECHECK).await;
                continue;
            }
            Route::NeedsUpdate { version } => {
                host.set_status(Status::ServiceNeedsUpdate { version, app: APP_VERSION.into() });
                tokio::time::sleep(SERVICE_RECHECK).await;
                continue;
            }
            Route::AppOlder { version } => {
                host.set_status(Status::ServiceNewer { version, app: APP_VERSION.into() });
                tokio::time::sleep(SERVICE_RECHECK).await;
                continue;
            }
        }
        match open_service() {
            Service::Pipe(pipe) => {
                host.service.store(true, Ordering::Release);
                return pipe;
            }
            Service::None => {}
            Service::Busy => {
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
            Service::Untrusted(reason) => {
                // Someone else serves the service's name: not trusted, and
                // no core of our own started beside the service
                host.set_status(Status::ServiceUntrusted { reason });
                tokio::time::sleep(Duration::from_secs(10)).await;
                continue;
            }
        }
        match ClientOptions::new().open(name) {
            Ok(pipe) => {
                host.service.store(false, Ordering::Release);
                return pipe;
            }
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && host.updating() => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let due = spawned.is_none_or(|t| t.elapsed() >= CORE_RESPAWN_INTERVAL);
                if due {
                    spawned = Some(Instant::now());
                    spawn_failed = match spawn_core() {
                        Ok(()) => false,
                        Err(detail) => {
                            // Keep looking: the user may start it by hand
                            host.set_status(Status::CoreMissing { detail });
                            true
                        }
                    };
                } else if !spawn_failed && spawned.is_some_and(|t| t.elapsed() >= CORE_START_WAIT) {
                    host.set_status(Status::CoreMissing { detail: "bluecaster-core.exe did not open its pipe".into() });
                }
            }
            Err(_) => {}
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// The service's pipe (a core running as a service), once its server is
/// the service folder's core (a test instance: the core beside this
/// program) and, for the real service, runs in
/// session 0 and is the process the service control manager runs as the
/// installed service (ipc::access::service_server_trusted).
///
/// A user cannot read a SYSTEM process's token. What the user can read
/// instead: the server's session (0, where no program the user starts
/// runs), its program, and the manager's record of the service (its
/// configured program and its process id), through read-only queries.
fn open_service() -> Service {
    let pipe = match win_sys::pipe_peer::open_client(&ipc::service_pipe_name()) {
        Ok(p) => p,
        Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => return Service::Busy,
        Err(_) => return Service::None,
    };
    // The real service runs the service folder's copy of the core; only a
    // test instance's runs beside this program
    let beside = match std::env::current_exe() {
        Ok(exe) => exe.with_file_name(ipc::service_state::CORE_EXE),
        Err(e) => return Service::Untrusted(e.to_string()),
    };
    match check_server(&pipe, &beside) {
        Ok(()) => Service::Pipe(pipe),
        Err(why) => Service::Untrusted(why),
    }
}

enum Service {
    /// No core runs as a service
    None,
    Busy,
    Pipe(NamedPipeClient),
    /// Another program serves the name: why
    Untrusted(String),
}

/// Whether the process serving `pipe` is the service's core (a test
/// instance: `beside`), checked the one way every client checks it
fn check_server(pipe: &NamedPipeClient, beside: &std::path::Path) -> Result<(), String> {
    use std::os::windows::io::AsHandle;
    ipc::service_state::check_service_pipe(pipe.as_handle(), beside)
}

/// The status for a core's reject: the service's version rule (it checks
/// the window's version too) shows as the add-on's own states
fn rejected(reason: String, service: bool) -> Status {
    use ipc::service_state::{APP_OLDER_THAN_SERVICE, SERVICE_NEEDS_UPDATE};
    let app = APP_VERSION.to_string();
    match reason.as_str() {
        SERVICE_NEEDS_UPDATE if service => Status::ServiceNeedsUpdate { version: String::new(), app },
        APP_OLDER_THAN_SERVICE if service => Status::ServiceNewer { version: String::new(), app },
        _ => Status::Rejected { reason },
    }
}

/// Starts `bluecaster-core.exe --background` from bluecaster.exe's folder (the
/// installer puts it there)
fn spawn_core() -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    // The core outlives the UI: no console, not in our process group (a
    // Ctrl+C in a developer's terminal must not reach it)
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let core = exe.with_file_name("bluecaster-core.exe");
    if !core.is_file() {
        return Err(format!("{} not found", core.display()));
    }
    std::process::Command::new(&core)
        .arg("--background")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("{}: {e}", core.display()))
}

async fn session(host: &Host, pipe: NamedPipeClient) -> Ended {
    let (mut rd, mut wr) = tokio::io::split(pipe);
    let hello = ClientMessage::Hello {
        protocol: PROTOCOL_VERSION,
        client: format!("bluecaster-ui/{}", env!("CARGO_PKG_VERSION")),
    };
    if ipc::write_frame(&mut wr, &hello).await.is_err() {
        return Ended::Broken;
    }
    let (tx, mut rx) = mpsc::unbounded_channel::<ClientMessage>();
    *lock(&host.out) = Some(tx);

    let writer = async move {
        while let Some(m) = rx.recv().await {
            if ipc::write_frame(&mut wr, &m).await.is_err() {
                break;
            }
        }
    };
    let reader = async {
        loop {
            // Read as plain JSON: a newer minor version of the core may send
            // event kinds this build does not know; the window ignores
            // them instead of the connection failing here
            let msg: Value = match ipc::read_frame(&mut rd).await {
                Ok(Some(m)) => m,
                Ok(None) | Err(_) => return Ended::Broken,
            };
            match msg.get("type").and_then(Value::as_str) {
                Some("welcome") => {
                    // A set-up that switched cores is over
                    host.set_switching(false);
                    let server = msg.get("server").and_then(Value::as_str).unwrap_or("").to_string();
                    host.set_status(Status::Connected { server });
                    // A new connection has no subscription yet
                    let merged = merge(lock(&host.subscriptions).values());
                    if merged != Subscription::default() {
                        host.send_subscription(merged);
                    }
                }
                Some("reject") => {
                    let reason = msg.get("reason").and_then(Value::as_str).unwrap_or("").to_string();
                    host.set_status(rejected(reason, host.service()));
                    return Ended::Rejected;
                }
                Some("response") => {
                    host.on_response(&msg);
                    continue;
                }
                _ => {}
            }
            // ui_close: the user chose Exit while the core runs as a
            // service; the core stays, this process ends as on a Bye
            let kind = msg.pointer("/event/kind").and_then(Value::as_str);
            let bye = matches!(kind, Some("bye" | "ui_close"));
            // Service mode: this process cannot read the core's files, so its
            // verbose logging follows the core's state instead
            if host.service() {
                crate::logging::follow_core(&msg);
            }
            host.emit(HostEvent::Message(msg));
            if bye {
                return Ended::Bye;
            }
        }
    };
    tokio::select! {
        () = writer => Ended::Broken,
        ended = reader => ended,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscriptions_merge_to_the_most_detailed() {
        let subs = [
            Subscription { logs: Some(LogLevel::Info), link: false, receiver: false },
            Subscription { logs: None, link: true, receiver: false },
            Subscription { logs: Some(LogLevel::Debug), link: false, receiver: true },
        ];
        assert_eq!(merge(subs.iter()), Subscription { logs: Some(LogLevel::Debug), link: true, receiver: true });
        assert_eq!(merge([].iter()), Subscription::default());
    }

    /// The service's pipe is trusted only when the service folder's core
    /// serves it as the service: here the test program serves a pipe of its
    /// own, beside itself, in this session, and is not trusted (the cases of
    /// the check itself are ipc::service_state's tests)
    #[test]
    fn the_service_pipe_is_checked_before_it_is_trusted() {
        if !ipc::instance_suffix().is_empty() {
            return;
        }
        let name = format!(r"\\.\pipe\bluecaster-test-ui-host-check-{}", std::process::id());
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let _server = win_sys::pipe_peer::create_service_server(&name, true).unwrap();
            let pipe = win_sys::pipe_peer::open_client(&name).unwrap();
            let me = std::env::current_exe().unwrap();
            assert!(check_server(&pipe, &me).is_err());
        });
    }

    #[test]
    fn the_services_version_rule_shows_as_its_states() {
        use ipc::service_state::{APP_OLDER_THAN_SERVICE, SERVICE_NEEDS_UPDATE};
        assert!(matches!(rejected(SERVICE_NEEDS_UPDATE.into(), true), Status::ServiceNeedsUpdate { .. }));
        assert!(matches!(rejected(APP_OLDER_THAN_SERVICE.into(), true), Status::ServiceNewer { .. }));
        // Only from the service; any other reject stays as it was
        assert_eq!(
            rejected(SERVICE_NEEDS_UPDATE.into(), false),
            Status::Rejected { reason: SERVICE_NEEDS_UPDATE.into() }
        );
        assert_eq!(rejected("protocol 2".into(), true), Status::Rejected { reason: "protocol 2".into() });
    }

    #[test]
    fn long_calls_get_long_timeouts() {
        assert_eq!(timeout_for("stream.stop", &Value::Null), Duration::from_secs(5));
        assert_eq!(timeout_for("device.scan", &serde_json::json!({ "seconds": 3 })), Duration::from_secs(30));
        assert_eq!(timeout_for("device.scan", &serde_json::json!({ "seconds": 30 })), Duration::from_secs(55));
        assert!(timeout_for("adapter.assign_winusb", &Value::Null) >= Duration::from_secs(60));
        // A 60 s download, the administrator prompt, the setup and a 10 s wait
        assert!(timeout_for("audio.install_vb_cable", &Value::Null) >= Duration::from_secs(120));
        assert!(timeout_for("audio.uninstall_cable", &Value::Null) >= Duration::from_secs(60));
        // Two 30 s fetches and the adapter list
        assert!(timeout_for("adapter.download_firmware", &Value::Null) > Duration::from_secs(60));
        // Discovery waits 1.5 s; connecting ends an older connection first
        assert!(timeout_for("receiver.discover", &Value::Null) >= Duration::from_secs(10));
        assert!(timeout_for("receiver.connect", &Value::Null) >= Duration::from_secs(10));
        // The core waits up to 10 s for the stack and 10 s for the self-test
        assert!(timeout_for("hid.enable", &Value::Null) > Duration::from_secs(20));
        assert!(timeout_for("hid.add", &Value::Null) >= Duration::from_secs(30));
    }

    /// Without a connection, a request answers "not connected" on its
    /// oneshot, awaited outside any tokio context (as the window does)
    #[test]
    fn requests_answer_on_their_oneshot_without_a_tokio_context() {
        use super::tests_support::{block_on, host};
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
        let (host, mut rx) = host(rt.handle().clone());
        let answer = host.request("main", "state.snapshot".into(), None);
        let result = block_on(answer).expect("the host answers");
        assert_eq!(result.unwrap_err().code, "ipc.not_connected");
        let bad = block_on(host.request("main", "no.such_method".into(), None)).unwrap();
        assert_eq!(bad.unwrap_err().code, "ipc.bad_request");
        host.set_status(Status::ServiceStopped);
        assert_eq!(rx.try_recv().unwrap(), HostEvent::Status(Status::ServiceStopped));
        // No event for the same status again
        host.set_status(Status::ServiceStopped);
        assert!(rx.try_recv().is_err());
    }
}

/// A host without its connection task, and a way to await its answers
/// outside tokio, for the tests of this crate
#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;

    pub(crate) fn host(runtime: tokio::runtime::Handle) -> (Arc<Host>, mpsc::UnboundedReceiver<HostEvent>) {
        let (events, rx) = mpsc::unbounded_channel();
        (Host::new(runtime, events), rx)
    }

    /// A minimal executor for a future, outside tokio: a thread parks until
    /// woken (what any non-tokio executor does)
    pub(crate) fn block_on<F: std::future::Future>(f: F) -> F::Output {
        use std::task::{Context, Poll, Wake, Waker};
        struct Unpark(std::thread::Thread);
        impl Wake for Unpark {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
        }
        let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
        let mut cx = Context::from_waker(&waker);
        let mut f = std::pin::pin!(f);
        loop {
            match f.as_mut().poll(&mut cx) {
                Poll::Ready(v) => return v,
                Poll::Pending => std::thread::park(),
            }
        }
    }
}
