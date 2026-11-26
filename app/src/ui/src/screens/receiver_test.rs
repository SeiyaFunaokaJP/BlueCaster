//! The peer receiver test of v1 (Peer Receiver Test, wx_receiver_dialog.cpp on the develop branch):
//! finds tools/linux_sink on another PC, streams to its Bluetooth adapter
//! without a saved profile (stream.start_custom), checks each step OK / NG
//! and sets what this PC sent beside what the receiver counted. The logic is
//! in ui_model::receiver; this is the form, the table and the log.
//!
//! The measurement (steps, log, Reset, the choices) belongs to the core
//! connection, not to this view: switching tabs keeps it.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, AsyncApp, Context, Div, Entity, EntityId, FontWeight, Global, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, WeakEntity, Window, div, px,
};
use ipc::{Method, ReceiverConnection, ReceiverInfo, ReceiverState};
use ui_model::codecs::{CODECS, bit_depths, codec_spec, fixed_bit_depth, sample_rates};
use ui_model::format::khz;
use ui_model::i18n::{t, tm};
use ui_model::receiver::{
    BUFFERS_MS, Baseline, CompareInputs, Inputs, LOG_KEEP, MTUS, ROW_IDS, STEP_IDS, StartOptions, StepState, TestModel,
    can_start, compare, configure_failed, new_model, no_baseline, row_key, start_failed, start_measurement,
    status_text, step_key, stop_measurement, take_baseline, update,
};
use ui_model::t;
use ui_model::view::receiver_test::{
    Choices, Source, connect_target, found_target, not_offered, offered, step_status, step_time, time_text, with_codec,
};

use crate::app::{self, AdapterSection, NoticeKind, Route, RouteName};
use crate::core::{self, Core, CoreEvent, WantKind};
use crate::host;
use crate::theme::{self, ActivePalette as _};
use crate::ui::{self, ButtonKind, Choice, IconName, icon};

/// What outlives the view: one per core connection
pub struct Session {
    model: TestModel,
    log: Vec<(i64, String)>,
    base: Baseline,
    choices: Choices,
    /// The address field; None until the view first opens
    address: Option<String>,
    auto_connected: bool,
}

impl Session {
    fn new() -> Session {
        Session {
            model: new_model(),
            log: Vec::new(),
            base: no_baseline(),
            choices: Choices::default(),
            address: None,
            auto_connected: false,
        }
    }

    fn add_log(&mut self, lines: Vec<String>) {
        if lines.is_empty() {
            return;
        }
        let time = now_ms() as i64;
        self.log.extend(lines.into_iter().map(|text| (time, text)));
        if self.log.len() > LOG_KEEP {
            self.log.drain(..self.log.len() - LOG_KEEP);
        }
    }
}

struct Sessions(Option<(EntityId, Entity<Session>)>);

impl Global for Sessions {}

