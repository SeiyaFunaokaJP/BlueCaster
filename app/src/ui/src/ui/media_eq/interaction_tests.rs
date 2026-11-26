//! The playback EQ driven as a user does: double-clicks, drags, the wheel,
//! keys on a focused point and typing into the fields (the old UI's
//! mediaEq.test.tsx is the spec).

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, Entity, MouseButton, Pixels, Point, TestAppContext};
use serde_json::{Value, json};
use ui_model::view::media_eq::f_of;

use super::*;
use crate::testing::{
    Harness, click_at, device, double_click_at, move_to, press_at, profile, release_at, snapshot, wheel_at,
};

const ADDR: &str = "0A:00:00:00:00:01";
const PROFILE: &str = "Test Phones | Auto";

fn state(eq: Value, streaming: bool) -> Value {
    let mut s = snapshot();
    s["devices"] = json!([device(ADDR, "Test Phones")]);
    s["profiles"] = json!([profile(PROFILE, ADDR, "Test Phones", json!({"media_eq": eq}))]);
    if streaming {
        s["stream"] = json!({"state": "streaming", "message": {"key": "status.ready", "args": []}, "profile": PROFILE});
    }
    s
}

fn open(cx: &mut TestAppContext, eq: Value, streaming: bool) -> (Harness, AnyWindowHandle, Entity<MediaEqEditor>) {
    let mut h = Harness::start(cx, state(eq, streaming));
    let (window, editor) = h.open(cx, |window, cx| cx.new(|cx| MediaEqEditor::new(ADDR.to_owned(), window, cx)));
    (h, window, editor)
}

/// A place of the graph (its 640×240 units) in window pixels
fn graph_at(editor: &Entity<MediaEqEditor>, cx: &TestAppContext, x: f64, y: f64) -> Point<Pixels> {
    cx.read(|cx| editor.read(cx).graph.read(cx).to_window(x, y))
}

/// Where point `i` is drawn
fn point_at(editor: &Entity<MediaEqEditor>, cx: &TestAppContext, i: usize) -> Point<Pixels> {
    let b = cx.read(|cx| editor.read(cx).graph.read(cx).bands[i]);
    graph_at(editor, cx, x_of(f64::from(b.frequency)), point_y(&b))
}

fn saved_bands(h: &Harness) -> Vec<EqPoint> {
    serde_json::from_value(h.last_saved()["media_eq"]["bands"].clone()).unwrap()
}

fn saves(h: &Harness) -> usize {
    h.calls("profile.save").len()
}

fn field_value(editor: &Entity<MediaEqEditor>, cx: &TestAppContext, kind: FieldKind) -> String {
    cx.read(|cx| {
        let e = editor.read(cx);
        let f = e.fields.iter().find(|f| f.kind == kind).unwrap();
        f.input.read(cx).value().to_string()
    })
}

fn selected(editor: &Entity<MediaEqEditor>, cx: &TestAppContext) -> Option<usize> {
    cx.read(|cx| editor.read(cx).selected)
}

/// Focuses a number field, as a click into it does
fn focus_field(h: &mut Harness, cx: &mut TestAppContext, w: AnyWindowHandle, e: &Entity<MediaEqEditor>, k: FieldKind) {
    let input = cx.read(|cx| e.read(cx).fields.iter().find(|f| f.kind == k).unwrap().input.clone());
    h.on(cx, w, |window, cx| {
        input.update(cx, |s, cx| s.focus(window, cx));
    });
}

/// Selects everything in the focused field and types `text` over it
fn type_over(h: &mut Harness, cx: &mut TestAppContext, w: AnyWindowHandle, text: &str) {
    h.on(cx, w, |window, cx| {
        window.press("ctrl-a", cx);
        window.input(text, cx);
    });
}

