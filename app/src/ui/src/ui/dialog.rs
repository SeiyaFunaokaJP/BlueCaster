//! A modal question. Escape and the backdrop cancel; Tab stays inside.
//!
//! [`show`] draws every dialog the same way: the title (after a
//! warning sign with `warn`), a subtitle under it, the body, and Cancel /
//! the extra button / the confirming button. A dialog without a confirming
//! button only closes: what is in it applies at once, so it has only the
//! close control in its title bar. While `busy` the confirming button
//! spins and Cancel, Escape, the backdrop and the close control do nothing:
//! closing the dialog would hide the spinner, not stop the work.
//!
//! [`ask`] closes before its action runs; [`ask_busy`] stays open while a
//! request the caller tracks runs; [`ask_run`] tracks the request itself and
//! closes when it went well (or always, as the caller decides).

use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, AsyncWindowContext, Entity, FontWeight, Global, IntoElement, ParentElement as _,
    Pixels, SharedString, Styled as _, Task, Window, WindowId, div, px,
};
use ui_model::i18n::t;

use super::{ButtonKind, IconName, button, icon, icon_button};
use crate::theme::{self, ActivePalette as _};

type Action = Rc<dyn Fn(&mut Window, &mut App)>;

/// A second way to go on, before the confirming button and the one
/// recommended: the confirming button then is a plain one
#[derive(Clone)]
pub struct Extra {
    pub label: SharedString,
    pub icon: Option<IconName>,
    pub on_click: Action,
}

/// What a question's dialog shows around its body. `confirm_disabled` and
/// `extra` are set as fields.
#[derive(Clone)]
pub struct Question {
    pub title: SharedString,
    /// A line under the title
    pub subtitle: Option<SharedString>,
    /// The confirming button's text; None: the dialog only closes (with
    /// the close control in its title bar, no buttons under it)
    pub confirm: Option<SharedString>,
    /// Shield when confirming asks for administrator rights
    pub confirm_icon: Option<IconName>,
    /// The other button's text
    pub cancel: SharedString,
    /// The confirming button destroys something
    pub danger: bool,
    /// A warning sign before the title: confirming loses something (issue #86)
    pub warn: bool,
    /// The confirmed action runs: the dialog stays, nothing cancels it
    pub busy: bool,
    /// The confirming button cannot be pressed yet (something in the
    /// dialog is still missing)
    pub confirm_disabled: bool,
    pub extra: Option<Extra>,
    /// Runs when Cancel, Escape, the backdrop or the close control close it
    pub on_cancel: Option<Action>,
}

impl Question {
    pub fn new(title: impl Into<SharedString>) -> Question {
        Question {
            title: title.into(),
            subtitle: None,
            confirm: None,
            confirm_icon: None,
            cancel: t("modal.cancel").into(),
            danger: false,
            warn: false,
            busy: false,
            confirm_disabled: false,
            extra: None,
            on_cancel: None,
        }
    }

    pub fn subtitle(mut self, text: impl Into<SharedString>) -> Self {
        self.subtitle = Some(text.into());
        self
    }

    pub fn confirm(mut self, label: impl Into<SharedString>) -> Self {
        self.confirm = Some(label.into());
        self
    }

    pub fn confirm_icon(mut self, icon: IconName) -> Self {
        self.confirm_icon = Some(icon);
        self
    }

    pub fn cancel(mut self, label: impl Into<SharedString>) -> Self {
        self.cancel = label.into();
        self
    }

    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    pub fn warn(mut self) -> Self {
        self.warn = true;
        self
    }

    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    pub fn on_cancel(mut self, on_cancel: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_cancel = Some(Rc::new(on_cancel));
        self
    }
}

/// The widest a dialog gets: a page's cards (72rem less the page's padding)
const MAX_WIDTH: Pixels = px(1104.);

