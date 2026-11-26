//! Diagnostics' codec details card: what an A2DP stream's codec runs with,
//! next to what the device offered (the SBC bitpool range, channel modes,
//! AAC object types ...; for the other codecs the rates, bit depths and peak
//! bit rate their endpoint lists, issue #86), and how the codec frames fill
//! a packet. The rows and their texts are ui_model::view::codec_details.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Div, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Role, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, div, px, relative,
};
use ipc::{CodecDetails, StreamInfo};
use ui_model::codecdetails::{BarSpan, ChipState};
use ui_model::view::codec_details::{BitpoolPart, BitpoolView, CdChip, CdExtra, CdRow, CdSection, codec_details_view};

use crate::theme::{self, ActivePalette as _};
use crate::ui::{self, charts};

/// A label and its value (`.lq-row`: the label column, the value beside it)
fn row(label: &str, value: impl IntoElement, cx: &App) -> Div {
    div()
        .flex()
        .items_start()
        .gap(theme::S3)
        .child(
            div()
                .w(relative(0.4))
                .min_w(px(128.))
                .flex_shrink_0()
                .pt(px(2.))
                .text_size(theme::FS_SMALL)
                .text_color(cx.palette().text_2)
                .child(label.to_owned()),
        )
        .child(div().flex().flex_col().flex_1().min_w_0().gap(theme::S1).child(value))
}

/// A setting as chips: in use, offered by the device, not offered (a
/// screen reader hears which)
fn chip(c: &CdChip, cx: &App) -> Stateful<Div> {
    let p = cx.palette();
    let name: SharedString = c.accessible_text().into();
    let base = div()
        // By its words: the rows of one card hold chips of the same number
        .id(super::child_id("cd-chip", &name))
        .role(Role::ListItem)
        .aria_label(name)
        .px(theme::S2)
        .h(px(20.))
        .flex()
        .items_center()
        .rounded_full()
        .border_1()
        .text_size(theme::FS_SMALL)
        .whitespace_nowrap()
        .child(c.text.clone());
    match c.state {
        ChipState::Used => {
            base.border_color(p.accent_fill).bg(p.accent_fill).text_color(p.on_accent).font_weight(FontWeight::SEMIBOLD)
        }
        ChipState::Offered => base.border_color(p.border_strong).text_color(p.text_2),
        ChipState::Off => base.border_dashed().border_color(p.border).text_color(p.text_3).opacity(0.6).line_through(),
    }
}

fn chips(list: &[CdChip], cx: &App) -> Div {
    div().flex().flex_wrap().gap(theme::S1).children(list.iter().map(|c| chip(c, cx)))
}

fn span(s: BarSpan) -> Div {
    div().absolute().left(relative((s.from / 100.0) as f32)).w(relative((s.width / 100.0) as f32))
}

fn swatch(part: BitpoolPart, cx: &App) -> Div {
    let p = cx.palette();
    let s = div().size(px(10.)).rounded(px(2.)).flex_shrink_0();
    match part {
        BitpoolPart::Device => s.bg(p.chart_2.opacity(0.35)),
        BitpoolPart::Config => s.bg(p.chart_2),
        BitpoolPart::Abr => s.border_2().border_dashed().border_color(p.chart_1),
        BitpoolPart::Used => s.bg(p.chart_1),
    }
}

/// The bitpool in use within the configured and the device's ranges
fn bitpool_bar_view(b: &BitpoolView, cx: &App) -> Div {
    let p = cx.palette();
    let bar = &b.bar;
    // An image to screen readers, named by the legend, which says the same
    let track = div()
        .id("bitpool-track")
        .role(Role::Image)
        .aria_label(b.label())
        .relative()
        .h(px(10.))
        .rounded_full()
        .bg(p.surface_3)
        .when_some(bar.device, |d, r| d.child(span(r).top_0().bottom_0().rounded_full().bg(p.chart_2.opacity(0.35))))
        .when_some(bar.config, |d, r| d.child(span(r).top(px(3.)).bottom(px(3.)).rounded_full().bg(p.chart_2)))
        // SBC ABR: the range the bitpool moves in, drawn around the track
        .when_some(bar.abr, |d, r| {
            d.child(
                span(r).top(px(-4.)).bottom(px(-4.)).border_2().border_dashed().border_color(p.chart_1).rounded(px(4.)),
            )
        })
        .child(
            div()
                .absolute()
                .left(relative((bar.used / 100.0) as f32))
                .ml(px(-4.))
                .top(px(-5.))
                .bottom(px(-5.))
                .w(px(8.))
                .p(px(2.))
                .rounded(px(4.))
                .bg(p.surface)
                .child(div().size_full().rounded(px(2.)).bg(p.chart_1)),
        );
    div()
        .flex()
        .flex_col()
        .gap(theme::S1)
        .pt(px(6.))
        .child(track)
        .child(
            div()
                .flex()
                .justify_between()
                .text_size(theme::FS_SMALL)
                .text_color(p.text_3)
                .child("0")
                .child(bar.max.to_string()),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap_x(theme::S3)
                .gap_y(theme::S1)
                .text_size(theme::FS_SMALL)
                .text_color(p.text_2)
                .children(b.parts.iter().map(|(part, text)| {
                    div().flex().items_center().gap(theme::S1).child(swatch(*part, cx)).child(text.clone())
                })),
        )
}

fn rows(list: &[CdRow], cx: &App) -> Div {
    div().flex().flex_col().gap(theme::S2).children(list.iter().map(|r| match r {
        CdRow::Value { label, value, extra } => {
            let shown = div().flex().flex_col().gap(theme::S1).child(div().child(value.clone())).when_some(
                extra.as_ref(),
                |d, e| match e {
                    CdExtra::Bitpool(b) => d.child(bitpool_bar_view(b, cx)),
                    CdExtra::Gauge(pct) => d.child(charts::gauge(Some(*pct), None, cx)),
                },
            );
            row(label, shown, cx)
        }
        CdRow::Chips { label, chips: list } => row(label, chips(list, cx), cx),
    }))
}

fn section(s: &CdSection, cx: &App) -> Div {
    let p = cx.palette();
    div()
        .flex()
        .flex_col()
        .gap(theme::S3)
        .child(
            div()
                .pt(theme::S2)
                .border_t_1()
                .border_color(p.border)
                .text_size(theme::FS_SMALL)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.text_2)
                .child(s.title.clone()),
        )
        .child(rows(&s.rows, cx))
        .children(s.hints.iter().map(|h| ui::hint(h.clone(), cx)))
}

/// The card (`CodecDetailsCard`)
pub fn codec_details_card(info: &StreamInfo, details: &CodecDetails, cx: &App) -> Div {
    let v = codec_details_view(info, details);
    ui::card(cx)
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(theme::S2)
                .text_size(theme::FS_LARGE)
                .child(div().font_weight(FontWeight::SEMIBOLD).child(v.title.clone()))
                .child(ui::muted(v.codec.clone(), cx)),
        )
        .child(rows(&v.rows, cx))
        .children(v.sections.iter().map(|s| section(s, cx)))
        .children(v.hints.iter().map(|h| ui::hint(h.clone(), cx)))
}
