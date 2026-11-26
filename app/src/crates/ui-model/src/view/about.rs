//! The decisions behind About, the service add-on (its row on the Developer
//! screen and the connection screen's notices) and VB-Audio's cables (the
//! line, the dialog, the dialog once after an update).

use ipc::{AudioDevice, Settings};

use crate::i18n::t;
use crate::links::third_party_url;
use crate::profile::{hifi_cable, vb_cable};
use crate::t;

// ---- About -----------------------------------------------------------------

/// Where the licence texts are
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Licenses {
    /// The folder shipped next to the programs
    Folder(String),
    /// A development build has none: the release's copy online
    Online(String),
}

/// The licence texts shipped next to the programs; a development build has
/// none, so the release's copy online
pub fn licenses(dir: Option<String>, version: &str) -> Licenses {
    match dir {
        Some(dir) if !dir.is_empty() => Licenses::Folder(dir),
        _ => Licenses::Online(third_party_url(version)),
    }
}

// ---- The service add-on ----------------------------------------------------

/// The service's state, without the versions it carries
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceKind {
    NotAdded,
    Running,
    Stopped,
    NeedsUpdate,
    AppOlder,
}

/// How a set-up ended
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupOutcome {
    Done,
    Declined,
    /// The set-up tool's exit code
    Failed(u32),
    NotStarted,
}

/// The message for a set-up that did not end well, or None
pub fn setup_error(outcome: SetupOutcome) -> Option<String> {
    match outcome {
        SetupOutcome::Done => None,
        SetupOutcome::Declined => Some(t("error.admin_declined")),
        SetupOutcome::Failed(code) => Some(t!("ui.service.failed", code)),
        SetupOutcome::NotStarted => Some(t("ui.service.not_started")),
    }
}

/// The state's line under "Keyboards and mice on every screen"
pub fn service_state_text(kind: ServiceKind) -> &'static str {
    match kind {
        ServiceKind::NotAdded => "ui.service.state_not_added",
        ServiceKind::Running => "ui.service.state_running",
        ServiceKind::Stopped => "ui.service.state_stopped",
        ServiceKind::NeedsUpdate => "ui.service.state_needs_update",
        ServiceKind::AppOlder => "ui.service.state_app_older",
    }
}

/// The row's first button
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceButton {
    /// Asks first (ServiceAddDialog), then installs
    Add,
    /// Runs the update
    Update,
    /// Interactive users may not start a service: the update starts it,
    /// with an administrator's approval
    Start,
}

/// The row's buttons: the first one, and whether Remove is offered
pub fn service_buttons(kind: ServiceKind) -> (Option<ServiceButton>, bool) {
    let first = match kind {
        ServiceKind::NotAdded => Some(ServiceButton::Add),
        ServiceKind::NeedsUpdate => Some(ServiceButton::Update),
        ServiceKind::Stopped => Some(ServiceButton::Start),
        ServiceKind::Running | ServiceKind::AppOlder => None,
    };
    (first, !matches!(kind, ServiceKind::NotAdded | ServiceKind::AppOlder))
}

// ---- VB-Audio's cables -----------------------------------------------------

/// The warning hidden on the Developer page (developer mode only)
pub fn cable_warning_hidden(s: &Settings) -> bool {
    s.developer_mode && (s.vb_cable_note_hidden || s.hifi_cable_note_hidden)
}

/// A cable is among Windows' outputs; None until they are read
pub fn cable_installed(devices: Option<&[AudioDevice]>, hifi: bool) -> Option<bool> {
    let list = devices?;
    Some(if hifi { hifi_cable(list).is_some() } else { vb_cable(list).is_some() })
}

/// The line on Audio Devices and the setup: None while hidden (outputs not
/// read yet, or hidden in developer mode); Some(true) while a cable is
/// missing (amber), Some(false) once both are installed (green)
pub fn cable_line(devices: Option<&[AudioDevice]>, s: &Settings) -> Option<bool> {
    let list = devices?;
    if cable_warning_hidden(s) {
        return None;
    }
    Some(vb_cable(list).is_none() || hifi_cable(list).is_none())
}

/// What the dialog once after an update does when the main window opens
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CablesOnceStart {
    /// Shown already, or a first setup (which shows the line itself)
    Nothing,
    /// The warning is hidden in developer mode: nothing to show, it only
    /// counts as shown
    CountAsShown,
    /// Read Windows' outputs, then [`cables_once_opens`]
    Read,
}

pub fn cables_once_start(s: &Settings) -> CablesOnceStart {
    if s.cables_dialog_shown || !s.setup_done {
        CablesOnceStart::Nothing
    } else if cable_warning_hidden(s) {
        CablesOnceStart::CountAsShown
    } else {
        CablesOnceStart::Read
    }
}

