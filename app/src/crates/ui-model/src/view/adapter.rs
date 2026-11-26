//! The Adapter screen's rules: the order of the adapters, their names and
//! ids, the adapter to use, which firmware controls an adapter's card shows,
//! the loader line, AFH's mode and channels, LE Audio's switch, and when the
//! chip tools may run a script. The window draws them (app/src/ui:
//! screens/adapter.rs and chip_tools.rs).

use ipc::{ActiveAdapter, AdapterInfo, AdapterState, AdapterStatus, DriverKind, LoaderStatus, ScriptCheck, Settings};

use crate::chips::{
    AdapterFirmware, CHIP_NOT_REALTEK, ChipSetting, TagStatus, chip_target_index, is_auto_chip, vendor_firmware,
};
use crate::i18n::t;
use crate::t;

/// The chip setting of `s`
pub fn chip_setting(s: &Settings) -> ChipSetting<'_> {
    ChipSetting { bt_chip_pid: s.bt_chip_pid, bt_chip_fw_stem: &s.bt_chip_fw_stem }
}

/// The adapters BlueCaster can use first (WinUSB), the ones without a driver
/// and then the ones it cannot use at all last
pub fn usable_first(list: &[AdapterInfo]) -> Vec<AdapterInfo> {
    let rank = |a: &AdapterInfo| {
        (if adapter_supported(a) { 0 } else { 3 })
            + match a.driver {
                DriverKind::Winusb => 0,
                DriverKind::None => 2,
                _ => 1,
            }
    };
    let mut sorted = list.to_vec();
    // Stable, as JavaScript's sort
    sorted.sort_by_key(rank);
    sorted
}

fn hex4(n: u16) -> String {
    format!("{n:04X}")
}

/// The adapter BlueCaster has open (AdapterState::active, which the core
/// takes from this list): the listed one with its VID:PID on WinUSB
pub fn is_active_adapter(a: &AdapterInfo, active: Option<&ActiveAdapter>) -> bool {
    active.is_some_and(|active| a.vid == active.vid && a.pid == active.pid && a.driver == DriverKind::Winusb)
}

/// Settings::adapter_id names this adapter (any case)
pub fn is_chosen_adapter(a: &AdapterInfo, chosen: &str) -> bool {
    let chosen = chosen.trim();
    !chosen.is_empty() && a.instance_id.to_lowercase() == chosen.to_lowercase()
}

/// The adapter button on a card ([`adapter_use_button`])
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UseButton {
    /// Use this adapter
    Use,
    /// Back to the first one found
    Auto,
}

/// The adapter button on a card on WinUSB: "auto" on the chosen one (back
/// to the first one found), "use" on the others but the one in use while
/// none is chosen, none on the rest
pub fn adapter_use_button(a: &AdapterInfo, chosen: &str, active: Option<&ActiveAdapter>) -> Option<UseButton> {
    if a.driver != DriverKind::Winusb {
        return None;
    }
    if is_chosen_adapter(a, chosen) {
        return Some(UseButton::Auto);
    }
    if chosen.trim().is_empty() && is_active_adapter(a, active) {
        return None;
    }
    Some(UseButton::Use)
}

/// A chosen adapter that is not on WinUSB (unplugged, or its driver changed)
pub fn chosen_missing(list: &[AdapterInfo], chosen: &str) -> bool {
    !chosen.trim().is_empty() && !list.iter().any(|a| a.driver == DriverKind::Winusb && is_chosen_adapter(a, chosen))
}

/// The buttons choosing the adapter to use show: once two are on WinUSB or
/// while one is chosen
pub fn show_choice(list: &[AdapterInfo], chosen: &str) -> bool {
    list.iter().filter(|a| a.driver == DriverKind::Winusb).count() > 1 || !chosen.trim().is_empty()
}

