//! profile.rs' tests: a device's profile, profiles, VB-Cable, the first-run
//! setup's sound and microphone, and the codecs a device offers

use std::collections::BTreeMap;

use ipc::{AudioDevice, CodecValues, Device, Profile, StreamState, StreamStatus};

use super::*;
use crate::codecs::card_of;
use crate::sbc::mock::{profile_with, sample_device, sample_profile};

fn of(p: &Profile) -> &str {
    card_of(&p.codec, &p.quality)
}

fn kept<'a>(p: &'a Profile, card: &str) -> &'a CodecValues {
    p.codec_settings.get(card).unwrap_or_else(|| panic!("{card} kept"))
}

// ---- a device's profile (Codecs screen, issue #86) ----

#[test]
fn keeps_the_settings_of_the_codec_it_leaves_and_brings_back_those_of_the_one_chosen() {
    let aac = Profile { abr: true, quality: "hq".into(), ..profile_with("aac") };
    let ldac = with_codec(&aac, "ldac", None);
    assert_eq!((ldac.codec.as_str(), ldac.abr), ("ldac", false));
    assert_eq!((kept(&ldac, "aac").abr, kept(&ldac, "aac").quality.as_str()), (true, "hq"));
    let back = with_codec(&ldac, "aac", None);
    assert_eq!((back.codec.as_str(), back.abr), ("aac", true));
    assert!(!back.codec_settings.contains_key("aac"));
    assert!(back.codec_settings.contains_key("ldac"));
    // The "..." dialog's values win over what was kept; a quality the codec
    // no longer offers is High (issue #86: AAC has High only)
    let v = CodecValues { quality: "sq".into(), abr: false, ..kept(&ldac, "aac").clone() };
    let sq = with_codec(&ldac, "aac", Some(&v));
    assert_eq!((sq.codec.as_str(), sq.quality.as_str(), sq.abr), ("aac", "hq", false));
    let v = CodecValues { quality: "sq".into(), ..kept(&ldac, "aac").clone() };
    let l = with_codec(&aac, "ldac", Some(&v));
    assert_eq!((l.codec.as_str(), l.quality.as_str()), ("ldac", "sq"));
    // Fitted to the codec: SBC's bitpool does not reach AAC
    assert_eq!(with_codec(&Profile { sbc_bitpool: 40, ..profile_with("sbc") }, "aac", None).sbc_bitpool, 0);
}

