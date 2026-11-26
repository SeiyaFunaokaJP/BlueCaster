//! home.rs' tests: Home's rules (how the window draws them is not tested
//! here)

use ipc::{
    AdapterInfo, AdapterState, AdapterStatus, Device, DriverKind, ErrorInfo, Msg, OtherRadio, Profile, Settings,
    Snapshot, SourceInfo, StreamInfo, StreamState, StreamStatus, UpdateState,
};

use super::*;
use crate::i18n::t;
use crate::profile::audio_entries;
use crate::sbc::mock::{sample_device, sample_profile};

fn msg(key: &str, args: &[&str]) -> Msg {
    Msg { key: key.into(), args: args.iter().map(|a| (*a).to_owned()).collect() }
}

fn settings() -> Settings {
    serde_json::from_value(serde_json::json!({
        "language": "en", "theme": "system", "start_with_windows": false, "start_minimized": false,
        "minimize_to_tray": true, "check_updates_on_startup": true, "include_prereleases": false,
        "media_keys": true, "developer_mode": false, "bt_chip_pid": 0,
        "bt_chip_fw_stem": "", "afh": "auto", "remote_sink": ""}))
    .unwrap()
}

const XM5: &str = "AC:80:0A:12:34:56";
const XB13: &str = "00:11:22:33:44:55";
const MOTO: &str = "54:2F:04:4B:29:DC";

fn adapter() -> AdapterInfo {
    serde_json::from_value(serde_json::json!({
        "instance_id": "USB\\VID_2357&PID_0604\\00E04C239987", "vid": 0x2357, "pid": 0x0604,
        "display_name": "RTL8761BU (0x2357:0x0604)", "device_name": "TP-Link UB500", "vendor": "Realtek",
        "driver": "winusb", "driver_service": "WinUSB", "realtek_chip": "8761bu", "firmware_present": true,
        "firmware_files": ["rtl8761bu_fw.bin", "rtl8761bu_config.bin"], "firmware_download": true,
        "restart_needed": false}))
    .unwrap()
}

/// Two saved devices with a profile each and one without, nothing streaming
fn snapshot() -> Snapshot {
    let xm5 = sample_profile();
    let xb13 = Profile {
        name: "Desk speaker (AAC)".into(),
        device_address: XB13.into(),
        device_name: "SRS-XB13".into(),
        codec: "aac".into(),
        ..sample_profile()
    };
    let device = |address: &str, name: &str| Device { name: name.into(), saved: true, ..sample_device(address) };
    Snapshot {
        adapter: AdapterState { status: AdapterStatus::Ready, adapters: vec![adapter()], ..AdapterState::default() },
        devices: vec![device(XM5, "WH-1000XM5"), device(XB13, "SRS-XB13"), device(MOTO, "moto buds+")],
        scanning: false,
        le_scanning: false,
        stream: StreamStatus::default(),
        active_profile: Some(xm5.name.clone()),
        profiles: vec![xm5, xb13],
        settings: settings(),
        update: UpdateState::default(),
        config_dir: String::new(),
        log_dir: String::new(),
        capture: None,
        clock_ms: None,
        v1_running: false,
        verbose: None,
        crashes: Vec::new(),
        audio_endpoints: 0,
        vendors: Vec::new(),
        hid_devices: Vec::new(),
        hid_candidates: Vec::new(),
        hid_passkey: None,
        hid_checks: Vec::new(),
        hid_rolled_back: None,
        hid_health: None,
        service_mode: false,
    }
}

fn info(codec: &str, kbps: u32) -> StreamInfo {
    StreamInfo {
        codec: codec.into(),
        bitrate_kbps: kbps,
        sample_rate: 96000,
        channels: 2,
        bits: 24,
        source_sample_rate: 48000,
        source_channels: 2,
        source_bits: 32,
        requested_codec: None,
        requested_in_use: false,
        abr: false,
        bitrate_capped: false,
        hi_res_set: false,
        low_latency: false,
        source: None,
        le: None,
        details: None,
    }
}