/// `s` without the first " (0x0BDA:0xA729)" (any case)
fn without_usb_ids(s: &str) -> String {
    let b = s.as_bytes();
    let hex = |i: usize| b.get(i).is_some_and(u8::is_ascii_hexdigit);
    let is_ids = |i: usize| {
        b.len() >= i + 15
            && b[i] == b'('
            && b[i + 1] == b'0'
            && b[i + 2].eq_ignore_ascii_case(&b'x')
            && (i + 3..i + 7).all(hex)
            && b[i + 7] == b':'
            && b[i + 8] == b'0'
            && b[i + 9].eq_ignore_ascii_case(&b'x')
            && (i + 10..i + 14).all(hex)
            && b[i + 14] == b')'
    };
    match (0..b.len()).find(|&i| is_ids(i)) {
        Some(i) => format!("{}{}", s[..i].trim_end(), &s[i + 15..]),
        None => s.to_owned(),
    }
}

/// `s` without the first " [unsupported]" (any case)
fn without_unsupported(s: &str) -> String {
    const TAG: &str = "[unsupported]";
    match s.to_ascii_lowercase().find(TAG) {
        Some(i) => format!("{}{}", s[..i].trim_end(), &s[i + TAG.len()..]),
        None => s.to_owned(),
    }
}

/// "Realtek Bluetooth 5.3 Adapter (RTL8761BU)": the name Device Manager
/// gives it with the chip (issue #86); before Windows' Bluetooth ever ran
/// it and with no product name of its own, the vendor and chip ("Realtek
/// RTL8761BU"), without the VID:PID the line below shows and the
/// "[unsupported]" tag says
pub fn adapter_name(a: &AdapterInfo) -> String {
    let name = a.windows_name.as_deref().map(str::trim).unwrap_or_default();
    if name.is_empty() {
        let chip = without_unsupported(&without_usb_ids(&a.display_name)).trim().to_owned();
        if a.vendor.is_empty() || chip.to_lowercase().starts_with(&a.vendor.to_lowercase()) {
            return chip;
        }
        return format!("{} {chip}", a.vendor);
    }
    match a.realtek_chip.as_deref().filter(|c| !c.is_empty()) {
        Some(chip) if !name.to_uppercase().contains(&chip.to_uppercase()) => format!("{name} ({chip})"),
        _ => name.to_owned(),
    }
}

/// "8C:88:4B:46:DD:AE | 0BDA:A729 | v5.1": what is known of it
pub fn adapter_ids(a: &AdapterInfo) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(address) = a.address.as_deref().filter(|s| !s.is_empty()) {
        parts.push(address.to_owned());
    }
    parts.push(format!("{}:{}", hex4(a.vid), hex4(a.pid)));
    if let Some(v) = a.bt_version.as_deref().filter(|s| !s.is_empty()) {
        parts.push(format!("v{v}"));
    }
    parts.join(" | ")
}

/// Why adapter_ids lacks the address or the Bluetooth version: neither
/// Windows' Bluetooth (it runs one radio only) nor BlueCaster has
/// used the adapter yet (issue #86); None when both are known
pub fn adapter_ids_hint(a: &AdapterInfo) -> Option<String> {
    let known = |v: &Option<String>| v.as_deref().is_some_and(|s| !s.is_empty());
    if known(&a.address) && known(&a.bt_version) { None } else { Some(t("ui.adapter.ids_unknown")) }
}

/// BlueCaster can stream through it: not a chip whose firmware loader it lacks
/// (MediaTek, Qualcomm). Those need their firmware loaded at every start
/// before they answer as a Bluetooth controller, so they cannot work at all:
/// neither WinUSB nor a firmware download is offered for them (issue #86).
/// Developer mode offers WinUSB anyway, for the chip probe
pub fn adapter_supported(a: &AdapterInfo) -> bool {
    a.vendor_note.as_ref().is_none_or(|n| n.key != "firmware.vendor_unsupported")
}

/// The text of a driver ([`driver_text`]) and its colour
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverText {
    pub text: String,
    pub status: TagStatus,
}

/// The driver the adapter has now, in two names only whatever the adapter
/// and its manufacturer (issue #86): WinUSB, or the "original" one WinUSB
/// replaces (Windows' BTHUSB, or a vendor's own such as MediaTek's filter or
/// a composite device's; the service shows on hover)
pub fn driver_text(d: DriverKind) -> DriverText {
    match d {
        DriverKind::Winusb => DriverText { text: t("ui.adapter.driver_winusb"), status: TagStatus::Good },
        DriverKind::Bluetooth | DriverKind::Other => {
            DriverText { text: t("ui.adapter.driver_original"), status: TagStatus::Warn }
        }
        DriverKind::None => DriverText { text: t("ui.adapter.driver_none"), status: TagStatus::Bad },
    }
}

