//! Taking over the pairings Windows' Bluetooth made with the adapter BlueCaster
//! now has on WinUSB (issue #86): in the setup's device step only. Shown only
//! while Windows paired an audio device through this adapter that BlueCaster
//! has not saved (other radios' pairings are no use here, and Windows may
//! have no radio running at all). Its button lists those devices, all ticked;
//! the core reads the keys of the ticked ones after an administrator prompt
//! and saves them with Windows' names.

use gpui_kit::component::Disableable as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AsyncApp, AsyncWindowContext, Context, Entity, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, WeakEntity, Window, div, px,
};
use ipc::{Method, WindowsImport, WindowsPairedDevice};
use ui_model::i18n::t;
use ui_model::view::devices::{import_notice, unsaved_windows, winusb_adapter};

use super::device_picker::ask_busy;
use super::dialog::Question;
use super::{ButtonKind, IconName, button, icon};
use crate::app::{self, NoticeKind};
use crate::core::Core;
use crate::theme::{self, ActivePalette as _};

pub struct ImportPairings {
    core: Entity<Core>,
    /// The adapter the list was asked for
    asked: Option<String>,
    /// What Windows paired through this adapter (no prompt to read the list)
    windows: Vec<WindowsPairedDevice>,
    /// The addresses left unticked in the list
    skipped: Vec<String>,
    busy: bool,
}

impl ImportPairings {
    pub fn new(cx: &mut Context<Self>) -> ImportPairings {
        let core = app::core(cx);
        cx.observe(&core, |this: &mut Self, _, cx| {
            this.refetch(false, cx);
            cx.notify();
        })
        .detach();
        let mut this = ImportPairings { core, asked: None, windows: Vec::new(), skipped: Vec::new(), busy: false };
        this.refetch(false, cx);
        this
    }

    /// Asks for the list again when the adapter changed, or `always`
    fn refetch(&mut self, always: bool, cx: &mut Context<Self>) {
        let adapter = self.core.read(cx).snap().and_then(winusb_adapter);
        if adapter == self.asked && !always {
            return;
        }
        self.asked = adapter.clone();
        let Some(adapter) = adapter else {
            self.windows.clear();
            return;
        };
        let task = app::request::<Option<Vec<WindowsPairedDevice>>>(
            Method::DeviceWindowsPairedThrough { adapter: adapter.clone() },
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let list = task.await.ok().flatten().unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                if this.asked.as_deref() == Some(adapter.as_str()) {
                    this.windows = list;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn unsaved(&self, cx: &Context<Self>) -> Vec<WindowsPairedDevice> {
        let devices = self.core.read(cx).snap().map(|s| s.devices.as_slice()).unwrap_or_default();
        unsaved_windows(&self.windows, devices)
    }

    fn ticked(&self, cx: &Context<Self>) -> Vec<String> {
        self.unsaved(cx).into_iter().map(|d| d.address).filter(|a| !self.skipped.contains(a)).collect()
    }

    fn import_now(&mut self, adapter: String, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = self.ticked(cx);
        if chosen.is_empty() || self.busy {
            return;
        }
        self.busy = true;
        cx.notify();
        let task = app::run(
            app::request::<WindowsImport>(Method::DeviceImportWindowsPairings { adapter, addresses: Some(chosen) }, cx),
            cx,
        );
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
            let result = task.await;
            let _ = cx.update(|window, cx| {
                window.close_dialog(cx);
                if let Some(r) = &result {
                    app::notify(import_notice(r), NoticeKind::Info, cx);
                }
                let _ = this.update(cx, |this, cx| {
                    this.busy = false;
                    if result.as_ref().is_some_and(|r| r.adapter_found) {
                        this.refetch(true, cx);
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }

    fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(adapter) = self.asked.clone() else { return };
        self.skipped.clear();
        let (weak, weak_body, weak_confirm) =
            (cx.entity().downgrade(), cx.entity().downgrade(), cx.entity().downgrade());
        ask_busy(
            Question::new(t("ui.adapter.import_windows"))
                .confirm(t("ui.devices.import_one"))
                .confirm_icon(IconName::Shield),
            move |cx| weak.upgrade().is_some_and(|e| e.read(cx).busy),
            move |_, cx| {
                let Some(this) = weak_body.upgrade() else { return div().into_any_element() };
                let p = *cx.palette();
                let s = this.read(cx);
                let devices = s.core.read(cx).snap().map(|s| s.devices.clone()).unwrap_or_default();
                let list = unsaved_windows(&s.windows, &devices);
                let busy = s.busy;
                div()
                    // Many devices scroll: the buttons stay in reach
                    .id("import-list")
                    .flex()
                    .flex_col()
                    .gap(theme::S2)
                    .max_h(px(320.))
                    .overflow_y_scrollbar()
                    .children(list.into_iter().enumerate().map(|(i, d)| {
                        let address = d.address.clone();
                        let weak = this.downgrade();
                        let name = if d.name.is_empty() { d.address.clone() } else { d.name.clone() };
                        div()
                            .flex()
                            .items_center()
                            .gap(theme::S2)
                            .child(
                                Checkbox::new(("import", i))
                                    .checked(!s.skipped.contains(&d.address))
                                    .disabled(busy)
                                    .accessibility_label(name.clone())
                                    .on_click(move |on: &bool, _, cx| {
                                        let (on, address) = (*on, address.clone());
                                        let _ = weak.update(cx, |this, cx| {
                                            this.skipped.retain(|a| *a != address);
                                            if !on {
                                                this.skipped.push(address);
                                            }
                                            cx.notify();
                                        });
                                    }),
                            )
                            .child(icon(IconName::Headphones))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .line_clamp(2)
                                            .text_ellipsis()
                                            .child(name),
                                    )
                                    .when(!d.name.is_empty(), |x| {
                                        x.child(
                                            div()
                                                .font_family(theme::FONT_MONO)
                                                .text_size(theme::FS_SMALL)
                                                .text_color(p.text_2)
                                                .child(d.address.clone()),
                                        )
                                    }),
                            )
                    }))
                    .into_any_element()
            },
            move |window, cx| {
                let adapter = adapter.clone();
                let _ = weak_confirm.update(cx, |this, cx| this.import_now(adapter, window, cx));
            },
            window,
            cx,
        );
    }
}

impl Render for ImportPairings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.asked.is_none() || self.unsaved(cx).is_empty() {
            return div();
        }
        ui_row(cx).child(
            button("import-one", t("ui.devices.import_one"), ButtonKind::Default)
                .on_click(cx.listener(|this, _, window, cx| this.open(window, cx))),
        )
    }
}

/// The row's text: what it does, and the hint
fn ui_row(cx: &Context<ImportPairings>) -> gpui_kit::Div {
    super::row().child(
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .child(div().child(t("ui.adapter.import_windows")))
            .child(super::hint(t("ui.devices.import_hint"), cx)),
    )
}
