//! Picking the headphones: the saved devices and the ones a scan found
//! (device.scan), or an address typed by hand; a saved device can be renamed
//! or removed from the list (v1 OnDeviceEditName / OnDeviceDelete). The
//! first-run setup connects one with a button on its row instead. With LE
//! Audio (experimental) on, the LE Audio entries too: a dual-mode device is
//! listed once, as its LE Audio entry (issue #86; ui_model::profile
//! listed_devices), and adding, renaming or forgetting it does both sides.
//! Once one is picked, the others are folded away until asked
//! for or a new scan. Used by the Audio Devices screen and the first-run setup.
//!
//! Which devices are listed and in which order is worked out in
//! ui_model::view::devices.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::radio::Radio;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AnyView, App, AppContext as _, AsyncApp, AsyncWindowContext, Context, Div, Entity, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Styled as _, Task, WeakEntity,
    Window, div, px,
};
use ipc::{AudioDevice, Device, Method, Snapshot, StreamState, WindowsPairedDevice};
use ui_model::format::parse_address;
use ui_model::i18n::t;
use ui_model::profile::{
    default_profile_name, device_key, device_profile, entry_base, new_profile, normalize_profile, setup_audio, twin_of,
    valid_device_name,
};
use ui_model::t;
use ui_model::view::devices::{
    SCAN_SECONDS, SETUP_SCAN_SECONDS, device_label, free_name, in_use, in_use_name, kind_tag, managed_saved,
    paired_list, picker_devices, picker_head_text, rename_hint, saved_name, saved_with, windows_paired, winusb_adapter,
};

use super::device_settings::DeviceSettings;
use super::dialog::{self, Question};
use super::{ButtonKind, IconName, button, icon, icon_button};
use crate::app::{self, RouteName};
use crate::core::Core;
use crate::theme::{self, ActivePalette as _};

// ---- Saving a device with its profile ---------------------------------------

/// The source and microphone a new device gets ([`setup_audio`]), held
/// across requests
#[derive(Debug, Clone, Default)]
pub struct SetupAudio {
    pub sound: Option<AudioDevice>,
    pub mic: Option<AudioDevice>,
}

impl SetupAudio {
    pub fn of(devices: &[AudioDevice]) -> SetupAudio {
        let a = setup_audio(devices);
        SetupAudio { sound: a.sound.cloned(), mic: a.mic.cloned() }
    }
}

fn snap_of(cx: &mut AsyncApp) -> Option<Snapshot> {
    cx.update(|cx| app::core(cx).read(cx).snap().cloned())
}

/// Runs a request as a button does (its error as a notice) -> it worked
async fn run_ok(method: Method, cx: &mut AsyncApp) -> bool {
    let task = cx.update(|cx| app::run(app::request::<serde_json::Value>(method, cx), cx));
    task.await.is_some()
}

/// Saves a device found by a scan or typed (the list keeps its name; an
/// unnamed one is saved under its address and can be renamed). A device
/// without a profile gets one that picks the best codec ("<name> | Auto").
/// A dual-mode device is saved with both sides, each with its profile
/// (issue #86: one device, one Add; "Use LE Audio when available" decides
/// which connects). The profile of the side added, or None when saving
/// failed. `audio`: the source and microphone for a new classic profile
/// ([`SetupAudio`]).
pub fn save_with_profile(
    address: String,
    name: String,
    le_audio: bool,
    audio: Option<SetupAudio>,
    cx: &App,
) -> Task<Option<String>> {
    cx.spawn(async move |cx: &mut AsyncApp| {
        let s = snap_of(cx)?;
        let twin = twin_of(&s.devices, &address, le_audio).is_some();
        let saved = saved_name(&s.devices, &address, &name, le_audio);
        for le in std::iter::once(le_audio).chain(twin.then_some(!le_audio)) {
            let s = snap_of(cx)?;
            // A saved device typed by its address keeps its name
            if !saved_with(&s.devices, &address, le, &saved) {
                let rename = Method::DeviceRename {
                    address: address.clone(),
                    name: saved.clone(),
                    le_audio: le.then_some(true),
                };
                if !run_ok(rename, cx).await {
                    return None;
                }
            }
        }
        let classic =
            if !le_audio || twin { classic_profile(&address, &saved, audio.as_ref(), cx).await } else { None };
        if !le_audio {
            return classic;
        }
        // The LE Audio side's own, with the device settings of the classic one
        let s = snap_of(cx)?;
        if let Some(own) = device_profile((&s).into(), &address, true) {
            return Some(own.name.clone());
        }
        let taken: Vec<&str> = s.profiles.iter().map(|p| p.name.as_str()).collect();
        let profile_name = free_name(&taken, &default_profile_name(&saved, &address, true, ""));
        let mut profile = entry_base(&address, &saved, true, &s.profiles);
        // A hearing aid streams through its own session, not LE Audio's
        profile.hearing_aid =
            s.devices.iter().any(|d| d.le_audio && d.hearing_aid && d.address.eq_ignore_ascii_case(&address));
        profile.name = profile_name.clone();
        let save = Method::ProfileSave { profile: Box::new(normalize_profile(&profile)), original_name: None };
        run_ok(save, cx).await.then_some(profile_name)
    })
}

