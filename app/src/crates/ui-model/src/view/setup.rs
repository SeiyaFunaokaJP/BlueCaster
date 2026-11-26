//! The first-run setup's rules: when it is needed, its steps, the adapter it
//! chooses and their order, when Next may be pressed, what the driver step
//! does, and when the device step's scan resumes. The window draws them
//! (app/src/ui: screens/setup.rs).

use ipc::{AdapterInfo, DriverKind, Profile, Snapshot, StreamState, StreamStatus, WindowsPairedDevice};

use crate::i18n::t;
use crate::profile::{AudioEntry, audio_entries};
use crate::view::adapter::{adapter_supported, usable_first};
use crate::view::stream::status_text;

/// No profile yet, or no adapter with the WinUSB driver
pub fn needs_setup(profiles: &[Profile], adapters: &[AdapterInfo]) -> bool {
    profiles.is_empty() || !adapters.iter().any(|a| a.driver == DriverKind::Winusb)
}

pub const STEP_KEYS: [&str; 4] =
    ["ui.setup.step_adapter", "ui.setup.step_driver", "ui.setup.step_device", "ui.setup.step_general"];
/// Each step's heading; the last two have headings of their own below it
pub const HEADING_KEYS: [&str; 4] =
    ["ui.setup.adapter_intro", "ui.setup.selected_adapter", "ui.setup.step_device", "ui.setup.step_general"];
pub const DEVICE_STEP: usize = 2;
pub const LAST: usize = STEP_KEYS.len() - 1;

/// The step's heading is shown (the device and last steps have headings of
/// their own; theirs is for screen readers only)
pub fn heading_shown(step: usize) -> bool {
    step < DEVICE_STEP
}

/// Usable first (the Adapters screen's order), the unsupported ones last;
/// with the same driver a Realtek chip goes before the others, so that a
/// PC's built-in adapter (often Intel) is not the one chosen by default
pub fn supported_first(list: &[AdapterInfo]) -> Vec<AdapterInfo> {
    let usable = usable_first(list);
    // Stable sort: within usable_first's driver rank, Realtek first, else as listed
    let rank = |a: &AdapterInfo| match a.driver {
        DriverKind::Winusb => 0,
        DriverKind::None => 2,
        _ => 1,
    };
    let mut supported: Vec<AdapterInfo> = usable.iter().filter(|a| adapter_supported(a)).cloned().collect();
    supported.sort_by_key(|a| (rank(a), a.realtek_chip.is_none()));
    supported.extend(usable.into_iter().filter(|a| !adapter_supported(a)));
    supported
}

/// The adapter the setup works with: the one chosen, else a supported one on
/// WinUSB, else a supported one, else the first listed
pub fn chosen_adapter(list: &[AdapterInfo], chosen: Option<&str>) -> Option<AdapterInfo> {
    let list = supported_first(list);
    list.iter()
        .find(|a| Some(a.instance_id.as_str()) == chosen)
        .or_else(|| list.iter().find(|a| a.driver == DriverKind::Winusb && adapter_supported(a)))
        .or_else(|| list.iter().find(|a| adapter_supported(a)))
        .or(list.first())
        .cloned()
}

/// Next can be pressed. Another BlueCaster (v1, or another v2) holds the
/// adapter: the banner asks to exit it first, the setup waits (Skip stays)
pub fn can_next(step: usize, v1_running: bool, adapter: Option<&AdapterInfo>) -> bool {
    if v1_running {
        return false;
    }
    match step {
        0 => adapter.is_some(),
        1 => adapter.is_some_and(|a| a.driver == DriverKind::Winusb),
        // Audio devices can be added later (issue #86)
        _ => true,
    }
}

/// "Start with Windows" and "Start in the tray" are on by default (issue
/// #86): turned on once when a first setup reaches them, unless both are on
/// already
pub fn turns_startup_on(first: bool, start_with_windows: bool, start_minimized: bool) -> bool {
    first && !(start_with_windows && start_minimized)
}

