//! One audio device's own settings on the Audio Devices screen (issue #86):
//! what is captured for it (one list, as on Home) and whether a virtual
//! device becomes the Windows default output (kept in its profile: the core
//! reads them from the profile it streams; a device without one gets it with
//! the first change). Windows Volume Sync, the buttons and the remembered
//! volume are one setting each for every device, above the list
//! (screens/devices.rs); the max media packet size is on the Developer page.
//! The capture source's format is a box beside it, and what the source does
//! is said below it (issue #86).
//!
//! The list's entries, the format and the profiles a change goes into are
//! worked out in ui_model::view::devices.

use gpui_kit::component::Sizable as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, AsyncApp, Context, Div, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div, px,
};
use ipc::{AudioApp, AudioDevice, Device, Method, Profile, Snapshot};
use ui_model::i18n::t;
use ui_model::profile::{
    AppCaptureSupport, DeviceFields, audio_entries, device_fields, device_fields_complete, device_key, profiles_of,
};
use ui_model::view::devices::{
    AudioLists, CaptureFormat, bits_text, capture_format, capture_hint, capture_value, device_profile_updates,
    device_sources, first_device_profile, format_warning, le_only, mic_device, newest_base, with_capture,
};

use super::capture_picker::capture_text;
use super::input_source::{InputSource, InputSourceProps};
use super::{BannerKind, Choice, IconName, icon};
use crate::app;
use crate::core::Core;
use crate::theme::{self, ActivePalette as _};

/// A small warning under a control (`.field-warn`), with its mark or without
pub fn field_warn(text: impl Into<SharedString>, with_icon: bool, cx: &App) -> Div {
    let p = *cx.palette();
    div()
        .flex()
        .items_center()
        .gap(theme::S1)
        .text_size(theme::FS_SMALL)
        .text_color(p.warn_text)
        .when(with_icon, |d| d.child(div().flex_none().child(icon(IconName::Warning).small())))
        .child(div().flex_1().min_w_0().child(text.into()))
}

/// Whether this Windows can capture one application (build 19041 or
/// later); the core still refuses the stream on an older one
pub fn app_capture_support() -> AppCaptureSupport {
    if windows_version::OsVersion::current().build >= 19041 { AppCaptureSupport::Yes } else { AppCaptureSupport::No }
}

pub struct DeviceSettings {
    core: Entity<Core>,
    device: Device,
    /// What is shown: the first profile's, until changed here
    f: DeviceFields,
    /// The first profile's settings as last seen (the core may change them
    /// itself: a cable that came or went, issue #86)
    seen: Option<DeviceFields>,
    saving: bool,
    // ---- Audio devices and apps (asked from the core when needed) ----------
    devices: Option<Vec<AudioDevice>>,
    /// Windows' list was read (not only failed to be): the VB-Cable
    /// suggestion relies on it
    listed: bool,
    apps: Option<Vec<AudioApp>>,
    audio_key: String,
    app_support: AppCaptureSupport,
    input_source: Entity<InputSource>,
}

