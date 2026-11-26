//! Bluetooth chips for the firmware (settings.bt_chip_pid / bt_chip_fw_stem),
//! the table of v1 (bt_adapter_enum.cpp chip_db, develop branch) and the core
//! (crates/bt-host/src/chips.rs).

use std::collections::HashSet;

use ipc::{AdapterInfo, DriverKind, FirmwareFile, LoaderStatus, Msg};

pub const CHIP_AUTO: u16 = 0;
pub const CHIP_NOT_REALTEK: u16 = 0xfffe;
pub const CHIP_CUSTOM: u16 = 0xffff;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chip {
    pub pid: u16,
    pub name: &'static str,
    /// Firmware file stem: <stem>_fw.bin, <stem>_config.bin
    pub stem: &'static str,
}

pub const REALTEK_CHIPS: [Chip; 9] = [
    Chip { pid: 0x8771, name: "RTL8761BU", stem: "rtl8761bu" },
    Chip { pid: 0xb82c, name: "RTL8822BU", stem: "rtl8822bu" },
    Chip { pid: 0xc821, name: "RTL8821CU", stem: "rtl8821cu" },
    Chip { pid: 0xc82c, name: "RTL8822CU", stem: "rtl8822cu" },
    Chip { pid: 0xd723, name: "RTL8723DU", stem: "rtl8723du" },
    Chip { pid: 0xc852, name: "RTL8852AU", stem: "rtl8852au" },
    Chip { pid: 0xb85b, name: "RTL8852BU", stem: "rtl8852bu" },
    Chip { pid: 0xc85a, name: "RTL8852CU", stem: "rtl8852cu" },
    Chip { pid: 0xb851, name: "RTL8851BU", stem: "rtl8851bu" },
];

fn table_chip(pid: u16) -> Option<&'static Chip> {
    REALTEK_CHIPS.iter().find(|c| c.pid == pid)
}

/// The settings.bt_chip_fw_stem that goes with a chip choice (as v1 stored it)
pub fn stem_for(pid: u16) -> &'static str {
    if pid == CHIP_CUSTOM {
        return "custom";
    }
    table_chip(pid).map_or("", |c| c.stem)
}

/// A chip setting as settings.bt_chip_pid / bt_chip_fw_stem hold it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChipSetting<'a> {
    pub bt_chip_pid: u16,
    pub bt_chip_fw_stem: &'a str,
}

/// An entry of the chip choice: the setting it saves and its text
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChipOption {
    /// Unique within the list (pid alone does not tell file-only chips apart)
    pub key: String,
    pub pid: u16,
    pub stem: String,
    pub label: String,
}

/// The texts of the first two entries of the chip choice
#[derive(Debug, Clone, Copy)]
pub struct ChipLabels<'a> {
    pub auto: &'a str,
    pub non_realtek: &'a str,
}

/// What [`build_chip_options`] returns
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChipOptions {
    pub options: Vec<ChipOption>,
    /// The key of the entry the setting matches
    pub selected: String,
}

fn option_key(pid: u16, stem: &str) -> String {
    format!("{pid}:{stem}")
}

/// True for the Auto setting (core chips::is_auto_chip)
pub fn is_auto_chip(s: &ChipSetting) -> bool {
    s.bt_chip_pid == CHIP_AUTO && s.bt_chip_fw_stem.is_empty()
}

