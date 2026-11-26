//! Starting bluecaster.exe again after a failure this process cannot
//! recover from in place, with the window that was shown. The core keeps
//! streaming meanwhile. Restarts that follow each other quickly are
//! counted (in the environment the new process inherits), so a failure on
//! every start ends the UI instead of looping; the next click on the tray
//! icon starts it afresh.
//!
//! The WebView2 window of 3.0.x restarted this way when WebView2's browser
//! process died under it (a DLL that other software injects into every
//! process, RivaTuner's RTSSHooks64.dll, took it down on 2026-10-22). The
//! GPUI window has no such process to lose; the restart stays for it to use.

use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// "<restarts>,<unix seconds of the first>" for the restarts in a row
const RESTARTS_ENV: &str = "BLUECASTER_UI_RESTARTS";
/// Restarts closer together than this count as in a row...
const RESTART_SPAN: Duration = Duration::from_secs(120);
/// ...and after this many the UI ends instead
const MAX_RESTARTS: u32 = 2;

/// The arguments and the RESTARTS_ENV value to start again with
static RELAUNCH: Mutex<Option<(Vec<String>, String)>> = Mutex::new(None);

/// Asks for a new bluecaster.exe with `args` (`--flyout`, or none for the
/// main window) once this one has exited ([`relaunch_if_due`]). False when
/// it restarted too often in a row already: the UI just ends. The caller
/// ends the process either way.
pub fn schedule_relaunch(args: Vec<String>) -> bool {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (restarts, first) = next_restart(std::env::var(RESTARTS_ENV).ok().as_deref(), now);
    if restarts > MAX_RESTARTS {
        tracing::error!(
            "failed again ({} restarts within {} s); ending the UI, the tray icon starts it again",
            restarts - 1,
            RESTART_SPAN.as_secs()
        );
        return false;
    }
    tracing::error!("starting bluecaster again ({restarts}/{MAX_RESTARTS})");
    *RELAUNCH.lock().unwrap_or_else(|e| e.into_inner()) = Some((args, format!("{restarts},{first}")));
    true
}

/// The count and start of the restarts in a row, this one included
fn next_restart(previous: Option<&str>, now: u64) -> (u32, u64) {
    let parsed = previous.and_then(|v| {
        let (n, t) = v.split_once(',')?;
        Some((n.parse::<u32>().ok()?, t.parse::<u64>().ok()?))
    });
    match parsed {
        Some((n, first)) if now.saturating_sub(first) < RESTART_SPAN.as_secs() => (n + 1, first),
        _ => (1, now),
    }
}

/// At the very end of the process, after the single instance was let go
/// (drop [`crate::single_instance::Primary`] first, or the new process
/// only forwards its arguments to this one): starts the new bluecaster.exe
/// if one is due
pub fn relaunch_if_due() {
    let Some((args, restarts)) = RELAUNCH.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
    let started = std::env::current_exe()
        .and_then(|exe| std::process::Command::new(exe).args(&args).env(RESTARTS_ENV, restarts).spawn());
    if let Err(e) = started {
        tracing::error!("cannot start bluecaster again: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restarts_in_a_row_are_counted() {
        assert_eq!(next_restart(None, 1000), (1, 1000));
        assert_eq!(next_restart(Some("1,1000"), 1030), (2, 1000));
        assert_eq!(next_restart(Some("2,1000"), 1119), (3, 1000));
        // Long enough apart: a new series
        assert_eq!(next_restart(Some("2,1000"), 1120), (1, 1120));
        assert_eq!(next_restart(Some("garbage"), 1000), (1, 1000));
    }
}
