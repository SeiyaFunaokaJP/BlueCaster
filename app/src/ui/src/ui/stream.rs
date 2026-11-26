//! The streaming controls shared by Home, the flyout and the navigation's
//! status block: the connection ring, the Connect / Disconnect button,
//! volume, codec badge, battery, link quality and the error panel with its
//! fix; and the pieces of Home other screens show (the headset
//! microphone's row, the adapter's and v1's warnings).
//!
//! The decisions (labels from the state, what shows) are in
//! `ui_model::view::stream`, tested there; this module draws them.

use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, AnyView, App, AppContext as _, AsyncApp, BoxShadow, Context, Div,
    ElementId, Entity, FontWeight, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Task, WeakEntity, Window, div, point, px,
};
use ipc::{AdapterInfo, DiagInfo, DiagInfoParams, ErrorInfo, Method, Msg, Profile, StreamState, WindowsPairedDevice};
use ui_model::format::{kbps, khz, percent_to_volume, volume_percent};
use ui_model::i18n::{t, tm};
use ui_model::links::{BugReport, bug_report_url};
use ui_model::profile::AudioEntry;
use ui_model::t;
use ui_model::view::stream::{self as model, BatteryTone, FixAction, QualityStatus};

// The decisions the screens draw with
#[allow(unused_imports)]
pub use ui_model::view::stream::{
    FALLBACK_NOTE_MS, LinkQuality, MIC_NOTES, QUALITY_AIRTIME_MAX, QUALITY_QUEUE_BACKLOG_MS, QualityCause, abr_label,
    abr_tooltip_key, adapter_drivers, adapter_needing_driver, bitpool_text, bug_report_for, codec_details, codec_name,
    codec_stop_text, current_profile, entry_state, error_copy_text, fallback_note_key, hidden_by_hung, is_app_fault,
    link_quality, mic_shown, quality_label, shows_battery, state_label, status_text, voice_codec_name,
};

use crate::app::{self, AdapterSection, NoticeKind, Route, RouteName};
use crate::core::{self, CoreEvent, WantKind};
use crate::host;
use crate::theme::{self, ActivePalette as _, Palette};
use crate::ui::dialog::{self, Question};
use crate::ui::{self, BannerKind, ButtonKind, IconName, charts, icon};

/// The status colour of a state (`.st-<state>`): the dot's, and the text's
pub fn state_colors(state: StreamState, p: &Palette) -> (Hsla, Hsla) {
    match state {
        StreamState::Streaming => (p.good, p.good_text),
        StreamState::Connecting | StreamState::Reconnecting => (p.warn, p.warn_text),
        StreamState::Error => (p.bad, p.bad_text),
        StreamState::Idle => (p.text_3, p.text_2),
    }
}

/// A small round status mark (`.dot`)
pub fn dot(color: Hsla) -> Div {
    div().flex_none().size(px(8.)).rounded_full().bg(color)
}

/// A tooltip builder for `.tooltip(..)` of an element with an id
pub fn tip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text: SharedString = text.into();
    move |window, cx| Tooltip::new(text.clone()).build(window, cx)
}

/// Asks the windows to draw again (a timer ran out, a busy mark cleared)
fn redraw(cx: &mut AsyncApp) {
    cx.update(|cx| cx.refresh_windows());
}

/// The current state of the core (None before the first snapshot)
fn snap(cx: &App) -> Option<ipc::Snapshot> {
    app::core(cx).read(cx).snap().cloned()
}

// ---------------------------------------------------------------------------
// The ring and the Connect button
// ---------------------------------------------------------------------------

/// The ring around the device: pulses while connecting, glows while
/// streaming. `id` tells the rings on screen apart (their pulse). `size` is
/// the CSS `--size` where it is shown (Home's cards 88 px, `.home-device
/// .ring`; the flyout 56 px, `.fly-head .ring-small`); `small`: the thin
/// ring of `.ring-small`.
pub fn ring_sized(
    id: impl Into<ElementId>,
    state: StreamState,
    volume: Option<u8>,
    size: f32,
    small: bool,
    child: impl IntoElement,
    cx: &App,
) -> AnyElement {
    let p = *cx.palette();
    let inner = if small { size - 8. } else { size - 20. };
    let glow = volume.map_or(0.6, |v| 0.35 + (0.65 * f32::from(v)) / 127.);
    let border = match state {
        StreamState::Streaming => p.accent_fill,
        StreamState::Connecting | StreamState::Reconnecting => p.warn_text,
        StreamState::Error => p.bad_text,
        StreamState::Idle => p.border_strong,
    };
    let base = div()
        .flex_none()
        .size(px(size))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .border_2()
        .border_color(border)
        .child(
            div()
                .size(px(inner))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(p.surface_2)
                .text_color(if state == StreamState::Streaming { p.text } else { p.text_2 })
                .child(child),
        );
    match state {
        // Streaming: the accent ring, glowing a little more at higher volume
        StreamState::Streaming => base
            .shadow(vec![
                BoxShadow {
                    color: p.accent_fill.opacity(0.45),
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(10. + 18. * glow),
                    spread_radius: px(-2. + 4. * glow),
                    inset: false,
                },
                BoxShadow {
                    color: p.accent_fill.opacity(0.5),
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(6. * glow),
                    spread_radius: px(0.),
                    inset: false,
                },
            ])
            .into_any_element(),
        StreamState::Connecting | StreamState::Reconnecting => base
            .with_animation(id, Animation::new(Duration::from_millis(1600)).repeat(), move |el, delta| {
                // Half way: a soft halo, the ring at half strength
                let k = 1. - (delta * 2. - 1.).abs();
                el.border_color(p.warn_text.opacity(1. - 0.5 * k)).shadow(vec![BoxShadow {
                    color: p.warn.opacity(0.18 * k),
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(8. * k),
                    inset: false,
                }])
            })
            .into_any_element(),
        _ => base.into_any_element(),
    }
}