#[gpui_kit::test]
fn double_click_on_the_graph_adds_a_peak_there(cx: &mut TestAppContext) {
    let (mut h, w, e) = open(cx, json!({"enabled": true, "bands": []}), false);
    // 1 kHz is 2/3 of the way along the log axis; +6 dB a quarter from the top
    let at = graph_at(&e, cx, 38.0 + (592.0 * 50f64.log10()) / 3.0, 10.0 + 208.0 * 0.25);
    h.on(cx, w, |window, cx| double_click_at(window, at, cx));
    let bands = saved_bands(&h);
    assert_eq!(bands.len(), 1);
    assert_eq!(bands[0].band_type, EqBandType::Peaking);
    assert_eq!(bands[0].frequency, 1000.0);
    assert!((bands[0].gain - 6.0).abs() <= 0.1, "{}", bands[0].gain);
    assert_eq!(bands[0].q, 1.0);
    // The new point is selected, its values in the fields
    assert_eq!(selected(&e, cx), Some(0));
    assert_eq!(field_value(&e, cx, FieldKind::Frequency), "1000");
    // A double-click on the point puts its gain back to 0
    let p = point_at(&e, cx, 0);
    h.on(cx, w, |window, cx| double_click_at(window, p, cx));
    assert_eq!(saved_bands(&h)[0].gain, 0.0);
    // A single click on the background lets the selection go
    let empty = graph_at(&e, cx, 100.0, 200.0);
    h.on(cx, w, |window, cx| click_at(window, empty, cx));
    assert_eq!(selected(&e, cx), None);
    assert_eq!(field_value(&e, cx, FieldKind::Frequency), "");
}

#[gpui_kit::test]
fn a_double_click_does_nothing_while_the_eq_is_off(cx: &mut TestAppContext) {
    let (mut h, w, e) = open(cx, json!({"enabled": false, "bands": []}), false);
    let at = graph_at(&e, cx, 300.0, 100.0);
    h.on(cx, w, |window, cx| double_click_at(window, at, cx));
    assert_eq!(saves(&h), 0);
    // The ten points turning it on gives are shown, greyed out
    assert_eq!(cx.read(|cx| e.read(cx).graph.read(cx).bands.len()), 10);
    // A greyed point takes no keys
    let p = point_at(&e, cx, 5);
    h.on(cx, w, |window, cx| {
        click_at(window, p, cx);
        window.press("up", cx);
    });
    assert_eq!(saves(&h), 0);
}

#[gpui_kit::test]
fn the_keys_move_and_delete_a_focused_point(cx: &mut TestAppContext) {
    let (mut h, w, e) = open(cx, json!({"enabled": true, "bands": []}), false);
    h.on(cx, w, |window, cx| window.click("eq-add", cx));
    assert_eq!(saved_bands(&h), vec![EqPoint { frequency: 1000.0, gain: 0.0, q: 1.0, band_type: EqBandType::Peaking }]);
    // A click focuses the point; letting go without moving saves nothing
    let n = saves(&h);
    let p = point_at(&e, cx, 0);
    h.on(cx, w, |window, cx| click_at(window, p, cx));
    assert_eq!(saves(&h), n);
    h.on(cx, w, |window, cx| window.press("up", cx));
    assert_eq!(saved_bands(&h)[0].gain, 0.5);
    h.on(cx, w, |window, cx| window.press("shift-right", cx));
    assert_eq!(saved_bands(&h)[0].frequency, 2000.0);
    h.on(cx, w, |window, cx| window.press("left", cx));
    assert!(saved_bands(&h)[0].frequency < 2000.0);
    h.on(cx, w, |window, cx| window.press("shift-down", cx));
    assert_eq!(saved_bands(&h)[0].gain, -2.5);
    h.on(cx, w, |window, cx| window.press("pageup", cx));
    assert_eq!(saved_bands(&h)[0].q, 1.25);
    h.on(cx, w, |window, cx| window.press("pagedown", cx));
    assert_eq!(saved_bands(&h)[0].q, 1.0);
    // The fields follow the point
    assert_eq!(field_value(&e, cx, FieldKind::Gain), "-2.5");
    h.on(cx, w, |window, cx| window.press("delete", cx));
    assert!(saved_bands(&h).is_empty());
    assert_eq!(selected(&e, cx), None);
}

#[gpui_kit::test]
fn the_gain_held_to_its_limit_by_the_keys(cx: &mut TestAppContext) {
    let (mut h, w, e) = open(
        cx,
        json!({"enabled": true, "bands": [{"frequency": 1000.0, "gain": 11.0, "q": 1.0, "band_type": "Peaking"}]}),
        false,
    );
    let p = point_at(&e, cx, 0);
    h.on(cx, w, |window, cx| {
        click_at(window, p, cx);
        window.press("shift-up", cx);
    });
    assert_eq!(saved_bands(&h)[0].gain, 12.0);
}

