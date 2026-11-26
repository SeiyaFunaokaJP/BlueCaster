//! Typing on Audio Devices: a device's new name (64 characters at most, not
//! empty, no line breaks or "|") and an address typed by hand (the forms
//! ui_model::format::parse_address takes, full-width too). The old UI's
//! DevicePicker.tsx is the spec.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, TestAppContext};
use serde_json::json;

use super::*;
use crate::testing::{Answer, Harness, device, snapshot};

const ADDR: &str = "0A:00:00:00:00:03";

fn open(cx: &mut TestAppContext) -> (Harness, AnyWindowHandle, Entity<DevicePicker>) {
    let mut s = snapshot();
    s["devices"] = json!([device(ADDR, "Old Name")]);
    let mut h = Harness::start(cx, s);
    let props = DevicePickerProps { manage: true, ..Default::default() };
    let (w, picker) = h.open_sized(cx, (1000., 1200.), |window, cx| cx.new(|cx| DevicePicker::new(props, window, cx)));
    (h, w, picker)
}

fn start_rename(h: &mut Harness, cx: &mut TestAppContext, w: AnyWindowHandle, picker: &Entity<DevicePicker>) {
    let d = cx.read(|cx| app::core(cx).read(cx).snap().unwrap().devices[0].clone());
    h.on(cx, w, |window, cx| picker.update(cx, |p, cx| p.open_rename(&d, window, cx)));
}

fn name(picker: &Entity<DevicePicker>, cx: &TestAppContext) -> String {
    cx.read(|cx| picker.read(cx).new_name.read(cx).value().to_string())
}

fn open_dialog(h: &mut Harness, cx: &mut TestAppContext, w: AnyWindowHandle) -> bool {
    h.on(cx, w, |window, cx| window.has_active_dialog(cx))
}

#[gpui_kit::test]
fn a_new_name_is_held_to_64_characters(cx: &mut TestAppContext) {
    let (mut h, w, picker) = open(cx);
    start_rename(&mut h, cx, w, &picker);
    assert_eq!(name(&picker, cx), "Old Name");
    let long = "n".repeat(70);
    h.on(cx, w, |window, cx| {
        window.press("ctrl-a", cx);
        window.input(&long, cx);
    });
    assert_eq!(name(&picker, cx).chars().count(), 64);
    // Counted as the old UI did: a character outside the BMP is two
    h.on(cx, w, |window, cx| {
        window.press("ctrl-a", cx);
        window.input(&"n".repeat(63), cx);
        window.input("🎧", cx);
    });
    assert_eq!(name(&picker, cx).chars().count(), 63);
}

#[gpui_kit::test]
fn an_empty_or_broken_name_is_not_saved(cx: &mut TestAppContext) {
    let (mut h, w, picker) = open(cx);
    start_rename(&mut h, cx, w, &picker);
    for bad in ["   ", "Left|Right"] {
        h.on(cx, w, |window, cx| {
            window.press("ctrl-a", cx);
            window.press("backspace", cx);
            window.input(bad, cx);
            window.press("enter", cx);
        });
        assert!(h.calls("device.rename").is_empty(), "{bad:?} was sent");
        assert!(open_dialog(&mut h, cx, w));
        assert!(cx.read(|cx| picker.read(cx).rename_tried));
        // Save does the same
        h.on(cx, w, |window, cx| window.click("dialog-confirm", cx));
        assert!(h.calls("device.rename").is_empty());
    }
}

#[gpui_kit::test]
fn enter_saves_the_trimmed_name_and_closes(cx: &mut TestAppContext) {
    let (mut h, w, picker) = open(cx);
    h.answer("device.rename", Answer::Ok(serde_json::Value::Null));
    start_rename(&mut h, cx, w, &picker);
    h.on(cx, w, |window, cx| {
        window.press("ctrl-a", cx);
        window.input("  New Name ", cx);
        window.press("enter", cx);
    });
    let sent = h.calls("device.rename");
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["address"], ADDR);
    assert_eq!(sent[0]["name"], "New Name");
    assert!(!open_dialog(&mut h, cx, w));
}

#[gpui_kit::test]
fn escape_closes_the_rename_unless_it_is_saving(cx: &mut TestAppContext) {
    let (mut h, w, picker) = open(cx);
    h.answer("device.rename", Answer::Hold);
    start_rename(&mut h, cx, w, &picker);
    h.on(cx, w, |window, cx| {
        window.press("ctrl-a", cx);
        window.input("Held", cx);
        window.press("enter", cx);
    });
    assert_eq!(h.calls("device.rename").len(), 1);
    h.on(cx, w, |window, cx| window.press("escape", cx));
    assert!(open_dialog(&mut h, cx, w), "Escape closed it while saving");
    h.release(cx, "device.rename", Err("device.not_found"));
    // Failed: still open for another try; Escape now closes it
    assert!(open_dialog(&mut h, cx, w));
    h.on(cx, w, |window, cx| window.press("escape", cx));
    assert!(!open_dialog(&mut h, cx, w));
}