/// The chip choice of v1's firmware dialog: Auto, Not Realtek, every chip of
/// the table (files there or not), the chips known only by their firmware
/// files in the settings folder (adapter.firmware_files entries without a
/// table PID), Custom, and the current setting if no entry stands for it.
/// `selected` is the key of the entry the setting matches.
pub fn build_chip_options(setting: &ChipSetting, files: &[FirmwareFile], labels: &ChipLabels) -> ChipOptions {
    let mut options: Vec<ChipOption> = Vec::new();
    let add = |options: &mut Vec<ChipOption>, pid: u16, stem: &str, label: String| {
        options.push(ChipOption { key: option_key(pid, stem), pid, stem: stem.to_owned(), label })
    };
    add(&mut options, CHIP_AUTO, "", labels.auto.to_owned());
    add(&mut options, CHIP_NOT_REALTEK, "", labels.non_realtek.to_owned());
    for c in &REALTEK_CHIPS {
        add(&mut options, c.pid, c.stem, format!("{} ({}_fw.bin)", c.name, c.stem));
    }
    let table_stems: HashSet<&str> = REALTEK_CHIPS.iter().map(|c| c.stem).collect();
    let mut seen: HashSet<String> = HashSet::new();
    for f in files {
        let lower = f.stem.to_lowercase();
        // Table chips are listed already; custom has its own entry
        if f.pid.is_some()
            || table_stems.contains(lower.as_str())
            || lower == "custom"
            || seen.contains(&lower)
            || f.stem.is_empty()
        {
            continue;
        }
        seen.insert(lower);
        add(&mut options, CHIP_AUTO, &f.stem, format!("{} ({}_fw.bin)", f.display_name, f.stem));
    }
    add(&mut options, CHIP_CUSTOM, "custom", "Custom (custom_fw.bin, custom_config.bin)".to_owned());

    // v1: a PID matches by itself, PID 0 (Auto, a file-only chip) by the stem
    let (pid, stem) = (setting.bt_chip_pid, setting.bt_chip_fw_stem);
    let stem_lower = stem.to_lowercase();
    let found = options
        .iter()
        .find(
            |o| if o.pid != CHIP_AUTO { o.pid == pid } else { pid == CHIP_AUTO && o.stem.to_lowercase() == stem_lower },
        )
        .map(|o| o.key.clone());
    let selected = match found {
        Some(key) => key,
        None => {
            // A setting no entry stands for (a chip since dropped from the table, a
            // file since removed) stays choosable as it is
            add(&mut options, pid, stem, format!("0x{pid:04X} {stem}").trim_end().to_owned());
            options[options.len() - 1].key.clone()
        }
    };
    ChipOptions { options, selected }
}

/// The firmware file stem of a chip setting, None if it names no Realtek
/// firmware (core chips::firmware_files)
pub fn firmware_stem(pid: u16, stem: &str) -> Option<String> {
    if pid == CHIP_NOT_REALTEK {
        return None;
    }
    if let Some(c) = table_chip(pid) {
        return Some(c.stem.to_owned());
    }
    if pid == CHIP_CUSTOM {
        return Some("custom".to_owned());
    }
    if stem.is_empty() { None } else { Some(stem.to_owned()) }
}

/// What the Adapter screen needs of an adapter for the firmware
/// (`From<&AdapterInfo>` takes it from the core's adapter)
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AdapterFirmware {
    pub vendor: String,
    pub realtek_chip: Option<String>,
    pub firmware_present: Option<bool>,
    pub firmware_files: Vec<String>,
    /// A file that is there but is no Realtek firmware (a saved web page)
    pub firmware_invalid: Option<String>,
    /// The core can fetch the files from linux-firmware
    pub firmware_download: bool,
    /// The firmware comes from the vendor's Windows driver (a Realtek chip)
    pub firmware_from_driver: bool,
    pub vendor_note: Option<Msg>,
    /// None: not known (counts as WinUSB for a missing file's colour)
    pub driver: Option<DriverKind>,
}

impl From<&AdapterInfo> for AdapterFirmware {
    fn from(a: &AdapterInfo) -> AdapterFirmware {
        AdapterFirmware {
            vendor: a.vendor.clone(),
            realtek_chip: a.realtek_chip.clone(),
            firmware_present: a.firmware_present,
            firmware_files: a.firmware_files.clone(),
            firmware_invalid: a.firmware_invalid.clone(),
            firmware_download: a.firmware_download,
            firmware_from_driver: a.firmware_from_driver,
            vendor_note: a.vendor_note.clone(),
            driver: Some(a.driver),
        }
    }
}

