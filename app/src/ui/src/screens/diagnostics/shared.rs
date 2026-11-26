//! What the Diagnostics tabs share: the window-wide view state (masking, the
//! losses' Reset baseline, the core's clock), the link samples of the radio,
//! stream and capture tabs, their charts' common time axis, and the losses
//! card.

use std::rc::Rc;
use std::sync::LazyLock;

use gpui_kit::component::Disableable as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, BorrowAppContext as _, Context, Div, Entity, FontWeight, Global, ParentElement as _, SharedString,
    Styled as _, div, px,
};
use ipc::{CaptureState, DiagBundle, LeStreamInfo, LinkSample, StreamState, StreamStatus, WifiAccessPoint};
use ui_model::i18n::t;
use ui_model::linkstats::{LinkBaseline, LossStats, loss_stats, take_baseline};
use ui_model::logview::{core_time_of_day, next_epoch, time_of_day};
use ui_model::store::LINK_KEEP;
use ui_model::t;
use ui_model::view::charts::Status;
use ui_model::view::diagnostics::BandFilter;

use crate::app;
use crate::core::{self, CoreEvent, WantKind};
use crate::theme::{self, ActivePalette as _};
use crate::ui::charts::{ChartGroup, ChartHover, TimeChart, chart_grid, status_text};
use crate::ui::{self, ButtonKind, IconName, icon};

// ---- Window-wide view state (kept while the window lives, so it is still
// there when the user comes back to the screen) ----------------------------

pub struct DiagShared {
    /// Mask Bluetooth addresses in the log and in every copy
    pub mask: bool,
    /// The link counters when Reset was pressed
    pub reset_base: Option<LinkBaseline>,
    /// Without the core's clock: the wall clock (ms since 1970) at the
    /// core's time 0, estimated from live samples
    pub epoch: Option<i64>,
    last_core_time: i64,
    // The Diagnostics screen's
    /// The capture stopped last (file size, "show in Explorer")
    pub saved_capture: Option<CaptureState>,
    /// The diagnostic bundle saved last
    pub saved_bundle: Option<DiagBundle>,
    /// The bundle's HCI captures: off unless asked for (they hold Bluetooth
    /// addresses and device names, and most reports do not need them)
    pub with_hci: bool,
    // The channel map's
    /// The last Wi-Fi scan
    pub scan: Option<Vec<WifiAccessPoint>>,
    pub scan_error: Option<String>,
    /// The table's band: 2.4 GHz first, the only one Bluetooth shares
    pub band_filter: BandFilter,
}

impl Default for DiagShared {
    fn default() -> Self {
        DiagShared {
            mask: true,
            reset_base: None,
            epoch: None,
            last_core_time: -1,
            saved_capture: None,
            saved_bundle: None,
            with_hci: false,
            scan: None,
            scan_error: None,
            band_filter: BandFilter::default(),
        }
    }
}

impl Global for DiagShared {}

static DEFAULT: LazyLock<DiagShared> = LazyLock::new(DiagShared::default);

/// The window-wide state
pub fn shared(cx: &App) -> &DiagShared {
    cx.try_global::<DiagShared>().unwrap_or(&DEFAULT)
}

/// Changes the window-wide state; the views observing it draw again
pub fn update_shared(cx: &mut App, f: impl FnOnce(&mut DiagShared)) {
    if !cx.has_global::<DiagShared>() {
        cx.set_global(DiagShared::default());
    }
    cx.update_global::<DiagShared, _>(|s, _| f(s));
}

/// Mask Bluetooth addresses in the log and in every copy
pub fn mask(cx: &App) -> bool {
    shared(cx).mask
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// A live sample's core time: the estimate of the core's time 0 gets better
pub fn observe_core_time(time_ms: u64, cx: &mut App) {
    update_shared(cx, |s| {
        // The core's clock went back: a new core process
        if (time_ms as i64) + 1000 < s.last_core_time {
            s.epoch = None;
        }
        s.last_core_time = time_ms as i64;
        s.epoch = Some(next_epoch(s.epoch, time_ms, now_ms()));
    });
}

/// A core time as the time of day (v1): from the core's clock, else from
/// the estimate, else as the core's uptime until the clock is known
pub fn time_text(ms: u64, cx: &App) -> String {
    if let Some(c) = app::core(cx).read(cx).snap().and_then(|s| s.clock_ms) {
        return core_time_of_day(ms, c);
    }
    match shared(cx).epoch {
        Some(e) => time_of_day(ms, e),
        None => ui_model::format::clock(ms),
    }
}

// ---- Link samples ----------------------------------------------------------

pub use ui_model::view::diagnostics::DASH;

/// The time the charts show: every sample the store keeps (4 a second)
pub const CHART_SPAN_MS: f64 = LINK_KEEP as f64 * 250.0;

/// Asks for the link samples while the view `cx` belongs to lives (the
/// calling tab is open), and draws it again as they come
pub fn want_link<V: 'static>(cx: &mut Context<V>) {
    core::want(WantKind::Link, cx);
    let core = app::core(cx);
    cx.subscribe(&core, |_, _, ev: &CoreEvent, cx| {
        if *ev == CoreEvent::Link {
            cx.notify();
        }
    })
    .detach();
    cx.observe(&core, |_, _, cx| cx.notify()).detach();
}

/// The link samples and the stream, read for one drawing
pub struct Link {
    pub samples: Vec<LinkSample>,
    pub times: Rc<[f64]>,
    pub stream: StreamStatus,
    pub developer: bool,
    /// Capture drops in ms after the stream stopped: the last known rate
    pub rate: u32,
    reset: Option<LinkBaseline>,
}

