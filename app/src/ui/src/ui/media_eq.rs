//! A device's playback EQ, in a dialog from Audio Devices: on or off, and up to 16 points on
//! a frequency-response graph (as Easy-MIC-Booster's): double-click adds a
//! point (or puts a point's gain back to 0), dragging moves it (left/right
//! the frequency on a log scale, up/down the gain), the wheel over it changes
//! its Q, right-click changes its type or deletes it. The selected point's
//! values are fields below the graph. A focused point moves with the arrow
//! keys (Shift: bigger steps), Page Up / Page Down change its Q, Delete
//! removes it. A new EQ starts with the classic ten flat bands
//! (ui_model::eq DEFAULT_POINTS), shown greyed out until it is turned on; Reset
//! puts them back. Saved points are never replaced. Nothing hides (issue #86): while the EQ is off the graph,
//! the fields and the buttons stay, disabled, and the fields are empty while
//! no point is selected. Export saves the points as an Equalizer APO /
//! AutoEq parametric EQ (ui_model::eq eq_apo_text).
//!
//! While a point is dragged the stream plays it at once (media_eq.preview);
//! the profile is saved when it is let go. The curve is the core's own
//! filters computed here (ui_model::eq); the core lowers the level by the
//! highest boost and limits what still goes over, so the curve's shape is
//! what changes, not the loudness.
//!
//! Use: [`media_eq_settings`] (the line on a device, the editor in a dialog);
//! [`MediaEqEditor`] and [`EqGraph`] are the views inside it. The rules are
//! ui_model::view::media_eq.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::component::Disableable as _;
use gpui_kit::component::input::{InputEvent, InputState, NumberInput, NumberInputEvent, StepAction};
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AccessibleAction, Anchor, App, AppContext as _, AsyncApp, Bounds, ClickEvent, Context, CursorStyle, DismissEvent,
    Div, Entity, EventEmitter, FocusHandle, Focusable as _, FontWeight, Hsla, InteractiveElement as _, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _, Pixels, Point, Render,
    Role, ScrollDelta, ScrollEvent, StatefulInteractiveElement as _, Styled as _, Subscription, Task, WeakEntity,
    Window, anchored, canvas, deferred, div, point, px, relative,
};
use ipc::{Device, EqBandType, EqPoint, MediaEq, Method, StreamState};
use ui_model::eq::{
    BAND_TYPES, DEFAULT_POINTS, GRAPH_RATE, MAX_POINTS, eq_apo_text, has_q, is_default_points, response_curve,
    with_type,
};
use ui_model::i18n::t;
use ui_model::profile::{AudioEntry, audio_entries, device_key};
use ui_model::t;
use ui_model::view::media_eq::{
    self as rules, EXPORT_NAME, EqKey, FREQ_FIELD, GAIN_FIELD, GRID_DB, GRID_F, H, KeyAction, PLOT_BOTTOM, PLOT_LEFT,
    PLOT_RIGHT, PLOT_TOP, PREVIEW_MS, Q_FIELD, W, add_at, db_axis_label, eq_controls, field_text, freq_axis_label,
    point_label, point_y, shown_bands, tip_text, toggled, type_label, typed_number, without, x_of, y_of,
};

use crate::app;
use crate::host;
use crate::theme::{self, ActivePalette as _};
use crate::ui::charts::{Mark, ViewBox, plot};
use crate::ui::{self, ButtonKind, Choice, IconName, dialog, icon, menu_choice, menu_command};

/// A line saying whether the playback EQ is on, with the editor in a dialog
/// (`MediaEqSettings`)
pub fn media_eq_settings<'a>(device: &Device, cx: &'a App) -> impl IntoElement + use<'a> {
    let key = device_key(&device.address, device.le_audio);
    let on = entry_of(&key, cx).and_then(|e| e.profile).is_some_and(|p| p.media_eq.enabled);
    ui::nav_row(
        ("media-eq", ui_model_key_hash(&key)),
        t("eq.title"),
        t(if on { "ui.state_on" } else { "ui.state_off" }),
        false,
        move |_, window, cx| {
            let key = key.clone();
            let editor = cx.new(|cx| MediaEqEditor::new(key, window, cx));
            dialog::ask(
                dialog::Question::new(t("eq.title")),
                move |_, _| editor.clone().into_any_element(),
                |_, _| {},
                window,
                cx,
            );
        },
        cx,
    )
}

/// A stable number for an element id from a device key
fn ui_model_key_hash(key: &str) -> usize {
    key.bytes().fold(0usize, |h, b| h.wrapping_mul(31).wrapping_add(usize::from(b)))
}

fn entry_of(key: &str, cx: &App) -> Option<AudioEntry> {
    let core = app::core(cx);
    let snap = core.read(cx).snap()?;
    audio_entries(snap).into_iter().find(|e| e.key == key)
}

