//! The dialog rules, driven by keys and clicks: Escape, the backdrop, Cancel
//! and the close control close it (and run on_cancel) unless it is busy; a
//! dialog without a confirming button has only the close control; Enter
//! presses the focused button and nothing else (the old UI's controls.tsx
//! Dialog is the spec).

use std::cell::Cell;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, TestAppContext, point, px};

use super::*;
use crate::testing::{Blank, Harness, click_at, snapshot};

#[derive(Default)]
struct Seen {
    confirmed: Cell<u32>,
    cancelled: Cell<u32>,
    busy: Cell<bool>,
}

fn open(cx: &mut TestAppContext, confirm: bool) -> (Harness, AnyWindowHandle, Rc<Seen>) {
    let mut h = Harness::start(cx, snapshot());
    let (w, _) = h.open(cx, |_, cx| cx.new(|_| Blank));
    let seen = Rc::new(Seen::default());
    let (s1, s2, s3) = (seen.clone(), seen.clone(), seen.clone());
    h.on(cx, w, move |window, cx| {
        show(
            move |_, _| {
                let q = Question::new("Title").busy(s1.busy.get()).on_cancel({
                    let s = s1.clone();
                    move |_, _| s.cancelled.set(s.cancelled.get() + 1)
                });
                if confirm { q.confirm("Go") } else { q }
            },
            move |_, _| div().child("Body").into_any_element(),
            move |_, _| s2.confirmed.set(s2.confirmed.get() + 1),
            window,
            cx,
        );
        let _ = s3;
    });
    (h, w, seen)
}

fn is_open(h: &mut Harness, cx: &mut TestAppContext, w: AnyWindowHandle) -> bool {
    h.on(cx, w, |window, cx| window.has_active_dialog(cx))
}

fn set_busy(h: &mut Harness, cx: &mut TestAppContext, w: AnyWindowHandle, seen: &Seen, busy: bool) {
    seen.busy.set(busy);
    h.on(cx, w, |window, _| window.refresh());
}

/// A place on the backdrop, outside the dialog and below the title bar's
/// height (a press up there is the window's own)
fn backdrop(h: &mut Harness, cx: &mut TestAppContext, w: AnyWindowHandle) -> gpui_kit::Point<gpui_kit::Pixels> {
    let _ = (h, cx, w);
    point(px(10.), px(300.))
}

#[gpui_kit::test]
fn escape_closes_and_runs_on_cancel(cx: &mut TestAppContext) {
    let (mut h, w, seen) = open(cx, true);
    h.on(cx, w, |window, cx| window.press("escape", cx));
    assert!(!is_open(&mut h, cx, w));
    assert_eq!(seen.cancelled.get(), 1);
    assert_eq!(seen.confirmed.get(), 0);
}

#[gpui_kit::test]
fn the_backdrop_and_cancel_close_it(cx: &mut TestAppContext) {
    let (mut h, w, seen) = open(cx, true);
    let at = backdrop(&mut h, cx, w);
    h.on(cx, w, |window, cx| click_at(window, at, cx));
    assert!(!is_open(&mut h, cx, w));
    assert_eq!(seen.cancelled.get(), 1);
    let (mut h, w, seen) = open(cx, true);
    h.on(cx, w, |window, cx| window.click("dialog-cancel", cx));
    assert!(!is_open(&mut h, cx, w));
    assert_eq!(seen.cancelled.get(), 1);
}

#[gpui_kit::test]
fn busy_blocks_escape_the_backdrop_and_cancel(cx: &mut TestAppContext) {
    let (mut h, w, seen) = open(cx, true);
    set_busy(&mut h, cx, w, &seen, true);
    h.on(cx, w, |window, cx| window.press("escape", cx));
    assert!(is_open(&mut h, cx, w));
    let at = backdrop(&mut h, cx, w);
    h.on(cx, w, |window, cx| click_at(window, at, cx));
    assert!(is_open(&mut h, cx, w));
    assert_eq!(h.on(cx, w, |window, _| window.find("dialog-cancel").disabled()), Some(true));
    assert_eq!(h.on(cx, w, |window, _| window.find("dialog-confirm").disabled()), Some(true));
    h.on(cx, w, |window, cx| window.click("dialog-cancel", cx));
    assert!(is_open(&mut h, cx, w));
    assert_eq!(seen.cancelled.get(), 0);
    // No longer busy: Escape closes it again
    set_busy(&mut h, cx, w, &seen, false);
    h.on(cx, w, |window, cx| window.press("escape", cx));
    assert!(!is_open(&mut h, cx, w));
    assert_eq!(seen.cancelled.get(), 1);
}

#[gpui_kit::test]
fn without_confirm_only_the_close_control(cx: &mut TestAppContext) {
    let (mut h, w, seen) = open(cx, false);
    let (cancel, confirm, close) = h.on(cx, w, |window, _| {
        (
            window.try_find("dialog-cancel").is_some(),
            window.try_find("dialog-confirm").is_some(),
            window.try_find("dialog-close").is_some(),
        )
    });
    assert!(!cancel && !confirm && close);
    h.on(cx, w, |window, cx| window.click("dialog-close", cx));
    assert!(!is_open(&mut h, cx, w));
    assert_eq!(seen.cancelled.get(), 1);
}

