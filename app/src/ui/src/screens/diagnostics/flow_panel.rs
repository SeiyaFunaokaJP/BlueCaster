//! Connection flow tab: the eleven steps of the current attempt, their time
//! of day, elapsed time and detail, and a jump to the line that set each.

use std::rc::Rc;

use gpui_kit::component::Disableable as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Context, Div, FontWeight, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use ipc::{FlowStepView, FlowView};
use ui_model::flow::{STEPS, Step, step_detail};
use ui_model::format::mask_identifiers;
use ui_model::i18n::t;
use ui_model::t;
use ui_model::view::diagnostics::elapsed_text;

use super::shared::{mask, time_text};
use crate::app::{self, NoticeKind};
use crate::core::{self, CoreEvent, WantKind};
use crate::host;
use crate::theme::{self, ActivePalette as _, Palette};
use crate::ui::{self, ButtonKind, IconName, icon};

/// Jumps to the log line of a step (LogLine.seq)
pub type OnJump = Rc<dyn Fn(u64, &mut Window, &mut App)>;

fn status_icon(status: &str) -> Option<IconName> {
    match status {
        "running" => Some(IconName::ArrowSync),
        "done" => Some(IconName::CheckmarkCircle),
        "warning" => Some(IconName::Warning),
        "failed" => Some(IconName::ErrorCircle),
        _ => None,
    }
}

fn status_color(status: &str, p: &Palette) -> Hsla {
    match status {
        "done" => p.good_text,
        "running" | "warning" => p.warn_text,
        "failed" => p.bad_text,
        _ => p.text_2,
    }
}

fn status_text(status: &str) -> String {
    t(&format!("dbgc.status.{status}"))
}

/// The columns (v1's list columns): icon, step, status, time, elapsed, detail, jump
fn columns(cells: [Div; 7]) -> Div {
    let [i, step, status, time, elapsed, detail, jump] = cells;
    div()
        .flex()
        .items_center()
        .gap(theme::S3)
        .px(theme::S4)
        .py(theme::S2)
        .child(i.w(px(24.)).flex_none().flex().justify_center())
        .child(step.w(px(160.)).flex_none().min_w_0())
        .child(status.w(px(72.)).flex_none())
        .child(time.w(px(104.)).flex_none().whitespace_nowrap())
        .child(elapsed.w(px(80.)).flex_none())
        .child(detail.flex_1().min_w_0())
        .child(jump.w(px(32.)).flex_none())
}

pub struct FlowPanel {
    on_jump: OnJump,
}

impl FlowPanel {
    pub fn new(on_jump: OnJump, _window: &mut Window, cx: &mut Context<Self>) -> FlowPanel {
        core::want(WantKind::Flow, cx);
        let core = app::core(cx);
        cx.subscribe(&core, |_, _, ev: &CoreEvent, cx| {
            if *ev == CoreEvent::Flow {
                cx.notify();
            }
        })
        .detach();
        cx.observe(&core, |_, _, cx| cx.notify()).detach();
        cx.observe_global::<super::shared::DiagShared>(|_, cx| cx.notify()).detach();
        FlowPanel { on_jump }
    }
}

/// The core's flow; no session yet: none
fn session(cx: &App) -> Option<FlowView> {
    app::core(cx).read(cx).flow.clone().filter(|f| f.session > 0)
}

fn step_of(s: Option<&FlowView>, st: Step) -> Option<&FlowStepView> {
    s.and_then(|s| s.steps.iter().find(|x| x.step == st.as_str()))
}

/// Time of day of the line that set the step
fn step_time(x: Option<&FlowStepView>, s: Option<&FlowView>, cx: &App) -> String {
    let Some(x) = x else { return String::new() };
    let ms = x.time_ms.or_else(|| x.elapsed_ms.zip(s).map(|(e, s)| s.start_ms + e));
    ms.map_or(String::new(), |ms| time_text(ms, cx))
}

fn detail(x: Option<&FlowStepView>, cx: &App) -> String {
    let d = x.map_or(String::new(), |x| step_detail(&x.detail, x.detail_msg.as_ref()));
    if mask(cx) { mask_identifiers(&d) } else { d }
}

