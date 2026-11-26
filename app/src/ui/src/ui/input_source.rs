//! A device's Input Source on Audio Devices (issue #86; it was on Codecs):
//! the headphones' microphone rendered to an output of the PC, a virtual
//! cable whose recording side apps use as a microphone. Off, Auto (only
//! while an app records from it) or On, with the mode beside the device (below it
//! when narrow; issue #86: not above it), the device always there (disabled
//! while Off) so that
//! switching does not move the page. The list names each device by the side
//! apps pick ("CABLE Output", not "CABLE Input"; an output whose recording
//! side another one already names, with its own name added). Only the cables
//! a microphone can use: not the 16-channel one, outputs without a recording
//! side (under Monitoring) or the output source's cable, unless chosen or
//! Developer > Show every audio device is on (the source's cable then listed
//! but disabled, saying so; issue #86: the list read as chaos). A device
//! that has no profile yet shows what its first one gets: VB-CABLE, else the
//! Hi-Fi Cable, on Auto (Off for LE Audio, which takes Auto as On; the
//! core's default_mic_output). System Loopback's default output is not
//! another device's Output Source its stream made the default. The
//! Hi-Fi Cable with its two sides at different sample rates is said (it may
//! pass nothing then; on one PC it still did). Off here and the microphone's
//! button on the cards are one state (issue #86): a device whose mode is Auto
//! or On is muted by Off and shows Off while muted, so the button can open it
//! again in the same mode. Its recording side becomes Windows' default input
//! while the device is connected unless turned off (the core's
//! default_input.rs), so apps that record from the default microphone open it.
//! Its processing (gain, noise suppression, noise gate) follows, disabled
//! while the microphone is Off; it applies in place.
//!
//! The choices and warnings are worked out in ui_model::view::devices
//! (input_source_view).

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AsyncApp, Context, Entity, IntoElement, ParentElement as _, Render, SharedString, Styled as _, Task,
    WeakEntity, Window, div, px,
};
use ipc::{AudioDevice, Device, MicProcessing, Profile};
use ui_model::i18n::t;
use ui_model::profile::HeadsetMode;
use ui_model::t;
use ui_model::view::devices::{InputSourceView, MicPatch, OutputSource, choose_mode, input_source_view};

use std::rc::Rc;

use super::device_settings::field_warn;
use super::mic_processing::{MicProcessingSave, MicProcessingSource, mic_processing_row};
use super::{Choice, IconName, icon};
use crate::app;
use crate::core::Core;
use crate::theme::{self, ActivePalette as _};

pub use crate::screens::codecs::save_headset_mic;

/// What the parent shows of the device: its outputs and its output source
pub struct InputSourceProps {
    pub device: Device,
    pub devices: Vec<AudioDevice>,
    /// The device's output source as shown (a loop is refused)
    pub capture_mode: String,
    pub audio_device_id: String,
    pub auto_switch_device: Option<bool>,
}

pub struct InputSource {
    core: Entity<Core>,
    props: InputSourceProps,
    busy: bool,
}

impl InputSource {
    pub fn new(props: InputSourceProps, cx: &mut Context<Self>) -> InputSource {
        let core = app::core(cx);
        cx.observe(&core, |_, _, cx| cx.notify()).detach();
        InputSource { core, props, busy: false }
    }

    /// The parent's device, outputs or source changed
    pub fn set_props(&mut self, props: InputSourceProps, cx: &mut Context<Self>) {
        self.props = props;
        cx.notify();
    }

    fn view(&self, cx: &App) -> Option<InputSourceView> {
        let snap = self.core.read(cx).snap()?;
        let p = &self.props;
        let switches = app::switchable_source(&p.capture_mode)
            && app::default_device_switch(Some(&snap.settings), p.auto_switch_device);
        let source = OutputSource { capture_mode: &p.capture_mode, audio_device_id: &p.audio_device_id, switches };
        let key = ui_model::profile::device_key(&p.device.address, p.device.le_audio);
        input_source_view(snap, &key, &p.devices, source, app::all_audio_devices(Some(&snap.settings)))
    }