/// "Replace with WinUSB driver" can be chosen: one BlueCaster cannot use
/// keeps its driver (issue #86), but for the chip probe in developer mode
pub fn assign_enabled(a: &AdapterInfo, developer: bool) -> bool {
    adapter_supported(a) || developer
}

/// The hint under "Replace with WinUSB driver"
pub fn assign_hint(a: &AdapterInfo, developer: bool) -> String {
    t(if assign_enabled(a, developer) { "ui.adapter.assign_short" } else { "ui.adapter.assign_unsupported" })
}

/// How often the shown list is asked for anew: there is no Refresh button
/// (issue #86) and the core has no device notification, so an adapter
/// plugged in or out shows up by itself
pub const ADAPTER_POLL_MS: u64 = 5000;

/// The card the chip choice goes in (an index into the sorted list); none
/// for an adapter BlueCaster cannot use, the choice then stands on its own
/// below the cards
pub fn chip_card(sorted: &[AdapterInfo]) -> Option<usize> {
    let firmware: Vec<AdapterFirmware> = sorted.iter().map(AdapterFirmware::from).collect();
    chip_target_index(&firmware).filter(|&i| adapter_supported(&sorted[i]))
}

/// Another vendor's adapter: where its firmware comes from, in place of the
/// Realtek chip choice (issue #86) while that is Auto or Not Realtek
pub fn vendor_box(a: &AdapterInfo, s: &Settings) -> bool {
    vendor_firmware(&AdapterFirmware::from(a)).is_some()
        && (is_auto_chip(&chip_setting(s)) || s.bt_chip_pid == CHIP_NOT_REALTEK)
}

/// Realtek's Windows driver has the firmware (issue #86): nothing to do, as
/// for MediaTek. A file in the firmware folder still takes priority, and the
/// chip choice is back once it is set by hand or the firmware did not load.
pub fn driver_firmware(a: &AdapterInfo, s: &Settings, rom_firmware: bool) -> bool {
    a.firmware_from_driver && is_auto_chip(&chip_setting(s)) && !rom_firmware
}

/// The firmware folder's files although the driver has the firmware: the
/// card offers the way back to the driver's
pub fn custom_firmware(a: &AdapterInfo, s: &Settings) -> bool {
    a.firmware_driver && s.realtek_custom_firmware
}

/// The chip the first Realtek adapter has
pub fn detected_chip(adapters: &[AdapterInfo]) -> Option<&str> {
    adapters.iter().find_map(|a| a.realtek_chip.as_deref())
}

/// The hint under the chip choice: none while Auto has found a chip
pub fn chip_hint(s: &Settings, adapters: &[AdapterInfo]) -> Option<String> {
    if is_auto_chip(&chip_setting(s)) && detected_chip(adapters).is_some() {
        None
    } else {
        Some(t("firmware.select_chip"))
    }
}

/// The colour of a status line's dot and text
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Good,
    Warn,
    Bad,
    Idle,
}

/// The colour of the stack's state (`st-<status>`)
pub fn status_tone(status: AdapterStatus) -> Tone {
    match status {
        AdapterStatus::Ready => Tone::Good,
        AdapterStatus::Initializing => Tone::Warn,
        AdapterStatus::Error => Tone::Bad,
        AdapterStatus::Stopped => Tone::Idle,
    }
}

/// "Bluetooth stack: Ready": the stack's state
pub fn status_text(status: AdapterStatus) -> String {
    let key = match status {
        AdapterStatus::Stopped => "ui.adapter.status_stopped",
        AdapterStatus::Initializing => "ui.adapter.status_initializing",
        AdapterStatus::Ready => "ui.adapter.status_ready",
        AdapterStatus::Error => "ui.adapter.status_error",
    };
    crate::i18n::labelled(&t("ui.adapter.stack"), &t(key))
}