#[test]
fn saves_aptx_adaptive_lls_card_as_aptx_adaptive_in_low_latency_each_card_with_its_own_settings() {
    let hq = Profile { quality: "hq".into(), abr: true, sample_rate: 96000, ..profile_with("aptxad") };
    assert_eq!(of(&hq), "aptxad");
    let ll = with_codec(&hq, "aptxadll", None);
    // No ABR and no 96 kHz in Low Latency
    assert_eq!((ll.codec.as_str(), ll.quality.as_str(), ll.abr, ll.sample_rate), ("aptxad", "ll", false, 0));
    assert_eq!(of(&ll), "aptxadll");
    let k = kept(&ll, "aptxad");
    assert_eq!((k.quality.as_str(), k.abr, k.sample_rate), ("hq", true, 96000));
    // Back to aptX Adaptive: High with what it had, LL's kept apart
    let back = with_codec(&ll, "aptxad", None);
    assert_eq!((back.codec.as_str(), back.quality.as_str(), back.abr, back.sample_rate), ("aptxad", "hq", true, 96000));
    assert_eq!(kept(&back, "aptxadll").quality, "ll");
    assert!(!back.codec_settings.contains_key("aptxad"));
    // From another codec, and away to it
    let p = with_codec(&Profile { quality: "sq".into(), ..profile_with("ldac") }, "aptxadll", None);
    assert_eq!((p.codec.as_str(), p.quality.as_str()), ("aptxad", "ll"));
    let p = with_codec(&ll, "ldac", None);
    assert_eq!((p.codec.as_str(), p.quality.as_str()), ("ldac", "hq"));
    // A profile saved before the card (aptX Adaptive, Low Latency) is on it
    let old = Profile { quality: "ll".into(), ..profile_with("aptxad") };
    assert_eq!(of(&old), "aptxadll");
    let p = with_codec(&old, "aptxad", None);
    assert_eq!((p.codec.as_str(), p.quality.as_str()), ("aptxad", "hq"));
    // Low Latency kept for aptX Adaptive by an earlier version comes back on LL's card, not on its own
    let legacy = CodecValues { quality: "ll".into(), sample_rate: 48000, ..kept(&ll, "aptxad").clone() };
    let kept_old = Profile { codec_settings: BTreeMap::from([("aptxad".into(), legacy)]), ..profile_with("aac") };
    assert_eq!(kept_for(&kept_old, "aptxadll").map(|v| v.quality.as_str()), Some("ll"));
    let p = with_codec(&kept_old, "aptxadll", None);
    assert_eq!((p.codec.as_str(), p.quality.as_str(), p.sample_rate), ("aptxad", "ll", 48000));
    let p = with_codec(&kept_old, "aptxad", None);
    assert_eq!((p.codec.as_str(), p.quality.as_str()), ("aptxad", "hq"));
    // ... never as aptX Adaptive's own: they move to LL's card
    assert!(kept_for(&kept_old, "aptxad").is_none());
    let k = kept(&p, "aptxadll");
    assert_eq!((k.quality.as_str(), k.sample_rate), ("ll", 48000));
    assert!(!p.codec_settings.contains_key("aptxad"));
    let via_ll = with_codec(&with_codec(&kept_old, "aptxadll", None), "aptxad", None);
    assert_eq!((via_ll.codec.as_str(), via_ll.quality.as_str()), ("aptxad", "hq"));
    let k = kept(&via_ll, "aptxadll");
    assert_eq!((k.quality.as_str(), k.sample_rate), ("ll", 48000));
}

const NONE: &[&str] = &[];

#[test]
fn names_it_after_the_device_and_codec_unless_the_user_named_it() {
    let p = Profile { name: "Buds | Auto".into(), device_name: "Buds".into(), ..profile_with("aac") };
    assert_eq!(followed_name(&p, "AAC", NONE), "Buds | AAC");
    assert_eq!(followed_name(&Profile { name: String::new(), ..p.clone() }, "AAC", &["Buds | AAC"]), "Buds | AAC (2)");
    assert_eq!(followed_name(&Profile { name: "Gaming".into(), ..p.clone() }, "AAC", NONE), "Gaming");
    assert_eq!(followed_name(&Profile { le_audio: true, name: String::new(), ..p }, "", NONE), "Buds LE Audio");
}

#[test]
fn is_the_one_streaming_else_the_active_one_else_the_first() {
    let named = |name: &str| Profile { name: name.into(), ..sample_profile() };
    let profiles = [named("A"), named("B"), Profile { le_audio: true, ..named("LE") }];
    let idle = StreamStatus { state: StreamState::Idle, ..Default::default() };
    let streaming = StreamStatus { state: StreamState::Streaming, profile: Some("A".into()), ..Default::default() };
    fn name(profiles: &[Profile], active: Option<&str>, stream: &StreamStatus, le: bool) -> Option<String> {
        let view = ProfilesView { profiles, active_profile: active, stream };
        device_profile(view, &profiles[0].device_address, le).map(|p| p.name.clone())
    }
    assert_eq!(name(&profiles, None, &idle, false).as_deref(), Some("A"));
    assert_eq!(name(&profiles, Some("B"), &idle, false).as_deref(), Some("B"));
    assert_eq!(name(&profiles, Some("B"), &streaming, false).as_deref(), Some("A"));
    assert_eq!(name(&profiles, Some("B"), &idle, true).as_deref(), Some("LE"));
}

