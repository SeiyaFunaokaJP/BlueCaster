//! The headset microphone's processing, in a dialog opened under its Input
//! Source: a switch over all of it (off keeps the values, greyed out), then
//! gain, noise suppression's strength and the noise gate's threshold (each
//! at its lowest: off). Kept in the device's profile; the running
//! microphone takes a change in place, nothing reconnects. A slider saves
//! when let go and shows what it holds until the profile has it. Nothing
//! hides (issue #86): while the microphone is Off the line and the controls
//! stay, disabled.
//!
//! Use: [`mic_processing_row`] in the Input Source section; it opens a dialog
//! with [`MicProcessingControls`]. The caller hands over how to read the
//! current processing (the dialog draws it again whenever the core changes)
//! and how to save one.

use std::rc::Rc;

use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, AsyncApp, Context, ElementId, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    Subscription, Task, WeakEntity, Window, div, px,
};
use ipc::MicProcessing;
use ui_model::i18n::t;
use ui_model::view::mic_processing::{
    MIC_GAIN_RANGE, MIC_GATE_RANGE, MIC_SUPPRESSION_RANGE, MicSetting, gain_text, gate_shown, gate_text,
    mic_processing_on, slider_shown, sliders_disabled, suppression_text, with_setting,
};

use crate::app;
use crate::theme::{self, ActivePalette as _};
use crate::ui::{self, dialog};

/// The processing shown now and whether its controls are disabled (`off`:
/// the microphone is Off, or `busy`: a change is on its way); None when the
/// device is gone
pub type MicProcessingSource = Rc<dyn Fn(&App) -> Option<(MicProcessing, bool)>>;
/// Saves the processing -> true once saved
pub type MicProcessingSave = Rc<dyn Fn(MicProcessing, &mut App) -> Task<bool>>;

/// A line saying whether the processing is on, with the dialog behind it
/// (`off`: the microphone is Off, the line disabled)
pub fn mic_processing_row(
    id: impl Into<ElementId>,
    value: &MicProcessing,
    off: bool,
    source: MicProcessingSource,
    on_change: MicProcessingSave,
    cx: &App,
) -> impl IntoElement {
    ui::nav_row(
        id,
        t("ui.mic_processing"),
        t(if mic_processing_on(value) { "ui.state_on" } else { "ui.state_off" }),
        off,
        move |_, window, cx| {
            let (source, on_change) = (source.clone(), on_change.clone());
            let controls = cx.new(|cx| MicProcessingControls::new(source, on_change, window, cx));
            dialog::ask(
                dialog::Question::new(t("ui.mic_processing")),
                move |_, _| controls.clone().into_any_element(),
                |_, _| {},
                window,
                cx,
            );
        },
        cx,
    )
}

/// One of the sliders: being dragged, then saved and not back from the core yet
struct MicSlider {
    state: Entity<SliderState>,
    held: Option<i32>,
    pending: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Which {
    Gain,
    Suppression,
    Gate,
}

impl Which {
    const ALL: [Which; 3] = [Which::Gain, Which::Suppression, Which::Gate];

    fn range(self) -> (i32, i32) {
        match self {
            Which::Gain => MIC_GAIN_RANGE,
            Which::Suppression => MIC_SUPPRESSION_RANGE,
            Which::Gate => MIC_GATE_RANGE,
        }
    }

    fn value(self, p: &MicProcessing) -> i32 {
        match self {
            Which::Gain => i32::from(p.gain_db),
            Which::Suppression => i32::from(p.suppression),
            Which::Gate => gate_shown(p),
        }
    }

    fn setting(self, v: i32) -> MicSetting {
        match self {
            Which::Gain => MicSetting::Gain(v),
            Which::Suppression => MicSetting::Suppression(v),
            Which::Gate => MicSetting::Gate(v),
        }
    }

    fn text(self, v: i32) -> String {
        match self {
            Which::Gain => gain_text(v),
            Which::Suppression => suppression_text(v),
            Which::Gate => gate_text(v),
        }
    }

    fn label(self) -> String {
        t(match self {
            Which::Gain => "ui.mic_gain",
            Which::Suppression => "ui.mic_noise_suppression",
            Which::Gate => "ui.mic_gate",
        })
    }

