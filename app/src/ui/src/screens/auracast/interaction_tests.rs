//! The Auracast tab's source and volume: a device as the source goes out
//! with its id and name, on start and while live; the follow switch sends
//! auracast.set_follow_volume and shows what the core keeps (the old UI's
//! Developer.test.tsx is the spec).

use std::time::Duration;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, TestAppContext};
use serde_json::{Value, json};

use super::*;
use crate::testing::{Answer, Harness, snapshot};

const SPEAKERS: &str = "{0.0.0.00000000}.{speakers}";
const MIC: &str = "{0.0.1.00000000}.{mic}";

fn audio_device(id: &str, name: &str) -> Value {
    json!({"id": id, "name": name, "is_default": false, "is_virtual": false})
}

fn open(cx: &mut TestAppContext, follow: bool) -> (Harness, AnyWindowHandle, Entity<Auracast>) {
    let mut s = snapshot();
    s["settings"]["developer_mode"] = json!(true);
    let mut h = Harness::start(cx, s);
    h.answer("audio.devices", Answer::Ok(json!([audio_device(SPEAKERS, "Speakers (Realtek(R) Audio)")])));
    h.answer("audio.input_devices", Answer::Ok(json!([audio_device(MIC, "Microphone (Realtek(R) Audio)")])));
    h.answer("auracast.follow_volume", Answer::Ok(json!(follow)));
    h.answer("auracast.set_follow_volume", Answer::Ok(json!(false)));
    h.answer("auracast.set_source", Answer::Ok(json!(true)));
    h.answer("stream.start_broadcast", Answer::Ok(Value::Null));
    let (w, view) = h.open_sized(cx, (1000., 1400.), |window, cx| cx.new(|cx| Auracast::new(window, cx)));
    (h, w, view)
}

fn choose(h: &mut Harness, cx: &mut TestAppContext, w: AnyWindowHandle, view: &Entity<Auracast>, source: Source) {
    h.on(cx, w, |_, cx| view.update(cx, |a, cx| a.change_source(source, cx)));
}

fn choices(view: &Entity<Auracast>, cx: &TestAppContext) -> Choices {
    cx.read(|cx| view.read(cx).session.read(cx).choices.clone())
}

#[gpui_kit::test]
fn starts_a_device_broadcast_with_its_id_name_and_follow(cx: &mut TestAppContext) {
    let (mut h, w, view) = open(cx, false);
    // Windows' output and recording devices, each under its heading
    let (outputs, inputs) = cx.read(|cx| (view.read(cx).outputs.len(), view.read(cx).inputs.len()));
    assert_eq!((outputs, inputs), (1, 1));
    choose(&mut h, cx, w, &view, Source::Output(SPEAKERS.into()));
    // Not live: nothing goes out yet
    assert!(h.calls("auracast.set_source").is_empty());
    h.on(cx, w, |window, cx| window.click("auracast-follow", cx));
    assert_eq!(h.calls("auracast.set_follow_volume"), [json!({"follow": true})]);
    assert!(choices(&view, cx).follow);
    h.on(cx, w, |window, cx| window.click("auracast-start", cx));
    let start = h.calls("stream.start_broadcast");
    assert_eq!(start.len(), 1);
    assert_eq!(start[0]["source"], "output");
    assert_eq!(start[0]["source_id"], SPEAKERS);
    assert_eq!(start[0]["source_name"], "Speakers (Realtek(R) Audio)");
    assert_eq!(start[0]["follow_volume"], true);
}

#[gpui_kit::test]
fn a_device_chosen_while_live_goes_out_with_its_id_and_name(cx: &mut TestAppContext) {
    let (mut h, w, view) = open(cx, false);
    h.edit(cx, |s| {
        s["stream"] = json!({"state": "streaming", "message": {"key": "status.broadcasting", "args": []}});
    });
    choose(&mut h, cx, w, &view, Source::Input(MIC.into()));
    assert_eq!(
        h.calls("auracast.set_source"),
        [json!({"source": "input", "source_id": MIC, "source_name": "Microphone (Realtek(R) Audio)"})]
    );
    // The stream names nothing yet: the choice stays after the settle time
    cx.executor().advance_clock(Duration::from_millis(SOURCE_SETTLE_MS + 100));
    h.settle(cx);
    assert_eq!(choices(&view, cx).source, Source::Input(MIC.into()));
    // A recording device: nothing to follow, the switch is off
    h.on(cx, w, |window, cx| window.click("auracast-follow", cx));
    assert!(h.calls("auracast.set_follow_volume").is_empty());
}

#[gpui_kit::test]
fn the_follow_switch_shows_what_the_core_keeps(cx: &mut TestAppContext) {
    let (h, _, view) = open(cx, true);
    assert!(choices(&view, cx).follow);
    // Read, not sent back
    assert!(h.calls("auracast.set_follow_volume").is_empty());
}
