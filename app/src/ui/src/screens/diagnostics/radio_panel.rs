//! Radio tab: the BR/EDR link. The link quality and what lowered it, RSSI,
//! channel hopping, the share of the radio's time the stream takes. In
//! developer mode the controller's own figures too (its link quality, failed
//! contacts) in the charts and the link card. The device card: the device's
//! Bluetooth chip for everyone; the link's role, mode, key size and
//! supervision timeout in developer mode. (The adapter's backlog is on the
//! Stream tab, next to the send queue it follows.) Below them the channel
//! map (ChannelMap).

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Entity, FontWeight, IntoElement, ParentElement as _, Render, Styled as _, Window, div,
};
use ipc::LinkDetails;
use ui_model::i18n::t;
use ui_model::linkstats::{EDR3_MAX_KBPS, gauge_value};
use ui_model::t;
use ui_model::view::charts::Status;
use ui_model::view::diagnostics::{
    BR_CHANNELS, afh_text, air_text, chip_packet_text, failed_text, fixed, key_size_text, link_quality_text, mode_text,
    role_text, round, rssi_db, rssi_value, supervision_text,
};
use ui_model::view::stream::{QUALITY_AIRTIME_MAX, QualityCause, QualityStatus, link_quality, quality_label};

use super::channel_map::ChannelMap;
use super::shared::{DASH, Link, group, groups, heading, link_charts, not_streaming, row, rows, want_link};
use crate::theme::{self, ActivePalette as _};
use crate::ui::charts::{ChartGroup, ChartHover, ChartRef, ChartSeries, RefKind, TimeChart, gauge, status_text};

const fn cause_key(c: QualityCause) -> &'static str {
    match c {
        QualityCause::Drops => "quality.dropped",
        QualityCause::Queue => "ui.diag.cause_queue",
        QualityCause::Airtime => "ui.diag.cause_airtime",
    }
}

fn pct(v: f64) -> String {
    format!("{} %", round(v))
}

pub struct RadioPanel {
    hover: Entity<ChartHover>,
    channel_map: Entity<ChannelMap>,
}

impl RadioPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> RadioPanel {
        want_link(cx);
        RadioPanel { hover: ChartGroup::hover_for(cx), channel_map: cx.new(|cx| ChannelMap::new(window, cx)) }
    }
}

