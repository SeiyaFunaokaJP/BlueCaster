//! Keyboards and mice through BlueCaster (experimental, developer mode,
//! issue #413): the Developer section with its switch and devices, the
//! question before turning it off, and the banner over every screen while
//! it is on. Turning it on goes through hid_enable_dialog.rs.
//!
//! A device being added stays only once its input reaches Windows: the
//! core counts down (Snapshot.hid_checks) and removes it when no input
//! arrives in time, also with this window closed. The UI only shows the
//! countdown and offers Remove; nothing here can confirm a device.

use std::time::{Duration, Instant};

use gpui_kit::component::Disableable as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, AsyncApp, Context, Div, Entity, FontWeight, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Task, WeakEntity, Window, div, px,
};
use ipc::{Device, HidCheck, HidCounts, HidDevice, HidPasskey, HidRolledBack, HidStatus, Method};
use ui_model::i18n::t;
use ui_model::t;
use ui_model::view::devices::SCAN_SECONDS;
use ui_model::view::hid::{check_left, hid_candidates, hid_check_hint, hid_on, hid_test_failed, name_or, passkey_name};

use crate::app;
use crate::core::Core;
use crate::screens::hid_enable_dialog::open_hid_enable_dialog;
use crate::screens::service_addon::ServiceRow;
use crate::theme::{self, ActivePalette as _};
use crate::ui::dialog::{self, Question};
use crate::ui::{self, BannerKind, ButtonKind, IconName, icon};

/// How often the counts are read while the section is on screen
const COUNTS_EVERY_MS: u64 = 2000;

/// A dialog's title after a warning sign: confirming loses something
pub(crate) fn warn_title(title: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .gap(theme::S2)
        .child(div().flex().text_color(cx.palette().warn_text).child(icon(IconName::Warning)))
        .child(title.into())
}

/// A question with a warning sign whose confirmed request runs while the
/// dialog stays (closing it would hide the spinner, not stop the work); it
/// closes once `run` is done. Cancel, Escape and the backdrop do nothing
/// meanwhile.
pub(crate) fn warn_dialog(
    title: impl Into<SharedString>,
    text: impl Into<SharedString>,
    confirm: impl Into<SharedString>,
    run: impl Fn(&mut App) -> Task<()> + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let (title, text, confirm) = (title.into(), text.into(), confirm.into());
    let busy = dialog::busy_state(cx);
    dialog::ask_run(
        move |_, _| Question::new(title.clone()).warn().confirm(confirm.clone()),
        busy,
        move |_, _| div().child(text.clone()).into_any_element(),
        move |_, cx| {
            let task = run(cx);
            cx.spawn(async move |_: &mut AsyncApp| {
                task.await;
                true
            })
        },
        window,
        cx,
    );
}

/// Asks before turning it off: the mouse in hand may be one of them
pub fn hid_off_dialog(window: &mut Window, cx: &mut App) {
    warn_dialog(
        t("ui.dev.hid_off_title"),
        t("ui.dev.hid_off_text"),
        t("ui.dev.hid_off_confirm"),
        |cx| {
            let off = app::run(app::request::<serde_json::Value>(Method::HidDisable, cx), cx);
            cx.spawn(async move |_: &mut AsyncApp| {
                off.await;
            })
        },
        window,
        cx,
    );
}

/// While it is on: over every screen, not dismissable
pub fn hid_notice(cx: &App) -> impl IntoElement + use<> {
    let snap = app::core(cx).read(cx).snap();
    let Some(snap) = snap.filter(|s| hid_on(Some(&s.settings))) else { return div().into_any_element() };
    let text = t(if snap.service_mode { "ui.dev.hid_banner_text_service" } else { "ui.dev.hid_banner_text" });
    let body = div().flex().flex_col().gap(theme::S1).child(text).when(hid_test_failed(Some(snap)), |d| {
        d.child(div().font_weight(FontWeight::SEMIBOLD).child(t("ui.dev.hid_banner_test_failed")))
    });
    let off = ui::button("hid-notice-off", t("ui.dev.hid_off_confirm"), ButtonKind::Default)
        .on_click(|_, window, cx| hid_off_dialog(window, cx));
    ui::banner(
        BannerKind::Warn,
        Some(t("ui.dev.hid_banner_title").into()),
        Some(body.into_any_element()),
        Some(div().flex().child(off).into_any_element()),
        None,
        cx,
    )
    .into_any_element()
}

