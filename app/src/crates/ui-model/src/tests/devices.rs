//! view/devices.rs' tests: the capture picker's choices, the Remove button
//! of each entry Home shows, the keys' rows of the pairing dialog and the
//! device picker's order

use ipc::{
    AudioApp, AudioDevice, Device, DevicePairing, LeKeysInfo, LeKeysMissing, LinkKeyInfo, Profile, WindowsImport,
};

use super::*;
use crate::profile::listed_devices;
use crate::sbc::mock::{sample_device, sample_profile};

fn audio_device(id: &str, name: &str, is_default: bool, is_virtual: bool) -> AudioDevice {
    AudioDevice {
        id: id.into(),
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

fn devices() -> Vec<AudioDevice> {
    vec![audio_device("spk", "Speakers", true, false), audio_device("cable", "CABLE Input", false, true)]
}

fn apps() -> Vec<AudioApp> {
    vec![AudioApp { exe: "Spotify.exe".into(), name: "Spotify".into() }]
}

fn labels(c: &[CaptureChoice]) -> Vec<&str> {
    c.iter().map(|x| x.label.as_str()).collect()
}

// ---- the capture picker's choices ----

#[test]
fn offers_virtual_devices_and_playing_apps_and_keeps_the_profiles_own() {
    let p = Profile {
        capture_mode: "app".into(),
        app_exe: "foobar2000.exe".into(),
        app_name: "foobar2000".into(),
        ..sample_profile()
    };
    let c = capture_choices(&p, &devices(), &apps(), true, false, "");
    assert_eq!(labels(&c.virtual_devices), ["CABLE Input"]);
    let values: Vec<&str> = c.apps.iter().map(|x| x.value.as_str()).collect();
    assert_eq!(values, ["app:foobar2000.exe", "app:spotify.exe"]);
    assert_eq!(c.apps[0].label, "foobar2000 (not playing now)");
}

#[test]
fn offers_no_apps_for_le_audio_or_where_windows_cannot_capture_one() {
    let le = Profile { le_audio: true, ..sample_profile() };
    assert!(capture_choices(&le, &devices(), &apps(), true, false, "").apps.is_empty());
    assert!(capture_choices(&sample_profile(), &devices(), &apps(), false, false, "").apps.is_empty());
}

#[test]
fn offers_the_other_output_devices() {
    // Issue #193. Each device once: the virtual one stays under Virtual Device
    let p = sample_profile();
    assert_eq!(
        capture_choices(&p, &devices(), &apps(), true, false, "").output,
        [CaptureChoice { value: "output:spk".into(), label: "Speakers".into() }]
    );
    let gone = Profile {
        capture_mode: "output".into(),
        audio_device_id: "usb".into(),
        audio_device_name: "USB Speakers".into(),
        ..sample_profile()
    };
    assert_eq!(
        capture_choices(&gone, &devices(), &apps(), true, false, "").output[0],
        CaptureChoice { value: "output:usb".into(), label: "USB Speakers (not connected)".into() }
    );
    let o = with_capture(&p, "output:spk", &devices(), &apps());
    assert_eq!((o.capture_mode.as_str(), o.audio_device_id.as_str()), ("output", "spk"));
    assert_eq!(o.audio_device_name, "Speakers");
    assert_eq!(capture_value(&o), "output:spk");
}

#[test]
fn sets_what_the_capture_source_needs() {
    let p = sample_profile();
    let v = with_capture(&p, "virtual:cable", &devices(), &apps());
    assert_eq!((v.capture_mode.as_str(), v.audio_device_id.as_str()), ("virtual", "cable"));
    assert_eq!(v.audio_device_name, "CABLE Input");
    let a = with_capture(&p, "app:spotify.exe", &devices(), &apps());
    assert_eq!((a.capture_mode.as_str(), a.app_exe.as_str(), a.app_name.as_str()), ("app", "Spotify.exe", "Spotify"));
    assert_eq!(capture_value(&a), "app:spotify.exe");
    assert_eq!(with_capture(&a, "loopback", &devices(), &apps()).capture_mode, "loopback");
}

#[test]
fn leaves_the_16_channel_cable_and_the_microphones_cable_out_unless_chosen_or_all() {
    let mut list = devices();
    list.push(audio_device("c16", "CABLE In 16ch (VB-Audio Virtual Cable)", false, true));
    let p = sample_profile();
    assert_eq!(labels(&capture_choices(&p, &list, &[], true, false, "cable").virtual_devices), Vec::<&str>::new());
    assert_eq!(labels(&capture_choices(&p, &list, &[], true, true, "cable").virtual_devices).len(), 2);
    let chosen = Profile { capture_mode: "virtual".into(), audio_device_id: "c16".into(), ..sample_profile() };
    assert_eq!(labels(&capture_choices(&chosen, &list, &[], true, false, "").virtual_devices).len(), 2);
}

// ---- DeviceSettings: the one list ----

#[test]
fn lists_the_default_output_first_and_says_when_nothing_plays() {
    let list = devices();
    let lists = AudioLists { devices: Some(&list), listed: true, apps: Some(&[]), apps_ok: true };
    let o = device_sources(&sample_profile(), lists, false, "");
    assert_eq!(o[0].value, "loopback");
    assert!(o[0].group.is_none());
    let last = o.last().expect("an entry");
    assert_eq!((last.value.as_str(), last.disabled), ("app:", true));
    // No virtual device at all: VB-Cable's line, disabled
    let real = vec![audio_device("spk", "Speakers", true, false)];
    let lists = AudioLists { devices: Some(&real), listed: true, apps: None, apps_ok: true };
    let o = device_sources(&sample_profile(), lists, false, "");
    assert!(o.iter().any(|x| x.value == "virtual:" && x.disabled));
    // Not read: no such line
    let lists = AudioLists { devices: Some(&real), listed: false, apps: None, apps_ok: true };
    assert!(!device_sources(&sample_profile(), lists, false, "").iter().any(|x| x.value == "virtual:"));
}

#[test]
fn changes_each_profile_of_a_device_but_an_le_audio_ones_app() {
    let classic = sample_profile();
    let le = Profile { name: "LE".into(), le_audio: true, ..sample_profile() };
    let next = DeviceFields { capture_mode: "app".into(), app_exe: "a.exe".into(), ..device_fields(&classic) };
    let updates = device_profile_updates(&[&classic, &le], &next);
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].1, classic.name);
    // Unchanged: nothing to save
    assert!(device_profile_updates(&[&classic], &device_fields(&classic)).is_empty());
}

