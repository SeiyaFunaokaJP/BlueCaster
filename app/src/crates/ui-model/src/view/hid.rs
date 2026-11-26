//! Keyboards and mice through BlueCaster (experimental, developer mode,
//! issue #413): when it is in force, which scanned devices are offered,
//! and when the dialog that turns it on lets the user go on.

use ipc::{Device, HidDevice, Settings, Snapshot};

use crate::i18n::t;

/// How long the risks stay on screen before Turn on can be pressed
pub const HID_WAIT_SECONDS: u32 = 10;

/// The risks the dialog lists (language keys)
pub const RISKS: [&str; 4] =
    ["ui.dev.hid_risk_close", "ui.dev.hid_risk_signin", "ui.dev.hid_risk_admin", "ui.dev.hid_risk_games"];
/// Through the service they also work on the sign-in screen and as administrator
pub const RISKS_SERVICE: [&str; 3] = ["ui.dev.hid_risk_service", "ui.dev.hid_risk_sas", "ui.dev.hid_risk_games"];

/// The risks for a core that runs as a service or not
pub fn risks(service: bool) -> &'static [&'static str] {
    if service { &RISKS_SERVICE } else { &RISKS }
}

/// In force: developer mode on and the feature turned on
pub fn hid_on(s: Option<&Settings>) -> bool {
    s.is_some_and(|s| s.developer_mode && s.hid_host)
}

/// The scanned devices that may be keyboards or mice, not added yet:
/// classic ones that are not audio devices, then the Bluetooth LE keyboards
/// and mice. A device found both ways is listed once, as the classic one.
pub fn hid_candidates(devices: &[Device], le: &[Device], added: &[HidDevice]) -> Vec<Device> {
    let taken: Vec<String> = added.iter().map(|d| d.address.to_uppercase()).collect();
    let is_taken = |a: &str| taken.iter().any(|x| *x == a.to_uppercase());
    let classic: Vec<Device> = devices
        .iter()
        .filter(|d| !d.audio && !d.le_audio && d.hid_le.is_none() && !d.saved && !is_taken(&d.address))
        .cloned()
        .collect();
    let listed: Vec<String> = classic.iter().map(|d| d.address.to_uppercase()).collect();
    let le_only = le
        .iter()
        .filter(|d| d.hid_le.is_some() && !is_taken(&d.address) && !listed.contains(&d.address.to_uppercase()))
        .cloned();
    classic.into_iter().chain(le_only).collect()
}

/// The self-test of the path to Windows failed (at a start: a warning only)
pub fn hid_test_failed(snap: Option<&Snapshot>) -> bool {
    snap.and_then(|s| s.hid_health.as_ref()).is_some_and(|h| h.state == "failed")
}

/// What the user does with the new device: from its report descriptor
pub fn hid_check_hint(kind: &str) -> String {
    match kind {
        "keyboard" => t("ui.dev.hid_check_keyboard"),
        "mouse" => t("ui.dev.hid_check_mouse"),
        _ => t("ui.dev.hid_check_any"),
    }
}

/// The name a passkey or check shows: the device's, else its address
pub fn passkey_name<'a>(address: &'a str, added: &'a [HidDevice]) -> &'a str {
    let a = address.to_uppercase();
    added
        .iter()
        .find(|x| x.address.to_uppercase() == a)
        .map(|d| d.name.as_str())
        .filter(|n| !n.is_empty())
        .unwrap_or(address)
}

/// `name || address`
pub fn name_or<'a>(name: &'a str, address: &'a str) -> &'a str {
    if name.is_empty() { address } else { name }
}

/// Seconds the enable dialog still waits, `elapsed_ms` after it opened
pub fn wait_left(elapsed_ms: u64) -> u32 {
    let left = f64::from(HID_WAIT_SECONDS) - elapsed_ms as f64 / 1000.0;
    left.ceil().max(0.0) as u32
}

/// Seconds left of a check's countdown: `remaining_ms` as the core sent it,
/// `elapsed_ms` since it arrived
pub fn check_left(remaining_ms: u32, elapsed_ms: u64) -> u64 {
    (u64::from(remaining_ms).saturating_sub(elapsed_ms)).div_ceil(1000)
}

/// The digits of the typed code
pub fn digits(typed: &str) -> String {
    typed.chars().filter(char::is_ascii_digit).collect()
}

/// What the enable dialog has from the user
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnableInputs<'a> {
    /// "Another keyboard and mouse are at hand" ticked
    pub other_input: bool,
    /// "I understand the risk" ticked
    pub risk: bool,
    pub typed: &'a str,
    /// Seconds of the wait left
    pub left: u32,
    /// The core handed out a code
    pub has_code: bool,
}

