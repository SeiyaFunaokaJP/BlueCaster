//! Turning on keyboards and mice through BlueCaster (experimental, issue
//! #413). Dangerous: their input exists only while BlueCaster runs, so the
//! dialog asks for more than a click. Both statements ticked, the core's
//! code typed back (no paste), and a wait of HID_WAIT_SECONDS, so that the
//! risks are on screen long enough to be read. The core checks the code
//! and the statements again (hid.enable).

use std::time::Duration;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::{Dialog, DialogFooter};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, AsyncApp, AsyncWindowContext, Context, Entity, FontWeight, IntoElement, ParentElement as _,
    Render, Styled as _, Subscription, Task, WeakEntity, Window, div, px,
};
use ipc::{HidCode, Method};
use ui_model::i18n::t;
use ui_model::t;
use ui_model::view::hid::{EnableInputs, HID_WAIT_SECONDS, digits, enable_ready, risks, wait_left};

use crate::app;
use crate::core::Core;
use crate::screens::hid_host::warn_title;
use crate::theme::{self, ActivePalette as _};
use crate::ui::{self, ButtonKind, IconName, icon};

/// The dialog's state; each opening makes a new one, so it starts over:
/// nothing ticked, a new code, the full wait
pub struct HidEnableDialog {
    core: Entity<Core>,
    code: Option<String>,
    input: Entity<InputState>,
    other_input: bool,
    risk: bool,
    left: u32,
    error: Option<String>,
    busy: bool,
    _timer: Task<()>,
    _subs: Vec<Subscription>,
}

/// Opens the dialog
pub fn open_hid_enable_dialog(window: &mut Window, cx: &mut App) {
    let state = cx.new(|cx| HidEnableDialog::new(window, cx));
    window.open_dialog(cx, move |dialog, window, cx| {
        let dialog = dialog.width(ui::dialog::width(window, cx));
        HidEnableDialog::build(&state, dialog, cx)
    });
}