/// Outputs read: the dialog opens while a cable is missing; both cables
/// installed only counts as shown
pub fn cables_once_opens(devices: &[AudioDevice]) -> bool {
    vb_cable(devices).is_none() || hifi_cable(devices).is_none()
}

/// A cable's button
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CableButton {
    Uninstall,
    /// VB-CABLE: the core fetches VB-Audio's pack and runs its setup
    Install,
    /// VB-Audio's page (the Hi-Fi Cable, or VB-CABLE after a failed install)
    Download,
}

/// A cable's buttons: none until Windows' outputs are read
pub fn cable_buttons(installed: Option<bool>, hifi: bool, failed: bool) -> Vec<CableButton> {
    match installed {
        None => vec![],
        Some(true) => vec![CableButton::Uninstall],
        Some(false) if hifi => vec![CableButton::Download],
        Some(false) if failed => vec![CableButton::Install, CableButton::Download],
        Some(false) => vec![CableButton::Install],
    }
}

/// How an install or uninstall ended, for the cable's row
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CableAfter {
    /// "Restart" note, and Windows' outputs read again for a while
    pub restart: bool,
    /// The error goes to the notice line
    pub notify: bool,
    /// Install failed: VB-Audio's page is offered
    pub failed: bool,
}

/// `answer`: the request's result (done), or the error's code
pub fn cable_after(answer: Result<bool, &str>, uninstall: bool) -> CableAfter {
    match answer {
        Ok(true) => CableAfter::default(),
        // Done, but Windows still lists it, or does not list it yet
        Ok(false) => CableAfter { restart: true, ..CableAfter::default() },
        // No answer in time: the setup may still be running
        Err("ipc.timeout") => CableAfter { restart: true, ..CableAfter::default() },
        Err(_) => CableAfter { restart: false, notify: true, failed: !uninstall },
    }
}

/// How long Windows' outputs are read again after an install or removal
/// that did not see the cable change: every 2 s for 30 s
pub const POLL_MS: u64 = 2000;
pub const POLLS: u32 = 15;