/// What the driver step's one button does for an adapter
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DriverPlan {
    /// It switches the driver to WinUSB (an administrator prompt)
    pub needs_driver: bool,
    pub needs_firmware: bool,
    /// Firmware the user has to fetch: the button does the driver only
    pub manual_firmware: bool,
    /// The button has something to do
    pub todo: bool,
}

pub fn driver_plan(a: &AdapterInfo) -> DriverPlan {
    let needs_driver = a.driver != DriverKind::Winusb;
    let needs_firmware = a.firmware_present == Some(false);
    DriverPlan {
        needs_driver,
        needs_firmware,
        manual_firmware: needs_firmware && !a.firmware_download,
        todo: needs_driver || (needs_firmware && a.firmware_download),
    }
}

/// The adapter as the list after the driver change has it
pub fn after_assign(list: &[AdapterInfo], a: &AdapterInfo) -> AdapterInfo {
    list.iter().find(|x| x.instance_id == a.instance_id).unwrap_or(a).clone()
}

/// The firmware file the button downloads for the adapter, if any
pub fn firmware_to_download(a: &AdapterInfo) -> Option<String> {
    (a.firmware_present == Some(false) && a.firmware_download).then(|| a.firmware_files.first().cloned()).flatten()
}

/// Devices Windows paired through the adapter that go with it: not the
/// audio ones (the setup's import and pairing take them over, issue #86)
pub fn losing_devices(list: Vec<WindowsPairedDevice>) -> Vec<WindowsPairedDevice> {
    list.into_iter().filter(|d| !d.audio).collect()
}

/// The classic saved devices: what the setup finds and connects
pub fn classic_entries(snap: &Snapshot) -> Vec<AudioEntry> {
    audio_entries(snap).into_iter().filter(|e| !e.le_audio).collect()
}

/// A stream runs or is on its way: no scan part, and "Add a device" shows
pub fn stream_running(state: StreamState) -> bool {
    matches!(state, StreamState::Connecting | StreamState::Reconnecting | StreamState::Streaming)
}

/// No scan from a Connect until the stream it starts has ended; after a
/// failure none by itself either: a scan would clear the error on the row,
/// and the core refuses one until the session has cleaned up. True when the
/// scan resumes as the running state changed.
pub fn scan_resumes(was_running: bool, running: bool, state: StreamState) -> bool {
    was_running != running && !running && state != StreamState::Error
}