/// The big Connect / Disconnect button's look (`.connect-btn`): the accent
/// fill to connect, grey while it stops a stream (`on`)
pub fn connect_button(id: impl Into<ElementId>, state: StreamState, on: bool, compact: bool) -> Button {
    let kind = if on { ButtonKind::Default } else { ButtonKind::Strong };
    ui::button(id, model::connect_label(state), kind)
        .icon(icon(if on { IconName::PlugDisconnected } else { IconName::Power }))
        .w_full()
        .h(px(if compact { 52. } else { 64. }))
        .rounded(theme::RADIUS_CARD)
        .font_family(theme::FONT_DISPLAY)
        .text_size(if compact { theme::FS_LARGE } else { theme::FS_TITLE })
        .font_weight(FontWeight::SEMIBOLD)
}

/// What a failed request is handed to (Home keeps it as its error)
pub type OnError = Rc<dyn Fn(ErrorInfo, &mut App)>;

/// Connect / Disconnect for the current profile; reports a failed request
/// through `on_error`
pub struct ConnectButton {
    busy: bool,
    compact: bool,
    on_error: Option<OnError>,
}

impl ConnectButton {
    pub fn new(compact: bool, _window: &mut Window, cx: &mut Context<Self>) -> ConnectButton {
        cx.observe(&app::core(cx), |_, _, cx| cx.notify()).detach();
        ConnectButton { busy: false, compact, on_error: None }
    }

    pub fn on_error(mut self, f: impl Fn(ErrorInfo, &mut App) + 'static) -> Self {
        self.on_error = Some(Rc::new(f));
        self
    }

    fn press(&mut self, cx: &mut Context<Self>) {
        let Some(snap) = snap(cx) else { return };
        let method = if model::stream_active(snap.stream.state) {
            Method::StreamStop
        } else if let Some(p) = current_profile(&snap) {
            Method::StreamStart { profile: p.name.clone() }
        } else {
            return;
        };
        self.busy = true;
        cx.notify();
        let task = app::request::<serde_json::Value>(method, cx);
        let on_error = self.on_error.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                cx.notify();
            });
            if let (Err(e), Some(f)) = (result, on_error) {
                cx.update(|cx| f(e, cx));
            }
        })
        .detach();
    }
}

impl Render for ConnectButton {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(snap) = snap(cx) else { return div().into_any_element() };
        let state = snap.stream.state;
        let active = model::stream_active(state);
        // A broadcast has no profile, and stops from here as a device does
        connect_button("connect", state, active, self.compact)
            .disabled(self.busy || (current_profile(&snap).is_none() && !active))
            .on_click(cx.listener(|this, _, _, cx| this.press(cx)))
            .into_any_element()
    }
}

// ---------------------------------------------------------------------------
// Volume
// ---------------------------------------------------------------------------

// The volume a mute replaced (percent), for the click that unmutes it
static UNMUTED_PERCENT: Mutex<Option<u32>> = Mutex::new(None);
/// Unmuting with no volume from before the mute (muted by the headphones)
const UNMUTE_DEFAULT: u32 = 30;

/// The volume of the device in use; `off`: a device not connected, whose
/// card keeps the slider disabled so the cards stay alike (issue #86).
/// The speaker mutes and unmutes, as Windows' does (issue #86): volume 0 on
/// the headphones, then the volume from before.
pub struct VolumeControl {
    off: bool,
    slider: Entity<SliderState>,
    // The value under the pointer while dragging, so the core's echo of an
    // older value does not make the thumb jump back
    held: Option<u32>,
    last_sent: Option<Instant>,
    pending: Option<Task<()>>,
    release: Option<Task<()>>,
}

impl VolumeControl {
    pub fn new(off: bool, _window: &mut Window, cx: &mut Context<Self>) -> VolumeControl {
        cx.observe(&app::core(cx), |_, _, cx| cx.notify()).detach();
        let slider = cx.new(|_| SliderState::new().min(0.).max(100.).step(1.));
        cx.subscribe(&slider, |this: &mut Self, _, ev: &SliderEvent, cx| match ev {
            SliderEvent::Change(v) => {
                let v = v.end().round() as u32;
                this.held = Some(v);
                this.release = None;
                this.send(v, cx);
            }
            SliderEvent::Release(v) => {
                this.send(v.end().round() as u32, cx);
                this.release = Some(cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                    cx.background_executor().timer(Duration::from_millis(600)).await;
                    let _ = this.update(cx, |this, cx| {
                        this.held = None;
                        cx.notify();
                    });
                }));
            }
        })
        .detach();
        VolumeControl { off, slider, held: None, last_sent: None, pending: None, release: None }
    }

    /// The card's device is (no longer) the one connected
    pub fn set_off(&mut self, off: bool, cx: &mut Context<Self>) {
        if self.off != off {
            self.off = off;
            cx.notify();
        }
    }

    fn volume(&self, cx: &App) -> Option<u8> {
        if self.off { None } else { app::core(cx).read(cx).snap().and_then(|s| s.stream.volume) }
    }

    fn shown(&self, cx: &App) -> u32 {
        self.held.unwrap_or_else(|| self.volume(cx).map_or(0, |v| volume_percent(u32::from(v))))
    }

    fn send(&mut self, pct: u32, cx: &mut Context<Self>) {
        // At most ten requests a second while dragging, and the final value
        self.pending = None;
        // The executor's clock: the wall clock, and the test clock in tests
        let go = move |this: &mut VolumeControl, cx: &mut Context<Self>| {
            this.last_sent = Some(cx.background_executor().now());
            let volume = percent_to_volume(pct).min(127) as u8;
            // Fire and forget: a failure shows as the slider coming back
            drop(app::request::<serde_json::Value>(Method::StreamSetVolume { volume }, cx));
        };
        let now = cx.background_executor().now();
        let since = self.last_sent.map_or(Duration::MAX, |t| now.saturating_duration_since(t));
        let Some(wait) = Duration::from_millis(100).checked_sub(since).filter(|w| !w.is_zero()) else {
            go(self, cx);
            return;
        };
        self.pending = Some(cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |this, cx| go(this, cx));
        }));
    }

    fn toggle_mute(&mut self, cx: &mut Context<Self>) {
        let shown = self.shown(cx);
        let mut unmuted = UNMUTED_PERCENT.lock().unwrap_or_else(|e| e.into_inner());
        if self.volume(cx).is_some() && shown == 0 {
            let back = unmuted.take().unwrap_or(UNMUTE_DEFAULT);
            drop(unmuted);
            self.send(back, cx);
        } else {
            *unmuted = Some(shown);
            drop(unmuted);
            self.send(0, cx);
        }
    }
}