/// The XM5 in `state`
fn with_stream(state: StreamState, key: &str) -> Snapshot {
    let mut snap = snapshot();
    snap.stream = StreamStatus {
        state,
        message: msg(key, &[]),
        profile: Some("WH-1000XM5 (LDAC)".into()),
        info: (state == StreamState::Streaming).then(|| info("LDAC", 990)),
        volume: Some(80),
        ..StreamStatus::default()
    };
    snap
}

fn card(snap: &Snapshot, name: &str) -> DeviceCard {
    let entries = audio_entries(snap);
    device_card(snap, entries.iter().find(|e| e.name == name).unwrap())
}

fn press(snap: &Snapshot, name: &str) -> CardPress {
    let entries = audio_entries(snap);
    card_press(snap, entries.iter().find(|e| e.name == name).unwrap())
}

#[test]
fn connects_and_disconnects_with_the_devices_own_button() {
    let idle = snapshot();
    assert_eq!(press(&idle, "WH-1000XM5"), CardPress::Start("WH-1000XM5 (LDAC)".into()));
    let snap = with_stream(StreamState::Streaming, "status.connected");
    let wh = card(&snap, "WH-1000XM5");
    assert!(wh.live && wh.running && wh.dot);
    assert_eq!(wh.label, "Disconnect");
    assert_eq!(wh.status, "Connected");
    assert_eq!(press(&snap, "WH-1000XM5"), CardPress::Stop);
    // The other device: ready, not connected, its own Connect
    let xb = card(&snap, "SRS-XB13");
    assert!(!xb.live && !xb.running);
    assert_eq!(xb.status, "Disconnected");
    assert_eq!(xb.label, "Connect");
    // Connecting: Cancel stops it
    let connecting = with_stream(StreamState::Connecting, "status.connecting");
    assert_eq!(card(&connecting, "WH-1000XM5").label, t("modal.cancel"));
    assert_eq!(press(&connecting, "WH-1000XM5"), CardPress::Stop);
}

#[test]
fn switches_to_another_device_with_its_connect() {
    let snap = with_stream(StreamState::Streaming, "status.connected");
    // stream.start stops the running stream itself: no stream.stop
    assert_eq!(press(&snap, "SRS-XB13"), CardPress::Start("Desk speaker (AAC)".into()));
}

#[test]
fn gives_a_device_without_a_profile_one_when_connected() {
    assert_eq!(press(&snapshot(), "moto buds+"), CardPress::StartAuto);
}

#[test]
fn connects_a_device_again_after_an_error_instead_of_offering_disconnect() {
    let snap = with_stream(StreamState::Error, "error.connect_no_answer");
    let wh = card(&snap, "WH-1000XM5");
    assert!(wh.live && !wh.running);
    assert_eq!(wh.status, "Error");
    // The red word alone: a red dot read as recording (issue #86)
    assert!(!wh.dot);
    assert_eq!(wh.label, "Connect");
    assert_eq!(press(&snap, "WH-1000XM5"), CardPress::Start("WH-1000XM5 (LDAC)".into()));
}

#[test]
fn shows_the_battery_of_the_device_in_use_also_when_refused() {
    let mut snap = with_stream(StreamState::Streaming, "status.connected");
    assert!(!card(&snap, "WH-1000XM5").battery);
    snap.stream.battery_refused = true;
    assert!(card(&snap, "WH-1000XM5").battery);
    assert!(!card(&snap, "SRS-XB13").battery);
    snap.stream.battery_refused = false;
    snap.stream.battery = Some(80);
    assert!(card(&snap, "WH-1000XM5").battery);
}

#[test]
fn shows_the_capture_source_waiting_for_the_application() {
    let mut snap = with_stream(StreamState::Streaming, "status.connected");
    snap.stream.profile = Some("Desk speaker (AAC)".into());
    snap.profiles[1].app_exe = "Spotify.exe".into();
    snap.profiles[1].app_name = "Spotify".into();
    snap.stream.info.as_mut().unwrap().source =
        Some(SourceInfo { kind: "app".into(), name: "spotify.exe".into(), waiting: true });
    assert_eq!(card(&snap, "SRS-XB13").waiting, Some(crate::t!("ui.source.waiting", "Spotify")));
    assert_eq!(card(&snap, "WH-1000XM5").waiting, None);
    snap.stream.info.as_mut().unwrap().source.as_mut().unwrap().waiting = false;
    assert_eq!(card(&snap, "SRS-XB13").waiting, None);
}

