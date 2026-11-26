//! A saved device's services and pairing keys (issue #86), from the
//! Services tag of its row. An LE Audio entry has its GATT services from its
//! last connection and its LE keys only. In developer mode the link key BlueCaster keeps
//! for it and the LE keys derived from it (LE Audio on) come first, folded
//! until clicked and only then asked for, each "not available" when
//! BlueCaster has none; below them the services it lists (the last
//! connection's while it is not connected), the part that grows. Both are
//! tables of the same look.
//!
//! The rows are worked out in ui_model::view::devices (key_rows).

use std::time::Duration;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AccessibleAction, AnyElement, App, AppContext as _, AsyncApp, Context, Div, Entity, FontWeight,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Role, StatefulInteractiveElement as _,
    Styled as _, WeakEntity, Window, div, px,
};
use ipc::{Device, DevicePairing, Method, ServiceList};
use ui_model::i18n::t;
use ui_model::pairing::service_label;
use ui_model::view::devices::{BUSY_RETRIES, BUSY_RETRY_MS, KeyRow, key_rows, le_missing_text, retry_services};

use super::dialog::{self, Question};
use super::{IconName, icon, icon_button};
use crate::app::{self, NoticeKind};
use crate::host;
use crate::theme::{self, ActivePalette as _};

/// What the dialog shows; an error is the answer here (not connected, no
/// answer): shown in place
struct PairingState {
    device: Device,
    keys_open: bool,
    /// The keys, once unfolded and answered
    pairing: Option<Result<DevicePairing, String>>,
    /// The services, once answered
    services: Option<Result<ServiceList, String>>,
}

impl PairingState {
    /// Busy: still loading, asked again shortly, never shown as an error
    /// unless it lasts
    fn ask_services(&mut self, cx: &mut Context<Self>) {
        let (address, le) = (self.device.address.clone(), self.device.le_audio);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut retries = BUSY_RETRIES;
            loop {
                let method = Method::DeviceServices { address: address.clone(), le_audio: le.then_some(true) };
                let task = cx.update(|cx| app::request::<ServiceList>(method, cx));
                match task.await {
                    Err(e) if retry_services(&e.code, retries) => {
                        retries -= 1;
                        cx.background_executor().timer(Duration::from_millis(BUSY_RETRY_MS)).await;
                        // The dialog closed meanwhile: not asked again
                        if this.upgrade().is_none() {
                            return;
                        }
                    }
                    answer => {
                        let _ = this.update(cx, |this, cx| {
                            this.services = Some(answer.map_err(|e| app::error_text(&e)));
                            cx.notify();
                        });
                        return;
                    }
                }
            }
        })
        .detach();
    }

    /// The keys only in developer mode (the core refuses them otherwise),
    /// and only once unfolded: secrets, not on screen with every opening
    fn toggle_keys(&mut self, cx: &mut Context<Self>) {
        self.keys_open = !self.keys_open;
        cx.notify();
        if !self.keys_open || self.pairing.is_some() {
            return;
        }
        let task = app::request::<DevicePairing>(Method::DevicePairing { address: self.device.address.clone() }, cx);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let answer = task.await.map_err(|e| app::error_text(&e));
            let _ = this.update(cx, |this, cx| {
                this.pairing = Some(answer);
                cx.notify();
            });
        })
        .detach();
    }
}

fn copy(text: String, cx: &mut App) {
    host::copy_text(text, cx);
    app::notify(t("ui.pairing.copied"), NoticeKind::Info, cx);
}

/// A table's look: a heading row, then rows with a line between them
/// The two columns' widths: a fixed one, or None for the rest. The rows use
/// the same, so a heading stands over its column.
type Widths = [Option<gpui_kit::Pixels>; 2];
const KEY_COLUMNS: Widths = [Some(px(160.)), None];
const SERVICE_COLUMNS: Widths = [None, Some(px(300.))];

/// A cell of a column of `widths`
fn column(widths: Widths, i: usize) -> Div {
    match widths[i] {
        Some(w) => div().w(w).flex_none(),
        None => div().flex_1().min_w_0(),
    }
}

fn table(head: [String; 2], widths: Widths, rows: Vec<Div>, cx: &App) -> Div {
    let p = *cx.palette();
    div()
        .flex()
        .flex_col()
        .mt(theme::S2)
        .text_size(theme::FS_BODY)
        .child(
            div()
                .flex()
                .gap(theme::S3)
                .py(theme::S1)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.text_2)
                .text_size(theme::FS_SMALL)
                .children(head.into_iter().enumerate().map(|(i, h)| column(widths, i).child(h))),
        )
        .children(rows.into_iter().map(|r| r.py(theme::S2).border_t_1().border_color(p.border)))
}

/// One key: its hex digits (upper case) in the order Bluetooth sends them,
/// with a copy button, or "not available"
fn key_row(i: usize, row: KeyRow, cx: &App) -> Div {
    let p = *cx.palette();
    let value: AnyElement = match row.value {
        Some(v) => div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(theme::S2)
            .child(div().font_family(theme::FONT_MONO).text_size(theme::FS_SMALL).child(v.clone()))
            .when_some(row.tag, |d, tag| d.child(super::device_picker::tag(tag, cx)))
            .child(
                icon_button(("copy-key", i), IconName::Copy, t("ui.pairing.copy_key"))
                    .on_click(move |_, _, cx| copy(v.clone(), cx)),
            )
            .into_any_element(),
        None => div().text_color(p.text_2).child(t("ui.pairing.not_available")).into_any_element(),
    };
    div()
        .flex()
        .items_center()
        .gap(theme::S3)
        .child(column(KEY_COLUMNS, 0).font_weight(FontWeight::SEMIBOLD).child(row.name))
        .child(column(KEY_COLUMNS, 1).child(value))
}

