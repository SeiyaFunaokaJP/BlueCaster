//! codecs.rs' tests: the Codecs screen's rules, the headset microphone's and
//! LE Audio's codecs, and the SBC XQ bitpool range.

use ipc::{
    AdapterState, CodecDeviceCaps, Device, Msg, Profile, Settings, Snapshot, StreamInfo, StreamState, StreamStatus,
    UpdateState,
};

use super::*;
use crate::codecs::{CARD_ORDER, CodecSupport};
use crate::profile::audio_entries;
use crate::sbc::mock::{sample_device, sample_profile, xm5_sbc};

const XM5: &str = "AC:80:0A:12:34:56";
const XB13: &str = "00:1A:7D:DA:71:13";
const MOTO: &str = "34:1C:F0:AA:BB:CC";
const QC: &str = "AC:BF:71:AA:BB:CC";
const EARFUN: &str = "E8:AB:FA:AA:BB:CC";
const HUAWEI: &str = "CC:FF:90:12:34:56";
const NEW: &str = "54:2F:04:4B:29:DC";

fn caps(codec: &str, sample_rates: &[u32], bit_depths: &[u32], max_bitrate: u32) -> CodecDeviceCaps {
    CodecDeviceCaps {
        codec: codec.into(),
        sample_rates: sample_rates.to_vec(),
        bit_depths: bit_depths.to_vec(),
        max_bitrate,
        lhdcv5_low_latency: false,
    }
}

fn settings() -> Settings {
    serde_json::from_value(serde_json::json!({
        "language": "en", "theme": "system", "start_with_windows": false, "start_minimized": false,
        "minimize_to_tray": true, "check_updates_on_startup": true, "include_prereleases": false,
        "media_keys": true, "developer_mode": false, "bt_chip_pid": 0,
        "bt_chip_fw_stem": "", "afh": "auto", "remote_sink": "", "auto_codec": false}))
    .unwrap()
}

fn saved(address: &str, name: &str, codecs: &[&str]) -> Device {
    Device {
        name: name.into(),
        saved: true,
        codecs: codecs.iter().map(|c| (*c).to_owned()).collect(),
        ..sample_device(address)
    }
}

