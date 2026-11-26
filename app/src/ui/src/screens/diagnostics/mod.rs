//! Diagnostics: live graphs of the stream, the radio and the audio capture
//! (an LE Audio stream's ISO counters on the stream tab), the headset
//! microphone test, the eleven-step connection flow, the log with level,
//! text and attempt filters, HCI capture, "copy diagnostic info", verbose
//! logging, the diagnostic bundle (one .zip of logs, captures and crash
//! reports for a report). The receiver test is on the Developer screen. Each
//! tab is its own file here, and its own view: made when the tab is chosen
//! and dropped when another is, so what it asks the core for ends with it.

pub mod capture_panel;
pub mod channel_map;
pub mod flow_panel;
pub mod log_panel;
pub mod mic_panel;
pub mod radio_panel;
pub mod shared;
pub mod stream_panel;

use std::rc::Rc;

use gpui_kit::component::Disableable as _;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AnyView, App, AppContext as _, AsyncApp, Context, Entity, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Task, WeakEntity, Window,
    div, px,
};
use ipc::{CaptureState, DiagBundle, DiagBundleParams, DiagInfo, DiagInfoParams, Method};
use ui_model::i18n::t;
use ui_model::links::{BugReport, base_name};
use ui_model::linkstats::format_bytes;
use ui_model::t;
use ui_model::view::diagnostics::{
    GITHUB_ATTACHMENT_LIMIT, VerboseChoice, verbose_choice, verbose_help, verbose_level,
};
use ui_model::view::stream::bug_report_for;

use self::capture_panel::CapturePanel;
use self::flow_panel::{FlowPanel, OnJump};
use self::log_panel::{JUMP_WAIT, LogPanel, LogView};
use self::mic_panel::MicPanel;
use self::radio_panel::RadioPanel;
use self::shared::{DiagShared, observe_core_time, shared, update_shared};
use self::stream_panel::StreamPanel;
use crate::app::{self, NoticeKind};
use crate::core::{self, Core, CoreEvent, WantKind};
use crate::host;
use crate::theme::{self, ActivePalette as _};
use crate::ui::dialog::{self, Question};
use crate::ui::{self, BannerKind, ButtonKind, Choice, IconName, icon};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Stream,
    Radio,
    Capture,
    Mic,
    Flow,
    Log,
}

pub struct Diagnostics {
    core: Entity<Core>,
    tab: Tab,
    /// The chosen tab's view
    panel: AnyView,
    cap_busy: bool,
    bundle_busy: bool,
    reporting: bool,
    log_view: Entity<LogView>,
    connected: bool,
    jump_wait: Option<Task<()>>,
}

impl Diagnostics {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Diagnostics {
        if !cx.has_global::<DiagShared>() {
            cx.set_global(DiagShared::default());
        }
        let core = app::core(cx);
        cx.observe_global::<DiagShared>(|_, cx| cx.notify()).detach();
        cx.observe(&core, |this, core, cx| {
            let c = core.read(cx);
            // The adapters' own addresses: <adapter> in masked copies
            if let Some(s) = c.snap() {
                ui_model::format::know_adapters(s.adapter.adapters.iter().map(|a| a.address.as_deref()));
            }
            let connected = c.connected();
            if this.connected && !connected {
                update_shared(cx, |s| s.epoch = None);
            }
            this.connected = connected;
            cx.notify();
        })
        .detach();
        // Live samples while the screen is open: the clock for the time of
        // day, and the link history; info lines so a step's line is there to jump to
        core::want(WantKind::Link, cx);
        core::want(WantKind::Logs(ipc::LogLevel::Info), cx);
        cx.subscribe(&core, |_, core, ev: &CoreEvent, cx| {
            if *ev == CoreEvent::Link
                && let Some(last) = core.read(cx).store.link().back()
            {
                let time = last.time_ms;
                observe_core_time(time, cx);
            }
        })
        .detach();
        if let Some(s) = core.read(cx).snap() {
            ui_model::format::know_adapters(s.adapter.adapters.iter().map(|a| a.address.as_deref()));
        }
        let connected = core.read(cx).connected();
        let log_view = cx.new(|_| LogView::default());
        let panel = cx.new(|cx| StreamPanel::new(window, cx)).into();
        #[cfg_attr(not(debug_assertions), allow(unused_mut))]
        let mut this = Diagnostics {
            core,
            tab: Tab::Stream,
            panel,
            cap_busy: false,
            bundle_busy: false,
            reporting: false,
            log_view,
            connected,
            jump_wait: None,
        };
        // For looking at one screen: open another tab first
        if let Some(name) = crate::demo::diag_tab() {
            let tab = match name.as_str() {
                "radio" => Tab::Radio,
                "capture" => Tab::Capture,
                "mic" => Tab::Mic,
                "flow" => Tab::Flow,
                "log" => Tab::Log,
                _ => Tab::Stream,
            };
            this.select(tab, window, cx);
        }
        this
    }