/// The firmware state of the chip setting (v1 FirmwareDialog::
/// UpdateFirmwareStatus):
/// - present / missing: the Realtek files of the chosen (or, for Auto, the
///   detected) chip, as the core checked them for the adapters
/// - invalid: one of them is there but is no Realtek firmware, usually a web
///   page saved instead of the file (v1 firmware.invalid)
/// - missing and invalid say whether the core can download the files
/// - unchecked: no adapter to check them for; from the folder scan if it
///   lists the file, unknown for Custom (the scan leaves it out)
/// - vendor: no Realtek chip, and an adapter of a vendor the core has a note
///   for (Intel, Broadcom, CSR, unsupported)
/// - non_realtek, auto_no_adapter, auto_unrecognized, select_chip: v1's texts
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChipStatus {
    Present { files: Vec<String> },
    Missing { files: Vec<String>, download: bool },
    Invalid { files: Vec<String>, file: String, download: bool },
    Unchecked { files: Vec<String>, present: Option<bool> },
    Vendor { note: Msg, vendor: String },
    NonRealtek,
    AutoNoAdapter,
    AutoUnrecognized,
    SelectChip,
}

impl ChipStatus {
    /// The status as a name: "present", "missing", ... "select_chip"
    pub fn kind(&self) -> &'static str {
        match self {
            ChipStatus::Present { .. } => "present",
            ChipStatus::Missing { .. } => "missing",
            ChipStatus::Invalid { .. } => "invalid",
            ChipStatus::Unchecked { .. } => "unchecked",
            ChipStatus::Vendor { .. } => "vendor",
            ChipStatus::NonRealtek => "non_realtek",
            ChipStatus::AutoNoAdapter => "auto_no_adapter",
            ChipStatus::AutoUnrecognized => "auto_unrecognized",
            ChipStatus::SelectChip => "select_chip",
        }
    }
}

/// See [`ChipStatus`]; `found` is the folder scan (adapter.firmware_files)
pub fn chip_status(setting: &ChipSetting, adapters: &[AdapterFirmware], found: &[FirmwareFile]) -> ChipStatus {
    let auto = is_auto_chip(setting);
    let realtek = adapters.iter().find(|a| a.realtek_chip.is_some());
    let not_realtek = setting.bt_chip_pid == CHIP_NOT_REALTEK;
    if not_realtek || (auto && realtek.is_none()) {
        // v1 tells about another vendor only when no Realtek adapter is there
        let other = if realtek.is_some() { None } else { adapters.iter().find(|a| a.vendor_note.is_some()) };
        if let Some(other) = other
            && let Some(note) = &other.vendor_note
        {
            return ChipStatus::Vendor { note: note.clone(), vendor: other.vendor.clone() };
        }
        if not_realtek {
            return ChipStatus::NonRealtek;
        }
        return if adapters.is_empty() { ChipStatus::AutoNoAdapter } else { ChipStatus::AutoUnrecognized };
    }
    // The core resolves the setting for each adapter; Auto by its own chip
    let checked = if auto { realtek } else { adapters.iter().find(|a| a.firmware_present.is_some()) };
    if let Some(checked) = checked
        && let Some(present) = checked.firmware_present
    {
        let download = checked.firmware_download;
        let files = checked.firmware_files.clone();
        if present {
            return ChipStatus::Present { files };
        }
        if let Some(file) = checked.firmware_invalid.as_ref().filter(|f| !f.is_empty()) {
            return ChipStatus::Invalid { files, file: file.clone(), download };
        }
        return ChipStatus::Missing { files, download };
    }
    let Some(stem) = firmware_stem(setting.bt_chip_pid, setting.bt_chip_fw_stem) else {
        return ChipStatus::SelectChip;
    };
    let files = vec![format!("{stem}_fw.bin"), format!("{stem}_config.bin")];
    let lower = stem.to_lowercase();
    let entry = found.iter().find(|f| f.stem.to_lowercase() == lower);
    let present = match entry {
        Some(e) => Some(e.has_config),
        None if lower == "custom" => None,
        None => Some(false),
    };
    ChipStatus::Unchecked { files, present }
}

/// Where another vendor's adapter gets its firmware (issue #86: the Realtek
/// chip choice means nothing to it): its vendor's Windows driver (MediaTek,
/// Qualcomm), the Wi-Fi driver (Marvell), files in the firmware folder
/// (Intel; Broadcom's optional), or none needed. None for Realtek, an
/// unknown chip and one BlueCaster cannot use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VendorFirmware {
    Driver,
    Wifi,
    Folder,
    Optional,
    None,
}