// ---- saving (screens/codecs.rs save_media_eq) ------------------------------------

pub use crate::screens::codecs::save_media_eq;

// ---- the editor ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldKind {
    Frequency,
    Gain,
    Q,
}

impl FieldKind {
    fn range(self) -> (f64, f64, f64) {
        match self {
            FieldKind::Frequency => FREQ_FIELD,
            FieldKind::Gain => GAIN_FIELD,
            FieldKind::Q => Q_FIELD,
        }
    }

    fn of(self, b: &EqPoint) -> f32 {
        match self {
            FieldKind::Frequency => b.frequency,
            FieldKind::Gain => b.gain,
            FieldKind::Q => b.q,
        }
    }

    fn with(self, b: &EqPoint, v: f64) -> EqPoint {
        let v = v as f32;
        match self {
            FieldKind::Frequency => EqPoint { frequency: v, ..*b },
            FieldKind::Gain => EqPoint { gain: v, ..*b },
            FieldKind::Q => EqPoint { q: v, ..*b },
        }
    }
}

/// A number typed in, taken when it is left or Enter is pressed; empty
/// (and disabled) while there is no value
struct NumberField {
    kind: FieldKind,
    input: Entity<InputState>,
    /// The text last put in the box (None: put it again)
    synced: Option<String>,
}

/// The editor in the dialog (`MediaEqEditor`): the device's EQ by its key
pub struct MediaEqEditor {
    key: String,
    /// What is shown while a change is on its way (dragged, or saved and not
    /// back in the profile yet)
    draft: Option<MediaEq>,
    selected: Option<usize>,
    last_preview: Option<Instant>,
    pending_preview: Option<Task<()>>,
    export_error: Option<String>,
    graph: Entity<EqGraph>,
    fields: Vec<NumberField>,
    _subscriptions: Vec<Subscription>,
}

