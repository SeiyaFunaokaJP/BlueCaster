//! Diagnostics > Radio, below the link figures: the channels the link hops
//! over (the controller's AFH map) on the 2.4 GHz band, the Wi-Fi channels
//! drawn under it, a few tips, and in developer mode a Wi-Fi scan: its
//! 2.4 GHz access points drawn over the same band, the table filtered by band.

use std::collections::BTreeSet;

use gpui_kit::component::Disableable as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AsyncApp, Context, Div, FontWeight, IntoElement, ParentElement as _, Render, Styled as _, WeakEntity, Window,
    div, px, relative,
};
use ipc::{LinkSample, Method, StreamState, WifiAccessPoint};
use ui_model::i18n::{lang, t};
use ui_model::radio::{
    BT_CHANNELS, BT_FIRST_MHZ, WIFI_CLEAR_CHANNELS, WIFI_HALF_WIDTH_MHZ, ap_level, busiest_wifi_channel,
    channels_in_use, parse_wifi_channels, wifi_center_mhz, wifi_covers,
};
use ui_model::t;
use ui_model::view::diagnostics::{BandFilter, band_of, near, scan_empty_key, shown};

use super::shared::{group, heading, shared, update_shared, want_link};
use crate::app;
use crate::host;
use crate::theme::{self, ActivePalette as _, Palette};
use crate::ui::charts::{Mark, ViewBox, plot};
use crate::ui::{self, BannerKind, ButtonKind, Choice, IconName, icon};

const DOCS: &str = "https://seiyafunaokajp.github.io/BlueCaster/";

// The drawing: 10 units per MHz, 2400-2490 MHz
const X0: f64 = 2400.0;
const W: f64 = 900.0;
const AP_TOP: f64 = 4.0;
const AP_H: f64 = 56.0;
const STRIP_Y: f64 = AP_TOP + AP_H + 4.0;
const STRIP_H: f64 = 28.0;
const AXIS_Y: f64 = STRIP_Y + STRIP_H + 16.0;
const H: f64 = AXIS_Y + 6.0;

fn x(mhz: f64) -> f64 {
    (mhz - X0) * 10.0
}

pub struct ChannelMap {
    busy: bool,
}

impl ChannelMap {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> ChannelMap {
        want_link(cx);
        cx.observe_global::<super::shared::DiagShared>(|_, cx| cx.notify()).detach();
        let mut this = ChannelMap { busy: false };
        // For looking at one screen: scan at once
        if crate::demo::diag_scan() {
            this.run_scan(cx);
        }
        this
    }