#[gpui_kit::test]
fn the_wheel_over_a_point_changes_its_q(cx: &mut TestAppContext) {
    let (mut h, w, e) = open(
        cx,
        json!({"enabled": true, "bands": [{"frequency": 1000.0, "gain": 3.0, "q": 1.0, "band_type": "Peaking"}]}),
        false,
    );
    let p = point_at(&e, cx, 0);
    // Up (away from the user): a narrower peak, a higher Q
    h.on(cx, w, |window, cx| wheel_at(window, p, 1.0, cx));
    let up = saved_bands(&h)[0].q;
    assert!(up > 1.0, "{up}");
    assert_eq!(up, ui_model::eq::round_q(1.0015f64.powf(100.0)) as f32);
    h.on(cx, w, |window, cx| wheel_at(window, p, -2.0, cx));
    assert!(saved_bands(&h)[0].q < up);
    // A shelf has no Q: the wheel leaves it
    let n = saves(&h);
    h.edit(cx, |s| s["profiles"][0]["media_eq"]["bands"][0]["band_type"] = json!("HighShelf"));
    h.on(cx, w, |window, cx| wheel_at(window, p, 1.0, cx));
    assert_eq!(saves(&h), n);
}

#[gpui_kit::test]
fn a_drag_previews_while_moving_and_saves_once_when_let_go(cx: &mut TestAppContext) {
    let (mut h, w, e) = open(
        cx,
        json!({"enabled": true, "bands": [{"frequency": 1000.0, "gain": 0.0, "q": 1.0, "band_type": "Peaking"}]}),
        true,
    );
    let n = saves(&h);
    let start = point_at(&e, cx, 0);
    h.on(cx, w, |window, cx| {
        move_to(window, start, false, cx);
        press_at(window, start, MouseButton::Left, 1, cx);
    });
    // The first move is played at once
    let at = graph_at(&e, cx, 300.0, 60.0);
    h.on(cx, w, |window, cx| move_to(window, at, true, cx));
    assert_eq!(h.calls("media_eq.preview").len(), 1);
    // Moves within one preview interval: one more preview, nothing saved
    for x in [250.0, 200.0, 150.0] {
        let at = graph_at(&e, cx, x, 60.0);
        h.on(cx, w, |window, cx| move_to(window, at, true, cx));
    }
    assert_eq!(saves(&h), n);
    assert_eq!(h.calls("media_eq.preview").len(), 1);
    cx.executor().advance_clock(Duration::from_millis(PREVIEW_MS + 5));
    h.settle(cx);
    let previews = h.calls("media_eq.preview");
    assert_eq!(previews.len(), 2);
    assert_eq!(previews[1]["name"], PROFILE);
    // The preview carries the last move
    let last: Vec<EqPoint> = serde_json::from_value(previews[1]["eq"]["bands"].clone()).unwrap();
    assert_eq!(f64::from(last[0].frequency), ui_model::eq::round_freq(f_of(150.0)));
    // Beyond the plot (the far left, the top): held to 20 Hz, +12 dB
    let corner = graph_at(&e, cx, 10.0, 0.0);
    h.on(cx, w, |window, cx| move_to(window, corner, true, cx));
    let shown = cx.read(|cx| e.read(cx).graph.read(cx).bands[0]);
    assert_eq!((shown.frequency, shown.gain), (20.0, 12.0));
    assert_eq!(saves(&h), n);
    h.on(cx, w, |window, cx| release_at(window, corner, MouseButton::Left, 1, cx));
    assert_eq!(saves(&h), n + 1);
    let b = saved_bands(&h)[0];
    assert_eq!((b.frequency, b.gain), (20.0, 12.0));
    // A preview still pending went with the save
    let sent = h.calls("media_eq.preview").len();
    cx.executor().advance_clock(Duration::from_millis(200));
    h.settle(cx);
    assert_eq!(saves(&h), n + 1);
    assert_eq!(h.calls("media_eq.preview").len(), sent);
}

#[gpui_kit::test]
fn previews_are_sent_at_most_every_40_ms(cx: &mut TestAppContext) {
    let (mut h, w, e) = open(
        cx,
        json!({"enabled": true, "bands": [{"frequency": 1000.0, "gain": 0.0, "q": 1.0, "band_type": "Peaking"}]}),
        true,
    );
    let start = point_at(&e, cx, 0);
    h.on(cx, w, |window, cx| {
        move_to(window, start, false, cx);
        press_at(window, start, MouseButton::Left, 1, cx);
    });
    for step in 0..10 {
        let at = graph_at(&e, cx, 100.0 + 40.0 * f64::from(step), 100.0);
        h.on(cx, w, |window, cx| move_to(window, at, true, cx));
        cx.executor().advance_clock(Duration::from_millis(10));
        h.settle(cx);
    }
    // 100 ms of moves every 10 ms: not one preview per move
    let sent = h.calls("media_eq.preview").len();
    assert!((1..=4).contains(&sent), "{sent} previews");
    let end = graph_at(&e, cx, 500.0, 100.0);
    h.on(cx, w, |window, cx| release_at(window, end, MouseButton::Left, 1, cx));
}