impl MediaEqEditor {
    /// The editor of the device entry `key` (ui_model::profile::device_key)
    pub fn new(key: String, window: &mut Window, cx: &mut Context<Self>) -> MediaEqEditor {
        let core = app::core(cx);
        let mut subs = vec![cx.observe(&core, |_, _, cx| cx.notify())];
        let graph = cx.new(|cx| EqGraph::new(window, cx));
        subs.push(cx.subscribe_in(&graph, window, |this: &mut Self, _, ev: &EqGraphEvent, _, cx| match ev {
            EqGraphEvent::Select(i) => {
                this.selected = *i;
                cx.notify();
            }
            EqGraphEvent::Change(bands, done) => this.set_bands(bands.clone(), *done, cx),
        }));
        let mut fields = Vec::new();
        for kind in [FieldKind::Frequency, FieldKind::Gain, FieldKind::Q] {
            // No step of the box's own: the arrow keys and buttons come here
            // as Step, step by the field's own amount and save at once
            let input = cx.new(|cx| {
                let mut s = InputState::new(window, cx);
                s.set_step(None, window, cx);
                s
            });
            subs.push(cx.subscribe_in(&input, window, move |this: &mut Self, _, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    this.field_done(kind, cx);
                }
            }));
            subs.push(cx.subscribe_in(&input, window, move |this: &mut Self, _, ev: &NumberInputEvent, _, cx| {
                let NumberInputEvent::Step(action) = ev;
                this.field_step(kind, *action, cx);
            }));
            fields.push(NumberField { kind, input, synced: None });
        }
        MediaEqEditor {
            key,
            draft: None,
            selected: None,
            last_preview: None,
            pending_preview: None,
            export_error: None,
            graph,
            fields,
            _subscriptions: subs,
        }
    }

    fn entry(&self, cx: &App) -> Option<AudioEntry> {
        entry_of(&self.key, cx)
    }

    fn saved(&self, cx: &App) -> MediaEq {
        self.entry(cx).and_then(|e| e.profile).map(|p| p.media_eq).unwrap_or_default()
    }

    fn eq(&self, cx: &App) -> MediaEq {
        self.draft.clone().unwrap_or_else(|| self.saved(cx))
    }

    // ---- live preview while dragging, saved when let go ------------------------

    fn send_preview(&mut self, cx: &mut Context<Self>) {
        self.pending_preview = None;
        // The executor's clock: the wall clock, and the test clock in tests
        self.last_preview = Some(cx.background_executor().now());
        let (Some(p), Some(d)) = (self.entry(cx).and_then(|e| e.profile), self.draft.clone()) else { return };
        let core = app::core(cx);
        let Some(snap) = core.read(cx).snap() else { return };
        if snap.stream.profile.as_deref() != Some(p.name.as_str()) || snap.stream.state == StreamState::Idle {
            return;
        }
        app::request::<serde_json::Value>(Method::MediaEqPreview { name: p.name, eq: d }, cx).detach();
    }

    fn preview(&mut self, next: MediaEq, cx: &mut Context<Self>) {
        self.draft = Some(next);
        cx.notify();
        if self.pending_preview.is_some() {
            return;
        }
        let now = cx.background_executor().now();
        let since = self.last_preview.map_or(u64::MAX, |t| now.saturating_duration_since(t).as_millis() as u64);
        let wait = PREVIEW_MS.saturating_sub(since);
        self.pending_preview = Some(cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor().timer(Duration::from_millis(wait)).await;
            let _ = this.update(cx, |this, cx| this.send_preview(cx));
        }));
    }

    fn commit(&mut self, next: MediaEq, cx: &mut Context<Self>) {
        self.pending_preview = None;
        self.draft = Some(next.clone());
        cx.notify();
        let Some(e) = self.entry(cx) else {
            self.draft = None;
            return;
        };
        let task = save_media_eq(&e, next, cx);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            if !task.await {
                let _ = this.update(cx, |this, cx| {
                    this.draft = None;
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn set_bands(&mut self, bands: Vec<EqPoint>, done: bool, cx: &mut Context<Self>) {
        let next = MediaEq { bands, ..self.eq(cx) };
        if done { self.commit(next, cx) } else { self.preview(next, cx) }
    }

    fn sel(&self, cx: &App) -> Option<EqPoint> {
        self.selected.and_then(|i| self.eq(cx).bands.get(i).copied())
    }

    fn edit_selected(&mut self, f: impl FnOnce(&EqPoint) -> EqPoint, cx: &mut Context<Self>) {
        let Some(i) = self.selected else { return };
        let bands = self.eq(cx).bands;
        let Some(b) = bands.get(i) else { return };
        let next = rules::replace(&bands, i, &f(b));
        self.set_bands(next, true, cx);
    }

    fn add_point(&mut self, cx: &mut Context<Self>) {
        let Some((bands, i)) = rules::add_point(&self.eq(cx).bands) else { return };
        self.set_bands(bands, true, cx);
        self.selected = Some(i);
    }

    fn delete_selected(&mut self, cx: &mut Context<Self>) {
        let Some(i) = self.selected else { return };
        self.selected = None;
        let bands = without(&self.eq(cx).bands, i);
        self.set_bands(bands, true, cx);
    }

    fn reset(&mut self, cx: &mut Context<Self>) {
        self.selected = None;
        self.set_bands(DEFAULT_POINTS.to_vec(), true, cx);
    }

    fn export_points(&mut self, cx: &mut Context<Self>) {
        self.export_error = None;
        let task = host::save_text_file(EXPORT_NAME, eq_apo_text(&self.eq(cx).bands), cx);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            if let Err(e) = task.await {
                let _ = this.update(cx, |this, cx| {
                    this.export_error = Some(t!("eq.export_failed", e));
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }

    fn field(&mut self, kind: FieldKind) -> &mut NumberField {
        let i = self.fields.iter().position(|f| f.kind == kind).unwrap_or(0);
        &mut self.fields[i]
    }

    /// The typed value taken (held to its range), or the box shows the value again
    fn field_done(&mut self, kind: FieldKind, cx: &mut Context<Self>) {
        let Some(b) = self.sel(cx) else { return };
        let (min, max, _) = kind.range();
        let text = self.field(kind).input.read(cx).value().to_string();
        self.field(kind).synced = None;
        match typed_number(&text, min, max) {
            Some(v) if v != f64::from(kind.of(&b)) => self.edit_selected(|b| kind.with(b, v), cx),
            _ => cx.notify(),
        }
    }

    /// The box's up/down buttons and keys: one step, held to the range
    fn field_step(&mut self, kind: FieldKind, action: StepAction, cx: &mut Context<Self>) {
        let Some(b) = self.sel(cx) else { return };
        let (min, max, step) = kind.range();
        let cur = f64::from(kind.of(&b));
        let v = (cur + if action == StepAction::Increment { step } else { -step }).clamp(min, max);
        self.field(kind).synced = None;
        self.edit_selected(|b| kind.with(b, v), cx);
    }

    fn number_field(&mut self, field: FieldSpec, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let FieldSpec { kind, label, unit, wide, value, enabled } = field;
        let text = field_text(value, enabled);
        let f = self.field(kind);
        if f.synced.as_deref() != Some(text.as_str()) {
            f.input.update(cx, |s, cx| s.set_value(text.clone(), window, cx));
            f.synced = Some(text);
        }
        let input = f.input.clone();
        let p = *cx.palette();
        eq_field(label.clone(), cx).child(
            div()
                .flex()
                .items_center()
                .gap(theme::S1)
                // As tall as the selects beside it, so the labels above line up
                .child(NumberInput::new(&input).disabled(!enabled).accessibility_label(label).w(px(if wide {
                    132.
                } else {
                    112.
                })))
                .when_some(unit, |d, u| d.child(div().text_color(p.text_3).child(u))),
        )
    }
}

/// What one of the selected point's number boxes shows
struct FieldSpec {
    kind: FieldKind,
    label: String,
    unit: Option<&'static str>,
    /// The frequency's box: room for five digits
    wide: bool,
    /// None: no point selected
    value: Option<f32>,
    enabled: bool,
}

/// A labelled field under the graph (`.eq-field`)
fn eq_field(label: String, cx: &App) -> Div {
    div().flex().flex_col().gap(theme::S1).text_size(theme::FS_SMALL).text_color(cx.palette().text_2).child(label)
}

impl Render for MediaEqEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let has_entry = self.entry(cx).is_some();
        let saved = self.saved(cx);
        if self.draft.as_ref() == Some(&saved) {
            self.draft = None;
        }
        let eq = self.eq(cx);
        self.selected = rules::valid_selection(&eq, self.selected);
        let c = eq_controls(&eq, has_entry, self.selected);
        // Editing: on, with a profile to save it in; off, everything stays but disabled
        if !c.active {
            self.selected = None;
        }
        let selected = self.selected;
        let sel = selected.and_then(|i| eq.bands.get(i).copied());
        self.graph.update(cx, |g, cx| g.set_props(shown_bands(&eq), selected, !c.active, cx));

        let p = *cx.palette();
        // While none is selected, a blank first entry: the box shows nothing
        let mut point_choices: Vec<Choice<i32>> = Vec::new();
        if selected.is_none() {
            point_choices.push(Choice::new(-1, "").disabled(true));
        }
        point_choices.extend((0..eq.bands.len()).map(|i| Choice::new(i as i32, (i + 1).to_string())));
        let type_choices: Vec<Choice<EqBandType>> = if sel.is_some() {
            BAND_TYPES.iter().map(|b| Choice::new(*b, type_label(*b))).collect()
        } else {
            Vec::new()
        };

        let freq = self.number_field(
            FieldSpec {
                kind: FieldKind::Frequency,
                label: t("eq.frequency"),
                unit: Some("Hz"),
                wide: true,
                value: sel.map(|b| b.frequency),
                enabled: c.frequency,
            },
            window,
            cx,
        );
        let gain = self.number_field(
            FieldSpec {
                kind: FieldKind::Gain,
                label: t("eq.gain"),
                unit: Some("dB"),
                wide: false,
                value: sel.map(|b| b.gain),
                enabled: c.gain,
            },
            window,
            cx,
        );
        let q = self.number_field(
            FieldSpec {
                kind: FieldKind::Q,
                label: "Q".to_owned(),
                unit: None,
                wide: false,
                value: sel.map(|b| b.q),
                enabled: c.q,
            },
            window,
            cx,
        );

        let fields = div()
            .flex()
            .flex_wrap()
            .items_end()
            .justify_between()
            .gap_x(theme::S4)
            .gap_y(theme::S3)
            .child(
                eq_field(t("eq.point"), cx).child(
                    div()
                        .flex()
                        .items_center()
                        .gap(theme::S1)
                        .child(
                            ui::icon_button("eq-delete-point", IconName::Delete, t("eq.delete_point"))
                                .disabled(!c.delete)
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.delete_selected(cx))),
                        )
                        .child(div().min_w(px(72.)).child(ui::select_labelled(
                            "eq-point",
                            t("eq.point"),
                            &selected.map_or(-1, |i| i as i32),
                            point_choices,
                            !c.point_pick,
                            cx.listener(|this, v: &i32, _, cx| {
                                this.selected = usize::try_from(*v).ok();
                                cx.notify();
                            }),
                        ))),
                ),
            )
            .child(eq_field(t("eq.type"), cx).child(div().min_w(px(112.)).child(ui::select_labelled(
                "eq-type",
                t("eq.type"),
                &sel.map_or(EqBandType::Peaking, |b| b.band_type),
                type_choices,
                !c.type_pick,
                cx.listener(|this, v: &EqBandType, _, cx| {
                    let v = *v;
                    this.edit_selected(|b| with_type(b, v), cx);
                }),
            ))))
            .child(freq)
            .child(gain)
            .child(q);

        // The buttons under a line, at the end as in every dialog
        let actions = div()
            .flex()
            .flex_wrap()
            .justify_end()
            .gap(theme::S2)
            .mt(theme::S1)
            .pt(theme::S4)
            .border_t_1()
            .border_color(p.border)
            .child(
                ui::button("eq-add", t("eq.add_point"), ButtonKind::Default)
                    .icon(icon(IconName::Add))
                    .disabled(!c.add)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.add_point(cx))),
            )
            .child(
                ui::button("eq-reset", t("eq.reset"), ButtonKind::Default)
                    .icon(icon(IconName::ArrowReset))
                    .disabled(!c.reset)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.reset(cx))),
            )
            .child(
                ui::button("eq-export", t("eq.export"), ButtonKind::Default)
                    .icon(icon(IconName::ArrowDownload))
                    .disabled(!c.export)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.export_points(cx))),
            );

        let enabled = eq.enabled;
        div()
            .flex()
            .flex_col()
            .gap(theme::S3)
            .child(ui::toggle(
                "eq-enable",
                t("eq.enable"),
                None,
                enabled,
                !c.toggle,
                cx.listener(|this, v: &bool, _, cx| {
                    // A new EQ starts with the default points
                    let next = toggled(&this.eq(cx), *v);
                    this.commit(next, cx);
                }),
                cx,
            ))
            .child(self.graph.clone())
            .child(ui::hint(t("eq.graph_hint"), cx))
            .child(fields)
            .child(actions)
            .when_some(self.export_error.clone(), |d, e| {
                d.child(div().text_size(theme::FS_SMALL).text_color(p.bad_text).child(e))
            })
    }
}

