//! Adapter: the USB Bluetooth adapters with their driver and firmware,
//! WinUSB assign / restore, the chip for the firmware, and AFH.
//!
//! The rules (order, names, which controls a card shows) are in
//! ui_model::view::adapter, tested there; this file draws them.

use std::collections::HashMap;
use std::time::Duration;

use gpui_kit::component::Disableable as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Sizable as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, AsyncApp, Context, Div, ElementId, Entity, Focusable as _, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollAnchor, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, Task, WeakEntity, Window, WindowVisibility, div, px,
};
use ipc::{AdapterInfo, DiagInfo, DiagInfoParams, DriverKind, FirmwareFile, Method, ProbeResult, Snapshot};
use ui_model::chips::{
    AdapterFirmware, ChipLabels, ChipStatus, LoaderOutcome, TagStatus, VendorFirmware, adapter_firmware_tag,
    build_chip_options, chip_status, loader_outcome, vendor_firmware,
};
use ui_model::i18n::{Arg, t, tf, tm};
use ui_model::links::{ChipReport, LINKS, chip_report_url, vendor_firmware_url};
use ui_model::t;
use ui_model::view::adapter::{
    AfhMode, Tone, UseButton, adapter_use_button, afh_avoided, afh_channels_bad, afh_mode, afh_shown, afh_strictness,
    assign_enabled, assign_hint, chip_card, chip_hint, chip_setting, chosen_missing, custom_firmware, detected_chip,
    driver_firmware, is_active_adapter, is_chosen_adapter, le_audio_toggle_shown, loader_line, parse_afh_channels,
    show_choice, status_text, status_tone, vendor_box,
};
use ui_model::view::stream::hidden_by_hung;
// Other screens use these (Setup, Auracast, the chip tools)
pub use ui_model::view::adapter::{
    ADAPTER_POLL_MS, adapter_ids, adapter_ids_hint, adapter_name, adapter_supported, driver_text, usable_first,
};

use crate::app::{self, AdapterSection, AppState, NoticeKind, RouteName};
use crate::core::{Core, CoreEvent, WantKind};
use crate::host;
use crate::theme::{self, ActivePalette as _};
use crate::ui::stream::{
    WinusbMode, adapter_hung, error_panel, firmware_download, firmware_folder_button, winusb_dialog,
};
use crate::ui::{self, BannerKind, ButtonKind, Choice, IconName, icon};

/// An element id from a stable key (an adapter's instance ID)
fn key_id(prefix: &str, key: &str) -> ElementId {
    ElementId::Name(SharedString::from(format!("{prefix}-{key}")))
}

/// A small rounded tag (`.tag`, `.tag.st-*`), its title on hover
pub fn tag(
    id: ElementId,
    text: impl Into<SharedString>,
    status: Option<TagStatus>,
    title: Option<String>,
    cx: &App,
) -> AnyElement {
    let p = cx.palette();
    let (fg, line) = match status {
        Some(TagStatus::Good) => (p.good_text, p.good_line),
        Some(TagStatus::Warn) => (p.warn_text, p.warn_line),
        Some(TagStatus::Bad) => (p.bad_text, p.bad_line),
        None => (p.text_2, p.border_strong),
    };
    let d = div()
        .id(id)
        // A long one (Russian) wraps inside its box, not out of it
        .max_w_full()
        .flex()
        .items_center()
        .px(theme::S2)
        .min_h(px(20.))
        .border_1()
        .border_color(line)
        .rounded_full()
        .text_size(theme::FS_SMALL)
        .font_weight(FontWeight::NORMAL)
        .text_color(fg)
        .child(text.into());
    match title.filter(|t| !t.is_empty()) {
        Some(title) => d.tooltip(move |window, cx| Tooltip::new(title.clone()).build(window, cx)).into_any_element(),
        None => d.into_any_element(),
    }
}

/// A status line (`.status-line.st-*`): a dot and the text in its colour
fn status_line(tone: Tone, cx: &App) -> Div {
    let p = cx.palette();
    let (dot, text) = match tone {
        Tone::Good => (p.good, p.good_text),
        Tone::Warn => (p.warn, p.warn_text),
        Tone::Bad => (p.bad, p.bad_text),
        Tone::Idle => (p.text_3, p.text_2),
    };
    div()
        .flex()
        .items_center()
        .gap(theme::S2)
        .min_w_0()
        .text_color(text)
        .child(div().flex_none().size(px(8.)).rounded_full().bg(dot))
}

/// A list of file names (`.mono.files`)
fn file_list(files: &[String]) -> Div {
    div()
        .flex()
        .flex_col()
        .pl(theme::S5)
        .font_family(theme::FONT_MONO)
        .children(files.iter().map(|f| div().child(format!("• {f}"))))
}

/// An error under a control (`.field-error`)
fn field_error(text: String, cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .gap(theme::S1)
        .text_size(theme::FS_SMALL)
        .text_color(cx.palette().bad_text)
        .child(icon(IconName::ErrorCircle).small())
        .child(text)
}

