//! The mixer's number fields typed into: a value counts when the field is
//! left or Enter is pressed and goes out after a short pause; one out of
//! range or not a number puts the field back; a field being typed into is
//! never overwritten by a new state from the core.

use std::time::Duration;

use gpui_kit::{AnyWindowHandle, Entity, IntoElement, Render, TestAppContext};
use serde_json::{Value, json};

use super::*;
use crate::testing::{Answer, Harness, snapshot};

/// The mixer's section in a window of its own
struct Host {
    mixer: Entity<AuracastMixer>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let section = self.mixer.update(cx, |m, cx| m.section(Vec::new(), cx));
        div().size_full().children(section)
    }
}

fn mix_state(auto: bool, center_db: f32) -> Value {
    let zeros = json!(vec![vec![0.0; 8]; 4]);
    let present = json!(vec![true; 8]);
    json!({
        "mix": {
            "auto": auto, "center_db": center_db, "lfe_db": 0.0, "surround_db": 0.0, "normalize": true,
            "matrix": zeros, "delay_ms": [0.0, 0.0, 0.0, 0.0], "invert": [false, false, false, false]
        },
        "presets": [], "live": false, "channels": 8, "present": present, "rear": true, "effective": zeros
    })
}

fn open(cx: &mut TestAppContext) -> (Harness, AnyWindowHandle, Entity<AuracastMixer>) {
    let mut h = Harness::start(cx, snapshot());
    h.answer("auracast.mix", Answer::Ok(mix_state(true, -3.0)));
    h.answer("auracast.mix_set", Answer::Ok(mix_state(true, -3.0)));
    let (w, host) = h.open_sized(cx, (1200., 900.), |window, cx| {
        let mixer = cx.new(|cx| AuracastMixer::new(window, cx));
        cx.new(|_| Host { mixer })
    });
    let mixer = cx.read(|cx| host.read(cx).mixer.clone());
    h.on(cx, w, |window, cx| mixer.update(cx, |m, cx| m.set_props(Some((true, AuracastSub::None, false)), window, cx)));
    (h, w, mixer)
}

fn field(mixer: &Entity<AuracastMixer>, cx: &TestAppContext, kind: MixField) -> Entity<InputState> {
    cx.read(|cx| mixer.read(cx).input(kind))
}

fn shown(mixer: &Entity<AuracastMixer>, cx: &TestAppContext, kind: MixField) -> String {
    let input = field(mixer, cx, kind);
    cx.read(|cx| input.read(cx).value().to_string())
}

/// Focuses the field and types `text` over what it shows
fn type_into(h: &mut Harness, cx: &mut TestAppContext, w: AnyWindowHandle, input: &Entity<InputState>, text: &str) {
    h.on(cx, w, |window, cx| input.update(cx, |s, cx| s.focus(window, cx)));
    h.key(cx, w, "ctrl-a");
    h.key(cx, w, "backspace");
    h.type_text(cx, w, text);
}

/// What the mixer sent once its pause is over
fn sent(h: &mut Harness, cx: &mut TestAppContext) -> Vec<Value> {
    cx.executor().advance_clock(Duration::from_millis(MIX_SEND_MS + 10));
    h.settle(cx);
    h.calls("auracast.mix_set").into_iter().map(|p| p["mix"].clone()).collect()
}

#[gpui_kit::test]
fn a_level_counts_on_enter_and_goes_out_after_the_pause(cx: &mut TestAppContext) {
    let (mut h, w, mixer) = open(cx);
    assert_eq!(shown(&mixer, cx, MixField::Center), "-3");
    let center = field(&mixer, cx, MixField::Center);
    type_into(&mut h, cx, w, &center, "-6");
    assert!(sent(&mut h, cx).is_empty(), "sent while typing");
    h.key(cx, w, "enter");
    let mixes = sent(&mut h, cx);
    assert_eq!(mixes.len(), 1);
    assert_eq!(mixes[0]["center_db"], json!(-6.0));
}