/// The classic side's profile, made when it has none
async fn classic_profile(address: &str, saved: &str, audio: Option<&SetupAudio>, cx: &mut AsyncApp) -> Option<String> {
    let s = snap_of(cx)?;
    if let Some(own) = device_profile((&s).into(), address, false) {
        return Some(own.name.clone());
    }
    let taken: Vec<&str> = s.profiles.iter().map(|p| p.name.as_str()).collect();
    let profile_name = free_name(&taken, &default_profile_name(saved, address, false, &t("codec.auto")));
    let mut profile = new_profile();
    // Captured by System Loopback, as every new device (issue #86), unless
    // the setup gives it a cable or an output; its microphone on Auto where
    // it gives a cable, off on a sound card
    if let Some(sound) = audio.and_then(|a| a.sound.as_ref()) {
        profile.capture_mode = if sound.is_virtual { "virtual" } else { "output" }.into();
        profile.audio_device_id = sound.id.clone();
        profile.audio_device_name = sound.name.clone();
    }
    if let Some(mic) = audio.and_then(|a| a.mic.as_ref()) {
        profile.headset_mic = if mic.is_virtual { "auto" } else { "off" }.into();
        profile.mic_device_id = mic.id.clone();
        profile.mic_device_name = mic.name.clone();
    }
    profile.name = profile_name.clone();
    profile.device_address = address.to_owned();
    profile.device_name = saved.to_owned();
    let save = Method::ProfileSave { profile: Box::new(normalize_profile(&profile)), original_name: None };
    run_ok(save, cx).await.then_some(profile_name)
}

// A dialog that waits for its request (moved to ui/dialog.rs)
pub use super::dialog::ask_busy;

/// A tag (`.tag`): a small rounded label
pub fn tag(text: impl Into<SharedString>, cx: &App) -> Div {
    let p = *cx.palette();
    div()
        .flex()
        .flex_none()
        .items_center()
        .px(theme::S2)
        .min_h(px(24.))
        .border_1()
        .border_color(p.border_strong)
        .rounded_full()
        .text_size(theme::FS_SMALL)
        .text_color(p.text_2)
        .whitespace_nowrap()
        .child(text.into())
}

// ---- The picker --------------------------------------------------------------

pub type OnPick = Rc<dyn Fn(&str, &str, bool, &mut Window, &mut App)>;
pub type OnConnect = Rc<dyn Fn(&str, &str, &mut Window, &mut App)>;
pub type PairedState = Rc<dyn Fn(&Device, &App) -> AnyElement>;
/// What runs once Windows' pairing is not in the way
type Go = Rc<dyn Fn(&mut Window, &mut App)>;

/// What the owner sets; [`DevicePicker::set_props`] changes it
#[derive(Clone, Default)]
pub struct DevicePickerProps {
    pub address: String,
    /// The chosen entry is the LE Audio one
    pub le_audio: bool,
    pub on_pick: Option<OnPick>,
    pub error: Option<String>,
    /// Classic entries only (the first-run setup makes an A2DP profile)
    pub classic_only: bool,
    /// The Audio Devices screen: no choosing; scan, add, rename and forget
    pub manage: bool,
    /// The profile editor: the saved devices only, to choose from; they are
    /// managed on the Audio Devices screen
    pub saved_only: bool,
    /// With manage: a saved device's own settings ([`DeviceSettings`]) below its row
    pub settings: bool,
    /// The first-run setup: no choosing; Connect on each row and Use on the
    /// typed address
    pub on_connect: Option<OnConnect>,
    /// With on_connect: the address being connected (its button spins, the
    /// others wait)
    pub connecting: Option<String>,
    /// With on_connect: no connection now (another BlueCaster holds the adapter)
    pub connect_disabled: bool,
    /// With on_connect: the saved devices stand in a list of their own, each
    /// with its state before its icon (issue #86)
    pub paired: Option<PairedState>,
    /// With paired: what the scan shows while it runs
    pub scan_text: Option<String>,
    /// With paired: no scan part now (a device is connected)
    pub no_scan: bool,
    /// With paired: shown between the saved devices and the scan
    pub children: Option<AnyView>,
    /// The first-run setup: the scan starts by itself and goes on until a
    /// device connects, Stop is pressed or the step is left (issue #86)
    pub continuous: bool,
}

