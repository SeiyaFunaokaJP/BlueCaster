//! stream.rs' tests: the streaming controls, the main window's helpers and
//! the headset chip

use ipc::{
    AdapterState, CodecDetails, CodecStop, Device, LinkSample, Msg, Profile, SbcDetails, Settings, Snapshot,
    SourceInfo, StreamInfo, StreamState, StreamStatus, UpdateState, VoiceStatus,
};

use super::*;
use crate::profile::audio_entries;
use crate::sbc::mock::{sample_device, sample_profile};

fn sample(i: u64) -> LinkSample {
    LinkSample {
        time_ms: i * 250,
        rssi: Some(-18),
        afh_in_use: Some(79),
        afh_usable: Some(79),
        bytes_per_second: 125_000.0,
        packets_per_second: 188.0,
        airtime: 0.4,
        ..LinkSample::default()
    }
}

fn run(n: u64, f: impl Fn(u64, &mut LinkSample)) -> Vec<LinkSample> {
    (0..n)
        .map(|i| {
            let mut s = sample(i);
            f(i, &mut s);
            s
        })
        .collect()
}

fn quality(level: u8, status: QualityStatus, pc_busy: bool, causes: Vec<QualityCause>) -> Option<LinkQuality> {
    Some(LinkQuality { level, status, pc_busy, causes })
}

// ---- link quality ----

#[test]
fn is_good_on_a_link_that_delivers_everything_whatever_the_rssi() {
    let q = link_quality(&run(60, |i, s| s.rssi = Some(if i % 2 == 1 { -27 } else { -6 })));
    assert_eq!(q, quality(4, QualityStatus::Good, false, vec![]));
}

#[test]
fn does_not_blame_the_radio_for_audio_the_pc_lost_at_capture() {
    let q = link_quality(&run(60, |i, s| s.capture_dropped = if i < 50 { 0 } else { 480 }));
    assert_eq!(q, quality(4, QualityStatus::Warn, true, vec![]));
    let later = run(60, |i, s| s.capture_dropped = if i < 10 { 0 } else { 480 });
    assert!(!link_quality(&later).unwrap().pc_busy);
}

#[test]
fn says_what_the_radio_does_when_both_go_wrong() {
    let q = link_quality(&run(60, |i, s| {
        s.dropped = if i < 50 { 0 } else { 128 };
        s.capture_dropped = if i < 50 { 0 } else { 480 };
    }));
    assert_eq!(q, quality(1, QualityStatus::Bad, false, vec![QualityCause::Drops]));
}

#[test]
fn is_poor_while_packets_drop_and_recovers_10_s_later() {
    let drops = run(60, |i, s| s.dropped = if i < 50 { 0 } else { 128 });
    assert_eq!(link_quality(&drops).unwrap().status, QualityStatus::Bad);
    let mut later = drops.clone();
    later.extend(run(40, |_, s| s.dropped = 128));
    assert_eq!(link_quality(&later).unwrap().status, QualityStatus::Good);
}

#[test]
fn is_fair_while_the_send_queue_backs_up() {
    let q = link_quality(&run(30, |i, s| s.queue_ms = Some(if i == 25 { 200 } else { 0 }))).unwrap();
    assert_eq!(q.status, QualityStatus::Warn);
    assert!(!q.pc_busy);
    assert_eq!(q.causes, vec![QualityCause::Queue]);
}

#[test]
fn names_every_cause_that_lowered_it() {
    let q = link_quality(&run(60, |i, s| {
        s.dropped = if i < 55 { 0 } else { 2 };
        s.queue_ms = Some(if i == 58 { 100 } else { 0 });
        s.airtime = 0.85;
    }))
    .unwrap();
    assert_eq!(q.causes, vec![QualityCause::Drops, QualityCause::Queue, QualityCause::Airtime]);
    assert_eq!(q.level, 2);
}

#[test]
fn has_no_data_when_nothing_streams() {
    assert_eq!(link_quality(&[]), None);
    let idle = LinkSample { bytes_per_second: 0.0, rssi: None, ..sample(0) };
    assert_eq!(link_quality(&[idle]), None);
    assert_eq!(quality_label(None), "No data");
}

