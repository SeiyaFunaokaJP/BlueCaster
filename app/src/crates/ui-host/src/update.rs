//! In-app updates. The core's daily check finds the release (and its
//! manifest, latest.json); here the installer is downloaded and its
//! signature checked against the key built into this program, then
//! installed once nothing streams:
//!
//! 1. download (the progress goes to the window as [`HostEvent::UpdatePhase`]);
//! 2. ask the core `app.quit_for_update`. It refuses while it streams or
//!    connects, and the request is repeated every few seconds until it
//!    agrees: an update never cuts the audio;
//! 3. wait until the core has exited (stream stopped, adapter released),
//!    without starting it again or ending this process on its Bye;
//! 4. run the installer (Inno Setup, `/UPDATE /SILENT`: progress only, no
//!    questions); this process exits. The installer starts bluecaster.exe again,
//!    which starts the core.
//!
//! Only an installed copy with an updater key updates itself. The portable
//! copy (no unins000.exe next to it) and builds without a key show the
//! release page instead (Phase::Unavailable).
//!
//! A window connected to the service add-on updates the user's own install
//! as well: the service runs its own copy of the core from Program Files,
//! so nothing of the user's install is in use but this program. The
//! service refuses `app.quit_for_update` (Windows starts and stops it):
//! there is no core of the user's to wait for, and this process just
//! closes for the installer. The next start finds the service at the old
//! version and offers its update (service.rs).
//!
//! What tauri-plugin-updater 2 did, done here without Tauri, with the same
//! formats and rules: the manifest is Tauri's latest.json (`version`,
//! `platforms."windows-x86_64"` / `"windows-aarch64"` with `url` and
//! `signature`); the signature is Tauri's (a minisign signature over the
//! installer's bytes, base64-wrapped like the public key), and the version
//! in its trusted comment must be the manifest's (requireSignedVersion);
//! the installer runs as the plugin ran an NSIS setup.exe in its "basicUi"
//! mode.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::connection::{Host, HostEvent, lock};

