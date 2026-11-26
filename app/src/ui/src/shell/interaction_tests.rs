//! The main window by keyboard: Ctrl+Shift+D opens Diagnostics (not while the
//! setup shows), Tab walks the navigation and then the screen's controls in
//! order, Enter and Space follow a navigation link.

use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext};
use serde_json::json;

use super::*;
use crate::testing::{Answer, Harness, device, profile, snapshot};

const ADDR: &str = "0A:00:00:00:00:01";

fn open(cx: &mut TestAppContext, route: RouteName) -> (Harness, AnyWindowHandle) {
    let mut s = snapshot();
    s["settings"]["setup_done"] = json!(true);
    s["settings"]["cables_dialog_shown"] = json!(true);
    let mut h = Harness::start(cx, s);
    h.answer("audio.devices", Answer::Ok(json!([])));
    cx.update(|cx| {
        crate::bind_shortcuts(cx);
        app::navigate(route, cx);
    });
    let (w, _) = h.open_sized(cx, (1000., 1400.), |window, cx| cx.new(|cx| MainWindow::new(window, cx)));
    (h, w)
}

fn route(cx: &TestAppContext) -> RouteName {
    cx.read(|cx| app::state(cx).read(cx).route.name)
}

#[gpui_kit::test]
fn ctrl_shift_d_opens_diagnostics(cx: &mut TestAppContext) {
    let (mut h, w) = open(cx, RouteName::Settings);
    h.on(cx, w, |window, cx| window.press("ctrl-shift-d", cx));
    assert_eq!(route(cx), RouteName::Diagnostics);
}

#[gpui_kit::test]
fn ctrl_shift_d_does_nothing_while_the_setup_shows(cx: &mut TestAppContext) {
    let (mut h, w) = open(cx, RouteName::Settings);
    cx.update(|cx| app::state(cx).update(cx, |s, cx| s.set_setup_active(true, cx)));
    h.settle(cx);
    h.on(cx, w, |window, cx| window.press("ctrl-shift-d", cx));
    assert_eq!(route(cx), RouteName::Settings);
}

/// What has the keyboard: a control registered for tests by its id, else "?"
fn focused(h: &mut Harness, cx: &mut TestAppContext, w: AnyWindowHandle) -> String {
    h.on(cx, w, |window, _| {
        window
            .elements()
            .into_iter()
            .filter(|e| e.focused() == Some(true))
            .map(|e| format!("{:?}", e.path().last()))
            .next_back()
            .unwrap_or_else(|| "?".into())
    })
}

#[gpui_kit::test]
fn tab_walks_the_navigation_then_every_setting_in_order(cx: &mut TestAppContext) {
    let (mut h, w) = open(cx, RouteName::Settings);
    let mut seen = Vec::new();
    for _ in 0..60 {
        h.on(cx, w, |window, cx| window.press("tab", cx));
        seen.push(focused(&mut h, cx, w));
    }
    let at = |id: &str| seen.iter().position(|s| s.contains(&format!("\"{id}\"")));
    // Every control of the screen is reached, in the screen's order; "Start
    // minimized" is off while "Start with Windows" is, and skipped
    let order = [
        "language",
        "start-with-windows",
        "minimize-to-tray",
        "flyout-from",
        "notify-connected",
        "battery-low",
        "check-update",
        "check-updates",
        "include-prereleases",
        "developer-mode",
        "run-setup",
        "quit",
    ];
    assert_eq!(at("start-minimized"), None);
    let places: Vec<Option<usize>> = order.iter().map(|id| at(id)).collect();
    for (id, p) in order.iter().zip(&places) {
        assert!(p.is_some(), "Tab never reached {id}: {seen:?}");
    }
    let places: Vec<usize> = places.into_iter().flatten().collect();
    assert!(places.windows(2).all(|p| p[0] < p[1]), "out of order: {order:?} at {places:?} in {seen:?}");
    // The navigation's links come before the screen (a wide window has no
    // navigation button): they take the keyboard too (not registered for
    // tests: "?")
    let links = seen[..places[0]].iter().filter(|s| *s == "?").count();
    assert!(links >= 7, "{links} navigation links reached: {seen:?}");
}