#[test]
fn says_the_format() {
    assert_eq!(format_text(48000, 0), "48000 Hz");
    assert_eq!(format_text(44100, 32), "44100 Hz / 32-bit");
}

// ---- InputSource ----

#[test]
fn names_cables_by_the_side_apps_pick() {
    let mut a = audio_device("a", "CABLE Input (VB-Audio Virtual Cable)", false, true);
    a.capture_name = Some("CABLE Output".into());
    let mut b = audio_device("b", "CABLE In 16ch (VB-Audio Virtual Cable)", false, true);
    b.capture_name = Some("CABLE Output".into());
    let list = vec![b, a, audio_device("spk", "Speakers", true, false)];
    let shown = Profile { mic_device_id: "a".into(), ..sample_profile() };
    let o = mic_options(&list, &shown, "", true);
    let names: Vec<&str> = o.iter().map(|x| x.label.as_str()).collect();
    assert_eq!(names, ["CABLE Output", "CABLE Output · CABLE In 16ch", "Speakers"]);
    // Without "all": the cable only; with none chosen, "None" first and disabled
    let none = Profile { mic_device_id: String::new(), ..sample_profile() };
    let o = mic_options(&list, &none, "", false);
    assert_eq!(o.len(), 2);
    assert!(o[0].disabled && o[0].value.is_empty());
}