/// Manifests are taken from this repository's releases only
const MANIFEST_PREFIX: &str = "https://github.com/SeiyaFunaokaJP/BlueCaster/releases/download/";
/// Asking the core again while it streams
const RETRY_INTERVAL: Duration = Duration::from_secs(3);
/// Stopping a stream and resetting the adapter takes a few seconds
const CORE_EXIT_TIMEOUT: Duration = Duration::from_secs(20);
/// The updater key built into this program: installer/build-release.ps1
/// sets BLUECASTER_UPDATER_PUBKEY for a signed release (build.rs); empty
/// in other builds
const PUBKEY: &str = env!("BLUECASTER_UPDATER_PUBKEY_BUILT");
/// The installer's command line, as tauri-plugin-updater built it for an
/// NSIS setup.exe in "basicUi" mode (`/UPDATE`, none of NSIS's own options)
/// with our arguments. Inno Setup's /SILENT is NSIS's passive mode;
/// BlueCaster.iss starts bluecaster.exe again after an /UPDATE, so no
/// /R /ARGS (restart_after_install off)
const INSTALLER_ARGS: &str = "/UPDATE /SILENT /NORESTART";
/// The product name (Tauri's productName), in the installer's temporary
/// file names as the plugin named them
const APP_NAME: &str = "BlueCaster";
const USER_AGENT: &str = concat!("bluecaster-ui/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Phase {
    /// No in-app update: "portable" (nothing installed to update) or
    /// "unsigned" (built without the updater key)
    Unavailable {
        reason: &'static str,
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

/// A downloaded installer that passed the signature check
struct Ready {
    version: String,
    signature: String,
    bytes: Vec<u8>,
}

pub(crate) struct Updates {
    phase: Mutex<Phase>,
    ready: Mutex<Option<Ready>>,
}

/// The updater key is built in
fn has_pubkey() -> bool {
    !PUBKEY.trim().is_empty()
}

/// Installed by the Inno Setup installer (installer/BlueCaster.iss), which
/// leaves its uninstaller here
fn installed() -> bool {
    std::env::current_exe().map(|e| e.with_file_name("unins000.exe").is_file()).unwrap_or(false)
}

impl Updates {
    pub(crate) fn new() -> Updates {
        let phase = if !installed() {
            Phase::Unavailable { reason: "portable" }
        } else if !has_pubkey() {
            Phase::Unavailable { reason: "unsigned" }
        } else {
            Phase::Idle
        };
        Updates { phase: Mutex::new(phase), ready: Mutex::new(None) }
    }
}

impl Host {
    /// The in-app update's state (also sent as [`HostEvent::UpdatePhase`]).
    /// It updates the user's own install, also while the window uses the
    /// service add-on: the service has its own copy, which then needs an
    /// update (service.rs).
    pub fn update_phase(&self) -> Phase {
        lock(&self.updates.phase).clone()
    }

    /// Downloading or waiting to install: the process must stay
    pub fn update_busy(&self) -> bool {
        matches!(
            *lock(&self.updates.phase),
            Phase::Downloading { .. } | Phase::Waiting { .. } | Phase::Installing { .. }
        )
    }

    fn set_phase(&self, phase: Phase) {
        *lock(&self.updates.phase) = phase.clone();
        self.emit(HostEvent::UpdatePhase(phase));
    }

    fn update_failed(&self, code: &str, detail: impl ToString) {
        let detail = detail.to_string();
        eprintln!("bluecaster: update failed ({code}): {detail}");
        // The code only: the detail may name a path or an address
        tracing::warn!("update failed ({code})");
        self.set_phase(Phase::Failed { code: code.into(), detail });
    }

    /// Starts the update to the release whose manifest is `manifest_url`
    /// (UpdateInfo.manifest_url): downloads it and installs it as soon as
    /// nothing streams. Only this repository's release manifests are
    /// accepted. Ok also when one is already under way. Installing ends
    /// this process (the installer starts it again).
    pub fn update_install(self: &Arc<Self>, manifest_url: String) -> Result<(), String> {
        match self.update_phase() {
            Phase::Unavailable { reason } => return Err(format!("no in-app update ({reason})")),
            Phase::Downloading { .. } | Phase::Waiting { .. } | Phase::Installing { .. } => return Ok(()),
            Phase::Idle | Phase::Failed { .. } => {}
        }
        if !manifest_url.starts_with(MANIFEST_PREFIX) || !manifest_url.ends_with("/latest.json") {
            return Err(format!("not a manifest of BlueCaster's releases: {manifest_url}"));
        }
        let url: reqwest::Url = manifest_url.parse().map_err(|e| format!("{manifest_url}: {e}"))?;
        self.set_phase(Phase::Downloading { version: String::new(), downloaded: 0, total: None });
        let host = self.clone();
        self.runtime.spawn(async move {
            if let Err((code, detail)) = download(&host, url).await {
                host.update_failed(code, detail);
                return;
            }
            loop {
                match install(&host).await {
                    Ok(true) => tokio::time::sleep(RETRY_INTERVAL).await,
                    // install() returns only on failure: success ends the process
                    Ok(false) => return,
                    Err((code, detail)) => {
                        host.set_updating(false);
                        host.update_failed(code, detail);
                        return;
                    }
                }
            }
        });
        Ok(())
    }
}

type Failure = (&'static str, String);

/// The updater's platform key: Tauri's `{os}-{arch}`
fn target() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "windows-aarch64"
    } else if cfg!(target_arch = "x86") {
        "windows-i686"
    } else {
        "windows-x86_64"
    }
}

/// A release from the manifest, for this platform
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: semver::Version,
    pub url: reqwest::Url,
    /// Tauri's signature: the minisign signature, base64-wrapped
    pub signature: String,
}

#[derive(Deserialize)]
struct Platform {
    url: String,
    signature: String,
}