fn type_address(h: &mut Harness, cx: &mut TestAppContext, w: AnyWindowHandle, p: &Entity<DevicePicker>, text: &str) {
    let input = cx.read(|cx| p.read(cx).manual.clone());
    h.on(cx, w, |window, cx| {
        input.update(cx, |s, cx| s.focus(window, cx));
        window.press("ctrl-a", cx);
        window.press("backspace", cx);
        window.input(text, cx);
    });
}

fn use_enabled(h: &mut Harness, cx: &mut TestAppContext, w: AnyWindowHandle) -> bool {
    h.on(cx, w, |window, _| window.find("use-address").disabled() != Some(true))
}

#[gpui_kit::test]
fn a_typed_address_in_any_form_it_takes(cx: &mut TestAppContext) {
    let (mut h, w, picker) = open(cx);
    assert!(!use_enabled(&mut h, cx, w));
    for good in [
        "aa:bb:cc:dd:ee:ff",
        "AA-BB-CC-DD-EE-FF",
        "aabbccddeeff",
        "aa bb cc dd ee ff",
        " aa.bb.cc.dd.ee.ff ",
        "ＡＡ：ＢＢ：ＣＣ：ＤＤ：ＥＥ：ＦＦ",
    ] {
        type_address(&mut h, cx, w, &picker, good);
        assert!(use_enabled(&mut h, cx, w), "{good:?} not taken");
    }
    for bad in ["aa:bb:cc:dd:ee", "aa:bb:cc:dd:ee:fg", "aa:bb:cc:dd:ee:ff:00", "aa::bb:cc:dd:ee:ff"] {
        type_address(&mut h, cx, w, &picker, bad);
        assert!(!use_enabled(&mut h, cx, w), "{bad:?} taken");
    }
}

#[gpui_kit::test]
fn add_saves_the_typed_address_and_empties_the_field(cx: &mut TestAppContext) {
    let (mut h, w, picker) = open(cx);
    h.answer("device.windows_paired", Answer::Ok(json!([])));
    h.answer("audio.devices", Answer::Ok(json!([])));
    h.answer("device.rename", Answer::Ok(serde_json::Value::Null));
    h.answer("stream.start", Answer::Ok(serde_json::Value::Null));
    type_address(&mut h, cx, w, &picker, "ａａ－ｂｂ－ｃｃ－ｄｄ－ｅｅ－０１");
    h.on(cx, w, |window, cx| window.click("use-address", cx));
    // Saved under its address, upper-case with colons
    assert_eq!(h.calls("device.rename")[0]["address"], "AA:BB:CC:DD:EE:01");
    let saved = h.last_saved();
    assert_eq!(saved["device_address"], "AA:BB:CC:DD:EE:01");
    assert_eq!(cx.read(|cx| picker.read(cx).manual.read(cx).value().to_string()), "");
}

#[gpui_kit::test]
fn adding_an_le_audio_device_saves_it(cx: &mut TestAppContext) {
    // Add on an LE Audio device skips Windows' pairings and went on while
    // the picker was still being updated: the app stopped (a picker updated
    // twice)
    let mut s = snapshot();
    s["devices"] =
        json!([{"address": "0A:00:00:00:00:04", "name": "Buds", "saved": false, "audio": true, "le_audio": true}]);
    s["settings"]["le_audio_experimental"] = json!(true);
    s["settings"]["developer_mode"] = json!(true);
    s["adapter"]["le_audio"] = json!(true);
    let mut h = Harness::start(cx, s);
    h.answer("audio.devices", Answer::Ok(json!([])));
    h.answer("device.rename", Answer::Ok(serde_json::Value::Null));
    h.answer("stream.start", Answer::Ok(serde_json::Value::Null));
    let props = DevicePickerProps { manage: true, ..Default::default() };
    let (w, _) = h.open_sized(cx, (1000., 1200.), |window, cx| cx.new(|cx| DevicePicker::new(props, window, cx)));
    h.on(cx, w, |window, cx| window.click("add", cx));
    h.settle(cx);
    assert_eq!(h.calls("device.rename")[0]["address"], "0A:00:00:00:00:04");
    assert!(h.calls("device.windows_paired").is_empty());
}
