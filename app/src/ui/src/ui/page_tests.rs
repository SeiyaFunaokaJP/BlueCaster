//! A page's controls scrolled out of view stay pressable by assistive
//! technology: its press arrives as a click at the control's centre, outside
//! the window, and the page scrolls the control into view and presses it.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Context, MouseButton, Render, TestAppContext};

use super::*;
use crate::testing::{Harness, move_to, press_at, release_at, snapshot};

struct Tall {
    pressed: Rc<Cell<u32>>,
}

impl Render for Tall {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let pressed = self.pressed.clone();
        page("tall", "Tall")
            .child(div().flex_none().h(px(1500.)))
            .child(button("far", "Far", ButtonKind::Default).on_click(move |_, _, _| pressed.set(pressed.get() + 1)))
            .child(div().flex_none().h(px(200.)))
    }
}

#[gpui_kit::test]
fn a_press_below_the_window_scrolls_the_control_into_view_and_presses_it(cx: &mut TestAppContext) {
    let mut h = Harness::start(cx, snapshot());
    let pressed = Rc::new(Cell::new(0));
    let p = pressed.clone();
    let (w, _) = h.open_sized(cx, (800., 600.), move |_, cx| cx.new(|_| Tall { pressed: p }));
    let far = h.on(cx, w, |window, _| window.find("far").bounds().center());
    assert!(far.y > px(600.), "the button starts below the window: {far:?}");
    h.on(cx, w, |window, cx| {
        press_at(window, far, MouseButton::Left, 1, cx);
        release_at(window, far, MouseButton::Left, 1, cx);
    });
    // Two frames as the platform delivers them (what is due on the next frame
    // runs before it is drawn): the page scrolled, then the press
    for _ in 0..2 {
        h.on(cx, w, |window, cx| {
            window.simulate_next_frame(cx);
            window.render_frame(cx);
        });
    }
    let now = h.on(cx, w, |window, _| window.find("far").bounds().center());
    assert!(now.y > px(0.) && now.y < px(600.), "scrolled into view: {now:?}");
    assert_eq!(pressed.get(), 1);
    // A click inside the window is the pointer's own: nothing moves
    let before = now;
    let empty = point(px(400.), px(300.));
    h.on(cx, w, |window, cx| {
        press_at(window, empty, MouseButton::Left, 1, cx);
        release_at(window, empty, MouseButton::Left, 1, cx);
    });
    assert_eq!(h.on(cx, w, |window, _| window.find("far").bounds().center()), before);
    assert_eq!(pressed.get(), 1);
}

#[gpui_kit::test]
fn a_press_on_the_title_bar_or_the_edge_is_left_to_the_window(cx: &mut TestAppContext) {
    let mut h = Harness::start(cx, snapshot());
    let pressed = Rc::new(Cell::new(0));
    let p = pressed.clone();
    let (w, _) = h.open_sized(cx, (800., 600.), move |_, cx| cx.new(|_| Tall { pressed: p }));
    let before = h.on(cx, w, |window, _| window.find("far").bounds().center());
    // Windows sends a press on the title bar's maximize button, or on the
    // bottom edge, as one outside the page, where the pointer moved to
    for at in [point(px(700.), px(-16.)), point(px(400.), px(604.))] {
        h.on(cx, w, |window, cx| {
            move_to(window, at, false, cx);
            press_at(window, at, MouseButton::Left, 1, cx);
            release_at(window, at, MouseButton::Left, 1, cx);
        });
        for _ in 0..2 {
            h.on(cx, w, |window, cx| {
                window.simulate_next_frame(cx);
                window.render_frame(cx);
            });
        }
        assert_eq!(h.on(cx, w, |window, _| window.find("far").bounds().center()), before, "not scrolled for {at:?}");
    }
    assert_eq!(pressed.get(), 0);
}