/// The test devices, "normal" and "capped": the WH-1000XM5 (SBC, AAC, LDAC,
/// with its profile on LDAC), the SRS-XB13 (never connected, its profile on
/// AAC), the moto buds+ (SBC max bitpool 39, AAC at 131 kbps, LHDC V5), two
/// aptX Adaptive earbuds, Huawei earbuds that offer LDAC once asked, and a
/// device saved without a profile
fn snapshot() -> Snapshot {
    let xm5 = Device {
        sbc: Some(xm5_sbc()),
        codec_caps: vec![
            caps("sbc", &[44100, 48000], &[], 0),
            caps("aac", &[44100, 48000], &[], 320_000),
            caps("ldac", &[44100, 48000, 88200, 96000], &[], 0),
        ],
        ..saved(XM5, "WH-1000XM5", &["sbc", "aac", "ldac"])
    };
    let moto = Device {
        sbc: Some(ipc::SbcDeviceCaps { max_bitpool: 39, ..xm5_sbc() }),
        codec_caps: vec![
            caps("sbc", &[44100, 48000], &[], 0),
            caps("aac", &[44100, 48000], &[], 0x020000),
            caps("lhdcv5", &[44100, 48000], &[16, 24], 0),
        ],
        ..saved(MOTO, "moto buds+", &["sbc", "aac", "lhdcv5"])
    };
    let qc = Device {
        codec_caps: vec![caps("aptxad", &[48000], &[], 0)],
        ..saved(QC, "QC Ultra Earbuds", &["sbc", "aac", "aptxad"])
    };
    let earfun = Device {
        codec_caps: vec![caps("aptxad", &[48000, 96000], &[], 0)],
        ..saved(EARFUN, "EarFun Air Pro 4+", &["sbc", "aac", "aptxad"])
    };
    let huawei = Device {
        codecs_on_request: vec!["ldac".into()],
        ..saved(HUAWEI, "HUAWEI FreeBuds Pro 4", &["sbc", "sbcxq", "aac"])
    };
    let xb13 = Profile {
        name: "Desk speaker (AAC)".into(),
        device_address: XB13.into(),
        device_name: "SRS-XB13".into(),
        codec: "aac".into(),
        ..sample_profile()
    };
    Snapshot {
        adapter: AdapterState::default(),
        devices: vec![xm5, saved(XB13, "SRS-XB13", &[]), moto, qc, earfun, huawei, saved(NEW, "moto", &[])],
        scanning: false,
        le_scanning: false,
        stream: StreamStatus::default(),
        active_profile: Some(sample_profile().name),
        profiles: vec![sample_profile(), xb13],
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

fn entry(snap: &Snapshot, address: &str) -> AudioEntry {
    audio_entries(snap).into_iter().find(|e| e.address == address).expect("a saved device")
}

fn ids(v: &DeviceCardsView) -> Vec<&'static str> {
    v.cards.iter().map(|c| c.id).collect()
}

fn cards(snap: &Snapshot, address: &str, hide: bool) -> DeviceCardsView {
    device_cards(snap, &entry(snap, address), hide, snap.settings.auto_codec, None)
}

fn set_profile(snap: &mut Snapshot, address: &str, f: impl FnOnce(&mut Profile)) {
    let p = snap.profiles.iter_mut().find(|p| p.device_address == address).expect("a profile");
    f(p);
}

fn info(codec: &str, bitrate_kbps: u32) -> StreamInfo {
    StreamInfo {
        codec: codec.into(),
        bitrate_kbps,
        sample_rate: 48000,
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

fn streaming(snap: &mut Snapshot, profile: &str, codec: &str) {
    snap.stream = StreamStatus {
        state: StreamState::Streaming,
        message: Msg { key: "status.connected".into(), args: Vec::new() },
        profile: Some(profile.into()),
        info: Some(info(codec, 990)),
        ..StreamStatus::default()
    };
}

// ---- the Codecs screen ----

#[test]
fn lists_each_saved_device_with_its_codec_cards_the_codec_in_use_chosen() {
    let snap = snapshot();
    let wh = cards(&snap, XM5, true);
    // Highest sound quality first; no Auto card (Auto is one switch for
    // every device, issue #86)
    assert_eq!(ids(&wh), ["ldac", "aac", "sbc"]);
    assert_eq!(wh.picked.as_deref(), Some("ldac"));
    assert!(wh.cards[0].selected && !wh.cards[1].selected);
    // Only the supported ones shown: no "Supported" tag on each
    assert!(wh.cards.iter().all(|c| c.support.is_none()));
    // Nothing to reset while the order is the default one
    assert!(!wh.reset && !wh.unknown);
    // A device never connected: its codec chosen and SBC
    let xb = cards(&snap, XB13, true);
    assert_eq!(ids(&xb), ["aac", "sbc"]);
    assert_eq!(xb.picked.as_deref(), Some("aac"));
    assert!(xb.unknown);
}

#[test]
fn a_device_never_connected_shows_sbc_alone_until_every_codec_is_shown() {
    let snap = snapshot();
    let new = cards(&snap, NEW, true);
    assert_eq!(ids(&new), ["sbc"]);
    // Every device has it: not "Unconfirmed"
    assert_eq!(new.cards[0].support, None);
    let all = cards(&snap, NEW, false);
    assert!(ids(&all).contains(&"aac") && ids(&all).contains(&"ldac"));
    // Opus 05 only in developer mode
    assert_eq!(all.cards.len(), CARD_ORDER.len() - 1);
    assert!(!ids(&all).contains(&"opus05"));
}

#[test]
fn a_card_chosen_saves_the_codec_a_card_the_device_does_not_offer_asked_for_anyway() {
    let snap = snapshot();
    let wh = entry(&snap, XM5);
    let shown = shown_profile(&snap, &wh);
    assert!(forced_for(&snap, &wh, &shown, Some("ldac"), "aptxhd"));
    assert!(!forced_for(&snap, &wh, &shown, Some("ldac"), "aac"));
    let p = chosen_profile(&shown, false, "aptxhd", None, None, true);
    assert_eq!((p.codec.as_str(), p.codec_forced), ("aptxhd", true));
    let p = chosen_profile(&p, false, "aac", None, None, false);
    assert_eq!((p.codec.as_str(), p.codec_forced), ("aac", false));
    // Auto is never forced
    assert!(!chosen_profile(&shown, false, "auto", None, None, true).codec_forced);
    // A forced one stays shown chosen
    let mut forced = snap.clone();
    set_profile(&mut forced, XM5, |p| {
        p.codec = "aptxhd".into();
        p.codec_forced = true;
    });
    assert_eq!(picked_codec(&forced, &entry(&forced, XM5), false, &CARD_ORDER).as_deref(), Some("aptxhd"));
    // An order dragged but not saved yet goes with the choice
    let order = placed(&card_order::<&str>(&[]), "aac", "ldac");
    let p = chosen_profile(&shown, false, "aac", None, Some(&order), false);
    let at = |c: &str| p.codec_order.iter().position(|x| x == c);
    assert!(at("aac").is_some() && at("aac") < at("ldac"));
    // An LE Audio entry's profile is not a codec's
    assert_eq!(chosen_profile(&shown, true, "aac", None, None, false), shown);
}

#[test]
fn auto_select_shows_what_auto_picks_the_codec_chosen_back_once_off() {
    let mut snap = snapshot();
    set_profile(&mut snap, XM5, |p| p.codec = "aac".into());
    let wh = entry(&snap, XM5);
    // The WH-1000XM5 supported SBC, AAC and LDAC: Auto picks LDAC
    assert_eq!(picked_codec(&snap, &wh, true, &CARD_ORDER).as_deref(), Some("ldac"));
    assert_eq!(picked_codec(&snap, &wh, false, &CARD_ORDER).as_deref(), Some("aac"));
    // Streaming: the codec it streams with
    streaming(&mut snap, "WH-1000XM5 (LDAC)", "AAC");
    assert_eq!(picked_codec(&snap, &entry(&snap, XM5), true, &CARD_ORDER).as_deref(), Some("aac"));
}

#[test]
fn keeps_ldac_choosable_on_earbuds_that_list_it_only_once_asked() {
    let mut snap = snapshot();
    let buds = cards(&snap, HUAWEI, true);
    let ldac = buds.cards.iter().find(|c| c.id == "ldac").expect("LDAC shown");
    assert_eq!(ldac.support, Some(CodecSupport::Unknown));
    assert!(!ids(&buds).contains(&"aptxhd"));
    snap.profiles.push(Profile {
        name: "HUAWEI FreeBuds Pro 4 | LDAC".into(),
        device_address: HUAWEI.into(),
        device_name: "HUAWEI FreeBuds Pro 4".into(),
        codec: "ldac".into(),
        ..sample_profile()
    });
    // LDAC itself, not the best codec they listed
    assert_eq!(picked_codec(&snap, &entry(&snap, HUAWEI), false, &CARD_ORDER).as_deref(), Some("ldac"));
}

#[test]
fn shows_pipewires_opus_only_in_developer_mode_one_chosen_falls_back_without_it() {
    let mut snap = snapshot();
    snap.devices[0].codecs.push("opus05".into());
    set_profile(&mut snap, XM5, |p| p.codec = "opus05".into());
    let off = cards(&snap, XM5, false);
    assert!(!ids(&off).contains(&"opus05"));
    assert_eq!(off.picked.as_deref(), Some("ldac"));
    snap.settings.developer_mode = true;
    let on = cards(&snap, XM5, false);
    assert!(ids(&on).contains(&"opus05"));
    assert_eq!(on.picked.as_deref(), Some("opus05"));
}

// ---- codec cards ----

#[test]
fn show_what_the_device_offered_when_it_last_connected() {
    let snap = snapshot();
    let all = cards(&snap, XM5, false);
    let tag = |v: &DeviceCardsView, id: &str| v.cards.iter().find(|c| c.id == id).and_then(|c| c.support);
    assert_eq!(tag(&all, "ldac"), Some(CodecSupport::Offered));
    assert_eq!(tag(&all, "aptxhd"), Some(CodecSupport::NotOffered));
    assert!(all.cards.iter().find(|c| c.id == "ldac").unwrap().selected);
    // Not known before the first connection: SBC supported, SBC XQ not known
    let xb = cards(&snap, XB13, false);
    assert_eq!(tag(&xb, "aac"), Some(CodecSupport::Unknown));
    assert_eq!(tag(&xb, "sbc"), Some(CodecSupport::Offered));
    assert_eq!(tag(&xb, "sbcxq"), Some(CodecSupport::Unknown));
    assert_eq!(tag(&cards(&snap, XB13, true), "sbc"), None);
}

#[test]
fn are_put_in_the_devices_own_order_kept_in_its_profile() {
    let snap = snapshot();
    let wh = cards(&snap, XM5, true);
    // AAC before LDAC, the first place
    let (order, place) = moved_by(&wh.order, &ids(&wh), "aac", -1).expect("moved");
    assert_eq!(place, 1);
    let keys = order_keys(&order);
    assert!(keys.iter().position(|c| c == "aac") < keys.iter().position(|c| c == "ldac"));
    // Every card once, Opus 05 and FastStream too
    let mut sorted = keys.clone();
    sorted.sort();
    let mut all: Vec<String> = CARD_ORDER.iter().map(|c| (*c).to_owned()).collect();
    all.sort();
    assert_eq!(sorted, all);
    let moved = device_cards(&snap, &entry(&snap, XM5), true, false, Some(&order));
    assert_eq!(ids(&moved), ["aac", "ldac", "sbc"]);
    assert!(moved.reset);
    // No further than the first place
    assert!(moved_by(&order, &ids(&moved), "aac", -1).is_none());
    // Back after it: the default order of the cards shown, saved as none
    let (back, _) = moved_by(&order, &ids(&moved), "aac", 1).expect("moved");
    assert!(shown_in_default_order(&back, &["ldac", "aac", "sbc"]));
    assert!(order_keys(&CARD_ORDER).is_empty());
    // Dragged with the pointer: SBC put where LDAC is
    let dragged = placed(&wh.order, "sbc", "ldac");
    assert_eq!(ids(&device_cards(&snap, &entry(&snap, XM5), true, false, Some(&dragged))), ["sbc", "ldac", "aac"]);
}

#[test]
fn keep_the_devices_order_under_auto_select_which_follows_it() {
    let mut snap = snapshot();
    set_profile(&mut snap, XM5, |p| {
        p.codec = "auto".into();
        p.codec_order = vec!["sbc".into(), "aac".into()];
    });
    // Auto-select off, no codec chosen: Auto picks the first card offered
    let off = cards(&snap, XM5, true);
    assert_eq!(ids(&off), ["sbc", "aac", "ldac"]);
    assert_eq!(off.picked.as_deref(), Some("sbc"));
    assert!(off.reset);
    // On: the same order, the same pick
    let on = device_cards(&snap, &entry(&snap, XM5), true, true, None);
    assert_eq!(ids(&on), ["sbc", "aac", "ldac"]);
    assert_eq!(on.picked.as_deref(), Some("sbc"));
    // LDAC dragged first: Auto picks it
    let dragged = placed(&on.order, "ldac", "sbc");
    assert_eq!(device_cards(&snap, &entry(&snap, XM5), true, true, Some(&dragged)).picked.as_deref(), Some("ldac"));
}

// ---- the device's activity ----

#[test]
fn waits_while_the_stream_starts_again_and_names_the_codec_streaming() {
    let mut snap = snapshot();
    assert!(!restarting(Some(&snap)));
    snap.stream.state = StreamState::Reconnecting;
    assert!(restarting(Some(&snap)));
    assert!(!restarting(None));
    streaming(&mut snap, "WH-1000XM5 (LDAC)", "LDAC");
    assert_eq!(streaming_codec(&snap, &entry(&snap, XM5)), Some("ldac"));
    assert_eq!(streaming_codec(&snap, &entry(&snap, XB13)), None);
    // aptX Adaptive LL's card while aptX Adaptive streams as chosen in Low
    // Latency, not under Auto-select
    set_profile(&mut snap, XM5, |p| {
        p.codec = "aptxad".into();
        p.quality = "ll".into();
    });
    streaming(&mut snap, "WH-1000XM5 (LDAC)", "aptX Adaptive");
    assert_eq!(streaming_codec(&snap, &entry(&snap, XM5)), Some("aptxadll"));
    snap.settings.auto_codec = true;
    assert_eq!(streaming_codec(&snap, &entry(&snap, XM5)), Some("aptxad"));
}

#[test]
fn saves_the_codec_in_use_with_the_slider_alone_without_starting_again() {
    let p = Profile { codec: "sbc".into(), ..sample_profile() };
    let mut v = CodecChoice::from(&p);
    v.sbc_bitpool = 40;
    v.sbc_min_bitpool = 10;
    assert!(only_bitpools(&p, &v));
    v.abr = true;
    assert!(!only_bitpools(&p, &v));
}

// ---- codec settings ----

fn props<'a>(snap: &'a Snapshot, device: &'a Device, sbc: Option<&'a ipc::SbcDeviceCaps>) -> SettingsProps<'a> {
    SettingsProps {
        caps: &device.codec_caps,
        sbc_caps: sbc,
        developer: snap.settings.developer_mode,
        match_source: false,
        source_format: None,
        device_name: &device.name,
    }
}

fn device<'a>(snap: &'a Snapshot, address: &str) -> &'a Device {
    snap.devices.iter().find(|d| d.address == address).expect("a device")
}