// ---- profiles ----

#[test]
fn validates_before_saving() {
    let ok = sample_profile();
    assert!(validate_profile(&ok, NONE).is_empty());
    let key = |p: Option<Problem>| p.map(|p| p.key);
    assert_eq!(
        key(validate_profile(&Profile { name: "  ".into(), ..ok.clone() }, NONE).name),
        Some("ui.profile.err_name_empty")
    );
    assert_eq!(key(validate_profile(&ok, &[ok.name.as_str()]).name), Some("ui.profile.err_name_taken"));
    assert_eq!(
        key(validate_profile(&Profile { device_address: String::new(), ..ok.clone() }, NONE).device),
        Some("ui.profile.err_device")
    );
    assert_eq!(
        key(validate_profile(&Profile { device_address: "12:34".into(), ..ok.clone() }, NONE).device),
        Some("error.invalid_address_format")
    );
    assert_eq!(
        validate_profile(&Profile { max_media_payload: 600, ..ok.clone() }, NONE).packet.map(|p| p.args),
        Some(vec![Arg::Num(679.0), Arg::Num(1679.0)])
    );
    assert!(validate_profile(&Profile { max_media_payload: 1680, ..ok.clone() }, NONE).packet.is_some());
    assert_eq!(
        key(validate_profile(&Profile { capture_mode: "virtual".into(), ..ok.clone() }, NONE).audio_device),
        Some("error.no_virtual_device_selected")
    );
    assert_eq!(
        key(validate_profile(&Profile { capture_mode: "app".into(), app_exe: String::new(), ..ok }, NONE).app),
        Some("error.no_app_selected")
    );
}

// ---- VB-Cable, the first-run setup ----

fn device(name: &str, is_virtual: bool, is_default: bool) -> AudioDevice {
    AudioDevice {
        id: name.into(),
        name: name.into(),
        short_name: None,
        adapter_name: None,
        is_default,
        is_virtual,
        is_display: false,
        mix_rate: None,
        mix_channels: None,
        mix_bits: None,
        capture_name: None,
        capture_rate: None,
    }
}

fn cable(name: &str) -> AudioDevice {
    device(name, true, false)
}

#[test]
fn prefers_the_standard_cable_to_the_16_channel_one() {
    let list = [
        device("Speakers", false, false),
        cable("CABLE In 16ch (VB-Audio Virtual Cable)"),
        cable("CABLE Input (VB-Audio Virtual Cable)"),
    ];
    assert_eq!(vb_cable(&list).map(|d| d.name.as_str()), Some("CABLE Input (VB-Audio Virtual Cable)"));
}

#[test]
fn takes_the_16_channel_one_when_it_is_the_only_one() {
    assert!(vb_cable(&[cable("CABLE In 16ch (VB-Audio Virtual Cable)")]).unwrap().name.contains("16ch"));
    assert!(vb_cable(&[device("Speakers", false, false)]).is_none());
}

#[test]
fn leaves_out_the_default_output_another_devices_stream_switched_to() {
    // issue #86: EarFun's Output Source, the default while it streams,
    // locked that cable out of Moto Buds' lists
    let list = [
        device("Speakers", false, false),
        AudioDevice { is_default: true, ..cable("Hi-Fi Cable Input (VB-Audio Hi-Fi Cable)") },
    ];
    let earfun = Profile {
        name: "EarFun".into(),
        capture_mode: "virtual".into(),
        audio_device_id: list[1].id.clone(),
        ..sample_profile()
    };
    let out = |state: StreamState, p: &Profile| {
        let stream = StreamStatus { state, profile: Some("EarFun".into()), ..Default::default() };
        let profiles = [p.clone()];
        own_default_output(&list, ProfilesView { profiles: &profiles, active_profile: None, stream: &stream })
    };
    assert_eq!(out(StreamState::Streaming, &earfun), "");
    assert_eq!(out(StreamState::Idle, &earfun), list[1].id);
    assert_eq!(out(StreamState::Streaming, &Profile { capture_mode: "loopback".into(), ..earfun.clone() }), list[1].id);
}

