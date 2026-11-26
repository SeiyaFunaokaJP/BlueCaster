//! First-run setup, shown when there is no profile or no adapter on WinUSB,
//! until it was finished or skipped once (settings.setup_done; Settings can
//! run it again).
//! Four steps (issue #86): 1 choose the adapter, 2 driver and firmware (one
//! button switches it to WinUSB and downloads its firmware), 3 find the audio
//! devices: the scan starts by itself, and Connect on a device makes its
//! "<name> | Auto" profile and connects with it, so one click is enough.
//! 4 the general settings worth asking (Start with Windows and in the tray are
//! on by default). The scan runs while nothing is connected, as the core
//! refuses one during a stream: a connected device joins the saved ones above
//! the list, and adding another one disconnects it. New devices get the
//! installed cables (setup_audio: the Hi-Fi Cable for the sound and VB-Cable
//! for the microphone, a single one for the microphone); the step suggests
//! the cables that are not. The setup has a window size of its own (shell.rs).
//!
//! The rules (steps, order, Next, what the driver button does) are in
//! ui_model::view::setup, tested there; this file draws them.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::radio::Radio;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AnyView, App, AppContext as _, AsyncApp, AsyncWindowContext, Context, Div, ElementId, Entity,
    FontWeight, Image, ImageFormat, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div, img, px,
};
use ipc::{AdapterInfo, AudioDevice, Device, DriverKind, ErrorInfo, Method, Snapshot, StreamState};
use ui_model::chips::TagStatus;
use ui_model::i18n::t;
use ui_model::links::{LINKS, firmware_url};
use ui_model::profile::device_key;
use ui_model::t;
use ui_model::view::setup::{
    DEVICE_STEP, HEADING_KEYS, LAST, STEP_KEYS, after_assign, can_next, chosen_adapter, classic_entries, driver_plan,
    firmware_to_download, heading_shown, losing_devices, paired_label, scan_resumes, stream_running, supported_first,
    turns_startup_on,
};

use crate::app::{self, RouteName};
use crate::core::Core;
use crate::host;
use crate::screens::adapter::{adapter_ids, adapter_ids_hint, adapter_name, adapter_supported, driver_text, tag};
use crate::screens::settings::{battery_low_choice, theme_choice};
use crate::theme::{self, ActivePalette as _};
use crate::ui::cable_notes::cable_notes;
use crate::ui::device_picker::{
    DevicePicker, DevicePickerProps, OnConnect, PairedState, SetupAudio, save_with_profile,
};
use crate::ui::dialog::{self, Question};
use crate::ui::import_pairings::ImportPairings;
use crate::ui::stream::{
    adapter_hung, dot, entry_state, firmware_folder_button, paired_devices_note, state_colors, tip, v1_running,
    windows_devices,
};
use crate::ui::{self, BannerKind, ButtonKind, IconName, icon};

const APP_ICON: &[u8] = include_bytes!("../../resources/app-icon.png");

/// No profile yet, or no adapter with the WinUSB driver
pub fn needs_setup(snap: &Snapshot) -> bool {
    ui_model::view::setup::needs_setup(&snap.profiles, &snap.adapter.adapters)
}

pub fn open(window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| Setup::new(window, cx)).into()
}

/// What the driver step is doing
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Phase {
    #[default]
    Idle,
    Driver,
    Firmware,
}

/// The driver step's state, anew each time the step opens
#[derive(Debug, Default)]
struct DriverStep {
    phase: Phase,
    failed: Option<String>,
    /// The download outlived the UI's wait; the core finishes it and publishes the adapter
    slow: bool,
    /// Reading the devices Windows paired through the adapter: they go with
    /// it, so the setup asks first (issue #86: a built-in adapter has the keyboard)
    checking: bool,
}

/// The device step's state, made when the step opens; the picker gone ends
/// its scan
struct DeviceStep {
    picker: Entity<DevicePicker>,
    import: Entity<ImportPairings>,
    on_connect: OnConnect,
    paired: PairedState,
    /// Windows' outputs, once read: what new devices get from the cables
    /// (setup_audio); the step suggests the cables it lacks (issue #86)
    devices: Option<Vec<AudioDevice>>,
    audio_key: String,
    /// The device being connected: its button spins
    connecting: Option<String>,
    /// No scan from a Connect until the stream it starts has ended; after a
    /// failure none by itself either: a scan would clear the error on the
    /// row, and the core refuses one until the session has cleaned up
    paused: bool,
    was_running: bool,
    stopping: bool,
    /// What the picker was last given (connecting, disabled, no scan, continuous)
    given: Option<(Option<String>, bool, bool, bool)>,
}