pub struct DevicePicker {
    core: Entity<Core>,
    props: DevicePickerProps,
    /// Picked here: the list shows the chosen entry only
    folded: bool,
    manual: Entity<InputState>,
    /// Stop pressed: a continuous scan ends there
    stopped: bool,
    disconnecting: bool,
    busy: bool,
    // Rename: the device and the name being typed
    renaming: Option<Device>,
    new_name: Entity<InputState>,
    rename_tried: bool,
    /// The saved devices' settings, one view per device (issue #86: the
    /// settings of one device showed on, and were saved into, the next one
    /// once the core's list changed order)
    settings: HashMap<String, Entity<DeviceSettings>>,
}

impl DevicePicker {
    pub fn new(props: DevicePickerProps, window: &mut Window, cx: &mut Context<Self>) -> DevicePicker {
        let core = app::core(cx);
        cx.observe(&core, |_, _, cx| cx.notify()).detach();
        let manual = cx.new(|cx| InputState::new(window, cx).placeholder("AA:BB:CC:DD:EE:FF"));
        cx.subscribe_in(&manual, window, |_, _, e: &InputEvent, _, cx| {
            if matches!(e, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        // 64 at most, counted as the old UI's maxLength did (UTF-16 units)
        let new_name = cx.new(|cx| InputState::new(window, cx).validate(|s, _| s.encode_utf16().count() <= 64));
        cx.subscribe_in(&new_name, window, |this: &mut Self, _, e: &InputEvent, window, cx| match e {
            InputEvent::Change => cx.notify(),
            InputEvent::PressEnter { .. } => this.rename(window, cx),
            _ => {}
        })
        .detach();
        // The picker gone: a continuous scan ends there
        cx.on_release(|this: &mut Self, cx| {
            let scanning = app::core(cx).read(cx).snap().is_some_and(|s| s.scanning);
            if this.props.continuous && scanning {
                app::request::<serde_json::Value>(Method::DeviceScanStop, cx).detach();
            }
        })
        .detach();
        let start = props.continuous && !core.read(cx).snap().is_some_and(|s| s.scanning);
        let mut this = DevicePicker {
            core,
            props,
            folded: false,
            manual,
            stopped: false,
            disconnecting: false,
            busy: false,
            renaming: None,
            new_name,
            rename_tried: false,
            settings: HashMap::new(),
        };
        if start {
            this.scan(cx);
        }
        this
    }

    pub fn set_props(&mut self, props: DevicePickerProps, cx: &mut Context<Self>) {
        let before = self.props.continuous;
        self.props = props;
        // Continuous again (the setup's device disconnected to choose another): the scan resumes
        if self.props.continuous && !before && !self.scanning(cx) {
            self.scan(cx);
        }
        cx.notify();
    }

    fn scanning(&self, cx: &App) -> bool {
        self.core.read(cx).snap().is_some_and(|s| s.scanning)
    }

    fn chosen(&self, d: &Device) -> bool {
        d.address == self.props.address && d.le_audio == self.props.le_audio
    }

    fn pick(&mut self, address: &str, name: &str, le_audio: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.folded = true;
        cx.notify();
        if let Some(pick) = self.props.on_pick.clone() {
            let (address, name) = (address.to_owned(), name.to_owned());
            // After this update: the owner may set the props again
            window.defer(cx, move |window, cx| pick(&address, &name, le_audio, window, cx));
        }
    }

    fn scan(&mut self, cx: &mut Context<Self>) {
        self.folded = false;
        self.stopped = false;
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            loop {
                let Ok(task) = this.update(cx, |this, cx| {
                    let seconds = if this.props.continuous { SETUP_SCAN_SECONDS } else { SCAN_SECONDS };
                    app::run(app::request::<serde_json::Value>(Method::DeviceScan { seconds }, cx), cx)
                }) else {
                    return;
                };
                let ok = task.await.is_some();
                // A refused scan (a device streams) is not tried again
                let again = this
                    .update(cx, |this, _| {
                        ok && this.props.continuous && !this.stopped && this.props.connecting.is_none()
                    })
                    .unwrap_or(false);
                if !again {
                    return;
                }
            }
        })
        .detach();
    }

    /// The devices show up as they answer: the user stops once theirs is there
    fn stop_scan(&mut self, cx: &mut Context<Self>) {
        self.stopped = true;
        app::send(Method::DeviceScanStop, cx);
    }

    fn scan_clicked(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(snap) = self.core.read(cx).snap() else { return };
        if snap.scanning {
            self.stop_scan(cx);
        } else if in_use(snap.stream.state) {
            // A device in use: the scan asks to disconnect it first (issue #86)
            let text = t!("ui.devices.scan_disconnect", in_use_name(snap));
            let (weak, weak_busy) = (cx.entity().downgrade(), cx.entity().downgrade());
            ask_busy(
                Question::new(t("modal.confirm_title")).confirm(t("connection.disconnect")),
                move |cx| weak_busy.upgrade().is_some_and(|e| e.read(cx).disconnecting),
                move |_, _| div().child(text.clone()).into_any_element(),
                move |window, cx| {
                    let _ = weak.update(cx, |this, cx| this.disconnect_and_scan(window, cx));
                },
                window,
                cx,
            );
        } else {
            self.scan(cx);
        }
    }

    fn disconnect_and_scan(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.disconnecting {
            return;
        }
        self.disconnecting = true;
        cx.notify();
        let stop = app::run(app::request::<serde_json::Value>(Method::StreamStop, cx), cx);
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
            stop.await;
            loop {
                let busy = cx
                    .update(|_, cx| app::core(cx).read(cx).snap().is_some_and(|s| in_use(s.stream.state)))
                    .unwrap_or(false);
                if !busy {
                    break;
                }
                cx.background_executor().timer(Duration::from_millis(100)).await;
            }
            let _ = cx.update(|window, cx| {
                window.close_dialog(cx);
                let _ = this.update(cx, |this, cx| {
                    this.disconnecting = false;
                    this.scan(cx);
                    cx.notify();
                });
            });
        })
        .detach();
    }

    fn open_rename(&mut self, d: &Device, window: &mut Window, cx: &mut Context<Self>) {
        self.renaming = Some(d.clone());
        self.rename_tried = false;
        // A name longer than the field takes (a device's own can be) is cut
        // to what fits: the field refuses a longer one and would stay empty
        let mut units = 0;
        let name: String = d
            .name
            .chars()
            .take_while(|c| {
                units += c.len_utf16();
                units <= 64
            })
            .collect();
        self.new_name.update(cx, |s, cx| {
            s.set_value(name, window, cx);
        });
        let (weak, weak_body, weak_confirm) =
            (cx.entity().downgrade(), cx.entity().downgrade(), cx.entity().downgrade());
        let hint = rename_hint(d);
        ask_busy(
            Question::new(t("device.edit_name_title")).confirm(t("modal.save")),
            move |cx| weak.upgrade().is_some_and(|e| e.read(cx).busy),
            move |_, cx| {
                let Some(this) = weak_body.upgrade() else { return div().into_any_element() };
                let this = this.read(cx);
                let value = this.new_name.read(cx).value();
                let error = (this.rename_tried && !valid_device_name(&value)).then(|| t("ui.device.err_name").into());
                super::field_stacked(
                    t("device.enter_name"),
                    Some(hint.clone().into()),
                    error,
                    Input::new(&this.new_name).aria_label(t("device.enter_name")),
                    cx,
                )
                .into_any_element()
            },
            move |window, cx| {
                let _ = weak_confirm.update(cx, |this, cx| this.rename(window, cx));
            },
            window,
            cx,
        );
        self.new_name.update(cx, |s, cx| s.focus(window, cx));
    }

    fn rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = self.renaming.clone() else { return };
        let n = self.new_name.read(cx).value().trim().to_owned();
        self.rename_tried = true;
        cx.notify();
        if !valid_device_name(&n) || self.busy {
            return;
        }
        self.busy = true;
        // Both sides of a dual-mode device (issue #86)
        let twin = self
            .core
            .read(cx)
            .snap()
            .and_then(|s| twin_of(&s.devices, &d.address, d.le_audio).filter(|t| t.saved).cloned());
        let named = |x: &Device, cx: &App| {
            app::run(
                app::request::<serde_json::Value>(
                    Method::DeviceRename {
                        address: d.address.clone(),
                        name: n.clone(),
                        le_audio: x.le_audio.then_some(true),
                    },
                    cx,
                ),
                cx,
            )
        };
        let first = named(&d, cx);
        let second = twin.map(|t| named(&t, cx));
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
            let mut ok = first.await.is_some();
            if ok && let Some(second) = second {
                ok = second.await.is_some();
            }
            let _ = cx.update(|window, cx| {
                let Ok(pick) = this.update(cx, |this, cx| {
                    this.busy = false;
                    cx.notify();
                    if !ok {
                        return None;
                    }
                    this.renaming = None;
                    // The profile shows the new name (v1 set the name field too)
                    if this.chosen(&d) { this.props.on_pick.clone() } else { None }
                }) else {
                    return;
                };
                if ok {
                    window.close_dialog(cx);
                }
                if let Some(pick) = pick {
                    pick(&d.address, &n, d.le_audio, window, cx);
                }
            });
        })
        .detach();
    }

    /// A device without a profile gets one and is connected with it right
    /// away (pairing it, as other systems do); the user makes more. A running
    /// stream and the first-run setup (it connects the device itself) are left
    /// alone.
    fn add(
        &mut self,
        address: String,
        name: String,
        le_audio: bool,
        clear: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        let Some(snap) = self.core.read(cx).snap() else { return };
        let had = snap.profiles.iter().any(|p| p.device_address.eq_ignore_ascii_case(&address));
        self.busy = true;
        cx.notify();
        // Its source and microphone as the first-run setup gives them (issue #86)
        let devices = (!had).then(|| app::request::<Vec<AudioDevice>>(Method::AudioDevices, cx));
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
            let audio = match devices {
                Some(task) => task.await.ok().map(|d| SetupAudio::of(&d)),
                None => None,
            };
            let Ok(save) = cx.update(|_, cx| save_with_profile(address, name, le_audio, audio, cx)) else { return };
            let profile = save.await;
            let start = cx.update(|_, cx| {
                let state = app::core(cx).read(cx).snap().map(|s| s.stream.state);
                let setup = app::state(cx).read(cx).setup_active;
                match profile {
                    Some(profile)
                        if !had && !setup && matches!(state, Some(StreamState::Idle | StreamState::Error)) =>
                    {
                        Some(app::run(app::request::<serde_json::Value>(Method::StreamStart { profile }, cx), cx))
                    }
                    _ => None,
                }
            });
            if let Ok(Some(start)) = start {
                start.await;
            }
            let _ = cx.update(|window, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.busy = false;
                    if clear {
                        this.manual.update(cx, |s, cx| s.set_value("", window, cx));
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }

    /// A device Windows' Bluetooth has paired too may be taken by both: a
    /// warning first (issue #86; importing Windows' pairings is the setup's
    /// job). LE Audio entries and a list Windows cannot give pass.
    fn unless_windows_paired(
        &mut self,
        address: String,
        name: String,
        le_audio: bool,
        go: Go,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // `go` updates this picker: once this update is over (as the paths
        // below, which ask the core first), or the app stops on a picker
        // updated twice
        if le_audio {
            return window.defer(cx, move |window, cx| go(window, cx));
        }
        let adapter = self.core.read(cx).snap().and_then(winusb_adapter);
        let through = adapter.map(|adapter| {
            app::request::<Option<Vec<WindowsPairedDevice>>>(Method::DeviceWindowsPairedThrough { adapter }, cx)
        });
        cx.spawn_in(window, async move |_, cx: &mut AsyncWindowContext| {
            let through: Vec<String> = match through {
                Some(task) => task.await.ok().flatten().unwrap_or_default().into_iter().map(|d| d.address).collect(),
                None => Vec::new(),
            };
            let paired = if windows_paired(&through, &address) {
                through
            } else {
                let Ok(task) = cx.update(|_, cx| app::request::<Option<Vec<String>>>(Method::DeviceWindowsPaired, cx))
                else {
                    return;
                };
                task.await.ok().flatten().unwrap_or_default()
            };
            let _ = cx.update(|window, cx| {
                if !windows_paired(&paired, &address) {
                    return go(window, cx);
                }
                let text = t!("ui.devices.windows_paired_text", if name.is_empty() { &address } else { &name });
                let go = go.clone();
                dialog::ask(
                    Question::new(t("ui.devices.windows_paired_title")).confirm(t("ui.devices.add_anyway")),
                    move |_, _| div().child(text.clone()).into_any_element(),
                    move |window, cx| go(window, cx),
                    window,
                    cx,
                );
            });
        })
        .detach();
    }

    fn open_forget(&mut self, d: &Device, window: &mut Window, cx: &mut Context<Self>) {
        let (weak, weak_confirm) = (cx.entity().downgrade(), cx.entity().downgrade());
        let device = d.clone();
        let shown = d.clone();
        ask_busy(
            Question::new(t("modal.confirm_title")).confirm(t("ui.device.forget")).danger(),
            move |cx| weak.upgrade().is_some_and(|e| e.read(cx).busy),
            move |_, cx| {
                let p = *cx.palette();
                div()
                    .flex()
                    .flex_col()
                    .gap(theme::S2)
                    .child(div().child(t("ui.device.forget_confirm")))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(theme::S2)
                            .child(
                                div().min_w_0().max_w_full().font_weight(FontWeight::BOLD).child(device_label(&shown)),
                            )
                            .child(
                                div().font_family(theme::FONT_MONO).text_color(p.text_2).child(shown.address.clone()),
                            )
                            .when(shown.le_audio, |d| d.child(tag(t(ui_model::view::devices::le_tag(&shown)), cx))),
                    )
                    .into_any_element()
            },
            move |window, cx| {
                let device = device.clone();
                let _ = weak_confirm.update(cx, |this, cx| this.forget(device, window, cx));
            },
            window,
            cx,
        );
    }

    fn forget(&mut self, d: Device, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        cx.notify();
        // Both sides of a dual-mode device (issue #86)
        let twin = self
            .core
            .read(cx)
            .snap()
            .and_then(|s| twin_of(&s.devices, &d.address, d.le_audio).filter(|t| t.saved).cloned());
        let address = d.address.clone();
        let forget = move |le: bool, cx: &App| {
            app::run(
                app::request::<serde_json::Value>(
                    Method::DeviceForget { address: address.clone(), le_audio: le.then_some(true) },
                    cx,
                ),
                cx,
            )
        };
        let first = forget(d.le_audio, cx);
        let le_twin = twin.map(|t| t.le_audio);
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
            first.await;
            if let Some(le) = le_twin
                && let Ok(task) = cx.update(|_, cx| forget(le, cx))
            {
                task.await;
            }
            let _ = cx.update(|window, cx| {
                window.close_dialog(cx);
                let _ = this.update(cx, |this, cx| {
                    this.busy = false;
                    cx.notify();
                });
            });
        })
        .detach();
    }

    // ---- Drawing ----------------------------------------------------------------

    /// One device's row; on the Audio Devices screen a saved one has its
    /// settings below it
    fn row(&self, d: &Device, prefix: &str, cx: &mut Context<Self>) -> AnyElement {
        let p = *cx.palette();
        let key = device_key(&d.address, d.le_audio);
        let label = device_label(d);
        let paired = self.props.paired.as_ref().filter(|_| d.saved).map(|f| f(d, cx));
        let has_paired = paired.is_some();
        let chosen = self.chosen(d);
        let picking = !self.props.manage && self.props.on_connect.is_none();
        let mut addresses = vec![d.address.clone()];
        // Every address of a set: both earbuds'
        addresses.extend(d.set_addresses.iter().cloned());
        let text = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .child(div().font_weight(FontWeight::SEMIBOLD).line_clamp(2).text_ellipsis().child(label.clone()))
            .child(
                div()
                    .font_family(theme::FONT_MONO)
                    .text_size(theme::FS_SMALL)
                    .text_color(p.text_2)
                    .child(addresses.join(", ")),
            );
        let services = {
            let d = d.clone();
            Button::new("services")
                .label(t("ui.pairing.services"))
                .small()
                .outline()
                .rounded(px(999.))
                .h(px(24.))
                .text_size(theme::FS_SMALL)
                .on_click(move |_, window, cx| super::pairing_dialog::open(&d, window, cx))
        };
        let tags = div()
            .flex()
            .flex_wrap()
            .justify_end()
            .items_center()
            .gap(theme::S1)
            .when(d.saved && !self.props.saved_only, |x| x.child(services))
            .when(d.saved, |x| x.child(tag(t("device.tag_saved"), cx)))
            .when_some(kind_tag(d), |x, k| x.child(tag(k, cx)))
            .when_some(d.rssi.filter(|_| !has_paired), |x, rssi| x.child(tag(format!("{rssi} dBm"), cx)));
        let actions = div().flex().items_center().gap(theme::S1);
        let actions = if let (Some(connect), false) = (self.props.on_connect.clone(), has_paired) {
            let (a, n, le) = (d.address.clone(), d.name.clone(), d.le_audio);
            actions.child(
                button(
                    "connect",
                    t("connection.connect"),
                    if d.audio { ButtonKind::Strong } else { ButtonKind::Default },
                )
                .icon(icon(IconName::Power))
                .loading(self.props.connecting.as_deref() == Some(d.address.as_str()))
                .disabled(self.props.connecting.is_some() || self.props.connect_disabled)
                .on_click(cx.listener(move |this, _, window, cx| {
                    let (a2, n2, connect) = (a.clone(), n.clone(), connect.clone());
                    let go: Go = Rc::new(move |window, cx| connect(&a2, &n2, window, cx));
                    this.unless_windows_paired(a.clone(), n.clone(), le, go, window, cx);
                })),
            )
        } else {
            actions
        };
        let actions = actions
            .when(self.props.manage && !d.saved, |x| {
                let (a, n, le) = (d.address.clone(), d.name.clone(), d.le_audio);
                x.child(
                    button("add", t("ui.devices.add"), ButtonKind::Subtle)
                        .icon(icon(IconName::Add))
                        .disabled(self.busy)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let weak = cx.entity().downgrade();
                            let (a2, n2) = (a.clone(), n.clone());
                            let go: Go = Rc::new(move |window, cx| {
                                let (a3, n3) = (a2.clone(), n2.clone());
                                let _ = weak.update(cx, |this, cx| this.add(a3, n3, le, false, window, cx));
                            });
                            this.unless_windows_paired(a.clone(), n.clone(), le, go, window, cx);
                        })),
                )
            })
            .when(d.saved && !self.props.saved_only, |x| {
                let (edit, forget) = (d.clone(), d.clone());
                x.child(
                    icon_button("edit", IconName::Edit, format!("{}: {label}", t("device.edit_name")))
                        .on_click(cx.listener(move |this, _, window, cx| this.open_rename(&edit, window, cx))),
                )
                .child(
                    icon_button("forget", IconName::Delete, format!("{}: {label}", t("ui.device.forget")))
                        .on_click(cx.listener(move |this, _, window, cx| this.open_forget(&forget, window, cx))),
                )
            });
        let (a, n, le) = (d.address.clone(), d.name.clone(), d.le_audio);
        div()
            .id(SharedString::from(format!("row-{prefix}{key}")))
            .flex()
            .items_center()
            .gap(theme::S3)
            .px(theme::S3)
            .py(theme::S2)
            .when(chosen, |x| x.bg(p.surface_2).border_l_2().border_color(p.accent_fill))
            .when(!d.audio, |x| x.opacity(0.6))
            .when(picking, |x| {
                x.child(
                    Radio::new("pick")
                        .checked(chosen)
                        .accessibility_label(label.clone())
                        .on_click(cx.listener(move |this, _: &bool, window, cx| this.pick(&a, &n, le, window, cx))),
                )
            })
            .when_some(paired, |x, s| x.child(s))
            .child(div().flex_none().child(icon(if d.audio { IconName::Headphones } else { IconName::Bluetooth })))
            .child(text)
            .child(tags)
            .child(actions)
            .into_any_element()
    }

    /// A bordered list of rows (`.device-list`), scrolling past 16rem
    fn list(&self, id: &str, rows: Vec<AnyElement>, cx: &App) -> impl IntoElement + use<> {
        let p = *cx.palette();
        div()
            .id(SharedString::from(id.to_owned()))
            .flex()
            .flex_col()
            .border_1()
            .border_color(p.border)
            .rounded(theme::RADIUS)
            .max_h(px(256.))
            .overflow_y_scrollbar()
            .children(
                rows.into_iter()
                    .enumerate()
                    .map(|(i, r)| div().when(i > 0, |x| x.border_t_1().border_color(p.border)).child(r)),
            )
    }

    fn empty_line(text: String, cx: &App) -> Div {
        div().p(theme::S3).text_color(cx.palette().text_2).child(text)
    }

    fn head(&self, snap: &Snapshot, count: usize, cx: &mut Context<Self>) -> Div {
        let scanning = snap.scanning;
        let text = picker_head_text(snap, self.props.scan_text.as_deref(), count);
        let action = if self.props.saved_only {
            button("manage", t("ui.devices.manage"), ButtonKind::Default)
                .icon(icon(IconName::Bluetooth))
                .on_click(|_, _, cx| app::navigate(RouteName::Devices, cx))
        } else {
            button("scan", t(if scanning { "connection.scan_stop" } else { "connection.scan" }), ButtonKind::Default)
                .icon(icon(if scanning { IconName::Stop } else { IconName::Search }))
                .on_click(cx.listener(|this, _, window, cx| this.scan_clicked(window, cx)))
        };
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(theme::S3)
            .child(div().text_color(cx.palette().text_2).child(text))
            .child(action)
    }

    fn manual_field(&self, cx: &mut Context<Self>) -> Div {
        let text = self.manual.read(cx).value().to_string();
        let address = parse_address(&text);
        let error = if !text.is_empty() && address.is_none() {
            Some(t("error.invalid_address_format"))
        } else {
            self.props.error.clone()
        };
        let connecting = self.props.connecting.clone();
        let disabled = address.is_none()
            || connecting.is_some()
            || (self.props.on_connect.is_some() && self.props.connect_disabled);
        let loading = connecting.is_some() && connecting == address;
        let use_it = button(
            "use-address",
            t(if self.props.manage { "ui.devices.add" } else { "ui.devices.use_address" }),
            ButtonKind::Default,
        )
        .loading(loading)
        .disabled(disabled)
        .on_click(cx.listener(move |this, _, window, cx| {
            let Some(a) = address.clone() else { return };
            if let Some(connect) = this.props.on_connect.clone() {
                let a2 = a.clone();
                let go: Go = Rc::new(move |window, cx| connect(&a2, "", window, cx));
                this.unless_windows_paired(a, String::new(), false, go, window, cx);
            } else if this.props.manage {
                let weak = cx.entity().downgrade();
                let a2 = a.clone();
                let go: Go = Rc::new(move |window, cx| {
                    let a3 = a2.clone();
                    let _ = weak.update(cx, |this, cx| this.add(a3, String::new(), false, true, window, cx));
                });
                this.unless_windows_paired(a, String::new(), false, go, window, cx);
            } else {
                // A typed address keeps the kind chosen (the profile's check box)
                let le = this.props.le_audio;
                this.pick(&a, "", le, window, cx);
            }
        }));
        let control = div()
            .flex()
            .items_center()
            .gap(theme::S2)
            .child(
                div().w(px(240.)).child(
                    Input::new(&self.manual)
                        .font_family(theme::FONT_MONO)
                        // The field's label without its colon ("Manual:")
                        .aria_label(SharedString::from(
                            t("connection.manual").trim_end_matches([':', '：']).to_owned(),
                        )),
                ),
            )
            .child(use_it);
        super::field_stacked(
            t("connection.manual"),
            Some(t("ui.devices.manual_hint").into()),
            error.map(Into::into),
            control,
            cx,
        )
    }

    /// The Audio Devices screen (issue #86): the saved devices, each with
    /// its settings, then a card to add one (scan, or the address by hand).
    /// The saved ones are those Home and Codecs show (LE Audio entries on an
    /// adapter not known to stream it could not be removed here)
    fn managed(&mut self, snap: &Snapshot, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let p = *cx.palette();
        let saved = managed_saved(
            &snap.devices,
            ui_model::profile::le_audio_on(&snap.settings),
            ui_model::profile::hearing_aids_on(&snap.settings),
            self.props.classic_only,
        );
        // One device, one entry (issue #86): its settings view goes by its key
        let keys: Vec<String> = saved.iter().map(|d| device_key(&d.address, d.le_audio)).collect();
        self.settings.retain(|k, _| keys.contains(k));
        let mut entries = Vec::new();
        for (d, key) in saved.iter().zip(&keys) {
            let settings = self.props.settings.then(|| {
                let view = self
                    .settings
                    .entry(key.clone())
                    .or_insert_with(|| cx.new(|cx| DeviceSettings::new(d.clone(), window, cx)))
                    .clone();
                view.update(cx, |s, _| s.set_device(d.clone()));
                view
            });
            entries.push(
                div()
                    .border_1()
                    .border_color(p.border)
                    .rounded(theme::RADIUS)
                    .bg(p.surface)
                    .child(self.row(d, "s", cx))
                    .when_some(settings, |x, s| x.child(s)),
            );
        }
        let saved_list = div().flex().flex_col().gap(theme::S3).map(|x| {
            if entries.is_empty() {
                x.child(Self::empty_line(t("ui.devices.none_saved"), cx))
            } else {
                x.children(entries)
            }
        });
        let found: Vec<Device> =
            picker_devices(snap, self.props.classic_only, false, false).into_iter().filter(|d| !d.saved).collect();
        let rows: Vec<AnyElement> = found.iter().map(|d| self.row(d, "f", cx)).collect();
        let add = super::card(cx)
            .child(
                div()
                    .text_size(theme::FS_SMALL)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.text_2)
                    .child(t("ui.devices.add_title")),
            )
            .child(self.head(snap, found.len(), cx))
            .when(!found.is_empty(), |x| x.child(self.list("found", rows, cx)))
            .when(found.is_empty() && snap.scanning, |x| {
                x.child(self.list(
                    "found",
                    vec![Self::empty_line(t("connection.no_devices"), cx).into_any_element()],
                    cx,
                ))
            })
            .child(self.manual_field(cx));
        div().flex().flex_col().gap(theme::S4).child(saved_list).child(add)
    }
}