// ---- codec summary ----

fn sbc_abr() -> CodecDetails {
    CodecDetails {
        mtu: 1011,
        rtp: true,
        media_header: 1,
        frame_samples: 128,
        frame_bytes: Some(95),
        frames_per_packet: Some(10),
        sbc: Some(SbcDetails {
            channel_mode: "joint_stereo".into(),
            blocks: 16,
            subbands: 8,
            allocation: "loudness".into(),
            bitpool: 41,
            config_bitpool: Some((2, 53)),
            device: None,
            abr_bitpool: Some((19, 53)),
        }),
        aac: None,
        device: None,
    }
}

fn sbc_info() -> StreamInfo {
    StreamInfo {
        codec: "SBC".into(),
        bitrate_kbps: 285,
        sample_rate: 48000,
        channels: 2,
        bits: 16,
        source_sample_rate: 48000,
        source_channels: 2,
        source_bits: 32,
        requested_codec: None,
        requested_in_use: false,
        abr: true,
        bitrate_capped: false,
        hi_res_set: false,
        low_latency: false,
        source: None,
        le: None,
        details: Some(Box::new(sbc_abr())),
    }
}

#[test]
fn shows_sbcs_bitpool_now_and_its_ceiling_under_abr() {
    let info = sbc_info();
    assert_eq!(codec_details(&info), ["285 kbps", "48 kHz", "16-bit", "ABR", "bitpool 41/53"]);
    assert_eq!(codec_details(&info).join(" | "), "285 kbps | 48 kHz | 16-bit | ABR | bitpool 41/53");
    assert_eq!(abr_tooltip_key(&info), "tooltip.abr");
}

#[test]
fn shows_no_bitpool_without_abr_nor_for_other_codecs() {
    let mut details = sbc_abr();
    details.sbc.as_mut().unwrap().abr_bitpool = None;
    let fixed = StreamInfo { abr: false, details: Some(Box::new(details)), ..sbc_info() };
    assert_eq!(codec_details(&fixed), ["285 kbps", "48 kHz", "16-bit"]);
    let ldac = StreamInfo { codec: "LDAC".into(), bitrate_kbps: 660, bits: 24, details: None, ..sbc_info() };
    assert_eq!(codec_details(&ldac), ["660 kbps", "48 kHz", "24-bit", "ABR"]);
    assert_eq!(abr_tooltip_key(&ldac), "tooltip.abr");
}

#[test]
fn calls_aacs_flag_vbr() {
    let aac = StreamInfo { codec: "AAC".into(), bitrate_kbps: 256, sample_rate: 44100, details: None, ..sbc_info() };
    assert_eq!(codec_details(&aac), ["256 kbps", "44.1 kHz", "16-bit", "VBR"]);
    assert_eq!(abr_tooltip_key(&aac), "tooltip.vbr");
}

#[test]
fn says_beside_the_codec_why_the_chosen_one_is_not_used() {
    assert_eq!(fallback_note_key(&sbc_info(), "status.connected"), None);
    let opus = StreamInfo { requested_codec: Some("Opus".into()), ..sbc_info() };
    assert_eq!(fallback_note_key(&opus, "status.connected_fallback"), Some("ui.codec.fallback_unsupported"));
    let in_use = StreamInfo { requested_in_use: true, ..opus.clone() };
    assert_eq!(fallback_note_key(&in_use, "status.connected_fallback_in_use"), Some("ui.codec.fallback_in_use"));
    // Opus 05 outside developer mode
    let opus05 = StreamInfo { requested_codec: Some("Opus 05".into()), ..sbc_info() };
    assert_eq!(fallback_note_key(&opus05, "status.connected_fallback_developer"), Some("ui.codec.fallback_developer"));
    // A later message (a reconnect) keeps the note
    assert_eq!(fallback_note_key(&opus, "status.reconnected"), Some("ui.codec.fallback_unsupported"));
    assert_eq!(fallback_note(&opus, "status.connected_fallback"), "(Opus is unsupported)");
    assert_eq!(fallback_note(&sbc_info(), "status.connected"), "");
}

// ---- the main window's helpers ----

