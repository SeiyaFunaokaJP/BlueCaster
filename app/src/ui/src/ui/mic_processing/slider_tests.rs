//! The microphone's sliders dragged: nothing is saved while one moves, the
//! value is saved once when it is let go, and the slider shows what it holds
//! until the profile has it (the old UI's headsetMic.test.tsx is the spec).

use std::cell::RefCell;

use gpui_kit::test::{TestEventExt as _, TestWindowExt as _};
use gpui_kit::{AnyWindowHandle, MouseButton, TestAppContext, point};

use super::*;
use crate::testing::{Harness, snapshot};

type Saved = Rc<RefCell<Vec<MicProcessing>>>;

fn open(cx: &mut TestAppContext) -> (Harness, AnyWindowHandle, Entity<MicProcessingControls>, Saved) {
    let mut h = Harness::start(cx, snapshot());
    let saved: Saved = Rc::default();
    let current = Rc::new(RefCell::new(MicProcessing { enabled: true, ..MicProcessing::default() }));
    let source: MicProcessingSource = {
        let current = current.clone();
        Rc::new(move |_| Some((*current.borrow(), false)))
    };
    // Saved as the core would: the profile has it a moment later
    let on_change: MicProcessingSave = {
        let saved = saved.clone();
        Rc::new(move |p, cx| {
            saved.borrow_mut().push(p);
            *current.borrow_mut() = p;
            Task::ready(true).tap(cx)
        })
    };
    let (w, view) = h.open_sized(cx, (600., 400.), move |window, cx| {
        cx.new(|cx| MicProcessingControls::new(source, on_change, window, cx))
    });
    (h, w, view, saved)
}

trait Tap {
    fn tap(self, cx: &mut App) -> Self;
}

impl Tap for Task<bool> {
    fn tap(self, cx: &mut App) -> Self {
        cx.refresh_windows();
        self
    }
}

#[gpui_kit::test]
fn a_slider_saves_once_when_let_go(cx: &mut TestAppContext) {
    let (mut h, w, view, saved) = open(cx);
    // The gain's slider, the first of three
    let gain = cx.read(|cx| view.read(cx).sliders[0].state.entity_id());
    let scope = ElementId::NamedInteger("slider".into(), gain.as_u64());
    let track = h.on(cx, w, |window, _| window.within(scope.clone()).find("slider-bar-container").bounds());
    let from = point(track.left() + track.size.width * 0.5, track.center().y);
    let to = point(track.left() + track.size.width * 0.9, track.center().y);
    h.on(cx, w, |window, cx| {
        window.pointer_move(from, None, cx);
        window.pointer_down(from, MouseButton::Left, cx);
        window.pointer_move(to, Some(MouseButton::Left), cx);
        window.pointer_move(to, Some(MouseButton::Left), cx);
    });
    assert!(saved.borrow().is_empty(), "saved while it moves");
    let held = cx.read(|cx| view.read(cx).sliders[0].held);
    assert!(held.is_some());
    h.on(cx, w, |window, cx| window.pointer_up(to, MouseButton::Left, cx));
    assert_eq!(saved.borrow().len(), 1);
    assert_eq!(Some(i32::from(saved.borrow()[0].gain_db)), held);
    // The other sliders kept their values
    assert_eq!(saved.borrow()[0].suppression, 0);
}

#[gpui_kit::test]
fn a_click_on_the_value_it_has_saves_nothing(cx: &mut TestAppContext) {
    let (mut h, w, view, saved) = open(cx);
    let gain = cx.read(|cx| view.read(cx).sliders[0].state.entity_id());
    let scope = ElementId::NamedInteger("slider".into(), gain.as_u64());
    let thumb = h.on(cx, w, |window, _| window.within(scope.clone()).find(("slider-thumb", 0u32)).bounds().center());
    h.on(cx, w, |window, cx| {
        window.pointer_move(thumb, None, cx);
        window.pointer_down(thumb, MouseButton::Left, cx);
        window.pointer_up(thumb, MouseButton::Left, cx);
    });
    assert!(saved.borrow().is_empty());
}