#[gpui_kit::test]
fn no_preview_while_the_device_does_not_stream(cx: &mut TestAppContext) {
    let (mut h, w, e) = open(
        cx,
        json!({"enabled": true, "bands": [{"frequency": 1000.0, "gain": 0.0, "q": 1.0, "band_type": "Peaking"}]}),
        false,
    );
    let start = point_at(&e, cx, 0);
    let to = graph_at(&e, cx, 200.0, 50.0);
    h.on(cx, w, |window, cx| {
        move_to(window, start, false, cx);
        press_at(window, start, MouseButton::Left, 1, cx);
        move_to(window, to, true, cx);
    });
    cx.executor().advance_clock(Duration::from_millis(100));
    h.settle(cx);
    assert!(h.calls("media_eq.preview").is_empty());
    h.on(cx, w, |window, cx| release_at(window, to, MouseButton::Left, 1, cx));
    assert_eq!(saves(&h), 1);
}

#[gpui_kit::test]
fn typed_values_are_taken_on_enter_held_to_their_range(cx: &mut TestAppContext) {
    let (mut h, w, e) = open(cx, json!({"enabled": true, "bands": []}), false);
    h.on(cx, w, |window, cx| window.click("eq-add", cx));
    focus_field(&mut h, cx, w, &e, FieldKind::Gain);
    let n = saves(&h);
    type_over(&mut h, cx, w, "40");
    // Nothing is taken while typing
    assert_eq!(saves(&h), n);
    h.on(cx, w, |window, cx| window.press("enter", cx));
    assert_eq!(saved_bands(&h)[0].gain, 12.0);
    assert_eq!(field_value(&e, cx, FieldKind::Gain), "12");
    focus_field(&mut h, cx, w, &e, FieldKind::Q);
    type_over(&mut h, cx, w, "2.5");
    h.on(cx, w, |window, cx| window.press("enter", cx));
    assert_eq!(saved_bands(&h)[0].q, 2.5);
    focus_field(&mut h, cx, w, &e, FieldKind::Frequency);
    type_over(&mut h, cx, w, "5");
    h.on(cx, w, |window, cx| window.press("enter", cx));
    assert_eq!(saved_bands(&h)[0].frequency, 20.0);
    assert_eq!(field_value(&e, cx, FieldKind::Frequency), "20");
}

#[gpui_kit::test]
fn a_typed_value_is_taken_when_the_field_is_left(cx: &mut TestAppContext) {
    let (mut h, w, e) = open(cx, json!({"enabled": true, "bands": []}), false);
    h.on(cx, w, |window, cx| window.click("eq-add", cx));
    focus_field(&mut h, cx, w, &e, FieldKind::Frequency);
    type_over(&mut h, cx, w, "4000");
    // Tab leaves the field
    let input = cx.read(|cx| e.read(cx).fields[0].input.clone());
    h.on(cx, w, |window, cx| window.press("tab", cx));
    let still = h.on(cx, w, |window, cx| input.read(cx).focus_handle(cx).is_focused(window));
    assert!(!still, "Tab left the focus in the field");
    assert_eq!(saved_bands(&h)[0].frequency, 4000.0);
    // A click on the graph's background lets the point go first: what was
    // typed goes with it (as in the old UI, where the selection was gone
    // when the field's change came)
    focus_field(&mut h, cx, w, &e, FieldKind::Gain);
    type_over(&mut h, cx, w, "-3");
    let n = saves(&h);
    let empty = graph_at(&e, cx, 100.0, 200.0);
    h.on(cx, w, |window, cx| click_at(window, empty, cx));
    assert_eq!(saves(&h), n);
    assert_eq!(field_value(&e, cx, FieldKind::Gain), "");
}

