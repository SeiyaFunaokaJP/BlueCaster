//! Stream tab: what goes out and how long it waits. Bitrate, packets, the
//! send queue (depth and the oldest packet's wait), packets dropped before
//! the radio; the codec and its details, an LE Audio stream's ISO counters
//! and its estimated latency,
//! the losses, and the backlog the Bluetooth adapter holds after the send
//! queue, and the playout delay the device reports. In developer mode the
//! encoder's load: its time per time of audio, and per frame. While a
//! classic device plays over its call link (the headset microphone open,
//! A2DP suspended) the figures are the call link's, measured the same way
//! (what only an A2DP stream has stays empty), the codec is the voice codec,
//! and the Microphone card counts what comes back.

use std::rc::Rc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{Context, Entity, IntoElement, ParentElement as _, Render, Styled as _, Window, div};
use ipc::LinkSample;
use ui_model::format::kbps;
use ui_model::i18n::t;
use ui_model::linkstats::{drops_per_sample, le_latency, ms_fine, percent};
use ui_model::t;
use ui_model::view::charts::Status;
use ui_model::view::diagnostics::{
    backlog_text, codec_text, count_text, encode_text, fixed, iso_count, ms_text, packets_text, pct_text, queue_status,
    queue_text, round, sink_delay_text, voice_link, wait_text,
};
use ui_model::view::stream::QUALITY_QUEUE_BACKLOG_MS;

use super::shared::{DASH, Link, group, groups, heading, link_charts, loss_card, not_streaming, row, rows, want_link};
use crate::theme;
use crate::ui::charts::{ChartGroup, ChartHover, ChartRef, ChartSeries, RefKind, TimeChart};
use crate::ui::codec_details::codec_details_card;

/// The microphone since the call link's samples began (the window shown)
struct Mic {
    packets: u64,
    bad: u64,
    concealed: u64,
}

fn mic(samples: &[LinkSample]) -> Option<Mic> {
    let last = samples.last()?.voice_rx?;
    let mut first = samples.len() - 1;
    while first > 0 && samples[first - 1].voice_rx.is_some() {
        first -= 1;
    }
    let base = samples[first].voice_rx?;
    Some(Mic {
        packets: last.packets.saturating_sub(base.packets),
        bad: last.bad.saturating_sub(base.bad),
        concealed: last.concealed.saturating_sub(base.concealed),
    })
}

/// The microphone's packets per sample damaged on the air
fn mic_bad(samples: &[LinkSample]) -> Vec<Option<f64>> {
    samples
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let p = i.checked_sub(1).map(|j| &samples[j])?;
            let (now, before) = (s.voice_rx?, p.voice_rx?);
            Some(now.bad.saturating_sub(before.bad) as f64)
        })
        .collect()
}

fn drop_status(v: f64) -> Status {
    if v == 0.0 {
        Status::Good
    } else if v < 3.0 {
        Status::Warn
    } else {
        Status::Bad
    }
}

pub struct StreamPanel {
    hover: Entity<ChartHover>,
}

impl StreamPanel {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> StreamPanel {
        want_link(cx);
        StreamPanel { hover: ChartGroup::hover_for(cx) }
    }
}

