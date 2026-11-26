//! The core as the windows see it: one entity for the process (both windows
//! share it), holding the connection's status, the state store
//! (ui_model::store) and the streams views subscribe to while they live.
//!
//! Views observe the entity for state (`cx.observe(&core, ..)`), and
//! subscribe to [`CoreEvent`] for what changes several times a second.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::{App, AppContext as _, AsyncApp, Context, Entity, EventEmitter, Task, WeakEntity};
use ipc::{ErrorInfo, FlowView, LogLevel, LogLine, Method, Msg, ReceiverState, ServerMessage, Snapshot};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::oneshot;
use ui_model::store::{Change, Store};
use ui_model::subscription::{SubKey, SubState, Wants};

/// The connection to the core (ui_host::connection::Status, as the screens
/// read it)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnStatus {
    Connecting,
    Connected {
        server: String,
    },
    CoreMissing {
        detail: String,
    },
    Rejected {
        reason: String,
    },
    /// The service add-on is another version than this window: nothing
    /// connects until it is updated
    ServiceNeedsUpdate {
        version: String,
        app: String,
    },
    /// The service add-on is newer than this window: the app needs the update
    ServiceNewer {
        version: String,
        app: String,
    },
    /// The service add-on is added but stopped
    ServiceStopped,
    /// The service's pipe is served by something else than the service
    ServiceUntrusted {
        reason: String,
    },
}

/// What the store needs from the host: the real connection (host.rs) or a
/// fake core (demo.rs, tests)
pub trait Transport: Send + Sync + 'static {
    fn status(&self) -> ConnStatus;
    /// One request; the answer needs no tokio context to await
    fn call(&self, method: String, params: Option<Value>) -> oneshot::Receiver<Result<Value, ErrorInfo>>;
}

/// What changed besides the state (the state: `cx.observe`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreEvent {
    Link,
    Log,
    Receiver,
    Flow,
}

const FLOW_REFRESH: Duration = Duration::from_secs(1);

pub struct Core {
    pub conn: ConnStatus,
    pub store: Store,
    /// The core's connection flow, while someone wants it
    pub flow: Option<FlowView>,
    transport: Arc<dyn Transport>,

    // Subscriptions (diag.subscribe): views ask for what they show while
    // they live; one subscription for the process
    next_want: u64,
    link_wants: BTreeMap<u64, ()>,
    receiver_wants: BTreeMap<u64, ()>,
    log_wants: BTreeMap<u64, LogLevel>,
    flow_wants: BTreeMap<u64, ()>,
    /// What was sent last
    sub: SubState,
    push_queued: bool,
    flow_timer: Option<Task<()>>,
}

impl EventEmitter<CoreEvent> for Core {}

fn ipc_error(code: &str) -> ErrorInfo {
    ErrorInfo { code: code.into(), message: Msg::new("") }
}

/// A request's answer as `R`; `()` for the methods that answer nothing
async fn answer<R: DeserializeOwned>(rx: oneshot::Receiver<Result<Value, ErrorInfo>>) -> Result<R, ErrorInfo> {
    let value = rx.await.map_err(|_| ipc_error("ipc.disconnected"))??;
    serde_json::from_value(value).map_err(|e| {
        tracing::warn!("an answer of the core does not read: {e}");
        ipc_error("ipc.bad_answer")
    })
}

impl Core {
    pub fn new(transport: Arc<dyn Transport>) -> Core {
        Core {
            conn: ConnStatus::Connecting,
            store: Store::new(),
            flow: None,
            transport,
            next_want: 0,
            link_wants: BTreeMap::new(),
            receiver_wants: BTreeMap::new(),
            log_wants: BTreeMap::new(),
            flow_wants: BTreeMap::new(),
            sub: SubState::default(),
            push_queued: false,
            flow_timer: None,
        }
    }

    /// The state, once the core answered
    pub fn snap(&self) -> Option<&Snapshot> {
        self.store.snap.as_ref()
    }

    pub fn connected(&self) -> bool {
        matches!(self.conn, ConnStatus::Connected { .. })
    }

    /// Connected and the first snapshot is here: the screens can draw
    pub fn ready(&self) -> bool {
        self.connected() && self.store.snap.is_some()
    }

