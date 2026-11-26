//! Headless interaction tests: the real views in a window of GPUI's test
//! platform (no screen, no GPU), driven by pointer, keyboard and wheel input,
//! with a fake core behind them.
//!
//! ```ignore
//! let mut h = Harness::start(cx, snapshot_with_device());
//! let (window, view) = h.open(cx, |window, cx| cx.new(|cx| MediaEqEditor::new(key, window, cx)));
//! h.on(cx, window, |w, cx| w.click("eq-add", cx));
//! h.settle(cx);
//! assert_eq!(h.calls("profile.save").len(), 1);
//! ```
//!
//! [`Fake`] serves the snapshot, records every request, answers from a
//! table and acts as the core does for `profile.save` and `settings.set`
//! (the snapshot changes, and [`Harness::settle`] sends it to the views).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Entity, InputEvent, KeyDownEvent, KeyUpEvent, Keystroke,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Render, ScrollDelta, ScrollEvent,
    TestAppContext, Window, WindowBounds, WindowOptions, point, px, size,
};
use ipc::{ErrorInfo, Msg, ServerMessage, Snapshot};
use serde_json::{Value, json};
use tokio::sync::oneshot;

use crate::core::{ConnStatus, Core, Transport};

/// One request a view sent
#[derive(Debug, Clone)]
pub struct Call {
    pub method: String,
    pub params: Value,
}

/// How the fake core answers a method
#[derive(Debug, Clone)]
pub enum Answer {
    Ok(Value),
    /// Fails with this error code
    Err(String),
    /// Never answers by itself: [`Harness::release`] answers it later
    Hold,
}

#[derive(Default)]
struct Inner {
    snapshot: Value,
    answers: HashMap<String, Answer>,
    calls: Vec<Call>,
    held: Vec<(String, oneshot::Sender<Result<Value, ErrorInfo>>)>,
    /// The snapshot changed since the views last got it
    changed: bool,
}

/// A core that serves a snapshot the test builds, and records every request
#[derive(Default)]
pub struct Fake {
    inner: Mutex<Inner>,
}

fn err(code: &str) -> ErrorInfo {
    ErrorInfo { code: code.into(), message: Msg::new("") }
}

impl Fake {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// `profile.save` as the core does it: the profile replaces the one of its
/// old name (or is added)
fn save_profile(snapshot: &mut Value, params: &Value) {
    let Some(profile) = params.get("profile").cloned() else { return };
    let old = params.get("original_name").and_then(Value::as_str).map(str::to_owned);
    let name = profile.get("name").and_then(Value::as_str).unwrap_or_default().to_owned();
    let Some(list) = snapshot.get_mut("profiles").and_then(Value::as_array_mut) else { return };
    let at = list.iter().position(|p| {
        let n = p.get("name").and_then(Value::as_str);
        n == old.as_deref() || n == Some(name.as_str())
    });
    match at {
        Some(i) => list[i] = profile,
        None => list.push(profile),
    }
}

impl Transport for Fake {
    fn status(&self) -> ConnStatus {
        ConnStatus::Connected { server: "test".into() }
    }

