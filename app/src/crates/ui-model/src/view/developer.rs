//! The Developer screen's rules: what counts as changed, how its number and
//! text fields read what is typed, and the vendors' overrides (issue #237).

use std::collections::BTreeMap;

use ipc::{DevDebug, DevLog, DevOverrides, KeyCounts, Settings, VendorMode, VendorOverride, WindowsValueName};

use crate::codecs::{PACKET_MAX, PACKET_MIN};

/// Developer > Codec settings with nothing overridden
pub fn no_overrides() -> DevOverrides {
    DevOverrides::default()
}

/// Developer > Debug: experiments for one adapter's transport (issue #86)
pub fn no_debug() -> DevDebug {
    DevDebug::default()
}

/// Developer > Logging as it comes
pub fn default_log() -> DevLog {
    DevLog::default()
}

/// The microphone presets Developer > Debug offers (le-audio presets.rs)
pub const MIC_PRESETS: [&str; 16] = [
    "16_1_1", "16_2_1", "24_1_1", "24_2_1", "32_1_1", "32_2_1", "48_1_1", "48_2_1", "16_2_2", "24_2_2", "32_2_2",
    "48_2_2", "32_1_gs", "32_2_gs", "48_1_gs", "48_2_gs",
];

/// When vendors' protocols are used, in the order the select lists them
pub const VENDOR_MODES: [VendorMode; 3] = [VendorMode::Auto, VendorMode::Always, VendorMode::Never];

/// The language key of a vendor mode's label
pub fn vendor_mode_key(m: VendorMode) -> &'static str {
    match m {
        VendorMode::Auto => "ui.dev.vendor_mode.auto",
        VendorMode::Always => "ui.dev.vendor_mode.always",
        VendorMode::Never => "ui.dev.vendor_mode.never",
    }
}

/// Whether vendor overrides are on (defaults left in the map count as off)
pub fn vendors_active(value: &BTreeMap<String, VendorOverride>) -> bool {
    value.values().any(|o| o.mode != VendorMode::Auto || !o.options.is_empty())
}

/// A vendor's override (Auto, no options while it has none)
pub fn vendor_of(value: &BTreeMap<String, VendorOverride>, id: &str) -> VendorOverride {
    value.get(id).cloned().unwrap_or_default()
}

/// The overrides with one vendor's mode changed
pub fn vendor_with_mode(
    value: &BTreeMap<String, VendorOverride>,
    id: &str,
    mode: VendorMode,
) -> BTreeMap<String, VendorOverride> {
    let mut next = value.clone();
    next.insert(id.to_owned(), VendorOverride { mode, ..vendor_of(value, id) });
    next
}

/// The overrides with one vendor's option turned on or off
pub fn vendor_with_option(
    value: &BTreeMap<String, VendorOverride>,
    id: &str,
    option: &str,
    on: bool,
) -> BTreeMap<String, VendorOverride> {
    let mut o = vendor_of(value, id);
    if on {
        o.options.push(option.to_owned());
    } else {
        o.options.retain(|x| x != option);
    }
    let mut next = value.clone();
    next.insert(id.to_owned(), o);
    next
}

/// Every setting of this screen at its default (developer mode stays on)
pub fn all_default(s: &Settings) -> bool {
    !s.volume_mixer
        && !s.all_audio_devices
        && !s.vb_cable_note_hidden
        && !s.hifi_cable_note_hidden
        && s.dev_overrides == no_overrides()
        && !vendors_active(&s.dev_vendors)
        && s.dev_log == default_log()
        && s.dev_debug == no_debug()
}

/// "Reset all": every setting of this screen back to its default
pub fn reset_all(s: &mut Settings) {
    s.volume_mixer = false;
    s.all_audio_devices = false;
    s.vb_cable_note_hidden = false;
    s.hifi_cable_note_hidden = false;
    s.dev_overrides = no_overrides();
    s.dev_vendors = BTreeMap::new();
    s.dev_log = default_log();
    s.dev_debug = no_debug();
}

/// A field's text as JavaScript's `Number(text)`: empty or blank is 0,
/// anything that is not a number is NaN
fn js_number(text: &str) -> f64 {
    let s = text.trim();
    if s.is_empty() {
        return 0.0;
    }
    // Rust reads "inf" and "nan" too; JavaScript only "Infinity"
    if s.chars().any(|c| c.is_ascii_alphabetic() && c != 'e' && c != 'E') {
        return f64::NAN;
    }
    s.parse::<f64>().unwrap_or(f64::NAN)
}

/// `Math.round`: halves go up
fn js_round(n: f64) -> f64 {
    (n + 0.5).floor()
}