#[gpui_kit::test]
fn the_close_control_does_nothing_while_busy(cx: &mut TestAppContext) {
    let (mut h, w, seen) = open(cx, false);
    set_busy(&mut h, cx, w, &seen, true);
    h.on(cx, w, |window, cx| window.click("dialog-close", cx));
    assert!(is_open(&mut h, cx, w));
    h.on(cx, w, |window, cx| window.press("escape", cx));
    assert!(is_open(&mut h, cx, w));
    assert_eq!(seen.cancelled.get(), 0);
}

/// A window with a button of its own behind the dialog
struct Behind;

impl gpui_kit::Render for Behind {
    fn render(&mut self, _: &mut Window, _: &mut gpui_kit::Context<Self>) -> impl IntoElement {
        div().size_full().child(button("behind", "Behind", ButtonKind::Default).on_click(|_, _, _| {}))
    }
}

#[gpui_kit::test]
fn tab_keeps_the_keyboard_inside_the_dialog(cx: &mut TestAppContext) {
    let mut h = Harness::start(cx, snapshot());
    let (w, _) = h.open(cx, |_, cx| cx.new(|_| Behind));
    h.on(cx, w, |window, cx| {
        ask(Question::new("Title").confirm("Go"), |_, _| div().child("Body").into_any_element(), |_, _| {}, window, cx)
    });
    let focused = |h: &mut Harness, cx: &mut TestAppContext| {
        h.on(cx, w, |window, _| {
            ["dialog-cancel", "dialog-confirm", "behind"]
                .into_iter()
                .find(|id| window.try_find(*id).is_some_and(|e| e.focused() == Some(true)))
        })
    };
    let mut seen = Vec::new();
    for key in ["tab"; 6].into_iter().chain(["shift-tab"; 4]) {
        h.on(cx, w, |window, cx| window.press(key, cx));
        seen.push(focused(&mut h, cx));
    }
    assert!(!seen.contains(&Some("behind")), "the keyboard left the dialog: {seen:?}");
    assert!(seen.contains(&Some("dialog-cancel")) && seen.contains(&Some("dialog-confirm")), "{seen:?}");
    // Tab goes round: each button comes again
    assert!(seen.iter().filter(|s| **s == Some("dialog-confirm")).count() >= 2, "{seen:?}");
}

#[gpui_kit::test]
fn a_dialog_is_as_wide_as_the_page(cx: &mut TestAppContext) {
    // A dialog with content of its own (no confirming button)
    let (mut h, w, _) = open(cx, false);
    let width =
        |h: &mut Harness, cx: &mut TestAppContext| h.on(cx, w, |window, _| window.find("surface").bounds().size.width);
    // No page (the setup): the window less the margins
    assert_eq!(width(&mut h, cx), px(1000. - 48.));
    // Beside the navigation: the page less its padding
    h.on(cx, w, |window, cx| set_page_width(window, Some(px(760.)), cx));
    h.on(cx, w, |window, _| window.refresh());
    assert_eq!(width(&mut h, cx), px(760. - 48.));
    // Never wider than the page's cards
    h.on(cx, w, |window, cx| set_page_width(window, Some(px(3000.)), cx));
    h.on(cx, w, |window, _| window.refresh());
    assert_eq!(width(&mut h, cx), px(1000. - 48.));
}

#[gpui_kit::test]
fn a_question_is_narrow(cx: &mut TestAppContext) {
    let (mut h, w, _) = open(cx, true);
    let width =
        |h: &mut Harness, cx: &mut TestAppContext| h.on(cx, w, |window, _| window.find("surface").bounds().size.width);
    // Cancel and a confirming button: no wider than Windows' own questions
    assert_eq!(width(&mut h, cx), px(560.));
    // In a narrow page: the page less its padding
    h.on(cx, w, |window, cx| set_page_width(window, Some(px(400.)), cx));
    h.on(cx, w, |window, _| window.refresh());
    assert_eq!(width(&mut h, cx), px(400. - 48.));
}

#[gpui_kit::test]
fn enter_presses_the_focused_button_only(cx: &mut TestAppContext) {
    let (mut h, w, seen) = open(cx, true);
    // Nothing of the dialog focused by a click: Enter confirms nothing
    h.on(cx, w, |window, cx| window.press("enter", cx));
    assert!(is_open(&mut h, cx, w));
    assert_eq!(seen.confirmed.get(), 0);
    // Tab to a button, then Enter presses that one
    let mut pressed = None;
    for _ in 0..4 {
        h.on(cx, w, |window, cx| window.press("tab", cx));
        let focus = h.on(cx, w, |window, _| {
            ["dialog-cancel", "dialog-confirm"].into_iter().find(|id| window.find(*id).focused() == Some(true))
        });
        if let Some(id) = focus {
            pressed = Some(id);
            break;
        }
    }
    let pressed = pressed.expect("Tab reaches the dialog's buttons");
    h.on(cx, w, |window, cx| window.press("enter", cx));
    if pressed == "dialog-confirm" {
        assert_eq!((seen.confirmed.get(), seen.cancelled.get()), (1, 0));
    } else {
        assert_eq!((seen.confirmed.get(), seen.cancelled.get()), (0, 1));
        assert!(!is_open(&mut h, cx, w));
    }
}
