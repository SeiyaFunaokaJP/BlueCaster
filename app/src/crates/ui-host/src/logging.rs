//! The window process' own log: `<config dir>\logs\<YYYY-MM-DD>\ui.log`
//! next to the core's core.log (local dates and timestamps as the core's).
//! The core removes the old day folders (7 days, developer mode: 1 to 365),
//! daily and when the setting changes; this process only stops at 365.
//! It holds what the core cannot see: the windows, the connection to the
//! core, updates and the service add-on's set-up.
//!
//! Filter: `BLUECASTER_LOG` (EnvFilter syntax, default "info"), as the core.
//! Verbose logging the core keeps in `logs\verbose.json`, and developer
//! mode's verbose logging that stays on (settings.json), also turn this
//! process' own modules up, from its next start. A core that runs as a
//! Windows service keeps those files where this process cannot read them:
//! then the same two switches come from the core's state over the
//! connection ([`follow_core`]), and take effect at once.
//!
//! Panics and crashes in native code leave crash reports in `logs\crash\`
//! (win_sys::crash).

use std::sync::{Mutex, OnceLock};

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::FormatTime;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// The settings folder's layout, shared with the core (bluecaster_core::paths,
/// compiled here as well: this process does not link the core's crate)
#[path = "../../core/src/paths.rs"]
#[allow(dead_code)]
pub mod paths;

/// The core prunes by the retention setting; this is only a ceiling (the
/// setting's maximum), so a longer retention set while this process runs
/// is not cut short by it
const KEEP_DAYS: usize = 365;

/// Local wall-clock timestamps, "2026-10-21 23:11:18.502" (as the core's)
#[derive(Clone, Copy)]
struct LocalTimer {
    offset: time::UtcOffset,
}

impl FormatTime for LocalTimer {
    fn format_time(&self, w: &mut Writer<'_>) -> std::fmt::Result {
        let t = time::OffsetDateTime::now_utc().to_offset(self.offset);
        write!(
            w,
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
            t.year(),
            u8::from(t.month()),
            t.day(),
            t.hour(),
            t.minute(),
            t.second(),
            t.millisecond()
        )
    }
}

/// The settings folder of 2.0.x is moved by whichever process comes first,
/// but not under a 2.0.x core or v1 that still writes to it (the core
/// stops them and moves it then). Call before [`init`].
pub fn set_old_app_check() {
    paths::set_old_app_check(old_app_runs);
}

/// A 2.0.x core or v1 runs (not when a core of this version runs already:
/// it holds their mutexes itself, and has dealt with their folder)
fn old_app_runs() -> bool {
    use win_sys::single_instance::{V1_MUTEX_NAME, is_held};
    !is_held(&ipc::core_mutex_name()) && (is_held(&ipc::old_core_mutex_name()) || is_held(V1_MUTEX_NAME))
}

/// Installs the file logger and the panic hook; keep the guard until exit
/// (the lines still queued are written when it drops). Call first thing in
/// main: the local time offset is read here, before other threads start.
pub fn init() -> Option<WorkerGuard> {
    let offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    // Renames the folder of 2.0.x if the core has not yet
    let config = paths::config_dir();
    std::fs::create_dir_all(paths::log_dir(&config)).ok()?;
    // The day folder is made at the first line (the core may not have made
    // today's yet)
    let file = paths::DayLogFile::new(&config, paths::LogFile::Ui, offset, || KEEP_DAYS);
    let (writer, guard) = tracing_appender::non_blocking(file);
    let verbose = verbose_level(&config).max(always_verbose(&config));
    let base = std::env::var("BLUECASTER_LOG").ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| "info".into());
    let (filter, handle) = tracing_subscriber::reload::Layer::new(filter_for(&base, verbose));
    let layer = tracing_subscriber::fmt::layer().with_writer(writer).with_ansi(false).with_timer(LocalTimer { offset });
    if tracing_subscriber::registry().with(filter).with(layer).try_init().is_ok() {
        let _ = FILTER.set(Filter { handle, base, followed: Mutex::new(Followed::default()) });
    }
    if let Some(note) = paths::take_folder_note() {
        tracing::info!("{note}");
    }
    if let Some(l) = verbose {
        tracing::info!("verbose logging ({l}) as the core's");
    }
    let about = format!("bluecaster-ui {}", env!("CARGO_PKG_VERSION"));
    win_sys::crash::install(&paths::crash_dir(&config), "bluecaster-ui", &about);
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let reason = format!("panic in thread {}: {info}", std::thread::current().name().unwrap_or("unnamed"));
        tracing::error!("{reason}");
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        if let Some(p) = win_sys::crash::write(&reason, &backtrace) {
            tracing::error!("crash report written: {}", p.display());
        }
        default_hook(info);
    }));
    Some(guard)
}

