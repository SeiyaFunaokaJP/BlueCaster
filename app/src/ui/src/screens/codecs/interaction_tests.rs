//! The codec cards driven as a user does: dragged into an order, moved with
//! Alt+arrow keys, chosen with a click, Enter or Space, and the bitpool's
//! two handles in a codec's "..." dialog (the old UI's codecs.test.tsx is
//! the spec).

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, MouseButton, Point, TestAppContext, point, px};
use serde_json::{Value, json};

use super::*;
use crate::testing::{Answer, Harness, move_to, press_at, profile, release_at, snapshot};

const ADDR: &str = "0A:00:00:00:00:02";
const PROFILE: &str = "Test Buds | SBC";

fn state(auto: bool, codec: &str) -> Value {
    let mut s = snapshot();
    s["settings"]["auto_codec"] = json!(auto);
    s["devices"] = json!([{
        "address": ADDR, "name": "Test Buds", "saved": true, "audio": true,
        "codecs": ["ldac", "aac", "sbc"],
        "sbc": {"sample_rates": [44100, 48000], "channel_modes": ["joint_stereo"], "blocks": [16],
                "subbands": [8], "allocations": ["loudness"], "min_bitpool": 2, "max_bitpool": 53},
        "codec_caps": [
            {"codec": "ldac", "sample_rates": [44100, 48000], "bit_depths": [], "max_bitrate": 0, "lhdcv5_low_latency": false},
            {"codec": "aac", "sample_rates": [44100, 48000], "bit_depths": [], "max_bitrate": 320000, "lhdcv5_low_latency": false},
            {"codec": "sbc", "sample_rates": [44100, 48000], "bit_depths": [], "max_bitrate": 0, "lhdcv5_low_latency": false}
        ]
    }]);
    s["profiles"] = json!([profile(PROFILE, ADDR, "Test Buds", json!({"codec": codec}))]);
    s
}

fn open(cx: &mut TestAppContext, auto: bool, codec: &str) -> (Harness, AnyWindowHandle, Entity<Codecs>) {
    let mut h = Harness::start(cx, state(auto, codec));
    h.answer("audio.devices", Answer::Ok(json!([])));
    let (w, view) = h.open_sized(cx, (1000., 1400.), |window, cx| cx.new(|cx| Codecs::new(window, cx)));
    (h, w, view)
}

fn section(view: &Entity<Codecs>, cx: &TestAppContext) -> Entity<DeviceCodecs> {
    cx.read(|cx| view.read(cx).sections[0].1.clone())
}

/// A point on card `id`'s face, left of its "..." button
fn card_at(h: &mut Harness, cx: &mut TestAppContext, w: AnyWindowHandle, id: &str) -> Point<Pixels> {
    let more = format!("codec-{ADDR}-{id}-more");
    h.on(cx, w, |window, _| {
        let b = window.find(SharedString::from(more)).bounds();
        point(b.origin.x - px(40.), b.center().y + px(8.))
    })
}

fn saved_order(h: &Harness) -> Vec<String> {
    serde_json::from_value(h.last_saved()["codec_order"].clone()).unwrap()
}

fn before(order: &[String], a: &str, b: &str) -> bool {
    let at = |x: &str| order.iter().position(|c| c == x).unwrap_or(usize::MAX);
    at(a) < at(b)
}

fn focus_card(h: &mut Harness, cx: &mut TestAppContext, w: AnyWindowHandle, view: &Entity<Codecs>, id: &str) {
    let s = section(view, cx);
    let handle = cx.read(|cx| s.read(cx).focus.get(id).cloned()).expect("the card has a focus handle");
    h.on(cx, w, |window, cx| handle.focus(window, cx));
}

#[gpui_kit::test]
fn a_dragged_card_takes_its_place_and_the_order_is_saved_on_drop(cx: &mut TestAppContext) {
    let (mut h, w, _view) = open(cx, false, "sbc");
    let from = card_at(&mut h, cx, w, "aac");
    let to = card_at(&mut h, cx, w, "ldac");
    h.on(cx, w, |window, cx| {
        move_to(window, from, false, cx);
        press_at(window, from, MouseButton::Left, 1, cx);
    });
    for step in 1..=6 {
        let f = step as f32 / 6.;
        let at = point(from.x + (to.x - from.x) * f, from.y + (to.y - from.y) * f);
        h.on(cx, w, |window, cx| move_to(window, at, true, cx));
    }
    // Nothing saved while it is dragged
    assert!(h.calls("profile.save").is_empty());
    h.on(cx, w, |window, cx| release_at(window, to, MouseButton::Left, 1, cx));
    let saves = h.calls("profile.save");
    assert_eq!(saves.len(), 1, "{saves:?}");
    let order = saved_order(&h);
    assert!(before(&order, "aac", "ldac"), "{order:?}");
    // The drop chose nothing
    assert_eq!(h.last_saved()["codec"], "sbc");
}

#[gpui_kit::test]
fn a_click_without_moving_chooses_the_card(cx: &mut TestAppContext) {
    let (mut h, w, _view) = open(cx, false, "sbc");
    let at = card_at(&mut h, cx, w, "aac");
    h.on(cx, w, |window, cx| crate::testing::click_at(window, at, cx));
    assert_eq!(h.calls("profile.save").len(), 1);
    assert_eq!(h.last_saved()["codec"], "aac");
    // The chosen card again: nothing
    h.on(cx, w, |window, cx| crate::testing::click_at(window, at, cx));
    assert_eq!(h.calls("profile.save").len(), 1);
}