/// Developer > App settings: the switch, and while it is on the devices
pub struct HidSection {
    core: Entity<Core>,
    service_row: Entity<ServiceRow>,
    devices: Option<Entity<HidDevices>>,
}

impl HidSection {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> HidSection {
        let core = app::core(cx);
        cx.observe(&core, |this, core, cx| {
            this.sync_devices(&core, cx);
            cx.notify();
        })
        .detach();
        let service_row = cx.new(|cx| ServiceRow::new(window, cx));
        let mut this = HidSection { core: core.clone(), service_row, devices: None };
        this.sync_devices(&core, cx);
        this
    }

    /// The devices' part lives while the feature is on
    fn sync_devices(&mut self, core: &Entity<Core>, cx: &mut Context<Self>) {
        let on = hid_on(core.read(cx).snap().map(|s| &s.settings));
        if on && self.devices.is_none() {
            self.devices = Some(cx.new(HidDevices::new));
        } else if !on {
            self.devices = None;
        }
    }
}

impl Render for HidSection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(snap) = self.core.read(cx).snap() else { return div().into_any_element() };
        let on = hid_on(Some(&snap.settings));
        let hint = t(if snap.service_mode { "ui.dev.hid_toggle_hint_service" } else { "ui.dev.hid_toggle_hint" });
        ui::section(t("ui.dev.hid_section"), None, cx)
            // Controlled: the switch shows what the core has; turning it on
            // or off goes through the dialogs
            .child(ui::toggle(
                "hid-toggle",
                t("ui.dev.hid_toggle"),
                Some(hint.into()),
                on,
                false,
                |v, window, cx| {
                    if *v {
                        open_hid_enable_dialog(window, cx);
                    } else {
                        hid_off_dialog(window, cx);
                    }
                },
                cx,
            ))
            .child(self.service_row.clone())
            .when_some(self.devices.clone(), |d, devices| d.child(devices))
            .into_any_element()
    }
}

/// `.hid-passkey`: a block framed in the accent colour
fn passkey_frame(cx: &App) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(theme::S1)
        .p(theme::S3)
        .border_1()
        .border_color(cx.palette().accent_fill)
        .rounded(theme::RADIUS)
}

/// While a keyboard being added pairs: the number the user types on it
pub fn hid_passkey_block(passkey: &HidPasskey, added: &[HidDevice], cx: &App) -> Div {
    passkey_frame(cx)
        .child(div().font_weight(FontWeight::SEMIBOLD).child(passkey_name(&passkey.address, added).to_owned()))
        .child(
            div()
                .font_family(theme::FONT_MONO)
                .text_size(theme::FS_PAGE)
                .font_weight(FontWeight::SEMIBOLD)
                .child(passkey.passkey.clone()),
        )
        .child(div().child(t("ui.dev.hid_passkey_hint")))
}