mod setup {
    use super::*;

    fn speakers() -> AudioDevice {
        device("Speakers (Realtek(R) Audio)", false, true)
    }
    fn hdmi() -> AudioDevice {
        AudioDevice { is_display: true, ..device("LG TV (NVIDIA High Definition Audio)", false, false) }
    }
    fn vb() -> AudioDevice {
        cable("CABLE Input (VB-Audio Virtual Cable)")
    }
    fn hifi() -> AudioDevice {
        cable("Hi-Fi Cable Input (VB-Audio Hi-Fi Cable)")
    }
    fn not_default(d: AudioDevice) -> AudioDevice {
        AudioDevice { is_default: false, ..d }
    }
    fn default(d: AudioDevice) -> AudioDevice {
        AudioDevice { is_default: true, ..d }
    }

    fn names(list: &[AudioDevice]) -> (Option<String>, Option<String>) {
        let a = setup_audio(list);
        (a.sound.map(|d| d.name.clone()), a.mic.map(|d| d.name.clone()))
    }

    fn both(sound: AudioDevice, mic: AudioDevice) -> (Option<String>, Option<String>) {
        (Some(sound.name), Some(mic.name))
    }

    #[test]
    fn both_cables_the_hifi_cable_for_the_sound_vb_cable_for_the_microphone() {
        assert_eq!(names(&[speakers(), vb(), hifi()]), both(hifi(), vb()));
    }

    #[test]
    fn one_cable_the_microphone_the_sound_from_the_sound_card_or_hdmi_whichever_is_windows_default() {
        assert_eq!(names(&[speakers(), vb()]), both(speakers(), vb()));
        assert_eq!(names(&[speakers(), hifi()]), both(speakers(), hifi()));
        assert_eq!(names(&[default(hdmi()), vb()]), both(hdmi(), vb()));
        // The #86 reports: VB-Cable the default, a monitor's HDMI, no sound card
        let list = [cable("CABLE In 16ch (VB-Audio Virtual Cable)"), default(vb()), hdmi()];
        assert_eq!(names(&list), both(hdmi(), vb()));
        assert_eq!(names(&[not_default(speakers()), default(hifi())]), both(speakers(), hifi()));
    }

    #[test]
    fn no_cable_a_sound_card_and_hdmi_the_sound_from_hdmi_the_microphone_into_the_sound_card() {
        assert_eq!(names(&[speakers(), hdmi()]), both(hdmi(), speakers()));
        assert_eq!(names(&[not_default(speakers()), default(hdmi())]), both(hdmi(), speakers()));
        // One of them alone: Windows default, no microphone
        assert_eq!(names(&[speakers()]), (None, None));
        assert_eq!(names(&[default(hdmi())]), (None, None));
    }

    #[test]
    fn one_cable_a_sound_card_and_hdmi_the_sound_from_hdmi() {
        assert_eq!(names(&[speakers(), hdmi(), vb()]), both(hdmi(), vb()));
        // Several: the one that is Windows' default output, else the first
        let monitor = AudioDevice { is_display: true, ..device("M28U (NVIDIA High Definition Audio)", false, false) };
        assert_eq!(names(&[speakers(), hdmi(), monitor.clone(), vb()]), both(hdmi(), vb()));
        assert_eq!(names(&[not_default(speakers()), hdmi(), default(monitor.clone()), vb()]), both(monitor, vb()));
        // No HDMI: Windows' default output before the first one
        let usb = device("Speakers (USB Audio)", false, false);
        assert_eq!(names(&[usb.clone(), speakers(), vb()]), both(speakers(), vb()));
        assert_eq!(names(&[usb.clone(), not_default(speakers()), default(vb())]), both(usb, vb()));
    }