impl Link {
    pub fn read(cx: &App) -> Link {
        let core = app::core(cx).read(cx);
        let samples: Vec<LinkSample> = core.store.link().iter().cloned().collect();
        let times: Rc<[f64]> = samples.iter().map(|s| s.time_ms as f64).collect();
        let (stream, developer) =
            core.snap().map(|s| (s.stream.clone(), s.settings.developer_mode)).unwrap_or_default();
        let rate = samples
            .iter()
            .rev()
            .find_map(|s| s.source_rate.filter(|&r| r != 0))
            .or_else(|| stream.info.as_ref().map(|i| i.source_sample_rate))
            .unwrap_or(0);
        Link { samples, times, stream, developer, rate, reset: shared(cx).reset_base }
    }

    pub fn streaming(&self) -> bool {
        self.stream.state == StreamState::Streaming
    }

    pub fn last(&self) -> Option<&LinkSample> {
        self.samples.last()
    }

    /// The last sample while streaming
    pub fn live(&self) -> Option<&LinkSample> {
        self.last().filter(|_| self.streaming())
    }

    /// The last sample while packets go out (the "last second" rows)
    pub fn sending(&self) -> Option<&LinkSample> {
        self.live().filter(|s| s.packets_per_second > 0.0)
    }

    /// LE Audio: the stream that was set up
    pub fn le(&self) -> Option<&LeStreamInfo> {
        self.stream.info.as_ref().and_then(|i| i.le.as_ref()).filter(|_| self.streaming())
    }

    /// Losses since the stream started or since Reset
    pub fn loss(&self) -> Option<LossStats> {
        self.last().and_then(|s| loss_stats(s, self.reset.as_ref(), Some(self.rate)))
    }

    /// One value per sample
    pub fn series(&self, pick: impl Fn(&LinkSample) -> Option<f64>) -> Vec<Option<f64>> {
        self.samples.iter().map(pick).collect()
    }
}

/// The charts of a tab: one time axis, one hover cursor
pub fn link_charts(link: &Link, hover: &Entity<ChartHover>, charts: Vec<TimeChart>) -> Div {
    let group = ChartGroup::new(link.last().map(|s| s.time_ms as f64), CHART_SPAN_MS, hover.clone());
    chart_grid(charts.into_iter().map(|c| c.group(&group)).collect())
}

pub fn not_streaming(link: &Link, cx: &App) -> Option<Div> {
    (!link.streaming()).then(|| ui::muted(t("quality.not_streaming"), cx))
}

/// A label and its value (`.lq-row`), the value in a status colour, with
/// something under it (a gauge)
pub fn row(
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    status: Option<Status>,
    under: Option<AnyElement>,
    cx: &App,
) -> Div {
    let p = cx.palette();
    div()
        .flex()
        .items_start()
        .gap(theme::S3)
        .child(
            div()
                .w(gpui_kit::relative(0.4))
                .min_w(px(128.))
                .flex_none()
                .pt(px(2.))
                .text_size(theme::FS_SMALL)
                .text_color(p.text_2)
                .child(label.into()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(theme::S1)
                .text_color(status.map_or(p.text, |s| status_text(s, p)))
                .child(value.into())
                .children(under),
        )
}

/// The rows of a card (`.lq-rows`)
pub fn rows(children: impl IntoIterator<Item = Div>) -> Div {
    div().flex().flex_col().gap(theme::S2).children(children)
}

/// A card of rows (`.card.lq-group`)
pub fn group(cx: &App) -> Div {
    ui::card(cx).pt(theme::S3)
}

/// A card's heading (`.lq-group h2`)
pub fn heading(text: impl Into<SharedString>) -> Div {
    div().text_size(theme::FS_BODY).font_weight(FontWeight::SEMIBOLD).child(text.into())
}

/// The cards of a tab side by side, wrapping (`.lq-groups`: at least 22rem each)
pub fn groups(cards: impl IntoIterator<Item = Div>) -> Div {
    div().flex().flex_wrap().items_start().gap(theme::S4).children(
        cards.into_iter().map(|c| div().flex_grow(1.).flex_shrink(1.).flex_basis(px(352.)).min_w(px(300.)).child(c)),
    )
}

/// Losses since the stream started or since Reset (one baseline for every tab)
pub fn loss_card(
    link: &Link,
    id: &'static str,
    rows_of: impl FnOnce(&LossStats) -> Vec<Div>,
    fallback: Vec<Div>,
    hint: Option<String>,
    cx: &App,
) -> Div {
    let loss = link.loss();
    let title = match (&loss, shared(cx).reset_base) {
        (Some(l), Some(base)) if l.since_reset => t!("ui.diag.loss_since_reset", time_text(base.time_ms, cx)),
        _ => t("ui.diag.loss_since_stream"),
    };
    let last = link.last().cloned();
    let reset = ui::button(id, t("quality.reset"), ButtonKind::Subtle)
        .icon(icon(IconName::ArrowReset))
        .disabled(last.as_ref().is_none_or(|s| s.counters.is_none()))
        .on_click(move |_, _, cx| {
            let base = take_baseline(last.as_ref());
            update_shared(cx, |s| s.reset_base = base);
        });
    group(cx)
        .child(div().flex().justify_between().items_center().gap(theme::S3).child(heading(title)).child(reset))
        .child(rows(match &loss {
            Some(l) => rows_of(l),
            None => fallback,
        }))
        .when_some(hint, |d, h| d.child(ui::hint(h, cx)))
}