pub struct Setup {
    core: Entity<Core>,
    icon: Arc<Image>,
    step: usize,
    chosen: Option<String>,
    /// A first setup: no profile yet when it reached the devices
    first: bool,
    /// "Start with Windows" and "Start in the tray" are on by default (issue
    /// #86): turned on once when a first setup reaches them; switched off
    /// there, they stay off
    startup_on: bool,
    /// Counts the steps opened: a request that ends after its step was left
    /// leaves the new one alone
    visit: u64,
    refreshing: bool,
    driver: DriverStep,
    device: Option<DeviceStep>,
}

impl Setup {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Setup {
        let core = app::core(cx);
        cx.observe(&core, |this: &mut Self, _, cx| {
            this.core_changed(cx);
            cx.notify();
        })
        .detach();
        cx.observe_window_activation(window, |this: &mut Self, window, cx| {
            if window.is_window_active() && this.device.is_some() {
                this.load_devices(cx);
            }
        })
        .detach();
        let mut this = Setup {
            core,
            icon: Arc::new(Image::from_bytes(ImageFormat::Png, APP_ICON.to_vec())),
            step: 0,
            chosen: None,
            first: false,
            startup_on: false,
            visit: 0,
            refreshing: false,
            driver: DriverStep::default(),
            device: None,
        };
        // For looking at one screen: open on another step
        if let Some(step) = crate::demo::setup_step() {
            this.go(step.min(LAST), window, cx);
        }
        this
    }