impl Render for VolumeControl {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let disabled = self.volume(cx).is_none();
        let shown = self.shown(cx);
        if self.held.is_none() && self.slider.read(cx).value().end().round() as u32 != shown {
            self.slider.update(cx, |s, cx| s.set_value(shown as f32, window, cx));
        }
        // A device not connected is not muted: its speaker is just disabled
        let muted = !disabled && shown == 0;
        let label = t(if muted { "ui.volume.unmute" } else { "ui.volume.mute" });
        div()
            .flex()
            .items_center()
            .gap(theme::S3)
            .py(theme::S2)
            .px(theme::S1)
            .text_color(p.text_2)
            .when(disabled, |d| d.opacity(0.55))
            .child(
                ui::icon_button("volume-mute", if muted { IconName::SpeakerMute } else { IconName::Speaker2 }, label)
                    .disabled(disabled)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_mute(cx))),
            )
            .child(div().flex_1().min_w_0().child(Slider::new(&self.slider).disabled(disabled).accessibility_label(
                if disabled { format!("{}: {}", t("volume.label"), t("ui.volume.unknown")) } else { t("volume.label") },
            )))
            .child(div().min_w(px(48.)).text_right().text_color(p.text).child(if disabled {
                "–".to_owned()
            } else {
                format!("{shown}%")
            }))
    }
}

// ---------------------------------------------------------------------------
// Battery
// ---------------------------------------------------------------------------

/// The battery level: a small battery that fills and changes colour with
/// it, and the number, without the word beside the symbol (issue #86);
/// None is "unknown", with the reason on hover. `sub`: the number in the
/// secondary text colour (the navigation's block).
pub fn battery_level(id: impl Into<ElementId>, percent: Option<u8>, sub: bool, cx: &App) -> AnyElement {
    let p = cx.palette();
    let level = model::battery_level(percent);
    let color = match model::battery_tone(percent) {
        BatteryTone::Unknown => p.text_2,
        BatteryTone::Bad => p.bad,
        BatteryTone::Warn => p.warn,
        BatteryTone::Good => p.good,
    };
    let symbol = div()
        .flex()
        .flex_none()
        .items_center()
        .child(
            div()
                .w(px(19.))
                .h(px(12.))
                .p(px(1.5))
                .rounded(px(3.))
                .border_1()
                .border_color(color.opacity(0.7))
                .child(div().h_full().w(px(15. * f32::from(level) / 100.)).rounded(px(1.5)).bg(color)),
        )
        .child(div().w(px(2.)).h(px(5.)).rounded_r(px(1.)).bg(color.opacity(0.7)));
    let number = if percent.is_some() { format!("{level}%") } else { "?".to_owned() };
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(theme::S2)
        .child(symbol)
        .child(div().when(sub, |d| d.text_color(p.text_2)).child(number))
        .when(percent.is_none(), |d| d.tooltip(tip(t("ui.home.battery_refused_tip"))))
        .into_any_element()
}

// ---------------------------------------------------------------------------
// The codec
// ---------------------------------------------------------------------------

// The note shown and since when, for every badge: one that comes on screen
// later (another page) does not show it again
struct NoteShown {
    key: String,
    at: Instant,
    timer: bool,
}
static NOTE_SHOWN: Mutex<Option<NoteShown>> = Mutex::new(None);

/// The note for its first few seconds only (FALLBACK_NOTE_MS); the windows
/// draw again when it is over
fn note_fresh(key: Option<String>, cx: &App) -> bool {
    let mut shown = NOTE_SHOWN.lock().unwrap_or_else(|e| e.into_inner());
    let Some(key) = key else {
        *shown = None;
        return false;
    };
    if shown.as_ref().is_none_or(|s| s.key != key) {
        *shown = Some(NoteShown { key, at: Instant::now(), timer: false });
    }
    let Some(s) = shown.as_mut() else { return false };
    let Some(left) = Duration::from_millis(FALLBACK_NOTE_MS).checked_sub(s.at.elapsed()) else { return false };
    if !s.timer {
        s.timer = true;
        cx.spawn(async move |cx: &mut AsyncApp| {
            cx.background_executor().timer(left).await;
            redraw(cx);
        })
        .detach();
    }
    true
}

/// Where a [`codec_badge`] stands
#[derive(Default)]
pub struct CodecBadge {
    /// Only the bit rate (and ABR) in the badge
    pub compact: bool,
    /// Beside the badge (the flyout's link quality)
    pub aside: Option<AnyElement>,
    /// In the name's place: the codec chosen right there (Home, the flyout)
    pub picker: Option<AnyElement>,
    /// In the navigation's block: the name and the details in one box
    pub pill: bool,
    /// On a device's card: the chooser and the details end on one line
    pub stretch: bool,
}