    fn run_scan(&mut self, cx: &mut Context<Self>) {
        self.busy = true;
        update_shared(cx, |s| s.scan_error = None);
        cx.notify();
        let task = app::request::<Vec<WifiAccessPoint>>(Method::DiagWifiScan, cx);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(list) => update_shared(cx, |s| s.scan = Some(list)),
                    Err(e) => update_shared(cx, |s| {
                        s.scan = None;
                        s.scan_error = Some(app::error_text(&e));
                    }),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// The band: access points of the last scan (louder ones higher), the 79
/// channels, Wi-Fi channels 1-13 under the strip
fn band(in_use: Option<&[bool]>, avoided: &BTreeSet<usize>, aps: &[&WifiAccessPoint], p: &Palette) -> Div {
    // Room above the strip only for access points to draw
    let top = if aps.is_empty() { STRIP_Y - 4.0 } else { 0.0 };
    let mut marks = Vec::new();
    for ap in aps {
        let c = wifi_center_mhz(ap.channel as u32) as f64;
        let y = AP_TOP + AP_H * (1.0 - ap_level(ap.rssi as f64));
        let l = x(c - WIFI_HALF_WIDTH_MHZ as f64);
        let r = x(c + WIFI_HALF_WIDTH_MHZ as f64);
        marks.push(Mark::Shape {
            points: vec![(l, STRIP_Y - 2.0), (l + 30.0, y), (r - 30.0, y), (r, STRIP_Y - 2.0)],
            fill: gpui_kit::Hsla { a: 0.18, ..p.chart_2 },
            stroke: Some((p.chart_2, 1.5)),
        });
    }
    for n in 0..BT_CHANNELS {
        let fill = match in_use.map(|u| u[n]) {
            None => p.surface_3,
            Some(true) => p.chart_1,
            Some(false) => gpui_kit::Hsla { a: 0.55, ..p.bad },
        };
        marks.push(Mark::Rect {
            x: x((BT_FIRST_MHZ as usize + n) as f64) + 1.0,
            y: STRIP_Y,
            w: 8.0,
            h: STRIP_H,
            fill,
            stroke: avoided.contains(&n).then_some((p.text_2, 1.5)),
        });
    }
    let view = ViewBox::new(0.0, top, W, H - top);
    let labels = (1u32..=13).map(|ch| {
        let strong = WIFI_CLEAR_CHANNELS.contains(&ch);
        let cx_frac = (x(wifi_center_mhz(ch) as f64) + 5.0) / W;
        div()
            .absolute()
            .left(relative(cx_frac as f32))
            .ml(px(-12.))
            .w(px(24.))
            .top(relative(((AXIS_Y - 13.0 - top) / (H - top)) as f32))
            .flex()
            .justify_center()
            .text_size(theme::FS_SMALL)
            .text_color(if strong { p.text } else { p.text_3 })
            .when(strong, |d| d.font_weight(FontWeight::SEMIBOLD))
            .child(ch.to_string())
    });
    div()
        .relative()
        .w_full()
        .aspect_ratio((W / (H - top)) as f32)
        .child(plot(view, marks, None).size_full())
        .children(labels)
}

fn swatch(color: gpui_kit::Hsla) -> Div {
    div().w(px(12.)).h(px(12.)).rounded(px(2.)).bg(color)
}

impl Render for ChannelMap {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let core = app::core(cx).read(cx);
        let snap = core.snap();
        let developer = snap.is_some_and(|s| s.settings.developer_mode);
        // The last reading while audio is sent
        let sample: Option<LinkSample> =
            snap.filter(|s| s.stream.state == StreamState::Streaming).and_then(|_| core.store.link().back().cloned());
        let state = shared(cx);
        let scan = state.scan.clone();
        let scan_error = state.scan_error.clone();
        let filter = state.band_filter;

        let in_use = channels_in_use(sample.as_ref().and_then(|s| s.afh_map.as_deref()));
        let used = in_use.as_ref().map_or(0, |u| u.iter().filter(|b| **b).count());
        // Bluetooth channels BlueCaster asked the adapter to avoid (Wi-Fi
        // channels of the AFH setting)
        let avoided: BTreeSet<usize> = match &sample {
            Some(s) if s.afh_usable.is_some() && !s.afh_avoided.is_empty() => {
                parse_wifi_channels(&s.afh_avoided).into_iter().flat_map(wifi_covers).collect()
            }
            _ => BTreeSet::new(),
        };
        let busiest = busiest_wifi_channel(in_use.as_deref());
        let scan_list = scan.clone().unwrap_or_default();
        let aps = near(&scan_list);

        let map = group(cx)
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .gap(theme::S3)
                    .child(heading(t("ui.radio.channels")))
                    .when(in_use.is_some(), |d| d.child(ui::muted(t!("ui.radio.in_use", used), cx))),
            )
            .when(in_use.is_none(), |d| d.child(ui::muted(t("quality.not_streaming"), cx)))
            .when(in_use.is_some() || !aps.is_empty(), |d| {
                d.child(band(in_use.as_deref(), &avoided, &aps, &p))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_x(theme::S4)
                            .gap_y(theme::S2)
                            .text_size(theme::FS_SMALL)
                            .when(in_use.is_some(), |d| {
                                d.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(theme::S1)
                                        .child(swatch(p.chart_1))
                                        .child(t("ui.radio.legend_on")),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(theme::S1)
                                        .child(swatch(gpui_kit::Hsla { a: 0.55, ..p.bad }))
                                        .child(t("ui.radio.legend_off")),
                                )
                            })
                            .when(!aps.is_empty(), |d| {
                                d.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(theme::S1)
                                        .child(swatch(p.chart_2))
                                        .child(t("ui.radio.scan_title")),
                                )
                            })
                            .child(div().text_color(p.text_2).child(t("ui.radio.axis"))),
                    )
                    .when_some(busiest, |d, b| d.child(ui::hint(t!("ui.radio.busy_near", b), cx)))
            });

        let docs_url =
            format!("{DOCS}{}wireless#checklist", if lang() == "en" { String::new() } else { format!("{}/", lang()) });
        let tips = group(cx)
            .child(heading(t("ui.radio.tips")))
            .child(
                div().flex().flex_col().gap(theme::S1).children(
                    ["ui.radio.tip_usb", "ui.radio.tip_dongles", "ui.radio.tip_wifi", "ui.radio.tip_one_link"]
                        .into_iter()
                        .map(|k| {
                            div().flex().gap(theme::S2).child("\u{2022}").child(div().flex_1().min_w_0().child(t(k)))
                        }),
                ),
            )
            .child(
                div().flex().child(
                    ui::button("radio-more", t("ui.radio.more"), ButtonKind::Subtle)
                        .icon(icon(IconName::Open))
                        .on_click(move |_, _, cx| host::open_url(&docs_url, cx)),
                ),
            );

        let scan_card = developer.then(|| {
            group(cx)
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .items_center()
                        .gap(theme::S3)
                        .child(heading(t("ui.radio.scan_title")))
                        .child(
                            ui::button("wifi-scan", t("ui.radio.scan"), ButtonKind::Default)
                                .icon(icon(IconName::Search))
                                .loading(self.busy)
                                .disabled(self.busy)
                                .on_click(cx.listener(|this, _, _, cx| this.run_scan(cx))),
                        ),
                )
                .when_some(scan_error, |d, e| {
                    d.child(ui::banner(
                        BannerKind::Error,
                        None,
                        Some(div().child(e).into_any_element()),
                        None,
                        None,
                        cx,
                    ))
                })
                .when_some(scan, |d, list| {
                    if list.is_empty() {
                        return d.child(ui::muted(t("ui.radio.scan_none"), cx));
                    }
                    let rows = shown(&list, filter);
                    d.child(div().flex().child(ui::segmented(
                        "wifi-band",
                        &filter,
                        vec![
                            Choice::new(BandFilter::G2_4, "2.4 GHz"),
                            Choice::new(BandFilter::G5, "5 GHz"),
                            Choice::new(BandFilter::G6, "6 GHz"),
                            Choice::new(BandFilter::All, t("ui.radio.band_all")),
                        ],
                        |v: &BandFilter, _, cx| {
                            let v = *v;
                            update_shared(cx, |s| s.band_filter = v);
                        },
                    )))
                    .child(if rows.is_empty() {
                        ui::muted(t(scan_empty_key(filter)), cx)
                    } else {
                        scan_table(&rows, cx)
                    })
                })
        });

        div().flex().flex_col().gap(theme::S4).child(map).child(tips).children(scan_card)
    }
}

