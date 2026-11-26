//! The keyboards-and-mice dialog's gate, driven as a user does: both boxes
//! ticked, the code typed (never pasted, six characters at most), the wait
//! over; Enter and the button confirm only then, and nothing closes it while
//! the request runs (the old UI's hidHost.test.tsx is the spec).

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, ClipboardItem, ElementId, TestAppContext};
use serde_json::json;
use ui_model::view::hid::HID_WAIT_SECONDS;

use super::*;
use crate::testing::{Answer, Blank, Harness, snapshot};

const CODE: &str = "482913";

fn open(cx: &mut TestAppContext) -> (Harness, AnyWindowHandle, Entity<HidEnableDialog>) {
    let mut h = Harness::start(cx, snapshot());
    h.answer("hid.code", Answer::Ok(json!({"code": CODE})));
    let (w, _) = h.open(cx, |_, cx| cx.new(|_| Blank));
    let state = h.on(cx, w, |window, cx| {
        let state = cx.new(|cx| HidEnableDialog::new(window, cx));
        let s = state.clone();
        window.open_dialog(cx, move |dialog, _, cx| HidEnableDialog::build(&s, dialog, cx));
        state
    });
    (h, w, state)
}

fn typed(state: &Entity<HidEnableDialog>, cx: &TestAppContext) -> String {
    cx.read(|cx| state.read(cx).typed(cx))
}

fn focus_code(h: &mut Harness, cx: &mut TestAppContext, w: AnyWindowHandle, state: &Entity<HidEnableDialog>) {
    let input = cx.read(|cx| state.read(cx).input.clone());
    h.on(cx, w, |window, cx| input.update(cx, |s, cx| s.focus(window, cx)));
}

fn wait_out(h: &mut Harness, cx: &mut TestAppContext) {
    cx.executor().advance_clock(Duration::from_secs(u64::from(HID_WAIT_SECONDS)) + Duration::from_millis(300));
    h.settle(cx);
}

fn tick_both(h: &mut Harness, cx: &mut TestAppContext, w: AnyWindowHandle) {
    h.on(cx, w, |window, cx| {
        window.click("hid-confirm-other", cx);
        window.click("hid-confirm-risk", cx);
    });
}

fn dialog_open(h: &mut Harness, cx: &mut TestAppContext, w: AnyWindowHandle) -> bool {
    h.on(cx, w, |window, cx| window.has_active_dialog(cx))
}

fn confirm_enabled(h: &mut Harness, cx: &mut TestAppContext, w: AnyWindowHandle) -> bool {
    h.on(cx, w, |window, _| window.find("dialog-confirm").disabled() != Some(true))
}

#[gpui_kit::test]
fn the_code_field_takes_six_characters_and_no_paste(cx: &mut TestAppContext) {
    let (mut h, w, state) = open(cx);
    focus_code(&mut h, cx, w, &state);
    h.on(cx, w, |window, cx| window.input("12345678", cx));
    assert_eq!(typed(&state, cx), "123456");
    // Pasted: nothing arrives, also into an empty field
    h.on(cx, w, |window, cx| {
        window.press("ctrl-a", cx);
        window.press("backspace", cx);
    });
    assert_eq!(typed(&state, cx), "");
    cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string(CODE.to_owned())));
    h.on(cx, w, |window, cx| window.press("ctrl-v", cx));
    assert_eq!(typed(&state, cx), "");
    // Shift+Insert pastes too on Windows
    h.on(cx, w, |window, cx| window.press("shift-insert", cx));
    assert_eq!(typed(&state, cx), "");
}

#[gpui_kit::test]
fn waits_for_both_statements_the_code_and_the_countdown(cx: &mut TestAppContext) {
    let (mut h, w, state) = open(cx);
    assert_eq!(h.calls("hid.code").len(), 1);
    focus_code(&mut h, cx, w, &state);
    h.on(cx, w, |window, cx| window.input(CODE, cx));
    tick_both(&mut h, cx, w);
    // The countdown runs: not yet
    assert!(!confirm_enabled(&mut h, cx, w));
    focus_code(&mut h, cx, w, &state);
    h.on(cx, w, |window, cx| window.press("enter", cx));
    assert!(h.calls("hid.enable").is_empty());
    cx.executor().advance_clock(Duration::from_secs(u64::from(HID_WAIT_SECONDS) - 1));
    h.settle(cx);
    assert!(!confirm_enabled(&mut h, cx, w));
    assert_eq!(cx.read(|cx| state.read(cx).left), 1);
    cx.executor().advance_clock(Duration::from_millis(1300));
    h.settle(cx);
    assert_eq!(cx.read(|cx| state.read(cx).left), 0);
    assert!(confirm_enabled(&mut h, cx, w));
    h.answer("hid.enable", Answer::Ok(serde_json::Value::Null));
    focus_code(&mut h, cx, w, &state);
    h.on(cx, w, |window, cx| window.press("enter", cx));
    let sent = h.calls("hid.enable");
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0], json!({"code": CODE, "other_input_at_hand": true, "risk_understood": true}));
    assert!(!dialog_open(&mut h, cx, w));
}