/// The codec in two lines (issue #86): its name, then what it runs with in
/// a badge. Dimmed while a classic device's microphone is open: its codec
/// is paused for the call link (issue #86); the microphone has its own
/// line ([`mic_badge`]). None while nothing streams.
pub fn codec_badge(opts: CodecBadge, cx: &App) -> Option<AnyElement> {
    let p = *cx.palette();
    let core = app::core(cx);
    let s = &core.read(cx).snap()?.stream;
    let i = s.info.as_ref()?;
    let paused = s.voice.as_ref().is_some_and(|v| v.link == "hfp");
    // The bit rate lowered while the link is slow after the microphone (#86)
    let slow = s.slow_link;
    let message_key = s.message.key.as_str();
    let help = model::fallback_help(i, message_key);
    let note = model::fallback_note(i, message_key);
    let key = (!note.is_empty()).then(|| format!("{}|{}|{note}", s.profile.as_deref().unwrap_or(""), i.codec));
    let fresh = note_fresh(key, cx);

    let name = div()
        .id("codec-name")
        .flex()
        .items_center()
        .gap(theme::S1)
        .min_w_0()
        .max_w_full()
        .when(!opts.pill, |d| d.text_size(theme::FS_BODY))
        .text_color(p.text)
        .when(opts.stretch, |d| d.w_full())
        .when(i.requested_codec.is_some(), |d| {
            d.child(div().flex_none().text_color(p.warn_text).child(icon(IconName::Warning).small()))
                .tooltip(tip(help.clone()))
        })
        .map(|d| match opts.picker {
            Some(picker) => d.child(div().flex_1().min_w_0().child(picker)),
            None => d.child(div().flex_none().font_weight(FontWeight::SEMIBOLD).truncate().child(model::codec_name(i))),
        })
        .when(fresh, |d| {
            d.child(div().min_w_0().truncate().text_size(theme::FS_SMALL).text_color(p.text_2).child(note))
        });

    let mut badge = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_x(theme::S2)
        .max_w_full()
        .text_size(theme::FS_SMALL)
        .text_color(p.text_2)
        // As wide as its words first: what is beside it goes below it before
        // the words wrap (German, Korean)
        .when(opts.stretch, |d| d.flex_grow(1.))
        .when(!opts.pill, |d| {
            d.min_h(px(24.))
                .px(theme::S2)
                .border_1()
                .border_color(p.border_strong)
                .rounded(theme::RADIUS)
                .bg(p.surface_2)
        })
        .child(
            div()
                .id("codec-kbps")
                .child(format!("{} kbps", slow.unwrap_or(i.bitrate_kbps)))
                .when_some(model::bitrate_note(i), |d, n| d.tooltip(tip(n))),
        );
    if !opts.compact {
        badge = badge
            .child(div().child(format!("{} kHz", khz(i.sample_rate))))
            .when(i.bits > 0, |d| d.child(div().child(t(&format!("bitdepth.{}", i.bits)))));
    }
    // Last, as in the sidebar's line (issue #86)
    badge = badge
        .when(i.abr, |d| d.child(div().id("codec-abr").child(abr_label(i)).tooltip(tip(t(abr_tooltip_key(i))))))
        // LHDC V5 low latency in use (issue #543)
        .when(i.low_latency, |d| d.child(div().child(t("ui.codec.ll_on"))))
        // A line of its own: the others read the same for every codec (issue #86)
        .when_some(ui_model::codecdetails::live_bitpool(i), |d, bp| {
            d.child(div().id("codec-bitpool").w_full().child(bitpool_text(i).unwrap_or_default()).tooltip(tip(t!(
                "tooltip.sbc_bitpool_live",
                bp.min,
                bp.max
            ))))
        });

    let line = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_x(theme::S3)
        .gap_y(theme::S1)
        .min_w_0()
        .max_w_full()
        .when(opts.stretch, |d| d.w_full())
        .child(badge)
        .children(opts.aside);

    let slow_line = (slow.is_some() && !paused).then(|| {
        div()
            .flex()
            .items_start()
            .gap(theme::S1)
            .min_w_0()
            .text_size(theme::FS_SMALL)
            .text_color(p.text_2)
            .child(div().flex_none().text_color(p.warn_text).child(icon(IconName::Warning).small()))
            // Wraps within the card
            .child(div().flex_1().min_w_0().child(t("ui.codec.slow_link")))
    });

    Some(
        div()
            .flex()
            .flex_col()
            .items_start()
            .gap(if opts.pill { px(2.) } else { theme::S1 })
            .min_w_0()
            .max_w_full()
            .when(opts.stretch || opts.pill, |d| d.w_full().items_stretch())
            .when(opts.pill, |d| {
                d.py(theme::S1)
                    .px(theme::S2)
                    .border_1()
                    .border_color(p.border_strong)
                    .rounded(theme::RADIUS)
                    .bg(p.surface_2)
            })
            .when(paused, |d| d.opacity(0.45))
            .child(name)
            .child(line)
            .children(slow_line)
            .into_any_element(),
    )
}

/// The headset microphone's line below the codec (issue #86): "Mic: mSBC
/// 16 kHz" with a red dot while it is open, else dimmed with its mode (Off
/// while disabled on its button)
pub fn mic_badge(id: impl Into<ElementId>, profile: Option<&Profile>, cx: &App) -> Option<AnyElement> {
    let p = *cx.palette();
    let core = app::core(cx);
    let snap = core.read(cx).snap()?;
    if !mic_shown(snap, profile) {
        return None;
    }
    let profile = profile?;
    let voice = snap.stream.voice.as_ref();
    let content: Vec<AnyElement> = match voice {
        Some(v) => vec![
            div().child(voice_codec_name(&v.codec)).into_any_element(),
            div().child(format!("{} kHz", khz(v.rate))).into_any_element(),
        ],
        None => vec![div().child(model::mic_idle_text(profile, snap.settings.developer_mode)).into_any_element()],
    };
    Some(
        div()
            .id(id)
            .flex()
            .flex_1()
            .items_center()
            .gap_x(theme::S2)
            .min_w_0()
            .min_h(px(24.))
            .px(theme::S2)
            .border_1()
            .border_color(p.border_strong)
            .rounded(theme::RADIUS)
            .bg(p.surface_2)
            .text_size(theme::FS_SMALL)
            .text_color(if voice.is_some() { p.text } else { p.text_2 })
            .when(voice.is_none(), |d| d.opacity(0.55))
            .when(voice.is_some_and(|v| v.link == "hfp"), |d| d.tooltip(tip(t("ui.headset_mic_quality"))))
            .child(dot(if voice.is_some() { p.bad } else { p.text_3 }))
            .child(div().font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t("ui.mic_label")))
            .children(content)
            .into_any_element(),
    )
}

// Entries whose microphone mute is being saved: their chip waits
static MIC_SAVING: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

fn mic_saving(key: &str) -> bool {
    MIC_SAVING.lock().unwrap_or_else(|e| e.into_inner()).contains(key)
}

fn set_mic_saving(key: &str, on: bool) {
    let mut set = MIC_SAVING.lock().unwrap_or_else(|e| e.into_inner());
    if on {
        set.insert(key.to_owned());
    } else {
        set.remove(key);
    }
}

