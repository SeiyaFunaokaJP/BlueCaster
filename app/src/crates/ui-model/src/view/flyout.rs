//! The tray flyout's rules: which parts of its page show (as Home's card,
//! issue #86), the error it shows, and where an error's fix leads from it.

use ipc::{ErrorInfo, Msg, Snapshot, StreamState};

use super::stream::{FixAction, current_profile, shows_battery};
use crate::profile::audio_entries;

/// What the flyout's page shows
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FlyoutParts {
    /// No saved device: the empty card offering to add one, as Home does
    pub no_device: bool,
    /// A device is chosen: the device chooser is the title (else "none
    /// selected"), and the volume and Connect follow
    pub profile: bool,
    /// Battery | state
    pub battery: bool,
    /// As on Home: the volume while a device is in use
    pub volume: bool,
    /// Connect is shown, with "Open BlueCaster" beside it
    pub actions: bool,
    /// The capture sign: the capture source is chosen in the main window
    pub capture: bool,
    /// An update is available: the footer leads to Settings
    pub update: bool,
    /// Only when there is something to show: an update, or "Open
    /// BlueCaster" with no Connect beside it
    pub footer: bool,
}

pub fn flyout_parts(snap: &Snapshot) -> FlyoutParts {
    let no_device = audio_entries(snap).is_empty();
    let profile = current_profile(snap).is_some();
    let s = &snap.stream;
    let actions = !no_device && profile;
    let update = snap.update.available.is_some();
    FlyoutParts {
        no_device,
        profile,
        battery: s.state != StreamState::Idle && shows_battery(s),
        volume: actions && s.state != StreamState::Idle,
        actions,
        capture: snap.capture.is_some(),
        update,
        footer: update || !actions,
    }
}

/// The error the flyout shows: a failed request of its own, else the
/// stream's error
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlyoutError {
    pub code: Option<String>,
    pub message: Msg,
}

pub fn flyout_error(request: Option<&ErrorInfo>, snap: &Snapshot) -> Option<FlyoutError> {
    if let Some(e) = request {
        return Some(FlyoutError { code: Some(e.code.clone()), message: e.message.clone() });
    }
    (snap.stream.state == StreamState::Error).then(|| FlyoutError { code: None, message: snap.stream.message.clone() })
}

