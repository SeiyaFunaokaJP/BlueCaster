//! Small charts for the link samples. No chart library: polylines over the
//! last minute of samples are all these need, drawn on a canvas that
//! stretches a drawing of a fixed size to its box, lines staying their
//! width in pixels.
//! The Diagnostics charts of one tab share a time axis and a hover cursor
//! ([`ChartGroup`]).
//!
//! [`plot`] is the drawing primitive the other graphs can use too (the
//! playback EQ's frequency response): lines, filled shapes and rectangles in
//! the drawing's units.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Bounds, Canvas, Context, Div, ElementId, Entity, FontWeight, Hsla, InteractiveElement as _,
    IntoElement, LineCap, LineJoin, MouseDownEvent, MouseMoveEvent, ParentElement as _, PathBuilder, PathStyle, Pixels,
    RenderOnce, SharedString, StatefulInteractiveElement as _, StrokeOptions, Styled as _, Window, canvas, div, point,
    px, relative,
};
use ui_model::series::{last_known_index, nearest_index, scale_of};
use ui_model::view::charts::{self as geo, Pt, Status};

use crate::theme::{self, ActivePalette as _, Palette};

// ---- The drawing primitive ---------------------------------------------------

/// What [`plot`] draws, in the drawing's units
#[derive(Debug, Clone)]
pub enum Mark {
    /// A polyline; `width` in pixels whatever the scale
    Line { points: Vec<Pt>, color: Hsla, width: f32, dashed: bool },
    /// A closed shape, filled, with an optional outline
    Shape { points: Vec<Pt>, fill: Hsla, stroke: Option<(Hsla, f32)> },
    /// A rectangle, filled, with an optional outline
    Rect { x: f64, y: f64, w: f64, h: f64, fill: Hsla, stroke: Option<(Hsla, f32)> },
}

/// The visible part of a drawing: x, y, width, height (an SVG viewBox)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewBox {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl ViewBox {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> ViewBox {
        ViewBox { x, y, w, h }
    }

    /// A point of the drawing in the element's pixels
    fn map(&self, b: &Bounds<Pixels>, (x, y): Pt) -> gpui_kit::Point<Pixels> {
        let sx = f64::from(b.size.width) / self.w.max(f64::MIN_POSITIVE);
        let sy = f64::from(b.size.height) / self.h.max(f64::MIN_POSITIVE);
        point(b.origin.x + px(((x - self.x) * sx) as f32), b.origin.y + px(((y - self.y) * sy) as f32))
    }
}

fn stroke_options(width: f32) -> PathStyle {
    PathStyle::Stroke(StrokeOptions {
        line_width: width,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Default::default()
    })
}

fn paint_outline(
    points: &[gpui_kit::Point<Pixels>],
    color: Hsla,
    width: f32,
    closed: bool,
    dashed: bool,
    window: &mut Window,
) {
    if points.len() < 2 {
        return;
    }
    let mut b = PathBuilder::stroke(px(width)).with_style(stroke_options(width));
    if dashed {
        b = b.dash_array(&[px(4.), px(3.)]);
    }
    b.add_polygon(points, closed);
    if let Ok(path) = b.build() {
        window.paint_path(path, color);
    }
}

fn paint_fill(points: &[gpui_kit::Point<Pixels>], color: Hsla, window: &mut Window) {
    if points.len() < 3 {
        return;
    }
    let mut b = PathBuilder::fill();
    b.add_polygon(points, true);
    if let Ok(path) = b.build() {
        window.paint_path(path, color);
    }
}

/// A canvas that draws `marks` of the drawing `view` stretched to its box.
/// `bounds` (when given) learns the box, for pointer positions.
pub fn plot(view: ViewBox, marks: Vec<Mark>, bounds: Option<Rc<Cell<Bounds<Pixels>>>>) -> Canvas<()> {
    canvas(
        move |b, _, _| {
            if let Some(cell) = &bounds {
                cell.set(b);
            }
        },
        move |b, _, window, _| {
            for m in &marks {
                match m {
                    Mark::Line { points, color, width, dashed } => {
                        let pts: Vec<_> = points.iter().map(|p| view.map(&b, *p)).collect();
                        paint_outline(&pts, *color, *width, false, *dashed, window);
                    }
                    Mark::Shape { points, fill, stroke } => {
                        let pts: Vec<_> = points.iter().map(|p| view.map(&b, *p)).collect();
                        paint_fill(&pts, *fill, window);
                        if let Some((color, width)) = stroke {
                            paint_outline(&pts, *color, *width, true, false, window);
                        }
                    }
                    Mark::Rect { x, y, w, h, fill, stroke } => {
                        let pts: Vec<_> = [(*x, *y), (x + w, *y), (x + w, y + h), (*x, y + h)]
                            .iter()
                            .map(|p| view.map(&b, *p))
                            .collect();
                        paint_fill(&pts, *fill, window);
                        if let Some((color, width)) = stroke {
                            paint_outline(&pts, *color, *width, true, false, window);
                        }
                    }
                }
            }
        },
    )
}

