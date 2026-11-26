//! A core that only shows a saved state, for looking at the screens without
//! a core: `BLUECASTER_UI_DEMO=<file.json>`. The file is a snapshot (the
//! "state" of a snapshot message), or `{"state": <snapshot>, "answers":
//! {"<method>": <result>, ...}}` with the answers of other requests; a
//! request without one fails. `BLUECASTER_UI_ROUTE=<route>` opens that screen
//! ("codecs", "adapter/afh", "setup", or "flyout" for the tray flyout); the
//! window then opens without taking the focus, and
//! `BLUECASTER_UI_SIZE=<width>x<height>` sets its size. With a route set,
//! three more open one part of a screen: `BLUECASTER_UI_SETUP_STEP=<n>` the
//! setup's step n (0 is the first), `BLUECASTER_UI_DIAG_TAB=<tab>` that tab
//! of Diagnostics ("radio", "capture", "mic", "flow", "log"), and
//! `BLUECASTER_UI_DIAG_SCAN=1` starts the Wi-Fi scan of Diagnostics > Radio.
//! `BLUECASTER_UI_DEMO_CALLS=<file>` writes every request to that file.

use std::sync::Arc;

use ipc::{ErrorInfo, Msg, Snapshot};
use serde_json::Value;
use tokio::sync::oneshot;

use crate::core::{ConnStatus, Transport};

pub struct Demo {
    snapshot: Value,
    answers: serde_json::Map<String, Value>,
}

/// The screen to open and the window's size, for looking at one screen
pub fn route() -> Option<String> {
    std::env::var("BLUECASTER_UI_ROUTE").ok()
}

pub fn size() -> Option<(f32, f32)> {
    let text = std::env::var("BLUECASTER_UI_SIZE").ok()?;
    let (w, h) = text.split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

/// A variable that only counts together with `BLUECASTER_UI_ROUTE`
fn with_route(name: &str) -> Option<String> {
    route().and_then(|_| std::env::var(name).ok())
}

/// The setup's step to open on
pub fn setup_step() -> Option<usize> {
    with_route("BLUECASTER_UI_SETUP_STEP")?.parse().ok()
}

/// The Diagnostics tab to open on
pub fn diag_tab() -> Option<String> {
    with_route("BLUECASTER_UI_DIAG_TAB")
}

/// Diagnostics > Radio scans for Wi-Fi at once
pub fn diag_scan() -> bool {
    with_route("BLUECASTER_UI_DIAG_SCAN").is_some()
}

/// The demo core of the file `BLUECASTER_UI_DEMO` names, if set and readable
pub fn from_env() -> Option<Arc<dyn Transport>> {
    let path = std::env::var_os("BLUECASTER_UI_DEMO")?;
    let text = std::fs::read_to_string(&path).map_err(|e| tracing::error!("the demo state does not open: {e}")).ok()?;
    let mut file: Value = serde_json::from_str(&text).map_err(|e| tracing::error!("the demo state: {e}")).ok()?;
    let answers = match file.get_mut("answers").map(Value::take) {
        Some(Value::Object(a)) => a,
        _ => Default::default(),
    };
    let snapshot = file.get_mut("state").map(Value::take).unwrap_or(file);
    if let Err(e) = serde_json::from_value::<Snapshot>(snapshot.clone()) {
        tracing::error!("the demo state is no snapshot: {e}");
        return None;
    }
    Some(Arc::new(Demo { snapshot, answers }))
}

/// Appends each request as one JSON line to the file
/// `BLUECASTER_UI_DEMO_CALLS` names, so a test can see what a press sent
fn record_call(method: &str, params: Option<&Value>) {
    use std::io::Write;
    let Some(path) = std::env::var_os("BLUECASTER_UI_DEMO_CALLS") else { return };
    let line = serde_json::json!({ "method": method, "params": params });
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{line}");
    }
}

impl Transport for Demo {
    fn status(&self) -> ConnStatus {
        ConnStatus::Connected { server: "demo".into() }
    }

    fn call(&self, method: String, params: Option<Value>) -> oneshot::Receiver<Result<Value, ErrorInfo>> {
        record_call(&method, params.as_ref());
        let (tx, rx) = oneshot::channel();
        let answer = match method.as_str() {
            "state.snapshot" => Ok(self.snapshot.clone()),
            "diag.subscribe" => Ok(Value::Null),
            // No lines unless the file gives some
            "diag.recent_logs" => Ok(self.answers.get("diag.recent_logs").cloned().unwrap_or(Value::Array(Vec::new()))),
            other => match self.answers.get(other) {
                Some(answer) => Ok(answer.clone()),
                None => Err(ErrorInfo { code: "demo".into(), message: Msg::new("") }),
            },
        };
        let _ = tx.send(answer);
        rx
    }
}