impl Render for RadioPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let link = Link::read(cx);
        let developer = link.developer;
        let live = link.live().cloned();
        let sending = link.sending().cloned();
        let chip = live.as_ref().and_then(|s| s.remote_chip.as_deref());
        let details = live.as_ref().and_then(|s| s.link_details.as_deref());
        let detail = |f: fn(&LinkDetails) -> String| details.map_or(DASH.to_owned(), f);
        // An LE Audio stream has none of these figures; its counters are on the Stream tab
        let br_edr = link.le().is_none();

        let quality = if link.streaming() { link_quality(&link.samples) } else { None };
        let causes = quality.as_ref().map_or(String::new(), |q| {
            let mut list: Vec<String> = q.causes.iter().map(|c| t(cause_key(*c))).collect();
            if q.pc_busy {
                list.push(t("quality.capture_dropped"));
            }
            list.join(&t("radio.separator"))
        });
        let verdict_color = quality.as_ref().map_or(p.text, |q| {
            status_text(
                match q.status {
                    QualityStatus::Good => Status::Good,
                    QualityStatus::Warn => Status::Warn,
                    QualityStatus::Bad => Status::Bad,
                },
                &p,
            )
        });

        let theory_pct = sending.as_ref().and_then(|s| s.bandwidth_share).map(|b| b * 100.0);
        let air_pct = sending.as_ref().map(|s| s.airtime * 100.0);
        let air_high = air_pct.unwrap_or(0.0) >= QUALITY_AIRTIME_MAX * 100.0;

        let charts = br_edr.then(|| {
            let times = link.times.clone();
            let mut charts = vec![
                TimeChart::new(
                    "rssi",
                    t("ui.diag.rssi"),
                    times.clone(),
                    vec![ChartSeries::new(link.series(|s| s.rssi.map(f64::from)))],
                    rssi_db,
                )
                .min(-40.0)
                .max(Some(10.0))
                .refs(vec![ChartRef { value: 0.0, kind: RefKind::Target }])
                .format_value(|v, _| rssi_value(v)),
                TimeChart::new(
                    "afh",
                    t("ui.diag.afh"),
                    times.clone(),
                    vec![
                        ChartSeries::labelled(t("ui.diag.afh_in_use"), link.series(|s| s.afh_in_use.map(f64::from))),
                        ChartSeries::labelled(
                            t("ui.diag.afh_usable_short"),
                            link.series(|s| s.afh_usable.map(f64::from)),
                        ),
                    ],
                    |v| format!("{}", round(v)),
                )
                .min(0.0)
                .max(Some(BR_CHANNELS)),
                TimeChart::new(
                    "band",
                    t("quality.section_band"),
                    times.clone(),
                    vec![
                        ChartSeries::labelled(
                            t("quality.airtime"),
                            link.series(|s| (s.packets_per_second > 0.0).then_some(s.airtime * 100.0)),
                        ),
                        ChartSeries::labelled(
                            t("quality.theory"),
                            link.series(|s| {
                                s.bandwidth_share.filter(|_| s.packets_per_second > 0.0).map(|b| b * 100.0)
                            }),
                        ),
                    ],
                    pct,
                )
                .min(0.0)
                .max(Some(100.0))
                .refs(vec![ChartRef { value: QUALITY_AIRTIME_MAX * 100.0, kind: RefKind::Limit }])
                .format_value(|v, _| format!("{} %", fixed(v, 1))),
            ];
            if developer {
                charts.push(
                    TimeChart::new(
                        "hci-link-quality",
                        t("ui.diag.hci_link_quality"),
                        times,
                        vec![ChartSeries::new(
                            link.series(|s| s.link_details.as_ref().and_then(|d| d.link_quality).map(f64::from)),
                        )],
                        |v| format!("{}", round(v)),
                    )
                    .min(0.0)
                    .max(Some(255.0)),
                );
            }
            link_charts(&link, &self.hover, charts)
        });

        let link_card = group(cx)
            .child(heading(t("quality.title")))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_baseline()
                    .gap_x(theme::S3)
                    .gap_y(theme::S1)
                    .child(
                        div()
                            .text_size(theme::FS_LARGE)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(verdict_color)
                            .child(quality_label(quality.as_ref())),
                    )
                    .when(!causes.is_empty(), |d| {
                        d.child(div().text_size(theme::FS_SMALL).text_color(p.text_2).child(causes.clone()))
                    }),
            )
            .when(br_edr, |d| {
                let rssi = live.as_ref().and_then(|s| s.rssi).map_or(DASH.to_owned(), |r| rssi_value(r as f64));
                d.child(rows(
                    [
                        row(t("ui.diag.rssi"), rssi, None, None, cx),
                        row(t("ui.diag.afh"), afh_text(live.as_ref()), None, None, cx),
                    ]
                    .into_iter()
                    .chain(
                        developer
                            .then(|| row(t("ui.diag.hci_link_quality"), detail(link_quality_text), None, None, cx)),
                    )
                    .chain(developer.then(|| {
                        let failed = details.and_then(|d| d.failed_contacts_stream).unwrap_or(0) > 0;
                        row(t("ui.diag.failed_contacts"), detail(failed_text), failed.then_some(Status::Warn), None, cx)
                    })),
                ))
            });
        let mut cards = vec![link_card];

        if br_edr {
            // The device's chip (everyone), the link's state (developer mode)
            let mut device_rows = vec![
                row(
                    t("ui.diag.chip_maker"),
                    chip.and_then(|c| c.company.clone()).unwrap_or_else(|| DASH.into()),
                    None,
                    None,
                    cx,
                ),
                row(
                    t("ui.diag.chip_version"),
                    chip.and_then(|c| c.version.clone()).unwrap_or_else(|| DASH.into()),
                    None,
                    None,
                    cx,
                ),
                row(t("ui.diag.chip_packet"), chip_packet_text(chip), None, None, cx),
            ];
            if developer {
                device_rows.extend([
                    row(t("ui.diag.role"), detail(role_text), None, None, cx),
                    row(t("ui.diag.mode"), detail(mode_text), None, None, cx),
                    row(t("ui.diag.key_size"), detail(key_size_text), None, None, cx),
                    row(t("ui.diag.supervision_timeout"), detail(supervision_text), None, None, cx),
                ]);
            }
            cards.push(group(cx).child(heading(t("ui.diag.audio_device"))).child(rows(device_rows)));

            // The BR/EDR link's band: not for LE Audio
            let theory_text = theory_pct.map_or(DASH.to_owned(), |v| t!("quality.theory_value", EDR3_MAX_KBPS, v));
            let air_status = air_high.then_some(Status::Bad);
            cards.push(
                group(cx)
                    .child(heading(t("quality.section_band")))
                    .child(rows([
                        row(
                            t("quality.theory"),
                            theory_text,
                            None,
                            Some(gauge(theory_pct.map(|v| gauge_value(Some(v))), None, cx).into_any_element()),
                            cx,
                        ),
                        row(
                            t("quality.airtime"),
                            if air_pct.is_some() { air_text(sending.as_ref()) } else { DASH.to_owned() },
                            air_status,
                            Some(gauge(air_pct.map(|v| gauge_value(Some(v))), air_status, cx).into_any_element()),
                            cx,
                        ),
                    ]))
                    .child(crate::ui::hint(t("quality.band_help"), cx)),
            );
        }

        div()
            .flex()
            .flex_col()
            .gap(theme::S4)
            .when_some(not_streaming(&link, cx), |d, n| d.child(n))
            .children(charts)
            .child(groups(cards))
            .child(self.channel_map.clone())
    }
}