#[gpui_kit::test]
fn a_level_not_a_number_goes_back_and_one_out_of_range_is_held_to_it(cx: &mut TestAppContext) {
    let (mut h, w, mixer) = open(cx);
    let lfe = field(&mixer, cx, MixField::Lfe);
    for bad in ["abc", "", "-"] {
        type_into(&mut h, cx, w, &lfe, bad);
        h.key(cx, w, "enter");
        assert_eq!(shown(&mixer, cx, MixField::Lfe), "0", "{bad:?} kept");
    }
    assert!(sent(&mut h, cx).is_empty());
    // Above +12 dB: +12 goes out
    type_into(&mut h, cx, w, &lfe, "13");
    h.key(cx, w, "tab");
    assert_eq!(sent(&mut h, cx).last().map(|m| m["lfe_db"].clone()), Some(json!(12.0)));
}

#[gpui_kit::test]
fn a_field_being_typed_into_keeps_its_text_when_the_state_comes_again(cx: &mut TestAppContext) {
    let (mut h, w, mixer) = open(cx);
    let center = field(&mixer, cx, MixField::Center);
    type_into(&mut h, cx, w, &center, "-1");
    // The mixer is read again (the broadcast's rear changed)
    h.answer("auracast.mix", Answer::Ok(mix_state(true, -9.0)));
    h.on(cx, w, |window, cx| {
        mixer.update(cx, |m, cx| m.set_props(Some((false, AuracastSub::None, false)), window, cx))
    });
    assert_eq!(shown(&mixer, cx, MixField::Center), "-1");
    // The other fields show the new state
    assert_eq!(shown(&mixer, cx, MixField::Surround), "0");
    // Left with Tab: the typed value counts
    h.key(cx, w, "tab");
    let mixes = sent(&mut h, cx);
    assert_eq!(mixes.last().map(|m| m["center_db"].clone()), Some(json!(-1.0)));
}

/// The earbuds' mixer (Developer > TWS), off until turned on
fn open_tws(cx: &mut TestAppContext) -> (Harness, AnyWindowHandle, Entity<AuracastMixer>) {
    let mut h = Harness::start(cx, snapshot());
    let mut st = mix_state(true, -3.0);
    st["rear"] = json!(false);
    st["enabled"] = json!(false);
    h.answer("tws.mix", Answer::Ok(st.clone()));
    st["enabled"] = json!(true);
    h.answer("tws.mix_set", Answer::Ok(st));
    let (w, host) = h.open_sized(cx, (1200., 900.), |window, cx| {
        let mixer = cx.new(|cx| AuracastMixer::new_for(MixTarget::Tws, window, cx));
        cx.new(|_| Host { mixer })
    });
    let mixer = cx.read(|cx| host.read(cx).mixer.clone());
    h.on(cx, w, |window, cx| {
        mixer.update(cx, |m, cx| m.set_props(Some((false, AuracastSub::None, false)), window, cx))
    });
    (h, w, mixer)
}

#[gpui_kit::test]
fn the_earbuds_mixer_reads_and_saves_through_tws_and_turns_on_from_its_switch(cx: &mut TestAppContext) {
    let (mut h, w, mixer) = open_tws(cx);
    assert!(h.methods().iter().any(|m| m == "tws.mix"));
    assert!(!h.methods().iter().any(|m| m.starts_with("auracast.")));
    assert_eq!(cx.read(|cx| mixer.read(cx).output_names(AuracastSub::None).len()), 2);
    h.on(cx, w, |window, cx| mixer.update(cx, |m, cx| m.set_enabled(true, window, cx)));
    h.settle(cx);
    let calls = h.calls("tws.mix_set");
    assert_eq!(calls.last().map(|p| p["enabled"].clone()), Some(json!(true)));
    assert_eq!(cx.read(|cx| mixer.read(cx).state.as_ref().map(|s| s.enabled)), Some(true));
    // A level typed goes out through tws.mix_set too, without turning it on or off
    let center = field(&mixer, cx, MixField::Center);
    type_into(&mut h, cx, w, &center, "-6");
    h.key(cx, w, "enter");
    cx.executor().advance_clock(Duration::from_millis(MIX_SEND_MS + 10));
    h.settle(cx);
    let last = h.calls("tws.mix_set").last().cloned().unwrap();
    assert_eq!(last["mix"]["center_db"], json!(-6.0));
    assert_eq!(last["enabled"], Value::Null);
    assert!(h.calls("auracast.mix_set").is_empty());
}
