//! An app's volume slider in the Volume Mixer dragged: at most ten requests
//! a second and the last value; a refresh does not move the thumb while it
//! is held (the old UI's Mixer.test.tsx is the spec).

use gpui_kit::test::{TestEventExt as _, TestWindowExt as _};
use gpui_kit::{AnyWindowHandle, MouseButton, Pixels, Point, TestAppContext, point};
use serde_json::json;

use super::*;
use crate::testing::{Answer, Harness, snapshot};

fn open(cx: &mut TestAppContext) -> (Harness, AnyWindowHandle, Entity<MixerList>) {
    let mut h = Harness::start(cx, snapshot());
    h.answer("audio.mixer", Answer::Ok(json!([{"key": "player.exe", "name": "Player", "volume": 50, "muted": false}])));
    h.answer("audio.mixer_set", Answer::Ok(serde_json::Value::Null));
    let (w, list) = h.open_sized(cx, (500., 200.), |window, cx| cx.new(|cx| MixerList::new(window, cx)));
    (h, w, list)
}

fn track_at(h: &mut Harness, cx: &mut TestAppContext, w: AnyWindowHandle, f: f32) -> Point<Pixels> {
    h.on(cx, w, |window, _| {
        let b = window.find("slider-bar-container").bounds();
        point(b.left() + b.size.width * f, b.center().y)
    })
}

fn volumes(h: &Harness) -> Vec<u64> {
    h.calls("audio.mixer_set").iter().filter_map(|p| p["volume"].as_u64()).collect()
}

#[gpui_kit::test]
fn a_drag_sends_at_most_ten_a_second_and_the_last_value(cx: &mut TestAppContext) {
    let (mut h, w, list) = open(cx);
    assert_eq!(cx.read(|cx| list.read(cx).rows.len()), 1);
    let start = track_at(&mut h, cx, w, 0.2);
    h.on(cx, w, |window, cx| {
        window.pointer_move(start, None, cx);
        window.pointer_down(start, MouseButton::Left, cx);
    });
    assert_eq!(volumes(&h).len(), 1);
    // A second of moves, one every 20 ms
    for step in 0..50u32 {
        let at = track_at(&mut h, cx, w, 0.2 + 0.6 * step as f32 / 50.);
        h.on(cx, w, |window, cx| window.pointer_move(at, Some(MouseButton::Left), cx));
        cx.executor().advance_clock(Duration::from_millis(20));
        h.settle(cx);
    }
    let n = volumes(&h).len();
    assert!((9..=12).contains(&n), "{n} requests in a second");
    let end = track_at(&mut h, cx, w, 0.8);
    h.on(cx, w, |window, cx| {
        window.pointer_move(end, Some(MouseButton::Left), cx);
        window.pointer_up(end, MouseButton::Left, cx);
    });
    cx.executor().advance_clock(Duration::from_millis(150));
    h.settle(cx);
    let held = cx.read(|cx| list.read(cx).rows[0].slider.read(cx).value().start().round() as u64);
    assert_eq!(*volumes(&h).last().unwrap(), held);
    assert_eq!(h.calls("audio.mixer_set")[0]["app"], "player.exe");
}

#[gpui_kit::test]
fn a_refresh_does_not_move_a_held_thumb(cx: &mut TestAppContext) {
    let (mut h, w, list) = open(cx);
    let at = track_at(&mut h, cx, w, 0.9);
    h.on(cx, w, |window, cx| {
        window.pointer_move(at, None, cx);
        window.pointer_down(at, MouseButton::Left, cx);
    });
    let held = cx.read(|cx| list.read(cx).rows[0].held);
    assert!(held.is_some_and(|v| v > 80));
    // The refresh (every two seconds) still says 50
    cx.executor().advance_clock(Duration::from_millis(REFRESH_MS + 10));
    h.settle(cx);
    let value = cx.read(|cx| list.read(cx).rows[0].slider.read(cx).value().start().round() as u8);
    assert_eq!(Some(value), held);
    h.on(cx, w, |window, cx| window.pointer_up(at, MouseButton::Left, cx));
}
