//! Microphone tab: the headset microphone test. Record opens the voice link
//! of the classic stream (the core starts the stream again first when its
//! stack has no voice), the bar shows the microphone's level as it records,
//! Play sends the recording back to the headphones with its level on the same
//! bar. The test ends when the tab closes (the core also ends one nobody asks
//! about), and the stream goes back to what headset mode wants.

use std::time::Duration;

use gpui_kit::component::Disableable as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AsyncApp, Context, IntoElement, ParentElement as _, Render, Styled as _, Task, WeakEntity, Window, div, px,
};
use ipc::{Method, MicTestState, StreamState};
use ui_model::i18n::t;
use ui_model::view::diagnostics::{MIC_FALL, MIC_SEGMENTS, bar_share, mic_progress};

use crate::app;
use crate::theme::{self, ActivePalette as _};
use crate::ui::{self, ButtonKind, IconName, icon};

/// How often the state is asked for while a test runs
const POLL: Duration = Duration::from_millis(70);

fn idle() -> MicTestState {
    MicTestState { phase: "idle".into(), level: 0.0, recorded_ms: 0, max_ms: 10000, played_ms: 0, error: None }
}

pub struct MicPanel {
    st: MicTestState,
    bar: f64,
    error: Option<String>,
    busy: bool,
    timer: Option<Task<()>>,
}

impl MicPanel {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> MicPanel {
        let core = app::core(cx);
        cx.observe(&core, |_, _, cx| cx.notify()).detach();
        // The test ends with the tab
        cx.on_release(|this: &mut MicPanel, cx| {
            let was = this.running();
            this.stop();
            if was {
                app::request::<serde_json::Value>(Method::MicTestEnd, cx).detach();
            }
        })
        .detach();
        MicPanel { st: idle(), bar: 0.0, error: None, busy: false, timer: None }
    }

    fn running(&self) -> bool {
        self.st.phase != "idle"
    }

    fn take(&mut self, s: MicTestState, cx: &mut Context<Self>) {
        if let Some(e) = &s.error {
            self.error = Some(t(e));
        }
        self.bar = bar_share(s.level as f64).max(self.bar * MIC_FALL);
        let idle = s.phase == "idle";
        self.st = s;
        if idle {
            self.stop();
        } else if self.timer.is_none() {
            self.timer = Some(cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                loop {
                    cx.background_executor().timer(POLL).await;
                    let Ok(task) = this.update(cx, |_, cx| app::request::<MicTestState>(Method::MicTestStatus, cx))
                    else {
                        break;
                    };
                    let result = task.await;
                    let go_on = this
                        .update(cx, |this, cx| {
                            match result {
                                Ok(s) => this.take(s, cx),
                                Err(_) => this.stop(),
                            }
                            cx.notify();
                            this.timer.is_some()
                        })
                        .unwrap_or(false);
                    if !go_on {
                        break;
                    }
                }
            }));
        }
        cx.notify();
    }

    fn stop(&mut self) {
        // Dropping the task ends the polling; called from inside it, the
        // loop sees no timer and ends
        self.timer = None;
        self.bar = 0.0;
    }

    fn act(&mut self, method: Method, cx: &mut Context<Self>) {
        self.busy = true;
        if method == Method::MicTestRecord {
            self.error = None;
        }
        cx.notify();
        let task = app::request::<MicTestState>(method, cx);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(s) => this.take(s, cx),
                    Err(e) => this.error = Some(app::error_text(&e)),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for MicPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let streaming = app::core(cx).read(cx).snap().is_some_and(|s| s.stream.state == StreamState::Streaming);
        let phase = self.st.phase.as_str();
        let playing = phase == "playing";
        let lit = (self.bar * MIC_SEGMENTS as f64).round() as usize;

        let record = if phase == "recording" || phase == "starting" {
            ui::button("mic-stop", t("ui.mic_test.stop"), ButtonKind::Strong)
                .icon(icon(IconName::Stop))
                .loading(phase == "starting")
                .on_click(cx.listener(|this, _, _, cx| this.act(Method::MicTestStop, cx)))
        } else {
            ui::button("mic-record", t("ui.mic_test.record"), ButtonKind::Strong)
                .icon(icon(IconName::Record))
                .loading(self.busy)
                .disabled(self.busy || !streaming)
                .on_click(cx.listener(|this, _, _, cx| this.act(Method::MicTestRecord, cx)))
        };
        let play = if playing {
            ui::button("mic-play-stop", t("ui.mic_test.stop"), ButtonKind::Default)
                .icon(icon(IconName::Stop))
                .on_click(cx.listener(|this, _, _, cx| this.act(Method::MicTestStop, cx)))
        } else {
            ui::button("mic-play", t("ui.mic_test.play"), ButtonKind::Default)
                .icon(icon(IconName::Play))
                .disabled(self.busy || !(phase == "recorded" && self.st.recorded_ms > 0))
                .on_click(cx.listener(|this, _, _, cx| this.act(Method::MicTestPlay, cx)))
        };
        let on = if playing { p.good_text } else { p.accent_fill };
        let meter = div().flex_grow(1.).flex_basis(px(256.)).min_w_0().h(px(32.)).flex().gap(px(3.)).children(
            (0..MIC_SEGMENTS)
                .map(|i| div().flex_1().min_w(px(2.)).rounded_full().bg(if i < lit { on } else { p.border_strong })),
        );

        div()
            .flex()
            .flex_col()
            .gap(theme::S3)
            .when(!streaming && !self.running(), |d| d.child(ui::muted(t("quality.not_streaming"), cx)))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_x(theme::S3)
                    .gap_y(theme::S2)
                    .child(record)
                    .child(play)
                    .child(meter)
                    .child(div().min_w(px(110.)).text_color(p.text_2).child(mic_progress(&self.st))),
            )
            .child(ui::hint(t("ui.mic_test.hint"), cx))
            .when_some(self.error.clone(), |d, e| d.child(div().text_color(p.bad_text).child(e)))
    }
}