// ---- the graph ----------------------------------------------------------------------

/// What the graph asks of its owner
#[derive(Debug, Clone, PartialEq)]
pub enum EqGraphEvent {
    Select(Option<usize>),
    /// New points; `done`: the change is complete (save it), else it is
    /// being dragged
    Change(Vec<EqPoint>, bool),
}

/// The frequency-response graph with its points (`EqGraph`). Its owner
/// sets what it shows with [`EqGraph::set_props`] and takes its events.
pub struct EqGraph {
    bands: Vec<EqPoint>,
    selected: Option<usize>,
    /// Shown greyed out, nothing on it reacts (the EQ is off)
    disabled: bool,
    drag: Option<usize>,
    hover: Option<usize>,
    /// The points while dragging (the last move), saved when let go
    dragged: Option<Vec<EqPoint>>,
    menu: Option<(Entity<PopupMenu>, Point<Pixels>)>,
    menu_sub: Option<Subscription>,
    /// The graph's box on screen, from the last frame
    bounds: Rc<Cell<Bounds<Pixels>>>,
    focus: Vec<FocusHandle>,
    _focus_subs: Vec<Subscription>,
}

impl EventEmitter<EqGraphEvent> for EqGraph {}

impl EqGraph {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> EqGraph {
        let mut focus = Vec::new();
        let mut subs = Vec::new();
        for i in 0..MAX_POINTS {
            let handle = cx.focus_handle().tab_stop(true);
            subs.push(cx.on_focus(&handle, window, move |this: &mut Self, _, cx| {
                if !this.disabled && i < this.bands.len() {
                    cx.emit(EqGraphEvent::Select(Some(i)));
                }
            }));
            focus.push(handle);
        }
        EqGraph {
            bands: Vec::new(),
            selected: None,
            disabled: false,
            drag: None,
            hover: None,
            dragged: None,
            menu: None,
            menu_sub: None,
            bounds: Rc::new(Cell::new(Bounds::default())),
            focus,
            _focus_subs: subs,
        }
    }