/// VB-Audio's site as a link shows it: "vb-cable.com"
pub fn link_text(url: &str) -> &str {
    let s = url.strip_prefix("https://").unwrap_or(url);
    s.strip_suffix('/').unwrap_or(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(name: &str, is_virtual: bool) -> AudioDevice {
        AudioDevice {
            id: name.to_owned(),
            name: name.to_owned(),
            short_name: None,
            adapter_name: None,
            is_default: false,
            is_virtual,
            is_display: false,
            mix_rate: None,
            mix_channels: None,
            mix_bits: None,
            capture_name: None,
            capture_rate: None,
        }
    }

    fn vb() -> AudioDevice {
        device("CABLE Input (VB-Audio Virtual Cable)", true)
    }

    fn hifi() -> AudioDevice {
        device("Hi-Fi Cable Input (VB-Audio Hi-Fi Cable)", true)
    }

    fn settings() -> Settings {
        serde_json::from_value(serde_json::json!({
            "language": "en", "theme": "system", "start_with_windows": false, "start_minimized": false,
            "minimize_to_tray": false, "check_updates_on_startup": true, "include_prereleases": false,
            "media_keys": true, "developer_mode": false, "bt_chip_pid": 0, "bt_chip_fw_stem": "",
            "afh": "auto", "remote_sink": "", "setup_done": true, "cables_dialog_shown": true
        }))
        .unwrap()
    }

    #[test]
    fn licenses_online_in_a_development_build() {
        assert_eq!(licenses(Some("C:\\x".into()), "3.0.4"), Licenses::Folder("C:\\x".into()));
        assert!(
            matches!(licenses(None, "3.0.4"), Licenses::Online(u) if u.ends_with("/v3.0.4/THIRD_PARTY_LICENSES.md"))
        );
    }

    // The service add-on is offered by what it gives, removed after asking,
    // and started from the connection screen
    #[test]
    fn service_row_buttons_by_state() {
        assert_eq!(service_buttons(ServiceKind::NotAdded), (Some(ServiceButton::Add), false));
        assert_eq!(service_buttons(ServiceKind::Running), (None, true));
        assert_eq!(service_buttons(ServiceKind::Stopped), (Some(ServiceButton::Start), true));
        assert_eq!(service_buttons(ServiceKind::NeedsUpdate), (Some(ServiceButton::Update), true));
        assert_eq!(service_buttons(ServiceKind::AppOlder), (None, false));
        assert_eq!(service_state_text(ServiceKind::NotAdded), "ui.service.state_not_added");
        assert_eq!(service_state_text(ServiceKind::Running), "ui.service.state_running");
    }

    // "says so when the administrator prompt is declined", "shows the set-up
    // tool's error code"
    #[test]
    fn setup_errors() {
        assert_eq!(setup_error(SetupOutcome::Done), None);
        assert_eq!(setup_error(SetupOutcome::Declined), Some(t("error.admin_declined")));
        assert_eq!(setup_error(SetupOutcome::Failed(7)), Some(t!("ui.service.failed", 7)));
        assert_eq!(setup_error(SetupOutcome::NotStarted), Some(t("ui.service.not_started")));
    }

    // Audio Devices without a virtual device: the line, and
    // the developer switch that hides it only in developer mode
    #[test]
    fn the_line_amber_green_or_hidden() {
        let s = settings();
        assert_eq!(cable_line(None, &s), None);
        assert_eq!(cable_line(Some(&[]), &s), Some(true));
        assert_eq!(cable_line(Some(&[vb()]), &s), Some(true));
        assert_eq!(cable_line(Some(&[vb(), hifi()]), &s), Some(false));
        let hidden = Settings { vb_cable_note_hidden: true, hifi_cable_note_hidden: true, ..settings() };
        assert_eq!(cable_line(Some(&[]), &hidden), Some(true));
        let dev = Settings { developer_mode: true, ..hidden };
        assert_eq!(cable_line(Some(&[]), &dev), None);
        let dev_vb = Settings { developer_mode: true, vb_cable_note_hidden: true, ..settings() };
        assert_eq!(cable_line(Some(&[vb(), hifi()]), &dev_vb), None);
    }

    // "VB-CABLE installs here, the Hi-Fi Cable is downloaded from VB-Audio",
    // "offers VB-Audio's page when VB-CABLE cannot be downloaded"
    #[test]
    fn cable_rows_buttons() {
        assert_eq!(cable_installed(None, false), None);
        assert_eq!(cable_installed(Some(&[vb()]), false), Some(true));
        assert_eq!(cable_installed(Some(&[vb()]), true), Some(false));
        assert_eq!(cable_buttons(None, false, false), vec![]);
        assert_eq!(cable_buttons(Some(false), false, false), vec![CableButton::Install]);
        assert_eq!(cable_buttons(Some(false), true, false), vec![CableButton::Download]);
        assert_eq!(cable_buttons(Some(false), false, true), vec![CableButton::Install, CableButton::Download]);
        assert_eq!(cable_buttons(Some(true), true, false), vec![CableButton::Uninstall]);
        assert_eq!(cable_buttons(Some(true), false, true), vec![CableButton::Uninstall]);
    }

    // "shows VB-CABLE installed when the setup's answer comes too late, with
    // no error or Download (issue #86)"
    #[test]
    fn install_answers() {
        assert_eq!(cable_after(Ok(true), false), CableAfter::default());
        assert_eq!(cable_after(Ok(false), true), CableAfter { restart: true, notify: false, failed: false });
        assert_eq!(cable_after(Err("ipc.timeout"), false), CableAfter { restart: true, notify: false, failed: false });
        assert_eq!(cable_after(Err("audio.failed"), false), CableAfter { restart: false, notify: true, failed: true });
        assert_eq!(cable_after(Err("audio.failed"), true), CableAfter { restart: false, notify: true, failed: false });
    }

    // "virtual audio devices after an update (issue #86)"
    #[test]
    fn the_dialog_once_after_an_update() {
        let updated = Settings { cables_dialog_shown: false, ..settings() };
        assert_eq!(cables_once_start(&updated), CablesOnceStart::Read);
        // A new install, or a first setup
        assert_eq!(cables_once_start(&settings()), CablesOnceStart::Nothing);
        assert_eq!(cables_once_start(&Settings { setup_done: false, ..updated.clone() }), CablesOnceStart::Nothing);
        let hidden = Settings { developer_mode: true, hifi_cable_note_hidden: true, ..updated };
        assert_eq!(cables_once_start(&hidden), CablesOnceStart::CountAsShown);
        // The Hi-Fi Cable is missing: it opens; both: only counted as shown
        assert!(cables_once_opens(&[vb()]));
        assert!(!cables_once_opens(&[vb(), hifi()]));
    }

    #[test]
    fn links_show_the_site() {
        assert_eq!(link_text("https://vb-cable.com/"), "vb-cable.com");
        assert_eq!(link_text("https://vb-audio.com/"), "vb-audio.com");
    }
}
