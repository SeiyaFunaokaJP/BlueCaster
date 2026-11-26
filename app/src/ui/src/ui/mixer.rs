//! The apps on the default output device, each with its own volume (issue
//! #223): on the Volume Mixer screen and in the flyout, in developer mode.

use std::time::{Duration, Instant};

use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::{
    AppContext as _, AsyncWindowContext, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Subscription, Task, WeakEntity, Window, div, px,
};
use ipc::{Method, MixerApp};
use ui_model::i18n::t;
use ui_model::view::mixer::{
    REFRESH_MS, RELEASE_MS, app_name, mute_label, mute_request, reconcile, send_wait_ms, shown_volume, volume_request,
};

use crate::app;
use crate::theme::{self, ActivePalette as _};
use crate::ui::{self, IconName};

/// One app's line. Kept by key through a refresh: a new slider would take
/// the thumb out of the pointer's hand.
struct MixerRow {
    app: MixerApp,
    slider: Entity<SliderState>,
    /// The value under the pointer, which a refresh does not move
    held: Option<u8>,
    last_sent: Option<Instant>,
    pending: Option<Task<()>>,
    release: Option<Task<()>>,
    _changes: Subscription,
}

pub struct MixerList {
    rows: Vec<MixerRow>,
    loaded: bool,
    _refresh: Task<()>,
}

/// Fire and forget: a failed change shows nothing (the next refresh tells)
fn send(method: Method, cx: &gpui_kit::App) {
    app::request::<serde_json::Value>(method, cx).detach();
}

impl MixerList {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> MixerList {
        // Sessions come and go without an event
        let refresh = cx.spawn_in(window, async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
            loop {
                if this.update_in(cx, |this, window, cx| this.refresh(window, cx)).is_err() {
                    return;
                }
                cx.background_executor().timer(Duration::from_millis(REFRESH_MS)).await;
            }
        });
        MixerList { rows: Vec::new(), loaded: false, _refresh: refresh }
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // A window nobody sees asks nothing
        if !window.is_visible() && self.loaded {
            return;
        }
        let task = app::request::<Vec<MixerApp>>(Method::AudioMixer, cx);
        cx.spawn_in(window, async move |this, cx| {
            let Ok(list) = task.await else { return };
            let _ = this.update_in(cx, |this, window, cx| this.take(list, window, cx));
        })
        .detach();
    }

    /// A refresh by key: rows kept keep their slider and the value in hand
    fn take(&mut self, list: Vec<MixerApp>, window: &mut Window, cx: &mut Context<Self>) {
        let keys: Vec<String> = self.rows.iter().map(|r| r.app.key.clone()).collect();
        let mut old: Vec<Option<MixerRow>> = std::mem::take(&mut self.rows).into_iter().map(Some).collect();
        let from = reconcile(&keys, &list);
        for (a, from) in list.into_iter().zip(from) {
            let row = match from.and_then(|i| old[i].take()) {
                Some(mut row) => {
                    if row.held.is_none() && row.app.volume != a.volume {
                        row.slider.update(cx, |s, cx| s.set_value(f32::from(a.volume), window, cx));
                    }
                    row.app = a;
                    row
                }
                None => self.make_row(a, window, cx),
            };
            self.rows.push(row);
        }
        self.loaded = true;
        cx.notify();
    }

    fn make_row(&mut self, a: MixerApp, window: &mut Window, cx: &mut Context<Self>) -> MixerRow {
        let slider = cx.new(|_| SliderState::new().min(0.).max(100.).step(1.).default_value(f32::from(a.volume)));
        let key = a.key.clone();
        let changes = cx.subscribe_in(&slider, window, move |this, _, ev: &SliderEvent, _, cx| match ev {
            SliderEvent::Change(v) => this.moved(&key, v.start().round().clamp(0., 100.) as u8, cx),
            SliderEvent::Release(_) => this.let_go(&key, cx),
        });
        MixerRow { app: a, slider, held: None, last_sent: None, pending: None, release: None, _changes: changes }
    }

    fn row(&mut self, key: &str) -> Option<&mut MixerRow> {
        self.rows.iter_mut().find(|r| r.app.key == key)
    }

    /// At most ten requests a second while dragging, and the final value
    fn moved(&mut self, key: &str, volume: u8, cx: &mut Context<Self>) {
        let Some(row) = self.row(key) else { return };
        row.held = Some(volume);
        row.release = None;
        row.pending = None;
        // The executor's clock: the wall clock, and the test clock in tests
        let now = cx.background_executor().now();
        let wait =
            send_wait_ms(row.last_sent.map_or(u64::MAX, |t| now.saturating_duration_since(t).as_millis() as u64));
        if wait == 0 {
            row.last_sent = Some(now);
            send(volume_request(key, volume), cx);
        } else {
            let key = key.to_owned();
            row.pending = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
                cx.background_executor().timer(Duration::from_millis(wait)).await;
                let _ = this.update(cx, |this, cx| {
                    let now = cx.background_executor().now();
                    if let Some(row) = this.row(&key) {
                        row.last_sent = Some(now);
                    }
                    send(volume_request(&key, volume), cx);
                });
            }));
        }
        cx.notify();
    }

    fn let_go(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(row) = self.row(key) else { return };
        let key = key.to_owned();
        row.release = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            cx.background_executor().timer(Duration::from_millis(RELEASE_MS)).await;
            let _ = this.update(cx, |this, cx| {
                if let Some(row) = this.row(&key) {
                    if let Some(volume) = row.held.take() {
                        row.app.volume = volume;
                    }
                    cx.notify();
                }
            });
        }));
    }

    fn toggle_mute(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(row) = self.row(key) else { return };
        let muted = !row.app.muted;
        row.app.muted = muted;
        send(mute_request(key, muted), cx);
        cx.notify();
    }
}

impl Render for MixerList {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let list = div().flex().flex_col().gap(theme::S1);
        if self.loaded && self.rows.is_empty() {
            return list.child(ui::muted(t("ui.mixer.empty"), cx).p(theme::S3));
        }
        list.children(self.rows.iter().enumerate().map(|(i, r)| {
            let name: SharedString = app_name(&r.app).into();
            let label = mute_label(r.app.muted);
            let shown = shown_volume(r.held, r.app.volume);
            let key = r.app.key.clone();
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .pt(theme::S1)
                .text_size(theme::FS_SMALL)
                .child(div().truncate().child(name.clone()))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(theme::S3)
                        .py(theme::S2)
                        .px(theme::S1)
                        .text_color(p.text_2)
                        // The speaker mutes and unmutes (issue #86)
                        .child(
                            ui::icon_button(
                                ("mixer-mute", i),
                                if r.app.muted { IconName::SpeakerMute } else { IconName::Speaker2 },
                                label.clone(),
                            )
                            .accessibility_label(SharedString::from(format!("{name}: {label}")))
                            .on_click(cx.listener(move |this, _, _, cx| this.toggle_mute(&key, cx))),
                        )
                        .child(Slider::new(&r.slider).flex_1().accessibility_label(name.clone()))
                        .child(div().min_w(px(48.)).text_right().text_color(p.text).child(format!("{shown}%"))),
                )
        }))
    }
}

#[cfg(test)]
mod slider_tests;