#[gpui_kit::test]
fn an_invalid_value_goes_back(cx: &mut TestAppContext) {
    let (mut h, w, e) = open(cx, json!({"enabled": true, "bands": []}), false);
    h.on(cx, w, |window, cx| window.click("eq-add", cx));
    let n = saves(&h);
    focus_field(&mut h, cx, w, &e, FieldKind::Gain);
    type_over(&mut h, cx, w, "");
    h.on(cx, w, |window, cx| window.press("enter", cx));
    assert_eq!(saves(&h), n);
    assert_eq!(field_value(&e, cx, FieldKind::Gain), "0");
    focus_field(&mut h, cx, w, &e, FieldKind::Q);
    type_over(&mut h, cx, w, "abc");
    h.on(cx, w, |window, cx| window.press("enter", cx));
    assert_eq!(saves(&h), n);
    assert_eq!(field_value(&e, cx, FieldKind::Q), "1");
}

#[gpui_kit::test]
fn the_fields_step_with_the_arrow_keys(cx: &mut TestAppContext) {
    let (mut h, w, e) = open(cx, json!({"enabled": true, "bands": []}), false);
    h.on(cx, w, |window, cx| window.click("eq-add", cx));
    focus_field(&mut h, cx, w, &e, FieldKind::Gain);
    h.on(cx, w, |window, cx| window.press("up", cx));
    assert_eq!(saved_bands(&h)[0].gain, 0.1);
    assert_eq!(field_value(&e, cx, FieldKind::Gain), "0.1");
    focus_field(&mut h, cx, w, &e, FieldKind::Q);
    h.on(cx, w, |window, cx| window.press("down", cx));
    assert_eq!(saved_bands(&h)[0].q, 0.99);
    focus_field(&mut h, cx, w, &e, FieldKind::Frequency);
    h.on(cx, w, |window, cx| window.press("up", cx));
    assert_eq!(saved_bands(&h)[0].frequency, 1001.0);
}

#[gpui_kit::test]
fn a_failed_save_puts_the_point_back(cx: &mut TestAppContext) {
    let (mut h, w, e) = open(
        cx,
        json!({"enabled": true, "bands": [{"frequency": 1000.0, "gain": 0.0, "q": 1.0, "band_type": "Peaking"}]}),
        false,
    );
    h.answer("profile.save", crate::testing::Answer::Err("profile.save_failed".into()));
    let p = point_at(&e, cx, 0);
    h.on(cx, w, |window, cx| {
        click_at(window, p, cx);
        window.press("up", cx);
    });
    assert_eq!(h.methods().iter().filter(|m| *m == "profile.save").count(), 1);
    // The core's points again, and the error said
    assert_eq!(cx.read(|cx| e.read(cx).eq(cx).bands[0].gain), 0.0);
    assert!(cx.read(|cx| app::state(cx).read(cx).notice.is_some()));
}

/// Whether the right-click menu is open
fn menu_open(e: &Entity<MediaEqEditor>, cx: &TestAppContext) -> bool {
    cx.read(|cx| e.read(cx).graph.read(cx).menu.is_some())
}

/// The open menu's entries of one kind ("choice": a point's types,
/// "command": the rest), top to bottom: where each is and whether it is off
fn entries(
    h: &mut Harness,
    cx: &mut TestAppContext,
    w: AnyWindowHandle,
    kind: &'static str,
) -> Vec<(Point<Pixels>, bool)> {
    h.on(cx, w, move |window, _| {
        window.find_all(kind).into_iter().map(|e| (e.bounds().center(), e.disabled() == Some(true))).collect()
    })
}

#[gpui_kit::test]
fn a_right_click_on_a_point_offers_its_types_and_delete(cx: &mut TestAppContext) {
    let (mut h, w, e) = open(
        cx,
        json!({"enabled": true, "bands": [{"frequency": 1000.0, "gain": 3.0, "q": 1.0, "band_type": "Peaking"}]}),
        false,
    );
    let p = point_at(&e, cx, 0);
    h.on(cx, w, |window, cx| {
        move_to(window, p, false, cx);
        press_at(window, p, MouseButton::Right, 1, cx);
        release_at(window, p, MouseButton::Right, 1, cx);
    });
    assert!(menu_open(&e, cx));
    assert_eq!(selected(&e, cx), Some(0), "the point is selected");
    // Its five types, then Delete point
    let types = entries(&mut h, cx, w, "choice");
    assert_eq!(types.len(), BAND_TYPES.len());
    assert_eq!(entries(&mut h, cx, w, "command").len(), 1);
    // A click on High shelf makes it one and closes the menu
    let high_shelf = BAND_TYPES.iter().position(|t| *t == EqBandType::HighShelf).unwrap();
    let at = types[high_shelf].0;
    h.on(cx, w, |window, cx| click_at(window, at, cx));
    assert_eq!(saved_bands(&h)[0].band_type, EqBandType::HighShelf);
    assert!(!menu_open(&e, cx));
    // Choosing the type it has saves nothing more
    let n = saves(&h);
    h.on(cx, w, |window, cx| {
        press_at(window, p, MouseButton::Right, 1, cx);
        release_at(window, p, MouseButton::Right, 1, cx);
    });
    let at = entries(&mut h, cx, w, "choice")[high_shelf].0;
    h.on(cx, w, |window, cx| click_at(window, at, cx));
    assert_eq!(saves(&h), n);
    // Delete point
    h.on(cx, w, |window, cx| {
        press_at(window, p, MouseButton::Right, 1, cx);
        release_at(window, p, MouseButton::Right, 1, cx);
    });
    let at = entries(&mut h, cx, w, "command")[0].0;
    h.on(cx, w, |window, cx| click_at(window, at, cx));
    assert!(saved_bands(&h).is_empty());
    assert!(!menu_open(&e, cx));
}