/// The width of each window's page (beside the navigation), told by the
/// window as it draws; a window without one (the setup) has none
#[derive(Default)]
struct PageWidths(HashMap<WindowId, Pixels>);

impl Global for PageWidths {}

/// The window's page is `width` wide (None: it shows no page)
pub fn set_page_width(window: &Window, width: Option<Pixels>, cx: &mut App) {
    let id = window.window_handle().window_id();
    let widths = &mut cx.default_global::<PageWidths>().0;
    match width {
        Some(w) => widths.insert(id, w),
        None => widths.remove(&id),
    };
}

/// The widest a question gets (Cancel and a confirming button under a few
/// lines): Windows' own are no wider
const QUESTION_WIDTH: Pixels = px(560.);

/// A dialog with content of its own (no confirming button) is as wide as the
/// page's cards, centred in the window: the
/// page's width less its padding, at most [`MAX_WIDTH`]; without a page, the
/// window's width less the same margin
pub fn width(window: &Window, cx: &App) -> Pixels {
    let viewport = window.viewport_size().width;
    let page = cx
        .try_global::<PageWidths>()
        .and_then(|p| p.0.get(&window.window_handle().window_id()).copied())
        .unwrap_or(viewport);
    (page.min(viewport) - theme::S6 * 2.).min(MAX_WIDTH).max(px(0.))
}

/// Opens the dialog. `question` and `body` run again whenever the dialog is
/// drawn, so they read the state they show (make entities such as text
/// inputs before, not in them). `on_confirm` runs when the confirming
/// button is chosen and decides itself when to close the dialog
/// (`window.close_dialog(cx)`).
pub fn show(
    question: impl Fn(&mut Window, &mut App) -> Question + 'static,
    body: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    on_confirm: impl Fn(&mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let on_confirm: Action = Rc::new(on_confirm);
    window.open_dialog(cx, move |dialog, window, cx| {
        let q = question(window, cx);
        let busy = q.busy;
        // Cancel, Escape, the backdrop and the close control: nothing while busy
        let cancel: Action = {
            let on_cancel = q.on_cancel.clone();
            Rc::new(move |window, cx| {
                if busy {
                    return;
                }
                window.close_dialog(cx);
                if let Some(f) = &on_cancel {
                    f(window, cx);
                }
            })
        };
        let p = *cx.palette();
        let heading = div()
            .flex()
            .flex_col()
            .gap(theme::S1)
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .flex()
                    // The sign beside the first line of a title that wraps
                    .items_start()
                    .gap(theme::S2)
                    .text_size(theme::FS_TITLE)
                    .when(q.warn, |d| {
                        d.child(div().flex().pt(px(4.)).text_color(p.warn_text).child(icon(IconName::Warning)))
                    })
                    .child(div().flex_1().min_w_0().child(super::text(q.title.clone()))),
            )
            .when_some(q.subtitle.clone(), |d, s| {
                d.child(
                    div()
                        .font_weight(FontWeight::NORMAL)
                        .text_size(theme::FS_SMALL)
                        .text_color(p.text_2)
                        .child(super::text(s)),
                )
            });
        let head =
            div().flex().items_start().justify_between().gap(theme::S3).child(heading).when(q.confirm.is_none(), |d| {
                let cancel = cancel.clone();
                d.child(
                    icon_button("dialog-close", IconName::Dismiss, t("modal.close"))
                        .small()
                        .on_click(move |_, window, cx| cancel(window, cx)),
                )
            });
        let veto = {
            let on_cancel = q.on_cancel.clone();
            move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| {
                if busy {
                    return false;
                }
                if let Some(f) = &on_cancel {
                    f(window, cx);
                }
                true
            }
        };
        // A question sits in the middle of the window, as Windows' do
        let question = q.confirm.is_some();
        let wide = width(window, cx);
        let dialog = dialog
            .width(if question { wide.min(QUESTION_WIDTH) } else { wide })
            .when(question, |d| d.centered())
            .title(head)
            .close_button(false)
            // Escape, the backdrop: as Cancel
            .on_cancel(veto)
            // Enter presses the focused button only: the key goes on to it
            .on_ok(|_, _, cx| {
                cx.propagate();
                false
            })
            .child(div().flex().flex_col().gap(theme::S3).child(body(window, cx)));
        let Some(label) = q.confirm.clone() else { return dialog };
        let mut footer = DialogFooter::new().child({
            let cancel = cancel.clone();
            button("dialog-cancel", q.cancel.clone(), ButtonKind::Default)
                .disabled(busy)
                .on_click(move |_, window, cx| cancel(window, cx))
        });
        // With the extra button, busy is the extra's work
        if let Some(x) = q.extra.clone() {
            let mut b = button("dialog-extra", x.label, ButtonKind::Strong)
                .loading(busy)
                .disabled(busy)
                .on_click(move |_, window, cx| (x.on_click)(window, cx));
            if let Some(i) = x.icon {
                b = b.icon(icon(i));
            }
            footer = footer.child(b);
        }
        let kind = if q.danger {
            ButtonKind::Danger
        } else if q.extra.is_some() {
            ButtonKind::Default
        } else {
            ButtonKind::Strong
        };
        let on_confirm = on_confirm.clone();
        let mut confirm = button("dialog-confirm", label, kind)
            .loading(busy && q.extra.is_none())
            .disabled(busy || q.confirm_disabled)
            .on_click(move |_, window, cx| on_confirm(window, cx));
        if let Some(i) = q.confirm_icon {
            confirm = confirm.icon(icon(i));
        }
        dialog.footer(footer.child(confirm).into_any_element())
    });
}