#[test]
fn source_text_names_what_a_stream_captures() {
    let p = Profile { app_exe: "Spotify.exe".into(), app_name: "Spotify".into(), ..sample_profile() };
    let src = |kind: &str, name: &str, waiting: bool| SourceInfo { kind: kind.into(), name: name.into(), waiting };
    assert_eq!(source_text(&src("tone", "", false), None).text, "Test tone");
    assert_eq!(source_text(&src("loopback", "", false), None).text, "Default device");
    assert_eq!(source_text(&src("virtual", "CABLE Input", false), None).text, "CABLE Input");
    assert_eq!(source_text(&src("app", "spotify.exe", false), Some(&p)).text, "Spotify");
    let waiting = source_text(&src("app", "foo.exe", true), Some(&p));
    assert!(waiting.waiting);
    assert_eq!(waiting.text, crate::t!("ui.source.waiting", "foo.exe"));
}

fn msg(key: &str, args: &[&str]) -> Msg {
    Msg { key: key.into(), args: args.iter().map(|a| (*a).to_owned()).collect() }
}

#[test]
fn a_reconnect_reads_as_connecting_and_its_end_as_connected() {
    use StreamState::*;
    let st = |state, m: Msg| status_text(state, &m, None);
    assert_eq!(st(Reconnecting, msg("status.reconnect_attempt", &["1", "3"])), "Connecting...");
    assert_eq!(st(Connecting, msg("status.connecting_retry", &["2", "3"])), "Connecting...");
    assert_eq!(st(Streaming, msg("status.reconnected", &[])), "Connected");
    // Why another codec is used is on the codec's line (issue #86)
    assert_eq!(st(Streaming, msg("status.connected_fallback", &["Opus", "aptX"])), "Connected");
    assert_eq!(st(Streaming, msg("status.connected_fallback_in_use", &["LDAC", "AAC"])), "Connected");
    assert_eq!(st(Connecting, msg("status.initializing_btstack", &[])), "Starting Bluetooth...");
    assert_eq!(st(Error, msg("error.connect_no_answer", &[])), "Error");
    // An error the codec caused names it in a few words (issue #86)
    let refused = msg("error.codec_unsupported_refused", &["LDAC"]);
    let stopped = |codec: &str, kind| status_text(Error, &refused, Some(&CodecStop { codec: codec.into(), kind }));
    assert_eq!(stopped("ldac", CodecStopKind::Unsupported), "LDAC is unsupported");
    assert_eq!(stopped("sbcxq", CodecStopKind::SettingsUnsupported), "Unsupported SBC XQ settings");
    assert_eq!(stopped("opus05", CodecStopKind::InUse), "Opus (PipeWire) is in use");
    assert_eq!(stopped("aac", CodecStopKind::Refused), "AAC was refused");
    assert_eq!(status_text(Error, &refused, None), "Error");
    assert_eq!(st(Idle, msg("", &[])), "Disconnected");
    // Streaming reads "Connected" everywhere (issue #86); the other states keep their words
    assert_eq!(st(Streaming, msg("", &[])), "Connected");
    assert_eq!(state_label(Streaming), "Connected");
    assert_eq!(state_label(Connecting), "Connecting...");
    assert_eq!(state_label(Reconnecting), "Reconnecting...");
    assert_eq!(state_label(Idle), "Disconnected");
}

#[test]
fn error_copy_text_adds_the_requests_code() {
    let m = msg("error.connect_no_answer", &[]);
    assert_eq!(error_copy_text(&m, None), t("error.connect_no_answer"));
    assert!(error_copy_text(&m, Some("stream.failed")).ends_with("\n[stream.failed]"));
}

#[test]
fn offers_no_bug_report_for_a_cause_on_the_users_side() {
    assert!(!is_app_fault("error.connect_no_answer", None));
    assert!(!is_app_fault("error.encoder_init", Some("stream.bad_codec")));
    assert!(is_app_fault("error.encoder_init", None));
    // Not known to be the user's: BlueCaster's own
    assert!(is_app_fault("error.something_new", None));
}

#[test]
fn capture_sign_text_from_v1s_title_suffix() {
    assert_eq!(capture_running_text(), "HCI capture running");
}