    /// A line under the label: what the setting does
    fn hint(self) -> Option<String> {
        (self == Which::Gate).then(|| t("ui.mic_gate_hint"))
    }
}

/// The switch and the three sliders (`MicProcessingControls`)
pub struct MicProcessingControls {
    source: MicProcessingSource,
    on_change: MicProcessingSave,
    sliders: Vec<MicSlider>,
    _subscriptions: Vec<Subscription>,
}

impl MicProcessingControls {
    pub fn new(
        source: MicProcessingSource,
        on_change: MicProcessingSave,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> MicProcessingControls {
        let core = app::core(cx);
        let mut subs = vec![cx.observe(&core, |_, _, cx| cx.notify())];
        let current = source(cx).map(|(p, _)| p).unwrap_or_default();
        let mut sliders = Vec::new();
        for (i, which) in Which::ALL.into_iter().enumerate() {
            let (lo, hi) = which.range();
            let state = cx.new(|_| {
                SliderState::new().min(lo as f32).max(hi as f32).step(1.0).default_value(which.value(&current) as f32)
            });
            subs.push(cx.subscribe(&state, move |this: &mut Self, _, ev: &SliderEvent, cx| match ev {
                SliderEvent::Change(v) => {
                    this.sliders[i].held = Some(v.end().round() as i32);
                    cx.notify();
                }
                SliderEvent::Release(_) => this.let_go(i, which, cx),
            }));
            sliders.push(MicSlider { state, held: None, pending: None });
        }
        MicProcessingControls { source, on_change, sliders, _subscriptions: subs }
    }

    fn save(&self, s: MicSetting, cx: &mut App) -> Option<Task<bool>> {
        let (p, _) = (self.source)(cx)?;
        Some((self.on_change)(with_setting(&p, s), cx))
    }

    fn let_go(&mut self, i: usize, which: Which, cx: &mut Context<Self>) {
        let held = self.sliders[i].held.take();
        let value = (self.source)(cx).map(|(p, _)| which.value(&p));
        let Some(v) = held.filter(|v| Some(*v) != value) else {
            cx.notify();
            return;
        };
        self.sliders[i].pending = Some(v);
        cx.notify();
        let Some(task) = self.save(which.setting(v), cx) else { return };
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            task.await;
            let _ = this.update(cx, |this, cx| {
                this.sliders[i].pending = None;
                cx.notify();
            });
        })
        .detach();
    }

    fn slider(
        &self,
        i: usize,
        which: Which,
        p: &MicProcessing,
        disabled: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        let s = &self.sliders[i];
        let shown = slider_shown(s.held, s.pending, which.value(p));
        // Keep the slider on what it shows (the profile's value once saved)
        if s.held.is_none() && s.state.read(cx).value().end().round() as i32 != shown {
            s.state.update(cx, |st, cx| st.set_value(shown as f32, window, cx));
        }
        let pal = *cx.palette();
        let dim = |on: gpui_kit::Hsla| if disabled { pal.disabled_text } else { on };
        let label = which.label();
        ui::setting_row(cx)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(div().text_color(dim(pal.text)).child(label.clone()))
                    .when_some(which.hint(), |d, h| {
                        d.child(div().text_size(theme::FS_SMALL).text_color(dim(pal.text_2)).child(h))
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(theme::S3)
                    .w(px(288.))
                    .child(div().flex_1().child(Slider::new(&s.state).disabled(disabled).accessibility_label(label)))
                    .child(
                        div().min_w(px(56.)).flex().justify_end().text_color(dim(pal.text_2)).child(which.text(shown)),
                    ),
            )
    }
}

impl Render for MicProcessingControls {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some((p, disabled)) = (self.source)(cx) else { return div() };
        let slider_off = sliders_disabled(&p, disabled);
        let this = cx.entity().downgrade();
        let mut out = div().flex().flex_col().child(ui::toggle(
            "mic-processing-enable",
            t("ui.mic_processing_enable"),
            None,
            p.enabled,
            disabled,
            move |v, _, cx| {
                let v = *v;
                let _ = this.update(cx, |this, cx| {
                    if let Some(task) = this.save(MicSetting::Enabled(v), cx) {
                        task.detach();
                    }
                });
            },
            cx,
        ));
        for (i, which) in Which::ALL.into_iter().enumerate() {
            let el = self.slider(i, which, &p, slider_off, window, cx);
            out = out.child(el);
        }
        out
    }
}

#[cfg(test)]
mod slider_tests;