/// Saves the device's profile with the microphone muted or not -> true once saved
fn save_headset_mic(entry: &AudioEntry, mic_muted: bool, cx: &App) -> Task<bool> {
    crate::screens::codecs::save_headset_mic(entry, move |p| p.mic_muted = mic_muted, cx)
}

/// The headset microphone's mute at one click (Home's card and the
/// flyout): a microphone icon, crossed out while muted, which closes the
/// microphone link as Off does and keeps the mode chosen on Audio Devices
/// (Auto / On). Only for a device in use with headset mode on and its
/// microphone output chosen. An icon alone keeps the card still as it
/// switches (issue #86); Auto with no microphone link open shows dimmed,
/// and why the microphone did not open (MIC_NOTES) shows on it, not as the
/// card's status.
pub fn headset_chip(entry: &AudioEntry, cx: &App) -> Option<AnyElement> {
    let p = *cx.palette();
    let core = app::core(cx);
    let chip = model::headset_chip(core.read(cx).snap()?, entry)?;
    let name = if chip.note.is_some() {
        IconName::Warning
    } else if chip.muted {
        IconName::MicOff
    } else {
        IconName::Mic
    };
    let (line, color) = if chip.note.is_some() {
        (p.warn_text, p.warn_text)
    } else if chip.on && !chip.waiting {
        (p.accent_fill, p.accent_fill)
    } else {
        (p.border_strong, p.text_2)
    };
    let id: SharedString = format!("headset-chip-{}", entry.key).into();
    let saving = mic_saving(&entry.key);
    let entry = entry.clone();
    let muted = chip.muted;
    Some(
        Button::new(id)
            .ghost()
            .xsmall()
            .icon(icon(name).xsmall().text_color(color))
            .accessibility_label(chip.action)
            .tooltip(chip.title)
            .w(px(24.))
            .h(px(20.))
            .rounded_full()
            .border_1()
            .border_color(line)
            .when(chip.waiting, |b| b.border_dashed())
            .disabled(saving)
            .on_click(move |_, _, cx| {
                set_mic_saving(&entry.key, true);
                let task = save_headset_mic(&entry, !muted, cx);
                let key = entry.key.clone();
                cx.spawn(async move |cx: &mut AsyncApp| {
                    task.await;
                    set_mic_saving(&key, false);
                    redraw(cx);
                })
                .detach();
                cx.refresh_windows();
            })
            .into_any_element(),
    )
}

/// The microphone's button and its line below the codec (Home's card, the
/// flyout, the navigation's block; issue #86)
pub fn mic_row(entry: &AudioEntry, cx: &App) -> Option<AnyElement> {
    mic_row_with(entry, theme::S2, px(0.), cx)
}

/// The navigation's block: the button hangs out to the left, under the
/// state's dot, so the line starts where the codec's box does
pub fn mic_row_under_dot(entry: &AudioEntry, cx: &App) -> Option<AnyElement> {
    // The dot's button (16) and the gap to the text (8), less the row's own gap
    mic_row_with(entry, px(2.), px(-26.), cx)
}

fn mic_row_with(entry: &AudioEntry, gap: gpui_kit::Pixels, shift: gpui_kit::Pixels, cx: &App) -> Option<AnyElement> {
    let core = app::core(cx);
    if !mic_shown(core.read(cx).snap()?, entry.profile.as_ref()) {
        return None;
    }
    let id: SharedString = format!("mic-badge-{}", entry.key).into();
    Some(
        div()
            .flex()
            .items_center()
            .gap(gap)
            .ml(shift)
            .min_w_0()
            .children(headset_chip(entry, cx))
            .children(mic_badge(id, entry.profile.as_ref(), cx))
            .into_any_element(),
    )
}

// ---------------------------------------------------------------------------
// Link quality
// ---------------------------------------------------------------------------

/// The link's quality now: none unless it streams, whatever the last
/// samples said
fn quality_now(cx: &App) -> Option<LinkQuality> {
    let core = app::core(cx);
    let core = core.read(cx);
    if core.snap()?.stream.state != StreamState::Streaming {
        return None;
    }
    let samples: Vec<ipc::LinkSample> = core.store.link().iter().cloned().collect();
    link_quality(&samples)
}

/// Four bars and the quality's label; draw it from a view that redraws on
/// [`CoreEvent::Link`] ([`QualityIndicator`] does)
pub fn quality_indicator(cx: &App) -> AnyElement {
    let p = *cx.palette();
    let q = quality_now(cx);
    let label = quality_label(q.as_ref());
    let (st, st_text) = match q.as_ref().map(|q| q.status) {
        Some(QualityStatus::Good) => (p.good, p.good_text),
        Some(QualityStatus::Warn) => (p.warn, p.warn_text),
        Some(QualityStatus::Bad) => (p.bad, p.bad_text),
        None => (p.text_2, p.text_2),
    };
    let level = q.as_ref().map_or(0, |q| q.level);
    div()
        .id("quality")
        .flex()
        .items_center()
        .gap(theme::S2)
        .text_size(theme::FS_SMALL)
        .text_color(st_text)
        .tooltip(tip(format!("{}: {label}", t("quality.title"))))
        .child(div().flex().items_end().gap(px(2.)).h(px(14.)).children((1..=4u8).map(|n| {
            div().w(px(3.)).h(px(14. * f32::from(n) / 4.)).rounded(px(1.)).bg(if level >= n { st } else { p.surface_3 })
        })))
        .child(label)
        .into_any_element()
}

/// The link quality indicator as a view of its own: it redraws with every
/// link sample
pub struct QualityIndicator;

impl QualityIndicator {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> QualityIndicator {
        let core = app::core(cx);
        cx.observe(&core, |_, _, cx| cx.notify()).detach();
        cx.subscribe(&core, |_, _, ev: &CoreEvent, cx| {
            if *ev == CoreEvent::Link {
                cx.notify();
            }
        })
        .detach();
        QualityIndicator
    }
}

impl Render for QualityIndicator {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        quality_indicator(cx)
    }
}