    #[test]
    fn one_cable_that_is_windows_default_output_and_no_real_one_the_sound_no_microphone() {
        assert_eq!(names(&[default(vb())]), (Some(vb().name), None));
    }

    #[test]
    fn no_cable_and_no_hdmi_system_loopback_no_microphone() {
        assert_eq!(names(&[speakers(), cable("CABLE In 16ch (VB-Audio Virtual Cable)")]), (None, None));
    }
}

// ---- the codecs a device offers ----

fn dev(address: &str, codecs: &[&str]) -> Device {
    Device { saved: true, codecs: codecs.iter().map(|c| (*c).to_owned()).collect(), ..sample_device(address) }
}

fn audio(id: &str, is_default: bool, mix_rate: Option<u32>, mix_bits: Option<u16>) -> AudioDevice {
    AudioDevice { mix_rate, mix_channels: Some(2), mix_bits, ..device(id, !is_default, is_default) }
}

mod default_output {
    use super::*;

    fn out(id: &str, short: &str, adapter: Option<&str>, is_virtual: bool) -> AudioDevice {
        AudioDevice {
            name: adapter.map_or_else(|| short.to_owned(), |a| format!("{short} ({a})")),
            short_name: Some(short.into()),
            adapter_name: adapter.map(str::to_owned),
            is_virtual,
            is_display: false,
            ..audio(id, false, None, None)
        }
    }

    fn all() -> [AudioDevice; 7] {
        [
            out("hifi", "Hi-Fi Cable Input", Some("VB-Audio Hi-Fi Cable"), true),
            out("cable", "CABLE Input", Some("VB-Audio Virtual Cable"), true),
            out("c16", "CABLE In 16ch", Some("VB-Audio Virtual Cable"), true),
            out("spk", "Speakers", Some("Realtek(R) Audio"), false),
            out("tv1", "VX239", Some("HD Audio Driver for Display Audio"), false),
            out("tv2", "FS2434", Some("HD Audio Driver for Display Audio"), false),
            out("wdm", "Line", Some("Yamaha SYNCROOM Driver (WDM)"), true),
        ]
    }

    #[test]
    fn names_a_virtual_device_by_its_adapter_without_input_output() {
        let all = all();
        assert_eq!(default_output_name(&all[0], &all), "VB-Audio Hi-Fi Cable");
    }

    #[test]
    fn keeps_the_devices_own_name_where_the_adapter_would_not_tell_it_apart() {
        let all = all();
        let [hifi, cable, cable16, spk, tv1, ..] = all.clone();
        // Two outputs of one cable
        assert_eq!(default_output_name(&cable, &all), "CABLE Input");
        assert_eq!(default_output_name(&cable16, &all), "CABLE In 16ch");
        assert_eq!(default_output_name(&cable, &[hifi, cable.clone(), spk.clone()]), "VB-Audio Virtual Cable");
        // Sound devices: display audio and Realtek say nothing of which device
        assert_eq!(default_output_name(&spk, &all), "Speakers");
        assert_eq!(default_output_name(&tv1, &all), "VX239");
    }

    #[test]
    fn never_nests_parentheses_and_falls_back_to_the_full_name() {
        let all = all();
        assert_eq!(default_output_name(&all[6], &all), "Line");
        let spk = AudioDevice { short_name: None, adapter_name: None, ..all[3].clone() };
        assert_eq!(default_output_name(&spk, &all), "Speakers (Realtek(R) Audio)");
        let hifi = AudioDevice { adapter_name: None, ..all[0].clone() };
        assert_eq!(default_output_name(&hifi, &all), "Hi-Fi Cable Input");
    }
}