#[test]
fn a_report_puts_the_details_before_the_diagnostic_info() {
    let info = diag_info_body("```text\nBlueCaster 3.0.4\n```\n");
    assert_eq!(info, "BlueCaster 3.0.4\n");
    let r = report_text("Crash: core\npanic", "bluecaster_core::encode::run\n\n", &info);
    assert_eq!(r.log, "bluecaster_core::encode::run\n\nBlueCaster 3.0.4\n");
    assert_eq!(r.text, "Crash: core\npanic\n\n```text\nbluecaster_core::encode::run\n\nBlueCaster 3.0.4\n```\n");
    // Without the core's info: what the UI has
    assert_eq!(report_text("Error", "", "").text, "Error");
}

#[test]
fn update_notes_read_as_plain_lines() {
    let lines = note_lines(
        "## What's new\n- Faster **scan**: devices show up as they answer\n\n* A [notification](https://x/y) when a `device` connects (#86)\n",
    );
    let texts: Vec<(&str, bool)> = lines.iter().map(|l| (l.text.as_str(), l.bullet)).collect();
    assert_eq!(
        texts,
        [
            ("What's new", false),
            ("Faster scan: devices show up as they answer", true),
            ("A notification when a device connects (#86)", true),
        ]
    );
}

// ---- devices, the status block and the headset chip ----

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