#[test]
fn offers_to_add_a_device_beside_a_lone_card_not_beside_two() {
    let mut snap = snapshot();
    assert!(!offers_add(&audio_entries(&snap)));
    snap.devices.truncate(1);
    assert!(offers_add(&audio_entries(&snap)));
    snap.devices.clear();
    assert!(!offers_add(&audio_entries(&snap)));
}

#[test]
fn shows_the_cause_of_an_error() {
    assert_eq!(home_error(&snapshot(), None), None);
    let snap = with_stream(StreamState::Error, "error.connect_no_answer");
    assert_eq!(home_error(&snap, None), Some(HomeError { code: None, message: msg("error.connect_no_answer", &[]) }));
    // The adapter's, while the stream is fine
    let mut adapter = snapshot();
    adapter.adapter.status = AdapterStatus::Error;
    adapter.adapter.error = Some(msg("error.btstack_init", &[]));
    assert_eq!(home_error(&adapter, None).map(|e| e.message.key), Some("error.btstack_init".into()));
    // A failed request first, with its code
    let failed = ErrorInfo { code: "adapter.driver_missing".into(), message: msg("error.driver", &[]) };
    assert_eq!(home_error(&snap, Some(&failed)).and_then(|e| e.code), Some("adapter.driver_missing".into()));
}

#[test]
fn asks_for_a_power_cycle_instead_of_the_stacks_error_when_the_adapter_hung() {
    let mut snap = snapshot();
    snap.adapter.status = AdapterStatus::Error;
    snap.adapter.error = Some(msg("error.btstack_init", &[]));
    snap.adapter.hung = true;
    // Not "check the WinUSB driver" beside it
    assert_eq!(home_error(&snap, None), None);
    // Another error still shows
    let mut stream = with_stream(StreamState::Error, "error.connect_no_answer");
    stream.adapter.hung = true;
    assert!(home_error(&stream, None).is_some());
}

#[test]
fn drops_a_requests_error_once_the_adapter_is_on_winusb() {
    let mut snap = snapshot();
    snap.adapter.adapters[0].driver = DriverKind::Bluetooth;
    let before = request_error_key(&snap);
    snap.adapter.adapters[0].driver = DriverKind::Winusb;
    assert_ne!(request_error_key(&snap), before);
    // ... or the stream moves on
    let streaming = with_stream(StreamState::Connecting, "status.connecting");
    assert_ne!(request_error_key(&streaming), request_error_key(&snapshot()));
}

#[test]
fn shows_the_firmware_bar_with_the_files_and_the_download() {
    let mut snap = snapshot();
    assert_eq!(firmware_bar(&snap), None);
    snap.adapter.adapters[0].firmware_present = Some(false);
    let Some(FirmwareBar::Missing { adapters, title_key, downloadable }) = firmware_bar(&snap) else {
        panic!("no firmware bar")
    };
    assert_eq!(adapters.len(), 1);
    assert_eq!(adapters[0].firmware_files.join(", "), "rtl8761bu_fw.bin, rtl8761bu_config.bin");
    assert_eq!(title_key, "firmware.missing_manual");
    assert!(downloadable.is_some());
    // A file that is no firmware
    snap.adapter.adapters[0].firmware_invalid = Some("rtl8761bu_fw.bin".into());
    assert!(matches!(firmware_bar(&snap), Some(FirmwareBar::Missing { title_key: "firmware.invalid_short", .. })));
    // Not for an adapter Windows runs: it needs none yet (issue #86)
    snap.adapter.adapters[0].driver = DriverKind::Bluetooth;
    assert_eq!(firmware_bar(&snap), None);
    // The ROM code running (issue #81)
    snap.adapter.rom_firmware = true;
    assert_eq!(firmware_bar(&snap), Some(FirmwareBar::Rom));
}