    fn snap<'a>(&self, cx: &'a App) -> Option<&'a Snapshot> {
        self.core.read(cx).snap()
    }

    /// The adapter the setup works with
    fn adapter(&self, cx: &App) -> Option<AdapterInfo> {
        chosen_adapter(&self.snap(cx)?.adapter.adapters, self.chosen.as_deref())
    }

    /// Finished or skipped: it does not open by itself again (issue #86), nor
    /// do the cables, shown here, after it
    fn finish(&mut self, cx: &mut Context<Self>) {
        if self.snap(cx).is_some_and(|s| !s.settings.setup_done) {
            app::save_settings(
                |s| {
                    s.setup_done = true;
                    s.cables_dialog_shown = true;
                },
                cx,
            );
        }
        app::state(cx).update(cx, |s, cx| s.set_setup_active(false, cx));
        app::navigate(RouteName::Home, cx);
    }

    fn go(&mut self, n: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(snap) = self.snap(cx).cloned() else { return };
        if n == DEVICE_STEP && snap.profiles.is_empty() {
            self.first = true;
        }
        if n == LAST && !self.startup_on {
            self.startup_on = true;
            let s = &snap.settings;
            if turns_startup_on(self.first, s.start_with_windows, s.start_minimized) {
                app::save_settings(
                    |s| {
                        s.start_with_windows = true;
                        s.start_minimized = true;
                    },
                    cx,
                );
            }
        }
        self.step = n;
        self.visit += 1;
        self.driver = DriverStep::default();
        self.device = (n == DEVICE_STEP).then(|| self.device_step(&snap, window, cx));
        if self.device.is_some() {
            self.load_devices(cx);
            self.sync_picker(cx);
        }
        cx.notify();
    }

    // ---- Step 1: the adapter ----------------------------------------------------

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.refreshing = true;
        cx.notify();
        let task = app::run(app::request::<Vec<AdapterInfo>>(Method::AdapterList, cx), cx);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            task.await;
            let _ = this.update(cx, |this, cx| {
                this.refreshing = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn step_adapter(&self, snap: &Snapshot, cx: &mut Context<Self>) -> Div {
        let p = *cx.palette();
        let adapters = &snap.adapter.adapters;
        let chosen = self.adapter(cx).map(|a| a.instance_id);
        let list = if adapters.is_empty() {
            (!snap.adapter.hung).then(|| {
                let body = div().child(t("ui.adapter.none_hint")).into_any_element();
                ui::banner(BannerKind::Warn, Some(t("firmware.no_adapters").into()), Some(body), None, None, cx)
                    .into_any_element()
            })
        } else {
            let rows = supported_first(adapters).into_iter().enumerate().map(|(i, a)| {
                let on = chosen.as_deref() == Some(a.instance_id.as_str());
                let ok = adapter_supported(&a);
                let id = a.instance_id.clone();
                // One BlueCaster cannot use is listed, not chosen (issue #86)
                div()
                    .when(i > 0, |d| d.border_t_1().border_color(p.border))
                    .when(on, |d| d.bg(p.surface_2))
                    .when(ok && !on, |d| d.hover(|s| s.bg(p.surface_2)))
                    .relative()
                    .child(
                        Radio::new(ElementId::Name(format!("adapter-{}", a.instance_id).into()))
                            .w_full()
                            .items_center()
                            .px(theme::S3)
                            .py(theme::S2)
                            .checked(on)
                            .disabled(!ok)
                            .accessibility_label(adapter_name(&a))
                            .on_click(cx.listener(move |this, _: &bool, _, cx| {
                                this.chosen = Some(id.clone());
                                cx.notify();
                            }))
                            .child(adapter_line(&a, !ok, cx)),
                    )
                    .when(on, |d| d.child(div().absolute().left_0().top_0().bottom_0().w(px(3.)).bg(p.accent_fill)))
            });
            Some(
                div()
                    .id("setup-adapters")
                    .flex()
                    .flex_col()
                    .border_1()
                    .border_color(p.border)
                    .rounded(theme::RADIUS)
                    .max_h(px(256.))
                    .overflow_y_scrollbar()
                    .children(rows)
                    .into_any_element(),
            )
        };
        div().flex().flex_col().gap(theme::S4).children(adapter_hung(cx)).children(list).child(
            div().flex().child(
                ui::button("setup-refresh", t("ui.adapter.refresh"), ButtonKind::Default)
                    .icon(icon(IconName::ArrowSync))
                    .loading(self.refreshing)
                    .disabled(self.refreshing)
                    .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
            ),
        )
    }

    // ---- Step 2: driver and firmware ----------------------------------------------
    //
    // One explanation and one button (issue #86: the driver and firmware
    // choices were intimidating): it switches the adapter to WinUSB and
    // downloads its firmware when it needs some. The separate steps stay on
    // the Adapters screen. Firmware BlueCaster cannot fetch still has the
    // manual way.

    fn start(&mut self, a: AdapterInfo, window: &mut Window, cx: &mut Context<Self>) {
        if a.driver == DriverKind::Winusb {
            return self.set_up(a, cx);
        }
        self.driver.checking = true;
        cx.notify();
        let visit = self.visit;
        // Not the audio devices: the setup's import and pairing take them
        // over (issue #86)
        let task = windows_devices(&a, cx);
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
            let list = losing_devices(task.await);
            let _ = cx.update(|window, cx| {
                let current = this
                    .update(cx, |this, cx| {
                        if this.visit != visit {
                            return false;
                        }
                        this.driver.checking = false;
                        cx.notify();
                        if list.is_empty() {
                            this.set_up(a.clone(), cx);
                        }
                        true
                    })
                    .unwrap_or(false);
                if current && !list.is_empty() {
                    let weak = this.clone();
                    dialog::ask(
                        Question::new(t("ui.adapter.paired_title"))
                            .confirm(t("ui.setup.auto_button"))
                            .confirm_icon(IconName::Shield)
                            .danger()
                            .warn(),
                        move |_, cx| paired_devices_note(&list, cx).into_any_element(),
                        move |_, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                if let Some(a) = this.adapter(cx) {
                                    this.set_up(a, cx);
                                }
                            });
                        },
                        window,
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    fn set_up(&mut self, a: AdapterInfo, cx: &mut Context<Self>) {
        self.driver.failed = None;
        self.driver.slow = false;
        let visit = self.visit;
        let assign = (a.driver != DriverKind::Winusb).then(|| {
            self.driver.phase = Phase::Driver;
            app::request::<Vec<AdapterInfo>>(Method::AdapterAssignWinusb { instance_id: a.instance_id.clone() }, cx)
        });
        cx.notify();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let update = |cx: &mut AsyncApp, f: &dyn Fn(&mut DriverStep)| {
                let _ = this.update(cx, |this, cx| {
                    if this.visit == visit {
                        f(&mut this.driver);
                        cx.notify();
                    }
                });
            };
            let failed = |e: &ErrorInfo| {
                let text = app::error_text(e);
                move |d: &mut DriverStep| d.failed = Some(text.clone())
            };
            let now = match assign {
                Some(task) => match task.await {
                    Ok(list) => after_assign(&list, &a),
                    Err(e) => {
                        let f = failed(&e);
                        return update(cx, &move |d| {
                            f(d);
                            d.phase = Phase::Idle;
                        });
                    }
                },
                None => a,
            };
            if let Some(file) = firmware_to_download(&now) {
                update(cx, &|d| d.phase = Phase::Firmware);
                let task = cx.update(|cx| {
                    app::request::<serde_json::Value>(Method::AdapterDownloadFirmware { file: file.clone() }, cx)
                });
                match task.await {
                    Ok(_) => {}
                    Err(e) if e.code == "ipc.timeout" => update(cx, &|d| d.slow = true),
                    Err(e) => {
                        let f = failed(&e);
                        update(cx, &move |d| f(d));
                    }
                }
            }
            update(cx, &|d| d.phase = Phase::Idle);
        })
        .detach();
    }

    fn step_driver(&self, a: Option<AdapterInfo>, v1: bool, cx: &mut Context<Self>) -> Div {
        let p = *cx.palette();
        let Some(a) = a else { return ui::muted(t("firmware.no_adapters"), cx) };
        let plan = driver_plan(&a);
        let d = &self.driver;
        let files = a.firmware_files.join(", ");
        let body = if d.phase != Phase::Idle {
            Some(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(theme::S3)
                    .py(theme::S4)
                    .text_center()
                    .child(Spinner::new().large().color(p.text_2))
                    .child(t(if d.phase == Phase::Driver { "ui.setup.auto_driver" } else { "firmware.downloading" }))
                    .into_any_element(),
            )
        } else if plan.todo {
            let bullet = |content: AnyElement| {
                div().flex().gap(theme::S2).child("•").child(div().flex_1().min_w_0().child(content))
            };
            let start = a.clone();
            Some(
                div()
                    .flex()
                    .flex_col()
                    .gap(theme::S4)
                    .child(div().child(t(if plan.needs_driver {
                        "ui.setup.auto_explain_admin"
                    } else {
                        "ui.setup.auto_explain"
                    })))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(theme::S1)
                            .pl(theme::S5)
                            .when(plan.needs_driver, |x| {
                                x.child(bullet(div().child(t("ui.setup.auto_will_driver")).into_any_element()))
                            })
                            .when(plan.needs_firmware && a.firmware_download, |x| {
                                x.child(bullet(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap(theme::S1)
                                        .child(
                                            div()
                                                .flex()
                                                .flex_wrap()
                                                .gap_x(theme::S1)
                                                .child(t("ui.setup.auto_will_firmware"))
                                                .child(ui::mono(files.clone())),
                                        )
                                        .child(ui::hint(t("ui.setup.license_note"), cx))
                                        .into_any_element(),
                                ))
                            }),
                    )
                    .child(
                        div().flex().child(
                            ui::button("setup-auto", t("ui.setup.auto_button"), ButtonKind::Strong)
                                .icon(icon(if plan.needs_driver { IconName::Shield } else { IconName::Checkmark }))
                                .loading(d.checking)
                                .disabled(v1 || !adapter_supported(&a) || d.checking)
                                .on_click(
                                    cx.listener(move |this, _, window, cx| this.start(start.clone(), window, cx)),
                                ),
                        ),
                    )
                    .child(ui::hint(t("ui.setup.auto_advanced"), cx))
                    .into_any_element(),
            )
        } else if !plan.manual_firmware {
            Some(
                ui::banner(BannerKind::Good, Some(t("ui.setup.driver_ok").into()), None, None, None, cx)
                    .into_any_element(),
            )
        } else {
            None
        };
        let failed = d.failed.clone().map(|why| {
            let body = div()
                .flex()
                .flex_col()
                .gap(theme::S2)
                .child(div().child(why))
                .child(ui::hint(t("ui.setup.auto_advanced"), cx))
                .into_any_element();
            ui::banner(BannerKind::Error, Some(t("ui.setup.auto_failed").into()), Some(body), None, None, cx)
        });
        let manual = (d.phase == Phase::Idle && plan.manual_firmware).then(|| {
            let url = firmware_url(&a.vendor);
            let title =
                if a.firmware_invalid.is_some() { t("firmware.invalid_short") } else { t("firmware.missing_manual") };
            let actions = ui::inline()
                .child(
                    ui::button("setup-fw-page", t("firmware.open_download_page"), ButtonKind::Default)
                        .icon(icon(IconName::Open))
                        .on_click(move |_, _, cx| host::open_url(url, cx)),
                )
                .children(firmware_folder_button("setup-fw-folder", ButtonKind::Default, cx))
                .child(
                    ui::button("setup-guide", t("ui.setup.guide"), ButtonKind::Subtle)
                        .icon(icon(IconName::Open))
                        .on_click(|_, _, cx| host::open_url(LINKS.setup_guide, cx)),
                );
            let body = div()
                .flex()
                .flex_col()
                .gap(theme::S2)
                .when_some(a.firmware_invalid.clone(), |x, file| x.child(div().child(t!("firmware.invalid", &file))))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_x(theme::S1)
                        .child(t("firmware.required_files"))
                        .child(ui::mono(files.clone())),
                )
                .child(ui::hint(t("firmware.manual_instructions"), cx));
            ui::banner(
                BannerKind::Warn,
                Some(title.into()),
                Some(body.into_any_element()),
                Some(actions.into_any_element()),
                None,
                cx,
            )
        });
        div()
            .flex()
            .flex_col()
            .gap(theme::S4)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(theme::S2)
                    .px(theme::S3)
                    .py(theme::S2)
                    .border_1()
                    .border_dashed()
                    .border_color(p.border_strong)
                    .rounded(theme::RADIUS)
                    .child(adapter_line(&a, false, cx)),
            )
            .when(!adapter_supported(&a), |x| {
                x.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(theme::S1)
                        .text_color(p.bad_text)
                        .child(icon(IconName::Warning).small())
                        .child(t("ui.setup.unsupported_note")),
                )
            })
            .children(body)
            .children(failed)
            .when(d.slow && plan.needs_firmware, |x| {
                x.child(ui::banner(BannerKind::Info, Some(t("firmware.download_slow").into()), None, None, None, cx))
            })
            .when(!plan.needs_driver && a.restart_needed, |x| {
                let title = t!("ui.adapter.assigned_restart", &a.display_name);
                x.child(ui::banner(BannerKind::Warn, Some(title.into()), None, None, None, cx))
            })
            .children(manual)
    }

    // ---- Step 3: the audio devices ------------------------------------------------

    fn device_step(&self, snap: &Snapshot, window: &mut Window, cx: &mut Context<Self>) -> DeviceStep {
        let weak = cx.weak_entity();
        let on_connect: OnConnect = Rc::new(move |address: &str, name: &str, _: &mut Window, cx: &mut App| {
            let (address, name) = (address.to_owned(), name.to_owned());
            let _ = weak.update(cx, |this, cx| this.connect(address, name, cx));
        });
        // A saved device's state, as a dot: connecting it is left to the main
        // window (issue #86)
        let paired: PairedState = Rc::new(|d: &Device, cx: &App| {
            let key = device_key(&d.address, d.le_audio);
            let snap = app::core(cx).read(cx).snap();
            let state = snap
                .and_then(|s| classic_entries(s).into_iter().find(|e| e.key == key).map(|e| entry_state(s, &e)))
                .unwrap_or(StreamState::Idle);
            let label = snap.map(|s| paired_label(state, &s.stream)).unwrap_or_default();
            let (color, _) = state_colors(state, cx.palette());
            dot(color).id(SharedString::from(format!("state-{key}"))).tooltip(tip(label)).into_any_element()
        });
        let import = cx.new(ImportPairings::new);
        let picker = cx.new(|cx| DevicePicker::new(DevicePickerProps::default(), window, cx));
        DeviceStep {
            picker,
            import,
            on_connect,
            paired,
            devices: None,
            audio_key: ui::audio_devices_key(Some(snap)),
            connecting: None,
            paused: false,
            was_running: stream_running(snap.stream.state),
            stopping: false,
            given: None,
        }
    }

    /// The picker gets the step's state when it changed
    fn sync_picker(&mut self, cx: &mut Context<Self>) {
        let Some(snap) = self.snap(cx) else { return };
        let (v1, running) = (snap.v1_running, stream_running(snap.stream.state));
        let Some(d) = self.device.as_mut() else { return };
        let now = (d.connecting.clone(), v1, running, !d.paused);
        if d.given.as_ref() == Some(&now) {
            return;
        }
        d.given = Some(now);
        let props = DevicePickerProps {
            classic_only: true,
            on_connect: Some(d.on_connect.clone()),
            connecting: d.connecting.clone(),
            connect_disabled: v1,
            paired: Some(d.paired.clone()),
            scan_text: Some(t("ui.setup.scanning")),
            no_scan: running,
            children: Some(d.import.clone().into()),
            continuous: !d.paused,
            ..Default::default()
        };
        d.picker.update(cx, |p, cx| p.set_props(props, cx));
    }

    fn core_changed(&mut self, cx: &mut Context<Self>) {
        let Some(snap) = self.snap(cx) else { return };
        let (key, running, state) =
            (ui::audio_devices_key(Some(snap)), stream_running(snap.stream.state), snap.stream.state);
        let Some(d) = self.device.as_mut() else { return };
        if scan_resumes(d.was_running, running, state) {
            d.paused = false;
        }
        d.was_running = running;
        let reload = key != d.audio_key;
        d.audio_key = key;
        if reload {
            self.load_devices(cx);
        }
        self.sync_picker(cx);
    }

    fn load_devices(&mut self, cx: &mut Context<Self>) {
        let task = app::request::<Vec<AudioDevice>>(Method::AudioDevices, cx);
        let visit = self.visit;
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            if let Ok(list) = task.await {
                let _ = this.update(cx, |this, cx| {
                    if this.visit == visit
                        && let Some(d) = this.device.as_mut()
                    {
                        d.devices = Some(list);
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    /// One click: the device is saved, gets its "<name> | Auto" profile (the
    /// best codec it has) and is connected with it
    fn connect(&mut self, address: String, name: String, cx: &mut Context<Self>) {
        let v1 = self.snap(cx).is_some_and(|s| s.v1_running);
        let visit = self.visit;
        let Some(d) = self.device.as_mut() else { return };
        if d.connecting.is_some() || v1 {
            return;
        }
        d.connecting = Some(address.clone());
        d.paused = true;
        // Windows' outputs not read yet (a click as the step opens): read
        // first, or the device would start without its cable (issue #86)
        let audio = d.devices.as_deref().map(SetupAudio::of);
        let load = audio.is_none().then(|| app::request::<Vec<AudioDevice>>(Method::AudioDevices, cx));
        self.sync_picker(cx);
        cx.notify();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let audio = match load {
                Some(task) => {
                    let list = task.await.ok();
                    let audio = list.as_deref().map(SetupAudio::of);
                    let _ = this.update(cx, |this, _| {
                        if this.visit == visit
                            && let Some(d) = this.device.as_mut()
                        {
                            d.devices = list;
                        }
                    });
                    audio
                }
                None => audio,
            };
            let save = cx.update(|cx| save_with_profile(address, name, false, audio, cx));
            let ok = match save.await {
                Some(profile) => start_stream(profile, cx).await,
                None => false,
            };
            let _ = this.update(cx, |this, cx| {
                if this.visit != visit {
                    return;
                }
                if let Some(d) = this.device.as_mut() {
                    if !ok {
                        d.paused = false;
                    }
                    d.connecting = None;
                }
                this.sync_picker(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Add a device: the one connected makes room, the scan resumes
    fn disconnect(&mut self, cx: &mut Context<Self>) {
        let running = self.snap(cx).is_some_and(|s| stream_running(s.stream.state));
        let Some(d) = self.device.as_mut() else { return };
        d.stopping = true;
        cx.notify();
        let stop = running.then(|| app::run(app::request::<serde_json::Value>(Method::StreamStop, cx), cx));
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            if let Some(stop) = stop {
                stop.await;
            }
            let _ = this.update(cx, |this, cx| {
                if let Some(d) = this.device.as_mut() {
                    d.stopping = false;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn step_device(&self, snap: &Snapshot, cx: &mut Context<Self>) -> Div {
        let Some(d) = self.device.as_ref() else { return div() };
        let weak = cx.entity().downgrade();
        div()
            .flex()
            .flex_col()
            .gap(theme::S4)
            // At the top, above the paired devices, as on Devices (issue #86)
            .child(cable_notes(d.devices.as_deref(), move |cx| {
                let _ = weak.update(cx, |this, cx| this.load_devices(cx));
            }))
            .child(d.picker.clone())
            .when(stream_running(snap.stream.state), |x| {
                x.child(
                    div().flex().child(
                        ui::button("setup-add-device", t("ui.home.add_device"), ButtonKind::Default)
                            .icon(icon(IconName::Add))
                            .loading(d.stopping)
                            .disabled(d.stopping)
                            .on_click(cx.listener(|this, _, _, cx| this.disconnect(cx))),
                    ),
                )
            })
    }

    // ---- Step 4: the general settings ---------------------------------------------

    /// The general settings worth asking at the start (issue #86): the same
    /// controls as on Settings, saved at once
    fn step_finish(&self, snap: &Snapshot, cx: &App) -> Div {
        let s = &snap.settings;
        let h3 = |key: &str| div().font_family(theme::FONT_DISPLAY).font_weight(FontWeight::SEMIBOLD).child(t(key));
        div()
            .flex()
            .flex_col()
            .gap(theme::S4)
            .child(ui::first_row(theme_choice(s, cx)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(theme::S3)
                    .child(h3("ui.settings.startup"))
                    .child(ui::toggle(
                        "setup-start-with-windows",
                        t("settings.start_with_windows"),
                        None,
                        s.start_with_windows,
                        false,
                        |v, _, cx| {
                            let v = *v;
                            app::save_settings(move |s| s.start_with_windows = v, cx);
                        },
                        cx,
                    ))
                    .child(ui::toggle(
                        "setup-start-minimized",
                        t("ui.settings.start_minimized"),
                        Some(t("ui.settings.start_minimized_logon").into()),
                        s.start_minimized,
                        !s.start_with_windows,
                        |v, _, cx| {
                            let v = *v;
                            app::save_settings(move |s| s.start_minimized = v, cx);
                        },
                        cx,
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(theme::S3)
                    .child(h3("ui.settings.notifications"))
                    .child(ui::toggle(
                        "setup-notify-connected",
                        t("ui.settings.notify_connected"),
                        None,
                        s.notify_connected,
                        false,
                        |v, _, cx| {
                            let v = *v;
                            app::save_settings(move |s| s.notify_connected = v, cx);
                        },
                        cx,
                    ))
                    .child(battery_low_choice(s, cx)),
            )
            .child(ui::toggle(
                "setup-check-updates",
                t("settings.check_updates_on_startup"),
                None,
                s.check_updates_on_startup,
                false,
                |v, _, cx| {
                    let v = *v;
                    app::save_settings(move |s| s.check_updates_on_startup = v, cx);
                },
                cx,
            ))
    }

    // ---- The frame ------------------------------------------------------------------

    fn stepper(&self, cx: &App) -> Div {
        let p = *cx.palette();
        div().flex().gap(theme::S2).children(STEP_KEYS.iter().enumerate().map(|(i, key)| {
            let (on, done) = (i == self.step, i < self.step);
            let (line, color) = if on {
                (p.accent_fill, p.text)
            } else if done {
                (p.text_2, p.text_2)
            } else {
                (p.border, p.text_3)
            };
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap(theme::S2)
                .pt(theme::S2)
                .border_t_2()
                .border_color(line)
                .text_color(color)
                .text_size(theme::FS_SMALL)
                .when(on, |d| d.font_weight(FontWeight::SEMIBOLD))
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .size(px(20.))
                        .rounded_full()
                        .border_1()
                        .border_color(color)
                        .map(|d| {
                            if done {
                                d.child(icon(IconName::Checkmark).small())
                            } else {
                                d.child(format!("{}", i + 1))
                            }
                        }),
                )
                .child(div().min_w_0().child(t(key)))
        }))
    }

    fn footer(&self, can_next: bool, cx: &mut Context<Self>) -> Div {
        let step = self.step;
        div()
            .flex()
            .items_center()
            .gap(theme::S2)
            .when(step < LAST, |d| {
                d.child(
                    ui::button("setup-skip", t("ui.setup.skip"), ButtonKind::Subtle)
                        .on_click(cx.listener(|this, _, _, cx| this.finish(cx))),
                )
            })
            .child(div().flex_1())
            .when(step > 0, |d| {
                d.child(
                    ui::button("setup-back", t("ui.setup.back"), ButtonKind::Default)
                        .icon(icon(IconName::ArrowLeft))
                        .on_click(cx.listener(move |this, _, window, cx| this.go(step - 1, window, cx))),
                )
            })
            .map(|d| {
                if step < LAST {
                    d.child(
                        ui::button("setup-next", t("ui.setup.next"), ButtonKind::Strong)
                            .icon(icon(IconName::ArrowRight))
                            .disabled(!can_next)
                            .on_click(cx.listener(move |this, _, window, cx| this.go(step + 1, window, cx))),
                    )
                } else {
                    d.child(
                        ui::button("setup-finish", t("ui.setup.finish"), ButtonKind::Strong)
                            .icon(icon(IconName::Checkmark))
                            .on_click(cx.listener(|this, _, _, cx| this.finish(cx))),
                    )
                }
            })
    }
}

/// The adapter's name and what BlueCaster can do with it (issue #86): Device
/// Manager's name with the chip as the title, its IDs and the driver below,
/// and a green "Supported" or red "Not supported" tag
fn adapter_line(a: &AdapterInfo, dim: bool, cx: &App) -> Div {
    let p = cx.palette();
    let ok = adapter_supported(a);
    let ids = format!("{} · {}", adapter_ids(a), driver_text(a.driver).text);
    let ids_line = div().id(ElementId::Name(format!("ids-{}", a.instance_id).into())).text_color(p.text_2).child(ids);
    let ids_line = match adapter_ids_hint(a) {
        Some(hint) => ids_line.tooltip(tip(hint)).into_any_element(),
        None => ids_line.into_any_element(),
    };
    div()
        .flex()
        .flex_1()
        .min_w_0()
        .items_center()
        .gap(theme::S3)
        .child(div().flex_none().child(icon(IconName::UsbStick)))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .when(dim, |d| d.opacity(0.6))
                .child(div().font_weight(FontWeight::SEMIBOLD).child(adapter_name(a)))
                .child(ids_line),
        )
        .child(tag(
            ElementId::Name(format!("support-{}", a.instance_id).into()),
            t(if ok { "ui.setup.supported" } else { "ui.adapter.firmware_unsupported" }),
            Some(if ok { TagStatus::Good } else { TagStatus::Bad }),
            None,
            cx,
        ))
}

/// Sets the profile active and starts the stream once no scan runs (a
/// connection waits for the scan to end) -> it worked; a failure is a notice
async fn start_stream(profile: String, cx: &mut AsyncApp) -> bool {
    let request = |method: Method, cx: &mut AsyncApp| cx.update(|cx| app::request::<serde_json::Value>(method, cx));
    let fail = |e: ErrorInfo, cx: &mut AsyncApp| {
        cx.update(|cx| app::notify(app::error_text(&e), app::NoticeKind::Error, cx));
        false
    };
    if let Err(e) = request(Method::ProfileSetActive { name: profile.clone() }, cx).await {
        return fail(e, cx);
    }
    let scanning = |cx: &mut AsyncApp| cx.update(|cx| app::core(cx).read(cx).snap().is_some_and(|s| s.scanning));
    // The stream waits for the scan to end
    if scanning(cx)
        && let Err(e) = request(Method::DeviceScanStop, cx).await
    {
        return fail(e, cx);
    }
    while scanning(cx) {
        cx.background_executor().timer(Duration::from_millis(200)).await;
    }
    match request(Method::StreamStart { profile }, cx).await {
        Ok(_) => true,
        Err(e) => fail(e, cx),
    }
}

impl Render for Setup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(snap) = self.snap(cx).cloned() else { return div().into_any_element() };
        let p = *cx.palette();
        let adapter = self.adapter(cx);
        let next = can_next(self.step, snap.v1_running, adapter.as_ref());
        let body = match self.step {
            0 => self.step_adapter(&snap, cx),
            1 => self.step_driver(adapter, snap.v1_running, cx),
            DEVICE_STEP => self.step_device(&snap, cx),
            _ => self.step_finish(&snap, cx),
        };
        div()
            .id("setup-wrap")
            .size_full()
            .bg(p.bg)
            .overflow_y_scrollbar()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(theme::S5)
                    .w_full()
                    .max_w(px(704.))
                    .mx_auto()
                    .px(theme::S6)
                    .py(theme::S8)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(theme::S4)
                            .child(img(self.icon.clone()).size(px(32.)).flex_none())
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .font_family(theme::FONT_DISPLAY)
                                            .text_size(theme::FS_TITLE)
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(t("ui.setup.wizard_title")),
                                    )
                                    .child(ui::muted(t("ui.setup.intro"), cx)),
                            ),
                    )
                    .children(v1_running(cx))
                    .child(self.stepper(cx))
                    .child(
                        ui::card(cx)
                            .gap(theme::S4)
                            .p(theme::S5)
                            .when(heading_shown(self.step), |d| {
                                d.child(
                                    div()
                                        .font_family(theme::FONT_DISPLAY)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(t(HEADING_KEYS[self.step])),
                                )
                            })
                            .child(body),
                    )
                    .child(self.footer(next, cx)),
            )
            .into_any_element()
    }
}