/// Bandwidth over the last minute, while the view is on screen
pub struct LinkSparkline;

impl LinkSparkline {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> LinkSparkline {
        core::want(WantKind::Link, cx);
        cx.subscribe(&app::core(cx), |_, _, ev: &CoreEvent, cx| {
            if *ev == CoreEvent::Link {
                cx.notify();
            }
        })
        .detach();
        LinkSparkline
    }
}

impl Render for LinkSparkline {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let values: Vec<Option<f64>> =
            app::core(cx).read(cx).store.link().iter().map(|s| Some(kbps(s.bytes_per_second))).collect();
        let latest = values.last().copied().flatten().map(|v| v.round() as i64);
        let text = latest.map_or("–".to_owned(), |v| format!("{v} kbps"));
        div()
            .id("link-mini")
            .flex()
            .items_center()
            .gap(theme::S2)
            .min_w_0()
            .tooltip(tip(format!(
                "{}: {}",
                t("quality.bitrate"),
                latest.map_or("–".to_owned(), |v| format!("{v} kbps"))
            )))
            .child(
                div().flex_1().min_w_0().h(px(20.)).child(charts::sparkline(&values, Some(0.), None, cx).size_full()),
            )
            .child(div().flex_none().text_size(theme::FS_SMALL).text_color(p.text_2).child(text))
    }
}

// ---------------------------------------------------------------------------
// Errors and their fixes
// ---------------------------------------------------------------------------

/// The fix's button symbol
pub fn fix_icon(fix: &FixAction) -> IconName {
    match fix {
        FixAction::AssignWinusb(_) => IconName::Shield,
        FixAction::Adapter | FixAction::AdapterFirmware => IconName::UsbStick,
        FixAction::FirmwarePage(_) => IconName::Open,
        FixAction::RestartAdapter => IconName::ArrowReset,
        FixAction::Retry(_) => IconName::ArrowSync,
        FixAction::Codecs => IconName::Headphones,
        FixAction::Devices => IconName::Bluetooth,
        FixAction::Diagnostics => IconName::Pulse,
    }
}

/// The next step for an error: a code from a request, and/or the message
/// key (None before the first state)
pub fn fix_for(code: Option<&str>, key: &str, cx: &App) -> Option<FixAction> {
    let core = app::core(cx);
    let snap = core.read(cx).snap()?;
    Some(model::fix_for(snap, code, key))
}

/// Runs a fix -> done when its request (if any) ended; a failure shows as
/// a notice
pub fn run_fix(fix: &FixAction, window: &mut Window, cx: &mut App) -> Task<()> {
    let request = |method: Method, cx: &mut App| {
        let task = app::run(app::request::<serde_json::Value>(method, cx), cx);
        cx.spawn(async move |_: &mut AsyncApp| {
            task.await;
        })
    };
    match fix {
        FixAction::AssignWinusb(a) => {
            winusb_dialog((**a).clone(), WinusbMode::Assign, |_, _| {}, window, cx);
            Task::ready(())
        }
        FixAction::Adapter => {
            app::navigate(RouteName::Adapter, cx);
            Task::ready(())
        }
        FixAction::AdapterFirmware => {
            app::navigate(Route { name: RouteName::Adapter, arg: Some(AdapterSection::Firmware) }, cx);
            Task::ready(())
        }
        FixAction::FirmwarePage(url) => {
            host::open_url(url, cx);
            Task::ready(())
        }
        FixAction::RestartAdapter => request(Method::AdapterRestart, cx),
        FixAction::Retry(profile) => request(Method::StreamStart { profile: profile.clone() }, cx),
        FixAction::Codecs => {
            app::navigate(RouteName::Codecs, cx);
            Task::ready(())
        }
        FixAction::Devices => {
            app::navigate(RouteName::Devices, cx);
            Task::ready(())
        }
        FixAction::Diagnostics => {
            app::navigate(RouteName::Diagnostics, cx);
            Task::ready(())
        }
    }
}

/// The core's diagnostic info, masked, without its fence ("" when it does
/// not answer)
fn diag_info(cx: &App) -> Task<String> {
    let task = app::request::<DiagInfo>(Method::DiagInfo(Some(DiagInfoParams { mask: Some(true) })), cx);
    cx.spawn(async move |_: &mut AsyncApp| task.await.map(|d| model::diag_info_body(&d.text)).unwrap_or_default())
}

/// Copies a report, ready to paste: `head` (the error, the crash),
/// `details` (a crash report's text) and the diagnostic info
pub fn copy_report(head: String, details: String, cx: &App) {
    let info = diag_info(cx);
    cx.spawn(async move |cx: &mut AsyncApp| {
        let info = info.await;
        let report = model::report_text(&head, &details, &info);
        cx.update(|cx| {
            host::copy_text(report.text, cx);
            app::notify(t("ui.error.copied"), NoticeKind::Info, cx);
        });
    })
    .detach();
}

/// Opens a GitHub issue prefilled with a report; the whole text also goes
/// to the clipboard (the URL holds only part of a long log)
pub fn open_report(head: String, details: String, cx: &App) {
    let info = diag_info(cx);
    cx.spawn(async move |cx: &mut AsyncApp| {
        let info = info.await;
        let report = model::report_text(&head, &details, &info);
        cx.update(|cx| {
            let extra = BugReport { what_happened: Some(format!("{head}\n\n")), ..BugReport::default() };
            let url = match snap(cx) {
                Some(snap) => bug_report_for(&snap, &report.log, extra),
                None => bug_report_url(&BugReport { info: report.log.clone(), ..extra }),
            };
            host::open_url(&url, cx);
            host::copy_text(report.text, cx);
            app::notify(t("ui.error.report_opened"), NoticeKind::Info, cx);
        });
    })
    .detach();
}

/// Copies an error with the log around it
pub fn copy_error(message: &Msg, code: Option<&str>, cx: &App) {
    copy_report(error_copy_text(message, code), String::new(), cx);
}

/// A GitHub issue about an error
pub fn report_error(message: &Msg, code: Option<&str>, cx: &App) {
    open_report(error_copy_text(message, code), String::new(), cx);
}