    fn call(&self, method: String, params: Option<Value>) -> oneshot::Receiver<Result<Value, ErrorInfo>> {
        let (tx, rx) = oneshot::channel();
        let params = params.unwrap_or(Value::Null);
        let mut inner = self.lock();
        inner.calls.push(Call { method: method.clone(), params: params.clone() });
        let answer = match inner.answers.get(&method).cloned() {
            Some(Answer::Ok(v)) => Ok(v),
            Some(Answer::Err(code)) => Err(err(&code)),
            Some(Answer::Hold) => {
                inner.held.push((method, tx));
                return rx;
            }
            None => match method.as_str() {
                "state.snapshot" => Ok(inner.snapshot.clone()),
                "diag.subscribe" => Ok(Value::Null),
                "diag.recent_logs" => Ok(json!([])),
                "profile.save" => {
                    save_profile(&mut inner.snapshot, &params);
                    inner.changed = true;
                    Ok(Value::Null)
                }
                "settings.set" => {
                    if let Some(s) = params.get("settings") {
                        inner.snapshot["settings"] = s.clone();
                        inner.changed = true;
                    }
                    Ok(Value::Null)
                }
                _ => Err(err("test.unanswered")),
            },
        };
        let _ = tx.send(answer);
        rx
    }
}

/// The smallest valid state: no adapter, no devices, English
pub fn snapshot() -> Value {
    json!({
        "adapter": {"status": "ready", "adapters": []},
        "devices": [],
        "scanning": false,
        "stream": {"state": "idle", "message": {"key": "status.idle", "args": []}},
        "profiles": [],
        "active_profile": "",
        "settings": {
            "language": "en",
            "theme": "light",
            "start_with_windows": false,
            "start_minimized": false,
            "minimize_to_tray": true,
            "check_updates_on_startup": false,
            "include_prereleases": false,
            "media_keys": false,
            "developer_mode": false,
            "bt_chip_pid": 0,
            "bt_chip_fw_stem": "",
            "afh": "auto",
            "remote_sink": "auto"
        },
        "update": {"checking": false, "current_version": "3.0.4"},
        "config_dir": ""
    })
}

/// A saved audio device (a made-up name and address)
pub fn device(address: &str, name: &str) -> Value {
    json!({"address": address, "name": name, "saved": true, "audio": true})
}

/// A profile of a device with what a profile must have; `extra` overrides fields
pub fn profile(name: &str, address: &str, device_name: &str, extra: Value) -> Value {
    let mut p = json!({
        "name": name,
        "device_address": address,
        "device_name": device_name,
        "codec": "auto",
        "quality": "hq",
        "abr": false,
        "codec_fallback": true,
        "sample_rate": 0,
        "bit_depth": 0,
        "capture_mode": "loopback",
        "audio_device_id": "",
        "audio_device_name": "",
        "auto_switch_device": true,
        "app_exe": "",
        "app_name": "",
        "volume_sync": "auto",
        "volume_device_id": "",
        "volume_device_name": "",
        "max_media_payload": 0
    });
    if let (Some(p), Value::Object(extra)) = (p.as_object_mut(), extra) {
        p.extend(extra);
    }
    p
}

/// An empty view, for a window that only shows a dialog
pub struct Blank;

impl Render for Blank {
    fn render(&mut self, _: &mut Window, _: &mut gpui_kit::Context<Self>) -> impl gpui_kit::IntoElement {
        use gpui_kit::Styled as _;
        gpui_kit::div().size_full()
    }
}

// ---- pointer input at window positions -------------------------------------------
//
// gpui-kit's helpers find elements registered for tests; the app's own
// drawn controls (the EQ's points, the codec cards) are found by their
// geometry instead, and get these.

fn dispatch(window: &mut Window, event: impl InputEvent, cx: &mut App) {
    window.dispatch_event(event.to_platform_input(), cx);
    window.render_frame(cx);
}

/// The pointer moves to `at` (`pressed`: with the left button held)
pub fn move_to(window: &mut Window, at: Point<Pixels>, pressed: bool, cx: &mut App) {
    let modifiers = window.modifiers();
    let pressed_button = pressed.then_some(MouseButton::Left);
    dispatch(window, MouseMoveEvent { position: at, pressed_button, modifiers }, cx);
}

/// A button goes down at `at`; `clicks` 2 for the second press of a double-click
pub fn press_at(window: &mut Window, at: Point<Pixels>, button: MouseButton, clicks: usize, cx: &mut App) {
    let modifiers = window.modifiers();
    dispatch(window, MouseDownEvent { button, position: at, modifiers, click_count: clicks, first_mouse: false }, cx);
}

pub fn release_at(window: &mut Window, at: Point<Pixels>, button: MouseButton, clicks: usize, cx: &mut App) {
    let modifiers = window.modifiers();
    dispatch(window, MouseUpEvent { button, position: at, modifiers, click_count: clicks }, cx);
}

/// A left click at `at`
pub fn click_at(window: &mut Window, at: Point<Pixels>, cx: &mut App) {
    move_to(window, at, false, cx);
    press_at(window, at, MouseButton::Left, 1, cx);
    release_at(window, at, MouseButton::Left, 1, cx);
}

pub fn double_click_at(window: &mut Window, at: Point<Pixels>, cx: &mut App) {
    move_to(window, at, false, cx);
    for clicks in 1..=2 {
        press_at(window, at, MouseButton::Left, clicks, cx);
        release_at(window, at, MouseButton::Left, clicks, cx);
    }
}

/// The wheel turned over `at` by `lines` notches (positive: away from the user, up)
pub fn wheel_at(window: &mut Window, at: Point<Pixels>, lines: f32, cx: &mut App) {
    move_to(window, at, false, cx);
    let modifiers = window.modifiers();
    dispatch(
        window,
        ScrollEvent { position: at, delta: ScrollDelta::Lines(point(0., lines)), modifiers, ..Default::default() },
        cx,
    );
}

/// The fake core, the core entity on it, and the app's globals
pub struct Harness {
    pub fake: Arc<Fake>,
    pub core: Entity<Core>,
    rev: u64,
}

impl Harness {
    /// Sets up the app as main.rs does, with the fake core serving `snapshot`
    pub fn start(cx: &mut TestAppContext, snapshot: Value) -> Harness {
        serde_json::from_value::<Snapshot>(snapshot.clone()).expect("the test's snapshot is a snapshot");
        let fake = Arc::new(Fake::default());
        fake.lock().snapshot = snapshot;
        let transport: Arc<dyn Transport> = fake.clone();
        let core = cx.update(|cx| {
            gpui_kit::init(cx);
            let core = cx.new(|_| Core::new(transport.clone()));
            crate::app::init(core.clone(), cx);
            crate::host::init(None, cx);
            crate::theme::apply(crate::theme::ThemeSetting::Light, cx);
            core.update(cx, |core, cx| core.on_status(transport.status(), cx));
            core
        });
        let mut h = Harness { fake, core, rev: 1 };
        h.settle(cx);
        h
    }