impl DeviceSettings {
    pub fn new(device: Device, window: &mut Window, cx: &mut Context<Self>) -> DeviceSettings {
        let core = app::core(cx);
        let (f, seen, audio_key) = {
            let snap = core.read(cx).snap();
            let profiles = snap.map(|s| profiles_of(&s.profiles, &device.address)).unwrap_or_default();
            let seen = profiles.first().map(|p| device_fields(p));
            (device_fields(&newest_base(&profiles)), seen, super::audio_devices_key(snap))
        };
        let input_source = cx.new(|cx| InputSource::new(Self::input_props(&device, &[], &f), cx));
        cx.observe(&core, |this: &mut Self, core, cx| {
            let Some(snap) = core.read(cx).snap() else { return };
            // Also when the core changes the profile itself: the shown
            // settings follow, the list stays the same
            let first = profiles_of(&snap.profiles, &this.device.address).first().map(|p| device_fields(p));
            let key = super::audio_devices_key(Some(snap));
            if first.is_some() && first != this.seen {
                this.f = first.clone().unwrap_or_default();
                this.seen = first;
                this.sync_input(cx);
            }
            if key != this.audio_key {
                this.audio_key = key;
                this.load_devices(cx);
            }
            cx.notify();
        })
        .detach();
        // Asked again whenever the window comes back (apps start and stop playing)
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.load(cx);
            }
        })
        .detach();
        let mut this = DeviceSettings {
            core,
            device,
            f,
            seen,
            saving: false,
            devices: None,
            listed: false,
            apps: None,
            audio_key,
            app_support: app_capture_support(),
            input_source,
        };
        this.load(cx);
        this
    }

    /// The device of the entry (its name may change)
    pub fn set_device(&mut self, device: Device) {
        self.device = device;
    }

    fn input_props(device: &Device, devices: &[AudioDevice], f: &DeviceFields) -> InputSourceProps {
        InputSourceProps {
            device: device.clone(),
            devices: devices.to_vec(),
            capture_mode: f.capture_mode.clone(),
            audio_device_id: f.audio_device_id.clone(),
            auto_switch_device: Some(f.auto_switch_device),
        }
    }

    fn sync_input(&mut self, cx: &mut Context<Self>) {
        let props = Self::input_props(&self.device, self.devices.as_deref().unwrap_or_default(), &self.f);
        self.input_source.update(cx, |i, cx| i.set_props(props, cx));
    }

    fn load_devices(&mut self, cx: &mut Context<Self>) {
        let task = app::request::<Vec<AudioDevice>>(Method::AudioDevices, cx);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let list = task.await;
            let _ = this.update(cx, |this, cx| {
                this.listed = list.is_ok();
                this.devices = Some(list.unwrap_or_default());
                this.sync_input(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Both are in the capture source's list
    fn load(&mut self, cx: &mut Context<Self>) {
        self.load_devices(cx);
        let task = app::request::<Vec<AudioApp>>(Method::AudioApps, cx);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let list = task.await;
            let _ = this.update(cx, |this, cx| {
                this.apps = Some(list.unwrap_or_default());
                cx.notify();
            });
        })
        .detach();
    }

    /// The shown settings as a profile (what Home's capture source list takes)
    fn shown(&self, snap: &Snapshot) -> Profile {
        let profiles = profiles_of(&snap.profiles, &self.device.address);
        let mut p = newest_base(&profiles);
        self.f.apply_to(&mut p);
        p.le_audio = le_only(&self.device, &profiles);
        p
    }

    /// Changes the device's settings: into each of its profiles once
    /// complete (a virtual device or an app chosen); a profile in use starts
    /// again with them (profile.save in the core). A device without a
    /// profile gets one (Auto, as the Codecs screen would make it).
    fn apply(&mut self, patch: impl FnOnce(&mut DeviceFields), cx: &mut Context<Self>) {
        patch(&mut self.f);
        self.sync_input(cx);
        cx.notify();
        if !device_fields_complete(&self.f) {
            return;
        }
        self.saving = true;
        let (next, device) = (self.f.clone(), self.device.clone());
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let snap = |cx: &mut AsyncApp| cx.update(|cx| app::core(cx).read(cx).snap().cloned());
            if let Some(s) = snap(cx)
                && profiles_of(&s.profiles, &device.address).is_empty()
            {
                let profile = first_device_profile(&device, &next, &s.profiles);
                let save = Method::ProfileSave { profile: Box::new(profile), original_name: None };
                let task = cx.update(|cx| app::run(app::request::<serde_json::Value>(save, cx), cx));
                task.await;
            }
            if let Some(s) = snap(cx) {
                for (profile, original) in device_profile_updates(&profiles_of(&s.profiles, &device.address), &next) {
                    let save = Method::ProfileSave { profile: Box::new(profile), original_name: Some(original) };
                    let task = cx.update(|cx| app::run(app::request::<serde_json::Value>(save, cx), cx));
                    if task.await.is_none() {
                        break;
                    }
                }
            }
            let _ = this.update(cx, |this, cx| {
                this.saving = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn choose_source(&mut self, value: &str, cx: &mut Context<Self>) {
        let Some(snap) = self.core.read(cx).snap() else { return };
        let p = self.shown(snap);
        if value == capture_value(&p) {
            return;
        }
        let next = with_capture(
            &p,
            value,
            self.devices.as_deref().unwrap_or_default(),
            self.apps.as_deref().unwrap_or_default(),
        );
        self.apply(
            move |f| {
                f.capture_mode = next.capture_mode;
                f.audio_device_id = next.audio_device_id;
                f.audio_device_name = next.audio_device_name;
                f.app_exe = next.app_exe;
                f.app_name = next.app_name;
            },
            cx,
        );
    }
}

/// The format Windows mixes the captured device in, a read-only box beside
/// the capture source (issue #86), amber when a profile of the device asks
/// for another rate or bit depth (v1 capture.device_format)
fn capture_format_box(c: &CaptureFormat, cx: &App) -> impl IntoElement + use<> {
    let p = *cx.palette();
    let warn = c.off.is_some();
    let (line, text) = if warn { (p.warn_text, p.warn_text) } else { (p.border_strong, p.text) };
    let title = SharedString::from(ui_model::t!("capture.device_format", c.rate, c.bits));
    let part = || div().px(theme::S3);
    div()
        .id("capture-format")
        .flex()
        .flex_none()
        .items_center()
        .h(gpui_kit::rems(2.))
        .border_1()
        .border_color(line)
        .rounded(theme::RADIUS)
        .bg(p.surface_2)
        .text_color(text)
        .text_size(theme::FS_BODY)
        .when(warn, |d| d.child(div().pl(theme::S3).child(icon(IconName::Warning).small())))
        .child(part().child(format!("{} Hz", c.rate)))
        .child(part().border_l_1().border_color(line).child(bits_text(u32::from(c.bits))))
        .tooltip(move |window, cx| Tooltip::new(title.clone()).build(window, cx))
}

impl Render for DeviceSettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(snap) = self.core.read(cx).snap().cloned() else { return div() };
        let p = *cx.palette();
        let shown = self.shown(&snap);
        let mode = shown.capture_mode.clone();
        let profiles = profiles_of(&snap.profiles, &self.device.address);
        let devices = self.devices.as_deref().unwrap_or_default();
        let entry =
            audio_entries(&snap).into_iter().find(|e| e.key == device_key(&self.device.address, self.device.le_audio));
        let lists = AudioLists {
            devices: self.devices.as_deref(),
            listed: self.listed,
            apps: self.apps.as_deref(),
            apps_ok: self.app_support != AppCaptureSupport::No,
        };
        let sources: Vec<Choice<String>> =
            device_sources(&shown, lists, app::all_audio_devices(Some(&snap.settings)), &mic_device(entry.as_ref()))
                .into_iter()
                .map(|o| {
                    let mut c = Choice::new(o.value, o.label).disabled(o.disabled);
                    if let Some(g) = o.group {
                        c = c.group(g);
                    }
                    c
                })
                .collect();
        let format = capture_format(&device_fields(&shown), &profiles, devices, &snap);
        let eq = super::media_eq::media_eq_settings(&self.device, cx).into_any_element();
        let (source_icon, _) = capture_text(&shown);
        let control = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(theme::S2)
            .child(icon(source_icon))
            .child(div().flex_1().min_w(px(192.)).child(super::select_labelled(
                "capture-source",
                t("capture.mode"),
                &capture_value(&shown),
                sources,
                false,
                cx.listener(|this, v: &String, _, cx| this.choose_source(v, cx)),
            )))
            .when_some(format.as_ref(), |d, c| d.child(capture_format_box(c, cx)))
            .child(
                super::icon_button("capture-refresh", IconName::ArrowSync, t("capture.app_refresh"))
                    .on_click(cx.listener(|this, _, _, cx| this.load(cx))),
            );
        // What the capture source does, below it (one line per line of the text)
        let about = div()
            .flex()
            .flex_col()
            .gap(theme::S2)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .children(capture_hint(&mode).split('\n').map(|l| super::hint(l.to_owned(), cx))),
            )
            .when(mode == "app" && self.app_support == AppCaptureSupport::Unknown, |d| {
                d.child(super::hint(t("ui.profile.app_capture_requirement"), cx))
            });
        div()
            .flex()
            .flex_col()
            .gap(theme::S3)
            .px(theme::S4)
            .pt(theme::S3)
            .pb(theme::S4)
            .border_t_1()
            .border_color(p.border)
            .child(super::field_stacked(t("capture.mode"), None, None, control, cx))
            .child(about)
            .when_some(format.as_ref().and_then(format_warning), |d, w| d.child(field_warn(w, true, cx)))
            // Whether this Windows can capture one app: only the core knows
            // for sure (it refuses the stream on an older Windows)
            .when(self.f.capture_mode == "app" && self.app_support == AppCaptureSupport::No, |d| {
                d.child(super::banner(
                    BannerKind::Warn,
                    None,
                    Some(div().child(t("ui.profile.app_capture_unsupported")).into_any_element()),
                    None,
                    None,
                    cx,
                ))
            })
            .when(
                app::switchable_source(&self.f.capture_mode) && app::devices_switch_shown(Some(&snap.settings)),
                |d| {
                    d.child(super::toggle(
                        "auto-switch",
                        t("capture.auto_switch"),
                        None,
                        self.f.auto_switch_device,
                        false,
                        cx.listener(|this, v: &bool, _, cx| {
                            let v = *v;
                            this.apply(move |f| f.auto_switch_device = v, cx);
                        }),
                        cx,
                    ))
                },
            )
            // A line that opens a dialog reaches past the text on both sides (`.nav-row`)
            .child(eq)
            .children(super::le_channels::le_channels(&self.device, cx))
            .children(super::hearing_aid::beside(&self.device, cx))
            .child(self.input_source.clone())
    }
}
