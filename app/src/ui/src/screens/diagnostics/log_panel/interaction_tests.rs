//! The log list driven as a user does: a click selects a line, Ctrl+click
//! adds or removes one, Shift+click a range; Ctrl+A selects what the filter
//! shows, Ctrl+C copies the selection, Escape lets it go; the search box
//! filters as it is typed (the old UI's Diagnostics.test.tsx is the spec).

use gpui_kit::test::{TestEventExt as _, TestWindowExt as _};
use gpui_kit::{AnyWindowHandle, Modifiers, Pixels, Point, TestAppContext, point, px};
use serde_json::json;

use super::*;
use crate::testing::{Harness, click_at, snapshot};

const MESSAGES: [&str; 5] = ["alpha one", "beta two", "gamma three", "beta four", "delta five"];

fn open(cx: &mut TestAppContext) -> (Harness, AnyWindowHandle, Entity<LogPanel>, Entity<LogView>) {
    let mut h = Harness::start(cx, snapshot());
    for (i, m) in MESSAGES.iter().enumerate() {
        let line = json!({"time_ms": 1000 + i as u64, "level": "info", "target": "test", "message": m, "seq": i});
        h.event(cx, json!({"kind": "log", "value": line}));
    }
    let view = cx.update(|cx| cx.new(|_| LogView::default()));
    let v = view.clone();
    let (w, panel) = h.open_sized(cx, (1000., 900.), move |window, cx| cx.new(|cx| LogPanel::new(v, window, cx)));
    (h, w, panel, view)
}

/// Line `ix` of the list (as filtered), far to the right of its text: the
/// list box starts under the buttons' row, each line 20 px high after the
/// box's border and padding
fn line_at(h: &mut Harness, cx: &mut TestAppContext, w: AnyWindowHandle, ix: usize) -> Point<Pixels> {
    h.on(cx, w, |window, _| {
        let row = window.find("log-clear").bounds();
        let top = row.bottom() + theme::S3 + px(1.) + theme::S2;
        point(px(900.), top + px(LINE_H) * ix as f32 + px(LINE_H / 2.))
    })
}

fn selected(view: &Entity<LogView>, cx: &TestAppContext) -> Vec<String> {
    let mut s: Vec<String> = cx.read(|cx| view.read(cx).selected.iter().cloned().collect());
    s.sort();
    s
}

fn keys(ix: &[usize]) -> Vec<String> {
    ix.iter().map(|i| format!("#{i}")).collect()
}

fn click_with(h: &mut Harness, cx: &mut TestAppContext, w: AnyWindowHandle, at: Point<Pixels>, m: Modifiers) {
    h.on(cx, w, |window, cx| {
        window.change_modifiers(m, cx);
        click_at(window, at, cx);
        window.change_modifiers(Modifiers::default(), cx);
    });
}

#[gpui_kit::test]
fn clicks_select_one_ctrl_adds_and_shift_takes_a_range(cx: &mut TestAppContext) {
    let (mut h, w, _panel, view) = open(cx);
    let l1 = line_at(&mut h, cx, w, 1);
    h.on(cx, w, |window, cx| click_at(window, l1, cx));
    assert_eq!(selected(&view, cx), keys(&[1]));
    // An older line selected: new lines are not followed any more
    assert!(!cx.read(|cx| view.read(cx).follow));
    let l3 = line_at(&mut h, cx, w, 3);
    click_with(&mut h, cx, w, l3, Modifiers::control());
    assert_eq!(selected(&view, cx), keys(&[1, 3]));
    // Ctrl+click on a selected one takes it out
    click_with(&mut h, cx, w, l1, Modifiers::control());
    assert_eq!(selected(&view, cx), keys(&[3]));
    // Shift+click: from the last one clicked (also one Ctrl took out)
    let l0 = line_at(&mut h, cx, w, 0);
    click_with(&mut h, cx, w, l0, Modifiers::shift());
    assert_eq!(selected(&view, cx), keys(&[0, 1]));
    let l2 = line_at(&mut h, cx, w, 2);
    h.on(cx, w, |window, cx| click_at(window, l2, cx));
    click_with(&mut h, cx, w, l0, Modifiers::shift());
    assert_eq!(selected(&view, cx), keys(&[0, 1, 2]));
    // A plain click: that one alone
    let l4 = line_at(&mut h, cx, w, 4);
    h.on(cx, w, |window, cx| click_at(window, l4, cx));
    assert_eq!(selected(&view, cx), keys(&[4]));
}

#[gpui_kit::test]
fn ctrl_a_ctrl_c_and_escape(cx: &mut TestAppContext) {
    let (mut h, w, _panel, view) = open(cx);
    // A click gives the list the keyboard
    let l2 = line_at(&mut h, cx, w, 2);
    h.on(cx, w, |window, cx| click_at(window, l2, cx));
    h.on(cx, w, |window, cx| window.press("ctrl-a", cx));
    assert_eq!(selected(&view, cx), keys(&[0, 1, 2, 3, 4]));
    h.on(cx, w, |window, cx| window.press("ctrl-c", cx));
    let copied = cx.update(|cx| cx.read_from_clipboard()).and_then(|c| c.text()).unwrap_or_default();
    let lines: Vec<&str> = copied.lines().collect();
    assert_eq!(lines.len(), 5);
    assert!(lines[0].ends_with("INFO  test: alpha one"), "{copied}");
    assert!(lines[4].ends_with("test: delta five"));
    // "Copied 5 lines"
    assert!(cx.read(|cx| app::state(cx).read(cx).notice.is_some()));
    h.on(cx, w, |window, cx| window.press("escape", cx));
    assert!(selected(&view, cx).is_empty());
}

#[gpui_kit::test]
fn the_search_filters_as_typed_and_ctrl_a_takes_what_it_shows(cx: &mut TestAppContext) {
    let (mut h, w, panel, view) = open(cx);
    let search = cx.read(|cx| panel.read(cx).search.clone());
    h.on(cx, w, |window, cx| {
        search.update(cx, |s, cx| s.focus(window, cx));
        window.input("beta", cx);
    });
    assert_eq!(cx.read(|cx| view.read(cx).query.clone()), "beta");
    assert_eq!(cx.read(|cx| panel.read(cx).shown.len()), 2);
    // Ctrl+A in the search box selects its text, not the lines
    h.on(cx, w, |window, cx| window.press("ctrl-a", cx));
    assert!(selected(&view, cx).is_empty());
    let l0 = line_at(&mut h, cx, w, 0);
    h.on(cx, w, |window, cx| {
        click_at(window, l0, cx);
        window.press("ctrl-a", cx);
    });
    assert_eq!(selected(&view, cx), keys(&[1, 3]));
    // Emptied, everything shows again
    h.on(cx, w, |window, cx| {
        search.update(cx, |s, cx| s.focus(window, cx));
        window.press("ctrl-a", cx);
        window.press("backspace", cx);
    });
    assert_eq!(cx.read(|cx| panel.read(cx).shown.len()), 5);
}