/// Tauri's update manifest: the static form (`platforms`) or the one a
/// server answers for the platform it was asked about (`url`, `signature`)
#[derive(Deserialize)]
struct Manifest {
    #[serde(alias = "name")]
    version: String,
    pub_date: Option<String>,
    platforms: Option<std::collections::HashMap<String, Platform>>,
    url: Option<String>,
    signature: Option<String>,
}

/// Reads the manifest `json` for the platform `target` ("windows-x86_64").
/// The version may start with a "v"; the download must be https.
pub fn parse_manifest(json: &[u8], target: &str) -> Result<Release, String> {
    let m: Manifest = serde_json::from_slice(json).map_err(|e| format!("not an update manifest: {e}"))?;
    let version = parse_version(&m.version).map_err(|e| format!("version {:?}: {e}", m.version))?;
    if let Some(date) = &m.pub_date {
        time::OffsetDateTime::parse(date, &time::format_description::well_known::Rfc3339)
            .map_err(|e| format!("invalid value for `pub_date`: {e}"))?;
    }
    let platform = match (m.platforms, m.url, m.signature) {
        (Some(mut platforms), _, _) => {
            platforms.remove(target).ok_or_else(|| format!("the manifest has no release for {target}"))?
        }
        (None, Some(url), Some(signature)) => Platform { url, signature },
        (None, _, _) => return Err("the manifest has neither `platforms` nor `url` and `signature`".into()),
    };
    let url: reqwest::Url = platform.url.parse().map_err(|e| format!("download URL: {e}"))?;
    if url.scheme() != "https" {
        return Err("the download URL is not https".into());
    }
    Ok(Release { version, url, signature: platform.signature })
}

fn parse_version(v: &str) -> Result<semver::Version, semver::Error> {
    semver::Version::parse(v.trim_start_matches('v'))
}

/// The release is newer than this program (semver: an alpha comes after its
/// beta, "3.0.0-beta.4.alpha.1" > "3.0.0-beta.4")
fn is_newer(release: &semver::Version, current: &str) -> Result<bool, String> {
    let current = parse_version(current).map_err(|e| format!("this program's version: {e}"))?;
    Ok(*release > current)
}

fn base64_to_string(text: &str) -> Result<String, String> {
    let bytes =
        base64::engine::general_purpose::STANDARD.decode(text.trim()).map_err(|e| format!("not base64: {e}"))?;
    String::from_utf8(bytes).map_err(|_| "not UTF-8 inside the base64".into())
}

/// Checks the installer's bytes `data` against Tauri's `signature` (a
/// minisign signature, base64-wrapped) and the key `pubkey` (a minisign
/// public key, base64-wrapped), as tauri-plugin-updater does. With
/// `require_signed_version`, the signature must name the version (the
/// Tauri CLI's `--app-version`), and that version must be the manifest's
/// `announced_version`: the manifest is not signed, so a changed one could
/// otherwise pair a newer version number with an older signed installer.
pub fn verify_signature(
    data: &[u8],
    signature: &str,
    pubkey: &str,
    announced_version: &str,
    require_signed_version: bool,
) -> Result<(), String> {
    let key =
        minisign_verify::PublicKey::decode(&base64_to_string(pubkey)?).map_err(|e| format!("the updater key: {e}"))?;
    let signature =
        minisign_verify::Signature::decode(&base64_to_string(signature)?).map_err(|e| format!("the signature: {e}"))?;
    key.verify(data, &signature, true).map_err(|e| e.to_string())?;
    // Only now is the trusted comment usable: minisign's global signature
    // covers it, and verify above checked that
    verify_signed_version(signature.trusted_comment(), announced_version, require_signed_version)
}