#[test]
fn finds_the_profiles_device_by_its_address_as_typed() {
    let devices = [dev("AC:80:0A:12:34:56", &["sbc", "ldac"]), dev("00:1A:7D:DA:71:13", &[])];
    assert_eq!(known_codecs(&devices, "ac-80-0a-12-34-56"), ["sbc", "ldac"]);
    assert!(known_codecs(&devices, "00:1A:7D:DA:71:13").is_empty());
    assert!(known_codecs(&devices, "11:22:33:44:55:66").is_empty());
    assert!(known_codecs(&devices, "").is_empty());
}

mod le_audio_entries {
    use super::*;

    fn le(address: &str, set: bool) -> Device {
        Device { le_audio: true, le_audio_set: set, ..dev(address, &[]) }
    }

    fn keys(list: &[Device]) -> Vec<String> {
        list.iter().map(|d| device_key(&d.address, d.le_audio)).collect()
    }

    #[test]
    fn name_an_entry_by_address_and_kind_a_dual_mode_device_has_two() {
        let list =
            [dev("F4:4E:FD:00:11:22", &["sbc", "aac"]), le("F4:4E:FD:00:11:22", true), le("D0:3E:7D:4C:2A:10", false)];
        assert_eq!(keys(&list), ["F4:4E:FD:00:11:22", "F4:4E:FD:00:11:22/le", "D0:3E:7D:4C:2A:10/le"]);
        // The codecs belong to the classic entry
        let mut with_le = vec![le("F4:4E:FD:00:11:22", false)];
        with_le.extend(list);
        assert_eq!(known_codecs(&with_le, "F4:4E:FD:00:11:22"), ["sbc", "aac"]);
    }

    #[test]
    fn are_listed_only_with_le_audio_on_as_v1() {
        let list = [dev("AC:80:0A:12:34:56", &[]), le("F4:4E:FD:00:11:22", false)];
        assert_eq!(keys(&listed_devices(&list, false)), ["AC:80:0A:12:34:56"]);
        assert_eq!(listed_devices(&list, true).len(), 2);
    }

    #[test]
    fn list_a_dual_mode_device_once_its_le_audio_entry_saved_when_either_side_is() {
        let classic = Device { name: "Buds".into(), saved: true, ..dev("F4:4E:FD:00:11:22", &[]) };
        let list = [classic.clone(), Device { name: String::new(), ..le("f4:4e:fd:00:11:22", false) }];
        assert_eq!(listed_devices(&list, false), [classic]);
        let listed = listed_devices(&list, true);
        assert_eq!(listed.len(), 1);
        let one = &listed[0];
        assert_eq!(device_key(&one.address, one.le_audio), "F4:4E:FD:00:11:22/le");
        assert_eq!((one.name.as_str(), one.saved, one.le_audio), ("Buds", true, true));
    }

    #[test]
    fn name_a_new_profile_device_le_audio() {
        assert_eq!(
            default_profile_name("Galaxy Buds2 Pro", "F4:4E:FD:00:11:22", true, ""),
            "Galaxy Buds2 Pro LE Audio"
        );
        assert_eq!(default_profile_name("", "F4:4E:FD:00:11:22", true, ""), "F4:4E:FD:00:11:22 LE Audio");
        assert_eq!(default_profile_name("Galaxy Buds2 Pro", "F4:4E:FD:00:11:22", false, ""), "Galaxy Buds2 Pro");
        assert_eq!(
            default_profile_name("EarFun Air Pro 4+", "F4:4E:FD:00:11:22", false, "Auto"),
            "EarFun Air Pro 4+ | Auto"
        );
        assert_eq!(default_profile_name("", "", false, "Auto"), "");
        assert_eq!(default_profile_name("", "", true, ""), "");
    }

    #[test]
    fn refuse_application_capture() {
        let p = Profile {
            name: "P".into(),
            device_address: "F4:4E:FD:00:11:22".into(),
            capture_mode: "app".into(),
            app_exe: "x.exe".into(),
            ..new_profile()
        };
        assert!(validate_profile(&p, NONE).app.is_none());
        assert_eq!(
            validate_profile(&Profile { le_audio: true, ..p.clone() }, NONE).app,
            Some(Problem { key: "error.le_audio_capture", args: vec![] })
        );
        assert!(validate_profile(&Profile { le_audio: true, capture_mode: "loopback".into(), ..p }, NONE).is_empty());
    }
}