impl Render for DevicePicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(snap) = self.core.read(cx).snap().cloned() else { return div() };
        let p = *cx.palette();
        let paired_rows: Vec<AnyElement> = if self.props.paired.is_some() {
            paired_list(&snap.devices).iter().map(|d| self.row(d, "p", cx)).collect()
        } else {
            Vec::new()
        };
        let mut root = div().flex().flex_col().gap(theme::S3);
        if !paired_rows.is_empty() {
            root = root.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(theme::S2)
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(t("ui.setup.paired_title")))
                    .child(self.list("paired", paired_rows, cx)),
            );
        }
        if let Some(children) = self.props.children.clone() {
            root = root.child(children);
        }
        if self.props.manage {
            return root.child(self.managed(&snap, window, cx));
        }
        if self.props.no_scan {
            return root;
        }
        let devices =
            picker_devices(&snap, self.props.classic_only, self.props.saved_only, self.props.paired.is_some());
        let shown: Vec<Device> =
            if self.folded { devices.iter().filter(|d| self.chosen(d)).cloned().collect() } else { devices.clone() };
        let others = devices.len() - shown.len();
        root = root.child(self.head(&snap, devices.len(), cx));
        // Folded with a typed address: no row, only the button below
        if !self.folded || !shown.is_empty() {
            let rows: Vec<AnyElement> = if shown.is_empty() {
                let key = if self.props.saved_only { "ui.devices.saved_empty" } else { "connection.no_devices" };
                vec![Self::empty_line(t(key), cx).into_any_element()]
            } else {
                shown.iter().map(|d| self.row(d, "", cx)).collect()
            };
            root = root.child(self.list("devices", rows, cx));
        }
        if self.folded && others > 0 {
            root = root.child(
                div().flex().child(
                    button("show-others", t!("ui.devices.show_others", others), ButtonKind::Subtle)
                        .icon(icon(IconName::ChevronDown))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.folded = false;
                            cx.notify();
                        })),
                ),
            );
        }
        if let Some(error) = self.props.error.clone().filter(|_| self.props.saved_only) {
            root = root.child(
                div()
                    .flex()
                    .items_center()
                    .gap(theme::S1)
                    .text_size(theme::FS_SMALL)
                    .text_color(p.bad_text)
                    .child(icon(IconName::Warning).small())
                    .child(error),
            );
        }
        if !self.props.saved_only {
            root = root.child(self.manual_field(cx));
        }
        root
    }
}

#[cfg(test)]
mod interaction_tests;