/// A status as text colour (`st-*` on text)
pub fn status_text(s: Status, p: &Palette) -> Hsla {
    match s {
        Status::Good => p.good_text,
        Status::Warn => p.warn_text,
        Status::Bad => p.bad_text,
    }
}

/// A status as a fill (`--st`)
pub fn status_fill(s: Status, p: &Palette) -> Hsla {
    match s {
        Status::Good => p.good,
        Status::Warn => p.warn,
        Status::Bad => p.bad,
    }
}

// ---- Sparkline and Gauge ------------------------------------------------------

/// A tiny trend line (Home, flyout); give it a size (`.flex_1().h(..)`)
pub fn sparkline(values: &[Option<f64>], min: Option<f64>, max: Option<f64>, cx: &App) -> Canvas<()> {
    let (lo, hi) = geo::bounds(values, min, max);
    let color = cx.palette().text_2;
    // The drawing is 100 x 30, the line 28 tall one unit down
    let marks = geo::points(values, lo, hi, 100.0, 28.0)
        .into_iter()
        .map(|run| Mark::Line {
            points: run.into_iter().map(|(x, y)| (x, y + 1.0)).collect(),
            color,
            width: 1.5,
            dashed: false,
        })
        .collect();
    plot(ViewBox::new(0.0, 0.0, 100.0, 30.0), marks, None)
}

/// A horizontal bar for a share of 0..100 % (Diagnostics' bandwidth rows)
pub fn gauge(value: Option<f64>, status: Option<Status>, cx: &App) -> Div {
    let p = cx.palette();
    let v = value.filter(|v| v.is_finite()).map_or(0.0, |v| v.clamp(0.0, 100.0));
    div().w_full().h(px(6.)).rounded_full().bg(p.surface_3).overflow_hidden().child(
        div()
            .h_full()
            .w(relative((v / 100.0) as f32))
            .rounded_full()
            .bg(status.map_or(p.accent_fill, |s| status_fill(s, p))),
    )
}

// ---- Time charts (Diagnostics) ----------------------------------------------

/// The hovered core time of a chart group (None: not hovered)
#[derive(Debug, Default)]
pub struct ChartHover(pub Option<f64>);

/// Charts with one time axis ending at `end` (the latest sample) and one
/// hover cursor: the view that draws them owns the hover entity and
/// observes it
#[derive(Clone)]
pub struct ChartGroup {
    /// The axis: [start, end] in core time (ms)
    pub range: (f64, f64),
    pub hover: Entity<ChartHover>,
}

impl ChartGroup {
    pub fn new(end: Option<f64>, span: f64, hover: Entity<ChartHover>) -> ChartGroup {
        ChartGroup { range: geo::group_range(end, span), hover }
    }

    /// A hover entity for a view's charts; the view draws again as it moves
    pub fn hover_for<V: 'static>(cx: &mut Context<V>) -> Entity<ChartHover> {
        let hover = cx.new(|_| ChartHover::default());
        cx.observe(&hover, |_, _, cx| cx.notify()).detach();
        hover
    }
}

#[derive(Debug, Clone)]
pub struct ChartSeries {
    pub values: Vec<Option<f64>>,
    /// The legend's name (charts with more than one series)
    pub label: Option<SharedString>,
}

impl ChartSeries {
    pub fn new(values: Vec<Option<f64>>) -> ChartSeries {
        ChartSeries { values, label: None }
    }

    pub fn labelled(label: impl Into<SharedString>, values: Vec<Option<f64>>) -> ChartSeries {
        ChartSeries { values, label: Some(label.into()) }
    }
}

/// A horizontal line: a limit (warning colour) or a target the line should follow
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    Limit,
    Target,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChartRef {
    pub value: f64,
    pub kind: RefKind,
}

const W: f64 = 300.0;
const H: f64 = 96.0;
/// The scale's column: one width, so the time axes of a tab line up
const SCALE_W: Pixels = px(72.);

type Format = Rc<dyn Fn(f64) -> String>;
type FormatValue = Rc<dyn Fn(f64, usize) -> String>;
type StatusOf = Rc<dyn Fn(f64) -> Status>;