    pub fn set_props(&mut self, bands: Vec<EqPoint>, selected: Option<usize>, disabled: bool, cx: &mut Context<Self>) {
        // A drag in progress keeps the points it moved
        if self.drag.is_none() {
            self.bands = bands;
        }
        self.selected = selected;
        if disabled {
            self.menu = None;
            self.drag = None;
            self.dragged = None;
        }
        self.disabled = disabled;
        for (i, f) in self.focus.iter().enumerate() {
            let _ = f.clone().tab_stop(!disabled && i < self.bands.len());
        }
        let _ = cx;
    }

    /// A window position in the graph's units
    fn at(&self, pos: Point<Pixels>) -> (f64, f64) {
        let b = self.bounds.get();
        let (w, h) = (f64::from(b.size.width), f64::from(b.size.height));
        if w <= 0.0 || h <= 0.0 {
            return (f64::NAN, f64::NAN);
        }
        (f64::from(pos.x - b.origin.x) * W / w, f64::from(pos.y - b.origin.y) * H / h)
    }

    /// A place of the graph in window pixels
    fn to_window(&self, x: f64, y: f64) -> Point<Pixels> {
        let b = self.bounds.get();
        point(
            b.origin.x + px((x / W * f64::from(b.size.width)) as f32),
            b.origin.y + px((y / H * f64::from(b.size.height)) as f32),
        )
    }

    fn change(&mut self, bands: Vec<EqPoint>, done: bool, cx: &mut Context<Self>) {
        cx.emit(EqGraphEvent::Change(bands, done));
    }

    fn select(&mut self, i: Option<usize>, cx: &mut Context<Self>) {
        cx.emit(EqGraphEvent::Select(i));
    }

