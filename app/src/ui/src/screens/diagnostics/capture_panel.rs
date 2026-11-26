//! Audio capture tab: what Windows hands over. The capture rate against the
//! source's rate, audio lost at capture: dropped because the encoder fell
//! behind (the PC busy, not the radio) and lost in gaps Windows reports in the
//! captured audio (its own glitches); where the audio comes from. Per-app
//! capture counts its gaps without the audio lost in them: the count alone.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{Context, Entity, IntoElement, ParentElement as _, Render, Styled as _, Window, div};
use ui_model::i18n::t;
use ui_model::series::{capture_drops_per_sample, capture_lost_per_sample, rate_per_second};
use ui_model::view::charts::Status;
use ui_model::view::devices::format_text;
use ui_model::view::diagnostics::{capture_ms_text, fixed, gaps_text, khz_text};
use ui_model::view::stream::{current_profile, source_text};

use super::shared::{DASH, Link, group, groups, link_charts, loss_card, not_streaming, row, rows, want_link};
use crate::app;
use crate::theme;
use crate::ui::charts::{ChartGroup, ChartHover, ChartRef, ChartSeries, RefKind, TimeChart};

pub struct CapturePanel {
    hover: Entity<ChartHover>,
}

impl CapturePanel {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> CapturePanel {
        want_link(cx);
        CapturePanel { hover: ChartGroup::hover_for(cx) }
    }
}

impl Render for CapturePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let link = Link::read(cx);
        let rate = link.rate;
        let capture_rate = rate_per_second(&link.samples, |t| t.capture_frames);
        let drops: Vec<Option<f64>> = capture_drops_per_sample(&link.samples, rate).into_iter().map(Some).collect();
        let lost = capture_lost_per_sample(&link.samples, rate);
        // The rate the capture should run at (a scale of ±20 % around it)
        let nominal = if link.streaming() { rate as f64 } else { 0.0 };

        let charts = vec![
            TimeChart::new(
                "capture-rate",
                t("ui.diag.capture_rate"),
                link.times.clone(),
                vec![ChartSeries::new(capture_rate)],
                khz_text,
            )
            .min(if nominal > 0.0 { nominal * 0.8 } else { 0.0 })
            .max((nominal > 0.0).then_some(nominal * 1.2))
            .refs(if nominal > 0.0 { vec![ChartRef { value: nominal, kind: RefKind::Target }] } else { Vec::new() })
            .format_value(|v, _| format!("{} kHz", fixed(v / 1000.0, 2))),
            TimeChart::new(
                "capture-dropped",
                t("quality.capture_dropped"),
                link.times.clone(),
                vec![
                    ChartSeries::labelled(t("ui.quality.pc_busy"), drops),
                    ChartSeries::labelled(t("ui.diag.capture_gaps"), lost),
                ],
                capture_ms_text,
            )
            .min(0.0),
        ];

        let snap = app::core(cx).read(cx).snap().cloned();
        let info = link.stream.info.as_ref().filter(|_| link.streaming());
        let source = match (info.and_then(|i| i.source.as_ref()), &snap) {
            (Some(src), Some(snap)) => source_text(src, current_profile(snap)).text,
            _ => DASH.to_owned(),
        };
        let format =
            info.filter(|i| i.source_sample_rate > 0).map(|i| format_text(i.source_sample_rate, i.source_bits));

        let source_card = group(cx).child(rows(
            [row(t("capture.mode"), source, None, None, cx)]
                .into_iter()
                .chain(format.map(|f| row(t("ui.diag.source_format"), f, None, None, cx))),
        ));

        // Without the core's counters: the sample's own, since the stream started
        let last = link.last().cloned();
        let gaps_fallback = last.as_ref().map_or(DASH.to_owned(), |s| {
            let r = s.source_rate.filter(|&r| r != 0).unwrap_or(rate);
            let lost = (r > 0).then(|| (s.capture_lost.unwrap_or(0) as f64 * 1000.0) / r as f64);
            gaps_text(s.capture_discontinuities.unwrap_or(0), lost)
        });
        let loss = loss_card(
            &link,
            "lq-capture-loss-reset",
            |l| {
                let dropped = match l.capture_ms {
                    None => l.capture_dropped.to_string(),
                    Some(ms) => format!("{} ms", fixed(ms, 0)),
                };
                vec![
                    row(
                        t("quality.capture_dropped"),
                        format!("{dropped} ({}%)", fixed(l.capture_pct, 2)),
                        (l.capture_dropped > 0).then_some(Status::Bad),
                        None,
                        cx,
                    ),
                    row(
                        t("ui.diag.capture_gaps"),
                        gaps_text(l.capture_gaps, l.capture_lost_ms),
                        (l.capture_gaps > 0).then_some(Status::Warn),
                        None,
                        cx,
                    ),
                ]
            },
            vec![
                row(
                    t("quality.capture_dropped"),
                    last.as_ref().map_or(DASH.to_owned(), |s| s.capture_dropped.to_string()),
                    None,
                    None,
                    cx,
                ),
                row(t("ui.diag.capture_gaps"), gaps_fallback, None, None, cx),
            ],
            None,
            cx,
        );

        div()
            .flex()
            .flex_col()
            .gap(theme::S4)
            .when_some(not_streaming(&link, cx), |d, n| d.child(n))
            .child(link_charts(&link, &self.hover, charts))
            .child(groups([source_card, loss]))
    }
}
