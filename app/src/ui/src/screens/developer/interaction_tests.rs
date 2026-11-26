//! Developer's typed fields: digits only where a number goes, held to their
//! range and shown as saved once left or Enter is pressed, the MediaTek
//! commands' hex in capitals, an empty field back to Auto (the old UI's
//! Developer.test.tsx is the spec).

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, TestAppContext};
use serde_json::json;

use super::*;
use crate::testing::{Harness, snapshot};

fn open(cx: &mut TestAppContext) -> (Harness, AnyWindowHandle, Entity<Developer>) {
    let mut s = snapshot();
    s["settings"]["developer_mode"] = json!(true);
    let mut h = Harness::start(cx, s);
    let (w, dev) = h.open_sized(cx, (1000., 1400.), |window, cx| cx.new(|cx| Developer::new(window, cx)));
    (h, w, dev)
}

/// Shows a tab: only the fields on screen take typing
fn tab(h: &mut Harness, cx: &mut TestAppContext, w: AnyWindowHandle, d: &Entity<Developer>, tab: Tab) {
    h.on(cx, w, |window, cx| d.update(cx, |d, cx| d.select_tab(tab, window, cx)));
}

/// Types `text` over field `n`
fn type_into(h: &mut Harness, cx: &mut TestAppContext, w: AnyWindowHandle, d: &Entity<Developer>, n: Num, text: &str) {
    let input = cx.read(|cx| d.read(cx).input(n).clone());
    h.on(cx, w, |window, cx| {
        input.update(cx, |s, cx| s.focus(window, cx));
        window.press("ctrl-a", cx);
        window.press("backspace", cx);
        window.input(text, cx);
    });
}

fn shown(d: &Entity<Developer>, cx: &TestAppContext, n: Num) -> String {
    cx.read(|cx| d.read(cx).input(n).read(cx).value().to_string())
}

fn settings(h: &Harness) -> serde_json::Value {
    h.calls("settings.set").last().map(|p| p["settings"].clone()).expect("settings were saved")
}

#[gpui_kit::test]
fn numbers_take_digits_only_and_are_held_to_their_range(cx: &mut TestAppContext) {
    let (mut h, w, d) = open(cx);
    tab(&mut h, cx, w, &d, Tab::Overrides);
    type_into(&mut h, cx, w, &d, Num::AacBitrate, "9x9-9.9");
    assert_eq!(shown(&d, cx, Num::AacBitrate), "9999");
    assert!(h.calls("settings.set").is_empty(), "saved while typing");
    h.on(cx, w, |window, cx| window.press("enter", cx));
    assert_eq!(settings(&h)["dev_overrides"]["aac_bitrate_kbps"], 512);
    assert_eq!(shown(&d, cx, Num::AacBitrate), "512");
    // The queue limit: raised to its least, empty = the default
    type_into(&mut h, cx, w, &d, Num::QueueWait, "5");
    h.on(cx, w, |window, cx| window.press("tab", cx));
    assert_eq!(settings(&h)["dev_overrides"]["queue_max_wait_ms"], 50);
    assert_eq!(shown(&d, cx, Num::QueueWait), "50");
    type_into(&mut h, cx, w, &d, Num::QueueWait, "");
    h.on(cx, w, |window, cx| window.press("enter", cx));
    assert_eq!(settings(&h)["dev_overrides"]["queue_max_wait_ms"], 0);
}