#[test]
fn refuses_the_output_sources_cable_for_the_microphone() {
    let mut a = audio_device("a", "CABLE Input", false, true);
    a.capture_name = Some("CABLE Output".into());
    let list = vec![a];
    assert!(used_by_source(&list, "a", "a"));
    assert!(!used_by_source(&list, "a", ""));
    assert!(!used_by_source(&list, "", "a"));
    let shown = Profile { mic_device_id: "a".into(), ..sample_profile() };
    let o = mic_options(&list, &shown, "a", false);
    assert!(o[0].disabled);
}

#[test]
fn off_mutes_a_device_with_a_mode_and_turns_the_others_off() {
    use ipc::HeadsetMicMode::{Auto, Off, On};
    assert_eq!(choose_mode(Off, Auto, Auto), Some(MicPatch::Mute));
    assert_eq!(choose_mode(Off, Auto, Off), Some(MicPatch::Mode(Off)));
    assert_eq!(choose_mode(On, Off, Auto), Some(MicPatch::Mode(On)));
    assert_eq!(choose_mode(Auto, Auto, Auto), None);
}

// ---- each entry Home shows has its Remove button ----

fn saved(address: &str, name: &str, le_audio: bool) -> Device {
    Device { name: name.into(), saved: true, le_audio, ..sample_device(address) }
}

#[test]
fn lists_every_entry_home_shows_for_removal() {
    let list = vec![
        saved("AC:80:0A:12:34:56", "WH-1000XM5", false),
        saved("F4:4E:FD:00:11:22", "Buds", false),
        saved("F4:4E:FD:00:11:22", "Buds", true),
        Device { name: "Keyboard".into(), audio: false, ..sample_device("00:11:22:33:44:55") },
    ];
    // LE Audio off; on (also on an adapter no stack has checked yet: Home
    // lists its LE Audio entries, so Audio Devices does too)
    for le_on in [false, true] {
        let home: Vec<(String, bool)> =
            listed_devices(&list, le_on).into_iter().filter(|d| d.saved).map(|d| (d.name, d.le_audio)).collect();
        let mut here: Vec<(String, bool)> =
            managed_saved(&list, le_on, false).into_iter().map(|d| (d.name, d.le_audio)).collect();
        let mut home = home;
        home.sort();
        here.sort();
        assert_eq!(here, home);
    }
    // The dual-mode device once, as its LE Audio entry
    let on = managed_saved(&list, true, false);
    assert_eq!(on.len(), 2);
    assert!(on.iter().any(|d| d.le_audio));
}

// ---- the picker's order and names ----

#[test]
fn sorts_saved_then_audio_by_signal() {
    let list = vec![
        Device { audio: false, rssi: Some(-30), ..sample_device("00:00:00:00:00:01") },
        Device { rssi: Some(-80), ..sample_device("00:00:00:00:00:02") },
        Device { rssi: Some(-40), ..sample_device("00:00:00:00:00:03") },
        saved("00:00:00:00:00:04", "Saved", false),
    ];
    let order: Vec<String> = sort_devices(&list).iter().map(|d| d.address[15..].to_owned()).collect();
    assert_eq!(order, ["04", "03", "02", "01"]);
}

#[test]
fn saves_a_device_under_its_name_else_the_saved_one_else_its_address() {
    let list = vec![saved("AA:BB:CC:DD:EE:FF", "Mine", false)];
    assert_eq!(saved_name(&list, "AA:BB:CC:DD:EE:FF", " New ", false), "New");
    assert_eq!(saved_name(&list, "aa:bb:cc:dd:ee:ff", "", false), "Mine");
    // The LE Audio side takes its classic twin's name
    assert_eq!(saved_name(&list, "AA:BB:CC:DD:EE:FF", "", true), "Mine");
    assert_eq!(saved_name(&list, "11:22:33:44:55:66", "", false), "11:22:33:44:55:66");
    assert_eq!(free_name(&["A", "A (2)"], "A"), "A (3)");
    assert_eq!(free_name(&["B"], "A"), "A");
}

