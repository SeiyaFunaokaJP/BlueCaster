//! The volume slider dragged: the first value goes at once, then at most ten
//! a second and the last one; the thumb stays under the pointer while the
//! core's older values come back, and follows the core again after it is let
//! go (the old UI's stream.tsx VolumeControl is the spec).

use gpui_kit::test::{TestEventExt as _, TestWindowExt as _};
use gpui_kit::{AnyWindowHandle, MouseButton, Pixels, Point, TestAppContext, point};
use serde_json::json;

use super::*;
use crate::testing::{Harness, snapshot};

fn open(cx: &mut TestAppContext) -> (Harness, AnyWindowHandle, Entity<VolumeControl>) {
    let mut s = snapshot();
    s["stream"] = json!({"state": "streaming", "message": {"key": "status.ready", "args": []}, "volume": 64});
    let mut h = Harness::start(cx, s);
    let (w, v) = h.open_sized(cx, (500., 200.), |window, cx| cx.new(|cx| VolumeControl::new(false, window, cx)));
    (h, w, v)
}

/// A place on the track, `f` of its length from its start
fn track_at(h: &mut Harness, cx: &mut TestAppContext, w: AnyWindowHandle, f: f32) -> Point<Pixels> {
    h.on(cx, w, |window, _| {
        let b = window.find("slider-bar-container").bounds();
        point(b.left() + b.size.width * f, b.center().y)
    })
}

fn sent(h: &Harness) -> Vec<u64> {
    h.calls("stream.set_volume").iter().map(|p| p["volume"].as_u64().unwrap()).collect()
}

fn shown(v: &Entity<VolumeControl>, cx: &TestAppContext) -> u32 {
    cx.read(|cx| v.read(cx).shown(cx))
}

fn wire(pct: u32) -> u64 {
    u64::from(percent_to_volume(pct).min(127))
}

#[gpui_kit::test]
fn a_drag_sends_at_most_ten_a_second_and_the_last_value(cx: &mut TestAppContext) {
    let (mut h, w, v) = open(cx);
    let start = track_at(&mut h, cx, w, 0.3);
    h.on(cx, w, |window, cx| {
        window.pointer_move(start, None, cx);
        window.pointer_down(start, MouseButton::Left, cx);
    });
    // The press itself: at once
    let first = sent(&h);
    assert_eq!(first.len(), 1, "{first:?}");
    let pressed = shown(&v, cx);
    assert!((28..=32).contains(&pressed), "{pressed}");
    assert_eq!(first[0], wire(pressed));
    // Moves within 100 ms: nothing more yet
    for f in [0.4, 0.5, 0.6] {
        let at = track_at(&mut h, cx, w, f);
        h.on(cx, w, |window, cx| window.pointer_move(at, Some(MouseButton::Left), cx));
    }
    assert_eq!(sent(&h).len(), 1);
    let held = shown(&v, cx);
    assert!((58..=62).contains(&held), "{held}");
    cx.executor().advance_clock(Duration::from_millis(110));
    h.settle(cx);
    assert_eq!(sent(&h), vec![first[0], wire(held)]);
    // The core's echo of the first value does not take the thumb back
    h.edit(cx, |s| s["stream"]["volume"] = json!(first[0]));
    assert_eq!(shown(&v, cx), held);
    let end = track_at(&mut h, cx, w, 0.6);
    h.on(cx, w, |window, cx| window.pointer_up(end, MouseButton::Left, cx));
    cx.executor().advance_clock(Duration::from_millis(110));
    h.settle(cx);
    assert_eq!(*sent(&h).last().unwrap(), wire(held));
    // Let go: the core's value shows again after a moment
    cx.executor().advance_clock(Duration::from_millis(700));
    h.settle(cx);
    assert_eq!(shown(&v, cx), volume_percent(first[0] as u32));
    h.edit(cx, |s| s["stream"]["volume"] = json!(wire(held)));
    assert_eq!(shown(&v, cx), volume_percent(wire(held) as u32));
}

#[gpui_kit::test]
fn the_keys_move_the_volume_too(cx: &mut TestAppContext) {
    let (mut h, w, v) = open(cx);
    let before = shown(&v, cx);
    let slider = cx.read(|cx| v.read(cx).slider.entity_id());
    let id = ElementId::NamedInteger("slider".into(), slider.as_u64());
    // Tab reaches it
    let mut reached = false;
    for _ in 0..4 {
        h.on(cx, w, |window, cx| window.press("tab", cx));
        if h.on(cx, w, |window, _| window.find(id.clone()).focused()) == Some(true) {
            reached = true;
            break;
        }
    }
    assert!(reached, "Tab never reached the volume slider");
    let n = sent(&h).len();
    h.on(cx, w, |window, cx| window.press("right", cx));
    cx.executor().advance_clock(Duration::from_millis(110));
    h.settle(cx);
    assert!(sent(&h).len() > n);
    assert!(shown(&v, cx) > before || *sent(&h).last().unwrap() > wire(before));
}