    fn add_at(&mut self, x: f64, y: f64, cx: &mut Context<Self>) {
        let Some((bands, i)) = add_at(&self.bands, x, y) else { return };
        self.change(bands, true, cx);
        self.select(Some(i), cx);
    }

    fn on_background_down(&mut self, ev: &MouseDownEvent, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        if ev.click_count >= 2 {
            let (x, y) = self.at(ev.position);
            self.add_at(x, y, cx);
        } else {
            self.menu = None;
            self.select(None, cx);
        }
        cx.notify();
    }

    fn on_point_down(&mut self, i: usize, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if self.disabled {
            return;
        }
        let Some(b) = self.bands.get(i).copied() else { return };
        if ev.click_count >= 2 {
            // A double-click puts its gain back to 0
            if let Some(flat) = rules::gain_reset(&b) {
                self.change(rules::replace(&self.bands, i, &flat), true, cx);
            }
            return;
        }
        self.menu = None;
        self.focus[i].focus(window, cx);
        self.select(Some(i), cx);
        self.drag = Some(i);
        self.dragged = None;
        cx.notify();
    }

    fn on_move(&mut self, pos: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(i) = self.drag else { return };
        let (x, y) = self.at(pos);
        let Some(b) = self.bands.get(i) else { return };
        let Some(moved) = rules::dragged(b, x, y) else { return };
        let bands = rules::replace(&self.bands, i, &moved);
        self.bands = bands.clone();
        self.dragged = Some(bands.clone());
        self.change(bands, false, cx);
        cx.notify();
    }

    fn on_up(&mut self, cx: &mut Context<Self>) {
        if self.drag.take().is_none() {
            return;
        }
        if let Some(d) = self.dragged.take() {
            self.change(d, true, cx);
        }
        cx.notify();
    }

    fn on_wheel(&mut self, i: usize, ev: &ScrollEvent, cx: &mut Context<Self>) {
        let Some(b) = self.bands.get(i).copied() else { return };
        if self.disabled || !has_q(b.band_type) {
            return;
        }
        cx.stop_propagation();
        // As a browser's deltaY: positive scrolled down, about 100 a notch
        let delta_y = match ev.delta {
            ScrollDelta::Lines(l) => -f64::from(l.y) * 100.0,
            ScrollDelta::Pixels(p) => -f64::from(p.y),
        };
        if let Some(next) = rules::wheel_q(&b, delta_y) {
            self.change(rules::replace(&self.bands, i, &next), true, cx);
        }
    }

    fn on_key(&mut self, i: usize, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(b) = self.bands.get(i).copied() else { return };
        if self.disabled {
            return;
        }
        let Some(key) = EqKey::from_name(&ev.keystroke.key) else { return };
        match rules::key_action(&b, key, ev.keystroke.modifiers.shift) {
            KeyAction::Change(next) => {
                cx.stop_propagation();
                self.change(rules::replace(&self.bands, i, &next), true, cx);
            }
            KeyAction::Delete => {
                cx.stop_propagation();
                self.select(None, cx);
                self.change(without(&self.bands, i), true, cx);
            }
            KeyAction::OpenMenu => {
                cx.stop_propagation();
                let (x, y) = (x_of(f64::from(b.frequency)), point_y(&b));
                let at = self.to_window(x, y);
                self.open_menu(Some(i), (x, y), at, window, cx);
            }
            KeyAction::Ignore => {}
        }
    }

    /// The right-click menu: a point's type and Delete, or on the background
    /// Add point here and Reset
    fn open_menu(
        &mut self,
        point_index: Option<usize>,
        (x, y): (f64, f64),
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.entity().downgrade();
        let bands = self.bands.clone();
        let menu = PopupMenu::build(window, cx, move |mut menu, _, _| {
            match point_index.and_then(|i| bands.get(i).copied().map(|b| (i, b))) {
                Some((i, b)) => {
                    for kind in BAND_TYPES {
                        let weak = weak.clone();
                        menu = menu.item(menu_choice(type_label(kind).into(), b.band_type == kind, false).on_click(
                            move |_: &ClickEvent, _, cx| {
                                let _ = weak.update(cx, |g, cx| {
                                    let next = rules::replace(&g.bands, i, &with_type(&b, kind));
                                    g.change(next, true, cx);
                                });
                            },
                        ));
                    }
                    let weak = weak.clone();
                    menu.separator().item(menu_command(t("eq.delete_point").into(), false).on_click(
                        move |_: &ClickEvent, _, cx| {
                            let _ = weak.update(cx, |g, cx| {
                                g.select(None, cx);
                                let next = without(&g.bands, i);
                                g.change(next, true, cx);
                            });
                        },
                    ))
                }
                None => {
                    let (w1, w2) = (weak.clone(), weak.clone());
                    menu.item(menu_command(t("eq.add_point_here").into(), bands.len() >= MAX_POINTS).on_click(
                        move |_: &ClickEvent, _, cx| {
                            let _ = w1.update(cx, |g, cx| g.add_at(x, y, cx));
                        },
                    ))
                    .item(menu_command(t("eq.reset").into(), is_default_points(&bands)).on_click(
                        move |_: &ClickEvent, _, cx| {
                            let _ = w2.update(cx, |g, cx| {
                                g.select(None, cx);
                                g.change(DEFAULT_POINTS.to_vec(), true, cx);
                            });
                        },
                    ))
                }
            }
        });
        menu.focus_handle(cx).focus(window, cx);
        self.menu_sub = Some(cx.subscribe_in(&menu, window, |this: &mut Self, _, _: &DismissEvent, _, cx| {
            this.menu = None;
            cx.notify();
        }));
        self.menu = Some((menu, at));
        cx.notify();
    }