fn session_for(core: &Entity<Core>, cx: &mut App) -> Entity<Session> {
    if let Some(Sessions(Some((id, s)))) = cx.try_global::<Sessions>()
        && *id == core.entity_id()
    {
        return s.clone();
    }
    let s = cx.new(|_| Session::new());
    cx.set_global(Sessions(Some((core.entity_id(), s.clone()))));
    s
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn rate_label(r: u32) -> String {
    if r == 0 { t("codec.auto") } else { format!("{} kHz", khz(r)) }
}

fn depth_label(d: u32) -> String {
    if d == 0 { t("codec.auto") } else { t(&format!("bitdepth.{d}")) }
}

fn mtu_label(v: u32) -> String {
    if v == 0 { t("receiver.as_started") } else { v.to_string() }
}

fn buffer_label(v: u32) -> String {
    if v == 0 { t("receiver.as_started") } else { format!("{v} ms") }
}

fn step_icon(s: StepState) -> Option<IconName> {
    match s {
        StepState::Pending => None,
        StepState::Running => Some(IconName::ArrowSync),
        StepState::Ok => Some(IconName::CheckmarkCircle),
        StepState::Failed => Some(IconName::ErrorCircle),
    }
}

/// The quality choices; a codec without any: its one value, "-"
fn quality_options(c: &Choices) -> Vec<Choice<String>> {
    let q = codec_spec(&c.codec).qualities;
    if q.is_empty() {
        vec![Choice::new(c.quality.clone(), "-")]
    } else {
        q.iter().map(|x| Choice::new(x.id.to_owned(), t(x.label))).collect()
    }
}

pub struct ReceiverTest {
    core: Entity<Core>,
    session: Entity<Session>,
    address: Entity<InputState>,
    searching: bool,
    found: Option<Vec<ReceiverInfo>>,
    starting: bool,
    log_scroll: ScrollHandle,
    _tick: Task<()>,
    _subs: Vec<Subscription>,
}

impl ReceiverTest {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> ReceiverTest {
        let core = app::core(cx);
        core::want(WantKind::Receiver, cx);
        core::want(WantKind::Link, cx);
        let session = session_for(&core, cx);
        if session.read(cx).address.is_none() {
            let saved = core.read(cx).snap().map(|s| s.settings.remote_sink.clone()).unwrap_or_default();
            let address = if saved == "auto" { String::new() } else { saved };
            session.update(cx, |s, _| s.address = Some(address));
        }
        let text = session.read(cx).address.clone().unwrap_or_default();
        let address =
            cx.new(|cx| InputState::new(window, cx).placeholder(t("receiver.address_hint")).default_value(text));
        let subs = vec![
            cx.subscribe_in(&address, window, |this, input, ev: &InputEvent, _, cx| match ev {
                InputEvent::Change => {
                    let v = input.read(cx).value().to_string();
                    this.session.update(cx, |s, _| s.address = Some(v));
                }
                InputEvent::PressEnter { .. } => this.connect(cx).detach(),
                _ => {}
            }),
            // Refresh on every change of the receiver or the stream, and once
            // a second for the timeouts (v1's timer)
            cx.observe(&core, |this, _, cx| this.refresh(cx)),
            cx.subscribe(&core, |this, _, ev: &CoreEvent, cx| match ev {
                CoreEvent::Receiver => this.refresh(cx),
                CoreEvent::Link => cx.notify(),
                _ => {}
            }),
            cx.observe(&session, |_, _, cx| cx.notify()),
        ];
        let tick = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this.update(cx, |this, cx| this.refresh(cx)).is_err() {
                    break;
                }
            }
        });
        let mut this = ReceiverTest {
            core,
            session,
            address,
            searching: false,
            found: None,
            starting: false,
            log_scroll: ScrollHandle::new(),
            _tick: tick,
            _subs: subs,
        };
        this.refresh(cx);
        this.auto_connect(cx);
        this
    }

    fn receiver(&self, cx: &App) -> Option<ReceiverState> {
        self.core.read(cx).store.receiver.clone()
    }

    fn connection(&self, cx: &App) -> ReceiverConnection {
        self.receiver(cx).map_or(ReceiverConnection::Off, |r| r.connection)
    }

    fn add_log(&mut self, lines: Vec<String>, cx: &mut Context<Self>) {
        if lines.is_empty() {
            return;
        }
        self.session.update(cx, |s, _| s.add_log(lines));
        self.log_scroll.scroll_to_bottom();
        cx.notify();
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let core = self.core.read(cx);
        let Some(snap) = core.snap() else { return };
        let stream = snap.stream.clone();
        let r = core.store.receiver.clone();
        let service_text = tm(Some(&stream.message));
        let lines = self.session.update(cx, |s, _| {
            update(
                &mut s.model,
                &Inputs { receiver: r.as_ref(), stream: &stream, service_text: &service_text, now: now_ms() },
            )
        });
        self.add_log(lines, cx);
        cx.notify();
    }

    // ---- Receiver connection ----

    fn connect(&mut self, cx: &mut Context<Self>) -> Task<()> {
        let address = self.session.read(cx).address.clone().unwrap_or_default();
        let task = app::run(
            app::request::<ReceiverState>(Method::ReceiverConnect { target: connect_target(&address) }, cx),
            cx,
        );
        cx.spawn(async move |_, _: &mut AsyncApp| {
            task.await;
        })
    }

    fn disconnect(&mut self, cx: &mut Context<Self>) -> Task<()> {
        let task = app::run(app::request::<serde_json::Value>(Method::ReceiverDisconnect, cx), cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            task.await;
        })
    }

    fn toggle_connection(&mut self, cx: &mut Context<Self>) {
        if self.connection(cx) == ReceiverConnection::Off {
            self.connect(cx).detach();
        } else {
            self.disconnect(cx).detach();
        }
    }

    /// v1 looked for the receiver as soon as the window opened
    fn auto_connect(&mut self, cx: &mut Context<Self>) {
        if self.session.read(cx).auto_connected {
            return;
        }
        self.session.update(cx, |s, _| s.auto_connected = true);
        let status = app::request::<ReceiverState>(Method::ReceiverStatus, cx);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            if let Ok(r) = status.await
                && r.connection == ReceiverConnection::Off
            {
                let _ = this.update(cx, |this, cx| this.connect(cx).detach());
            }
        })
        .detach();
    }

    fn discover(&mut self, cx: &mut Context<Self>) {
        self.searching = true;
        cx.notify();
        let task = app::request::<Vec<ReceiverInfo>>(Method::ReceiverDiscover, cx);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let r = task.await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    Ok(list) => this.found = Some(list),
                    Err(e) => app::notify(app::error_text(&e), NoticeKind::Error, cx),
                }
                this.searching = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn use_found(&mut self, r: &ReceiverInfo, window: &mut Window, cx: &mut Context<Self>) {
        let target = found_target(r);
        let off = self.connection(cx) == ReceiverConnection::Off;
        let disconnect = (!off).then(|| self.disconnect(cx));
        // The field shows the target chosen (set_value sends no Change)
        self.session.update(cx, |s, _| s.address = Some(target.clone()));
        self.address.update(cx, |i, cx| i.set_value(target, window, cx));
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            if let Some(d) = disconnect {
                d.await;
            }
            let Ok(connect) = this.update(cx, |this, cx| this.connect(cx)) else { return };
            connect.await;
        })
        .detach();
    }

    // ---- Choices ----

    fn set(&mut self, patch: impl FnOnce(&mut Choices), cx: &mut Context<Self>) {
        self.session.update(cx, |s, cx| {
            patch(&mut s.choices);
            cx.notify();
        });
    }

    fn set_codec(&mut self, codec: &str, cx: &mut Context<Self>) {
        let codec = codec.to_owned();
        self.set(|c| *c = with_codec(c, &codec), cx);
    }

    // ---- Start / Stop ----

    fn start_stop(&mut self, cx: &mut Context<Self>) {
        if self.session.read(cx).model.measuring {
            self.add_log(vec![t("receiver.log_stopped")], cx);
            self.session.update(cx, |s, _| stop_measurement(&mut s.model));
            app::send(Method::StreamStop, cx);
            cx.notify();
            return;
        }
        let r = self.receiver(cx);
        let Some(info) = r.as_ref().and_then(|r| r.receiver.clone()).filter(|i| !i.bt_address.is_empty()) else {
            return;
        };
        let c = self.session.read(cx).choices.clone();
        let lines = self.session.update(cx, |s, _| {
            start_measurement(
                &mut s.model,
                &StartOptions { receiver: r.as_ref(), mtu: c.mtu, buffer_ms: c.buffer, now: now_ms() },
            )
        });
        let quality_label = quality_options(&c)
            .into_iter()
            .find(|q| q.value == c.quality)
            .map_or_else(|| "-".to_owned(), |q| q.label.to_string());
        let mut log = vec![t!(
            "receiver.log_start",
            t(codec_spec(&c.codec).label),
            quality_label,
            rate_label(c.rate),
            depth_label(c.depth),
            t(c.source.label_key()),
            &info.bt_address,
            mtu_label(c.mtu),
            buffer_label(c.buffer)
        )];
        log.extend(lines);
        self.add_log(log, cx);
        self.starting = true;
        cx.notify();
        // Receiver side first: it applies the settings to the connection that follows
        let configure = app::request::<serde_json::Value>(
            Method::ReceiverConfigure {
                mtu: (c.mtu != 0).then_some(c.mtu),
                buffer_ms: (c.buffer != 0).then_some(c.buffer),
            },
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            if let Err(e) = configure.await {
                let _ = this.update(cx, |this, cx| {
                    let lines =
                        this.session.update(cx, |s, _| configure_failed(&mut s.model, &app::error_text(&e), now_ms()));
                    this.add_log(lines, cx);
                });
            }
            let Ok(start) = this.update(cx, |_, cx| {
                app::request::<serde_json::Value>(
                    Method::StreamStartCustom {
                        device: info.bt_address.clone(),
                        device_name: (!info.bt_name.is_empty()).then(|| info.bt_name.clone()),
                        codec: c.codec.clone(),
                        quality: c.quality.clone(),
                        sample_rate: (c.rate != 0).then_some(c.rate),
                        bit_depth: (c.depth != 0).then_some(c.depth),
                        source: c.source.as_str().to_owned(),
                        source_id: None,
                        source_name: None,
                        max_media_payload: None,
                        abr: None,
                        lhdcv5_low_latency: None,
                        codec_fallback: None,
                    },
                    cx,
                )
            }) else {
                return;
            };
            let r = start.await;
            let _ = this.update(cx, |this, cx| {
                if let Err(e) = r {
                    let text = app::error_text(&e);
                    let lines = this.session.update(cx, |s, _| start_failed(&mut s.model, &text, now_ms()));
                    this.add_log(lines, cx);
                    app::notify(text, NoticeKind::Error, cx);
                }
                this.starting = false;
                cx.notify();
            });
        })
        .detach();
    }

    // ---- Status, steps, table, log ----

    fn reset(&mut self, cx: &mut Context<Self>) {
        let base = take_baseline(self.receiver(cx).as_ref());
        self.session.update(cx, |s, _| s.base = base);
        self.add_log(vec![t("ui.receiver.log_reset")], cx);
    }

    fn copy_log(&mut self, cx: &mut Context<Self>) {
        let lines: Vec<String> =
            self.session.read(cx).log.iter().map(|(time, text)| format!("{} {text}", time_text(*time))).collect();
        let n = lines.len();
        host::copy_text(lines.join("\n"), cx);
        app::notify(t!("dbgc.copied", n), NoticeKind::Info, cx);
    }

    fn address_section(&self, r: Option<&ReceiverState>, cx: &mut Context<Self>) -> Div {
        let p = *cx.palette();
        let off = r.is_none_or(|r| r.connection == ReceiverConnection::Off);
        let session = self.session.read(cx);
        let status =
            status_text(&session.model, r, &self.core.read(cx).snap().map(|s| s.stream.clone()).unwrap_or_default());
        let search = ui::button("receiver-search", t("ui.receiver.search"), ButtonKind::Subtle)
            .icon(icon(IconName::Search))
            .loading(self.searching)
            .on_click(cx.listener(|this, _, _, cx| this.discover(cx)));
        let toggle = if off {
            ui::button("receiver-connect", t("receiver.connect"), ButtonKind::Strong)
        } else {
            ui::button("receiver-connect", t("receiver.disconnect"), ButtonKind::Default)
                .icon(icon(IconName::PlugDisconnected))
        }
        .on_click(cx.listener(|this, _, _, cx| this.toggle_connection(cx)));
        let found = self.found.as_ref().map(|list| {
            if list.is_empty() {
                ui::muted(t("ui.receiver.none_found"), cx).into_any_element()
            } else {
                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(theme::S1)
                    .children(list.iter().enumerate().map(|(i, r)| {
                        let found = r.clone();
                        ui::button(
                            ("receiver-found", i),
                            t!("receiver.found", &r.host, &r.ip, &r.bt_address, r.mtu),
                            ButtonKind::Subtle,
                        )
                        .icon(icon(IconName::DesktopSpeaker))
                        .on_click(cx.listener(move |this, _, window, cx| this.use_found(&found, window, cx)))
                    }))
                    .into_any_element()
            }
        });
        ui::section(t("receiver.address"), Some(search.into_any_element()), cx)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(theme::S2)
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.address)
                                .aria_label(t("receiver.address"))
                                .font_family(theme::FONT_MONO)
                                .disabled(!off),
                        ),
                    )
                    .child(toggle),
            )
            .when_some(found, |d, f| d.child(f))
            .child(
                // Read out as it changes, as the old UI's status line
                ui::alert("receiver-status", status.lines.join("\n"), false)
                    .flex()
                    .flex_col()
                    .gap(theme::S1)
                    .when(status.bad, |d| d.text_color(p.bad_text))
                    .children(status.lines.into_iter().map(|l| div().child(l))),
            )
    }

    fn sent_section(&self, r: Option<&ReceiverState>, locked: bool, cx: &mut Context<Self>) -> Div {
        let c = self.session.read(cx).choices.clone();
        let offered = offered(r);
        let codecs = CODECS
            .iter()
            .map(|s| {
                let no = not_offered(s.id, &offered);
                let label = if no { t(s.label) + &t("ui.receiver.not_offered") } else { t(s.label) };
                Choice::new(s.id.to_owned(), label).disabled(no)
            })
            .collect();
        let has_qualities = !codec_spec(&c.codec).qualities.is_empty();
        // A codec with one quality or one bit depth: a value, not a list
        let quality: AnyElement = if has_qualities {
            ui::select(
                "receiver-quality",
                &c.quality,
                quality_options(&c),
                locked,
                cx.listener_fn(|this, v: &String, cx| {
                    let v = v.clone();
                    this.set(|c| c.quality = v, cx);
                }),
            )
            .into_any_element()
        } else {
            let label =
                quality_options(&c).into_iter().find(|o| o.value == c.quality).map_or("–".into(), |o| o.label);
            readonly(label, cx).into_any_element()
        };
        let depths = bit_depths(&c.codec);
        let depth: AnyElement = if depths.len() > 1 {
            ui::select(
                "receiver-depth",
                &c.depth,
                depths.into_iter().map(|d| Choice::new(d, depth_label(d))).collect(),
                locked,
                cx.listener_fn(|this, v: &u32, cx| {
                    let v = *v;
                    this.set(|c| c.depth = v, cx);
                }),
            )
            .into_any_element()
        } else {
            readonly(t(&format!("bitdepth.{}", fixed_bit_depth(&c.codec))), cx).into_any_element()
        };
        let sources = [Source::Tone, Source::Loopback]
            .into_iter()
            .map(|s| Choice::new(s, t(s.label_key())).disabled(locked))
            .collect();
        ui::section(t("receiver.settings_sent"), None, cx)
            .child(ui::field(
                t("receiver.codec"),
                None,
                None,
                ui::select(
                    "receiver-codec",
                    &c.codec,
                    codecs,
                    locked,
                    cx.listener_fn(|this, v: &String, cx| {
                        let v = v.clone();
                        this.set_codec(&v, cx);
                    }),
                ),
                cx,
            ))
            .child(ui::field(t("receiver.quality"), None, None, quality, cx))
            .child(ui::field(
                t("receiver.sample_rate"),
                None,
                None,
                ui::select(
                    "receiver-rate",
                    &c.rate,
                    sample_rates(&c.codec).into_iter().map(|r| Choice::new(r, rate_label(r))).collect(),
                    locked,
                    cx.listener_fn(|this, v: &u32, cx| {
                        let v = *v;
                        this.set(|c| c.rate = v, cx);
                    }),
                ),
                cx,
            ))
            .child(ui::field(t("receiver.bit_depth"), None, None, depth, cx))
            .child(ui::field(
                t("receiver.source"),
                Some(t("receiver.source_tooltip").into()),
                None,
                ui::segmented(
                    "receiver-source",
                    &c.source,
                    sources,
                    cx.listener_fn(|this, v: &Source, cx| {
                        let v = *v;
                        this.set(|c| c.source = v, cx);
                    }),
                ),
                cx,
            ))
            .child(ui::inline().child(ui::hint(t("ui.receiver.afh_hint"), cx)).child(
                ui::button("receiver-afh", t("ui.receiver.afh_open"), ButtonKind::Subtle).small().on_click(
                    |_, _, cx| app::navigate(Route { name: RouteName::Adapter, arg: Some(AdapterSection::Afh) }, cx),
                ),
            ))
    }

    fn received_section(&self, locked: bool, cx: &mut Context<Self>) -> Div {
        let c = self.session.read(cx).choices.clone();
        ui::section(t("receiver.settings_received"), None, cx)
            .child(ui::field(
                t("receiver.mtu"),
                Some(t("receiver.mtu_tooltip").into()),
                None,
                ui::select(
                    "receiver-mtu",
                    &c.mtu,
                    MTUS.iter().map(|v| Choice::new(*v, mtu_label(*v))).collect(),
                    locked,
                    cx.listener_fn(|this, v: &u32, cx| {
                        let v = *v;
                        this.set(|c| c.mtu = v, cx);
                    }),
                ),
                cx,
            ))
            .child(ui::field(
                t("receiver.buffer"),
                None,
                None,
                ui::select(
                    "receiver-buffer",
                    &c.buffer,
                    BUFFERS_MS.iter().map(|v| Choice::new(*v, buffer_label(*v))).collect(),
                    locked,
                    cx.listener_fn(|this, v: &u32, cx| {
                        let v = *v;
                        this.set(|c| c.buffer = v, cx);
                    }),
                ),
                cx,
            ))
    }

    fn checks_section(&self, r: Option<&ReceiverState>, measuring: bool, cx: &mut Context<Self>) -> Div {
        let p = *cx.palette();
        let start = if measuring {
            ui::button("receiver-start", t("receiver.stop"), ButtonKind::Danger).icon(icon(IconName::Stop))
        } else {
            ui::button("receiver-start", t("receiver.start"), ButtonKind::Strong).icon(icon(IconName::Play))
        }
        .loading(self.starting)
        .disabled(!measuring && !can_start(r))
        .on_click(cx.listener(|this, _, _, cx| this.start_stop(cx)));
        let model = &self.session.read(cx).model;
        let steps = STEP_IDS.iter().enumerate().map(|(i, id)| {
            let s = &model.steps[*id];
            let color: Hsla = match s.state {
                StepState::Ok => p.good_text,
                StepState::Running => p.warn_text,
                StepState::Failed => p.bad_text,
                StepState::Pending => p.text_2,
            };
            let mark: AnyElement = match step_icon(s.state) {
                Some(i) => icon(i).into_any_element(),
                None => div().size(px(8.)).rounded_full().border_1().border_color(p.text_3).into_any_element(),
            };
            div()
                .flex()
                .items_center()
                .gap(theme::S3)
                .px(theme::S4)
                .py(theme::S2)
                .min_h(px(40.))
                .when(i > 0, |d| d.border_t_1().border_color(p.border))
                .child(div().w(px(24.)).flex_none().flex().justify_center().text_color(color).child(mark))
                .child(div().w(px(192.)).flex_none().child(t(step_key(*id))))
                .child(
                    div()
                        .w(px(72.))
                        .flex_none()
                        .text_size(theme::FS_SMALL)
                        .text_color(color)
                        .child(step_status(s.state)),
                )
                .child(
                    div().w(px(80.)).flex_none().text_size(theme::FS_SMALL).text_color(p.text_2).child(step_time(s.at)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(theme::FS_SMALL)
                        .text_color(p.text_2)
                        .child(s.detail.clone()),
                )
        });
        ui::section(t("ui.receiver.checks"), Some(start.into_any_element()), cx).child(
            div()
                .flex()
                .flex_col()
                .border_1()
                .border_color(p.border)
                .rounded(theme::RADIUS_CARD)
                .bg(p.surface)
                .children(steps),
        )
    }

    fn comparison_section(&self, r: Option<&ReceiverState>, cx: &mut Context<Self>) -> Div {
        let p = *cx.palette();
        let core = self.core.read(cx);
        let stream = core.snap().map(|s| s.stream.clone()).unwrap_or_default();
        let link = core.store.link().back().cloned();
        let session = self.session.read(cx);
        let rows = compare(&CompareInputs {
            receiver: r,
            stream: &stream,
            link: link.as_ref(),
            base: &session.base,
            last_info: session.model.last_info.as_ref(),
        });
        let cell =
            |text: String, bad: bool| div().flex_1().min_w_0().when(bad, |d| d.text_color(p.bad_text)).child(text);
        let head = div()
            .flex()
            .gap(theme::S3)
            .py(theme::S1)
            .font_weight(FontWeight::SEMIBOLD)
            .child(div().w(px(176.)).flex_none())
            .child(div().flex_1().child(t("receiver.col_sent")))
            .child(div().flex_1().child(t("receiver.col_received")))
            .child(div().flex_1().child(t("ui.receiver.col_diff")));
        let body = ROW_IDS.iter().map(|id| {
            let row = &rows[*id];
            div()
                .flex()
                .gap(theme::S3)
                .py(theme::S1)
                .border_t_1()
                .border_color(p.border)
                .child(div().w(px(176.)).flex_none().font_weight(FontWeight::SEMIBOLD).child(t(row_key(*id))))
                .child(cell(row.sent.text.clone(), row.sent.bad))
                .child(cell(row.received.text.clone(), row.received.bad))
                .child(cell(row.diff.text.clone(), row.diff.bad))
        });
        let reset = ui::button("receiver-reset", t("quality.reset"), ButtonKind::Subtle)
            .icon(icon(IconName::ArrowReset))
            .on_click(cx.listener(|this, _, _, cx| this.reset(cx)));
        ui::section(t("ui.receiver.comparison"), Some(reset.into_any_element()), cx)
            .child(div().flex().flex_col().child(head).children(body))
            .child(ui::hint(t("receiver.help"), cx))
    }

    fn log_section(&self, cx: &mut Context<Self>) -> Div {
        let p = *cx.palette();
        let log = &self.session.read(cx).log;
        let empty = log.is_empty();
        let actions = ui::inline()
            .child(
                ui::button("receiver-copy-log", t("receiver.copy_log"), ButtonKind::Subtle)
                    .icon(icon(IconName::Copy))
                    .disabled(empty)
                    .on_click(cx.listener(|this, _, _, cx| this.copy_log(cx))),
            )
            .child(
                ui::button("receiver-clear-log", t("ui.diag.clear"), ButtonKind::Subtle)
                    .icon(icon(IconName::Delete))
                    .disabled(empty)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |s, cx| {
                            s.log.clear();
                            cx.notify();
                        });
                    })),
            );
        let lines: Vec<Div> = log
            .iter()
            .map(|(time, text)| {
                div()
                    .flex()
                    .gap(theme::S2)
                    .px(theme::S3)
                    .child(div().w(px(72.)).flex_none().text_color(p.text_3).child(time_text(*time)))
                    .child(div().flex_1().min_w_0().child(text.clone()))
            })
            .collect();
        ui::section(t("receiver.log"), Some(actions.into_any_element()), cx).child(
            div()
                .id("receiver-log")
                .h(px(192.))
                .py(theme::S2)
                .border_1()
                .border_color(p.border)
                .rounded(theme::RADIUS_CARD)
                .bg(p.surface)
                .font_family(theme::FONT_MONO)
                .text_size(theme::FS_SMALL)
                .overflow_y_scroll()
                .track_scroll(&self.log_scroll)
                .when(empty, |d| d.child(div().px(theme::S3).text_color(p.text_2).child(t("ui.diag.no_log"))))
                .children(lines),
        )
    }
}