    /// Opens a tab: its view is made now, the last one's dropped
    fn select(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        if tab == self.tab {
            return;
        }
        self.tab = tab;
        self.panel = match tab {
            Tab::Stream => cx.new(|cx| StreamPanel::new(window, cx)).into(),
            Tab::Radio => cx.new(|cx| RadioPanel::new(window, cx)).into(),
            Tab::Capture => cx.new(|cx| CapturePanel::new(window, cx)).into(),
            Tab::Mic => cx.new(|cx| MicPanel::new(window, cx)).into(),
            Tab::Flow => {
                let this = cx.entity().downgrade();
                let on_jump: OnJump = Rc::new(move |seq, window, cx| {
                    let _ = this.update(cx, |this, cx| this.jump_to(seq, window, cx));
                });
                cx.new(|cx| FlowPanel::new(on_jump, window, cx)).into()
            }
            Tab::Log => {
                let view = self.log_view.clone();
                cx.new(|cx| LogPanel::new(view, window, cx)).into()
            }
        };
        cx.notify();
    }

    fn toggle_capture(&mut self, cx: &mut Context<Self>) {
        let running = self.core.read(cx).snap().and_then(|s| s.capture.clone());
        self.cap_busy = true;
        cx.notify();
        if running.is_none() {
            update_shared(cx, |s| s.saved_capture = None);
        }
        let stop = running.is_some();
        let task = if stop {
            app::request::<Option<CaptureState>>(Method::DiagCaptureStop, cx)
        } else {
            let start = app::request::<serde_json::Value>(Method::DiagCaptureStart, cx);
            cx.background_spawn(async move { start.await.map(|_| None) })
        };
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.cap_busy = false;
                match result {
                    Ok(done) if stop => {
                        if let Some(done) = done.or(running) {
                            app::notify(
                                t!("ui.diag.capture_saved", format!("{} ({})", done.path, format_bytes(done.bytes))),
                                NoticeKind::Info,
                                cx,
                            );
                            update_shared(cx, |s| s.saved_capture = Some(done));
                        }
                    }
                    Ok(_) => {}
                    Err(e) => app::notify(t!("debug.capture_failed", app::error_text(&e)), NoticeKind::Error, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn copy_info(cx: &mut App) {
        let task = app::request::<DiagInfo>(Method::DiagInfo(Some(DiagInfoParams { mask: Some(shared(cx).mask) })), cx);
        cx.spawn(async move |cx: &mut AsyncApp| {
            let result = task.await;
            cx.update(|cx| match result {
                Ok(info) => {
                    host::copy_text(info.text, cx);
                    app::notify(t("ui.diag.copied"), NoticeKind::Info, cx);
                }
                Err(e) => app::notify(app::error_text(&e), NoticeKind::Error, cx),
            });
        })
        .detach();
    }

    fn change_verbose(v: VerboseChoice, cx: &mut App) {
        app::send(Method::DiagSetVerbose { level: verbose_level(v) }, cx);
    }

    /// -> whether it was saved (the dialog then closes; it stays after a failure)
    fn save_bundle(&mut self, cx: &mut Context<Self>) -> Task<bool> {
        self.bundle_busy = true;
        cx.notify();
        let s = shared(cx);
        let params = DiagBundleParams { mask: Some(s.mask), hci: Some(s.with_hci) };
        let task = app::request::<DiagBundle>(Method::DiagBundle(Some(params)), cx);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = task.await;
            let saved = result.is_ok();
            let _ = this.update(cx, |this, cx| {
                this.bundle_busy = false;
                match result {
                    Ok(b) => update_shared(cx, |s| s.saved_bundle = Some(b)),
                    Err(e) => app::notify(t!("ui.diag.bundle_failed", app::error_text(&e)), NoticeKind::Error, cx),
                }
                cx.notify();
            });
            saved
        })
    }

    /// The bundle's options first: no bundle until Save in the dialog
    fn ask_bundle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        let busy = dialog::busy_state(cx);
        dialog::ask_run(
            |_, _| Question::new(t("ui.diag.bundle")).confirm(t("ui.diag.bundle_save")),
            busy,
            |_, cx| {
                let s = shared(cx);
                let (hci, mask) = (s.with_hci, s.mask);
                let p = cx.palette();
                let hint = |key: &str| {
                    div().ml(px(28.)).mb(theme::S2).text_size(theme::FS_SMALL).text_color(p.text_2).child(t(key))
                };
                div()
                    .flex()
                    .flex_col()
                    .gap(theme::S3)
                    .child(div().child(t("ui.diag.bundle_dialog_text")))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(theme::S1)
                            .child(ui::check("bundle-hci", t("ui.diag.bundle_hci"), hci, false, |v, window, cx| {
                                let v = *v;
                                update_shared(cx, |s| s.with_hci = v);
                                window.refresh();
                            }))
                            .child(hint("ui.diag.bundle_hci_hint"))
                            .child(ui::check("bundle-mask", t("dbgc.mask"), mask, false, |v, window, cx| {
                                let v = *v;
                                update_shared(cx, |s| s.mask = v);
                                window.refresh();
                            }))
                            .child(hint("ui.diag.bundle_mask_hint")),
                    )
                    .into_any_element()
            },
            move |_, cx| this.update(cx, |this, cx| this.save_bundle(cx)).unwrap_or_else(|_| Task::ready(false)),
            window,
            cx,
        );
    }

    /// A prefilled GitHub issue in the browser, the bundle shown in Explorer
    /// to drag into it, the diagnostic info on the clipboard (a report is
    /// public: addresses masked)
    fn report(&mut self, b: DiagBundle, cx: &mut Context<Self>) {
        self.reporting = true;
        cx.notify();
        let task = app::request::<DiagInfo>(Method::DiagInfo(Some(DiagInfoParams { mask: Some(true) })), cx);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.reporting = false;
                match result {
                    Ok(info) => {
                        let snap = this.core.read(cx).snap().cloned();
                        if let Some(snap) = snap {
                            let extra = BugReport { bundle: Some(base_name(&b.path).to_owned()), ..Default::default() };
                            let url = bug_report_for(&snap, &info.text, extra);
                            host::reveal_path(&b.path, cx);
                            host::open_url(&url, cx);
                            // Only a convenience: the issue has the text already (when it fit)
                            host::copy_text(info.text, cx);
                            app::notify(t("ui.diag.report_opened"), NoticeKind::Info, cx);
                        }
                    }
                    Err(e) => app::notify(app::error_text(&e), NoticeKind::Error, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn jump_to(&mut self, seq: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.log_view.update(cx, |v, cx| {
            v.jump = Some(seq);
            cx.notify();
        });
        self.select(Tab::Log, window, cx);
        let view = self.log_view.clone();
        self.jump_wait = Some(cx.spawn(async move |_, cx: &mut AsyncApp| {
            cx.background_executor().timer(JUMP_WAIT).await;
            cx.update(|cx| {
                if view.read(cx).jump != Some(seq) {
                    return;
                }
                view.update(cx, |v, cx| {
                    v.jump = None;
                    cx.notify();
                });
                app::notify(t("ui.diag.line_gone"), NoticeKind::Error, cx);
            });
        }));
    }

    fn head(&self, recording: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let p = *cx.palette();
        let mask = shared(cx).mask;
        let capture = ui::button(
            "capture",
            if recording { t("debug.capture_stop") } else { t("debug.capture_start") },
            ButtonKind::Default,
        )
        .icon(icon(if recording { IconName::Stop } else { IconName::Record }))
        .loading(self.cap_busy)
        .disabled(self.cap_busy)
        .when(recording, |b| b.text_color(p.bad_text).border_color(p.bad_text))
        .on_click(cx.listener(|this, _, _, cx| this.toggle_capture(cx)));
        div()
            .flex()
            // Actions too long for the title's line (German) go below it
            .flex_wrap()
            .items_center()
            .gap(theme::S3)
            .min_h(px(36.))
            .child(
                div()
                    .flex_1()
                    .font_family(theme::FONT_DISPLAY)
                    .text_size(theme::FS_PAGE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(t("ui.nav.diagnostics")),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(theme::S2)
                    .child(div().id("mask-tip").tooltip(|w, cx| Tooltip::new(t("dbgc.mask_tip")).build(w, cx)).child(
                        ui::check("mask", t("dbgc.mask"), mask, false, |v, _, cx| {
                            let v = *v;
                            update_shared(cx, |s| s.mask = v);
                        }),
                    ))
                    .child(capture)
                    .child(
                        ui::button("copy-diag", t("dbgc.copy_diag"), ButtonKind::Default)
                            .icon(icon(IconName::Copy))
                            .on_click(|_, _, cx| Self::copy_info(cx)),
                    ),
            )
    }

    fn report_card(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let verbose = self.core.read(cx).snap().and_then(|s| s.verbose);
        let choice = verbose_choice(verbose.as_ref());
        let saved = shared(cx).saved_bundle.clone();
        let select = div().flex_none().flex().justify_end().child(ui::select_labelled(
            "verbose",
            t("ui.diag.verbose"),
            &choice,
            vec![
                Choice::new(VerboseChoice::Off, t("ui.diag.verbose_off")),
                Choice::new(VerboseChoice::Debug, t("ui.diag.verbose_debug")),
                Choice::new(VerboseChoice::Trace, t("ui.diag.verbose_trace")),
            ],
            false,
            |v: &VerboseChoice, _, cx| Self::change_verbose(*v, cx),
        ));
        let bundle_button = ui::button("bundle", t("ui.diag.bundle_button"), ButtonKind::Default)
            .icon(icon(IconName::ArrowDownload))
            .loading(self.bundle_busy)
            .disabled(self.bundle_busy)
            .on_click(cx.listener(|this, _, window, cx| this.ask_bundle(window, cx)));
        ui::section(t("ui.diag.report"), None, cx)
            .child(ui::field(t("ui.diag.verbose"), Some(verbose_help(verbose.as_ref()).into()), None, select, cx))
            .child(ui::field(t("ui.diag.bundle_title"), Some(t("ui.diag.bundle_tip").into()), None, bundle_button, cx))
            .when_some(saved, |d, b| d.child(self.bundle_saved(b, cx)))
    }

    fn bundle_saved(&self, b: DiagBundle, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let p = *cx.palette();
        let too_big = b.bytes > GITHUB_ATTACHMENT_LIMIT;
        let path = b.path.clone();
        let body: AnyElement = div()
            .flex()
            .flex_col()
            .gap(theme::S1)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .child(ui::mono(base_name(&b.path).to_owned()))
                    .child(div().text_color(p.text_2).child(format!(" \u{b7} {}", format_bytes(b.bytes)))),
            )
            .child(div().child(if too_big { t("ui.diag.bundle_too_big") } else { t("ui.diag.bundle_attach") }))
            .into_any_element();
        let actions: AnyElement = ui::inline()
            .child(
                ui::button("report-github", t("ui.diag.report_github"), ButtonKind::Strong)
                    .icon(icon(IconName::Open))
                    .loading(self.reporting)
                    .disabled(self.reporting)
                    .on_click(cx.listener(move |this, _, _, cx| this.report(b.clone(), cx))),
            )
            .child(
                ui::button("bundle-reveal", t("ui.diag.show_in_explorer"), ButtonKind::Subtle)
                    .icon(icon(IconName::FolderOpen))
                    .on_click(move |_, _, cx| host::reveal_path(&path, cx)),
            )
            .child(
                ui::button("bundle-close", t("modal.close"), ButtonKind::Subtle)
                    .icon(icon(IconName::Dismiss))
                    .on_click(|_, _, cx| update_shared(cx, |s| s.saved_bundle = None)),
            )
            .into_any_element();
        div().mt(theme::S2).child(ui::banner(
            if too_big { BannerKind::Warn } else { BannerKind::Good },
            Some(t("ui.diag.bundle_saved_title").into()),
            Some(body),
            Some(actions),
            None,
            cx,
        ))
    }

    /// The running capture (its size), or the file saved last
    fn capture_line(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = *cx.palette();
        let snap = self.core.read(cx).snap()?;
        let line = || div().flex().items_center().gap(theme::S2).text_size(theme::FS_SMALL).text_color(p.text_2);
        let path = |text: String| div().min_w_0().truncate().font_family(theme::FONT_MONO).child(text);
        if let Some(c) = &snap.capture {
            return Some(
                line()
                    .child(div().size(px(8.)).flex_none().rounded_full().bg(p.bad))
                    .child(t("ui.diag.capturing"))
                    .child(path(c.path.clone()))
                    .child(div().flex_none().text_color(p.text_3).child(format_bytes(c.bytes)))
                    .into_any_element(),
            );
        }
        let saved = shared(cx).saved_capture.clone()?;
        let service = snap.service_mode;
        let reveal = saved.path.clone();
        Some(
            line()
                .flex_wrap()
                .child(div().text_color(p.good_text).child(icon(IconName::CheckmarkCircle)))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(theme::S1)
                        .min_w_0()
                        .child(t!("ui.diag.capture_saved", ""))
                        .child(path(saved.path.clone()))
                        .child(
                            div().flex_none().text_color(p.text_3).child(format!("({})", format_bytes(saved.bytes))),
                        ),
                )
                // A core that runs as a service keeps the capture in its
                // protected folder: the user gets it in a diagnostic bundle
                .when(!service, |d| {
                    d.child(
                        ui::button("capture-reveal", t("ui.diag.show_in_explorer"), ButtonKind::Subtle)
                            .icon(icon(IconName::FolderOpen))
                            .on_click(move |_, _, cx| host::reveal_path(&reveal, cx)),
                    )
                })
                .child(
                    ui::icon_button("capture-close", IconName::Dismiss, t("modal.close"))
                        .on_click(|_, _, cx| update_shared(cx, |s| s.saved_capture = None)),
                )
                .into_any_element(),
        )
    }
}

impl Render for Diagnostics {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(recording) = self.core.read(cx).snap().map(|s| s.capture.is_some()) else {
            return div().into_any_element();
        };
        let this = cx.entity().downgrade();
        let tabs = vec![
            Choice::new(Tab::Stream, t("quality.section_stream")).icon(IconName::Pulse),
            Choice::new(Tab::Radio, t("ui.diag.tab_radio")).icon(IconName::Bluetooth),
            Choice::new(Tab::Capture, t("ui.diag.tab_capture")).icon(IconName::Speaker2),
            Choice::new(Tab::Mic, t("ui.diag.tab_mic")).icon(IconName::Mic),
            Choice::new(Tab::Flow, t("dbgc.section_flow")).icon(IconName::CheckmarkCircle),
            Choice::new(Tab::Log, t("dbgc.section_log")).icon(IconName::PeopleList),
        ];
        let head = self.head(recording, cx);
        let report = self.report_card(cx);
        let capture = self.capture_line(cx);
        div()
            .id("diagnostics")
            .size_full()
            .flex()
            .flex_col()
            .gap(theme::S4)
            .p(theme::S6)
            .overflow_y_scrollbar()
            .child(head)
            .child(report)
            .children(capture)
            .child(ui::tab_bar("diag-tabs", &self.tab, tabs, move |tab: &Tab, window, cx| {
                let tab = *tab;
                let _ = this.update(cx, |this, cx| this.select(tab, window, cx));
            }))
            .child(self.panel.clone())
            .into_any_element()
    }
}