    /// Sends a request -> its result. The task may be dropped: the request
    /// is sent all the same, only the answer goes nowhere.
    pub fn request<R: DeserializeOwned + Send + 'static>(
        &self,
        method: Method,
        cx: &App,
    ) -> Task<Result<R, ErrorInfo>> {
        let rx = self.send(method);
        cx.background_spawn(async move { answer(rx?).await })
    }

    fn send(&self, method: Method) -> Result<oneshot::Receiver<Result<Value, ErrorInfo>>, ErrorInfo> {
        // {"method": "...", "params": {...}} as the protocol writes a Method
        let mut value = serde_json::to_value(&method).map_err(|_| ipc_error("ipc.bad_request"))?;
        let name = value.get("method").and_then(Value::as_str).map(str::to_owned);
        let params = value.get_mut("params").map(Value::take);
        let name = name.ok_or_else(|| ipc_error("ipc.bad_request"))?;
        Ok(self.transport.call(name, params))
    }

    // ---- From the connection ------------------------------------------------

    pub fn on_message(&mut self, m: ServerMessage, cx: &mut Context<Self>) {
        let applied = self.store.on_message(m);
        match applied.change {
            Change::Nothing => {}
            Change::State => cx.notify(),
            Change::Link => cx.emit(CoreEvent::Link),
            Change::Log => cx.emit(CoreEvent::Log),
            Change::Receiver => cx.emit(CoreEvent::Receiver),
        }
        if applied.resync {
            self.resync(cx);
        }
    }

    pub fn on_status(&mut self, status: ConnStatus, cx: &mut Context<Self>) {
        let was = self.connected();
        self.conn = status;
        cx.notify();
        if self.connected() && !was {
            // A window opened after Hello missed the snapshot; on a fresh
            // connection the core sends one too (harmless)
            self.sub.reset();
            self.resync(cx);
            self.push_subscription(cx);
        }
    }

    fn resync(&mut self, cx: &mut Context<Self>) {
        let Some(token) = self.store.begin_resync() else { return };
        let task = self.request::<Snapshot>(Method::StateSnapshot, cx);
        cx.spawn(async move |this: WeakEntity<Core>, cx: &mut AsyncApp| {
            let snap = task.await.ok();
            let _ = this.update(cx, |core, cx| {
                if core.store.finish_resync(token, snap) {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    // ---- Subscriptions --------------------------------------------------------

    fn push_subscription(&mut self, cx: &mut Context<Self>) {
        if self.push_queued {
            return;
        }
        self.push_queued = true;
        // Creating one view and dropping another in the same update sends once
        cx.defer(move |cx| {
            let Some(core) = crate::app::try_core(cx) else { return };
            let hidden = crate::windows::none_shown(cx);
            core.update(cx, |core, cx| core.send_subscription(hidden, cx));
        });
    }

    /// No window of ours shows any more, or one shows again: the
    /// subscription asks for nothing while none shows
    pub fn shown_changed(&mut self, cx: &mut Context<Self>) {
        self.push_subscription(cx);
    }

    fn send_subscription(&mut self, hidden: bool, cx: &mut Context<Self>) {
        self.push_queued = false;
        let wants = Wants {
            link: !self.link_wants.is_empty(),
            receiver: !self.receiver_wants.is_empty(),
            // The most verbose level asked for
            logs: self.log_wants.values().copied().max(),
        };
        let Some(plan) = self.sub.plan(wants, hidden, self.connected()) else { return };
        self.sub.sent(&plan);
        if plan.clear_link {
            self.store.clear_link();
            cx.emit(CoreEvent::Link);
        }
        let SubKey { logs, link, receiver } = plan.key;
        let subscribe = self.request::<Value>(Method::DiagSubscribe { logs, link, receiver: Some(receiver) }, cx);
        // Lines logged while nobody listened (the window was closed, or
        // opened later): the core keeps the recent ones
        let recent =
            plan.recent_logs.map(|level| self.request::<Option<Vec<LogLine>>>(Method::DiagRecentLogs { level }, cx));
        cx.spawn(async move |this: WeakEntity<Core>, cx: &mut AsyncApp| {
            if subscribe.await.is_err() {
                let _ = this.update(cx, |core, _| core.sub.reset());
                return;
            }
            if let Some(recent) = recent
                && let Ok(lines) = recent.await
            {
                let _ = this.update(cx, |core, cx| {
                    core.store.merge_recent(lines.unwrap_or_default());
                    cx.emit(CoreEvent::Log);
                });
            }
        })
        .detach();
    }

    fn refresh_flow(&mut self, cx: &mut Context<Self>) {
        if !self.connected() || crate::windows::none_shown(cx) {
            return;
        }
        let task = self.request::<FlowView>(Method::DiagFlow, cx);
        cx.spawn(async move |this: WeakEntity<Core>, cx: &mut AsyncApp| {
            if let Ok(flow) = task.await {
                let _ = this.update(cx, |core, cx| {
                    core.flow = Some(flow);
                    cx.emit(CoreEvent::Flow);
                });
            }
        })
        .detach();
    }

    fn start_flow(&mut self, cx: &mut Context<Self>) {
        self.refresh_flow(cx);
        self.flow_timer = Some(cx.spawn(async move |this: WeakEntity<Core>, cx: &mut AsyncApp| {
            loop {
                cx.background_executor().timer(FLOW_REFRESH).await;
                if this.update(cx, |core, cx| core.refresh_flow(cx)).is_err() {
                    break;
                }
            }
        }));
    }

    pub fn clear_logs(&mut self, cx: &mut Context<Self>) {
        self.store.clear_logs();
        cx.emit(CoreEvent::Log);
    }

    fn add_want(&mut self, kind: WantKind, cx: &mut Context<Self>) -> u64 {
        self.next_want += 1;
        let id = self.next_want;
        match kind {
            WantKind::Link => {
                self.link_wants.insert(id, ());
            }
            WantKind::Logs(level) => {
                self.log_wants.insert(id, level);
            }
            WantKind::Receiver => {
                self.receiver_wants.insert(id, ());
                // Also asks for it at once
                if self.connected() {
                    let task = self.request::<ReceiverState>(Method::ReceiverStatus, cx);
                    cx.spawn(async move |this: WeakEntity<Core>, cx: &mut AsyncApp| {
                        if let Ok(state) = task.await {
                            let _ = this.update(cx, |core, cx| {
                                core.store.receiver = Some(state);
                                cx.emit(CoreEvent::Receiver);
                            });
                        }
                    })
                    .detach();
                }
            }
            WantKind::Flow => {
                self.flow_wants.insert(id, ());
                if self.flow_wants.len() == 1 {
                    self.start_flow(cx);
                }
                return id;
            }
        }
        self.push_subscription(cx);
        id
    }

    fn remove_want(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.flow_wants.remove(&id).is_some() {
            if self.flow_wants.is_empty() {
                self.flow_timer = None;
            }
            return;
        }
        self.link_wants.remove(&id);
        self.log_wants.remove(&id);
        self.receiver_wants.remove(&id);
        self.push_subscription(cx);
    }
}

/// What a view asks the core to send while it lives
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WantKind {
    /// Link samples ([`CoreEvent::Link`], `core.store.link()`)
    Link,
    /// Log lines at this level and above ([`CoreEvent::Log`], `core.store.logs()`)
    Logs(LogLevel),
    /// The receiver test's state ([`CoreEvent::Receiver`], `core.store.receiver`)
    Receiver,
    /// The connection flow, refreshed every second ([`CoreEvent::Flow`], `core.flow`)
    Flow,
}

/// Asks for `kind` until the view `cx` belongs to is released. Call it
/// when the view is created.
pub fn want<V: 'static>(kind: WantKind, cx: &mut Context<V>) {
    let core = crate::app::core(cx);
    let id = core.update(cx, |core, cx| core.add_want(kind, cx));
    cx.on_release(move |_, cx| {
        core.update(cx, |core, cx| core.remove_want(id, cx));
    })
    .detach();
}

/// Asks for `kind` until the returned guard is released with [`Want::release`]
/// (for a part of a view that comes and goes, such as a tab)
pub struct Want(Entity<Core>, u64);

impl Want {
    pub fn new(kind: WantKind, cx: &mut App) -> Want {
        let core = crate::app::core(cx);
        let id = core.update(cx, |core, cx| core.add_want(kind, cx));
        Want(core, id)
    }

    pub fn release(self, cx: &mut App) {
        self.0.update(cx, |core, cx| core.remove_want(self.1, cx));
    }
}