#[test]
fn says_what_an_import_did() {
    let r = WindowsImport { adapter_found: true, imported: vec!["A".into()], kept: vec![], added: vec![] };
    assert_eq!(import_notice(&r), t!("ui.adapter.import_done", 1));
    let none = WindowsImport { adapter_found: false, ..r };
    assert_eq!(import_notice(&none), t("ui.adapter.import_none"));
}

// ---- the pairing dialog's keys ----

fn link_key() -> Option<LinkKeyInfo> {
    Some(LinkKeyInfo { key: "00112233445566778899aabbccddeeff".into(), key_type: 4 })
}

#[test]
fn lists_the_link_key_and_says_why_there_are_no_le_keys() {
    let p = DevicePairing { link_key: link_key(), le_keys: None, le_keys_missing: Some(LeKeysMissing::NotAsked) };
    let rows = key_rows(false, &p);
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, [t("ui.pairing.link_key"), t("ui.pairing.ltk"), t("ui.pairing.irk")]);
    assert_eq!(rows[0].value.as_deref(), Some("00112233445566778899AABBCCDDEEFF"));
    assert_eq!(rows[0].tag, Some(t!("ui.pairing.key_type", 4)));
    // No LE keys: both rows say so, never a made-up value
    assert!(rows[1].value.is_none() && rows[2].value.is_none());
    assert_eq!(le_missing_text(&p), Some(t("ui.pairing.le_missing.not_asked")));
    let no_support = DevicePairing { le_keys_missing: Some(LeKeysMissing::NoSupport), ..p };
    assert_eq!(le_missing_text(&no_support), Some(t("ui.pairing.le_missing.no_support")));
}

#[test]
fn shows_the_le_keys_derived_from_the_link_key() {
    let p = DevicePairing {
        link_key: link_key(),
        le_keys: Some(LeKeysInfo {
            long_term_key: Some("0f0e0d0c0b0a09080706050403020100".into()),
            key_size: 16,
            identity_resolving_key: Some("a0a1a2a3a4a5a6a7a8a9aaabacadaeaf".into()),
            identity_address: "C0:11:22:33:44:55".into(),
            identity_random: true,
        }),
        le_keys_missing: None,
    };
    let rows = key_rows(false, &p);
    assert_eq!(rows[1].value.as_deref(), Some("0F0E0D0C0B0A09080706050403020100"));
    assert_eq!(rows[1].tag, Some(t!("ui.pairing.key_size", 16)));
    assert_eq!(rows[2].value.as_deref(), Some("A0A1A2A3A4A5A6A7A8A9AAABACADAEAF"));
    // Three keys to copy, no identity address, no "why"
    assert_eq!(rows.iter().filter(|r| r.value.is_some()).count(), 3);
    assert!(rows.iter().all(|r| r.value.as_deref() != Some("C0:11:22:33:44:55")));
    assert_eq!(le_missing_text(&p), None);
}

#[test]
fn lists_an_le_audio_entrys_le_keys_only_unless_its_pairing_gave_a_link_key() {
    let p = DevicePairing { link_key: None, le_keys: None, le_keys_missing: None };
    let names: Vec<String> = key_rows(true, &p).into_iter().map(|r| r.name).collect();
    assert_eq!(names, [t("ui.pairing.ltk"), t("ui.pairing.irk")]);
    let with_key = DevicePairing { link_key: link_key(), ..p };
    assert_eq!(key_rows(true, &with_key)[0].name, t("ui.pairing.link_key"));
}

#[test]
fn asks_for_the_services_again_while_another_query_runs() {
    assert!(retry_services("device.busy", 2));
    assert!(!retry_services("device.busy", 0));
    assert!(!retry_services("device.not_connected", 5));
}
