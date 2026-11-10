//! VB-CABLE from the package shipped beside the core, unmodified
//! (installer/build-release.ps1 puts VB-Audio's driver pack in `vb-cable\`),
//! installed when the user asks for it: its own setup, silently, as
//! administrator. VB-Audio allows this as long as the user can tell it is
//! their donationware, which the UI says beside the Install button.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use ipc::{ErrorInfo, Msg};
use win_sys::elevation::{self, ElevationError};

fn err(code: &str, key: &str) -> ErrorInfo {
    ErrorInfo { code: code.into(), message: Msg::new(key) }
}

/// The package's setup (64-bit Windows: x64, and ARM64 through emulation,
/// which installs the ARM64 driver), None where there is no package
pub fn setup() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.parent()?.join("vb-cable").join("VBCABLE_Setup_x64.exe");
    exe.is_file().then_some(exe)
}

/// How long the new outputs may take to show up after the setup ends
const APPEAR_WAIT: Duration = Duration::from_secs(10);

/// Runs the setup ("-i -h": install, no window) and waits for VB-CABLE's
/// output to appear. Ok(false): it did not appear (yet); VB-Audio's setup
/// may want Windows restarted first.
pub fn install() -> Result<bool, ErrorInfo> {
    let Some(exe) = setup() else {
        return Err(err("vb_cable.not_bundled", "error.vb_cable_not_bundled"));
    };
    let code = elevation::run_elevated(&exe, &["-i", "-h"]).map_err(|e| match e {
        ElevationError::Cancelled => err("vb_cable.uac_cancelled", "error.vb_cable_uac_cancelled"),
        e => {
            tracing::error!("VB-CABLE setup: {e}");
            err("vb_cable.install_failed", "error.vb_cable_install_failed")
        }
    })?;
    tracing::info!("VB-CABLE setup -> exit {code}");
    let started = Instant::now();
    loop {
        let found = audio_capture::render_devices()
            .is_ok_and(|list| list.iter().any(|d| bluecaster_core::profiles::is_vb_cable(&d.name, d.is_virtual)));
        if found {
            return Ok(true);
        }
        if started.elapsed() >= APPEAR_WAIT {
            tracing::info!("VB-CABLE: not among the outputs {APPEAR_WAIT:?} after its setup");
            return Ok(false);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}