#[gpui_kit::test]
fn the_mediatek_fields_and_le_audio_values(cx: &mut TestAppContext) {
    let (mut h, w, d) = open(cx);
    tab(&mut h, cx, w, &d, Tab::Debug);
    type_into(&mut h, cx, w, &d, Num::MtkPad, "99999");
    h.on(cx, w, |window, cx| window.press("enter", cx));
    assert_eq!(settings(&h)["dev_debug"]["mtk_iso_pad"], 4096);
    type_into(&mut h, cx, w, &d, Num::MtkCommand, " fc00 01 ");
    h.on(cx, w, |window, cx| window.press("enter", cx));
    assert_eq!(settings(&h)["dev_debug"]["mtk_command"], "FC00 01");
    assert_eq!(shown(&d, cx, Num::MtkCommand), "FC00 01");
    type_into(&mut h, cx, w, &d, Num::MtkRegister, " 40 01 0000abcd 00000001 ");
    h.on(cx, w, |window, cx| window.press("enter", cx));
    assert_eq!(settings(&h)["dev_debug"]["mtk_register"], "40 01 0000ABCD 00000001");
    type_into(&mut h, cx, w, &d, Num::LeLatencyPToC, "100");
    h.on(cx, w, |window, cx| window.press("enter", cx));
    assert_eq!(settings(&h)["dev_debug"]["le_latency_p_to_c_ms"], 100);
    // RTN 0 is a value of its own, empty is Auto
    type_into(&mut h, cx, w, &d, Num::LeRtnPToC, "0");
    h.on(cx, w, |window, cx| window.press("enter", cx));
    assert_eq!(settings(&h)["dev_debug"]["le_rtn_p_to_c"], 0);
    type_into(&mut h, cx, w, &d, Num::LeRtnPToC, "");
    h.on(cx, w, |window, cx| window.press("enter", cx));
    assert_eq!(settings(&h)["dev_debug"]["le_rtn_p_to_c"], serde_json::Value::Null);
    // The jitter buffer's margin: 0 is a value of its own, at most 7, empty is the core's 3
    type_into(&mut h, cx, w, &d, Num::LeBufferMargin, "0");
    h.on(cx, w, |window, cx| window.press("enter", cx));
    assert_eq!(settings(&h)["dev_debug"]["le_buffer_margin"], 0);
    type_into(&mut h, cx, w, &d, Num::LeBufferMargin, "9");
    h.on(cx, w, |window, cx| window.press("enter", cx));
    assert_eq!(settings(&h)["dev_debug"]["le_buffer_margin"], 7);
    assert_eq!(shown(&d, cx, Num::LeBufferMargin), "7");
    type_into(&mut h, cx, w, &d, Num::LeBufferMargin, "");
    h.on(cx, w, |window, cx| window.press("enter", cx));
    assert_eq!(settings(&h)["dev_debug"]["le_buffer_margin"], serde_json::Value::Null);
    tab(&mut h, cx, w, &d, Tab::Logging);
    type_into(&mut h, cx, w, &d, Num::KeepDays, "1000");
    h.on(cx, w, |window, cx| window.press("enter", cx));
    assert_eq!(settings(&h)["dev_log"]["keep_days"], 365);
    assert_eq!(shown(&d, cx, Num::KeepDays), "365");
}

#[gpui_kit::test]
fn the_log_note_is_held_to_200_characters(cx: &mut TestAppContext) {
    let (mut h, w, d) = open(cx);
    tab(&mut h, cx, w, &d, Tab::Logging);
    let note = cx.read(|cx| d.read(cx).note.clone());
    h.on(cx, w, |window, cx| {
        note.update(cx, |s, cx| s.focus(window, cx));
        window.input(&"m".repeat(210), cx);
    });
    assert_eq!(cx.read(|cx| note.read(cx).value().chars().count()), 200);
}

#[gpui_kit::test]
fn the_tws_tab_reads_the_earbuds_mixer_while_it_is_shown(cx: &mut TestAppContext) {
    let (mut h, w, d) = open(cx);
    assert!(!h.methods().iter().any(|m| m == "tws.mix"));
    tab(&mut h, cx, w, &d, Tab::Tws);
    h.settle(cx);
    assert!(h.methods().iter().any(|m| m == "tws.mix"));
    assert!(cx.read(|cx| d.read(cx).tws.is_some()));
    tab(&mut h, cx, w, &d, Tab::App);
    assert!(cx.read(|cx| d.read(cx).tws.is_none()));
}
