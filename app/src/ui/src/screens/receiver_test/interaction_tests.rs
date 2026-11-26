//! The receiver's address field typed into: it starts with the saved
//! address (empty for Auto), keeps what is typed for the session, and Enter
//! connects to it.

use gpui_kit::{AnyWindowHandle, Entity, TestAppContext};
use serde_json::json;

use super::*;
use crate::testing::{Harness, snapshot};

fn open(cx: &mut TestAppContext, saved: &str) -> (Harness, AnyWindowHandle, Entity<ReceiverTest>) {
    let mut s = snapshot();
    s["settings"]["remote_sink"] = json!(saved);
    let mut h = Harness::start(cx, s);
    let (w, view) = h.open_sized(cx, (1000., 1400.), |window, cx| cx.new(|cx| ReceiverTest::new(window, cx)));
    (h, w, view)
}

fn shown(view: &Entity<ReceiverTest>, cx: &TestAppContext) -> String {
    cx.read(|cx| view.read(cx).address.read(cx).value().to_string())
}

fn targets(h: &Harness) -> Vec<serde_json::Value> {
    h.calls("receiver.connect").into_iter().map(|p| p["target"].clone()).collect()
}

#[gpui_kit::test]
fn the_field_starts_with_the_saved_address_and_auto_is_empty(cx: &mut TestAppContext) {
    let (_h, _w, view) = open(cx, "192.168.1.20");
    assert_eq!(shown(&view, cx), "192.168.1.20");
    let (_h, _w, view) = open(cx, "auto");
    assert_eq!(shown(&view, cx), "");
}

#[gpui_kit::test]
fn enter_connects_to_the_typed_address(cx: &mut TestAppContext) {
    let (mut h, w, view) = open(cx, "auto");
    let input = cx.read(|cx| view.read(cx).address.clone());
    h.on(cx, w, |window, cx| input.update(cx, |s, cx| s.focus(window, cx)));
    h.type_text(cx, w, "192.168.1.5");
    assert_eq!(shown(&view, cx), "192.168.1.5");
    // Kept for the session (the view opened again shows it)
    assert_eq!(cx.read(|cx| view.read(cx).session.read(cx).address.clone()), Some("192.168.1.5".to_owned()));
    let before = targets(&h).len();
    h.key(cx, w, "enter");
    let sent = targets(&h);
    assert_eq!(sent.len(), before + 1, "Enter connects");
    assert_eq!(sent.last().cloned(), Some(serde_json::to_value(connect_target("192.168.1.5")).unwrap()));
}