    /// Opens a window (1000×800 unless `size` says otherwise) with the view
    /// `build` makes, inside the component library's root as the app's are
    pub fn open<V: Render>(
        &mut self,
        cx: &mut TestAppContext,
        build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
    ) -> (AnyWindowHandle, Entity<V>) {
        self.open_sized(cx, (1000., 800.), build)
    }

    pub fn open_sized<V: Render>(
        &mut self,
        cx: &mut TestAppContext,
        (w, h): (f32, f32),
        build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
    ) -> (AnyWindowHandle, Entity<V>) {
        let opened = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: point(px(0.), px(0.)),
                    size: size(px(w), px(h)),
                })),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, build).expect("a test window opens")
        });
        // An active window, as the user's: focus moving away from a field
        // (blur) is only told in an active one
        cx.update_window(opened.0, |_, window, _| window.activate_window()).expect("the test window is open");
        self.settle(cx);
        opened
    }

    /// Runs `f` on the window, then lets what it started finish
    pub fn on<R>(
        &mut self,
        cx: &mut TestAppContext,
        window: AnyWindowHandle,
        f: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> R {
        let r = cx
            .update_window(window, |_, window, cx| {
                // Queries read the last frame: one with the latest state
                window.render_frame(cx);
                f(window, cx)
            })
            .expect("the test window is open");
        self.settle(cx);
        r
    }

    /// Presses a key ("backspace", "ctrl-a", "enter") as the platform delivers
    /// it: the next frame is drawn only after what the key changed reached
    /// the views. gpui-kit's `press` and `input` draw a frame right after the
    /// key, before that; a view that puts its state back into a field while
    /// drawing (the AFH channels) then undoes the key.
    pub fn key(&mut self, cx: &mut TestAppContext, window: AnyWindowHandle, key: &str) {
        // The key goes where the last frame drawn says the focus is
        cx.update_window(window, |_, w, cx| {
            let k = Keystroke::parse_with_os(key, cx.platform_info().os).unwrap_or_else(|e| panic!("not a key: {e}"));
            if matches!(k.key.as_str(), "enter" | "tab") {
                let down = KeyDownEvent { keystroke: k.clone(), is_held: false, prefer_character_input: false };
                w.dispatch_event(down.to_platform_input(), cx);
            } else {
                w.dispatch_keystroke(k.clone(), cx);
            }
            w.dispatch_event(KeyUpEvent { keystroke: k }.to_platform_input(), cx);
        })
        .expect("the test window is open");
        self.settle(cx);
    }

    /// Types `text` at the focus, one key at a time as [`Harness::key`] does
    pub fn type_text(&mut self, cx: &mut TestAppContext, window: AnyWindowHandle, text: &str) {
        for c in text.chars() {
            let text = c.to_string();
            cx.update_window(window, |_, w, cx| {
                let mut k =
                    Keystroke::parse_with_os(&text, cx.platform_info().os).unwrap_or_else(|e| panic!("not a key: {e}"));
                k.key_char = Some(text.clone());
                w.dispatch_keystroke(k, cx)
            })
            .expect("the test window is open");
            self.settle(cx);
        }
    }

    /// Runs what is due, and sends the views the snapshot each time the
    /// fake core changed it, until nothing more happens
    pub fn settle(&mut self, cx: &mut TestAppContext) {
        loop {
            cx.run_until_parked();
            let state = {
                let mut inner = self.fake.lock();
                if !std::mem::take(&mut inner.changed) {
                    break;
                }
                inner.snapshot.clone()
            };
            self.push(cx, state);
        }
    }

    /// The core sends `state` as its snapshot
    pub fn push(&mut self, cx: &mut TestAppContext, state: Value) {
        let state: Snapshot = serde_json::from_value(state).expect("a snapshot");
        self.rev += 1;
        let m = ServerMessage::Snapshot { rev: self.rev, state: Box::new(state) };
        self.core.update(cx, |core, cx| core.on_message(m, cx));
        cx.run_until_parked();
    }

    /// The core sends an event (`{"kind": ..., "value": ...}`): a log line,
    /// a link sample, or a part of the state (the next revision)
    pub fn event(&mut self, cx: &mut TestAppContext, event: Value) {
        let state = !matches!(event["kind"].as_str(), Some("log" | "link"));
        if state {
            self.rev += 1;
        }
        let m: ServerMessage =
            serde_json::from_value(json!({"type": "event", "rev": self.rev, "event": event})).expect("an event");
        self.core.update(cx, |core, cx| core.on_message(m, cx));
        self.settle(cx);
    }

    /// Changes the fake core's state; [`Harness::settle`] sends it
    pub fn edit(&mut self, cx: &mut TestAppContext, f: impl FnOnce(&mut Value)) {
        {
            let mut inner = self.fake.lock();
            f(&mut inner.snapshot);
            inner.changed = true;
        }
        self.settle(cx);
    }

    /// Sets how `method` is answered from now on
    pub fn answer(&self, method: &str, answer: Answer) {
        self.fake.lock().answers.insert(method.to_owned(), answer);
    }

    /// Answers the requests of `method` held so far (see [`Answer::Hold`])
    pub fn release(&mut self, cx: &mut TestAppContext, method: &str, result: Result<Value, &str>) {
        let held: Vec<_> = {
            let mut inner = self.fake.lock();
            let (mine, rest) = std::mem::take(&mut inner.held).into_iter().partition(|(m, _)| m == method);
            inner.held = rest;
            mine
        };
        for (_, tx) in held {
            let _ = tx.send(result.clone().map_err(err));
        }
        self.settle(cx);
    }

    /// The params of every request of `method`, oldest first
    pub fn calls(&self, method: &str) -> Vec<Value> {
        self.fake.lock().calls.iter().filter(|c| c.method == method).map(|c| c.params.clone()).collect()
    }

    /// Every request's method, oldest first
    pub fn methods(&self) -> Vec<String> {
        self.fake.lock().calls.iter().map(|c| c.method.clone()).collect()
    }

    /// The last profile saved
    pub fn last_saved(&self) -> Value {
        self.calls("profile.save").last().and_then(|p| p.get("profile").cloned()).expect("a profile was saved")
    }
}