pub struct Adapter {
    core: Entity<Core>,
    // The list anew, one request at a time
    listing: bool,
    // Developer mode: WinUSB also for a chip BlueCaster cannot use yet, and the
    // chip probe (read commands only) for whoever adds its loader
    probing: bool,
    // The last probe of each adapter (instance ID): its summary and the
    // report to send stay on the card
    probed: HashMap<String, ProbeResult>,
    // The *_fw.bin files in the settings folder: chips the table lacks
    files: Vec<FirmwareFile>,
    afh_input: Entity<InputState>,
    afh_channels: String,
    afh_manual: bool,
    /// The placeholder last given to the channels box (its mode)
    afh_placeholder: Option<bool>,
    scroll: ScrollHandle,
    afh_anchor: ScrollAnchor,
    firmware_anchor: ScrollAnchor,
    /// The section to bring into view once the page is laid out
    scroll_to: Option<AdapterSection>,
    route_arg: Option<AdapterSection>,
    drawn: bool,
    _poll: Task<()>,
}

impl Adapter {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Adapter {
        let core = app::core(cx);
        let state = app::state(cx);
        cx.observe(&core, |_, _, cx| cx.notify()).detach();
        // Auto: the Wi-Fi channels the running stack avoids now
        crate::core::want(WantKind::Link, cx);
        cx.subscribe(&core, |_, _, ev: &CoreEvent, cx| {
            if *ev == CoreEvent::Link {
                cx.notify();
            }
        })
        .detach();
        // #/adapter/afh (the receiver test links there): the AFH section in view;
        // #/adapter/firmware (Home's firmware bar): the firmware part
        let route_arg = state.read(cx).route.arg;
        cx.observe(&state, |this: &mut Adapter, state: Entity<AppState>, cx| {
            let route = state.read(cx).route;
            if route.name == RouteName::Adapter && route.arg != this.route_arg {
                this.route_arg = route.arg;
                this.scroll_to = route.arg;
                cx.notify();
            }
        })
        .detach();

        let (afh, developer) =
            core.read(cx).snap().map(|s| (s.settings.afh.clone(), s.settings.developer_mode)).unwrap_or_default();
        let manual = afh_mode(&afh, developer) == AfhMode::Manual;
        let afh_input = cx.new(|cx| InputState::new(window, cx));
        cx.subscribe_in(&afh_input, window, |this: &mut Adapter, input, ev: &InputEvent, _, cx| match ev {
            InputEvent::Change if this.afh_manual => {
                this.afh_channels = input.read(cx).value().to_string();
                cx.notify();
            }
            InputEvent::Blur | InputEvent::PressEnter { .. } if this.afh_manual => this.save_channels(cx),
            _ => {}
        })
        .detach();