#[gpui_kit::test]
fn a_right_click_on_the_background_adds_a_point_or_resets(cx: &mut TestAppContext) {
    let (mut h, w, e) = open(cx, json!({"enabled": true, "bands": []}), false);
    let (at, elsewhere) = (graph_at(&e, cx, 300.0, 100.0), graph_at(&e, cx, 120.0, 200.0));
    let right_click = |h: &mut Harness, cx: &mut TestAppContext, at: Point<Pixels>| {
        h.on(cx, w, |window, cx| {
            move_to(window, at, false, cx);
            press_at(window, at, MouseButton::Right, 1, cx);
            release_at(window, at, MouseButton::Right, 1, cx);
        })
    };
    right_click(&mut h, cx, at);
    // Add point here, then Reset
    let commands = entries(&mut h, cx, w, "command");
    assert_eq!(commands.len(), 2);
    assert_eq!(entries(&mut h, cx, w, "choice").len(), 0);
    let add = commands[0].0;
    h.on(cx, w, |window, cx| click_at(window, add, cx));
    assert_eq!(saved_bands(&h).len(), 1);
    // Reset puts the ten default points back
    right_click(&mut h, cx, elsewhere);
    let reset = entries(&mut h, cx, w, "command")[1];
    h.on(cx, w, |window, cx| click_at(window, reset.0, cx));
    assert_eq!(saved_bands(&h), DEFAULT_POINTS.to_vec());
    // Escape closes the menu without doing anything
    right_click(&mut h, cx, elsewhere);
    let n = saves(&h);
    h.on(cx, w, |window, cx| window.press("escape", cx));
    assert!(!menu_open(&e, cx));
    assert_eq!(saves(&h), n);
}

#[gpui_kit::test]
fn the_menu_key_opens_a_focused_points_menu_and_the_keys_choose(cx: &mut TestAppContext) {
    let (mut h, w, e) = open(
        cx,
        json!({"enabled": true, "bands": [{"frequency": 1000.0, "gain": 3.0, "q": 1.0, "band_type": "Peaking"}]}),
        false,
    );
    let p = point_at(&e, cx, 0);
    h.on(cx, w, |window, cx| click_at(window, p, cx));
    h.on(cx, w, |window, cx| window.press("menu", cx));
    assert!(menu_open(&e, cx));
    // Down to the second type (Low shelf), Enter chooses it
    h.on(cx, w, |window, cx| {
        window.press("down", cx);
        window.press("down", cx);
    });
    h.on(cx, w, |window, cx| window.press("enter", cx));
    assert_eq!(saved_bands(&h)[0].band_type, BAND_TYPES[1]);
    assert!(!menu_open(&e, cx));
}

#[gpui_kit::test]
fn the_switch_turns_it_on_with_the_ten_default_points(cx: &mut TestAppContext) {
    let (mut h, w, _e) = open(cx, json!({"enabled": false, "bands": []}), false);
    h.on(cx, w, |window, cx| window.click("eq-enable", cx));
    let eq = h.last_saved()["media_eq"].clone();
    assert_eq!(eq["enabled"], true);
    let bands: Vec<EqPoint> = serde_json::from_value(eq["bands"].clone()).unwrap();
    assert_eq!(bands, DEFAULT_POINTS.to_vec());
    // Off keeps the points
    h.on(cx, w, |window, cx| window.click("eq-enable", cx));
    assert_eq!(h.last_saved()["media_eq"]["enabled"], false);
    assert_eq!(saved_bands(&h).len(), 10);
}