/// A device's codec dialog as it opens
fn open(snap: &Snapshot, address: &str, card: &str) -> CodecChoice {
    let e = entry(snap, address);
    let shown = shown_profile(snap, &e);
    let d = device(snap, address);
    settings_start(card, &shown, crate::profile::kept_for(&shown, card), &props(snap, d, d.sbc.as_ref()))
}

fn labels<T>(f: &Field<T>) -> Vec<String> {
    match f {
        Field::Fixed(text) => vec![text.clone()],
        Field::Choice { options, .. } => options.iter().map(|o| o.label.clone()).collect(),
    }
}

fn fixed<T>(f: &Field<T>) -> Option<&str> {
    match f {
        Field::Fixed(text) => Some(text),
        Field::Choice { .. } => None,
    }
}

#[test]
fn offers_what_the_device_does_not_list_once_its_supported_options_are_not_the_only_ones() {
    let snap = snapshot();
    let d = device(&snap, MOTO);
    let p = props(&snap, d, d.sbc.as_ref());
    let mut v = open(&snap, MOTO, "lhdcv5");
    let view = settings_view(&v, &p);
    assert_eq!(view.listed, Some((t!("ui.profile.codec_options_listed", "moto buds+"), true)));
    let not_96 = t!("ui.profile.sbc_not_offered", "96000 Hz");
    assert!(!labels(&view.rate).contains(&not_96));
    assert!(apply_change(&mut v, Change::Unlisted(true), &p));
    assert!(v.unlisted_options);
    assert!(labels(&settings_view(&v, &p).rate).contains(&not_96));
    // 96 kHz chosen, then only what it lists again: back to Auto
    apply_change(&mut v, Change::SampleRate(96000), &p);
    apply_change(&mut v, Change::Unlisted(false), &p);
    assert_eq!((v.unlisted_options, v.sample_rate), (false, 0));
    assert!(!labels(&settings_view(&v, &p).rate).contains(&not_96));
}