/// The filter: `base`, BlueCaster's own modules at `verbose` when set (the
/// window's crates, bluecaster*, this one and the screens' rules)
fn filter_for(base: &str, verbose: Option<&str>) -> tracing_subscriber::EnvFilter {
    let directives = match verbose {
        Some(l) => format!("{base},bluecaster={l},ui_host={l},ui_model={l},win_sys={l}"),
        None => base.to_string(),
    };
    tracing_subscriber::EnvFilter::try_new(directives).unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
}

type FilterHandle = tracing_subscriber::reload::Handle<tracing_subscriber::EnvFilter, tracing_subscriber::Registry>;

/// The installed filter, to change it while the process runs
struct Filter {
    handle: FilterHandle,
    base: String,
    followed: Mutex<Followed>,
}

static FILTER: OnceLock<Filter> = OnceLock::new();

/// The core's two switches as last seen over the connection
#[derive(Debug, Default, PartialEq)]
struct Followed {
    /// diag.set_verbose (the snapshot's `verbose`)
    verbose: Option<&'static str>,
    /// Developer mode's verbose logging that stays on (the settings)
    always: Option<&'static str>,
    /// The level set last; None: not set yet (the files' level is in force)
    applied: Option<Option<&'static str>>,
}

impl Followed {
    /// Takes in a server message; Some(level): the filter must change
    fn take(&mut self, msg: &serde_json::Value) -> Option<Option<&'static str>> {
        match msg.get("type").and_then(serde_json::Value::as_str) {
            Some("snapshot") => {
                self.verbose = verbose_of(msg.pointer("/state/verbose"));
                self.always = always_of(msg.pointer("/state/settings"));
            }
            Some("event") => match msg.pointer("/event/kind").and_then(serde_json::Value::as_str) {
                Some("verbose") => self.verbose = verbose_of(msg.pointer("/event/value")),
                Some("settings") => self.always = always_of(msg.pointer("/event/value")),
                _ => return None,
            },
            _ => return None,
        }
        let level = self.verbose.max(self.always);
        if self.applied == Some(level) {
            return None;
        }
        self.applied = Some(level);
        Some(level)
    }
}

fn level_name(v: Option<&serde_json::Value>) -> Option<&'static str> {
    match v?.as_str()? {
        "debug" => Some("debug"),
        "trace" => Some("trace"),
        _ => None,
    }
}

/// The snapshot's VerboseState (None or null: off)
fn verbose_of(v: Option<&serde_json::Value>) -> Option<&'static str> {
    let v = v?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as u64;
    if v.get("until_ms")?.as_u64()? <= now {
        return None;
    }
    level_name(v.get("level"))
}

/// The IPC settings' verbose logging that stays on, in force only in
/// developer mode
fn always_of(settings: Option<&serde_json::Value>) -> Option<&'static str> {
    let s = settings?;
    if s.get("developer_mode")?.as_bool() != Some(true) {
        return None;
    }
    level_name(s.pointer("/dev_log/always_verbose"))
}

/// Service mode: this process' verbose logging follows the core's state
/// (a server message: the snapshot, or a verbose or settings event).
/// The connection calls it ([`crate::connection`]).
pub fn follow_core(msg: &serde_json::Value) {
    let Some(f) = FILTER.get() else { return };
    let Some(level) = f.followed.lock().unwrap_or_else(|e| e.into_inner()).take(msg) else { return };
    if f.handle.reload(filter_for(&f.base, level)).is_ok() {
        match level {
            Some(l) => tracing::info!("verbose logging ({l}) as the core's"),
            None => tracing::info!("verbose logging off, as the core's"),
        }
    }
}