pub fn vendor_firmware(a: &AdapterFirmware) -> Option<VendorFirmware> {
    match a.vendor_note.as_ref().map(|n| n.key.as_str()) {
        Some("firmware.vendor_mediatek" | "firmware.vendor_qualcomm") => Some(VendorFirmware::Driver),
        Some("firmware.vendor_marvell") => Some(VendorFirmware::Wifi),
        Some("firmware.vendor_intel") => Some(VendorFirmware::Folder),
        Some("firmware.vendor_broadcom") => Some(VendorFirmware::Optional),
        Some("firmware.vendor_csr" | "firmware.vendor_barrot" | "firmware.vendor_actions") => {
            Some(VendorFirmware::None)
        }
        _ => None,
    }
}

/// How a firmware loader went ([`loader_outcome`])
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoaderOutcome {
    Ok,
    Failed,
    Running,
}

/// The running stack's firmware loader on this adapter (on WinUSB, its
/// vendor's): how it went, None when none ran on it
pub fn loader_outcome(a: &AdapterFirmware, loader: Option<&LoaderStatus>) -> Option<LoaderOutcome> {
    let loader = loader?;
    if a.driver != Some(DriverKind::Winusb) || loader.vendor.to_lowercase() != a.vendor.to_lowercase() {
        return None;
    }
    Some(match loader.ok {
        Some(true) => LoaderOutcome::Ok,
        Some(false) => LoaderOutcome::Failed,
        None => LoaderOutcome::Running,
    })
}

fn vendor_firmware_tag(v: VendorFirmware) -> &'static str {
    match v {
        VendorFirmware::Driver => "ui.adapter.firmware_from_driver",
        VendorFirmware::Wifi => "ui.adapter.firmware_from_wifi",
        VendorFirmware::Folder => "ui.adapter.firmware_from_folder",
        VendorFirmware::Optional => "ui.adapter.firmware_optional",
        VendorFirmware::None => "ui.adapter.firmware_not_needed",
    }
}

/// The colour of a [`FirmwareTag`]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagStatus {
    Good,
    Warn,
    Bad,
}

/// The adapter card's short firmware state: a language key (its argument,
/// if any) and its colour
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirmwareTag {
    pub key: &'static str,
    pub args: Option<Vec<String>>,
    pub status: TagStatus,
}

impl FirmwareTag {
    fn new(key: &'static str, status: TagStatus) -> FirmwareTag {
        FirmwareTag { key, args: None, status }
    }
}

/// See [`FirmwareTag`]
pub fn adapter_firmware_tag(a: &AdapterFirmware, setting: &ChipSetting, loader: Option<&LoaderStatus>) -> FirmwareTag {
    // Realtek's Windows driver has it (issue #86): as MediaTek's
    if a.firmware_present == Some(true) && a.firmware_from_driver {
        return FirmwareTag {
            key: "ui.adapter.firmware_from_driver",
            args: Some(vec![a.vendor.clone()]),
            status: TagStatus::Good,
        };
    }
    if a.firmware_present == Some(true) {
        return FirmwareTag::new("firmware.present", TagStatus::Good);
    }
    if a.firmware_present == Some(false) {
        // Needed once the adapter is on WinUSB: amber before (issue #86)
        let status = if matches!(a.driver, None | Some(DriverKind::Winusb)) { TagStatus::Bad } else { TagStatus::Warn };
        let invalid = a.firmware_invalid.as_ref().is_some_and(|f| !f.is_empty());
        return FirmwareTag::new(if invalid { "firmware.invalid_short" } else { "firmware.missing_short" }, status);
    }
    // BlueCaster cannot use it: red, as a missing file (issue #86)
    if a.vendor_note.as_ref().is_some_and(|n| n.key == "firmware.vendor_unsupported") {
        return FirmwareTag::new("ui.adapter.firmware_unsupported", TagStatus::Bad);
    }
    if let Some(source) = vendor_firmware(a) {
        match loader_outcome(a, loader) {
            Some(LoaderOutcome::Failed) => return FirmwareTag::new("ui.adapter.firmware_load_failed", TagStatus::Bad),
            Some(LoaderOutcome::Ok) => return FirmwareTag::new("ui.adapter.firmware_loaded", TagStatus::Good),
            _ => {}
        }
        return FirmwareTag {
            key: vendor_firmware_tag(source),
            args: (source == VendorFirmware::Driver).then(|| vec![a.vendor.clone()]),
            status: TagStatus::Good,
        };
    }
    if a.vendor_note.is_some() || setting.bt_chip_pid == CHIP_NOT_REALTEK {
        return FirmwareTag::new("ui.adapter.firmware_not_realtek", TagStatus::Good);
    }
    FirmwareTag::new("ui.adapter.firmware_choose_chip", TagStatus::Warn)
}