#[test]
fn lhdc_v5_asks_for_hi_res_again_in_developer_mode_only() {
    let mut snap = snapshot();
    let d = device(&snap, MOTO).clone();
    let v = open(&snap, MOTO, "lhdcv5");
    assert!(!settings_view(&v, &props(&snap, &d, d.sbc.as_ref())).ask_hi_res);
    snap.settings.developer_mode = true;
    assert!(settings_view(&v, &props(&snap, &d, d.sbc.as_ref())).ask_hi_res);
    // Nothing reported any more: neither the button nor the switch
    let forgot = Device { codec_caps: Vec::new(), sbc: None, ..d.clone() };
    let view = settings_view(&v, &props(&snap, &forgot, None));
    assert!(!view.ask_hi_res && view.listed.is_none());
    // Other codecs keep what the device reported
    let aac = open(&snap, MOTO, "aac");
    assert!(!settings_view(&aac, &props(&snap, &d, d.sbc.as_ref())).ask_hi_res);
}

#[test]
fn lhdc_v5_offers_192_khz_and_greys_out_the_quality_while_abr_is_on() {
    let snap = snapshot();
    let d = device(&snap, XB13);
    let p = props(&snap, d, None);
    let mut v = open(&snap, XB13, "lhdcv5");
    assert!(labels(&settings_view(&v, &p).rate).iter().any(|l| l.contains("192")));
    let disabled = |v: &CodecChoice| match settings_view(v, &p).quality {
        Field::Choice { disabled, .. } => disabled,
        Field::Fixed(_) => panic!("a choice"),
    };
    assert!(!disabled(&v));
    apply_change(&mut v, Change::Abr(true), &p);
    assert!(disabled(&v));
    // LDAC's ABR keeps the quality, and has no 192 kHz
    let mut ldac = open(&snap, XB13, "ldac");
    apply_change(&mut ldac, Change::Abr(true), &p);
    assert!(!disabled(&ldac));
    assert!(!labels(&settings_view(&ldac, &p).rate).iter().any(|l| l.contains("192")));
    // Unknown codecs listed: no "Show only supported options"
    assert!(settings_view(&v, &p).listed.is_none());
}