// Error panels whose fix runs: their button waits
static FIX_RUNNING: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// An error with its cause and the button that fixes it. `id` tells the
/// panels on screen apart (their buttons, the fix running).
pub fn error_panel(
    id: &str,
    code: Option<&str>,
    message: &Msg,
    title: Option<&str>,
    cx: &App,
) -> gpui_kit::Stateful<Div> {
    let fix = fix_for(code, &message.key, cx);
    let busy = FIX_RUNNING.lock().unwrap_or_else(|e| e.into_inner()).contains(id);
    let (code, message_c) = (code.map(str::to_owned), message.clone());
    let actions = ui::inline()
        .when_some(fix, |d, f| {
            let key = id.to_owned();
            d.child(
                ui::button(SharedString::from(format!("{id}-fix")), f.label(), ButtonKind::Strong)
                    .icon(icon(fix_icon(&f)))
                    .loading(busy)
                    .disabled(busy)
                    .on_click(move |_, window, cx| {
                        FIX_RUNNING.lock().unwrap_or_else(|e| e.into_inner()).insert(key.clone());
                        let task = run_fix(&f, window, cx);
                        let key = key.clone();
                        cx.spawn(async move |cx: &mut AsyncApp| {
                            task.await;
                            FIX_RUNNING.lock().unwrap_or_else(|e| e.into_inner()).remove(&key);
                            redraw(cx);
                        })
                        .detach();
                        cx.refresh_windows();
                    }),
            )
        })
        .child({
            let (m, c) = (message_c.clone(), code.clone());
            ui::button(SharedString::from(format!("{id}-copy")), t("ui.error.copy"), ButtonKind::Subtle)
                .icon(icon(IconName::Copy))
                .tooltip(t("ui.error.copy_hint"))
                .on_click(move |_, _, cx| copy_error(&m, c.as_deref(), cx))
        })
        .when(is_app_fault(&message.key, code.as_deref()), |d| {
            let (m, c) = (message_c.clone(), code.clone());
            d.child(
                ui::button(SharedString::from(format!("{id}-report")), t("ui.diag.report_github"), ButtonKind::Subtle)
                    .icon(icon(IconName::Open))
                    .tooltip(t("ui.error.report_hint"))
                    .on_click(move |_, _, cx| report_error(&m, c.as_deref(), cx)),
            )
        });
    ui::banner(
        BannerKind::Error,
        Some(title.map_or_else(|| t("status.error"), str::to_owned).into()),
        Some(div().child(ui::text(tm(Some(message)))).into_any_element()),
        Some(actions.into_any_element()),
        None,
        cx,
    )
}

// ---------------------------------------------------------------------------
// WinUSB
// ---------------------------------------------------------------------------

// What Windows paired through the adapter (ui/paired_devices.rs)
pub use super::paired_devices::{paired_devices_note, windows_devices};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WinusbMode {
    Assign,
    Restore,
}

/// What the WinUSB dialog runs when it closes: whether it was done
type OnWinusbClose = Rc<dyn Fn(bool, &mut App)>;

/// Assign WinUSB / restore the driver, after saying what it does: while
/// BlueCaster has the adapter, Windows' Bluetooth does not see it.
/// `on_close` gets whether it was done.
pub fn winusb_dialog(
    adapter: AdapterInfo,
    mode: WinusbMode,
    on_close: impl Fn(bool, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    // Assign: the devices Windows loses with the adapter, by name (issue #86)
    let paired: Entity<Vec<WindowsPairedDevice>> = cx.new(|_| Vec::new());
    if mode == WinusbMode::Assign {
        let task = windows_devices(&adapter, cx);
        let paired = paired.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let list = task.await;
            paired.update(cx, |p, _| *p = list);
            redraw(cx);
        })
        .detach();
    }
    let on_close: OnWinusbClose = Rc::new(on_close);
    // Devices Windows loses: a warning, those devices first (issue #86)
    let losing = {
        let paired = paired.clone();
        move |cx: &App| mode == WinusbMode::Assign && !paired.read(cx).is_empty()
    };
    let question = {
        let (losing, on_close) = (losing.clone(), on_close.clone());
        move |_: &mut Window, cx: &mut App| {
            let losing = losing(cx);
            let title = if losing {
                t("ui.adapter.paired_title")
            } else if mode == WinusbMode::Assign {
                t("ui.adapter.assign_title")
            } else {
                t("ui.adapter.restore_title")
            };
            let confirm =
                t(if mode == WinusbMode::Assign { "ui.adapter.assign_winusb" } else { "ui.adapter.restore_driver" });
            let on_close = on_close.clone();
            let mut q = Question::new(title)
                .confirm(confirm)
                .confirm_icon(IconName::Shield)
                .on_cancel(move |_, cx| on_close(false, cx));
            if mode == WinusbMode::Restore || losing {
                q = q.danger();
            }
            if losing {
                q = q.warn();
            }
            q
        }
    };
    let body = {
        let adapter = adapter.clone();
        move |_: &mut Window, cx: &mut App| {
            let p = *cx.palette();
            div()
                .flex()
                .flex_col()
                .gap(theme::S3)
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_x(theme::S1)
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(adapter.display_name.clone()))
                        .child(div().text_color(p.text_2).child(format!("· {}", adapter.device_name))),
                )
                .map(|d| {
                    if losing(cx) {
                        d.child(paired_devices_note(paired.read(cx), cx))
                    } else {
                        d.child(div().child(t(if mode == WinusbMode::Assign {
                            "ui.adapter.assign_explain"
                        } else {
                            "ui.adapter.restore_explain"
                        })))
                    }
                })
                .child(div().text_color(p.text_2).child(t("ui.adapter.uac_note")))
                .into_any_element()
        }
    };
    let busy = dialog::busy_state(cx);
    dialog::ask_run(
        question,
        busy,
        body,
        move |_, cx| winusb_go(adapter.clone(), mode, on_close.clone(), cx),
        window,
        cx,
    );
}