/// A labelled live chart over time: the latest value (or the hovered one)
/// in the header
#[derive(IntoElement)]
pub struct TimeChart {
    id: ElementId,
    title: SharedString,
    /// LinkSample.time_ms of each value
    times: Rc<[f64]>,
    series: Vec<ChartSeries>,
    min: Option<f64>,
    max: Option<f64>,
    /// The scale's labels
    format: Format,
    /// The value in the header and legend, when it says more than the scale
    /// (`i`: the sample shown, hovered or the latest known)
    format_value: Option<FormatValue>,
    /// The status colour of the first series' value
    status: Option<StatusOf>,
    refs: Vec<ChartRef>,
    group: Option<ChartGroup>,
}

impl TimeChart {
    pub fn new(
        id: impl Into<ElementId>,
        title: impl Into<SharedString>,
        times: Rc<[f64]>,
        series: Vec<ChartSeries>,
        format: impl Fn(f64) -> String + 'static,
    ) -> TimeChart {
        TimeChart {
            id: id.into(),
            title: title.into(),
            times,
            series,
            min: None,
            max: None,
            format: Rc::new(format),
            format_value: None,
            status: None,
            refs: Vec::new(),
            group: None,
        }
    }

    pub fn min(mut self, v: f64) -> Self {
        self.min = Some(v);
        self
    }

    pub fn max(mut self, v: Option<f64>) -> Self {
        self.max = v;
        self
    }

    pub fn format_value(mut self, f: impl Fn(f64, usize) -> String + 'static) -> Self {
        self.format_value = Some(Rc::new(f));
        self
    }

    pub fn status(mut self, f: impl Fn(f64) -> Status + 'static) -> Self {
        self.status = Some(Rc::new(f));
        self
    }

    pub fn refs(mut self, refs: Vec<ChartRef>) -> Self {
        self.refs = refs;
        self
    }

    pub fn group(mut self, group: &ChartGroup) -> Self {
        self.group = Some(group.clone());
        self
    }

    fn range(&self) -> (f64, f64) {
        match &self.group {
            Some(g) => g.range,
            None => geo::own_range(&self.times),
        }
    }

    /// The sample of a series shown: the hovered one, else its latest known
    fn shown(&self, s: &ChartSeries, at: Option<usize>) -> Option<usize> {
        match at {
            None => last_known_index(&s.values),
            Some(i) => s.values.get(i).copied().flatten().filter(|v| v.is_finite()).map(|_| i),
        }
    }

    fn text(&self, s: Option<&ChartSeries>, at: Option<usize>) -> String {
        let Some(s) = s else { return "\u{2013}".into() };
        let Some(i) = self.shown(s, at) else { return "\u{2013}".into() };
        let v = s.values[i].unwrap_or_default();
        match &self.format_value {
            Some(f) => f(v, i),
            None => (self.format)(v),
        }
    }
}

impl RenderOnce for TimeChart {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = *cx.palette();
        let (start, end) = self.range();
        let span = if end - start != 0.0 { end - start } else { 1.0 };
        let mut all: Vec<&[Option<f64>]> = self.series.iter().map(|s| s.values.as_slice()).collect();
        let ref_values: Vec<Option<f64>> = self.refs.iter().map(|r| Some(r.value)).collect();
        all.push(&ref_values);
        let (lo, hi) = scale_of(&all, self.min, self.max);
        let multi = self.series.len() > 1;
        let hover = self.group.as_ref().map(|g| g.hover.clone());
        let hovered = hover.as_ref().and_then(|h| h.read(cx).0);
        // The sample under the cursor
        let at = hovered.and_then(|h| nearest_index(&self.times, h));

        // The drawing: 300 x 100, the plot 96 tall two units down
        let mut marks = Vec::new();
        let grid = |y: f64| Mark::Line {
            points: vec![(0.0, y + 2.0), (W, y + 2.0)],
            color: p.border,
            width: 1.0,
            dashed: false,
        };
        marks.extend([grid(0.0), grid(H / 2.0), grid(H)]);
        for r in &self.refs {
            let y = geo::y_of(r.value, lo, hi, H) + 2.0;
            let color = if r.kind == RefKind::Limit { p.warn_text } else { p.text_3 };
            marks.push(Mark::Line { points: vec![(0.0, y), (W, y)], color, width: 1.0, dashed: true });
        }
        for (i, s) in self.series.iter().enumerate() {
            let color = match (multi, i) {
                (false, _) => p.text,
                (true, 0) => p.chart_1,
                (true, _) => p.chart_2,
            };
            for run in geo::time_points(&self.times, &s.values, start, span, lo, hi, W, H) {
                let points = run.into_iter().map(|(x, y)| (x, y + 2.0)).collect();
                marks.push(Mark::Line { points, color, width: 1.5, dashed: false });
            }
        }
        if let Some(i) = at {
            let x = ((self.times[i] - start) / span) * W;
            marks.push(Mark::Line { points: vec![(x, 2.0), (x, H + 2.0)], color: p.text_2, width: 1.0, dashed: false });
        }