/// A value where a codec has no choice (`.input.readonly`)
fn readonly(text: impl Into<SharedString>, cx: &App) -> Div {
    let p = cx.palette();
    div()
        .min_w(px(120.))
        .px(theme::S3)
        .py(px(5.))
        .rounded(theme::RADIUS)
        .border_1()
        .border_color(p.border)
        .bg(p.surface_2)
        .text_color(p.text_2)
        .child(text.into())
}

/// `cx.listener` for the controls whose callback takes the value only
trait ListenerFn<T> {
    fn listener_fn<E: 'static>(
        &self,
        f: impl Fn(&mut T, &E, &mut Context<T>) + 'static,
    ) -> impl Fn(&E, &mut Window, &mut App) + 'static;
}

impl<T: 'static> ListenerFn<T> for Context<'_, T> {
    fn listener_fn<E: 'static>(
        &self,
        f: impl Fn(&mut T, &E, &mut Context<T>) + 'static,
    ) -> impl Fn(&E, &mut Window, &mut App) + 'static {
        self.listener(move |this, e: &E, _, cx| f(this, e, cx))
    }
}

impl Render for ReceiverTest {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = self.receiver(cx);
        let measuring = self.session.read(cx).model.measuring;
        let locked = measuring || self.starting;
        div()
            .flex()
            .flex_col()
            .gap(theme::S4)
            .child(ui::hint(t("receiver.intro"), cx))
            .child(self.address_section(r.as_ref(), cx))
            .child(self.sent_section(r.as_ref(), locked, cx))
            .child(self.received_section(locked, cx))
            .child(self.checks_section(r.as_ref(), measuring, cx))
            .child(self.comparison_section(r.as_ref(), cx))
            .child(self.log_section(cx))
    }
}

#[cfg(test)]
mod interaction_tests;