#[test]
fn opening_alone_changes_nothing() {
    let snap = snapshot();
    let v = open(&snap, XM5, "ldac");
    let p = sample_profile();
    assert_eq!(v, CodecChoice::from(&p));
    let d = device(&snap, XM5);
    assert_eq!(
        settings_view(&v, &props(&snap, d, d.sbc.as_ref())).listed,
        Some((t!("ui.profile.codec_options_listed", "WH-1000XM5"), true))
    );
}

#[test]
fn matching_the_pcs_output_auto_shows_the_listed_rate_nearest_it() {
    let mut snap = snapshot();
    snap.settings.developer_mode = true;
    let d = device(&snap, MOTO);
    let v = open(&snap, MOTO, "lhdcv5");
    let p = SettingsProps { match_source: true, source_format: Some((44100, 16)), ..props(&snap, d, d.sbc.as_ref()) };
    assert_eq!(labels(&settings_view(&v, &p).rate)[0], t!("codec.auto_value", "44100 Hz"));
    // The format not known yet: Auto alone
    let p = SettingsProps { source_format: None, ..p };
    assert_eq!(labels(&settings_view(&v, &p).rate)[0], t("codec.auto"));
}

#[test]
fn lhdc_v5_offers_only_the_rates_the_device_lists() {
    let snap = snapshot();
    let d = device(&snap, MOTO);
    let view = settings_view(&open(&snap, MOTO, "lhdcv5"), &props(&snap, d, d.sbc.as_ref()));
    assert_eq!(labels(&view.rate), [t!("codec.auto_value", "48000 Hz"), "44100 Hz".into(), "48000 Hz".into()]);
    // High, Medium-High (issue #203), Medium, Low (issue #86)
    assert_eq!(
        labels(&view.quality),
        [
            format!("{} (1000 kbps)", t("quality.high")),
            format!("{} (900 kbps)", t("quality.medium_high")),
            format!("{} (500 kbps)", t("quality.medium")),
            format!("{} (400 kbps)", t("quality.low")),
        ]
    );
    // The device does not list low latency: no switch
    assert_eq!(view.low_latency, None);
}

#[test]
fn aac_shows_its_one_quality_at_the_devices_largest_bit_rate() {
    let snap = snapshot();
    let d = device(&snap, MOTO);
    let view = settings_view(&open(&snap, MOTO, "aac"), &props(&snap, d, d.sbc.as_ref()));
    assert_eq!(fixed(&view.quality), Some(format!("{} (131 kbps)", t("quality.high")).as_str()));
    assert_eq!(view.vbr, Some(false));
}

#[test]
fn sbcs_bitpool_stops_at_the_devices_largest_and_says_so() {
    let snap = snapshot();
    let d = device(&snap, MOTO);
    let p = props(&snap, d, d.sbc.as_ref());
    let mut v = open(&snap, MOTO, "sbc");
    let view = settings_view(&v, &p);
    let bp = view.bitpool.expect("a bitpool slider");
    // Auto: where the quality puts it, the device's largest here
    assert_eq!((bp.max, bp.high), (39, 39));
    assert_eq!(bp.high_text, t!("ui.profile.sbc_bitpool_auto", 39));
    assert_eq!(view.channel_mode, Some(t("ui.diag.cd_mode.joint_stereo")));
    // Never past the device's largest
    assert!(apply_change(&mut v, Change::HighBitpool(53), &p));
    assert_eq!(v.sbc_bitpool, 39);
    // Bitpool 39, 16 blocks, 8 subbands, joint stereo at 48 kHz
    assert_eq!(fixed(&settings_view(&v, &p).quality), Some("≈ 273 kbps"));
    assert_eq!((v.sbc_bitpool, v.sbc_min_bitpool), (39, 0));
}

#[test]
fn a_codec_with_one_bit_rate_shows_it_at_the_sample_rate_chosen() {
    let snap = snapshot();
    let d = device(&snap, XM5);
    let p = props(&snap, d, d.sbc.as_ref());
    let mut v = open(&snap, XM5, "aptxhd");
    assert_eq!(fixed(&settings_view(&v, &p).quality), Some("576 kbps"));
    apply_change(&mut v, Change::SampleRate(44100), &p);
    assert_eq!(fixed(&settings_view(&v, &p).quality), Some("529 kbps"));
}

#[test]
fn aptx_adaptive_offers_96_khz_only_to_a_device_that_lists_it() {
    let snap = snapshot();
    let rates = |address: &str| {
        let d = device(&snap, address);
        labels(&settings_view(&open(&snap, address, "aptxad"), &props(&snap, d, d.sbc.as_ref())).rate)
    };
    // One rate: shown fixed, not Auto and that rate to choose between
    assert_eq!(rates(QC), ["48000 Hz"]);
    assert_eq!(rates(EARFUN), [t!("codec.auto_value", "48000 Hz"), "48000 Hz".into(), "96000 Hz".into()]);
    let d = device(&snap, QC);
    assert!(matches!(settings_view(&open(&snap, QC, "aptxad"), &props(&snap, d, None)).rate, Field::Fixed(_)));
    assert_eq!(rates(XB13), ["48000 Hz"]);
}