impl HidEnableDialog {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> HidEnableDialog {
        let core = app::core(cx);
        // Six characters at most (maxLength), as typed
        let input = cx.new(|cx| InputState::new(window, cx).validate(|s, _| s.chars().count() <= 6));
        let subs = vec![
            cx.observe(&core, |_, _, cx| cx.notify()),
            cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => cx.notify(),
                InputEvent::PressEnter { .. } => this.confirm(window, cx),
                _ => {}
            }),
        ];
        // The executor's clock: the wall clock, and the test clock in tests
        let opened = cx.background_executor().now();
        let timer = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            loop {
                cx.background_executor().timer(Duration::from_millis(250)).await;
                let elapsed = cx.background_executor().now().saturating_duration_since(opened);
                let left = wait_left(elapsed.as_millis() as u64);
                let alive = this.update(cx, |this, cx| {
                    this.left = left;
                    cx.notify();
                });
                if alive.is_err() || left == 0 {
                    break;
                }
            }
        });
        let mut this = HidEnableDialog {
            core,
            code: None,
            input,
            other_input: false,
            risk: false,
            left: HID_WAIT_SECONDS,
            error: None,
            busy: false,
            _timer: timer,
            _subs: subs,
        };
        this.fetch_code(cx);
        this
    }

    fn fetch_code(&mut self, cx: &mut Context<Self>) {
        self.code = None;
        let task = app::request::<HidCode>(Method::HidCode, cx);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let r = task.await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    Ok(c) => this.code = Some(c.code),
                    Err(e) => this.error = Some(app::error_text(&e)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn typed(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    fn ready(&self, cx: &App) -> bool {
        let typed = self.typed(cx);
        enable_ready(&EnableInputs {
            other_input: self.other_input,
            risk: self.risk,
            typed: &typed,
            left: self.left,
            has_code: self.code.is_some(),
        })
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.ready(cx) || self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        let task = app::request::<serde_json::Value>(
            Method::HidEnable {
                code: digits(&self.typed(cx)),
                other_input_at_hand: self.other_input,
                risk_understood: self.risk,
            },
            cx,
        );
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
            let r = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match r {
                    Ok(_) => window.close_dialog(cx),
                    // A code counts once: a wrong or expired one needs a new one
                    Err(e) => {
                        this.error = Some(app::error_text(&e));
                        this.input.update(cx, |i, cx| i.set_value("", window, cx));
                        this.fetch_code(cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The dialog around the state: its title, body and buttons, drawn again
    /// whenever the state changes
    fn build(state: &Entity<Self>, dialog: Dialog, cx: &mut App) -> Dialog {
        let s = state.read(cx);
        let (busy, left, ready) = (s.busy, s.left, s.ready(cx));
        let confirm_label = if left > 0 { t!("ui.dev.hid_enable_wait", left) } else { t("ui.dev.hid_enable_confirm") };
        let cancel = ui::button("dialog-cancel", t("modal.cancel"), ButtonKind::Default)
            .disabled(busy)
            .on_click(|_, window, cx| window.close_dialog(cx));
        let confirm =
            ui::button("dialog-confirm", confirm_label, ButtonKind::Danger).loading(busy).disabled(!ready).on_click({
                let state = state.clone();
                move |_, window, cx| state.update(cx, |this, cx| this.confirm(window, cx))
            });
        let busy_now = state.clone();
        dialog
            .title(warn_title(t("ui.dev.hid_enable_title"), cx))
            .close_button(false)
            .keyboard(!busy)
            .overlay_closable(!busy)
            // While the confirmed request runs the dialog stays
            .on_cancel(move |_, _, cx| !busy_now.read(cx).busy)
            // Enter confirms through the code field only (once ready), or
            // presses the focused button; it never closes the dialog by itself
            .on_ok(|_, _, cx| {
                cx.propagate();
                false
            })
            .child(state.clone())
            .footer(DialogFooter::new().child(cancel).child(confirm).into_any_element())
    }
}

impl Render for HidEnableDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let service = self.core.read(cx).snap().is_some_and(|s| s.service_mode);
        let lead = t(if service { "ui.dev.hid_enable_lead_service" } else { "ui.dev.hid_enable_lead" });
        let risks = risks(service).iter().map(|k| {
            // min_w_0: the text wraps inside the dialog instead of widening the row
            div()
                .flex()
                .items_start()
                .gap(theme::S2)
                .child(div().child("•"))
                .child(div().flex_1().min_w_0().child(t(k)))
        });
        div()
            .flex()
            .flex_col()
            .gap(theme::S3)
            .child(div().font_weight(FontWeight::SEMIBOLD).child(lead))
            .child(div().flex().flex_col().gap(theme::S1).pl(theme::S2).children(risks))
            .child(
                Checkbox::new("hid-confirm-other")
                    .label(t("ui.dev.hid_confirm_other"))
                    .checked(self.other_input)
                    .on_change(cx.listener(|this, v: &bool, _, cx| {
                        this.other_input = *v;
                        cx.notify();
                    })),
            )
            .child(Checkbox::new("hid-confirm-risk").label(t("ui.dev.hid_confirm_risk")).checked(self.risk).on_change(
                cx.listener(|this, v: &bool, _, cx| {
                    this.risk = *v;
                    cx.notify();
                }),
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(theme::S2)
                    .mt(theme::S2)
                    .child(div().child(t("ui.dev.hid_code_prompt")))
                    // Read and typed, never copied
                    .child(
                        div()
                            .font_family(theme::FONT_MONO)
                            .text_size(theme::FS_PAGE)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.code.clone().unwrap_or_else(|| "······".into())),
                    )
                    .child(
                        div().w(px(140.)).child(
                            Input::new(&self.input)
                                .aria_label(t("ui.dev.hid_code_input"))
                                .font_family(theme::FONT_MONO)
                                // Typed, not pasted
                                .on_paste(|_, _, _| true),
                        ),
                    )
                    .when_some(self.error.clone(), |d, e| {
                        d.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(theme::S1)
                                .text_size(theme::FS_SMALL)
                                .text_color(p.bad_text)
                                .child(icon(IconName::ErrorCircle).small())
                                .child(e),
                        )
                    }),
            )
    }
}

#[cfg(test)]
mod interaction_tests;