/// The scan's access points (`.table`)
fn scan_table(rows: &[&WifiAccessPoint], cx: &App) -> Div {
    let p = cx.palette();
    let cell = |text: String, muted: bool| {
        div()
            .flex_1()
            .min_w_0()
            .px(theme::S2)
            .py(theme::S1)
            .truncate()
            .when(muted, |d| d.text_color(p.text_2))
            .child(text)
    };
    let line = || div().flex().border_b_1().border_color(p.border);
    div()
        .flex()
        .flex_col()
        .text_size(theme::FS_SMALL)
        .child(
            line()
                .font_weight(FontWeight::SEMIBOLD)
                .child(cell(t("ui.radio.network"), false))
                .child(cell(t("ui.radio.band"), false))
                .child(cell(t("ui.radio.channel"), false))
                .child(cell(t("ui.radio.signal"), false)),
        )
        .children(rows.iter().map(|ap| {
            let name = if ap.ssid.is_empty() { t("ui.radio.hidden") } else { ap.ssid.clone() };
            line()
                .child(cell(name, ap.ssid.is_empty()))
                .child(cell(format!("{} GHz", band_of(ap.mhz).as_str()), false))
                .child(cell(ap.channel.to_string(), false))
                .child(cell(format!("{} dBm", ap.rssi), false))
        }))
}