#[test]
fn aptx_adaptive_ll_is_a_card_of_its_own() {
    let snap = snapshot();
    let earfun = cards(&snap, EARFUN, true);
    let at = ids(&earfun).iter().position(|c| *c == "aptxad").expect("aptX Adaptive");
    assert_eq!(ids(&earfun)[at + 1], "aptxadll");
    let d = device(&snap, EARFUN);
    let p = props(&snap, d, None);
    // aptX Adaptive's own settings: High only (no Low Latency in it)
    let ad = settings_view(&open(&snap, EARFUN, "aptxad"), &p);
    assert_eq!(fixed(&ad.quality), Some(format!("{} (427 kbps)", t("quality.high")).as_str()));
    // LL's: its one bit rate, 48 kHz, 24-bit, no ABR
    let ll = settings_view(&open(&snap, EARFUN, "aptxadll"), &p);
    assert_eq!(fixed(&ll.quality), Some("265 kbps"));
    assert_eq!(fixed(&ll.rate), Some("48000 Hz"));
    assert_eq!(fixed(&ll.depth), Some(t("bitdepth.24").as_str()));
    assert_eq!(ll.abr, None);
}

#[test]
fn sbc_and_sbc_xq_keep_their_own_settings() {
    let snap = snapshot();
    let d = device(&snap, XB13);
    let p = props(&snap, d, None);
    let mut xq = open(&snap, XB13, "sbcxq");
    apply_change(&mut xq, Change::HighBitpool(35), &p);
    assert_eq!((xq.sbcxq_bitpool, xq.sbc_bitpool), (35, 0));
    // A profile on SBC XQ kept for SBC: SBC's dialog starts from its own
    let mut profile = Profile { codec: "sbcxq".into(), sbcxq_bitpool: 35, ..sample_profile() };
    profile
        .codec_settings
        .insert("sbc".into(), CodecValues { sbc_bitpool: 40, sample_rate: 44100, ..CodecValues::default() });
    let sbc = settings_start("sbc", &profile, crate::profile::kept_for(&profile, "sbc"), &p);
    assert_eq!((sbc.sbc_bitpool, sbc.sample_rate, sbc.sbcxq_bitpool), (40, 44100, 0));
}

// ---- SBC settings ----

#[test]
fn developer_mode_offers_the_frame_setup_the_device_listed() {
    let mut snap = snapshot();
    snap.settings.developer_mode = true;
    let d = device(&snap, XM5).clone();
    let p = props(&snap, &d, d.sbc.as_ref());
    let mut v = open(&snap, XM5, "sbc");
    let view = settings_view(&v, &p);
    assert_eq!(fixed(&view.quality), Some("≈ 357 kbps"));
    let frame = |view: &SettingsView, key: FrameKey| view.frame.iter().find(|f| f.key == key).map(|f| f.field.clone());
    // The WH-1000XM5 lists 8 subbands and loudness only: each shown fixed
    assert_eq!(frame(&view, FrameKey::Subbands).as_ref().and_then(fixed), Some("8"));
    assert_eq!(
        frame(&view, FrameKey::Allocation).as_ref().and_then(fixed),
        Some(t("ui.diag.cd_alloc.loudness").as_str())
    );
    assert_eq!(labels(&frame(&view, FrameKey::Blocks).unwrap()).len(), 5);
    // Dual channel is SBC XQ
    let mut modes = vec![t!("codec.auto_value", t("ui.diag.cd_mode.joint_stereo"))];
    modes.extend(["joint_stereo", "stereo", "mono"].map(|m| t(&format!("ui.diag.cd_mode.{m}"))));
    assert_eq!(labels(&frame(&view, FrameKey::ChannelMode).unwrap()), modes);
    assert_eq!(view.channel_mode, None);
    // 8 blocks: shorter frames, a higher rate
    apply_change(&mut v, frame_change(FrameKey::Blocks, "8"), &p);
    assert_eq!(fixed(&settings_view(&v, &p).quality), Some("≈ 396 kbps"));
    // Stereo at 44.1 kHz with bitpool 40
    apply_change(&mut v, frame_change(FrameKey::ChannelMode, "stereo"), &p);
    apply_change(&mut v, Change::SampleRate(44100), &p);
    apply_change(&mut v, Change::HighBitpool(40), &p);
    assert_eq!(fixed(&settings_view(&v, &p).quality), Some("≈ 286 kbps"));
    // Out of developer mode it no longer streams: the fixed frame's bit rate
    let p = SettingsProps { developer: false, ..p };
    let view = settings_view(&v, &p);
    assert_eq!(fixed(&view.quality), Some("≈ 256 kbps"));
    assert!(view.frame.is_empty());
    assert_eq!(view.channel_mode, Some(t("ui.diag.cd_mode.joint_stereo")));
    // ... and is kept
    assert_eq!((v.sbc_blocks, v.sbc_subbands, v.sbc_allocation.as_str()), (8, 0, ""));
    assert_eq!((v.sbc_channel_mode.as_str(), v.sbc_bitpool), ("stereo", 40));
}

#[test]
fn sbc_xqs_channel_mode_stays_fixed_in_developer_mode() {
    let mut snap = snapshot();
    snap.settings.developer_mode = true;
    let d = device(&snap, XM5).clone();
    let view = settings_view(&open(&snap, XM5, "sbcxq"), &props(&snap, &d, d.sbc.as_ref()));
    assert!(view.frame.iter().any(|f| f.key == FrameKey::Blocks));
    assert!(!view.frame.iter().any(|f| f.key == FrameKey::ChannelMode));
    assert_eq!(view.channel_mode, Some(t("ui.diag.cd_mode.dual_channel")));
    // One bit depth: a value, not a list (issue #86)
    assert_eq!(fixed(&view.depth), Some(t("bitdepth.16").as_str()));
}

