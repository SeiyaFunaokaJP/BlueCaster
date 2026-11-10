//! VB-CABLE installed when the user asks for it: VB-Audio's own driver pack,
//! fetched from VB-Audio's server at that moment (pinned by SHA-256, so a
//! pack VB-Audio replaced is refused rather than run), unpacked unmodified
//! into a folder of its own and its setup run silently as administrator.
//! BlueCaster itself ships none of it. The UI names VB-CABLE's origin and
//! that it is VB-Audio's donationware beside the Install button.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ipc::{ErrorInfo, Msg};
use sha2::{Digest, Sha256};
use win_sys::elevation::{self, ElevationError};

const PACK_URL: &str = "https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack46.zip";
const PACK_NAME: &str = "VBCABLE_Driver_Pack46.zip";
const PACK_SHA256: &str = "bc44e8974191a5a2427d6c979e2dab08dbb09925f3f435dfa2f0ba3e45b6eb90";
/// The pack is 1.3 MB
const MAX_PACK: u64 = 8 << 20;
const TIMEOUT: Duration = Duration::from_secs(60);
/// 64-bit Windows: x64, and ARM64 through emulation (the pack has the ARM64
/// driver)
const SETUP: &str = "VBCABLE_Setup_x64.exe";
/// How long the new outputs may take to show up after the setup ends
const APPEAR_WAIT: Duration = Duration::from_secs(10);

fn err(code: &str, key: &str) -> ErrorInfo {
    ErrorInfo { code: code.into(), message: Msg::new(key) }
}

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

/// The pack: from the download cache when it is the pinned one there (a
/// second try needs no download), else from VB-Audio
fn pack() -> Result<Vec<u8>, ErrorInfo> {
    let cache = bluecaster_core::firmware::cache_dir().join(PACK_NAME);
    if let Ok(data) = std::fs::read(&cache)
        && sha256_hex(&data) == PACK_SHA256
    {
        tracing::debug!("VB-CABLE: pack from the cache");
        return Ok(data);
    }
    let failed = || err("vb_cable.download_failed", "error.vb_cable_download_failed");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .user_agent(concat!("BlueCaster/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let started = Instant::now();
    let mut response = agent.get(PACK_URL).call().map_err(|e| {
        tracing::warn!("VB-CABLE: download failed: {e}");
        failed()
    })?;
    let data = response.body_mut().with_config().limit(MAX_PACK).read_to_vec().map_err(|e| {
        tracing::warn!("VB-CABLE: download failed: {e}");
        failed()
    })?;
    let hash = sha256_hex(&data);
    tracing::info!("VB-CABLE: pack downloaded, {} bytes in {} ms", data.len(), started.elapsed().as_millis());
    if hash != PACK_SHA256 {
        // VB-Audio changed the file: not run; the UI offers VB-Audio's page
        tracing::warn!("VB-CABLE: the pack is not the expected one (SHA-256 {hash})");
        return Err(err("vb_cable.pack_changed", "error.vb_cable_pack_changed"));
    }
    if let Some(dir) = cache.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&cache, &data);
    Ok(data)
}

/// Unpacks the pack, as it is, into a new folder of its own; the setup's path
fn unpack(data: &[u8]) -> Result<PathBuf, ErrorInfo> {
    let failed = || err("vb_cable.install_failed", "error.vb_cable_install_failed");
    let files = crate::zip::read_all(data).ok_or_else(|| {
        tracing::warn!("VB-CABLE: the pack could not be read");
        failed()
    })?;
    let dir = std::env::temp_dir().join(format!("bluecaster-vbcable-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| {
        tracing::warn!("VB-CABLE: folder not made: {e}");
        failed()
    })?;
    for (name, contents) in &files {
        // The pack is flat: a name with a path in it is not VB-Audio's
        if name.is_empty() || name.contains(['/', '\\', ':']) || name.starts_with('.') {
            tracing::warn!("VB-CABLE: unexpected entry in the pack");
            return Err(failed());
        }
        std::fs::write(dir.join(name), contents).map_err(|e| {
            tracing::warn!("VB-CABLE: file not written: {e}");
            failed()
        })?;
    }
    let setup = dir.join(SETUP);
    if !setup.is_file() {
        tracing::warn!("VB-CABLE: no {SETUP} in the pack");
        return Err(failed());
    }
    Ok(setup)
}

fn vb_cable_listed() -> bool {
    audio_capture::render_devices()
        .is_ok_and(|list| list.iter().any(|d| bluecaster_core::profiles::is_vb_cable(&d.name, d.is_virtual)))
}

/// Fetches the pack, runs its setup ("-i -h": install, no window) as
/// administrator and waits for VB-CABLE's output to appear. Ok(false): it
/// did not appear (yet); VB-Audio's setup may want Windows restarted first.
pub fn install() -> Result<bool, ErrorInfo> {
    let setup = unpack(&pack()?)?;
    let result = run(&setup);
    if let Some(dir) = setup.parent() {
        let _ = std::fs::remove_dir_all(dir);
    }
    result?;
    let started = Instant::now();
    loop {
        if vb_cable_listed() {
            return Ok(true);
        }
        if started.elapsed() >= APPEAR_WAIT {
            tracing::info!("VB-CABLE: not among the outputs {APPEAR_WAIT:?} after its setup");
            return Ok(false);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn run(setup: &Path) -> Result<(), ErrorInfo> {
    let code = elevation::run_elevated(setup, &["-i", "-h"]).map_err(|e| match e {
        ElevationError::Cancelled => err("vb_cable.uac_cancelled", "error.vb_cable_uac_cancelled"),
        e => {
            tracing::error!("VB-CABLE setup: {e}");
            err("vb_cable.install_failed", "error.vb_cable_install_failed")
        }
    })?;
    tracing::info!("VB-CABLE setup -> exit {code}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Network: VB-Audio still serves the pinned pack, and it unpacks with
    /// its setup (the setup is not run)
    #[test]
    #[ignore]
    fn fetches_and_unpacks_the_pack() {
        let setup = unpack(&pack().unwrap()).unwrap();
        let dir = setup.parent().unwrap().to_path_buf();
        assert!(std::fs::read_dir(&dir).unwrap().count() > 20);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