/// The adapter the chip choice is shown with on the Adapter screen, None for
/// none: the one chip_status reports on (the Realtek adapter, else one the core
/// checked files for, else one with a vendor note), else the first. The
/// setting is one for all adapters; the other cards keep their short tag.
pub fn chip_target_index(adapters: &[AdapterFirmware]) -> Option<usize> {
    let picks: [fn(&AdapterFirmware) -> bool; 3] =
        [|a| a.realtek_chip.is_some(), |a| a.firmware_present.is_some(), |a| a.vendor_note.is_some()];
    for pick in picks {
        if let Some(i) = adapters.iter().position(pick) {
            return Some(i);
        }
    }
    if adapters.is_empty() { None } else { Some(0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LABELS: ChipLabels = ChipLabels { auto: "Auto (RTL8761BU)", non_realtek: "Not Realtek" };

    fn setting(pid: u16, stem: &str) -> ChipSetting<'_> {
        ChipSetting { bt_chip_pid: pid, bt_chip_fw_stem: stem }
    }

    fn file(stem: &str, pid: Option<u16>, has_config: bool) -> FirmwareFile {
        FirmwareFile { stem: stem.to_owned(), display_name: stem.to_uppercase(), pid, has_config }
    }

    // ---- chip choice ----

    #[test]
    fn lists_auto_not_realtek_the_table_then_custom_as_v1() {
        let ChipOptions { options, selected } = build_chip_options(&setting(CHIP_AUTO, ""), &[], &LABELS);
        let mut want = vec!["Auto (RTL8761BU)".to_owned(), "Not Realtek".to_owned()];
        want.extend(REALTEK_CHIPS.iter().map(|c| format!("{} ({}_fw.bin)", c.name, c.stem)));
        want.push("Custom (custom_fw.bin, custom_config.bin)".to_owned());
        assert_eq!(options.iter().map(|o| o.label.clone()).collect::<Vec<_>>(), want);
        assert_eq!(selected, options[0].key);
        assert_eq!(options.iter().map(|o| &o.key).collect::<HashSet<_>>().len(), options.len());
    }

    #[test]
    fn adds_chips_known_only_by_their_firmware_files_before_custom() {
        let files = [
            file("rtl8761bu", Some(0x8771), true),
            file("rtl8723bu", None, true),
            file("RTL8723DU", None, true),
            file("custom", None, true),
            file("rtl8703bu", None, false),
        ];
        let ChipOptions { options, .. } = build_chip_options(&setting(CHIP_AUTO, ""), &files, &LABELS);
        let extra: Vec<(u16, &str, &str)> =
            options[2 + REALTEK_CHIPS.len()..].iter().map(|o| (o.pid, o.stem.as_str(), o.label.as_str())).collect();
        // Table chips (by PID or by stem) and custom are not listed twice
        assert_eq!(
            extra,
            [
                (CHIP_AUTO, "rtl8723bu", "RTL8723BU (rtl8723bu_fw.bin)"),
                (CHIP_AUTO, "rtl8703bu", "RTL8703BU (rtl8703bu_fw.bin)"),
                (CHIP_CUSTOM, "custom", "Custom (custom_fw.bin, custom_config.bin)"),
            ]
        );
    }

    #[test]
    fn selects_the_entry_the_setting_stands_for() {
        let files = [file("rtl8723bu", None, true)];
        let pick = |pid: u16, stem: &str| {
            let r = build_chip_options(&setting(pid, stem), &files, &LABELS);
            r.options.into_iter().find(|o| o.key == r.selected).unwrap()
        };
        assert_eq!(pick(CHIP_NOT_REALTEK, "").label, "Not Realtek");
        // A table PID matches by itself, whatever the stem
        assert_eq!(pick(0x8771, "rtl8761bu").label, "RTL8761BU (rtl8761bu_fw.bin)");
        assert_eq!(pick(0x8771, "").label, "RTL8761BU (rtl8761bu_fw.bin)");
        // PID 0 with a stem: the file-only chip (file names ignore case)
        let o = pick(CHIP_AUTO, "RTL8723BU");
        assert_eq!((o.pid, o.stem.as_str()), (CHIP_AUTO, "rtl8723bu"));
        assert_eq!(pick(CHIP_CUSTOM, "custom").stem, "custom");
    }

    #[test]
    fn keeps_a_setting_no_entry_stands_for_choosable() {
        let gone = build_chip_options(&setting(CHIP_AUTO, "rtl8723bu"), &[], &LABELS);
        let last = gone.options.last().unwrap();
        assert_eq!(gone.selected, last.key);
        assert_eq!((last.pid, last.stem.as_str(), last.label.as_str()), (CHIP_AUTO, "rtl8723bu", "0x0000 rtl8723bu"));

        let dropped = build_chip_options(&setting(0x1234, "old"), &[], &LABELS);
        let last = dropped.options.last().unwrap();
        assert_eq!((last.pid, last.stem.as_str(), last.label.as_str()), (0x1234, "old", "0x1234 old"));
        assert_eq!(dropped.selected, last.key);
    }

    // ---- firmware state ----

    fn rtl(present: Option<bool>) -> AdapterFirmware {
        AdapterFirmware {
            vendor: "Realtek".to_owned(),
            realtek_chip: Some("RTL8761BU".to_owned()),
            firmware_present: present,
            firmware_files: if present.is_none() {
                vec![]
            } else {
                vec!["rtl8761bu_fw.bin".to_owned(), "rtl8761bu_config.bin".to_owned()]
            },
            ..Default::default()
        }
    }

    fn other(vendor: &str, key: Option<&str>) -> AdapterFirmware {
        AdapterFirmware { vendor: vendor.to_owned(), vendor_note: key.map(Msg::new), ..Default::default() }
    }

    fn loader(vendor: &str, ok: Option<bool>) -> LoaderStatus {
        LoaderStatus { vendor: vendor.to_owned(), step: String::new(), percent: None, ok }
    }

    fn rtl_files() -> Vec<String> {
        vec!["rtl8761bu_fw.bin".to_owned(), "rtl8761bu_config.bin".to_owned()]
    }

    #[test]
    fn names_the_files_of_a_chip_setting() {
        assert_eq!(firmware_stem(0x8771, "").as_deref(), Some("rtl8761bu"));
        assert_eq!(firmware_stem(CHIP_CUSTOM, "custom").as_deref(), Some("custom"));
        assert_eq!(firmware_stem(CHIP_AUTO, "rtl8723bu").as_deref(), Some("rtl8723bu"));
        assert_eq!(firmware_stem(CHIP_AUTO, ""), None);
        assert_eq!(firmware_stem(CHIP_NOT_REALTEK, ""), None);
        assert_eq!(firmware_stem(0x1234, ""), None);
    }

    #[test]
    fn takes_present_missing_from_the_adapters_the_core_checked() {
        assert_eq!(
            chip_status(&setting(CHIP_AUTO, ""), &[rtl(Some(true))], &[]),
            ChipStatus::Present { files: rtl_files() }
        );
        assert_eq!(
            chip_status(
                &setting(0x8771, "rtl8761bu"),
                &[other("Intel", Some("firmware.vendor_intel")), rtl(Some(false))],
                &[]
            )
            .kind(),
            "missing"
        );
        // The core can fetch the files: the Download button
        assert_eq!(
            chip_status(
                &setting(CHIP_AUTO, ""),
                &[AdapterFirmware { firmware_download: true, ..rtl(Some(false)) }],
                &[]
            ),
            ChipStatus::Missing { files: rtl_files(), download: true }
        );
    }

    #[test]
    fn tells_about_another_vendor_where_v1_did_instead_of_no_files_needed() {
        let intel = other("Intel", Some("firmware.vendor_intel"));
        assert_eq!(
            chip_status(&setting(CHIP_AUTO, ""), std::slice::from_ref(&intel), &[]),
            ChipStatus::Vendor { note: Msg::new("firmware.vendor_intel"), vendor: "Intel".to_owned() }
        );
        assert_eq!(
            chip_status(&setting(CHIP_NOT_REALTEK, ""), &[other("Broadcom", Some("firmware.vendor_broadcom"))], &[])
                .kind(),
            "vendor"
        );
        // With a Realtek adapter there, Not Realtek is just that
        assert_eq!(chip_status(&setting(CHIP_NOT_REALTEK, ""), &[rtl(None), intel], &[]).kind(), "non_realtek");
        assert_eq!(chip_status(&setting(CHIP_NOT_REALTEK, ""), &[], &[]).kind(), "non_realtek");
    }

    #[test]
    fn asks_for_the_chip_when_auto_finds_none() {
        assert_eq!(chip_status(&setting(CHIP_AUTO, ""), &[], &[]).kind(), "auto_no_adapter");
        assert_eq!(chip_status(&setting(CHIP_AUTO, ""), &[other("Unknown", None)], &[]).kind(), "auto_unrecognized");
        assert_eq!(chip_status(&setting(0x1234, ""), &[other("Unknown", None)], &[]).kind(), "select_chip");
    }

    fn unchecked_present(s: ChipStatus) -> Option<bool> {
        match s {
            ChipStatus::Unchecked { present, .. } => present,
            other => panic!("not unchecked: {other:?}"),
        }
    }

    #[test]
    fn checks_the_folder_scan_when_no_adapter_is_there() {
        assert_eq!(
            chip_status(&setting(CHIP_AUTO, "rtl8723bu"), &[], &[file("RTL8723BU", None, true)]),
            ChipStatus::Unchecked {
                files: vec!["rtl8723bu_fw.bin".to_owned(), "rtl8723bu_config.bin".to_owned()],
                present: Some(true),
            }
        );
        assert_eq!(
            unchecked_present(chip_status(&setting(0xb82c, ""), &[], &[file("rtl8822bu", Some(0xb82c), false)])),
            Some(false)
        );
        assert_eq!(unchecked_present(chip_status(&setting(0xc821, ""), &[], &[])), Some(false));
        // The scan leaves custom out
        assert_eq!(unchecked_present(chip_status(&setting(CHIP_CUSTOM, "custom"), &[], &[])), None);
    }

    #[test]
    fn gives_each_adapter_card_a_short_state() {
        let s = setting(CHIP_AUTO, "");
        let tag = |key: &'static str, args: Option<Vec<&str>>, status: TagStatus| FirmwareTag {
            key,
            args: args.map(|a| a.into_iter().map(str::to_owned).collect()),
            status,
        };
        assert_eq!(adapter_firmware_tag(&rtl(Some(true)), &s, None), tag("firmware.present", None, TagStatus::Good));
        assert_eq!(
            adapter_firmware_tag(&rtl(Some(false)), &s, None),
            tag("firmware.missing_short", None, TagStatus::Bad)
        );
        // Realtek's Windows driver has it (issue #86): as MediaTek's
        assert_eq!(
            adapter_firmware_tag(&AdapterFirmware { firmware_from_driver: true, ..rtl(Some(true)) }, &s, None),
            tag("ui.adapter.firmware_from_driver", Some(vec!["Realtek"]), TagStatus::Good)
        );
        // Not on WinUSB yet (Windows runs it, or it is disabled): amber
        assert_eq!(
            adapter_firmware_tag(
                &AdapterFirmware { driver: Some(DriverKind::Bluetooth), ..rtl(Some(false)) },
                &s,
                None
            ),
            tag("firmware.missing_short", None, TagStatus::Warn)
        );
        // Another vendor's: where its firmware comes from (issue #86), how its
        // loader went once one ran on it
        assert_eq!(
            adapter_firmware_tag(&other("Intel", Some("firmware.vendor_intel")), &s, None).key,
            "ui.adapter.firmware_from_folder"
        );
        let mtk =
            AdapterFirmware { driver: Some(DriverKind::Winusb), ..other("MediaTek", Some("firmware.vendor_mediatek")) };
        assert_eq!(
            adapter_firmware_tag(&mtk, &s, None),
            tag("ui.adapter.firmware_from_driver", Some(vec!["MediaTek"]), TagStatus::Good)
        );
        assert_eq!(
            adapter_firmware_tag(&mtk, &s, Some(&loader("MediaTek", Some(true)))).key,
            "ui.adapter.firmware_loaded"
        );
        assert_eq!(
            adapter_firmware_tag(&mtk, &s, Some(&loader("MediaTek", Some(false)))),
            tag("ui.adapter.firmware_load_failed", None, TagStatus::Bad)
        );
        // Another adapter's loader, or not on WinUSB: not this one's
        assert_eq!(
            adapter_firmware_tag(&mtk, &s, Some(&loader("Intel", Some(false)))).key,
            "ui.adapter.firmware_from_driver"
        );
        assert_eq!(
            adapter_firmware_tag(
                &AdapterFirmware { driver: Some(DriverKind::Bluetooth), ..mtk.clone() },
                &s,
                Some(&loader("MediaTek", Some(false)))
            )
            .key,
            "ui.adapter.firmware_from_driver"
        );
        assert_eq!(
            adapter_firmware_tag(&other("CSR", Some("firmware.vendor_csr")), &s, None).key,
            "ui.adapter.firmware_not_needed"
        );
        assert_eq!(
            adapter_firmware_tag(&other("Qualcomm", Some("firmware.vendor_unsupported")), &s, None),
            tag("ui.adapter.firmware_unsupported", None, TagStatus::Bad)
        );
        assert_eq!(
            adapter_firmware_tag(&rtl(None), &setting(CHIP_NOT_REALTEK, ""), None).key,
            "ui.adapter.firmware_not_realtek"
        );
        assert_eq!(
            adapter_firmware_tag(&other("Unknown", None), &s, None),
            tag("ui.adapter.firmware_choose_chip", None, TagStatus::Warn)
        );
    }

    #[test]
    fn tells_a_file_that_is_no_firmware_apart_from_a_missing_one() {
        // Issue #66: kernel.org's preview page saved as rtl8761bu_fw.bin
        let page = AdapterFirmware { firmware_invalid: Some("rtl8761bu_fw.bin".to_owned()), ..rtl(Some(false)) };
        assert_eq!(
            chip_status(&setting(CHIP_AUTO, ""), std::slice::from_ref(&page), &[]),
            ChipStatus::Invalid { files: rtl_files(), file: "rtl8761bu_fw.bin".to_owned(), download: false }
        );
        assert_eq!(
            adapter_firmware_tag(&page, &setting(CHIP_AUTO, ""), None),
            FirmwareTag { key: "firmware.invalid_short", args: None, status: TagStatus::Bad }
        );
        assert_eq!(
            chip_status(
                &setting(CHIP_AUTO, ""),
                &[AdapterFirmware { firmware_invalid: None, ..rtl(Some(false)) }],
                &[]
            )
            .kind(),
            "missing"
        );
        // Present wins over a stale name
        assert_eq!(
            chip_status(
                &setting(CHIP_AUTO, ""),
                &[AdapterFirmware { firmware_invalid: Some("x".to_owned()), ..rtl(Some(true)) }],
                &[]
            )
            .kind(),
            "present"
        );
    }

    #[test]
    fn chip_target_index_picks_the_realtek_adapter_then_a_checked_one_then_a_vendor_note_then_the_first() {
        let base = AdapterFirmware { vendor: "x".to_owned(), ..Default::default() };
        assert_eq!(chip_target_index(&[]), None);
        assert_eq!(chip_target_index(&[base.clone(), base.clone()]), Some(0));
        let note = AdapterFirmware { vendor_note: Some(Msg::new("firmware.vendor_intel")), ..base.clone() };
        assert_eq!(chip_target_index(&[base.clone(), note.clone()]), Some(1));
        assert_eq!(
            chip_target_index(&[note.clone(), AdapterFirmware { firmware_present: Some(false), ..base.clone() }]),
            Some(1)
        );
        assert_eq!(
            chip_target_index(&[
                note,
                base.clone(),
                AdapterFirmware { realtek_chip: Some("RTL8761BU".to_owned()), ..base }
            ]),
            Some(2)
        );
    }
}