/// A device being added: the instruction and the core's countdown
pub fn hid_check_block(
    check: &HidCheck,
    left: u64,
    busy: bool,
    on_remove: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> Div {
    let text = div()
        .flex()
        .flex_col()
        .gap(theme::S1)
        .flex_1()
        .min_w_0()
        .child(div().font_weight(FontWeight::SEMIBOLD).child(name_or(&check.name, &check.address).to_owned()))
        .when(!check.waiting, |d| d.child(div().child(t("status.connecting"))))
        .when(check.waiting, |d| {
            d.child(div().child(hid_check_hint(&check.kind))).child(ui::muted(t!("ui.dev.hid_check_left", left), cx))
        });
    passkey_frame(cx).child(
        div().flex().items_start().gap(theme::S3).child(text).child(
            ui::button(
                SharedString::from(format!("hid-check-{}", check.address)),
                t("ui.device.forget"),
                ButtonKind::Subtle,
            )
            .loading(busy)
            .on_click(move |_, window, cx| on_remove(window, cx)),
        ),
    )
}

/// A device removed again: one sentence, Try again and Close
pub fn hid_rolled_back_block(
    rolled_back: &HidRolledBack,
    busy: bool,
    on_retry: impl Fn(&mut Window, &mut App) + 'static,
    on_close: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> Div {
    passkey_frame(cx).child(
        div()
            .flex()
            .items_start()
            .gap(theme::S3)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(t!("ui.dev.hid_rolled_back", name_or(&rolled_back.name, &rolled_back.address))),
            )
            .child(
                div()
                    .flex()
                    .gap(theme::S2)
                    .child(
                        ui::button("hid-retry", t("ui.fix.retry"), ButtonKind::Default)
                            .loading(busy)
                            .on_click(move |_, window, cx| on_retry(window, cx)),
                    )
                    .child(
                        ui::button("hid-rolled-back-close", t("modal.close"), ButtonKind::Subtle)
                            .disabled(busy)
                            .on_click(move |_, window, cx| on_close(window, cx)),
                    ),
            ),
    )
}

/// `.hid-list`: framed rows, a line between them
fn hid_list(rows: Vec<AnyElement>, cx: &App) -> Div {
    let border = cx.palette().border;
    div().flex().flex_col().border_1().border_color(border).rounded(theme::RADIUS).children(
        rows.into_iter()
            .enumerate()
            .map(move |(i, r)| div().when(i > 0, |d| d.border_t_1().border_color(border)).child(r)),
    )
}

/// `.row.hid-device`: the icon, the name and a second line, the buttons
fn hid_row(name: String, sub: Option<Div>, actions: Div, cx: &App) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(theme::S3)
        .px(theme::S3)
        .py(theme::S2)
        .min_h(px(40.))
        .child(div().flex().flex_none().text_color(cx.palette().text_2).child(icon(IconName::Bluetooth)))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(div().font_weight(FontWeight::SEMIBOLD).truncate().child(name))
                .when_some(sub, |d, s| d.child(s)),
        )
        .child(actions.ml_auto().flex().gap(theme::S2))
        .into_any_element()
}

/// A check as it arrived: the countdown runs from then
struct SeenCheck {
    check: HidCheck,
    at: Instant,
}

pub struct HidDevices {
    core: Entity<Core>,
    counts: Option<HidCounts>,
    // One request at a time: adding pairs, the others wait
    working: Option<String>,
    scanned: bool,
    checks: Vec<SeenCheck>,
    _counts: Task<()>,
    _ticker: Task<()>,
}

impl HidDevices {
    pub fn new(cx: &mut Context<Self>) -> HidDevices {
        let core = app::core(cx);
        cx.observe(&core, |this, _, cx| {
            this.see_checks(cx);
            cx.notify();
        })
        .detach();
        // The counts, now and every COUNTS_EVERY_MS while on screen
        let counts = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            loop {
                let Ok(task) = this.update(cx, |_, cx| app::request::<HidStatus>(Method::HidStatus, cx)) else { break };
                let counts = task.await.ok().and_then(|s| s.counts);
                if this
                    .update(cx, |this, cx| {
                        this.counts = counts;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
                cx.background_executor().timer(Duration::from_millis(COUNTS_EVERY_MS)).await;
            }
        });
        // The checks' countdowns
        let ticker = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            loop {
                cx.background_executor().timer(Duration::from_millis(250)).await;
                let alive = this.update(cx, |this, cx| {
                    if !this.checks.is_empty() {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        let mut this = HidDevices {
            core,
            counts: None,
            working: None,
            scanned: false,
            checks: Vec::new(),
            _counts: counts,
            _ticker: ticker,
        };
        this.see_checks(cx);
        this
    }

    /// The deadline as of when the core last sent each check
    fn see_checks(&mut self, cx: &App) {
        let now = Instant::now();
        let current = self.core.read(cx).snap().map(|s| s.hid_checks.clone()).unwrap_or_default();
        self.checks = current
            .into_iter()
            .map(|c| {
                let at = self.checks.iter().find(|s| s.check == c).map_or(now, |s| s.at);
                SeenCheck { check: c, at }
            })
            .collect();
    }

    fn set_working(&mut self, address: Option<String>, cx: &mut Context<Self>) {
        self.working = address;
        cx.notify();
    }

    fn scan(&mut self, cx: &mut Context<Self>) {
        let task = app::run(app::request::<serde_json::Value>(Method::DeviceScan { seconds: SCAN_SECONDS }, cx), cx);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            task.await;
            let _ = this.update(cx, |this, cx| {
                this.scanned = true;
                cx.notify();
            });
        })
        .detach();
    }

    fn add_address(&mut self, address: String, name: String, le: Option<u8>, cx: &mut Context<Self>) {
        self.set_working(Some(address.clone()), cx);
        let scanning = self.core.read(cx).snap().is_some_and(|s| s.scanning);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            // Pairing needs the radio: a scan still running ends first
            if scanning {
                let Ok(stop) = this.update(cx, |_, cx| app::request::<serde_json::Value>(Method::DeviceScanStop, cx))
                else {
                    return;
                };
                let _ = stop.await;
            }
            let Ok(add) = this.update(cx, |_, cx| {
                app::run(app::request::<serde_json::Value>(Method::HidAdd { address, name, le }, cx), cx)
            }) else {
                return;
            };
            add.await;
            let _ = this.update(cx, |this, cx| this.set_working(None, cx));
        })
        .detach();
    }