/// A saved device's state as its dot says it
pub fn paired_label(state: StreamState, stream: &StreamStatus) -> String {
    if state == StreamState::Idle {
        t("status.ready")
    } else {
        status_text(state, &stream.message, stream.codec_stop.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use ipc::Msg;

    use super::*;

    fn adapter(id: &str) -> AdapterInfo {
        AdapterInfo {
            instance_id: id.into(),
            vid: 0x2357,
            pid: 0x0604,
            display_name: String::new(),
            device_name: String::new(),
            vendor: String::new(),
            driver: DriverKind::Bluetooth,
            driver_service: String::new(),
            realtek_chip: None,
            firmware_present: None,
            firmware_files: vec![],
            firmware_invalid: None,
            firmware_download: false,
            firmware_from_driver: false,
            firmware_driver: false,
            vendor_note: None,
            restart_needed: false,
            le_audio: None,
            address: None,
            bt_version: None,
            windows_name: None,
        }
    }

    fn intel() -> AdapterInfo {
        AdapterInfo { vendor: "Intel".into(), ..adapter("intel") }
    }

    fn rtl() -> AdapterInfo {
        AdapterInfo { vendor: "Realtek".into(), realtek_chip: Some("RTL8761BU".into()), ..adapter("rtl") }
    }

    fn mediatek() -> AdapterInfo {
        AdapterInfo {
            vendor_note: Some(Msg { key: "firmware.vendor_unsupported".into(), args: vec![] }),
            ..adapter("mtk")
        }
    }

    fn ids(list: &[AdapterInfo]) -> Vec<&str> {
        list.iter().map(|a| a.instance_id.as_str()).collect()
    }

    fn winusb(a: AdapterInfo) -> AdapterInfo {
        AdapterInfo { driver: DriverKind::Winusb, ..a }
    }

    /// The mock's first run: the MediaTek card and the TP-Link dongle on
    /// Windows' driver, its firmware missing
    fn fresh() -> Vec<AdapterInfo> {
        let dongle = AdapterInfo {
            firmware_present: Some(false),
            firmware_files: vec!["rtl8761bu_fw.bin".into(), "rtl8761bu_config.bin".into()],
            firmware_download: true,
            ..rtl()
        };
        vec![AdapterInfo { driver: DriverKind::Other, ..mediatek() }, dongle]
    }

    // ---- the setup's adapter order ----

    #[test]
    fn puts_a_realtek_dongle_before_the_built_in_intel_adapter_on_the_same_driver() {
        assert_eq!(ids(&supported_first(&[intel(), mediatek(), rtl()])), ["rtl", "intel", "mtk"]);
    }

    #[test]
    fn keeps_an_adapter_already_on_winusb_first() {
        assert_eq!(ids(&supported_first(&[rtl(), winusb(intel())])), ["intel", "rtl"]);
        assert_eq!(ids(&supported_first(&[winusb(intel()), winusb(rtl())])), ["rtl", "intel"]);
    }

    // ---- the first-run setup when no adapter is on WinUSB ----

    #[test]
    fn is_needed_without_a_profile_or_an_adapter_on_winusb() {
        let profile = crate::profile::new_profile();
        assert!(needs_setup(&[], &fresh()));
        assert!(needs_setup(std::slice::from_ref(&profile), &fresh()));
        assert!(needs_setup(&[], &[winusb(rtl())]));
        assert!(!needs_setup(std::slice::from_ref(&profile), &[winusb(rtl())]));
    }

    #[test]
    fn waits_while_another_bluecaster_holds_the_adapter() {
        let chosen = chosen_adapter(&fresh(), None);
        assert!(can_next(0, false, chosen.as_ref()));
        assert!(!can_next(0, true, chosen.as_ref()));
        // No adapter: nothing to go on with
        assert!(!can_next(0, false, None));
        // The driver step: only once it is on WinUSB
        assert!(!can_next(1, false, chosen.as_ref()));
        assert!(can_next(1, false, Some(&winusb(rtl()))));
        assert!(!can_next(1, true, Some(&winusb(rtl()))));
        // Audio devices can be added later
        assert!(can_next(DEVICE_STEP, false, None));
        assert!(!can_next(DEVICE_STEP, true, None));
    }

    #[test]
    fn one_button_switches_the_driver_and_downloads_the_firmware() {
        let a = chosen_adapter(&fresh(), None).unwrap();
        let plan = driver_plan(&a);
        assert_eq!(plan, DriverPlan { needs_driver: true, needs_firmware: true, manual_firmware: false, todo: true });
        // After the driver change, the firmware the list then names
        let assigned = winusb(a.clone());
        let now = after_assign(std::slice::from_ref(&assigned), &a);
        assert_eq!(now.driver, DriverKind::Winusb);
        assert_eq!(firmware_to_download(&now).as_deref(), Some("rtl8761bu_fw.bin"));
        assert_eq!(after_assign(&[], &a).driver, DriverKind::Bluetooth);
        // Ready: nothing left to do
        let ready = AdapterInfo { firmware_present: Some(true), ..assigned };
        assert!(!driver_plan(&ready).todo);
        assert_eq!(firmware_to_download(&ready), None);
        // Firmware BlueCaster cannot fetch: the manual way
        let manual = AdapterInfo { firmware_download: false, ..winusb(a) };
        let plan = driver_plan(&manual);
        assert!(plan.manual_firmware && !plan.todo);
        assert_eq!(firmware_to_download(&manual), None);
    }

    // ---- the setup's adapter step ----

    #[test]
    fn chooses_the_supported_adapter_and_lists_the_other_last() {
        let list = supported_first(&fresh());
        assert_eq!(ids(&list), ["rtl", "mtk"]);
        assert!(adapter_supported(&list[0]) && !adapter_supported(&list[1]));
        assert_eq!(chosen_adapter(&fresh(), None).unwrap().instance_id, "rtl");
        // One chosen wins; an unknown one falls back
        let two = [rtl(), winusb(intel())];
        assert_eq!(chosen_adapter(&two, None).unwrap().instance_id, "intel");
        assert_eq!(chosen_adapter(&two, Some("rtl")).unwrap().instance_id, "rtl");
        assert_eq!(chosen_adapter(&two, Some("gone")).unwrap().instance_id, "intel");
        // Only unsupported ones: listed, the first one shown
        assert_eq!(chosen_adapter(&[mediatek()], None).unwrap().instance_id, "mtk");
        assert_eq!(chosen_adapter(&[], None), None);
    }

    // ---- the setup names the devices Windows loses ----

    #[test]
    fn names_every_device_windows_loses_but_the_audio_ones() {
        let d = |name: &str, audio: bool| WindowsPairedDevice { address: String::new(), name: name.into(), audio };
        let list = vec![
            d("Logitech K380", false),
            d("MX Master 3", false),
            d("", true),
            d("WH-1000XM4", true),
            d("Pixel 8", false),
        ];
        let names: Vec<String> = losing_devices(list).into_iter().map(|d| d.name).collect();
        assert_eq!(names, ["Logitech K380", "MX Master 3", "Pixel 8"]);
        // A list that cannot be read asks nothing
        assert!(losing_devices(Vec::new()).is_empty());
    }

    // ---- the setup's device step ----

    #[test]
    fn four_steps_with_their_titles() {
        let titles: Vec<String> = STEP_KEYS.iter().map(|k| t(k)).collect();
        assert_eq!(
            titles,
            ["Choose the adapter", "Driver and firmware", "Find your audio devices", "General Settings"]
        );
        assert_eq!(LAST, 3);
        assert!(heading_shown(0) && heading_shown(1) && !heading_shown(DEVICE_STEP) && !heading_shown(LAST));
    }

    #[test]
    fn starts_with_windows_in_the_tray_by_default_on_a_first_setup() {
        assert!(turns_startup_on(true, false, false));
        // Only Start with Windows on: the tray too
        assert!(turns_startup_on(true, true, false));
        assert!(!turns_startup_on(true, true, true));
        // Not a first setup: left as they are
        assert!(!turns_startup_on(false, false, false));
    }

    #[test]
    fn keeps_a_failed_connections_error_and_starts_no_scan_by_itself() {
        use StreamState::*;
        // The stream a Connect started has ended: the scan resumes
        assert!(scan_resumes(true, false, Idle));
        // It failed: no scan by itself
        assert!(!scan_resumes(true, false, Error));
        // Still on its way, or nothing changed
        assert!(!scan_resumes(false, true, Connecting));
        assert!(!scan_resumes(false, false, Idle));
        assert!(stream_running(Reconnecting) && stream_running(Streaming) && !stream_running(Error));
    }

    #[test]
    fn a_saved_device_reads_ready_until_it_connects() {
        let stream = StreamStatus {
            state: StreamState::Streaming,
            message: Msg { key: "status.connected".into(), args: vec![] },
            ..Default::default()
        };
        assert_eq!(paired_label(StreamState::Idle, &stream), t("status.ready"));
        assert_eq!(paired_label(StreamState::Streaming, &stream), "Connected");
    }
}