/// The dialog closes however it ended
fn winusb_go(a: AdapterInfo, mode: WinusbMode, on_close: OnWinusbClose, cx: &mut App) -> Task<bool> {
    let id = a.instance_id.clone();
    let assign = (mode == WinusbMode::Assign)
        .then(|| app::request::<Vec<AdapterInfo>>(Method::AdapterAssignWinusb { instance_id: id.clone() }, cx));
    let restore = (mode == WinusbMode::Restore)
        .then(|| app::request::<serde_json::Value>(Method::AdapterRestoreDriver { instance_id: id.clone() }, cx));
    cx.spawn(async move |cx: &mut AsyncApp| {
        let done = match (assign, restore) {
            (Some(task), _) => match task.await {
                Ok(list) => {
                    // Windows loads WinUSB only after a replug or a restart (issue #81)
                    let restart = list.iter().any(|x| x.instance_id == id && x.restart_needed);
                    cx.update(|cx| {
                        if restart {
                            app::notify(t!("ui.adapter.assigned_restart", &a.display_name), NoticeKind::Error, cx);
                        } else {
                            app::notify(t!("ui.adapter.assigned", &a.display_name), NoticeKind::Info, cx);
                        }
                    });
                    Ok(())
                }
                Err(e) => Err(e),
            },
            (None, Some(task)) => match task.await {
                Ok(_) => {
                    cx.update(|cx| app::notify(t!("ui.adapter.restored", &a.display_name), NoticeKind::Info, cx));
                    Ok(())
                }
                Err(e) => Err(e),
            },
            (None, None) => Ok(()),
        };
        if let Err(e) = &done {
            cx.update(|cx| app::notify(app::error_text(e), NoticeKind::Error, cx));
        }
        cx.update(|cx| on_close(done.is_ok(), cx));
        true
    })
}

// ---------------------------------------------------------------------------
// Firmware
// ---------------------------------------------------------------------------

/// Opens the firmware folder (the shell creates it if it is not there yet)
pub fn open_firmware_folder(cx: &mut App) {
    let Some(snap) = snap(cx) else { return };
    host::open_folder(&format!("{}\\firmware", snap.config_dir), cx);
}

/// The button that opens the firmware folder. None while the core runs as
/// a service: its folder is protected, and only the download fetches files.
pub fn firmware_folder_button(id: impl Into<ElementId>, kind: ButtonKind, cx: &App) -> Option<AnyElement> {
    let core = app::core(cx);
    if core.read(cx).snap().is_some_and(|s| s.service_mode) {
        return None;
    }
    Some(
        ui::button(id, t("firmware.open_folder"), kind)
            .icon(icon(IconName::FolderOpen))
            .on_click(|_, _, cx| open_firmware_folder(cx))
            .into_any_element(),
    )
}

/// Fetches an adapter's Realtek files from linux-firmware once the user
/// has read where they come from and under which terms (issue #81)
pub fn firmware_download(id: impl Into<ElementId>, files: Vec<String>, kind: ButtonKind) -> Button {
    ui::button(id, t("firmware.download"), kind)
        .icon(icon(IconName::ArrowDownload))
        .on_click(move |_, window, cx| firmware_download_dialog(files.clone(), window, cx))
}

fn firmware_download_dialog(files: Vec<String>, window: &mut Window, cx: &mut App) {
    let busy = dialog::busy_state(cx);
    let file = files.first().cloned().unwrap_or_default();
    let shown = busy.clone();
    let body = move |_: &mut Window, cx: &mut App| {
        let p = *cx.palette();
        let working = *shown.read(cx);
        let body = if working {
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(theme::S3)
                .py(theme::S4)
                .child(Spinner::new().large())
                .child(t("firmware.downloading"))
        } else {
            div()
                .flex()
                .flex_col()
                .gap(theme::S3)
                .child(t("firmware.download_explain"))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .font_family(theme::FONT_MONO)
                        .children(files.iter().map(|f| div().child(format!("• {f}")))),
                )
                .child(div().text_color(p.text_2).child(t("firmware.download_license")))
        };
        body.into_any_element()
    };
    dialog::ask_run(
        |_, _| Question::new(t("firmware.download_title")).confirm(t("firmware.download")),
        busy,
        body,
        move |_, cx| {
            let task = app::request::<serde_json::Value>(Method::AdapterDownloadFirmware { file: file.clone() }, cx);
            cx.spawn(async move |cx: &mut AsyncApp| {
                let result = task.await;
                cx.update(|cx| match result {
                    Ok(_) => app::notify(t("firmware.downloaded"), NoticeKind::Info, cx),
                    // The UI stopped waiting, the core did not: it publishes the adapter
                    // anew when the files are saved, so this is no failure yet
                    Err(e) if e.code == "ipc.timeout" => app::notify(t("firmware.download_slow"), NoticeKind::Info, cx),
                    Err(e) => app::notify(app::error_text(&e), NoticeKind::Error, cx),
                });
                // Closes however it ended
                true
            })
        },
        window,
        cx,
    );
}

// ---------------------------------------------------------------------------
// Home's pieces other screens show
// ---------------------------------------------------------------------------

/// The adapter seems hung (issue #86: Windows lists an unknown USB device
/// in its place): only a power cycle brings it back. On Home, the Adapter
/// screen and in the setup
pub fn adapter_hung(cx: &App) -> Option<AnyElement> {
    let core = app::core(cx);
    if !core.read(cx).snap()?.adapter.hung {
        return None;
    }
    let body = div().child(t("ui.adapter.hung_text")).into_any_element();
    Some(
        ui::banner(BannerKind::Warn, Some(t("ui.adapter.hung_title").into()), Some(body), None, None, cx)
            .into_any_element(),
    )
}

/// v1 still runs (both cannot use the adapter): on Home and in the setup
pub fn v1_running(cx: &App) -> Option<AnyElement> {
    let core = app::core(cx);
    if !core.read(cx).snap()?.v1_running {
        return None;
    }
    let body = div().child(t("ui.v1_running.text")).into_any_element();
    Some(
        ui::banner(BannerKind::Warn, Some(t("ui.v1_running.title").into()), Some(body), None, None, cx)
            .into_any_element(),
    )
}

#[cfg(test)]
mod slider_tests;