    fn add(&mut self, d: &Device, cx: &mut Context<Self>) {
        let name = name_or(&d.name, &d.address).to_owned();
        self.add_address(d.address.clone(), name, d.hid_le, cx);
    }

    fn retry(&mut self, r: &HidRolledBack, cx: &mut Context<Self>) {
        let name = name_or(&r.name, &r.address).to_owned();
        self.add_address(r.address.clone(), name, r.le, cx);
    }

    /// One request for a device: working on it until it answers
    fn run_for(&mut self, address: String, method: Method, cx: &mut Context<Self>) -> Task<()> {
        self.set_working(Some(address), cx);
        let task = app::run(app::request::<serde_json::Value>(method, cx), cx);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            task.await;
            let _ = this.update(cx, |this, cx| this.set_working(None, cx));
        })
    }

    fn cancel_check(&mut self, address: String, cx: &mut Context<Self>) {
        let method = Method::HidCheckCancel { address: address.clone() };
        self.run_for(address, method, cx).detach();
    }

    fn connect(&mut self, address: String, cx: &mut Context<Self>) {
        let method = Method::HidConnect { address: address.clone() };
        self.run_for(address, method, cx).detach();
    }

    /// Forget: asks first, then removes it
    fn ask_remove(&mut self, d: &HidDevice, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        let address = d.address.clone();
        warn_dialog(
            t!("ui.dev.hid_remove_title", name_or(&d.name, &d.address)),
            t("ui.dev.hid_remove_text"),
            t("ui.device.forget"),
            move |cx| {
                let address = address.clone();
                let method = Method::HidRemove { address: address.clone() };
                this.update(cx, |this, cx| this.run_for(address, method, cx)).unwrap_or_else(|_| Task::ready(()))
            },
            window,
            cx,
        );
    }
}

impl Render for HidDevices {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(snap) = self.core.read(cx).snap().cloned() else { return div().into_any_element() };
        let added = &snap.hid_devices;
        let working = self.working.clone();
        let busy_any = working.is_some();
        let now = Instant::now();
        let candidates = hid_candidates(&snap.devices, &snap.hid_candidates, added);

        let checks: Vec<AnyElement> = self
            .checks
            .iter()
            .map(|s| {
                let left = check_left(s.check.remaining_ms, now.duration_since(s.at).as_millis() as u64);
                let address = s.check.address.clone();
                let busy = working.as_deref() == Some(s.check.address.as_str());
                let this = cx.entity().downgrade();
                hid_check_block(
                    &s.check,
                    left,
                    busy,
                    move |_, cx| {
                        let _ = this.update(cx, |this, cx| this.cancel_check(address.clone(), cx));
                    },
                    cx,
                )
                .into_any_element()
            })
            .collect();