#[test]
fn sbc_offers_what_the_device_listed_and_shows_the_bit_rate_as_it_changes() {
    let snap = snapshot();
    let d = device(&snap, XM5);
    let p = props(&snap, d, d.sbc.as_ref());
    let mut v = open(&snap, XM5, "sbc");
    let view = settings_view(&v, &p);
    assert_eq!(fixed(&view.quality), Some("≈ 357 kbps"));
    // The frame itself is not a choice outside developer mode (issue #86)
    assert!(view.frame.is_empty());
    apply_change(&mut v, Change::SampleRate(44100), &p);
    apply_change(&mut v, Change::HighBitpool(40), &p);
    assert_eq!(fixed(&settings_view(&v, &p).quality), Some("≈ 256 kbps"));
    assert_eq!((v.sbc_bitpool, v.sample_rate), (40, 44100));
}

#[test]
fn one_slider_with_two_handles_sets_the_lowest_and_the_highest_bitpool() {
    let snap = snapshot();
    let d = device(&snap, XM5);
    let p = props(&snap, d, d.sbc.as_ref());
    let mut v = open(&snap, XM5, "sbc");
    let bp = |v: &CodecChoice| settings_view(v, &p).bitpool.expect("a slider");
    // Auto: the lowest at the start, the highest where High puts it (53)
    let b = bp(&v);
    assert_eq!((b.min, b.max, b.low, b.high), (2, 53, 2, 53));
    assert_eq!(b.low_text, t!("ui.profile.sbc_bitpool_auto", 2));
    assert_eq!(b.high_text, t!("ui.profile.sbc_bitpool_auto", 53));
    assert!(b.auto);
    // The highest down to 30: the bit rate follows it
    apply_change(&mut v, Change::HighBitpool(30), &p);
    assert_eq!(bp(&v).high_text, "30");
    assert_eq!(fixed(&settings_view(&v, &p).quality), Some("≈ 219 kbps"));
    assert!(!bp(&v).auto);
    // The lowest never passes the highest, nor the highest the lowest
    apply_change(&mut v, Change::LowBitpool(40), &p);
    assert_eq!(bp(&v).low, 30);
    apply_change(&mut v, Change::LowBitpool(20), &p);
    assert_eq!((bp(&v).low, bp(&v).low_text.as_str()), (20, "20"));
    apply_change(&mut v, Change::HighBitpool(10), &p);
    assert_eq!(bp(&v).high, 20);
    apply_change(&mut v, Change::HighBitpool(30), &p);
    assert_eq!((v.sbc_min_bitpool, v.sbc_bitpool, v.sbcxq_min_bitpool), (20, 30, 0));
    // Auto puts both back
    apply_change(&mut v, Change::BitpoolAuto, &p);
    let b = bp(&v);
    assert_eq!(
        (b.low_text, b.high_text),
        (t!("ui.profile.sbc_bitpool_auto", 2), t!("ui.profile.sbc_bitpool_auto", 53))
    );
    assert_eq!((v.sbc_min_bitpool, v.sbc_bitpool), (0, 0));
}

#[test]
fn sbc_xq_keeps_its_own_minimum_within_its_own_range() {
    let snap = snapshot();
    let d = device(&snap, XM5);
    let p = props(&snap, d, d.sbc.as_ref());
    let mut v = open(&snap, XM5, "sbcxq");
    let view = settings_view(&v, &p);
    // Auto at 48 kHz: 43 per channel
    assert_eq!(view.bitpool.as_ref().unwrap().high_text, t!("ui.profile.sbc_bitpool_auto", 43));
    assert_eq!(view.channel_mode, Some(t("ui.diag.cd_mode.dual_channel")));
    // With the highest on Auto, the lowest stops where Auto's highest is
    apply_change(&mut v, Change::LowBitpool(47), &p);
    assert_eq!(v.sbcxq_min_bitpool, 43);
    apply_change(&mut v, Change::LowBitpool(35), &p);
    assert_eq!((v.sbcxq_min_bitpool, v.sbcxq_bitpool, v.sbc_min_bitpool), (35, 0, 0));
}

#[test]
fn sbc_xqs_highest_bitpool_follows_the_sample_rate() {
    let snap = snapshot();
    let d = device(&snap, XM5);
    let p = props(&snap, d, d.sbc.as_ref());
    let mut v = open(&snap, XM5, "sbcxq");
    // Its bit rate at its own bitpool: 43 per channel at 48 kHz
    assert_eq!(fixed(&settings_view(&v, &p).quality), Some("≈ 552 kbps"));
    assert_eq!(settings_view(&v, &p).bitpool.unwrap().max, 43);
    apply_change(&mut v, Change::SampleRate(44100), &p);
    assert_eq!(settings_view(&v, &p).bitpool.unwrap().max, 47);
    apply_change(&mut v, Change::HighBitpool(47), &p);
    assert_eq!(fixed(&settings_view(&v, &p).quality), Some("≈ 551 kbps"));
    // Back at 48 kHz the bitpool comes down with the range
    apply_change(&mut v, Change::SampleRate(48000), &p);
    assert_eq!((v.sbcxq_bitpool, settings_view(&v, &p).bitpool.unwrap().high), (43, 43));
}

