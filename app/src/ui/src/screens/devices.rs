//! Audio Devices: what holds for every audio device (Windows Volume Sync, the
//! buttons, the remembered volume: one switch each, issue #86), then the
//! headphones and speakers BlueCaster knows, each with its own settings (capture
//! source, default output switching), and a card to add one: scan for new
//! ones or type an address. Profiles then pick from this list.

use gpui_kit::{
    AppContext as _, AsyncApp, Context, Entity, FontWeight, IntoElement, ParentElement as _, Render, Styled as _,
    WeakEntity, Window, div,
};
use ipc::{AudioDevice, Method, Settings};
use ui_model::i18n::t;

use crate::app;
use crate::core::Core;
use crate::theme::{self, ActivePalette as _};
use crate::ui;
use crate::ui::cable_notes::cable_notes;
use crate::ui::device_picker::{DevicePicker, DevicePickerProps};

pub struct Devices {
    core: Entity<Core>,
    /// Windows' outputs, for the cables it lacks (installed meanwhile: read
    /// again when the window comes back)
    devices: Option<Vec<AudioDevice>>,
    audio_key: String,
    picker: Entity<DevicePicker>,
}

impl Devices {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Devices {
        let core = app::core(cx);
        let audio_key = ui::audio_devices_key(core.read(cx).snap());
        cx.observe(&core, |this: &mut Self, core, cx| {
            let key = ui::audio_devices_key(core.read(cx).snap());
            if key != this.audio_key {
                this.audio_key = key;
                this.load(cx);
            }
            cx.notify();
        })
        .detach();
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.load(cx);
            }
        })
        .detach();
        let props = DevicePickerProps { manage: true, settings: true, ..Default::default() };
        let picker = cx.new(|cx| DevicePicker::new(props, window, cx));
        let mut this = Devices { core, devices: None, audio_key, picker };
        this.load(cx);
        this
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let task = app::request::<Vec<AudioDevice>>(Method::AudioDevices, cx);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            if let Ok(list) = task.await {
                let _ = this.update(cx, |this, cx| {
                    this.devices = Some(list);
                    cx.notify();
                });
            }
        })
        .detach();
    }
}

/// The settings shared by every audio device (Settings, saved at once; a
/// Volume Sync or output mute change starts the profile in use again in the core)
fn all_devices(s: &Settings, cx: &gpui_kit::App) -> impl IntoElement + use<> {
    let p = *cx.palette();
    ui::card(cx)
        .child(
            div()
                .text_size(theme::FS_SMALL)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.text_2)
                .child(t("ui.devices.all_title")),
        )
        .child(ui::toggle(
            "volume-sync",
            t("capture.volume_sync"),
            Some(t("capture.volume_sync_hint").into()),
            s.volume_sync,
            false,
            |v, _, cx| {
                let v = *v;
                app::save_settings(move |s| s.volume_sync = v, cx);
            },
            cx,
        ))
        .child(ui::toggle(
            "media-keys",
            t("settings.media_keys"),
            None,
            s.media_keys,
            false,
            |v, _, cx| {
                let v = *v;
                app::save_settings(move |s| s.media_keys = v, cx);
            },
            cx,
        ))
        .child(ui::toggle(
            "auto-reconnect",
            t("settings.auto_reconnect"),
            None,
            s.auto_reconnect,
            false,
            |v, _, cx| {
                let v = *v;
                app::save_settings(move |s| s.auto_reconnect = v, cx);
            },
            cx,
        ))
}

impl Render for Devices {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(settings) = self.core.read(cx).snap().map(|s| s.settings.clone()) else {
            return div().into_any_element();
        };
        let weak = cx.entity().downgrade();
        ui::page("devices", t("ui.nav.devices"))
            .child(cable_notes(self.devices.as_deref(), move |cx| {
                let _ = weak.update(cx, |this, cx| this.load(cx));
            }))
            .child(all_devices(&settings, cx))
            .child(self.picker.clone())
            .into_any_element()
    }
}