#[gpui_kit::test]
fn stays_shut_with_a_box_unticked_or_the_code_short(cx: &mut TestAppContext) {
    let (mut h, w, state) = open(cx);
    wait_out(&mut h, cx);
    focus_code(&mut h, cx, w, &state);
    h.on(cx, w, |window, cx| window.input(CODE, cx));
    // No box ticked
    assert!(!confirm_enabled(&mut h, cx, w));
    h.on(cx, w, |window, cx| window.press("enter", cx));
    // One box ticked
    h.on(cx, w, |window, cx| window.click("hid-confirm-other", cx));
    assert!(!confirm_enabled(&mut h, cx, w));
    focus_code(&mut h, cx, w, &state);
    h.on(cx, w, |window, cx| window.press("enter", cx));
    h.on(cx, w, |window, cx| window.click("dialog-confirm", cx));
    // Both, the code one digit short
    h.on(cx, w, |window, cx| window.click("hid-confirm-risk", cx));
    focus_code(&mut h, cx, w, &state);
    h.on(cx, w, |window, cx| window.press("backspace", cx));
    assert!(!confirm_enabled(&mut h, cx, w));
    h.on(cx, w, |window, cx| window.press("enter", cx));
    h.on(cx, w, |window, cx| window.click("dialog-confirm", cx));
    // Six characters, one of them no digit
    focus_code(&mut h, cx, w, &state);
    h.on(cx, w, |window, cx| window.input("x", cx));
    assert_eq!(typed(&state, cx).chars().count(), 6);
    assert!(!confirm_enabled(&mut h, cx, w));
    h.on(cx, w, |window, cx| window.press("enter", cx));
    assert!(h.calls("hid.enable").is_empty());
    assert!(dialog_open(&mut h, cx, w));
}

#[gpui_kit::test]
fn a_box_unticked_again_shuts_the_gate(cx: &mut TestAppContext) {
    let (mut h, w, state) = open(cx);
    wait_out(&mut h, cx);
    focus_code(&mut h, cx, w, &state);
    h.on(cx, w, |window, cx| window.input(CODE, cx));
    tick_both(&mut h, cx, w);
    assert!(confirm_enabled(&mut h, cx, w));
    h.on(cx, w, |window, cx| window.click("hid-confirm-risk", cx));
    assert!(!confirm_enabled(&mut h, cx, w));
    focus_code(&mut h, cx, w, &state);
    h.on(cx, w, |window, cx| window.press("enter", cx));
    assert!(h.calls("hid.enable").is_empty());
}

#[gpui_kit::test]
fn nothing_closes_it_while_the_request_runs_and_a_wrong_code_needs_a_new_one(cx: &mut TestAppContext) {
    let (mut h, w, state) = open(cx);
    wait_out(&mut h, cx);
    tick_both(&mut h, cx, w);
    focus_code(&mut h, cx, w, &state);
    h.on(cx, w, |window, cx| window.input(CODE, cx));
    h.answer("hid.enable", Answer::Hold);
    h.on(cx, w, |window, cx| window.click("dialog-confirm", cx));
    assert_eq!(h.calls("hid.enable").len(), 1);
    // Busy: Escape, Cancel and Enter again do nothing
    h.on(cx, w, |window, cx| window.press("escape", cx));
    assert!(dialog_open(&mut h, cx, w));
    assert_eq!(h.on(cx, w, |window, _| window.find("dialog-cancel").disabled()), Some(true));
    h.on(cx, w, |window, cx| window.click("dialog-cancel", cx));
    assert!(dialog_open(&mut h, cx, w));
    focus_code(&mut h, cx, w, &state);
    h.on(cx, w, |window, cx| window.press("enter", cx));
    assert_eq!(h.calls("hid.enable").len(), 1);
    // A wrong code: its error, the field emptied, a new code asked for
    h.release(cx, "hid.enable", Err("hid.wrong_code"));
    assert!(dialog_open(&mut h, cx, w));
    assert!(cx.read(|cx| state.read(cx).error.is_some()));
    assert_eq!(typed(&state, cx), "");
    assert_eq!(h.calls("hid.code").len(), 2);
    assert!(!confirm_enabled(&mut h, cx, w));
    // Not busy any more: Escape closes it, nothing turned on
    h.on(cx, w, |window, cx| window.press("escape", cx));
    assert!(!dialog_open(&mut h, cx, w));
    assert_eq!(h.calls("hid.enable").len(), 1);
}

#[gpui_kit::test]
fn a_reopened_dialog_starts_over(cx: &mut TestAppContext) {
    let (mut h, w, state) = open(cx);
    wait_out(&mut h, cx);
    tick_both(&mut h, cx, w);
    focus_code(&mut h, cx, w, &state);
    h.on(cx, w, |window, cx| window.input(CODE, cx));
    h.on(cx, w, |window, cx| window.click("dialog-cancel", cx));
    assert!(!dialog_open(&mut h, cx, w));
    h.on(cx, w, open_hid_enable_dialog);
    assert!(dialog_open(&mut h, cx, w));
    // The first one may still be fading out: the second's button
    let second = ElementId::NamedInteger("dialog-instance".into(), 1);
    let disabled = h.on(cx, w, |window, _| window.within(second).find("dialog-confirm").disabled());
    assert_eq!(disabled, Some(true));
    assert_eq!(h.calls("hid.code").len(), 2);
}