    /// Saves a change; the shown device goes with the first change (a device
    /// without a profile)
    fn save(&mut self, patch: impl FnOnce(&mut Profile) + 'static, cx: &mut Context<Self>) -> Task<bool> {
        let Some(v) = self.view(cx) else { return Task::ready(false) };
        self.busy = true;
        cx.notify();
        let (id, name) = (v.shown.mic_device_id.clone(), v.shown.mic_device_name.clone());
        let task = save_headset_mic(
            &v.entry,
            move |p| {
                p.mic_device_id = id;
                p.mic_device_name = name;
                patch(p);
            },
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let ok = task.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                cx.notify();
            });
            ok
        })
    }

    fn choose_mode(&mut self, m: HeadsetMode, cx: &mut Context<Self>) {
        let Some(v) = self.view(cx) else { return };
        match choose_mode(m, v.mode, v.saved) {
            Some(MicPatch::Mute) => self.save(|p| p.mic_muted = true, cx).detach(),
            Some(MicPatch::Mode(m)) => self
                .save(
                    move |p| {
                        p.headset_mic = m.as_str().into();
                        p.mic_muted = false;
                    },
                    cx,
                )
                .detach(),
            None => {}
        }
    }

    fn choose(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(d) = self.props.devices.iter().find(|x| x.id == id).cloned() else { return };
        if self.view(cx).is_some_and(|v| v.chosen == id) {
            return;
        }
        self.save(
            move |p| {
                p.mic_device_id = d.id;
                p.mic_device_name = d.name;
            },
            cx,
        )
        .detach();
    }
}

impl Render for InputSource {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(v) = self.view(cx) else { return div() };
        let Some(settings) = self.core.read(cx).snap().map(|s| s.settings.clone()) else { return div() };
        let p = *cx.palette();
        let busy = self.busy;
        let off = v.mode == HeadsetMode::Off;
        let options: Vec<Choice<String>> = v
            .options
            .iter()
            .map(|o| {
                let mut c = Choice::new(o.value.clone(), o.label.clone()).disabled(o.disabled);
                if let Some(g) = &o.group {
                    c = c.group(g.clone());
                }
                c
            })
            .collect();
        let modes: Vec<Choice<HeadsetMode>> = v
            .modes
            .iter()
            .map(|m| Choice::new(*m, t(&format!("ui.headset_mic_{}", m.as_str()))).disabled(busy))
            .collect();
        let pick = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(theme::S2)
            .child(icon(IconName::Mic))
            .child(div().flex_1().min_w(px(192.)).child(super::select_labelled(
                "mic-output",
                t("ui.mic_output"),
                &v.chosen,
                options,
                busy || off,
                cx.listener(|this, id: &String, _, cx| this.choose(id, cx)),
            )))
            .child(super::segmented(
                "mic-mode",
                &v.mode,
                modes,
                cx.listener(|this, m: &HeadsetMode, _, cx| this.choose_mode(*m, cx)),
            ));
        let processing = v.shown.mic_processing;
        // The processing's dialog reads it again whenever the core changes
        let weak = cx.entity().downgrade();
        let source: MicProcessingSource = Rc::new(move |cx: &App| {
            let this = weak.upgrade()?;
            let this = this.read(cx);
            this.view(cx).map(|v| (v.shown.mic_processing, v.mode == HeadsetMode::Off || this.busy))
        });
        let weak = cx.entity().downgrade();
        let on_change: MicProcessingSave = Rc::new(move |value: MicProcessing, cx: &mut App| {
            weak.update(cx, |this, cx| this.save(move |p| p.mic_processing = value, cx))
                .unwrap_or_else(|_| Task::ready(false))
        });
        div()
            .flex()
            .flex_col()
            .gap(theme::S3)
            .mx(-theme::S4)
            .px(theme::S4)
            .pt(theme::S3)
            .border_t_1()
            .border_color(p.border)
            .child(div().child(t("capture.input_source")))
            .child(pick)
            .child(super::hint(t("ui.headset_mic_quality"), cx))
            .when(v.no_output, |d| d.child(field_warn(t("ui.headset_mic_no_output"), false, cx)))
            .when(v.loops, |d| d.child(field_warn(t("capture.mic_loop_here"), true, cx)))
            .when_some(v.hifi_rates, |d, (play, rec)| {
                d.child(field_warn(t!("capture.hifi_rates_differ", play, rec), true, cx))
            })
            .when(v.plays_default, |d| d.child(field_warn(t("capture.mic_is_default_output"), true, cx)))
            .when(app::devices_switch_shown(Some(&settings)), |d| {
                d.child(super::toggle(
                    "mic-default-input",
                    SharedString::from(t("capture.mic_default_input")),
                    None,
                    v.shown.mic_default_input,
                    busy,
                    cx.listener(|this, on: &bool, _, cx| {
                        let on = *on;
                        this.save(move |p| p.mic_default_input = on, cx).detach();
                    }),
                    cx,
                ))
            })
            .child(mic_processing_row("mic-processing", &processing, off, source, on_change, cx))
    }
}