/// The level of the core's verbose logging while it is on ("debug",
/// "trace"): its `verbose.json` in the logs folder
fn verbose_level(config: &std::path::Path) -> Option<&'static str> {
    let text = std::fs::read_to_string(paths::verbose_file(config)).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as u64;
    if v.get("until_ms")?.as_u64()? <= now {
        return None;
    }
    match v.get("level")?.as_str()? {
        "debug" => Some("debug"),
        "trace" => Some("trace"),
        _ => None,
    }
}

/// Developer mode's verbose logging that stays on (settings.json
/// `dev_log.always_verbose`, only while `debug_mode` is on)
fn always_verbose(config: &std::path::Path) -> Option<&'static str> {
    let text = std::fs::read_to_string(paths::settings_file(config)).ok()?;
    always_verbose_of(&serde_json::from_str(&text).ok()?)
}

fn always_verbose_of(settings: &serde_json::Value) -> Option<&'static str> {
    if settings.get("debug_mode")?.as_bool() != Some(true) {
        return None;
    }
    match settings.get("dev_log")?.get("always_verbose")?.as_str()? {
        "debug" => Some("debug"),
        "trace" => Some("trace"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Service mode: the core's state over the connection sets the level
    #[test]
    fn the_level_follows_the_cores_state() {
        use serde_json::json;
        let later =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64 + 60_000;
        let mut f = Followed::default();
        let snap = |verbose: serde_json::Value, dev: bool, always: serde_json::Value| {
            json!({ "type": "snapshot", "rev": 1, "state": {
                "verbose": verbose,
                "settings": { "developer_mode": dev, "dev_log": { "always_verbose": always } } } })
        };
        // The first snapshot sets the level, off included
        assert_eq!(f.take(&snap(json!(null), false, json!(null))), Some(None));
        assert_eq!(f.take(&snap(json!(null), false, json!(null))), None, "no change");
        let on = json!({ "type": "event", "rev": 2, "event": { "kind": "verbose",
            "value": { "level": "debug", "until_ms": later } } });
        assert_eq!(f.take(&on), Some(Some("debug")));
        // Developer mode's switch, higher: trace wins
        let dev = json!({ "type": "event", "rev": 3, "event": { "kind": "settings",
            "value": { "developer_mode": true, "dev_log": { "always_verbose": "trace" } } } });
        assert_eq!(f.take(&dev), Some(Some("trace")));
        // Developer mode off: the switch is not in force
        let off = json!({ "type": "event", "rev": 4, "event": { "kind": "settings",
            "value": { "developer_mode": false, "dev_log": { "always_verbose": "trace" } } } });
        assert_eq!(f.take(&off), Some(Some("debug")));
        // An expired one is off; other messages change nothing
        let expired = json!({ "type": "event", "rev": 5, "event": { "kind": "verbose",
            "value": { "level": "trace", "until_ms": 1 } } });
        assert_eq!(f.take(&expired), Some(None));
        assert_eq!(f.take(&json!({ "type": "event", "event": { "kind": "stream", "value": {} } })), None);
        assert_eq!(f.take(&json!({ "type": "response", "id": 1 })), None);
    }

    #[test]
    fn always_verbose_only_in_developer_mode() {
        let on = serde_json::json!({ "debug_mode": true, "dev_log": { "always_verbose": "trace" } });
        assert_eq!(always_verbose_of(&on), Some("trace"));
        let off = serde_json::json!({ "debug_mode": false, "dev_log": { "always_verbose": "debug" } });
        assert_eq!(always_verbose_of(&off), None);
        let unset = serde_json::json!({ "debug_mode": true, "dev_log": { "always_verbose": "" } });
        assert_eq!(always_verbose_of(&unset), None);
        // "trace" sorts after "debug": the higher of the two switches wins
        assert_eq!(Some("debug").max(Some("trace")), Some("trace"));
        assert_eq!(None.max(Some("debug")), Some("debug"));
    }

    #[test]
    fn verbose_turns_up_the_windows_own_modules() {
        // Parses (an invalid directive would fall back to plain "info")
        let f = filter_for("warn", Some("debug")).to_string();
        assert!(f.contains("ui_host=debug") && f.contains("bluecaster=debug"), "{f}");
        assert_eq!(filter_for("warn", None).to_string(), "warn");
    }
}