        // v1 checked the firmware again when its dialog got the focus back: the
        // files may have been placed through Open Config Folder meanwhile. On
        // opening the screen and every few seconds while it is in view as well,
        // for an adapter plugged in or out
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.relist(cx);
            }
        })
        .detach();
        let poll = cx.spawn_in(window, async move |this: WeakEntity<Adapter>, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(ADAPTER_POLL_MS)).await;
                let alive = this.update_in(cx, |this, window, cx| {
                    if window.visibility() == WindowVisibility::Visible && !window.has_active_dialog(cx) {
                        this.relist(cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });

        let scroll = ScrollHandle::new();
        let mut this = Adapter {
            core,
            listing: false,
            probing: false,
            probed: HashMap::new(),
            files: Vec::new(),
            afh_input,
            afh_channels: if manual { afh } else { String::new() },
            afh_manual: manual,
            afh_placeholder: None,
            afh_anchor: ScrollAnchor::for_handle(scroll.clone()),
            firmware_anchor: ScrollAnchor::for_handle(scroll.clone()),
            scroll,
            scroll_to: route_arg,
            route_arg,
            drawn: false,
            _poll: poll,
        };
        this.relist(cx);
        this
    }

    /// The *_fw.bin files anew (the choice keeps the table's chips when it fails)
    fn load_files(&mut self, cx: &mut Context<Self>) -> Task<()> {
        let task = app::request::<Vec<FirmwareFile>>(Method::AdapterFirmwareFiles, cx);
        cx.spawn(async move |this: WeakEntity<Adapter>, cx: &mut AsyncApp| {
            if let Ok(files) = task.await {
                let _ = this.update(cx, |this, cx| {
                    this.files = files;
                    cx.notify();
                });
            }
        })
    }

    // The list anew, one request at a time
    fn relist(&mut self, cx: &mut Context<Self>) {
        if self.listing {
            return;
        }
        self.listing = true;
        // The core publishes the adapters as its state; the answer is not needed
        let list = app::request::<serde_json::Value>(Method::AdapterList, cx);
        let files = self.load_files(cx);
        cx.spawn(async move |this: WeakEntity<Adapter>, cx: &mut AsyncApp| {
            let _ = list.await;
            files.await;
            let _ = this.update(cx, |this, _| this.listing = false);
        })
        .detach();
    }

    fn choose_adapter(adapter_id: String, cx: &mut App) {
        app::save_settings(move |s| s.adapter_id = adapter_id, cx);
    }

    fn probe(&mut self, a: &AdapterInfo, cx: &mut Context<Self>) {
        self.probing = true;
        cx.notify();
        let id = a.instance_id.clone();
        let task = app::run(app::request::<ProbeResult>(Method::AdapterProbe { instance_id: id.clone() }, cx), cx);
        cx.spawn(async move |this: WeakEntity<Adapter>, cx: &mut AsyncApp| {
            let r = task.await;
            let _ = this.update(cx, |this, cx| {
                this.probing = false;
                if let Some(r) = r {
                    app::notify(t!("ui.adapter.probe_saved", &r.summary), NoticeKind::Info, cx);
                    host::reveal_path(&r.path, cx);
                    this.probed.insert(id, r);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// A prefilled chip report in the browser, with the probe report in it;
    /// one too long for the URL is shown in Explorer to drag into it
    fn report_chip(a: &AdapterInfo, r: &ProbeResult, version: String, cx: &mut App) {
        let info =
            app::run(app::request::<DiagInfo>(Method::DiagInfo(Some(DiagInfoParams { mask: Some(true) })), cx), cx);
        let (adapter, summary, report, path) =
            (a.display_name.clone(), r.summary.clone(), r.report.clone(), r.path.clone());
        cx.spawn(async move |cx: &mut AsyncApp| {
            let Some(info) = info.await else { return };
            let link = chip_report_url(&ChipReport { version, adapter, summary, info: info.text, report });
            cx.update(|cx| {
                if link.attach {
                    host::reveal_path(&path, cx);
                }
                host::open_url(&link.url, cx);
            });
        })
        .detach();
    }

    /// v1 FirmwareDialog: a chip choice saved; the core lists the adapters
    /// anew with the firmware state for this chip (and releases an idle
    /// stack); the folder may have changed as well
    fn choose_chip(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(snap) = self.core.read(cx).snap() else { return };
        let s = &snap.settings;
        let choice = build_chip_options(
            &chip_setting(s),
            &self.files,
            &ChipLabels {
                auto: &chip_auto_label(&snap.adapter.adapters),
                non_realtek: &t("firmware.chip_non_realtek"),
            },
        );
        let Some(o) = choice.options.into_iter().find(|o| o.key == key) else { return };
        if o.pid == s.bt_chip_pid && o.stem == s.bt_chip_fw_stem {
            return;
        }
        let saved = app::apply_settings(
            move |s| {
                s.bt_chip_pid = o.pid;
                s.bt_chip_fw_stem = o.stem;
            },
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Adapter>, cx: &mut AsyncApp| {
            if saved.await.is_some() {
                let Ok(files) = this.update(cx, |this, cx| this.load_files(cx)) else { return };
                files.await;
            }
        })
        .detach();
    }

    fn save_channels(&mut self, cx: &mut Context<Self>) {
        let Some(afh) = self.core.read(cx).snap().map(|s| s.settings.afh.clone()) else { return };
        if let Some(c) = parse_afh_channels(&self.afh_channels)
            && c != afh
        {
            app::save_settings(move |s| s.afh = c, cx);
        }
    }

    fn set_afh_mode(&mut self, v: AfhMode, cx: &mut Context<Self>) {
        self.afh_manual = v == AfhMode::Manual;
        if self.afh_manual {
            self.save_channels(cx);
        } else {
            app::save_settings(move |s| s.afh = v.setting().to_owned(), cx);
        }
        cx.notify();
    }

    fn head() -> Div {
        div()
            .flex()
            .items_center()
            .gap(theme::S3)
            .min_h(px(36.))
            .child(
                div()
                    .flex_1()
                    .font_family(theme::FONT_DISPLAY)
                    .text_size(theme::FS_PAGE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(t("ui.nav.adapter")),
            )
            .child(
                div().flex().flex_wrap().gap(theme::S2).child(
                    ui::button("adapter-restart", t("ui.adapter.restart"), ButtonKind::Default)
                        .icon(icon(IconName::ArrowReset))
                        .on_click(|_, _, cx| app::send(Method::AdapterRestart, cx)),
                ),
            )
    }

    /// The stack's state and which adapter it has open
    fn stack_line(snap: &Snapshot, cx: &App) -> Div {
        let adapter = &snap.adapter;
        let mut text = status_text(adapter.status);
        if let Some(a) = &adapter.active {
            let name = a.windows_name.as_deref().filter(|n| !n.is_empty()).unwrap_or(&a.display_name);
            text.push_str(" · ");
            text.push_str(&t!("ui.adapter.active_line", name));
        }
        status_line(status_tone(adapter.status), cx).child(div().flex_1().min_w_0().child(text))
    }

    fn adapter_card(
        &self,
        snap: &Snapshot,
        a: &AdapterInfo,
        i: usize,
        chip_at: Option<usize>,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = *cx.palette();
        let s = &snap.settings;
        let adapter = &snap.adapter;
        let developer = s.developer_mode;
        let chosen = s.adapter_id.clone();
        let id = a.instance_id.as_str();
        let active = is_active_adapter(a, adapter.active.as_ref());
        let drv = driver_text(a.driver);
        let fw = adapter_firmware_tag(&AdapterFirmware::from(a), &chip_setting(s), adapter.loader.as_ref());
        let fw_text = tf(fw.key, &fw.args.iter().flatten().map(|a| Arg::from(a.as_str())).collect::<Vec<_>>());
        let driver_fw = driver_firmware(a, s, adapter.rom_firmware);

        // Device Manager's name; the USB device's own on hover
        let device_name = a.device_name.clone();
        let name = div()
            .id(key_id("adapter-name", id))
            .font_family(theme::FONT_DISPLAY)
            .text_size(theme::FS_LARGE)
            .font_weight(FontWeight::SEMIBOLD)
            .min_w_0()
            .max_w_full()
            .child(adapter_name(a))
            .when(!device_name.is_empty(), |d| {
                d.tooltip(move |window, cx| Tooltip::new(device_name.clone()).build(window, cx))
            });
        let title = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(theme::S2)
            .child(name)
            .when(active, |d| {
                d.child(tag(key_id("active", id), t("ui.adapter.active"), Some(TagStatus::Good), None, cx))
            })
            .when(is_chosen_adapter(a, &chosen) && !active, |d| {
                d.child(tag(
                    key_id("chosen", id),
                    t("ui.adapter.chosen"),
                    Some(TagStatus::Warn),
                    Some(t("ui.adapter.chosen_hint")),
                    cx,
                ))
            });
        let ids_hint = adapter_ids_hint(a);
        let ids = div()
            .id(key_id("adapter-ids", id))
            .font_family(theme::FONT_MONO)
            .text_color(p.text_2)
            .child(adapter_ids(a))
            .when_some(ids_hint, |d, h| d.tooltip(move |window, cx| Tooltip::new(h.clone()).build(window, cx)));

        // Which adapter BlueCaster opens
        let use_button = show_choice(&adapter.adapters, &chosen)
            .then(|| adapter_use_button(a, &chosen, adapter.active.as_ref()))
            .flatten()
            .map(|b| match b {
                UseButton::Use => {
                    let instance_id = a.instance_id.clone();
                    ui::button(key_id("use", id), t("ui.adapter.use"), ButtonKind::Strong)
                        .icon(icon(IconName::Checkmark))
                        .tooltip(t("ui.adapter.use_hint"))
                        .on_click(move |_, _, cx| Adapter::choose_adapter(instance_id.clone(), cx))
                }
                UseButton::Auto => ui::button(key_id("use-auto", id), t("ui.adapter.use_auto"), ButtonKind::Default)
                    .icon(icon(IconName::ArrowReset))
                    .tooltip(t("ui.adapter.use_auto_hint"))
                    .on_click(|_, _, cx| Adapter::choose_adapter(String::new(), cx)),
            });

        let head = div()
            .flex()
            .items_start()
            .gap(theme::S3)
            .child(div().flex_none().mt(px(2.)).text_color(p.text_2).child(icon(IconName::UsbStick).size(px(24.))))
            .child(div().flex().flex_col().flex_1().min_w_0().gap(px(2.)).child(title).child(ids))
            .when_some(use_button, |d, b| d.child(div().flex_none().child(b)));

        // The driver box
        let driver_box = adapter_box(cx)
            .child(
                box_heading(t("ui.adapter.driver"))
                    .child(tag(
                        key_id("driver", id),
                        drv.text.clone(),
                        Some(drv.status),
                        Some(a.driver_service.clone()),
                        cx,
                    ))
                    .when(a.restart_needed, |d| {
                        d.child(tag(
                            key_id("restart-needed", id),
                            t("ui.adapter.restart_needed"),
                            Some(TagStatus::Warn),
                            Some(t("ui.adapter.restart_needed_hint")),
                            cx,
                        ))
                    }),
            )
            .map(|d| {
                let target = a.clone();
                if a.driver == DriverKind::Winusb {
                    d.child(
                        ui::button(key_id("restore", id), t("ui.adapter.restore_driver"), ButtonKind::Default)
                            .icon(icon(IconName::Shield))
                            .on_click(move |_, window, cx| {
                                winusb_dialog(target.clone(), WinusbMode::Restore, |_, _| {}, window, cx)
                            }),
                    )
                    .child(ui::hint(t("ui.adapter.restore_short"), cx))
                } else {
                    // One BlueCaster cannot use keeps its driver (issue #86)
                    d.child(
                        ui::button(key_id("assign", id), t("ui.adapter.assign_winusb"), ButtonKind::Strong)
                            .icon(icon(IconName::Shield))
                            .disabled(!assign_enabled(a, developer))
                            .on_click(move |_, window, cx| {
                                winusb_dialog(target.clone(), WinusbMode::Assign, |_, _| {}, window, cx)
                            }),
                    )
                    .child(ui::hint(assign_hint(a, developer), cx))
                }
            });

        // The firmware box
        let firmware_box = adapter_box(cx)
            .id(key_id("firmware-box", id))
            .when(chip_at == Some(i), |d| d.anchor_scroll(Some(self.firmware_anchor.clone())))
            .child(
                box_heading(t("ui.adapter.firmware"))
                    .child(tag(key_id("firmware", id), fw_text, Some(fw.status), None, cx))
                    // The running stack found it on its ROM code (issue #81)
                    .when(adapter.rom_firmware && a.realtek_chip.is_some() && a.driver == DriverKind::Winusb, |d| {
                        d.child(tag(
                            key_id("rom", id),
                            t("ui.adapter.rom_firmware"),
                            Some(TagStatus::Bad),
                            Some(t("firmware.rom_status")),
                            cx,
                        ))
                    }),
            )
            // The chip setting is one for all adapters: with the one it applies to
            .when(!driver_fw, |d| {
                if vendor_box(a, s) {
                    d.children(vendor_firmware_box(snap, a, cx))
                } else if chip_at == Some(i) {
                    d.child(self.chip_section(snap, Some(a), cx))
                } else {
                    d
                }
            })
            // Issue #86: other firmware (Linux's) for those who want it
            .when(driver_fw, |d| {
                d.child(
                    ui::button(key_id("custom-firmware", id), t("ui.adapter.custom_firmware"), ButtonKind::Default)
                        .icon(icon(IconName::Wrench))
                        .on_click(|_, _, cx| app::save_settings(|s| s.realtek_custom_firmware = true, cx)),
                )
            })
            .when(custom_firmware(a, s), |d| {
                d.child(
                    ui::button(key_id("driver-firmware", id), t("ui.adapter.driver_firmware"), ButtonKind::Default)
                        .icon(icon(IconName::ArrowReset))
                        .on_click(|_, _, cx| app::save_settings(|s| s.realtek_custom_firmware = false, cx)),
                )
            })
            .when(!adapter_supported(a), |d| {
                d.child(
                    ui::button(
                        key_id("download-unavailable", id),
                        t("ui.adapter.firmware_download_unavailable"),
                        ButtonKind::Default,
                    )
                    .icon(icon(IconName::ArrowDownload))
                    .disabled(true),
                )
                .child(ui::hint(t("firmware.vendor_unsupported"), cx))
            })
            .when(developer && a.driver == DriverKind::Winusb, |d| {
                let target = a.clone();
                let probe = ui::button(key_id("probe", id), t("ui.adapter.probe"), ButtonKind::Default)
                    .icon(icon(IconName::Search))
                    .disabled(self.probing)
                    .on_click(cx.listener(move |this, _, _, cx| this.probe(&target, cx)));
                let probed = self.probed.get(id).cloned().map(|r| {
                    let (target, version) = (a.clone(), snap.update.current_version.clone());
                    let summary = r.summary.clone();
                    let report =
                        ui::button(key_id("probe-report", id), t("ui.adapter.probe_report"), ButtonKind::Default)
                            .icon(icon(IconName::Open))
                            .on_click(move |_, _, cx| Adapter::report_chip(&target, &r, version.clone(), cx));
                    (summary, report)
                });
                d.child(probe).when_some(probed, |d, (summary, report)| d.child(ui::hint(summary, cx)).child(report))
            });

        ui::card(cx)
            // The adapter BlueCaster has open
            .when(active, |d| d.border_color(p.accent_fill))
            .child(head)
            .child(div().flex().flex_wrap().gap(theme::S3).child(driver_box).child(firmware_box))
    }

    /// v1 FirmwareDialog: the chip choice and the firmware state of that chip.
    /// In the adapter's firmware box (`adapter`), which has the heading; on
    /// its own section without
    fn chip_section(&self, snap: &Snapshot, adapter: Option<&AdapterInfo>, cx: &mut Context<Self>) -> AnyElement {
        let s = &snap.settings;
        let adapters = &snap.adapter.adapters;
        let choice = build_chip_options(
            &chip_setting(s),
            &self.files,
            &ChipLabels { auto: &chip_auto_label(adapters), non_realtek: &t("firmware.chip_non_realtek") },
        );
        let choices = choice.options.iter().map(|o| Choice::new(o.key.clone(), o.label.clone())).collect();
        let this = cx.weak_entity();
        let select = ui::select("bt-chip", &choice.selected, choices, false, move |key: &String, _, cx| {
            let key = key.clone();
            let _ = this.update(cx, |this, cx| this.choose_chip(&key, cx));
        });
        let firmware: Vec<AdapterFirmware> = adapters.iter().map(AdapterFirmware::from).collect();
        let status = chip_status(&chip_setting(s), &firmware, &self.files);
        let body = div()
            .flex()
            .flex_col()
            .gap(theme::S3)
            .w_full()
            .child(ui::field_stacked(t("settings.bt_chip"), chip_hint(s, adapters).map(Into::into), None, select, cx))
            .children(firmware_status(snap, &status, adapter.is_some(), cx))
            .child(
                ui::inline()
                    // Also to fetch the files again when they are there
                    .when_some(adapter.filter(|a| a.firmware_download), |d, a| {
                        d.child(firmware_download(
                            key_id("firmware-download", &a.instance_id),
                            a.firmware_files.clone(),
                            if a.firmware_present == Some(true) { ButtonKind::Default } else { ButtonKind::Strong },
                        ))
                    })
                    .children(firmware_folder_button("chip-firmware-folder", ButtonKind::Subtle, cx))
                    .child(
                        ui::button("setup-guide", t("ui.setup.guide"), ButtonKind::Subtle)
                            .icon(icon(IconName::Open))
                            .on_click(|_, _, cx| host::open_url(LINKS.setup_guide, cx)),
                    ),
            );
        match adapter {
            Some(_) => body.into_any_element(),
            None => ui::section(t("firmware.title"), None, cx)
                .id("firmware-section")
                .anchor_scroll(Some(self.firmware_anchor.clone()))
                .child(body)
                .into_any_element(),
        }
    }

    /// LE Audio for the whole stack, as Windows has it (issue #86), and
    /// hearing aids, which need Bluetooth LE but not LE Audio (discussion
    /// #584); both in developer mode
    fn le_audio_toggle(snap: &Snapshot, cx: &App) -> Option<Div> {
        let mut rows = Vec::new();
        if le_audio_toggle_shown(&snap.adapter, snap.settings.developer_mode) {
            rows.push(ui::toggle(
                "le-audio",
                t("ui.adapter.le_audio"),
                None,
                snap.settings.le_audio_experimental,
                false,
                |v, _, cx| {
                    let v = *v;
                    app::save_settings(move |s| s.le_audio_experimental = v, cx);
                },
                cx,
            ));
        }
        if snap.settings.developer_mode {
            rows.push(ui::toggle(
                "hearing-aids",
                t("ui.adapter.hearing_aids"),
                Some(t("ui.adapter.hearing_aids_hint").into()),
                snap.settings.hearing_aids,
                false,
                |v, _, cx| {
                    let v = *v;
                    app::save_settings(move |s| s.hearing_aids = v, cx);
                },
                cx,
            ));
        }
        let mut rows = rows.into_iter();
        let first = rows.next()?;
        Some(ui::card(cx).child(ui::first_row(first)).children(rows))
    }

    fn afh_section(
        &mut self,
        snap: &Snapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let afh = snap.settings.afh.as_str();
        let developer = snap.settings.developer_mode;
        // "blend-mild" / "-medium" / "-strong": experimental, developer mode
        // only; without it the stack runs Auto, and so it shows
        let strictness = afh_strictness(afh).unwrap_or("medium").to_owned();
        let mode = afh_mode(afh, developer);
        let manual = self.afh_manual;
        let bad = afh_channels_bad(manual, &self.afh_channels);
        // Auto: the Wi-Fi channels the running stack avoids now (empty while none
        // or no stack runs)
        let avoided = afh_avoided(self.core.read(cx).store.link().back().map(|l| l.afh_avoided.as_str()));
        // One box for the channels (issue #86): Auto shows what it chose, Off
        // nothing, both read only; Manual takes the user's
        let shown = afh_shown(manual, &self.afh_channels, mode, &avoided);
        let placeholder_changed = self.afh_placeholder != Some(manual);
        self.afh_placeholder = Some(manual);
        self.afh_input.update(cx, |input, cx| {
            // What the user is typing is never put back while the box has the
            // keyboard: it shows the box's own text again once it is left
            let editing = manual && input.focus_handle(cx).is_focused(window);
            if !editing && input.value().as_ref() != shown {
                input.set_value(shown.clone(), window, cx);
            }
            if placeholder_changed {
                input.set_placeholder(if manual { "6,11" } else { "" }, window, cx);
            }
        });

        let mut modes = vec![
            Choice::new(AfhMode::Auto, t("receiver.afh_auto")),
            Choice::new(AfhMode::Off, t("receiver.afh_off")),
            Choice::new(AfhMode::Manual, t("receiver.afh_manual")),
        ];
        if developer {
            modes.push(Choice::new(AfhMode::Blend, t("receiver.afh_blend")));
        }
        let this = cx.weak_entity();
        let chosen_mode = if manual { AfhMode::Manual } else { mode };
        let mode_choice = ui::segmented("afh-mode", &chosen_mode, modes, move |v: &AfhMode, _, cx| {
            let v = *v;
            let _ = this.update(cx, |this, cx| this.set_afh_mode(v, cx));
        });
        let p = *cx.palette();
        let channels = div()
            .w(px(96.))
            .font_family(theme::FONT_MONO)
            .when(!manual, |d| d.text_color(p.text_2))
            .child(Input::new(&self.afh_input).readonly(!manual).aria_label(t("receiver.afh_channels")));

        ui::section(t("receiver.afh"), None, cx).id("afh-section").anchor_scroll(Some(self.afh_anchor.clone())).child(
            // The text in a block of its own, the controls beside it (issue #86)
            div()
                .flex()
                .flex_wrap()
                .items_start()
                .justify_between()
                .gap_x(theme::S4)
                .gap_y(theme::S3)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .flex_basis(px(256.))
                        .min_w_0()
                        .gap(theme::S2)
                        .child(ui::hint(t("receiver.afh_tooltip"), cx))
                        .when(bad, |d| d.child(field_error(t("ui.adapter.afh_bad"), cx))),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_end()
                        .gap(theme::S2)
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .justify_end()
                                .gap(theme::S2)
                                .child(mode_choice)
                                .child(channels),
                        )
                        // The strength on a row of its own, below the mode (issue #86)
                        .when(!manual && mode == AfhMode::Blend, |d| {
                            d.child(ui::segmented(
                                "afh-strictness",
                                &strictness,
                                vec![
                                    Choice::new("mild".to_owned(), t("receiver.afh_mild")),
                                    Choice::new("medium".to_owned(), t("receiver.afh_medium")),
                                    Choice::new("strong".to_owned(), t("receiver.afh_strong")),
                                ],
                                |v: &String, _, cx| {
                                    let afh = format!("blend-{v}");
                                    app::save_settings(move |s| s.afh = afh, cx);
                                },
                            ))
                        }),
                ),
        )
    }
}

/// "Auto (RTL8761BU)": the chip choice's first entry
fn chip_auto_label(adapters: &[AdapterInfo]) -> String {
    let none = t("firmware.chip_auto_none");
    t!("firmware.chip_auto", detected_chip(adapters).unwrap_or(&none))
}

/// A box of an adapter card (`.adapter-box`): driver or firmware
fn adapter_box(cx: &App) -> Div {
    div()
        .flex()
        .flex_col()
        .items_start()
        .gap(theme::S3)
        .p(theme::S3)
        .flex_1()
        .flex_basis(px(256.))
        .min_w_0()
        .border_1()
        .border_color(cx.palette().border)
        .rounded(theme::RADIUS)
}

/// "Driver: " before its tags (`.adapter-box > h3`)
fn box_heading(label: String) -> Div {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(theme::S1)
        .text_size(theme::FS_BODY)
        .font_weight(FontWeight::SEMIBOLD)
        .child(ui_model::i18n::with_colon(&label))
}

/// Another vendor's adapter (issue #86): what to do about its firmware, and
/// only where there is something to do. Its vendor's driver or none needed:
/// nothing, unless loading failed; Intel's files (and Broadcom's optional
/// ones) in the firmware folder: where to get them; Marvell: Wi-Fi on
fn vendor_firmware_box(snap: &Snapshot, a: &AdapterInfo, cx: &App) -> Vec<AnyElement> {
    let firmware = AdapterFirmware::from(a);
    let source = vendor_firmware(&firmware);
    let outcome = loader_outcome(&firmware, snap.adapter.loader.as_ref());
    let url = vendor_firmware_url(&a.vendor);
    let actions = |guide: bool| -> AnyElement {
        ui::inline()
            .when_some(url, |d, u| {
                d.child(
                    ui::button(
                        key_id("vendor-download", &a.instance_id),
                        t("firmware.open_download_page"),
                        ButtonKind::Default,
                    )
                    .icon(icon(IconName::Open))
                    .on_click(move |_, _, cx| host::open_url(u, cx)),
                )
            })
            .children(firmware_folder_button(key_id("vendor-folder", &a.instance_id), ButtonKind::Subtle, cx))
            .when(guide, |d| {
                d.child(
                    ui::button(key_id("vendor-guide", &a.instance_id), t("ui.setup.guide"), ButtonKind::Subtle)
                        .icon(icon(IconName::Open))
                        .on_click(|_, _, cx| host::open_url(LINKS.setup_guide, cx)),
                )
            })
            .into_any_element()
    };
    let hint = |text: String| ui::hint(text, cx).into_any_element();
    if outcome == Some(LoaderOutcome::Failed) {
        let text = if source == Some(VendorFirmware::Driver) {
            t!("firmware.from_vendor_driver", &a.vendor)
        } else {
            t("firmware.intel_files")
        };
        vec![hint(text), actions(source == Some(VendorFirmware::Folder))]
    } else if source == Some(VendorFirmware::Folder) && outcome != Some(LoaderOutcome::Ok) {
        vec![hint(t("firmware.intel_files")), actions(true)]
    } else if source == Some(VendorFirmware::Optional) {
        vec![actions(false)]
    } else if source == Some(VendorFirmware::Wifi) {
        vec![hint(t("ui.adapter.firmware_keep_wifi"))]
    } else {
        vec![]
    }
}

/// v1 UpdateFirmwareStatus: the files, what is missing, a vendor's note.
/// `embedded`: in the adapter's firmware box, whose tag says the files are
/// there and which has the download button itself
fn firmware_status(snap: &Snapshot, st: &ChipStatus, embedded: bool, cx: &App) -> Option<gpui_kit::Stateful<Div>> {
    let open_page = |id: &'static str, url: &'static str| {
        ui::button(id, t("firmware.open_download_page"), ButtonKind::Default)
            .icon(icon(IconName::Open))
            .on_click(move |_, _, cx| host::open_url(url, cx))
    };
    // The core fetches the files itself; the page stays for doing it by hand
    let realtek_actions = |files: &[String], fetch: bool| -> AnyElement {
        ui::inline()
            .when(fetch && !embedded, |d| {
                d.child(firmware_download("status-firmware-download", files.to_vec(), ButtonKind::Strong))
            })
            .child(open_page("firmware-page", LINKS.firmware_realtek))
            .into_any_element()
    };
    let banner = |kind: BannerKind, title: Option<String>, body: Option<AnyElement>, actions: Option<AnyElement>| {
        ui::banner(kind, title.map(Into::into), body, actions, None, cx)
    };
    let text = |s: String| div().child(ui::text(s)).into_any_element();
    let present = |files: &[String]| {
        (!embedded).then(|| {
            banner(BannerKind::Good, Some(t("firmware.present")), Some(file_list(files).into_any_element()), None)
        })
    };
    match st {
        ChipStatus::Present { files } => present(files),
        ChipStatus::Unchecked { files, present: Some(true) } => present(files),
        ChipStatus::Missing { files, .. } | ChipStatus::Unchecked { files, .. } => {
            let missing = matches!(st, ChipStatus::Missing { .. } | ChipStatus::Unchecked { present: Some(false), .. });
            let fetch = matches!(st, ChipStatus::Missing { download: true, .. });
            let body = div()
                .flex()
                .flex_col()
                .gap(theme::S2)
                .child(t("firmware.required_files"))
                .child(file_list(files))
                .child(ui::hint(t("firmware.manual_instructions"), cx));
            Some(banner(
                if missing { BannerKind::Warn } else { BannerKind::Info },
                missing.then(|| t("firmware.missing_manual")),
                Some(body.into_any_element()),
                Some(realtek_actions(files, fetch)),
            ))
        }
        ChipStatus::Invalid { files, file, download } => {
            let body = div()
                .flex()
                .flex_col()
                .gap(theme::S2)
                .child(t!("firmware.invalid", file))
                .child(t("firmware.required_files"))
                .child(file_list(files));
            Some(banner(
                BannerKind::Warn,
                Some(t("firmware.invalid_short")),
                Some(body.into_any_element()),
                Some(realtek_actions(files, *download)),
            ))
        }
        ChipStatus::Vendor { note, vendor } => {
            // Per chip: a vendor's chips without a loader get no firmware page,
            // even when its other chips have one
            let unsupported = note.key == "firmware.vendor_unsupported";
            let url = if unsupported { None } else { vendor_firmware_url(vendor) };
            Some(banner(
                if unsupported { BannerKind::Warn } else { BannerKind::Info },
                None,
                Some(text(tm(Some(note)))),
                url.map(|u| ui::inline().child(open_page("vendor-page", u)).into_any_element()),
            ))
        }
        ChipStatus::NonRealtek => Some(banner(BannerKind::Good, None, Some(text(t("firmware.non_realtek"))), None)),
        // Not "bind it with Zadig" while the adapter seems hung (AdapterHung says what to do)
        ChipStatus::AutoNoAdapter => (!snap.adapter.hung)
            .then(|| banner(BannerKind::Warn, None, Some(text(t("firmware.auto_no_adapter"))), None)),
        ChipStatus::AutoUnrecognized => {
            Some(banner(BannerKind::Warn, None, Some(text(t("firmware.auto_unrecognized"))), None))
        }
        ChipStatus::SelectChip => Some(banner(BannerKind::Warn, None, Some(text(t("firmware.select_chip"))), None)),
    }
}

impl Render for Adapter {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(snap) = self.core.read(cx).snap().cloned() else { return div().into_any_element() };
        let adapter = &snap.adapter;
        let developer = snap.settings.developer_mode;
        let chosen = snap.settings.adapter_id.clone();
        // The order shown; the chip section goes with its adapter in it
        let sorted = usable_first(&adapter.adapters);
        // The card the chip choice goes in; none for an adapter BlueCaster cannot
        // use, the choice then stands on its own below the cards
        let chip_at = chip_card(&sorted);

        let mut page = div()
            .id("adapter")
            .size_full()
            .flex()
            .flex_col()
            .gap(theme::S4)
            .p(theme::S6)
            .pb(theme::S8)
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(ui::reveal_presses(&self.scroll))
            .child(Adapter::head())
            .child(Adapter::stack_line(&snap, cx))
            .when(chosen_missing(&adapter.adapters, &chosen), |d| {
                d.child(ui::banner(
                    BannerKind::Warn,
                    Some(t("ui.adapter.use_missing").into()),
                    None,
                    Some(
                        ui::inline()
                            .child(
                                ui::button("use-auto", t("ui.adapter.use_auto"), ButtonKind::Default)
                                    .icon(icon(IconName::ArrowReset))
                                    .on_click(|_, _, cx| Adapter::choose_adapter(String::new(), cx)),
                            )
                            .into_any_element(),
                    ),
                    None,
                    cx,
                ))
            })
            // A firmware loader while the adapter starts (Intel's download), and
            // how it ended: "ready" and its steps in developer mode only
            .when_some(loader_line(adapter.loader.as_ref(), developer), |d, l| {
                let p = *cx.palette();
                // One text that wraps as a whole, the step after the words
                d.child(
                    status_line(l.tone, cx).child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_wrap()
                            .gap_x(theme::S2)
                            .child(div().min_w_0().max_w_full().child(l.text))
                            .when_some(l.step, |d, step| {
                                d.child(
                                    div()
                                        .min_w_0()
                                        .max_w_full()
                                        .font_family(theme::FONT_MONO)
                                        .text_color(p.text_2)
                                        .child(format!("({step})")),
                                )
                            }),
                    ),
                )
            })
            .children(adapter_hung(cx))
            .when_some(
                adapter
                    .error
                    .as_ref()
                    .filter(|m| adapter.status == ipc::AdapterStatus::Error && !hidden_by_hung(&snap, Some(m))),
                |d, m| d.child(error_panel("adapter-error", None, m, None, cx)),
            );

        if adapter.adapters.is_empty() {
            if !adapter.hung {
                page = page.child(ui::banner(
                    BannerKind::Warn,
                    Some(t("firmware.no_adapters").into()),
                    Some(div().child(t("ui.adapter.none_hint")).into_any_element()),
                    None,
                    None,
                    cx,
                ));
            }
        } else {
            for (i, a) in sorted.iter().enumerate() {
                page = page.child(self.adapter_card(&snap, a, i, chip_at, cx));
            }
        }
        // No adapter to put it with: on its own, to choose the chip beforehand
        if chip_at.is_none() {
            page = page.child(self.chip_section(&snap, None, cx));
        }
        let page = page.children(Adapter::le_audio_toggle(&snap, cx)).child(self.afh_section(&snap, window, cx));
        // The section of the route in view, once the page has been laid out
        if let Some(section) = self.scroll_to {
            if self.drawn {
                self.scroll_to = None;
                match section {
                    AdapterSection::Afh => self.afh_anchor.scroll_to(window, cx),
                    AdapterSection::Firmware => self.firmware_anchor.scroll_to(window, cx),
                }
            } else {
                cx.on_next_frame(window, |_, _, cx| cx.notify());
            }
        }
        self.drawn = true;

        div().relative().size_full().child(page).vertical_scrollbar(&self.scroll).into_any_element()
    }
}

#[cfg(test)]
mod interaction_tests;