/// A firmware loader's line ([`loader_line`])
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoaderLine {
    pub tone: Tone,
    pub text: String,
    /// The step, in developer mode only
    pub step: Option<String>,
}

/// A firmware loader while the adapter starts (Intel's download), and how it
/// ended: "ready" and its steps in developer mode only
pub fn loader_line(loader: Option<&LoaderStatus>, developer: bool) -> Option<LoaderLine> {
    let l = loader?;
    if !developer && l.ok == Some(true) {
        return None;
    }
    let (tone, mut text) = match l.ok {
        Some(false) => (Tone::Bad, t!("ui.adapter.loader_failed", &l.vendor)),
        Some(true) => (Tone::Good, t!("ui.adapter.loader_ok", &l.vendor)),
        None => (Tone::Warn, t!("ui.adapter.loader_running", &l.vendor)),
    };
    if l.ok.is_none()
        && let Some(p) = l.percent
    {
        text.push_str(&format!(" {p}%"));
    }
    let step = (developer && !l.step.is_empty()).then(|| l.step.clone());
    Some(LoaderLine { tone, text, step })
}

/// "6,11": Wi-Fi channels 1..14, at least one
pub fn parse_afh_channels(s: &str) -> Option<String> {
    let parts: Vec<&str> = s.split(|c: char| c.is_whitespace() || c == ',').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return None;
    }
    let mut nums: Vec<u8> = Vec::new();
    for p in parts {
        // Number(): "6.0" and "0x6" are whole numbers too
        let n: f64 = if let Some(hex) = p.strip_prefix("0x").or_else(|| p.strip_prefix("0X")) {
            u32::from_str_radix(hex, 16).ok()? as f64
        } else {
            p.parse().ok()?
        };
        if n.fract() != 0. || !(1. ..=14.).contains(&n) {
            return None;
        }
        nums.push(n as u8);
    }
    nums.sort_unstable();
    nums.dedup();
    Some(nums.iter().map(u8::to_string).collect::<Vec<_>>().join(","))
}

/// LE Audio for the whole stack, as Windows has it (issue #86): only in
/// developer mode, and when an adapter is known to have LE Audio's
/// isochronous channels
pub fn le_audio_toggle_shown(adapter: &AdapterState, developer: bool) -> bool {
    developer && (adapter.le_audio == Some(true) || adapter.adapters.iter().any(|a| a.le_audio == Some(true)))
}

/// AFH's mode as the section shows it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfhMode {
    Auto,
    Off,
    Manual,
    /// Experimental, developer mode only
    Blend,
}

impl AfhMode {
    /// The value a mode's button saves (Manual saves the typed channels)
    pub fn setting(self) -> &'static str {
        match self {
            AfhMode::Auto => "auto",
            AfhMode::Off => "off",
            AfhMode::Manual => "manual",
            AfhMode::Blend => "blend-medium",
        }
    }
}

/// "blend-mild" / "-medium" / "-strong" -> "mild", ...
pub fn afh_strictness(afh: &str) -> Option<&str> {
    afh.strip_prefix("blend-").filter(|s| !s.is_empty())
}

/// "blend-*" is experimental, developer mode only; without it the stack runs
/// Auto, and so it shows
pub fn afh_mode(afh: &str, developer: bool) -> AfhMode {
    match afh {
        "auto" => AfhMode::Auto,
        "off" => AfhMode::Off,
        _ if afh_strictness(afh).is_some() => {
            if developer {
                AfhMode::Blend
            } else {
                AfhMode::Auto
            }
        }
        _ => AfhMode::Manual,
    }
}

/// Typed channels that are no list of Wi-Fi channels (an empty box is none)
pub fn afh_channels_bad(manual: bool, channels: &str) -> bool {
    manual && !channels.trim().is_empty() && parse_afh_channels(channels).is_none()
}

/// The Wi-Fi channels the running stack avoids now, from its last link
/// sample ("6, 11" -> "6,11"; empty while none or no stack runs)
pub fn afh_avoided(last_sample: Option<&str>) -> String {
    last_sample.map(|s| s.split_whitespace().collect::<String>()).unwrap_or_default()
}