mod capture_format {
    use super::*;

    fn list() -> [AudioDevice; 2] {
        [audio("speakers", true, Some(48000), Some(24)), audio("cable", false, Some(44100), Some(16))]
    }

    #[test]
    fn takes_the_default_device_for_loopback_the_chosen_one_for_a_virtual_device() {
        let list = list();
        assert_eq!(capture_device("loopback", "cable", &list).map(|d| d.id.as_str()), Some("speakers"));
        assert_eq!(capture_device("virtual", "cable", &list).map(|d| d.id.as_str()), Some("cable"));
        assert!(capture_device("virtual", "", &list).is_none());
        // Application capture: Windows converts to the rate asked for
        assert!(capture_device("app", "", &list).is_none());
    }

    #[test]
    fn warns_when_the_profiles_rate_or_bit_depth_differs() {
        let d = &list()[0];
        let fm = |rate, bits| format_mismatch(rate, bits, d.mix_rate, d.mix_bits);
        assert_eq!(fm(0, 0), FormatMismatch { rate: false, bits: false });
        assert_eq!(fm(48000, 24), FormatMismatch { rate: false, bits: false });
        assert_eq!(fm(96000, 16), FormatMismatch { rate: true, bits: true });
        assert_eq!(fm(44100, 0), FormatMismatch { rate: true, bits: false });
        // Not known: nothing to compare
        assert_eq!(format_mismatch(96000, 24, None, None), FormatMismatch { rate: false, bits: false });
    }
}

#[test]
fn follows_the_virtual_device_by_default_as_v1_and_the_core() {
    assert!(new_profile().auto_switch_device);
}

#[test]
fn captures_a_new_device_by_system_loopback_a_virtual_device_installed_or_not() {
    let p = new_profile();
    assert_eq!(
        (p.capture_mode.as_str(), p.audio_device_id.as_str(), p.audio_device_name.as_str()),
        ("loopback", "", "")
    );
    // The other entry's capture source counts first
    let other = Profile {
        device_address: "AA:BB:CC:DD:EE:FF".into(),
        capture_mode: "app".into(),
        app_exe: "x.exe".into(),
        ..new_profile()
    };
    let others = [other];
    assert_eq!(entry_base("aa:bb:cc:dd:ee:ff", "X", true, &others).capture_mode, "app");
    let b = entry_base("11:22:33:44:55:66", "Y", false, &others);
    assert_eq!((b.capture_mode.as_str(), b.audio_device_id.as_str()), ("loopback", ""));
}

#[test]
fn checks_a_device_name_as_the_core_does() {
    assert!(valid_device_name("WH-1000XM5"));
    assert!(!valid_device_name("  "));
    assert!(!valid_device_name("a|b"));
    assert!(!valid_device_name("a\nb"));
}

#[test]
fn reads_the_windows_version_from_the_client_hints() {
    // Windows 10 1909, 2004 and later, Windows 11
    assert_eq!(app_capture_support_from(Some("Windows"), Some("8.0.0")), AppCaptureSupport::No);
    assert_eq!(app_capture_support_from(Some("Windows"), Some("10.0.0")), AppCaptureSupport::Yes);
    assert_eq!(app_capture_support_from(Some("Windows"), Some("15.0.0")), AppCaptureSupport::Yes);
}

#[test]
fn does_not_guess_without_them() {
    assert_eq!(app_capture_support_from(None, None), AppCaptureSupport::Unknown);
    assert_eq!(app_capture_support_from(Some("macOS"), Some("14.0.0")), AppCaptureSupport::Unknown);
    assert_eq!(app_capture_support_from(Some("Windows"), Some("")), AppCaptureSupport::Unknown);
}