/// A whole number in min..=max from a field; the fallback when it is not one
pub fn clamp_int(text: &str, min: i64, max: i64, fallback: i64) -> i64 {
    let n = js_round(js_number(text));
    if !text.trim().is_empty() && n.is_finite() { (n as i64).clamp(min, max) } else { fallback }
}

/// A positive number clamped to min..=max; 0 (the default) for empty, 0 or
/// not a number
fn positive_clamped(text: &str, min: u32, max: u32) -> u32 {
    let n = js_round(js_number(text));
    if n.is_finite() && n > 0.0 { (n.min(f64::from(max)) as u32).max(min) } else { 0 }
}

/// The AAC bit rate field when it is left: 32..=512 kbps, 0 = Auto
pub fn commit_bitrate(text: &str) -> u32 {
    positive_clamped(text, 32, 512)
}

/// The send queue limit too; empty = the default (50..=2000 ms)
pub fn commit_queue_wait(text: &str) -> u32 {
    positive_clamped(text, 50, 2000)
}

/// The MediaTek padding field (0..=4096, 0 = the default)
pub fn commit_pad(text: &str) -> u16 {
    clamp_int(text, 0, 4096, 0) as u16
}

/// The MediaTek ISO queue field (0..=12, 0 = the default)
pub fn commit_iso_queue(text: &str) -> u8 {
    clamp_int(text, 0, 12, 0) as u8
}

/// An HCI command or register write as typed: trimmed, upper case
pub fn hex_text(text: &str) -> String {
    text.trim().to_uppercase()
}

/// One of the LE Audio numbers: empty keeps what BlueCaster plans (None),
/// else a whole number in 0..=max
pub fn le_number(text: &str, max: i64) -> Option<i64> {
    if text.trim().is_empty() { None } else { Some(clamp_int(text, 0, max, 0)) }
}

/// A number field's text for a value where 0 is the default: empty then
/// (`value || ""`)
pub fn zero_empty(n: u64) -> String {
    if n == 0 { String::new() } else { n.to_string() }
}

/// Logging: daily files kept, 1..=365 (the current value when not a number)
pub fn commit_keep_days(text: &str, current: u32) -> u32 {
    clamp_int(text, 1, 365, i64::from(current)) as u32
}

/// Logging: the daily size cap, 10..=10240 MB
pub fn commit_day_cap(text: &str, current: u32) -> u32 {
    clamp_int(text, 10, 10240, i64::from(current)) as u32
}

/// A device's max media packet size as typed: a whole number in range
pub fn packet_valid(text: &str) -> bool {
    let n = js_number(text);
    n.fract() == 0.0 && n >= f64::from(PACKET_MIN) && n <= f64::from(PACKET_MAX)
}

/// The value a packet size field saves when it is left: None when what is
/// typed is not valid or is the saved value
pub fn packet_commit(text: Option<&str>, saved: u16) -> Option<u16> {
    let s = text?;
    if !packet_valid(s) {
        return None;
    }
    let n = js_number(s) as u16;
    (n != saved).then_some(n)
}

/// The arguments of ui.dev.keys_link / keys_le
pub fn key_count_args(c: &KeyCounts) -> [u32; 4] {
    [c.same, c.differ, c.only_bluecaster, c.only_windows]
}