/// A question whose `on_confirm` runs once the dialog closed
pub fn ask(
    question: Question,
    body: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    on_confirm: impl Fn(&mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    show(
        move |_, _| question.clone(),
        body,
        move |window, cx| {
            window.close_dialog(cx);
            on_confirm(window, cx);
        },
        window,
        cx,
    );
}

/// A question whose confirming button runs a request: the dialog stays open
/// (the button spinning while `busy`) until `on_confirm` closes it with
/// `window.close_dialog(cx)`, so a failure can be tried again.
pub fn ask_busy(
    question: Question,
    busy: impl Fn(&App) -> bool + 'static,
    body: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    on_confirm: impl Fn(&mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    show(move |_, cx| question.clone().busy(busy(cx)), body, on_confirm, window, cx);
}

/// A question whose confirmed request `run` starts: `busy` (the caller's,
/// so the body can show it too) is true until the task ends, then the
/// dialog closes if the task gave true and stays for another try if not.
pub fn ask_run(
    question: impl Fn(&mut Window, &mut App) -> Question + 'static,
    busy: Entity<bool>,
    body: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    run: impl Fn(&mut Window, &mut App) -> Task<bool> + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let is_busy = busy.clone();
    show(
        move |window, cx| {
            let busy = *is_busy.read(cx);
            question(window, cx).busy(busy)
        },
        body,
        move |window, cx| {
            if *busy.read(cx) {
                return;
            }
            set(&busy, true, cx);
            let task = run(window, cx);
            let busy = busy.clone();
            window
                .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                    let close = task.await;
                    let _ = cx.update(|window, cx| {
                        set(&busy, false, cx);
                        if close {
                            window.close_dialog(cx);
                        }
                    });
                })
                .detach();
        },
        window,
        cx,
    );
}

/// A new `busy` for [`ask_run`]
pub fn busy_state(cx: &mut App) -> Entity<bool> {
    cx.new(|_| false)
}

fn set(busy: &Entity<bool>, value: bool, cx: &mut App) {
    busy.update(cx, |b, cx| {
        *b = value;
        cx.notify();
    });
    cx.refresh_windows();
}

#[cfg(test)]
mod interaction_tests;