/// The version the installer was signed for against the manifest's
fn verify_signed_version(trusted_comment: &str, announced: &str, require_signed_version: bool) -> Result<(), String> {
    let Some(signed) = signed_version(trusted_comment) else {
        return if require_signed_version { Err("the signature names no version".into()) } else { Ok(()) };
    };
    // As semver, so "1.2.3" and "v1.2.3" match; literally otherwise
    let matches = match (parse_version(signed), parse_version(announced)) {
        (Ok(s), Ok(a)) => s == a,
        _ => signed == announced,
    };
    if matches { Ok(()) } else { Err(format!("signed for version {signed}, announced as {announced}")) }
}

/// The `version` field of a signature's trusted comment, which the Tauri
/// CLI writes as tab-separated `key:value` pairs
/// (`timestamp:1700000000\tfile:app.exe\tversion:1.2.3`)
fn signed_version(trusted_comment: &str) -> Option<&str> {
    trusted_comment.split('\t').find_map(|field| field.strip_prefix("version:"))
}

/// The rustls crypto the client uses (ring, as the plugin installed it)
fn client() -> Result<reqwest::Client, String> {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        // Fails only when another thread installed one just now
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    // The system's proxy settings (reqwest "system-proxy"); Windows'
    // certificate store (rustls-platform-verifier)
    reqwest::Client::builder().user_agent(USER_AGENT).build().map_err(|e| e.to_string())
}

/// Fetches the manifest -> the release for this platform; None: not newer
async fn check(client: &reqwest::Client, url: reqwest::Url) -> Result<Option<Release>, Failure> {
    let res = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| ("update.check", e.to_string()))?;
    if res.status() == reqwest::StatusCode::NO_CONTENT {
        return Ok(None);
    }
    if !res.status().is_success() {
        return Err(("update.check", format!("the manifest's server answered {}", res.status())));
    }
    let body = res.bytes().await.map_err(|e| ("update.check", e.to_string()))?;
    let release = parse_manifest(&body, target()).map_err(|e| ("update.check", e))?;
    let newer = is_newer(&release.version, crate::APP_VERSION).map_err(|e| ("update.config", e))?;
    Ok(newer.then_some(release))
}

async fn download(host: &Host, url: reqwest::Url) -> Result<(), Failure> {
    let client = client().map_err(|e| ("update.config", e))?;
    let Some(release) = check(&client, url).await? else {
        return Err(("update.not_newer", "the manifest has no newer version for this platform".into()));
    };
    let version = release.version.to_string();
    let failed = |e: String| ("update.download", e);
    let mut res = client
        .get(release.url.clone())
        .header(reqwest::header::ACCEPT, "application/octet-stream")
        .send()
        .await
        .map_err(|e| failed(e.to_string()))?;
    if !res.status().is_success() {
        return Err(failed(format!("Download request failed with status: {}", res.status())));
    }
    let total: Option<u64> =
        res.headers().get(reqwest::header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok());
    let mut bytes = Vec::new();
    let mut last_emit = Instant::now();
    while let Some(chunk) = res.chunk().await.map_err(|e| failed(e.to_string()))? {
        bytes.extend_from_slice(&chunk);
        if last_emit.elapsed() >= Duration::from_millis(200) {
            last_emit = Instant::now();
            let downloaded = bytes.len() as u64;
            host.set_phase(Phase::Downloading { version: version.clone(), downloaded, total });
        }
    }
    // Checked against the built-in key before anything else happens to it
    verify_signature(&bytes, &release.signature, PUBKEY, &version, true).map_err(failed)?;
    tracing::info!("update: {} bytes downloaded and verified", bytes.len());
    *lock(&host.updates.ready) = Some(Ready { version: version.clone(), signature: release.signature, bytes });
    host.set_phase(Phase::Waiting { version });
    Ok(())
}