fn header(s: Option<&FlowView>, cx: &App) -> String {
    let Some(s) = s else { return t("dbgc.no_session") };
    t!("dbgc.session", s.session, time_text(s.start_ms, cx))
        + &s.stop_ms.map_or(String::new(), |stop| t!("dbgc.session_stopped", time_text(stop, cx)))
}

fn copy_flow(cx: &mut App) {
    let s = session(cx);
    if s.is_none() {
        return;
    }
    let lines: Vec<String> = STEPS
        .iter()
        .map(|&st| {
            let x = step_of(s.as_ref(), st);
            let status = status_text(x.map_or("pending", |x| x.status.as_str()));
            [
                t(&format!("dbgc.step.{}", st.as_str())),
                status,
                step_time(x, s.as_ref(), cx),
                elapsed_text(x.and_then(|x| x.elapsed_ms)),
                detail(x, cx),
            ]
            .join("\t")
        })
        .collect();
    let n = lines.len() + 1;
    let text = std::iter::once(header(s.as_ref(), cx)).chain(lines).collect::<Vec<_>>().join("\n");
    host::copy_text(if mask(cx) { mask_identifiers(&text) } else { text }, cx);
    app::notify(t!("dbgc.copied", n), NoticeKind::Info, cx);
}

impl Render for FlowPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let s = session(cx);
        let head = columns([
            div(),
            div().child(t("dbgc.col_step")),
            div().child(t("dbgc.col_status")),
            div().child(t("dbgc.col_time")),
            div().child(t("dbgc.col_elapsed")),
            div().child(t("dbgc.col_detail")),
            div(),
        ])
        .text_size(theme::FS_SMALL)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.text_2);

        let steps = STEPS.iter().enumerate().map(|(n, &st)| {
            let x = step_of(s.as_ref(), st);
            let status = x.map_or("pending", |x| x.status.as_str()).to_owned();
            let seq = x.and_then(|x| x.seq);
            let color = status_color(&status, &p);
            // One line: a detail's line breaks and tabs are spaces here
            let d = detail(x, cx).replace(['\r', '\n', '\t'], " ");
            let small = |d: Div| d.text_size(theme::FS_SMALL).text_color(p.text_2);
            let icon_cell = match status_icon(&status) {
                Some(i) => div().text_color(color).child(icon(i)),
                None => div().child(div().size(px(8.)).rounded_full().border_1().border_color(p.text_3)),
            };
            let detail_cell = div()
                .id(("flow-detail", n))
                .truncate()
                .text_size(theme::FS_SMALL)
                .text_color(p.text_2)
                .child(d.clone())
                .when(!d.is_empty(), |el| el.tooltip(move |w, cx| Tooltip::new(d.clone()).build(w, cx)));
            let on_jump = self.on_jump.clone();
            let jump_cell = div().when_some(seq, |el, seq| {
                el.child(
                    ui::icon_button(("flow-jump", n), IconName::ArrowRight, t("ui.diag.show_line"))
                        .on_click(move |_, window, cx| on_jump(seq, window, cx)),
                )
            });
            let on_jump = self.on_jump.clone();
            columns([
                icon_cell,
                div().child(t(&format!("dbgc.step.{}", st.as_str()))),
                small(div()).text_color(color).child(status_text(&status)),
                small(div()).child(step_time(x, s.as_ref(), cx)),
                small(div()).child(elapsed_text(x.and_then(|x| x.elapsed_ms))),
                div().child(detail_cell),
                jump_cell,
            ])
            .id(("flow-step", n))
            .min_h(px(40.))
            .border_t_1()
            .border_color(p.border)
            .on_click(move |e, window, cx| {
                if e.click_count() >= 2
                    && let Some(seq) = seq
                {
                    on_jump(seq, window, cx);
                }
            })
        });

        div()
            .flex()
            .flex_col()
            .gap(theme::S4)
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .gap(theme::S3)
                    .child(ui::muted(header(s.as_ref(), cx), cx))
                    .child(
                        ui::button("copy-flow", t("dbgc.copy_flow"), ButtonKind::Subtle)
                            .icon(icon(IconName::Copy))
                            .disabled(s.is_none())
                            .on_click(|_, _, cx| copy_flow(cx)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .bg(p.surface)
                    .border_1()
                    .border_color(p.border)
                    .rounded(theme::RADIUS_CARD)
                    .child(head)
                    .children(steps),
            )
            .child(ui::hint(t("dbgc.flow_hint"), cx))
    }
}
