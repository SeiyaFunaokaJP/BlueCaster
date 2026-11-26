//! What the window remembers across starts that is no setting of the core:
//! the release whose update dialog was answered ("Later"), "Show only
//! supported codecs" on Codecs, and Home's bar about another Bluetooth
//! adapter, once dismissed. They are `ui-state.json` in the signed-in
//! user's own BlueCaster folder (`%APPDATA%\BlueCaster`, or
//! BLUECASTER_CONFIG_DIR for a test instance). That folder stays the user's also while the core runs as a
//! Windows service (whose own data folder this process may not write to).
//!
//! The file is read once, at the first use, and written whole (a temporary
//! file renamed over it) when a value changes. Keys this version does not
//! know are kept. A file that cannot be read or written is logged and
//! otherwise ignored: the values then hold until the window process ends.
//! The demo state (BLUECASTER_UI_DEMO) never touches the file.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{Map, Value};

/// The file's name in the user's folder
const FILE_NAME: &str = "ui-state.json";

/// The update release answered with "Later" (or Update now): its dialog
/// does not open again
const UPDATE_LATER: &str = "update_later";
/// Codecs' "Show only supported codecs"; on unless turned off
const HIDE_UNSUPPORTED: &str = "codecs_hide_unsupported";
/// Home's bar about another active Bluetooth adapter, dismissed for good
const OTHER_RADIO_DISMISSED: &str = "other_radio_dismissed";

/// The values, and where they are kept (None: in memory only)
struct Store {
    file: Option<PathBuf>,
    values: Map<String, Value>,
}

static STORE: Mutex<Option<Store>> = Mutex::new(None);

/// Runs `f` on the store, read from the file at the first use
fn with_store<R>(f: impl FnOnce(&mut Store) -> R) -> R {
    let mut guard = STORE.lock().unwrap_or_else(|e| e.into_inner());
    let store = guard.get_or_insert_with(|| {
        let file = file_path();
        let values = file.as_deref().map(load).unwrap_or_default();
        Store { file, values }
    });
    f(store)
}

/// `ui-state.json` in the user's folder; None for the demo state and in tests
fn file_path() -> Option<PathBuf> {
    if cfg!(test) || std::env::var_os("BLUECASTER_UI_DEMO").is_some() {
        return None;
    }
    Some(ui_host::logging::paths::user_config_dir().join(FILE_NAME))
}

/// The values a file holds: none when it is missing, unreadable or no JSON
/// object
fn load(file: &Path) -> Map<String, Value> {
    match std::fs::read_to_string(file) {
        Ok(text) => parse(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Map::new(),
        Err(e) => {
            tracing::warn!("the window's state was not read: {:?}", e.kind());
            Map::new()
        }
    }
}

fn parse(text: &str) -> Map<String, Value> {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(values)) => values,
        Ok(_) => {
            tracing::warn!("the window's state is no JSON object; starting empty");
            Map::new()
        }
        Err(e) => {
            tracing::warn!("the window's state was not read: {e}");
            Map::new()
        }
    }
}

/// Writes the values to a temporary file next to `file`, then renames it
/// over `file` (which a crash then never leaves half written)
fn save(file: &Path, values: &Map<String, Value>) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(values).map_err(std::io::Error::other)?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, file).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Sets a value; the file is written when it changed
fn set(key: &str, value: Value) {
    with_store(|store| {
        if store.values.get(key) == Some(&value) {
            return;
        }
        store.values.insert(key.to_owned(), value);
        if let Some(file) = &store.file
            && let Err(e) = save(file, &store.values)
        {
            tracing::warn!("the window's state was not saved: {:?}", e.kind());
        }
    });
}

fn get(key: &str) -> Option<Value> {
    with_store(|store| store.values.get(key).cloned())
}

/// The release whose update dialog was answered (Later, Update now)
pub fn update_later() -> Option<String> {
    get(UPDATE_LATER).and_then(|v| v.as_str().map(str::to_owned))
}

pub fn set_update_later(version: &str) {
    set(UPDATE_LATER, Value::from(version));
}

/// Codecs' "Show only supported codecs" (on unless turned off)
pub fn hide_unsupported() -> bool {
    get(HIDE_UNSUPPORTED).and_then(|v| v.as_bool()).unwrap_or(true)
}

pub fn set_hide_unsupported(hide: bool) {
    set(HIDE_UNSUPPORTED, Value::from(hide));
}

/// Home's bar about another Bluetooth adapter was dismissed (for good)
pub fn other_radio_dismissed() -> bool {
    get(OTHER_RADIO_DISMISSED).and_then(|v| v.as_bool()).unwrap_or(false)
}

pub fn set_other_radio_dismissed() {
    set(OTHER_RADIO_DISMISSED, Value::from(true));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh folder of its own under the system's temp folder
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bluecaster-ui-state-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn values_survive_a_save_and_a_load() {
        let dir = temp_dir("roundtrip");
        let file = dir.join(FILE_NAME);
        let mut values = Map::new();
        values.insert(UPDATE_LATER.into(), Value::from("3.0.5"));
        values.insert(HIDE_UNSUPPORTED.into(), Value::from(false));
        values.insert(OTHER_RADIO_DISMISSED.into(), Value::from(true));
        save(&file, &values).unwrap();
        assert_eq!(load(&file), values);
        // No temporary file left behind
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unknown_keys_survive_a_change() {
        let dir = temp_dir("unknown");
        let file = dir.join(FILE_NAME);
        std::fs::write(&file, r#"{"later_version_key": {"a": [1, 2]}, "codecs_hide_unsupported": true}"#).unwrap();
        let mut values = load(&file);
        values.insert(HIDE_UNSUPPORTED.into(), Value::from(false));
        save(&file, &values).unwrap();
        let again = load(&file);
        assert_eq!(again.get("later_version_key"), Some(&serde_json::json!({"a": [1, 2]})));
        assert_eq!(again.get(HIDE_UNSUPPORTED), Some(&Value::from(false)));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_or_broken_file_reads_as_empty() {
        let dir = temp_dir("broken");
        let file = dir.join(FILE_NAME);
        assert!(load(&file).is_empty());
        std::fs::write(&file, "{not json").unwrap();
        assert!(load(&file).is_empty());
        std::fs::write(&file, "[1, 2]").unwrap();
        assert!(load(&file).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_save_replaces_the_file() {
        let dir = temp_dir("replace");
        let file = dir.join(FILE_NAME);
        let mut values = Map::new();
        values.insert(UPDATE_LATER.into(), Value::from("3.0.5"));
        save(&file, &values).unwrap();
        values.insert(UPDATE_LATER.into(), Value::from("3.0.6"));
        save(&file, &values).unwrap();
        assert_eq!(load(&file).get(UPDATE_LATER), Some(&Value::from("3.0.6")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