/// Turn on can be pressed: both statements, six digits typed, the wait
/// over and a code from the core
pub fn enable_ready(i: &EnableInputs) -> bool {
    i.other_input && i.risk && digits(i.typed).len() == 6 && i.left == 0 && i.has_code
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(address: &str, audio: bool, le_audio: bool, saved: bool, hid_le: Option<u8>) -> Device {
        serde_json::from_value(serde_json::json!({
            "address": address, "name": "", "saved": saved, "audio": audio, "rssi": null,
            "codecs": [], "sbc": null, "codec_caps": [], "codecs_on_request": [],
            "le_audio": le_audio, "le_audio_set": false, "hid_le": hid_le, "set_addresses": []
        }))
        .unwrap()
    }

    fn added(address: &str, connected: bool, le: bool) -> HidDevice {
        HidDevice { address: address.into(), name: "K".into(), connected, le }
    }

    #[test]
    fn offers_classic_devices_not_audio_not_added() {
        let list = [
            dev("A", false, false, false, None),
            dev("B", true, false, false, None),
            dev("C", false, true, false, None),
            dev("D", false, false, true, None),
            dev("E", false, false, false, None),
        ];
        let got = hid_candidates(&list, &[], &[added("e", false, false)]);
        assert_eq!(got.iter().map(|d| d.address.as_str()).collect::<Vec<_>>(), ["A"]);
    }

    #[test]
    fn adds_le_ones_once_per_device() {
        let classic = [dev("A", false, false, false, None), dev("B", false, false, false, None)];
        // B found both ways: listed as the classic one; D already added
        let le = [
            dev("b", false, false, false, Some(1)),
            dev("C", false, false, false, Some(0)),
            dev("D", false, false, false, Some(1)),
        ];
        let got = hid_candidates(&classic, &le, &[added("D", true, true)]);
        assert_eq!(
            got.iter().map(|d| (d.address.as_str(), d.hid_le)).collect::<Vec<_>>(),
            [("A", None), ("B", None), ("C", Some(0))]
        );
    }

    #[test]
    fn check_hints() {
        assert_eq!(hid_check_hint("keyboard"), t("ui.dev.hid_check_keyboard"));
        assert_eq!(hid_check_hint("mouse"), t("ui.dev.hid_check_mouse"));
        assert_eq!(hid_check_hint("both"), t("ui.dev.hid_check_any"));
        assert_eq!(hid_check_hint(""), t("ui.dev.hid_check_any"));
    }

    #[test]
    fn enable_waits_for_everything() {
        // hidTest: "waits for both statements, the code and the countdown"
        let all = EnableInputs { other_input: true, risk: true, typed: "123456", left: 0, has_code: true };
        assert!(enable_ready(&all));
        assert!(!enable_ready(&EnableInputs { left: 1, ..all }));
        // "stays disabled with a box unticked or the code short"
        assert!(!enable_ready(&EnableInputs { risk: false, ..all }));
        assert!(!enable_ready(&EnableInputs { other_input: false, ..all }));
        assert!(!enable_ready(&EnableInputs { typed: "12345", ..all }));
        assert!(!enable_ready(&EnableInputs { typed: "", ..all }));
        // No code from the core (yet, or it failed)
        assert!(!enable_ready(&EnableInputs { has_code: false, ..all }));
        // Only the digits count
        assert!(enable_ready(&EnableInputs { typed: "12 34 56", ..all }));
        assert!(!enable_ready(&EnableInputs { typed: "12a45", ..all }));
    }

    #[test]
    fn countdowns() {
        assert_eq!(wait_left(0), HID_WAIT_SECONDS);
        assert_eq!(wait_left(9_000), 1);
        assert_eq!(wait_left(9_999), 1);
        assert_eq!(wait_left(10_000), 0);
        assert_eq!(wait_left(60_000), 0);
        // hidTest: 30 s, 5 s later 25
        assert_eq!(check_left(30_000, 0), 30);
        assert_eq!(check_left(30_000, 5_000), 25);
        assert_eq!(check_left(30_000, 4_999), 26);
        assert_eq!(check_left(1_000, 9_000), 0);
    }

    #[test]
    fn risks_by_mode() {
        // hidTest: the app's risks without the service, the service's with it
        assert_eq!(risks(false), RISKS);
        assert_eq!(risks(true), RISKS_SERVICE);
        assert!(!risks(true).contains(&"ui.dev.hid_risk_signin"));
    }

    #[test]
    fn in_force_only_in_developer_mode() {
        let mut s: Settings = serde_json::from_value(serde_json::json!({
            "language": "en", "theme": "system", "start_with_windows": false, "start_minimized": false,
            "minimize_to_tray": false, "check_updates_on_startup": true, "include_prereleases": false,
            "media_keys": true, "developer_mode": false, "bt_chip_pid": 0, "bt_chip_fw_stem": "",
            "afh": "auto", "remote_sink": ""
        }))
        .unwrap();
        s.hid_host = true;
        assert!(!hid_on(Some(&s)));
        s.developer_mode = true;
        assert!(hid_on(Some(&s)));
        assert!(!hid_on(None));
    }

    #[test]
    fn names() {
        let list = [HidDevice {
            address: "AA:BB:CC:DD:EE:01".into(),
            name: "Desk keyboard".into(),
            connected: false,
            le: false,
        }];
        assert_eq!(passkey_name("aa:bb:cc:dd:ee:01", &list), "Desk keyboard");
        assert_eq!(passkey_name("AA:BB:CC:DD:EE:02", &list), "AA:BB:CC:DD:EE:02");
        assert_eq!(name_or("", "X"), "X");
    }
}