    fn marks(&self, cx: &App) -> Vec<Mark> {
        let p = cx.palette();
        let mut marks = vec![Mark::Rect {
            x: PLOT_LEFT,
            y: PLOT_TOP,
            w: PLOT_RIGHT - PLOT_LEFT,
            h: PLOT_BOTTOM - PLOT_TOP,
            fill: p.surface_2,
            stroke: None,
        }];
        for f in GRID_F {
            let x = x_of(f);
            marks.push(Mark::Line {
                points: vec![(x, PLOT_TOP), (x, PLOT_BOTTOM)],
                color: p.border,
                width: 1.0,
                dashed: false,
            });
        }
        for db in GRID_DB {
            let y = y_of(db);
            let color = if db == 0.0 { p.border_strong } else { p.border };
            marks.push(Mark::Line { points: vec![(PLOT_LEFT, y), (PLOT_RIGHT, y)], color, width: 1.0, dashed: false });
        }
        // The curve, held to the plot
        let line: Vec<(f64, f64)> = response_curve(&self.bands, 220, GRAPH_RATE)
            .iter()
            .map(|c| (x_of(c.f), y_of(c.db).clamp(PLOT_TOP, PLOT_BOTTOM)))
            .collect();
        let zero = y_of(0.0);
        let mut fill = line.clone();
        fill.push((PLOT_RIGHT, zero));
        fill.push((PLOT_LEFT, zero));
        let (stroke, fill_color) = if self.disabled {
            (p.disabled_text, Hsla { a: p.disabled_text.a * 0.08, ..p.disabled_text })
        } else {
            (p.accent_fill, Hsla { a: 0.14, ..p.accent_fill })
        };
        marks.push(Mark::Shape { points: fill, fill: fill_color, stroke: None });
        marks.push(Mark::Line { points: line, color: stroke, width: 2.5, dashed: false });
        marks
    }

    fn point_el(&self, i: usize, b: &EqPoint, window: &Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let p = *cx.palette();
        let on = self.selected == Some(i);
        let focused = self.focus[i].is_focused(window);
        let halo = !self.disabled && (on || self.hover == Some(i) || focused);
        let (dot_fill, dot_line, num) = if self.disabled {
            (p.surface, p.disabled_text, p.disabled_text)
        } else if on {
            (p.accent_fill, p.surface, p.on_accent)
        } else {
            (p.surface, p.accent_fill, p.accent_fill)
        };
        let (x, y) = (x_of(f64::from(b.frequency)), point_y(b));
        div()
            .id(("eq-point", i))
            .role(Role::Button)
            .aria_label(point_label(b, i))
            .absolute()
            .left(relative((x / W) as f32))
            .top(relative((y / H) as f32))
            .ml(px(-14.))
            .mt(px(-14.))
            .size(px(28.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .when(halo, |d| d.bg(Hsla { a: 0.22, ..p.accent_fill }))
            .when(focused && !self.disabled, |d| d.border_2().border_color(p.focus))
            .when(!self.disabled, |d| {
                // Pressed by assistive technology: selected, as by a click,
                // for the boxes under the graph
                let weak = cx.entity().downgrade();
                d.on_a11y_action(AccessibleAction::Click, move |_, window, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.focus[i].focus(window, cx);
                        this.select(Some(i), cx);
                        cx.notify();
                    });
                })
                .cursor(if self.drag.is_some() { CursorStyle::ClosedHand } else { CursorStyle::OpenHand })
                .track_focus(&self.focus[i])
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, ev: &MouseDownEvent, window, cx| this.on_point_down(i, ev, window, cx)),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        if this.disabled {
                            return;
                        }
                        let Some(b) = this.bands.get(i).copied() else { return };
                        this.select(Some(i), cx);
                        this.open_menu(Some(i), (x_of(f64::from(b.frequency)), point_y(&b)), ev.position, window, cx);
                    }),
                )
                .on_scroll(cx.listener(move |this, ev: &ScrollEvent, _, cx| this.on_wheel(i, ev, cx)))
                .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, window, cx| this.on_key(i, ev, window, cx)))
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    if *hovered {
                        this.hover = Some(i);
                    } else if this.hover == Some(i) {
                        this.hover = None;
                    }
                    cx.notify();
                }))
            })
            .child(
                div()
                    .size(px(18.))
                    .rounded_full()
                    .bg(dot_fill)
                    .border_2()
                    .border_color(dot_line)
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(10.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(num)
                    .child((i + 1).to_string()),
            )
    }

    fn axis_labels(&self, cx: &App) -> Vec<Div> {
        let p = cx.palette();
        let label = |text: String| div().absolute().text_size(px(11.)).text_color(p.text_3).h(px(14.)).child(text);
        let mut out = Vec::new();
        for f in GRID_F {
            out.push(
                label(freq_axis_label(f))
                    .left(relative((x_of(f) / W) as f32))
                    .ml(px(-20.))
                    .w(px(40.))
                    .flex()
                    .justify_center()
                    .top(relative(((H - 10.0) / H) as f32))
                    .mt(px(-7.)),
            );
        }
        for db in GRID_DB {
            out.push(
                label(db_axis_label(db))
                    .left_0()
                    .w(relative(((PLOT_LEFT - 6.0) / W) as f32))
                    .flex()
                    .justify_end()
                    .top(relative((y_of(db) / H) as f32))
                    .mt(px(-7.)),
            );
        }
        out
    }
}