        let rolled_back = snap.hid_rolled_back.as_ref().map(|r| {
            let busy = working.as_deref() == Some(r.address.as_str());
            let (this, again) = (cx.entity().downgrade(), r.clone());
            hid_rolled_back_block(
                r,
                busy,
                move |_, cx| {
                    let _ = this.update(cx, |this, cx| this.retry(&again, cx));
                },
                |_, cx| app::send(Method::HidCheckDismiss, cx),
                cx,
            )
        });

        let added_list = if added.is_empty() {
            ui::hint(t("ui.dev.hid_devices_empty"), cx)
        } else {
            let rows = added
                .iter()
                .map(|d| {
                    let id = d.address.clone();
                    let sub =
                        ui::muted(if d.connected { t("status.connected") } else { t("ui.dev.hid_not_connected") }, cx);
                    let actions = div()
                        .when(!d.connected, |a| {
                            a.child(
                                ui::button(
                                    SharedString::from(format!("hid-connect-{id}")),
                                    t("connection.connect"),
                                    ButtonKind::Default,
                                )
                                .loading(working.as_deref() == Some(id.as_str()))
                                .disabled(busy_any)
                                .on_click(cx.listener({
                                    let id = id.clone();
                                    move |this, _, _, cx| this.connect(id.clone(), cx)
                                })),
                            )
                        })
                        .child(
                            ui::button(
                                SharedString::from(format!("hid-forget-{id}")),
                                t("ui.device.forget"),
                                ButtonKind::Subtle,
                            )
                            .disabled(busy_any)
                            .on_click(cx.listener({
                                let d = d.clone();
                                move |this, _, window, cx| this.ask_remove(&d, window, cx)
                            })),
                        );
                    hid_row(name_or(&d.name, &d.address).to_owned(), Some(sub), actions, cx)
                })
                .collect();
            hid_list(rows, cx)
        };

        let scan = if snap.scanning {
            ui::button("hid-scan-stop", t("connection.scan_stop"), ButtonKind::Default)
                .on_click(|_, _, cx| app::send(Method::DeviceScanStop, cx))
        } else {
            ui::button("hid-scan", t("connection.scan"), ButtonKind::Default)
                .icon(icon(IconName::Search))
                .disabled(busy_any)
                .on_click(cx.listener(|this, _, _, cx| this.scan(cx)))
        };

        let candidate_list = (!candidates.is_empty()).then(|| {
            let rows = candidates
                .iter()
                .map(|d| {
                    let sub = (!d.name.is_empty()).then(|| {
                        ui::mono(d.address.clone()).text_size(theme::FS_SMALL).text_color(cx.palette().text_2)
                    });
                    let add = ui::button(
                        SharedString::from(format!("hid-add-{}", d.address)),
                        t("ui.devices.add"),
                        ButtonKind::Default,
                    )
                    .loading(working.as_deref() == Some(d.address.as_str()))
                    .disabled(busy_any)
                    .on_click(cx.listener({
                        let d = d.clone();
                        move |this, _, _, cx| this.add(&d, cx)
                    }));
                    hid_row(name_or(&d.name, &d.address).to_owned(), sub, div().child(add), cx)
                })
                .collect();
            hid_list(rows, cx)
        });

        div()
            .flex()
            .flex_col()
            .gap(theme::S3)
            .when_some(snap.hid_passkey.as_ref(), |d, p| d.child(hid_passkey_block(p, added, cx)))
            .children(checks)
            .when_some(rolled_back, |d, r| d.child(r))
            .child(added_list)
            .child(ui::inline().child(scan).child(ui::hint(t("ui.dev.hid_pairing_mode"), cx)))
            .when_some(candidate_list, |d, l| d.child(l))
            .when(self.scanned && !snap.scanning && candidates.is_empty(), |d| {
                d.child(ui::hint(t("ui.dev.hid_found_none"), cx))
            })
            .when_some(self.counts, |d, c| {
                d.child(
                    ui::hint(t!("ui.dev.hid_counts", c.reports, c.injected, c.blocked), cx)
                        .font_family(theme::FONT_MONO),
                )
            })
            .into_any_element()
    }
}