impl Render for StreamPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let link = Link::read(cx);
        let developer = link.developer;
        let voice = voice_link(&link.stream).cloned();
        let (live, sending) = (link.live().cloned(), link.sending().cloned());
        let times = link.times.clone();
        let backlog_ms = QUALITY_QUEUE_BACKLOG_MS as f64;

        // The encoder's bitrate: the line the actual bitrate should follow
        let target: Vec<ChartRef> = match &link.stream.info {
            Some(i) if link.streaming() && voice.is_none() && i.bitrate_kbps > 0 => {
                vec![ChartRef { value: i.bitrate_kbps as f64, kind: RefKind::Target }]
            }
            _ => Vec::new(),
        };
        let samples = Rc::new(link.samples.clone());
        let mut charts = vec![
            TimeChart::new(
                "bitrate",
                t("quality.bitrate"),
                times.clone(),
                vec![ChartSeries::new(link.series(|s| Some(kbps(s.bytes_per_second))))],
                |v| format!("{} kbps", round(v)),
            )
            .min(0.0)
            .refs(target),
            TimeChart::new(
                "packets",
                t("quality.packets"),
                times.clone(),
                vec![ChartSeries::new(link.series(|s| Some(s.packets_per_second)))],
                |v| format!("{} /s", round(v)),
            )
            .min(0.0),
            TimeChart::new(
                "queue",
                t("quality.queue"),
                times.clone(),
                vec![ChartSeries::new(link.series(|s| Some(s.queue_depth as f64)))],
                count_text,
            )
            .min(0.0)
            .status(|v| {
                if v < 8.0 {
                    Status::Good
                } else if v < 16.0 {
                    Status::Warn
                } else {
                    Status::Bad
                }
            }),
            TimeChart::new(
                "queue-wait",
                t("ui.diag.queue_wait"),
                times.clone(),
                vec![ChartSeries::new(link.series(|s| s.queue_ms.map(f64::from)))],
                ms_text,
            )
            .min(0.0)
            .refs(vec![ChartRef { value: backlog_ms, kind: RefKind::Limit }])
            .status(move |v| {
                if v < backlog_ms / 2.0 {
                    Status::Good
                } else if v < backlog_ms {
                    Status::Warn
                } else {
                    Status::Bad
                }
            }),
            TimeChart::new(
                "backlog",
                t("ui.diag.backlog"),
                times.clone(),
                vec![ChartSeries::new(link.series(|s| s.controller_backlog_ms))],
                ms_text,
            )
            .min(0.0)
            .format_value(move |_, i| backlog_text(samples.get(i))),
            TimeChart::new(
                "dropped",
                t("quality.dropped"),
                times.clone(),
                vec![ChartSeries::new(drops_per_sample(&link.samples).into_iter().map(|d| Some(d as f64)).collect())],
                count_text,
            )
            .min(0.0)
            .status(drop_status),
        ];
        if voice.is_some() {
            charts.push(
                TimeChart::new(
                    "mic-damaged",
                    t("ui.diag.mic_damaged"),
                    times.clone(),
                    vec![ChartSeries::new(mic_bad(&link.samples))],
                    count_text,
                )
                .min(0.0)
                .status(drop_status),
            );
        }
        if developer {
            charts.push(
                TimeChart::new(
                    "encode-load",
                    t("ui.diag.encode_load"),
                    times,
                    vec![ChartSeries::new(link.series(|s| s.encode.as_ref().map(|e| e.load * 100.0)))],
                    pct_text,
                )
                .min(0.0),
            );
        }

        let queue_st = live.as_ref().map(queue_status);
        let send = group(cx)
            .child(rows([row(t("quality.codec"), codec_text(&link.stream), None, None, cx)]))
            .child(heading(t("quality.section_send")))
            .child(rows(
                [
                    row(
                        t("quality.bitrate"),
                        sending.as_ref().map_or(DASH.into(), |s| t!("quality.bitrate_value", kbps(s.bytes_per_second))),
                        None,
                        None,
                        cx,
                    ),
                    row(t("quality.packets"), packets_text(sending.as_ref()), None, None, cx),
                    row(t("quality.queue"), queue_text(live.as_ref()), queue_st, None, cx),
                    row(t("ui.diag.queue_wait"), wait_text(live.as_ref()), queue_st, None, cx),
                    row(t("ui.diag.backlog"), backlog_text(live.as_ref()), None, None, cx),
                    row(t("quality.sink_delay"), sink_delay_text(live.as_ref()), None, None, cx),
                ]
                .into_iter()
                .chain(developer.then(|| row(t("ui.diag.encode_load"), encode_text(live.as_ref()), None, None, cx))),
            ));

        let mut cards = vec![send];
        // What the A2DP codec runs with and what the device offered
        if link.streaming()
            && link.le().is_none()
            && voice.is_none()
            && let Some(i) = &link.stream.info
            && let Some(d) = &i.details
        {
            cards.push(codec_details_card(i, d, cx));
        }
        if let Some(m) = mic(&link.samples) {
            cards.push(
                group(cx)
                    .child(heading(t("ui.diag.mic_title")))
                    .child(rows([
                        row(t("ui.diag.mic_received"), m.packets.to_string(), None, None, cx),
                        row(
                            t("ui.diag.mic_damaged"),
                            format!("{} ({}%)", m.bad, fixed(percent(m.bad as f64, m.packets as f64), 1)),
                            (m.bad > 0).then_some(Status::Warn),
                            None,
                            cx,
                        ),
                        row(t("ui.diag.mic_concealed"), m.concealed.to_string(), None, None, cx),
                    ]))
                    .child(crate::ui::hint(t("ui.diag.mic_help"), cx)),
            );
        }
        let iso = link.last().and_then(|s| s.iso);
        if link.le().is_some() || iso.is_some() {
            let warn = |n: Option<u64>, st: Status| (n.unwrap_or(0) > 0).then_some(st);
            let latency = le_latency(link.le(), iso.as_ref());
            let latency_parts = latency.map(|l| {
                crate::ui::hint(
                    t!(
                        "ui.diag.iso_latency_parts",
                        ms_fine(l.codec),
                        ms_fine(l.buffer),
                        l.radio.map_or(DASH.into(), ms_fine),
                        ms_fine(l.device)
                    ),
                    cx,
                )
                .into_any_element()
            });
            cards.push(
                group(cx)
                    .child(heading(t("ui.diag.iso_title")))
                    .child(rows([
                        row(
                            t("ui.diag.iso_config"),
                            link.le().map_or(DASH.into(), |l| l.summary.clone()),
                            None,
                            None,
                            cx,
                        ),
                        row(t("ui.diag.iso_cis"), iso_count(iso.map(|i| i.cis as u64)), None, None, cx),
                        row(t("ui.diag.iso_sdu_sets"), iso_count(iso.map(|i| i.sdu_sets)), None, None, cx),
                        row(
                            t("ui.diag.iso_silence"),
                            iso_count(iso.map(|i| i.silence)),
                            warn(iso.map(|i| i.silence), Status::Warn),
                            None,
                            cx,
                        ),
                        row(
                            t("ui.diag.iso_underruns"),
                            iso_count(iso.map(|i| i.underruns)),
                            warn(iso.map(|i| i.underruns), Status::Warn),
                            None,
                            cx,
                        ),
                        row(
                            t("ui.diag.iso_dropped"),
                            iso_count(iso.map(|i| i.dropped)),
                            warn(iso.map(|i| i.dropped), Status::Bad),
                            None,
                            cx,
                        ),
                        row(
                            t("ui.diag.iso_prefill"),
                            iso.map_or(DASH.into(), |i| t!("ui.diag.iso_prefill_value", i.prefill)),
                            None,
                            None,
                            cx,
                        ),
                        row(
                            t("ui.diag.iso_latency"),
                            latency.map_or(DASH.into(), |l| t!("ui.diag.iso_latency_value", l.total.round() as i64)),
                            None,
                            latency_parts,
                            cx,
                        ),
                    ]))
                    .child(crate::ui::hint(t("ui.diag.iso_help"), cx))
                    .when(latency.is_some(), |d| d.child(crate::ui::hint(t("ui.diag.iso_latency_hint"), cx))),
            );
        }
        let last_dropped = link.last().map(|s| s.dropped);
        cards.push(loss_card(
            &link,
            "lq-loss-reset",
            |l| {
                vec![
                    row(t("quality.sent_total"), l.sent.to_string(), None, None, cx),
                    row(
                        t("quality.dropped"),
                        format!("{} ({}%)", l.dropped, fixed(l.drop_pct, 2)),
                        (l.dropped > 0).then_some(Status::Bad),
                        None,
                        cx,
                    ),
                ]
            },
            vec![row(t("quality.dropped"), last_dropped.map_or(DASH.into(), |d| d.to_string()), None, None, cx)],
            Some(t("quality.loss_help")),
            cx,
        ));

        div()
            .flex()
            .flex_col()
            .gap(theme::S4)
            .when_some(not_streaming(&link, cx), |d, n| d.child(n))
            .child(link_charts(&link, &self.hover, charts))
            .child(groups(cards))
    }
}