/// The controls the keyboard should reach: every one registered for tests
/// that is shown, can be used and takes keys, by its path
fn controls(h: &mut Harness, cx: &mut TestAppContext, w: AnyWindowHandle) -> Vec<String> {
    use gpui_kit::Role;
    h.on(cx, w, |window, _| {
        window
            .elements()
            .into_iter()
            .filter(|e| e.visible() && e.disabled() != Some(true))
            .filter(|e| {
                matches!(
                    e.role(),
                    Some(
                        Role::Button | Role::Switch | Role::CheckBox | Role::TextInput | Role::SpinButton | Role::Link
                    )
                )
            })
            .map(|e| format!("{:?}", e.path()))
            .collect()
    })
}

/// Tab pressed until the keyboard is back where it started (at most
/// `max` times): what had it each time, by path ("?" for one not registered)
fn tab_round(h: &mut Harness, cx: &mut TestAppContext, w: AnyWindowHandle, max: usize) -> Vec<String> {
    let path = |h: &mut Harness, cx: &mut TestAppContext| {
        h.on(cx, w, |window, _| {
            window
                .elements()
                .into_iter()
                .filter(|e| e.focused() == Some(true))
                .map(|e| format!("{:?}", e.path()))
                .next_back()
                .unwrap_or_else(|| "?".into())
        })
    };
    let mut seen: Vec<String> = Vec::new();
    for _ in 0..max {
        h.on(cx, w, |window, cx| window.press("tab", cx));
        let now = path(h, cx);
        // Round: the first control registered for tests has the keyboard again
        if now != "?" && seen.iter().find(|s| *s != "?") == Some(&now) {
            return seen;
        }
        seen.push(now);
    }
    panic!("Tab never came back to the first control in {max} presses: {seen:?}");
}

/// Every control of the screen takes the keyboard in one round of Tab, and
/// the round ends where it began (nothing keeps the keyboard); with a saved
/// device and its profile, so the screens show their device controls
fn check_tab_round(route: RouteName, cx: &mut TestAppContext) {
    let (mut h, w) = open(cx, route);
    h.edit(cx, |s| {
        s["devices"] = json!([device(ADDR, "Test Phones")]);
        s["profiles"] = json!([profile("Test Phones | Auto", ADDR, "Test Phones", json!({}))]);
        s["active_profile"] = json!("Test Phones | Auto");
    });
    let seen = tab_round(&mut h, cx, w, 200);
    let missed: Vec<String> = controls(&mut h, cx, w).into_iter().filter(|c| !seen.contains(c)).collect();
    assert!(missed.is_empty(), "{route:?}: Tab never reached {missed:?}; it went {seen:?}");
    // No control twice in one round: the keyboard is not caught in a loop
    let mut sorted: Vec<&String> = seen.iter().filter(|s| *s != "?").collect();
    sorted.sort();
    let before = sorted.len();
    sorted.dedup();
    assert_eq!(sorted.len(), before, "{route:?}: a control came twice in one round: {seen:?}");
}

#[gpui_kit::test]
fn tab_reaches_every_control_of_home(cx: &mut TestAppContext) {
    check_tab_round(RouteName::Home, cx);
}

#[gpui_kit::test]
fn tab_reaches_every_control_of_codecs(cx: &mut TestAppContext) {
    check_tab_round(RouteName::Codecs, cx);
}

#[gpui_kit::test]
fn tab_reaches_every_control_of_audio_devices(cx: &mut TestAppContext) {
    check_tab_round(RouteName::Devices, cx);
}

#[gpui_kit::test]
fn tab_reaches_every_control_of_adapter(cx: &mut TestAppContext) {
    check_tab_round(RouteName::Adapter, cx);
}

#[gpui_kit::test]
fn tab_reaches_every_control_of_diagnostics(cx: &mut TestAppContext) {
    check_tab_round(RouteName::Diagnostics, cx);
}

#[gpui_kit::test]
fn enter_and_space_follow_a_navigation_link(cx: &mut TestAppContext) {
    let (mut h, w) = open(cx, RouteName::Settings);
    // Home's link, then Codecs'
    h.on(cx, w, |window, cx| {
        window.press("tab", cx);
        window.press("tab", cx);
    });
    h.on(cx, w, |window, cx| window.press("enter", cx));
    assert_eq!(route(cx), RouteName::Codecs, "the second link is Codecs");
    h.on(cx, w, |window, cx| {
        window.press("shift-tab", cx);
        window.press("space", cx);
    });
    assert_eq!(route(cx), RouteName::Home);
}