        let first = self.series.first().and_then(|s| self.shown(s, at).and_then(|i| s.values[i]));
        let value_color = match (first, &self.status) {
            (Some(v), Some(st)) => status_text(st(v), &p),
            _ => p.text,
        };
        let bounds: Rc<Cell<Bounds<Pixels>>> = Rc::default();
        let on_pointer = {
            let (bounds, hover) = (bounds.clone(), hover.clone());
            move |x: Pixels, cx: &mut App| {
                let Some(hover) = &hover else { return };
                let b = bounds.get();
                if b.size.width <= px(0.) {
                    return;
                }
                let f = (f64::from(x - b.origin.x) / f64::from(b.size.width)).clamp(0.0, 1.0);
                hover.update(cx, |h, cx| {
                    h.0 = Some(start + f * span);
                    cx.notify();
                });
            }
        };
        let on_move = on_pointer.clone();
        let leave = hover.clone();

        let title = div()
            .text_size(theme::FS_SMALL)
            .text_color(p.text_2)
            .font_weight(FontWeight::SEMIBOLD)
            .flex_none()
            .child(self.title.clone());
        let head = div().flex().justify_between().items_baseline().gap(theme::S2).child(title).when(!multi, |d| {
            d.child(
                div()
                    // A long value wraps under itself instead of leaving the card
                    .min_w_0()
                    .text_right()
                    .text_size(theme::FS_LARGE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(value_color)
                    .child(self.text(self.series.first(), at)),
            )
        });
        let legend = multi.then(|| {
            div()
                .flex()
                .flex_wrap()
                .gap_x(theme::S4)
                .gap_y(theme::S1)
                .text_size(theme::FS_SMALL)
                .text_color(p.text_2)
                .children(self.series.iter().enumerate().map(|(i, s)| {
                    div()
                        .flex()
                        .items_center()
                        .gap(theme::S2)
                        .child(div().w(px(12.)).h(px(3.)).rounded_full().bg(if i == 0 { p.chart_1 } else { p.chart_2 }))
                        .child(s.label.clone().unwrap_or_default())
                        .child(
                            div()
                                .text_size(theme::FS_BODY)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(p.text)
                                .child(self.text(Some(s), at)),
                        )
                }))
        });
        let plot_area = div()
            .id(self.id.clone())
            .flex_1()
            .min_w_0()
            .h(px(H as f32))
            .child(plot(ViewBox::new(0.0, 0.0, W, H + 4.0), marks, Some(bounds)).size_full())
            .on_mouse_move(move |e: &MouseMoveEvent, _, cx| on_move(e.position.x, cx))
            .on_mouse_down(gpui_kit::MouseButton::Left, move |e: &MouseDownEvent, _, cx| on_pointer(e.position.x, cx))
            .on_hover(move |hovered, _, cx| {
                if !*hovered && let Some(h) = &leave {
                    h.update(cx, |h, cx| {
                        h.0 = None;
                        cx.notify();
                    });
                }
            });
        let scale = div()
            .w(SCALE_W)
            .flex_none()
            .flex()
            .flex_col()
            .justify_between()
            .items_end()
            .text_size(theme::FS_SMALL)
            .text_color(p.text_3)
            .child((self.format)(hi))
            .child((self.format)(lo));
        let axis = div()
            .flex()
            .justify_between()
            .mr(SCALE_W + theme::S2)
            .text_size(theme::FS_SMALL)
            .text_color(p.text_3)
            .child(geo::axis_start_text(span))
            .child("0 s");
        div()
            .flex_1()
            .flex()
            .flex_col()
            .gap(theme::S2)
            .px(theme::S4)
            .py(theme::S3)
            .bg(p.surface)
            .border_1()
            .border_color(p.border)
            .rounded(theme::RADIUS_CARD)
            .child(head)
            .children(legend)
            .child(div().flex().gap(theme::S2).child(plot_area).child(scale))
            .child(axis)
    }
}

/// The charts of a tab side by side, wrapping (`.chart-grid`: at least 16rem each)
pub fn chart_grid(charts: Vec<TimeChart>) -> Div {
    div().flex().flex_wrap().gap(theme::S4).children(
        charts.into_iter().map(|c| {
            div().flex_grow(1.).flex_shrink(1.).flex_basis(px(256.)).min_w(px(256.)).flex().flex_col().child(c)
        }),
    )
}