#[test]
fn names_another_bluetooth_adapter() {
    let mut snap = snapshot();
    assert_eq!(other_radio_text(&snap), None);
    snap.adapter.other_radio = Some(OtherRadio { name: "MediaTek Bluetooth Adapter".into(), paired: true });
    assert_eq!(other_radio_text(&snap), Some(crate::t!("ui.home.other_adapter_paired", "MediaTek Bluetooth Adapter")));
    snap.adapter.other_radio.as_mut().unwrap().paired = false;
    assert_eq!(
        other_radio_text(&snap),
        Some(crate::t!("ui.home.other_adapter_unpaired", "MediaTek Bluetooth Adapter"))
    );
}

fn entry(snap: &Snapshot, name: &str) -> crate::profile::AudioEntry {
    audio_entries(snap).into_iter().find(|e| e.name == name).unwrap()
}

#[test]
fn chooses_the_codec_on_the_card_of_the_device_in_use_by_name_only() {
    let mut snap = with_stream(StreamState::Streaming, "status.connected");
    snap.devices[0].codecs = ["sbc", "aac", "ldac"].map(String::from).to_vec();
    snap.settings.auto_codec = false;
    let wh = entry(&snap, "WH-1000XM5");
    // The codecs the device supports in its order, no Auto-select entry
    let picker = codec_picker(&snap, &wh);
    assert_eq!(picker.choices, ["ldac", "aac", "sbc"]);
    assert_eq!(picker.value.as_deref(), Some("ldac"));
    // Saved at once, as a card chosen on Codecs
    assert_eq!(codec_pick(&snap, &wh, "aac"), CodecPick::Save { auto_off: false });
    assert_eq!(codec_pick(&snap, &wh, "ldac"), CodecPick::Keep);
    assert!(codec_restarts(&snap, &wh));
    // Under Auto-select it shows the codec streaming, by name
    snap.settings.auto_codec = true;
    snap.stream.info = Some(info("AAC", 256));
    let picker = codec_picker(&snap, &wh);
    assert_eq!(picker.value.as_deref(), Some("aac"));
    assert!(!picker.choices.iter().any(|c| c == "auto"));
    // A codec chosen under Auto-select turns it off first
    assert_eq!(codec_pick(&snap, &wh, "sbc"), CodecPick::Save { auto_off: true });
    // Its own codec while another one streams instead (in use elsewhere): kept
    snap.settings.auto_codec = false;
    assert_eq!(codec_pick(&snap, &wh, "ldac"), CodecPick::Keep);
}

#[test]
fn a_device_never_connected_offers_sbc_and_its_own_codec() {
    let snap = snapshot();
    let xb = entry(&snap, "SRS-XB13");
    let picker = codec_picker(&snap, &xb);
    assert_eq!(picker.value.as_deref(), Some("aac"));
    assert_eq!(picker.choices, ["aac", "sbc"]);
    // Not streaming: a choice starts nothing again
    assert!(!codec_restarts(&snap, &xb));
    // Auto with the codecs not known: none to show yet
    let moto = entry(&snap, "moto buds+");
    assert_eq!(codec_picker(&snap, &moto).value, None);
}

#[test]
fn says_hi_res_was_turned_on_until_dismissed() {
    let mut bar = HiResBar::default();
    let mut snap = with_stream(StreamState::Streaming, "status.connected");
    assert!(!bar.shown(&snap));
    snap.stream.info.as_mut().unwrap().hi_res_set = true;
    bar.update(&snap);
    assert!(bar.shown(&snap));
    bar.dismiss();
    assert!(!bar.shown(&snap));
    // The next stream it happens in says it again
    snap.stream.info.as_mut().unwrap().hi_res_set = false;
    bar.update(&snap);
    snap.stream.info.as_mut().unwrap().hi_res_set = true;
    bar.update(&snap);
    assert!(bar.shown(&snap));
}