/// One box for the channels (issue #86): Auto shows what it chose, Off
/// nothing, both read only; Manual takes the user's
pub fn afh_shown(manual: bool, channels: &str, mode: AfhMode, avoided: &str) -> String {
    if manual {
        channels.to_owned()
    } else if matches!(mode, AfhMode::Auto | AfhMode::Blend) {
        avoided.to_owned()
    } else {
        String::new()
    }
}

/// Chip tools: the checked script can run: no error, no line the vendor
/// refuses, and lines that may change the chip only when allowed
pub fn script_runnable(check: Option<&ScriptCheck>, allow_writes: bool) -> bool {
    check.is_some_and(|c| c.errors.is_empty() && c.denied.is_empty() && (c.writes.is_empty() || allow_writes))
}

#[cfg(test)]
mod tests {
    use ipc::Msg;

    use super::*;

    fn adapter(display_name: &str, vendor: &str) -> AdapterInfo {
        AdapterInfo {
            instance_id: "USB\\VID_0BDA&PID_A729\\1".into(),
            vid: 0x0bda,
            pid: 0xa729,
            display_name: display_name.into(),
            device_name: String::new(),
            vendor: vendor.into(),
            driver: DriverKind::Winusb,
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

    fn note(key: &str) -> Option<Msg> {
        Some(Msg { key: key.into(), args: vec![] })
    }

    fn winusb(instance_id: &str, vid: u16, pid: u16) -> AdapterInfo {
        AdapterInfo { instance_id: instance_id.into(), vid, pid, ..adapter("x", "") }
    }

    fn active(vid: u16, pid: u16) -> ActiveAdapter {
        ActiveAdapter {
            display_name: String::new(),
            windows_name: None,
            vid,
            pid,
            vendor: "Intel".into(),
            chip: None,
            bt_version: None,
        }
    }

    // ---- adapter names ----

    #[test]
    fn adapter_names() {
        let a =
            AdapterInfo { realtek_chip: Some("RTL8761BU".into()), ..adapter("RTL8761BU (0x0BDA:0xA729)", "Realtek") };
        // Windows' Bluetooth never ran it: vendor and chip
        assert_eq!(adapter_name(&a), "Realtek RTL8761BU");
        let mtk = AdapterInfo { realtek_chip: None, ..adapter("MediaTek (0x0E8D:0x7925) [unsupported]", "MediaTek") };
        assert_eq!(adapter_name(&mtk), "MediaTek");
        let named = AdapterInfo { windows_name: Some("Realtek Bluetooth 5.3 Adapter".into()), ..a.clone() };
        assert_eq!(adapter_name(&named), "Realtek Bluetooth 5.3 Adapter (RTL8761BU)");
        // The chip once only
        let named = AdapterInfo { windows_name: Some("Realtek RTL8761BU Bluetooth".into()), ..a.clone() };
        assert_eq!(adapter_name(&named), "Realtek RTL8761BU Bluetooth");
        assert_eq!(adapter_ids(&a), "0BDA:A729");
        // A Realtek adapter's USB serial gives the address before any stack ran it
        let addressed = AdapterInfo { address: Some("00:E0:4C:23:99:87".into()), ..a.clone() };
        assert_eq!(adapter_ids(&addressed), "00:E0:4C:23:99:87 | 0BDA:A729");
        assert_eq!(adapter_ids_hint(&addressed).as_deref(), Some("Shown once this adapter has been used."));
        let known = AdapterInfo { bt_version: Some("5.1".into()), ..addressed };
        assert_eq!(adapter_ids_hint(&known), None);
        assert_eq!(adapter_ids(&known), "00:E0:4C:23:99:87 | 0BDA:A729 | v5.1");
        // The driver in two names only, whatever the manufacturer
        assert_eq!(driver_text(DriverKind::Bluetooth).text, "Original driver");
        assert_eq!(driver_text(DriverKind::Other).text, "Original driver");
        assert_eq!(driver_text(DriverKind::Winusb).text, "WinUSB driver");
        assert_eq!(driver_text(DriverKind::None).text, "No driver");
        // An Intel adapter on WinUSB without a product string
        let intel = AdapterInfo { realtek_chip: None, ..adapter("Intel (0x8087:0x0033)", "Intel") };
        assert_eq!(adapter_name(&intel), "Intel");
    }

    // ---- the adapter to use ----

    #[test]
    fn marks_the_adapter_in_use_and_offers_the_ones_on_winusb() {
        let rtl = winusb("USB\\VID_2357&PID_0604\\1", 0x2357, 0x0604);
        let intel = winusb("USB\\VID_8087&PID_0033\\2", 0x8087, 0x0033);
        let on = active(0x8087, 0x0033);
        let active = Some(&on);
        assert!(is_active_adapter(&intel, active));
        assert!(!is_active_adapter(&rtl, active));
        assert!(!is_active_adapter(&intel, None));
        // The same VID:PID with Windows' driver is not the one open
        assert!(!is_active_adapter(&AdapterInfo { driver: DriverKind::Bluetooth, ..intel.clone() }, active));
        assert!(is_chosen_adapter(&intel, "usb\\vid_8087&pid_0033\\2"));
        assert!(!is_chosen_adapter(&intel, ""));
        // Nothing chosen: "use" on all on WinUSB but the one in use
        assert_eq!(adapter_use_button(&intel, "", active), None);
        assert_eq!(adapter_use_button(&rtl, "", active), Some(UseButton::Use));
        assert_eq!(adapter_use_button(&AdapterInfo { driver: DriverKind::Bluetooth, ..rtl.clone() }, "", active), None);
        // One chosen: "auto" on it, "use" on the others
        assert_eq!(adapter_use_button(&rtl, &rtl.instance_id, active), Some(UseButton::Auto));
        assert_eq!(adapter_use_button(&intel, &rtl.instance_id, active), Some(UseButton::Use));
        // A chosen adapter that is not on WinUSB is told, with a way back
        assert!(chosen_missing(std::slice::from_ref(&rtl), &intel.instance_id));
        assert!(!chosen_missing(&[rtl.clone(), intel.clone()], &intel.instance_id));
        assert!(!chosen_missing(std::slice::from_ref(&rtl), ""));
    }

    #[test]
    fn offers_the_choice_with_two_on_winusb_or_one_chosen() {
        // "is not offered with one adapter"
        let rtl = winusb("A", 1, 2);
        assert!(!show_choice(std::slice::from_ref(&rtl), ""));
        assert!(show_choice(std::slice::from_ref(&rtl), "A"));
        assert!(show_choice(&[rtl.clone(), winusb("B", 3, 4)], ""));
        let windows = AdapterInfo { driver: DriverKind::Bluetooth, ..winusb("B", 3, 4) };
        assert!(!show_choice(&[rtl, windows], " "));
    }

    // ---- adapter firmware (the card's rules) ----

    fn mediatek() -> AdapterInfo {
        AdapterInfo {
            driver: DriverKind::Other,
            driver_service: "mtkbtfilterx".into(),
            vendor_note: note("firmware.vendor_unsupported"),
            ..adapter("MediaTek (0x0E8D:0x7925) [unsupported]", "MediaTek")
        }
    }

    fn realtek() -> AdapterInfo {
        AdapterInfo {
            driver: DriverKind::Bluetooth,
            realtek_chip: Some("RTL8761BU".into()),
            firmware_present: Some(true),
            ..adapter("RTL8761BU (0x2357:0x0604)", "Realtek")
        }
    }

    fn intel() -> AdapterInfo {
        AdapterInfo { vendor_note: note("firmware.vendor_intel"), ..adapter("Intel (0x8087:0x0033)", "Intel") }
    }

    #[test]
    fn sorts_the_usable_adapters_first() {
        let none = AdapterInfo { driver: DriverKind::None, ..adapter("n", "") };
        let sorted = usable_first(&[mediatek(), none.clone(), realtek(), intel()]);
        let names: Vec<&str> = sorted.iter().map(|a| a.display_name.as_str()).collect();
        assert_eq!(
            names,
            ["Intel (0x8087:0x0033)", "RTL8761BU (0x2357:0x0604)", "n", "MediaTek (0x0E8D:0x7925) [unsupported]"]
        );
    }

    #[test]
    fn keeps_the_driver_of_an_adapter_bluecaster_cannot_use_but_in_developer_mode() {
        let mtk = mediatek();
        assert!(!adapter_supported(&mtk));
        assert!(!assign_enabled(&mtk, false));
        assert_eq!(assign_hint(&mtk, false), "BlueCaster cannot use this adapter, so its driver is left unchanged");
        // Developer mode: WinUSB for the chip probe
        assert!(assign_enabled(&mtk, true));
        assert!(assign_hint(&mtk, true).starts_with("Replaces the original driver"));
        assert!(assign_hint(&realtek(), false).starts_with("Replaces the original driver"));
    }

    #[test]
    fn puts_the_chip_choice_with_the_realtek_adapter_only() {
        let sorted = usable_first(&[mediatek(), realtek(), intel()]);
        let i = chip_card(&sorted).unwrap();
        assert_eq!(sorted[i].vendor, "Realtek");
        // Only an adapter BlueCaster cannot use: the choice on its own
        assert_eq!(chip_card(&[mediatek()]), None);
        assert_eq!(chip_card(&[]), None);
    }

    fn settings() -> Settings {
        serde_json::from_value(serde_json::json!({
            "language": "en", "theme": "system", "start_with_windows": false, "start_minimized": false,
            "minimize_to_tray": false, "check_updates_on_startup": true, "include_prereleases": false,
            "media_keys": true, "developer_mode": false, "bt_chip_pid": 0, "bt_chip_fw_stem": "",
            "afh": "auto", "remote_sink": ""
        }))
        .unwrap()
    }

    #[test]
    fn shows_another_vendors_firmware_in_place_of_the_chip_choice() {
        let mut s = settings();
        assert!(vendor_box(&intel(), &s));
        assert!(!vendor_box(&realtek(), &s));
        s.bt_chip_pid = CHIP_NOT_REALTEK;
        assert!(vendor_box(&intel(), &s));
        // A chip set by hand: the choice
        s.bt_chip_pid = 0x8771;
        s.bt_chip_fw_stem = "rtl8761bu".into();
        assert!(!vendor_box(&intel(), &s));
    }

    #[test]
    fn loads_realteks_driver_firmware_first_custom_firmware_on_request_and_back() {
        let rtl = AdapterInfo { firmware_from_driver: true, firmware_driver: true, ..realtek() };
        let mut s = settings();
        // Nothing to do: no chip choice
        assert!(driver_firmware(&rtl, &s, false));
        assert!(!custom_firmware(&rtl, &s));
        // On the ROM code: the choice is back
        assert!(!driver_firmware(&rtl, &s, true));
        // The firmware folder's files as before, and the way back
        s.realtek_custom_firmware = true;
        let custom = AdapterInfo { firmware_from_driver: false, ..rtl };
        assert!(!driver_firmware(&custom, &s, false));
        assert!(custom_firmware(&custom, &s));
    }

    #[test]
    fn hints_at_the_chip_until_auto_finds_one() {
        let s = settings();
        assert_eq!(chip_hint(&s, &[realtek()]), None);
        assert_eq!(chip_hint(&s, &[intel()]).as_deref(), Some(t("firmware.select_chip").as_str()));
        assert_eq!(detected_chip(&[intel(), realtek()]), Some("RTL8761BU"));
    }

    // ---- firmware loader progress ----

    #[test]
    fn shows_a_loader_while_the_adapter_starts_its_failure_and_its_steps_in_developer_mode() {
        let running = LoaderStatus { vendor: "Intel".into(), step: "Secure Send".into(), percent: Some(42), ok: None };
        let line = loader_line(Some(&running), false).unwrap();
        assert_eq!(line.text, "Intel firmware: loading 42%");
        assert_eq!(line.step, None);
        assert_eq!(line.tone, Tone::Warn);
        // Ready: only in developer mode, with the step
        let ready = LoaderStatus {
            vendor: "Intel".into(),
            step: "firmware ibt-0040-0041.sfi loaded".into(),
            percent: None,
            ok: Some(true),
        };
        assert_eq!(loader_line(Some(&ready), false), None);
        let line = loader_line(Some(&ready), true).unwrap();
        assert!(line.text.starts_with("Intel firmware: ready"));
        assert_eq!(line.step.as_deref(), Some("firmware ibt-0040-0041.sfi loaded"));
        let failed = LoaderStatus {
            vendor: "Intel".into(),
            step: "no answer from the controller (Read Version)".into(),
            percent: None,
            ok: Some(false),
        };
        let line = loader_line(Some(&failed), true).unwrap();
        assert!(line.text.starts_with("Intel firmware could not be loaded"));
        assert_eq!(line.tone, Tone::Bad);
        assert_eq!(loader_line(None, true), None);
    }

    // ---- the LE Audio switch ----

    #[test]
    fn le_audio_switch_in_developer_mode_when_an_adapter_supports_it() {
        let mut a =
            AdapterState { adapters: vec![AdapterInfo { le_audio: Some(true), ..realtek() }], ..Default::default() };
        assert!(le_audio_toggle_shown(&a, true));
        assert!(!le_audio_toggle_shown(&a, false));
        a.adapters[0].le_audio = Some(false);
        assert!(!le_audio_toggle_shown(&a, true));
        a.le_audio = Some(true);
        assert!(le_audio_toggle_shown(&a, true));
    }

    // ---- AFH ----

    #[test]
    fn afh_auto_and_off_read_only_manual_typed() {
        // Auto: what it avoids now
        let avoided = afh_avoided(Some(" 6, 11 "));
        assert_eq!(avoided, "6,11");
        assert_eq!(afh_shown(false, "", afh_mode("auto", false), &avoided), "6,11");
        assert_eq!(afh_avoided(None), "");
        // Off: nothing
        assert_eq!(afh_shown(false, "1", afh_mode("off", false), &avoided), "");
        // Manual: typed and saved sorted, once each
        assert_eq!(afh_mode("1,6", false), AfhMode::Manual);
        assert_eq!(afh_shown(true, "11, 6", AfhMode::Manual, &avoided), "11, 6");
        assert_eq!(parse_afh_channels("11, 6").as_deref(), Some("6,11"));
        assert_eq!(parse_afh_channels("6 6,1").as_deref(), Some("1,6"));
        assert_eq!(parse_afh_channels(""), None);
        assert_eq!(parse_afh_channels("0"), None);
        assert_eq!(parse_afh_channels("15"), None);
        assert_eq!(parse_afh_channels("6.5"), None);
        assert_eq!(parse_afh_channels("six"), None);
        assert!(afh_channels_bad(true, "6,x"));
        assert!(!afh_channels_bad(true, " "));
        assert!(!afh_channels_bad(false, "6,x"));
        // Blend: developer mode only, else it runs (and shows) Auto
        assert_eq!(afh_mode("blend-strong", true), AfhMode::Blend);
        assert_eq!(afh_mode("blend-strong", false), AfhMode::Auto);
        assert_eq!(afh_strictness("blend-strong"), Some("strong"));
        assert_eq!(afh_mode("blend-", true), AfhMode::Manual);
        assert_eq!(AfhMode::Blend.setting(), "blend-medium");
    }

    // ---- ChipTools ----

    #[test]
    fn runs_a_checked_script_and_one_that_may_change_the_chip_only_when_allowed() {
        let ok = ScriptCheck { errors: vec![], writes: vec![], denied: vec![] };
        assert!(script_runnable(Some(&ok), false));
        assert!(!script_runnable(None, true));
        let writes = ScriptCheck { writes: vec!["line 2: hci fc05 ff".into()], ..ok.clone() };
        assert!(!script_runnable(Some(&writes), false));
        assert!(script_runnable(Some(&writes), true));
        let denied = ScriptCheck { denied: vec!["line 1".into()], ..ok.clone() };
        assert!(!script_runnable(Some(&denied), true));
        let errors = ScriptCheck { errors: vec!["line 1".into()], ..ok };
        assert!(!script_runnable(Some(&errors), true));
    }
}