#[test]
fn abr_does_not_carry_between_sbc_sbc_xq_and_ldac() {
    let snap = snapshot();
    let d = device(&snap, XB13);
    let p = props(&snap, d, None);
    let sbc = Profile { codec: "sbc".into(), abr: true, ..sample_profile() };
    assert!(!settings_start("sbcxq", &sbc, None, &p).abr);
    assert!(settings_start("sbc", &sbc, None, &p).abr);
    // LDAC's ABR does not carry over to SBC's dialog
    let ldac = Profile { abr: true, ..sample_profile() };
    let v = settings_start("sbc", &ldac, None, &p);
    assert!(!v.abr);
    assert_eq!(settings_view(&v, &p).abr, Some(false));
    assert_eq!(settings_view(&settings_start("sbcxq", &ldac, None, &p), &p).abr, Some(false));
}

#[test]
fn the_bitpool_range_goes_up_to_sbcs_53_when_not_held_to_the_devices_list() {
    // The SBC XQ bitpool range
    assert_eq!(bitpool_range("sbcxq", None, 48000, false), BitpoolRange { min: 2, max: 43 });
    assert_eq!(bitpool_range("sbcxq", None, 48000, true), BitpoolRange { min: 2, max: 53 });
    assert_eq!(bitpool_range("sbc", None, 48000, true), BitpoolRange { min: 2, max: 53 });
    let capped = ipc::SbcDeviceCaps { min_bitpool: 10, max_bitpool: 39, ..xm5_sbc() };
    assert_eq!(bitpool_range("sbc", Some(&capped), 48000, false), BitpoolRange { min: 10, max: 39 });
}

// ---- the microphone's codec (headset microphone, LE Audio) ----

#[test]
fn call_codec_cards_show_those_the_headphones_list_tagged_with_every_one_shown() {
    let listed = vec!["msbc".to_owned(), "cvsd".to_owned()];
    let shown: Vec<&str> = call_cards(&listed, Some("msbc"), true).iter().map(|(c, _)| c.id).collect();
    assert_eq!(shown, ["msbc", "cvsd"]);
    assert!(call_cards(&listed, Some("msbc"), true).iter().all(|(_, tag)| tag.is_none()));
    let all = call_cards(&listed, Some("msbc"), false);
    let tags: Vec<(&str, Option<CodecSupport>)> = all.iter().map(|(c, tag)| (c.id, *tag)).collect();
    assert_eq!(
        tags,
        [
            ("lc3_swb", Some(CodecSupport::NotOffered)),
            ("msbc", Some(CodecSupport::Offered)),
            ("cvsd", Some(CodecSupport::Offered)),
        ]
    );
    // CVSD needs no listing: every headset has it
    assert_eq!(call_cards(&[], None, true).iter().map(|(c, _)| c.id).collect::<Vec<_>>(), ["cvsd"]);
    // "..." shows its settings, read only
    assert_eq!(fixed_values(&VOICE_CARDS[1], "msbc", None), ("16000 Hz".into(), "64 kbps".into()));
}

#[test]
fn the_call_link_shows_on_the_device_streaming_only() {
    let mut snap = snapshot();
    assert_eq!(mic_view(&snap, &entry(&snap, XM5)), MicView::default());
    streaming(&mut snap, "WH-1000XM5 (LDAC)", "LDAC");
    snap.stream.voice_codecs = vec!["msbc".into(), "cvsd".into()];
    snap.stream.voice =
        Some(ipc::VoiceStatus { codec: "msbc".into(), link: "hfp".into(), rate: 16000, mic_device_id: String::new() });
    let mic = mic_view(&snap, &entry(&snap, XM5));
    assert_eq!(mic.call_codec.as_deref(), Some("msbc"));
    assert!(mic.up(false));
    assert_eq!(mic.listed_calls, ["msbc", "cvsd"]);
    // Not the other device's
    assert!(!mic_view(&snap, &entry(&snap, XB13)).ours);
}

#[test]
fn the_le_audio_microphones_rate_is_chosen_among_those_the_headphones_allow() {
    let options = mic_rate_options(0, &[32000, 16000]).expect("a choice");
    let labels: Vec<&str> = options.iter().map(|o| o.label.as_str()).collect();
    assert_eq!(labels, [t!("codec.auto_value", "32000 Hz").as_str(), "32000 Hz", "16000 Hz"]);
    // One chosen they no longer allow stays, marked
    let options = mic_rate_options(24000, &[32000, 16000]).expect("a choice");
    assert_eq!(options.last().map(|o| o.label.clone()), Some(t!("ui.profile.sbc_not_offered", "24000 Hz")));
    // Nothing to choose: one rate, or none known
    assert!(mic_rate_options(0, &[32000]).is_none());
    // No bit rate shown, the rate the microphone runs at
    let mic = FixedCard { rate: Some(32000), ..LE_MIC_CARD };
    assert!(mic.no_bit_rate);
    assert_eq!(fixed_values(&mic, "lc3_mic", None).0, "32000 Hz");
}

#[test]
fn le_audio_has_lc3_and_its_low_latency_card_their_rates_the_streams() {
    assert_eq!(LE_CARDS.map(|c| c.id), ["lc3", "lc3_ll"]);
    assert_eq!(t(LE_CARDS[1].label), "LC3 LL");
    let info = info("LC3", 124);
    assert_eq!(fixed_values(&LE_CARDS[0], "lc3", Some(&info)), ("48000 Hz".into(), "124 kbps".into()));
    assert_eq!(fixed_values(&LE_CARDS[1], "lc3", Some(&info)), ("—".into(), "—".into()));
}