/// Two saved devices, the XM5 streaming LDAC with its microphone on Auto
fn snapshot() -> Snapshot {
    let xm5 = Profile { headset_mic: "auto".into(), mic_device_id: "{cable}".into(), ..sample_profile() };
    let xb13 = Profile {
        name: "SRS-XB13 | AAC".into(),
        device_address: XB13.into(),
        device_name: "SRS-XB13".into(),
        codec: "aac".into(),
        ..sample_profile()
    };
    let device = |address: &str, name: &str| Device { name: name.into(), saved: true, ..sample_device(address) };
    Snapshot {
        adapter: AdapterState::default(),
        devices: vec![device(XM5, "WH-1000XM5"), device(XB13, "SRS-XB13")],
        scanning: false,
        le_scanning: false,
        stream: StreamStatus {
            state: StreamState::Streaming,
            message: msg("status.connected", &[]),
            profile: Some(xm5.name.clone()),
            info: Some(StreamInfo {
                codec: "LDAC".into(),
                bitrate_kbps: 990,
                bits: 24,
                abr: false,
                details: None,
                ..sbc_info()
            }),
            ..StreamStatus::default()
        },
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

#[test]
fn each_card_shows_its_own_devices_state() {
    let snap = snapshot();
    let entries = audio_entries(&snap);
    assert_eq!(entries.len(), 2);
    assert_eq!(entry_state(&snap, &entries[0]), StreamState::Streaming);
    assert_eq!(entry_state(&snap, &entries[1]), StreamState::Idle);
    assert_eq!(current_profile(&snap).map(|p| p.name.as_str()), Some("WH-1000XM5 (LDAC)"));
    assert_eq!(pill_label(&snap, Some(&entries[0])), "WH-1000XM5, Connected, LDAC 990 kbps 48 kHz 24-bit");
    assert_eq!(pill_label(&snap, Some(&entries[1])), "SRS-XB13, Disconnected");
    let idle = Snapshot { stream: StreamStatus::default(), ..snap };
    assert_eq!(pill_label(&idle, None), "Disconnected");
}

#[test]
fn the_headset_chip_is_only_on_the_card_of_a_device_in_use_with_a_microphone_output() {
    let snap = snapshot();
    let entries = audio_entries(&snap);
    assert!(headset_chip(&snap, &entries[0]).is_some());
    assert!(mic_shown(&snap, entries[0].profile.as_ref()));
    assert!(headset_chip(&snap, &entries[1]).is_none());
    // Headset mode off: no chip
    let mut off = snap.clone();
    off.profiles[0].headset_mic = "off".into();
    assert!(headset_chip(&off, &audio_entries(&off)[0]).is_none());
    // No microphone output: no chip
    let mut none = snap.clone();
    none.profiles[0].mic_device_id.clear();
    assert!(headset_chip(&none, &audio_entries(&none)[0]).is_none());
    // Not connected: no chip
    let idle = Snapshot { stream: StreamStatus::default(), ..snap };
    assert!(headset_chip(&idle, &audio_entries(&idle)[0]).is_none());
}

#[test]
fn the_headset_chip_shows_auto_waiting_linked_and_muted() {
    let snap = snapshot();
    let chip = headset_chip(&snap, &audio_entries(&snap)[0]).unwrap();
    assert!(chip.on && chip.waiting && !chip.linked && !chip.muted);
    assert_eq!(chip.title, "Disable microphone");
    assert_eq!(mic_idle_text(&snap.profiles[0], false), "Auto");

    let mut linked = snap.clone();
    linked.stream.voice =
        Some(VoiceStatus { codec: "msbc".into(), link: "hfp".into(), rate: 16000, mic_device_id: "{cable}".into() });
    let chip = headset_chip(&linked, &audio_entries(&linked)[0]).unwrap();
    assert!(chip.on && chip.linked && !chip.waiting);
    assert_eq!(chip.title, "Disable microphone (mSBC)");

    let mut muted = snap.clone();
    muted.profiles[0].mic_muted = true;
    let chip = headset_chip(&muted, &audio_entries(&muted)[0]).unwrap();
    assert!(chip.muted && !chip.on && !chip.waiting);
    assert_eq!(chip.title, "Enable microphone");
    assert_eq!(mic_idle_text(&muted.profiles[0], false), "Off");
}

#[test]
fn the_headset_chip_shows_why_the_microphone_did_not_open_not_the_cards_status() {
    let mut snap = snapshot();
    snap.stream.message = msg("status.headset_mic_failed", &[]);
    let chip = headset_chip(&snap, &audio_entries(&snap)[0]).unwrap();
    assert_eq!(chip.note.as_deref(), Some(t("status.headset_mic_failed").as_str()));
    assert_eq!(chip.title, t("status.headset_mic_failed"));
    assert!(!chip.on && !chip.waiting);
    assert_eq!(stream_status_text(&snap.stream), "Connected");
}

#[test]
fn on_works_as_auto_outside_developer_mode() {
    let mut snap = snapshot();
    snap.profiles[0].headset_mic = "on".into();
    assert!(headset_chip(&snap, &audio_entries(&snap)[0]).unwrap().waiting);
    assert_eq!(mic_idle_text(&snap.profiles[0], false), "Auto");
    assert_eq!(mic_idle_text(&snap.profiles[0], true), "On");
    snap.settings.developer_mode = true;
    assert!(!headset_chip(&snap, &audio_entries(&snap)[0]).unwrap().waiting);
}

#[test]
fn an_error_leads_to_its_fix() {
    let snap = snapshot();
    assert_eq!(fix_for(&snap, None, "error.connect_no_answer"), FixAction::Retry("WH-1000XM5 (LDAC)".into()));
    assert_eq!(fix_for(&snap, None, "error.codec_not_supported"), FixAction::Codecs);
    assert_eq!(fix_for(&snap, None, "error.no_app_selected"), FixAction::Devices);
    assert_eq!(fix_for(&snap, None, "error.btstack_crash_x"), FixAction::RestartAdapter);
    assert_eq!(fix_for(&snap, None, "firmware.rom_running"), FixAction::AdapterFirmware);
    assert_eq!(fix_for(&snap, Some("adapter.driver_missing"), "x"), FixAction::Adapter);
    assert_eq!(fix_for(&snap, None, "error.something_new"), FixAction::Diagnostics);
    assert!(!hidden_by_hung(&snap, Some(&msg("error.btstack_init", &[]))));
}

#[test]
fn battery_changes_colour_with_its_level() {
    assert_eq!(battery_tone(None), BatteryTone::Unknown);
    assert_eq!(battery_tone(Some(15)), BatteryTone::Bad);
    assert_eq!(battery_tone(Some(40)), BatteryTone::Warn);
    assert_eq!(battery_tone(Some(41)), BatteryTone::Good);
    assert_eq!(battery_text(Some(80)), "Battery 80%");
    assert_eq!(battery_text(None), "Battery unknown");
}
