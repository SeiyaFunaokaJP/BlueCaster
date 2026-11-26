//! A drop-down list ([`select`]) by keyboard and pointer: Enter or Space
//! opens it on the focused button, the arrows move through its entries,
//! Enter chooses one and Escape closes it without choosing. Every entry is
//! an option assistive technology can choose (the menu's own entries offer
//! nothing to press), the chosen one checked.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Context, Render, TestAppContext};

use super::*;
use crate::testing::{Harness, click_at, snapshot};

struct Pick {
    value: u32,
    chosen: Rc<RefCell<Vec<u32>>>,
}

impl Render for Pick {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let chosen = self.chosen.clone();
        let this = cx.weak_entity();
        let choices = vec![
            Choice::new(1u32, "One"),
            Choice::new(2u32, "Two"),
            Choice::new(3u32, "Three").disabled(true),
            Choice::new(4u32, "Four"),
        ];
        div().size_full().p(px(20.)).child(select("pick", &self.value, choices, false, move |v: &u32, _, cx| {
            chosen.borrow_mut().push(*v);
            let v = *v;
            let _ = this.update(cx, |p, cx| {
                p.value = v;
                cx.notify();
            });
        }))
    }
}

fn open(cx: &mut TestAppContext) -> (Harness, AnyWindowHandle, Rc<RefCell<Vec<u32>>>) {
    let mut h = Harness::start(cx, snapshot());
    let chosen = Rc::new(RefCell::new(Vec::new()));
    let c = chosen.clone();
    let (w, _) = h.open(cx, move |_, cx| cx.new(|_| Pick { value: 2, chosen: c }));
    (h, w, chosen)
}

/// The entries shown: their label and whether each is the checked one
fn entries(h: &mut Harness, cx: &mut TestAppContext, w: AnyWindowHandle) -> Vec<(String, Option<bool>)> {
    h.on(cx, w, |window, _| {
        window.find_all("choice").into_iter().map(|e| (e.label().unwrap_or_default().to_owned(), e.checked())).collect()
    })
}

fn focus_list(h: &mut Harness, cx: &mut TestAppContext, w: AnyWindowHandle) {
    h.on(cx, w, |window, cx| window.press("tab", cx));
    assert_eq!(h.on(cx, w, |window, _| window.find("pick").focused()), Some(true), "Tab reaches the list");
}

#[gpui_kit::test]
fn every_entry_is_an_option_the_chosen_one_checked(cx: &mut TestAppContext) {
    let (mut h, w, _) = open(cx);
    h.on(cx, w, |window, cx| window.click("pick", cx));
    let shown = entries(&mut h, cx, w);
    assert_eq!(
        shown,
        [
            ("One".to_owned(), Some(false)),
            ("Two".to_owned(), Some(true)),
            ("Three".to_owned(), Some(false)),
            ("Four".to_owned(), Some(false))
        ]
    );
    let roles: Vec<_> = h.on(cx, w, |window, _| window.find_all("choice").into_iter().map(|e| e.role()).collect());
    assert!(roles.iter().all(|r| *r == Some(Role::MenuItemRadio)), "{roles:?}");
}

#[gpui_kit::test]
fn a_click_on_an_entry_chooses_it_and_the_chosen_one_changes_nothing(cx: &mut TestAppContext) {
    let (mut h, w, chosen) = open(cx);
    h.on(cx, w, |window, cx| window.click("pick", cx));
    let four = h.on(cx, w, |window, _| window.find_all("choice")[3].bounds().center());
    h.on(cx, w, |window, cx| click_at(window, four, cx));
    assert_eq!(*chosen.borrow(), [4]);
    assert!(entries(&mut h, cx, w).is_empty(), "the list closed");
    // The value it has, chosen again: nothing
    h.on(cx, w, |window, cx| window.click("pick", cx));
    let four = h.on(cx, w, |window, _| window.find_all("choice")[3].bounds().center());
    h.on(cx, w, |window, cx| click_at(window, four, cx));
    assert_eq!(*chosen.borrow(), [4]);
    // A choice that is off takes no click
    h.on(cx, w, |window, cx| window.click("pick", cx));
    let three = h.on(cx, w, |window, _| window.find_all("choice")[2].bounds().center());
    h.on(cx, w, |window, cx| click_at(window, three, cx));
    assert_eq!(*chosen.borrow(), [4]);
}

#[gpui_kit::test]
fn enter_opens_the_arrows_move_and_enter_chooses(cx: &mut TestAppContext) {
    let (mut h, w, chosen) = open(cx);
    focus_list(&mut h, cx, w);
    h.on(cx, w, |window, cx| window.press("enter", cx));
    assert_eq!(entries(&mut h, cx, w).len(), 4, "Enter opens the list");
    // From the top: One, Two, then Four (Three is off and skipped)
    h.on(cx, w, |window, cx| {
        window.press("down", cx);
        window.press("down", cx);
        window.press("down", cx);
    });
    h.on(cx, w, |window, cx| window.press("enter", cx));
    assert_eq!(*chosen.borrow(), [4]);
    assert!(entries(&mut h, cx, w).is_empty());
    // Up goes back
    h.on(cx, w, |window, cx| window.press("space", cx));
    assert_eq!(entries(&mut h, cx, w).len(), 4, "Space opens the list");
    h.on(cx, w, |window, cx| {
        window.press("down", cx);
        window.press("down", cx);
        window.press("up", cx);
    });
    h.on(cx, w, |window, cx| window.press("enter", cx));
    assert_eq!(*chosen.borrow(), [4, 1]);
}

#[gpui_kit::test]
fn escape_closes_the_list_without_choosing(cx: &mut TestAppContext) {
    let (mut h, w, chosen) = open(cx);
    focus_list(&mut h, cx, w);
    h.on(cx, w, |window, cx| window.press("enter", cx));
    h.on(cx, w, |window, cx| window.press("down", cx));
    h.on(cx, w, |window, cx| window.press("escape", cx));
    assert!(entries(&mut h, cx, w).is_empty());
    assert!(chosen.borrow().is_empty());
    // The keyboard is back on the list's button
    assert_eq!(h.on(cx, w, |window, _| window.find("pick").focused()), Some(true));
}