/// Where a fix runs from the flyout
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixTarget {
    /// A screen of the main window (a route, "adapter/firmware"): the main
    /// window opens there
    Main(&'static str),
    /// Right here (a request, the WinUSB dialog, a web page)
    Here,
}

/// In the flyout a fix that needs a screen opens the main window there
pub fn fix_target(code: Option<&str>, key: &str, fix: &FixAction) -> FixTarget {
    if key.starts_with("error.codec") || key.starts_with("error.app") || key.starts_with("error.no_") {
        return FixTarget::Main("profiles");
    }
    if code == Some("adapter.driver_missing") || key == "error.btstack_init" {
        return FixTarget::Main("adapter");
    }
    match fix {
        FixAction::Adapter => FixTarget::Main("adapter"),
        FixAction::AdapterFirmware => FixTarget::Main("adapter/firmware"),
        FixAction::Codecs => FixTarget::Main("codecs"),
        FixAction::Devices => FixTarget::Main("devices"),
        FixAction::Diagnostics => FixTarget::Main("diagnostics"),
        FixAction::AssignWinusb(_) | FixAction::FirmwarePage(_) | FixAction::RestartAdapter | FixAction::Retry(_) => {
            FixTarget::Here
        }
    }
}

#[cfg(test)]
mod tests {
    use ipc::{AdapterState, CaptureState, Device, Settings, StreamStatus, UpdateInfo, UpdateState};

    use super::*;
    use crate::sbc::mock::{sample_device, sample_profile};

    fn settings() -> Settings {
        serde_json::from_value(serde_json::json!({
            "language": "en", "theme": "system", "start_with_windows": false, "start_minimized": false,
            "minimize_to_tray": false, "check_updates_on_startup": true, "include_prereleases": false,
            "media_keys": true, "developer_mode": false, "bt_chip_pid": 0, "bt_chip_fw_stem": "",
            "afh": "auto", "remote_sink": "", "setup_done": true, "cables_dialog_shown": true
        }))
        .unwrap()
    }

    /// One saved device, its profile active, idle
    fn snapshot() -> Snapshot {
        let profile = sample_profile();
        let device = Device { name: "WH-1000XM5".into(), saved: true, ..sample_device(&profile.device_address) };
        let mut snap: Snapshot = serde_json::from_value(serde_json::json!({
            "adapter": AdapterState::default(), "devices": [], "scanning": false,
            "stream": StreamStatus::default(), "profiles": [], "active_profile": null,
            "settings": settings(), "update": UpdateState::default(), "config_dir": ""
        }))
        .unwrap();
        snap.devices = vec![device];
        snap.stream.message = Msg::new("status.idle");
        snap.active_profile = Some(profile.name.clone());
        snap.profiles = vec![profile];
        snap
    }

    // The flyout: Open BlueCaster beside Connect, not in
    // a row of its own (issue #86)
    #[test]
    fn open_goes_beside_connect_with_no_footer() {
        let parts = flyout_parts(&snapshot());
        assert!(parts.actions && parts.profile && !parts.no_device);
        assert!(!parts.footer);
        // Idle: no volume, no battery
        assert!(!parts.volume && !parts.battery);
    }

    // The flyout offers to add one, as Home does (issue #86)
    #[test]
    fn with_no_device_it_offers_to_add_one_and_open_keeps_its_own_row() {
        let mut snap = snapshot();
        snap.devices.clear();
        let parts = flyout_parts(&snap);
        assert!(parts.no_device);
        assert!(!parts.actions);
        assert!(parts.footer);
    }

    #[test]
    fn with_no_device_chosen_open_keeps_its_own_row() {
        let mut snap = snapshot();
        snap.active_profile = None;
        snap.profiles.clear();
        let parts = flyout_parts(&snap);
        assert!(!parts.profile && !parts.actions && parts.footer);
    }

    // The flyout looks like Home's card: the volume once
    // connected, then Disconnect
    #[test]
    fn volume_and_battery_while_a_device_is_in_use() {
        let mut snap = snapshot();
        snap.stream.state = StreamState::Streaming;
        snap.stream.profile = snap.active_profile.clone();
        assert!(flyout_parts(&snap).volume);
        assert!(!flyout_parts(&snap).battery);
        snap.stream.battery = Some(80);
        assert!(flyout_parts(&snap).battery);
        snap.stream.battery = None;
        snap.stream.battery_refused = true;
        assert!(flyout_parts(&snap).battery);
    }

    #[test]
    fn an_update_shows_the_footer() {
        let mut snap = snapshot();
        snap.update.available = Some(UpdateInfo {
            version: "3.0.5".into(),
            url: String::new(),
            prerelease: false,
            manifest_url: String::new(),
            notes: String::new(),
        });
        let parts = flyout_parts(&snap);
        assert!(parts.update && parts.footer && parts.actions);
    }

    // The capture sign shows (the capture source is chosen in the main
    // window)
    #[test]
    fn the_capture_sign_shows_while_a_capture_runs() {
        let mut snap = snapshot();
        assert!(!flyout_parts(&snap).capture);
        snap.capture = Some(CaptureState { path: "x.pklg".into(), bytes: 0, started_ms: 0 });
        assert!(flyout_parts(&snap).capture);
    }

    #[test]
    fn a_failed_request_comes_before_the_streams_error() {
        let mut snap = snapshot();
        assert_eq!(flyout_error(None, &snap), None);
        snap.stream.state = StreamState::Error;
        snap.stream.message = Msg::new("error.connect_no_answer");
        assert_eq!(
            flyout_error(None, &snap),
            Some(FlyoutError { code: None, message: Msg::new("error.connect_no_answer") })
        );
        let request = ErrorInfo { code: "adapter.driver_missing".into(), message: Msg::new("error.no_winusb") };
        assert_eq!(
            flyout_error(Some(&request), &snap),
            Some(FlyoutError { code: Some("adapter.driver_missing".into()), message: Msg::new("error.no_winusb") })
        );
    }

    #[test]
    fn fixes_that_are_a_screen_open_the_main_window_there() {
        let codecs = FixAction::Codecs;
        assert_eq!(fix_target(None, "error.codec_not_supported", &codecs), FixTarget::Main("profiles"));
        assert_eq!(fix_target(None, "error.no_app_selected", &FixAction::Devices), FixTarget::Main("profiles"));
        assert_eq!(fix_target(None, "error.app_capture_init", &FixAction::Devices), FixTarget::Main("profiles"));
        assert_eq!(
            fix_target(Some("adapter.driver_missing"), "x", &FixAction::RestartAdapter),
            FixTarget::Main("adapter")
        );
        assert_eq!(fix_target(None, "error.btstack_init", &FixAction::Adapter), FixTarget::Main("adapter"));
        assert_eq!(
            fix_target(None, "firmware.rom_running", &FixAction::AdapterFirmware),
            FixTarget::Main("adapter/firmware")
        );
        assert_eq!(fix_target(None, "error.other", &FixAction::Diagnostics), FixTarget::Main("diagnostics"));
        // Requests run right here
        assert_eq!(fix_target(None, "error.connect_no_answer", &FixAction::Retry("p".into())), FixTarget::Here);
        assert_eq!(fix_target(None, "error.btstack_no_answer", &FixAction::RestartAdapter), FixTarget::Here);
    }
}