/// Ok(true): the core streams, ask again later. Ok(false) cannot happen
/// on Windows (the install ends the process) but ends the loop elsewhere.
async fn install(host: &Arc<Host>) -> Result<bool, Failure> {
    // From here a Bye of the core does not end this process, and the core
    // is not started again when its pipe goes away
    host.set_updating(true);
    match host.call("update", "app.quit_for_update", None).await {
        Ok(_) => {}
        Err(e) if e.code == "app.streaming" => {
            host.set_updating(false);
            return Ok(true);
        }
        // The service add-on's core: it stays, and its files are not the
        // installer's. Streaming does not stop for this either.
        Err(e) if e.code == ipc::access::NOT_IN_SERVICE => {}
        // Not running (or gone while answering): nothing to stop
        Err(e) if e.code.starts_with("ipc.") => {}
        Err(e) => return Err(("update.core", e.code)),
    }
    let start = Instant::now();
    while win_sys::single_instance::is_held(&ipc::core_mutex_name()) {
        if start.elapsed() >= CORE_EXIT_TIMEOUT {
            return Err(("update.core_running", "bluecaster-core.exe did not exit".into()));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let Some(ready) = lock(&host.updates.ready).take() else {
        return Err(("update.lost", "the downloaded installer is gone".into()));
    };
    host.set_phase(Phase::Installing { version: ready.version.clone() });
    run_installer(&ready).map_err(|e| ("update.install", e))?;
    Ok(false)
}

/// The installer's file starts as every Windows program does ("MZ")
fn is_exe(bytes: &[u8]) -> bool {
    bytes.starts_with(b"MZ")
}

/// Writes the installer to a folder of its own in %TEMP% and starts it as
/// the plugin did (ShellExecute "open", shown, [`INSTALLER_ARGS`]), then
/// ends this process. Returns only on failure.
fn run_installer(ready: &Ready) -> Result<(), String> {
    use std::io::Write;
    // As the plugin's install(): checked again right before it runs
    verify_signature(&ready.bytes, &ready.signature, PUBKEY, &ready.version, true)?;
    if !is_exe(&ready.bytes) {
        return Err("the update is not an installer program".into());
    }
    let dir = tempfile::Builder::new()
        .prefix(&format!("{APP_NAME}-{}-updater-", ready.version))
        .tempdir()
        .map_err(|e| e.to_string())?
        .keep();
    let mut file = tempfile::Builder::new()
        .prefix(&format!("{APP_NAME}-{}-installer", ready.version))
        .suffix(".exe")
        .rand_bytes(0)
        .tempfile_in(&dir)
        .map_err(|e| e.to_string())?;
    file.write_all(&ready.bytes).map_err(|e| e.to_string())?;
    // Deleted again if it cannot be started; left for the installer when
    // it was (the process ends without dropping it)
    let path = file.into_temp_path();
    win_sys::shell::run(&path, INSTALLER_ARGS).map_err(|e| e.to_string())?;
    tracing::info!("update: installer started; bluecaster exiting");
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    // A throwaway key pair made with `tauri signer generate`, and the
    // signature of the bytes "test" made with `tauri signer sign
    // --app-version 2.0.0` (tauri-plugin-updater 2.14's own test vector)
    const PUBLIC_KEY: &str = "untrusted comment: minisign public key: A49EF4746FA6E6D7\nRWTX5qZvdPSepDGWg2bz8n6ArQnBMwgQK1geVosorcbLzcunD+KJ+cYe";
    const SIGNATURE: &str = "untrusted comment: signature from tauri secret key\nRUTX5qZvdPSepPF2zsjvP/L/TQtBLRnWRvaq+4omK1UEugtQp/USrGiI1v762sDy3id3aor7bRC9ETyeot6lMHHrWoZ5hjUM1Qc=\ntrusted comment: timestamp:1791649051\tfile:payload\tversion:2.0.0\nDDq2CLOEl5cQmC2eJRHlnZPVViOZsu1CjpPKQoei4A/XBfkDqGO0B+QMrcrHgEQ5qV3mne75tdFWhJpqUXh9AA==";
    // minisign-verify's public test vector of "test", signed without a version
    const UNVERSIONED_PUBLIC_KEY: &str = "untrusted comment: minisign public key E7620F1842B4E81F\nRWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
    const UNVERSIONED_SIGNATURE: &str = "untrusted comment: signature from minisign secret key\nRUQf6LRCGA9i559r3g7V1qNyJDApGip8MfqcadIgT9CuhV3EMhHoN1mGTkUidF/z7SrlQgXdy8ofjb7bNJJylDOocrCo8KLzZwo=\ntrusted comment: timestamp:1633700835\tfile:test\tprehashed\nwLMDjy9FLAuxZ3q4NlEvkgtyhrr0gtTu6KC4KBJdITbbOeAi1zBIYo0v4iTgt8jJpIidRJnp94ABQkJAgAooBQ==";

    fn b64(s: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(s)
    }

    #[test]
    fn a_signed_installer_of_the_announced_version_passes() {
        let (key, sig) = (b64(PUBLIC_KEY), b64(SIGNATURE));
        assert_eq!(verify_signature(b"test", &sig, &key, "2.0.0", true), Ok(()));
        assert_eq!(verify_signature(b"test", &sig, &key, "v2.0.0", true), Ok(()));
    }

    #[test]
    fn changed_bytes_another_key_or_another_version_fail() {
        let (key, sig) = (b64(PUBLIC_KEY), b64(SIGNATURE));
        assert!(verify_signature(b"tesT", &sig, &key, "2.0.0", true).is_err());
        assert!(verify_signature(b"test", &sig, &b64(UNVERSIONED_PUBLIC_KEY), "2.0.0", true).is_err());
        // The manifest announces a newer version than the signed one
        assert!(verify_signature(b"test", &sig, &key, "2.0.1", true).is_err());
        assert!(verify_signature(b"test", &sig, &key, "2.0.1", false).is_err());
        // Not base64, not a key
        assert!(verify_signature(b"test", &sig, PUBLIC_KEY, "2.0.0", true).is_err());
        assert!(verify_signature(b"test", &sig, "", "2.0.0", true).is_err());
    }

    #[test]
    fn a_signature_without_a_version_is_refused_when_one_is_required() {
        let (key, sig) = (b64(UNVERSIONED_PUBLIC_KEY), b64(UNVERSIONED_SIGNATURE));
        assert_eq!(verify_signature(b"test", &sig, &key, "9.9.9", false), Ok(()));
        assert!(verify_signature(b"test", &sig, &key, "9.9.9", true).is_err());
    }

    #[test]
    fn the_signed_version_is_read_from_the_trusted_comment() {
        assert_eq!(signed_version("timestamp:1700000000\tfile:app.exe\tversion:1.2.3"), Some("1.2.3"));
        assert_eq!(signed_version("timestamp:1600000000\tfile:app.exe"), None);
        assert_eq!(signed_version("timestamp:1\tfile:app-version:2.zip"), None);
        // Not semver: compared literally
        assert!(verify_signed_version("version:2024-01-01", "2024-01-01", true).is_ok());
        assert!(verify_signed_version("version:2024-01-01", "2024-01-02", true).is_err());
    }

    const MANIFEST: &str = r#"{
        "version": "3.0.5",
        "notes": "See the release page",
        "pub_date": "2026-11-23T10:00:00Z",
        "platforms": {
            "windows-x86_64": {
                "signature": "c2ln",
                "url": "https://github.com/SeiyaFunaokaJP/BlueCaster/releases/download/v3.0.5/BlueCaster-3.0.5-win64-setup.exe"
            },
            "windows-aarch64": {
                "signature": "YXJt",
                "url": "https://github.com/SeiyaFunaokaJP/BlueCaster/releases/download/v3.0.5/BlueCaster-3.0.5-arm64-setup.exe"
            }
        }
    }"#;

    #[test]
    fn the_manifest_gives_this_platforms_installer() {
        let x64 = parse_manifest(MANIFEST.as_bytes(), "windows-x86_64").unwrap();
        assert_eq!(x64.version, semver::Version::new(3, 0, 5));
        assert_eq!(x64.signature, "c2ln");
        assert!(x64.url.as_str().ends_with("-win64-setup.exe"));
        let arm = parse_manifest(MANIFEST.as_bytes(), "windows-aarch64").unwrap();
        assert!(arm.url.as_str().ends_with("-arm64-setup.exe"));
        assert!(parse_manifest(MANIFEST.as_bytes(), "windows-i686").is_err());
        assert!(target().starts_with("windows-"));
    }

    #[test]
    fn manifest_forms_and_what_is_refused() {
        // A server's answer for one platform; "name" and a "v" accepted
        let dynamic = r#"{ "name": "v3.1.0", "url": "https://example.invalid/setup.exe", "signature": "c2ln" }"#;
        let r = parse_manifest(dynamic.as_bytes(), "windows-x86_64").unwrap();
        assert_eq!(r.version, semver::Version::new(3, 1, 0));
        let http = r#"{ "version": "3.1.0", "url": "http://example.invalid/setup.exe", "signature": "c2ln" }"#;
        assert!(parse_manifest(http.as_bytes(), "windows-x86_64").is_err());
        let no_url = r#"{ "version": "3.1.0" }"#;
        assert!(parse_manifest(no_url.as_bytes(), "windows-x86_64").is_err());
        let bad_date = MANIFEST.replace("2026-11-23T10:00:00Z", "yesterday");
        assert!(parse_manifest(bad_date.as_bytes(), "windows-x86_64").is_err());
        let bad_version = MANIFEST.replace("3.0.5\",\n        \"notes", "three\",\n        \"notes");
        assert!(parse_manifest(bad_version.as_bytes(), "windows-x86_64").is_err());
        assert!(parse_manifest(b"<html>", "windows-x86_64").is_err());
    }

    #[test]
    fn only_a_newer_version_is_installed() {
        let v = |s: &str| semver::Version::parse(s).unwrap();
        assert_eq!(is_newer(&v("3.0.5"), "3.0.4"), Ok(true));
        assert_eq!(is_newer(&v("3.0.4"), "3.0.4"), Ok(false));
        assert_eq!(is_newer(&v("3.0.3"), "3.0.4"), Ok(false));
        // The alphas come after their beta, and the release after both
        assert_eq!(is_newer(&v("3.0.0-beta.4.alpha.1"), "3.0.0-beta.4"), Ok(true));
        assert_eq!(is_newer(&v("3.0.0"), "3.0.0-beta.4.alpha.1"), Ok(true));
    }

    #[test]
    fn the_installer_runs_as_the_plugin_ran_it() {
        assert_eq!(INSTALLER_ARGS, "/UPDATE /SILENT /NORESTART");
        assert!(is_exe(b"MZ\x90\x00"));
        assert!(!is_exe(b"PK\x03\x04"));
        assert!(!is_exe(b"M"));
    }

    #[test]
    fn the_window_reads_these_shapes() {
        assert_eq!(
            serde_json::to_value(Phase::Unavailable { reason: "portable" }).unwrap(),
            serde_json::json!({ "state": "unavailable", "reason": "portable" })
        );
        assert_eq!(
            serde_json::to_value(Phase::Downloading { version: "3.0.5".into(), downloaded: 10, total: None }).unwrap(),
            serde_json::json!({ "state": "downloading", "version": "3.0.5", "downloaded": 10, "total": null })
        );
    }

    /// Only the repository's release manifests are fetched; a test build
    /// (no uninstaller beside it) has no in-app update at all
    #[test]
    fn only_release_manifests_are_accepted() {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
        let (host, _rx) = crate::connection::tests_support::host(rt.handle().clone());
        let err = host.update_install("https://example.invalid/latest.json".into()).unwrap_err();
        assert!(err.contains("portable") || err.contains("not a manifest"), "{err}");
        assert!(!host.update_busy());
    }
}