/// The keys' heading that folds them: pressed, clicked, Enter or Space;
/// assistive technology sees whether it is unfolded and can unfold or fold it
fn keys_fold(state: &Entity<PairingState>, open: bool, cx: &App) -> impl IntoElement + use<> {
    let p = *cx.palette();
    let toggle = {
        let weak = state.downgrade();
        move |cx: &mut App| {
            let _ = weak.update(cx, |s, cx| s.toggle_keys(cx));
        }
    };
    let label = t("ui.pairing.key_title");
    let (click, key, expand, collapse) = (toggle.clone(), toggle.clone(), toggle.clone(), toggle);
    div()
        .id("keys-fold")
        .role(Role::Button)
        .aria_label(label.clone())
        .aria_expanded(open)
        .tab_index(0)
        .flex()
        .items_center()
        .gap(theme::S2)
        .h(px(32.))
        .px(theme::S3)
        .rounded(theme::RADIUS)
        .cursor_pointer()
        .hover(|s| s.bg(p.surface_2))
        .focus_visible(|s| s.bg(p.surface_2))
        .on_click(move |_, _, cx| click(cx))
        .on_key_down(move |e: &KeyDownEvent, _, cx| {
            if matches!(e.keystroke.key.as_str(), "enter" | "space") {
                key(cx);
            }
        })
        .on_a11y_action(AccessibleAction::Expand, move |_, _, cx| {
            if !open {
                expand(cx);
            }
        })
        .on_a11y_action(AccessibleAction::Collapse, move |_, _, cx| {
            if open {
                collapse(cx);
            }
        })
        .child(icon(if open { IconName::ChevronDown } else { IconName::ChevronRight }))
        .child(label)
}

fn body(state: &Entity<PairingState>, cx: &App) -> AnyElement {
    let s = state.read(cx);
    let p = *cx.palette();
    let keys_of = app::core(cx).read(cx).snap().is_some_and(|s| s.settings.developer_mode);
    let le = s.device.le_audio;
    let hint = |text: String| super::hint(text, cx);
    let keys = keys_of.then(|| {
        let fold = keys_fold(state, s.keys_open, cx);
        let part = div().flex().flex_col().child(div().flex().child(fold));
        match s.pairing.as_ref().filter(|_| s.keys_open) {
            Some(Ok(pairing)) => {
                let rows = key_rows(le, pairing).into_iter().enumerate().map(|(i, r)| key_row(i, r, cx)).collect();
                part.child(table([t("ui.pairing.col_name"), t("ui.pairing.col_value")], KEY_COLUMNS, rows, cx))
                    // Why there are no LE keys (issue #86)
                    .when_some(le_missing_text(pairing), |d, why| d.child(hint(why).mt(theme::S2)))
            }
            Some(Err(e)) => part.child(hint(e.clone())),
            None => part,
        }
    });
    let services = match &s.services {
        None => div().child(hint(t("ui.pairing.services_loading"))),
        Some(Err(e)) => div().child(hint(e.clone())),
        Some(Ok(list)) => div()
            .flex()
            .flex_col()
            .when(list.from_last_connection, |d| d.child(hint(t("ui.pairing.services_last"))))
            .map(|d| {
                if list.services.is_empty() {
                    return d.child(hint(t("ui.pairing.services_none")));
                }
                let rows = list
                    .services
                    .iter()
                    .map(|r| {
                        let name = service_label(r).unwrap_or_else(|| t("ui.pairing.unknown_service"));
                        div().flex().items_start().gap(theme::S3).child(column(SERVICE_COLUMNS, 0).child(name)).child(
                            column(SERVICE_COLUMNS, 1)
                                .flex()
                                .flex_col()
                                .font_family(theme::FONT_MONO)
                                .text_size(theme::FS_SMALL)
                                .text_color(p.text_2)
                                .children(r.uuids.iter().map(|u| div().child(u.clone()))),
                        )
                    })
                    .collect();
                d.child(table([t("ui.pairing.col_name"), t("ui.pairing.col_guid")], SERVICE_COLUMNS, rows, cx))
            }),
    };
    div()
        .flex()
        .flex_col()
        .gap(theme::S4)
        .when_some(keys, |d, k| d.child(k))
        .child(
            div()
                .flex()
                .flex_col()
                .when(keys_of, |d| d.child(div().font_weight(FontWeight::SEMIBOLD).child(t("ui.pairing.services"))))
                .child(services),
        )
        .into_any_element()
}

/// Opens the dialog of a saved device: its services, asked at once, and in
/// developer mode its keys, folded
pub fn open(device: &Device, window: &mut Window, cx: &mut App) {
    let state = cx.new(|cx| {
        let mut s = PairingState { device: device.clone(), keys_open: false, pairing: None, services: None };
        s.ask_services(cx);
        s
    });
    let name = if device.name.is_empty() { t("device.name_unknown") } else { device.name.clone() };
    dialog::ask(
        Question::new(t("ui.pairing.services")).subtitle(format!("{name} {}", device.address)),
        move |_, cx| body(&state, cx),
        |_, _| {},
        window,
        cx,
    );
}