/// What Windows keeps: names, types and sizes, never the data
pub fn value_names(list: &[WindowsValueName]) -> String {
    list.iter()
        .map(|v| {
            let count = if v.count > 1 { format!(", ×{}", v.count) } else { String::new() };
            format!("{} ({}, {} B{count})", v.name, v.kind, v.size)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        serde_json::from_value(serde_json::json!({
            "language": "en", "theme": "system", "start_with_windows": false, "start_minimized": false,
            "minimize_to_tray": false, "check_updates_on_startup": true, "include_prereleases": false,
            "media_keys": true, "developer_mode": true, "bt_chip_pid": 0, "bt_chip_fw_stem": "",
            "afh": "auto", "remote_sink": ""
        }))
        .unwrap()
    }

    #[test]
    fn vendors_by_id() {
        // Developer.test: "sets each vendor's protocol and options by vendor id (issue #237)"
        let none = BTreeMap::new();
        assert!(!vendors_active(&none));
        let v = vendor_with_mode(&none, "oppo", VendorMode::Always);
        assert_eq!(
            v,
            BTreeMap::from([("oppo".to_owned(), VendorOverride { mode: VendorMode::Always, options: vec![] })])
        );
        assert!(vendors_active(&v));
        let v = vendor_with_option(&v, "oppo", "every_connection", true);
        assert_eq!(v["oppo"].options, vec!["every_connection".to_owned()]);
        assert_eq!(v["oppo"].mode, VendorMode::Always);
        let v = vendor_with_option(&v, "oppo", "every_connection", false);
        let v = vendor_with_mode(&v, "oppo", VendorMode::Auto);
        // Defaults left in the map count as off
        assert!(!vendors_active(&v));
        assert_eq!(vendor_of(&none, "huawei"), VendorOverride::default());
    }

    #[test]
    fn debug_fields() {
        // Developer.test: "keeps the MediaTek experiments on the Debug tab"
        assert_eq!(commit_pad("99999"), 4096);
        assert_eq!(commit_pad(""), 0);
        assert_eq!(commit_pad("abc"), 0);
        assert_eq!(hex_text(" fc00 01 "), "FC00 01");
        assert_eq!(hex_text(" 40 01 0000abcd 00000001 "), "40 01 0000ABCD 00000001");
        assert_eq!(commit_iso_queue("20"), 12);
        // LE Audio: empty fields are Auto, RTN 0 is a value of its own
        assert_eq!(le_number("100", 4000), Some(100));
        assert_eq!(le_number("0", 15), Some(0));
        assert_eq!(le_number("", 15), None);
        assert_eq!(le_number("  ", 15), None);
        assert_eq!(le_number("99", 15), Some(15));
        assert_eq!(MIC_PRESETS.len(), 16);
    }

    #[test]
    fn overrides_fields() {
        // Developer.test: "saves the overrides and resets them from the app settings"
        assert_eq!(commit_bitrate("9999"), 512);
        assert_eq!(commit_bitrate("10"), 32);
        assert_eq!(commit_bitrate(""), 0);
        assert_eq!(commit_bitrate("-5"), 0);
        assert_eq!(commit_bitrate("x"), 0);
        assert_eq!(commit_queue_wait("5"), 50);
        assert_eq!(commit_queue_wait(""), 0);
        assert_eq!(commit_queue_wait("3000"), 2000);
        assert_eq!(commit_queue_wait("125.5"), 126);
        assert_eq!(zero_empty(0), "");
        assert_eq!(zero_empty(512), "512");
    }

    #[test]
    fn logging_fields() {
        // Developer.test: "saves the logging settings in range"
        assert_eq!(commit_keep_days("1000", 7), 365);
        assert_eq!(commit_keep_days("0", 7), 1);
        assert_eq!(commit_keep_days("", 7), 7);
        assert_eq!(commit_keep_days("x", 9), 9);
        assert_eq!(commit_day_cap("5", 100), 10);
        assert_eq!(commit_day_cap("99999", 100), 10240);
    }

    #[test]
    fn reset_all_clears_every_tab() {
        let mut s = settings();
        assert!(all_default(&s));
        s.dev_overrides.rtp_marker = true;
        s.dev_debug.mtk_iso_bulk = true;
        s.dev_debug.le_rtn_p_to_c = Some(0);
        s.dev_log.keep_days = 365;
        s.volume_mixer = true;
        s.dev_vendors = vendor_with_mode(&BTreeMap::new(), "oppo", VendorMode::Never);
        assert!(!all_default(&s));
        reset_all(&mut s);
        assert!(all_default(&s));
        assert_eq!(s.dev_debug, no_debug());
        assert!(s.developer_mode);
        // A vendor left at its defaults counts as default
        s.dev_vendors = vendor_with_mode(&BTreeMap::new(), "oppo", VendorMode::Auto);
        assert!(all_default(&s));
        // Alphas are not this screen's reset
        s.include_alphas = true;
        assert!(all_default(&s));
    }

    #[test]
    fn packet_sizes() {
        assert!(packet_valid("1023"));
        assert!(!packet_valid("678"));
        assert!(!packet_valid("1680"));
        assert!(!packet_valid("1000.5"));
        assert!(!packet_valid(""));
        assert_eq!(packet_commit(Some("800"), 1023), Some(800));
        assert_eq!(packet_commit(Some("1023"), 1023), None);
        assert_eq!(packet_commit(Some("1"), 1023), None);
        assert_eq!(packet_commit(None, 1023), None);
    }

    #[test]
    fn windows_value_names() {
        // Developer.test: names, types and sizes, never the data
        let v = |name: &str, kind: &str, size, count| WindowsValueName {
            name: name.into(),
            kind: kind.into(),
            size,
            count,
        };
        assert_eq!(
            value_names(&[v("LTK", "binary", 16, 1), v("ERand", "QWORD", 8, 1)]),
            "LTK (binary, 16 B), ERand (QWORD, 8 B)"
        );
        assert_eq!(value_names(&[v("IRK", "binary", 16, 3)]), "IRK (binary, 16 B, ×3)");
        assert_eq!(
            key_count_args(&KeyCounts { same: 1, differ: 0, only_bluecaster: 2, only_windows: 3 }),
            [1, 0, 2, 3]
        );
    }
}