impl Render for EqGraph {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let weak = cx.entity().downgrade();
        let dragging = self.drag.is_some();
        // While a point is dragged the graph follows the pointer anywhere in
        // the window
        let capture = canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                if !dragging {
                    return;
                }
                let w = weak.clone();
                window.on_mouse_event(move |ev: &MouseMoveEvent, phase, _, cx| {
                    if phase.bubble() {
                        let _ = w.update(cx, |g, cx| g.on_move(ev.position, cx));
                    }
                });
                let w = weak.clone();
                window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                    if phase.bubble() {
                        let _ = w.update(cx, |g, cx| g.on_up(cx));
                    }
                });
            },
        )
        .absolute()
        .size_full();

        let points: Vec<_> =
            self.bands.clone().iter().enumerate().map(|(i, b)| self.point_el(i, b, window, cx)).collect();

        // The hovered or dragged point's values (not over its menu)
        let tip = if self.menu.is_some() || self.disabled {
            None
        } else {
            self.drag.or(self.hover).and_then(|i| self.bands.get(i).copied())
        };
        let tip = tip.map(|b| {
            let (x, y) = (x_of(f64::from(b.frequency)), point_y(&b));
            let left = x > W * 0.7;
            let below = y < 40.0;
            let anchor = match (left, below) {
                (false, true) => Anchor::TopLeft,
                (true, true) => Anchor::TopRight,
                (false, false) => Anchor::BottomLeft,
                (true, false) => Anchor::BottomRight,
            };
            let offset = point(px(if left { -14. } else { 14. }), px(if below { 12. } else { -12. }));
            deferred(
                anchored().position(self.to_window(x, y)).anchor(anchor).offset(offset).child(
                    div()
                        .px(theme::S2)
                        .py(theme::S1)
                        .border_1()
                        .border_color(p.border_strong)
                        .rounded(theme::RADIUS)
                        .bg(p.surface_2)
                        .text_color(p.text)
                        .text_size(theme::FS_SMALL)
                        .whitespace_nowrap()
                        .child(tip_text(&b)),
                ),
            )
            .with_priority(1)
        });
        let menu = self.menu.as_ref().map(|(m, at)| {
            deferred(anchored().position(*at).snap_to_window_with_margin(px(8.)).child(m.clone())).with_priority(2)
        });

        div()
            .id("eq-graph")
            .role(Role::Group)
            .aria_label(t("eq.graph_label"))
            .relative()
            .w_full()
            .aspect_ratio((W / H) as f32)
            .when(dragging, |d| d.cursor(CursorStyle::ClosedHand))
            .when(!self.disabled, |d| {
                d.on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, ev: &MouseDownEvent, _, cx| this.on_background_down(ev, cx)),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                        if this.disabled {
                            return;
                        }
                        let at = this.at(ev.position);
                        this.open_menu(None, at, ev.position, window, cx);
                    }),
                )
            })
            .child(plot(ViewBox::new(0.0, 0.0, W, H), self.marks(cx), Some(self.bounds.clone())).absolute().size_full())
            .children(self.axis_labels(cx))
            .children(points)
            .child(capture)
            .children(tip)
            .children(menu)
    }
}

#[cfg(test)]
mod interaction_tests;