#[gpui_kit::test]
fn alt_arrows_move_a_focused_card(cx: &mut TestAppContext) {
    let (mut h, w, view) = open(cx, false, "sbc");
    focus_card(&mut h, cx, w, &view, "aac");
    h.on(cx, w, |window, cx| window.press("alt-left", cx));
    assert_eq!(h.calls("profile.save").len(), 1);
    assert!(before(&saved_order(&h), "aac", "ldac"));
    // First among those shown: no further
    h.on(cx, w, |window, cx| window.press("alt-up", cx));
    assert_eq!(h.calls("profile.save").len(), 1);
    // The card moved keeps the focus
    let s = section(&view, cx);
    let kept = h.on(cx, w, |window, cx| s.read(cx).focus["aac"].is_focused(window));
    assert!(kept);
    h.on(cx, w, |window, cx| window.press("alt-down", cx));
    assert_eq!(h.calls("profile.save").len(), 2);
    // Back where it was: the default order, saved as none
    assert!(saved_order(&h).is_empty(), "{:?}", saved_order(&h));
    // Plain arrows move nothing
    h.on(cx, w, |window, cx| window.press("right", cx));
    assert_eq!(h.calls("profile.save").len(), 2);
}

#[gpui_kit::test]
fn enter_and_space_choose_the_focused_card(cx: &mut TestAppContext) {
    let (mut h, w, view) = open(cx, false, "sbc");
    focus_card(&mut h, cx, w, &view, "aac");
    h.on(cx, w, |window, cx| window.press("enter", cx));
    assert_eq!(h.calls("profile.save").len(), 1);
    assert_eq!(h.last_saved()["codec"], "aac");
    focus_card(&mut h, cx, w, &view, "ldac");
    h.on(cx, w, |window, cx| window.press("space", cx));
    assert_eq!(h.calls("profile.save").len(), 2);
    assert_eq!(h.last_saved()["codec"], "ldac");
    // The chosen one: nothing
    h.on(cx, w, |window, cx| window.press("enter", cx));
    assert_eq!(h.calls("profile.save").len(), 2);
}

#[gpui_kit::test]
fn under_auto_select_the_keys_choose_and_move_nothing(cx: &mut TestAppContext) {
    let (mut h, w, view) = open(cx, true, "sbc");
    focus_card(&mut h, cx, w, &view, "aac");
    h.on(cx, w, |window, cx| {
        window.press("alt-left", cx);
        window.press("enter", cx);
    });
    assert!(h.calls("profile.save").is_empty());
}

#[gpui_kit::test]
fn the_bitpool_handles_move_apart_and_never_cross(cx: &mut TestAppContext) {
    let (mut h, w, _view) = open(cx, false, "sbc");
    let more = SharedString::from(format!("codec-{ADDR}-sbc-more"));
    h.on(cx, w, |window, cx| window.click(more, cx));
    // The thumbs are named by whether they start the range: 1 the low one
    let thumb = |h: &mut Harness, cx: &mut TestAppContext, low: bool| {
        h.on(cx, w, |window, _| window.find(("slider-thumb", u32::from(low))).bounds().center())
    };
    let bitpools = |h: &Harness| {
        let p = h.last_saved();
        (p["sbc_min_bitpool"].as_u64().unwrap(), p["sbc_bitpool"].as_u64().unwrap())
    };
    // The low handle dragged to the right: the lowest bitpool rises, the
    // highest stays on Auto
    let low = thumb(&mut h, cx, true);
    h.on(cx, w, |window, cx| window.drag(low, point(low.x + px(80.), low.y), cx));
    assert!(h.calls("profile.save").is_empty(), "saved while the handle moves");
    cx.executor().advance_clock(Duration::from_millis(SAVE_DELAY_MS + 10));
    h.settle(cx);
    assert_eq!(h.calls("profile.save").len(), 1);
    let (lo, hi) = bitpools(&h);
    assert!(lo > 2, "low {lo}");
    assert_eq!(hi, 0);
    // The high handle dragged left past the low one stops at it
    let high = thumb(&mut h, cx, false);
    let low = thumb(&mut h, cx, true);
    h.on(cx, w, |window, cx| window.drag(high, point(low.x - px(100.), high.y), cx));
    cx.executor().advance_clock(Duration::from_millis(SAVE_DELAY_MS + 10));
    h.settle(cx);
    let (lo2, hi2) = bitpools(&h);
    assert!(hi2 != 0 && lo2 <= hi2, "low {lo2} high {hi2}");
    assert_eq!(lo2, lo, "the low handle stays where it was");
    // The low handle dragged far right stops at the high one
    let low = thumb(&mut h, cx, true);
    h.on(cx, w, |window, cx| window.drag(low, point(low.x + px(400.), low.y), cx));
    cx.executor().advance_clock(Duration::from_millis(SAVE_DELAY_MS + 10));
    h.settle(cx);
    let (lo3, hi3) = bitpools(&h);
    assert!(lo3 <= hi3, "low {lo3} high {hi3}");
    // Auto puts both back
    h.on(cx, w, |window, cx| window.click("bitpool-auto", cx));
    cx.executor().advance_clock(Duration::from_millis(SAVE_DELAY_MS + 10));
    h.settle(cx);
    let p = h.last_saved();
    assert_eq!((p["sbc_min_bitpool"].as_u64(), p["sbc_bitpool"].as_u64()), (Some(0), Some(0)));
}
