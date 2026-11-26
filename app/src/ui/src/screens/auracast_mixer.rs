//! The matrix mixers (Developer screen): which source speakers (columns FL
//! FR FC LFE BL BR SL SR) make each output, plus a delay and polarity per
//! output. Automatic from three levels, or each gain by hand; named presets.
//! Auracast's (under the broadcast form, rows FL FR RL RR, auracast.*) and
//! the earbuds' of an LE Audio stream to a device (the TWS tab, rows FL FR,
//! tws.*: used once turned on, from the next connection). Every change is
//! saved and, while a stream runs with the mixer, heard at once.

use std::time::Duration;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, AsyncWindowContext, Context, Div, Entity, Focusable as _, InteractiveElement as _,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled, Subscription, Task, WeakEntity, Window,
    div, px,
};
use ipc::{AuracastMix, AuracastMixState, Method};
use ui_model::i18n::t;
use ui_model::view::auracast::{
    INPUT_NAMES, MAX_GAIN, MIX_SEND_MS, OUTPUT_NAMES, Unit, can_save_preset, delay_from, delay_in, gain_text, num,
    preset_label,
};

use crate::app;
use crate::theme::{self, ActivePalette as _};
use crate::ui::{self, ButtonKind, Choice, IconName, icon};

/// A field of the mixer: what its text stands for
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MixField {
    Cell(usize, usize),
    Delay(usize),
    Center,
    Lfe,
    Surround,
}

/// Whose mixer
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MixTarget {
    /// A broadcast's, four outputs
    Auracast,
    /// An LE Audio stream to a device's, two outputs
    Tws,
}

pub struct AuracastMixer {
    target: MixTarget,
    state: Option<AuracastMixState>,
    unit: Unit,
    preset: String,
    preset_name: Entity<InputState>,
    /// The grid's gains (4 rows of 8), each output's delay, the three levels
    fields: Vec<(MixField, Entity<InputState>)>,
    /// The broadcast's rear and whether it is on air (None: not shown)
    props: Option<(bool, bool)>,
    /// Changes go out after a short pause (typing, dragging); an answer is
    /// taken only when nothing changed since it was asked
    edits: u64,
    timer: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

fn new_input(window: &mut Window, cx: &mut Context<AuracastMixer>) -> Entity<InputState> {
    cx.new(|cx| InputState::new(window, cx))
}

impl AuracastMixer {
    /// Auracast's mixer
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> AuracastMixer {
        Self::new_for(MixTarget::Auracast, window, cx)
    }

    pub fn new_for(target: MixTarget, window: &mut Window, cx: &mut Context<Self>) -> AuracastMixer {
        let preset_name = cx.new(|cx| InputState::new(window, cx).placeholder(t("ui.dev.auracast_mix_preset_name")));
        let mut subs = vec![cx.subscribe_in(&preset_name, window, |_, _, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::Change) {
                cx.notify();
            }
        })];
        let mut fields = Vec::new();
        let mut kinds: Vec<MixField> =
            (0..OUTPUT_NAMES.len()).flat_map(|r| (0..INPUT_NAMES.len()).map(move |c| MixField::Cell(r, c))).collect();
        kinds.extend((0..OUTPUT_NAMES.len()).map(MixField::Delay));
        kinds.extend([MixField::Center, MixField::Lfe, MixField::Surround]);
        for kind in kinds {
            let input = new_input(window, cx);
            // A field's value counts when it is left or Enter is pressed
            subs.push(cx.subscribe_in(&input, window, move |this, _, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    this.commit(kind, window, cx);
                }
            }));
            fields.push((kind, input));
        }
        AuracastMixer {
            target,
            state: None,
            unit: Unit::Ms,
            preset: String::new(),
            preset_name,
            fields,
            props: None,
            edits: 0,
            timer: None,
            _subs: subs,
        }
    }

    /// The broadcast's rear and live (None: the mixer is not shown, its
    /// state goes); a change reads the mixer again
    pub fn set_props(&mut self, props: Option<(bool, bool)>, window: &mut Window, cx: &mut Context<Self>) {
        if self.props == props {
            return;
        }
        self.props = props;
        match props {
            None => {
                self.state = None;
                self.preset.clear();
                self.timer = None;
            }
            Some((rear, _)) => self.load(rear, window, cx),
        }
        cx.notify();
    }

    fn rear(&self) -> Option<bool> {
        self.props.map(|(rear, _)| rear)
    }

    fn tws(&self) -> bool {
        self.target == MixTarget::Tws
    }

    /// The outputs shown: the earbuds' two, a broadcast's four
    fn outputs(&self) -> &'static [&'static str] {
        let all: &'static [&'static str] = &OUTPUT_NAMES;
        if self.tws() { &all[..2] } else { all }
    }

    fn get_method(&self, rear: bool) -> Method {
        match self.target {
            MixTarget::Auracast => Method::AuracastMixGet { channels: None, rear: Some(rear) },
            MixTarget::Tws => Method::TwsMixGet { channels: None },
        }
    }

    fn set_method(&self, mix: AuracastMix, rear: bool, enabled: Option<bool>) -> Method {
        match self.target {
            MixTarget::Auracast => Method::AuracastMixSet { mix, channels: None, rear: Some(rear) },
            MixTarget::Tws => Method::TwsMixSet { mix, enabled, channels: None },
        }
    }

    fn load(&mut self, rear: bool, window: &mut Window, cx: &mut Context<Self>) {
        let task = app::request::<AuracastMixState>(self.get_method(rear), cx);
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
            let Ok(st) = task.await else { return };
            let _ = this.update_in(cx, |this, window, cx| this.take(st, window, cx));
        })
        .detach();
    }

    fn take(&mut self, st: AuracastMixState, window: &mut Window, cx: &mut Context<Self>) {
        self.state = Some(st);
        self.sync_fields(window, cx);
        cx.notify();
    }

    fn mix(&self) -> Option<&AuracastMix> {
        self.state.as_ref().map(|s| &s.mix)
    }

    fn cell(&self, row: usize, col: usize) -> f64 {
        let Some(st) = &self.state else { return 0. };
        let m = if st.mix.auto { &st.effective } else { &st.mix.matrix };
        m.get(row).and_then(|r| r.get(col)).copied().map_or(0., f64::from)
    }

    /// A field's text from the state
    fn text_of(&self, kind: MixField) -> String {
        let Some(mix) = self.mix() else { return String::new() };
        match kind {
            MixField::Cell(r, c) => gain_text(self.cell(r, c)),
            MixField::Delay(r) => delay_in(mix.delay_ms.get(r).copied().map_or(0., f64::from), self.unit),
            MixField::Center => ui_model::i18n::js_number(f64::from(mix.center_db)),
            MixField::Lfe => ui_model::i18n::js_number(f64::from(mix.lfe_db)),
            MixField::Surround => ui_model::i18n::js_number(f64::from(mix.surround_db)),
        }
    }

    /// The fields show the state, but for the one being typed in
    fn sync_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for (kind, input) in self.fields.clone() {
            let text = self.text_of(kind);
            let focused = input.read(cx).focus_handle(cx).is_focused(window);
            if !focused && input.read(cx).value() != text.as_str() {
                input.update(cx, |s, cx| s.set_value(text, window, cx));
            }
        }
    }

    fn reset_field(&mut self, kind: MixField, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.text_of(kind);
        if let Some((_, input)) = self.fields.iter().find(|(k, _)| *k == kind) {
            input.update(cx, |s, cx| s.set_value(text, window, cx));
        }
    }

    fn commit(&mut self, kind: MixField, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mix) = self.mix().cloned() else { return };
        let Some((_, input)) = self.fields.iter().find(|(k, _)| *k == kind) else { return };
        let text = input.read(cx).value().to_string();
        if text == self.text_of(kind) {
            return;
        }
        let mut next = mix;
        let ok = match kind {
            MixField::Cell(r, c) => {
                if next.auto {
                    false
                } else if let Some(g) = num(&text, -MAX_GAIN, MAX_GAIN) {
                    if let Some(cell) = next.matrix.get_mut(r).and_then(|row| row.get_mut(c)) {
                        *cell = g as f32;
                    }
                    true
                } else {
                    false
                }
            }
            MixField::Delay(r) => match delay_from(&text, self.unit) {
                Some(ms) => {
                    if let Some(d) = next.delay_ms.get_mut(r) {
                        *d = ms as f32;
                    }
                    true
                }
                None => false,
            },
            MixField::Center | MixField::Lfe | MixField::Surround => match num(&text, -60., 12.) {
                Some(v) => {
                    let v = v as f32;
                    match kind {
                        MixField::Center => next.center_db = v,
                        MixField::Lfe => next.lfe_db = v,
                        _ => next.surround_db = v,
                    }
                    true
                }
                None => false,
            },
        };
        if ok {
            self.change(next, window, cx);
        } else {
            self.reset_field(kind, window, cx);
        }
    }

    fn change(&mut self, mix: AuracastMix, window: &mut Window, cx: &mut Context<Self>) {
        let rear = self.rear();
        let (Some(st), Some(rear)) = (self.state.as_mut(), rear) else { return };
        st.mix = mix.clone();
        self.edits += 1;
        let n = self.edits;
        self.sync_fields(window, cx);
        cx.notify();
        let method = self.set_method(mix, rear, None);
        self.timer = Some(cx.spawn_in(window, async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
            cx.background_executor().timer(Duration::from_millis(MIX_SEND_MS)).await;
            let Ok(task) = this.update(cx, |_, cx| app::run(app::request::<AuracastMixState>(method, cx), cx)) else {
                return;
            };
            let Some(r) = task.await else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                if n == this.edits {
                    this.take(r, window, cx);
                }
            });
        }));
    }

    /// TWS: turned on or off, from the next connection (a change on its way
    /// goes with it)
    fn set_enabled(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(mix), Some(rear)) = (self.mix().cloned(), self.rear()) else { return };
        self.timer = None;
        self.edits += 1;
        let n = self.edits;
        if let Some(st) = self.state.as_mut() {
            st.enabled = on;
        }
        cx.notify();
        let task = app::run(app::request::<AuracastMixState>(self.set_method(mix, rear, Some(on)), cx), cx);
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
            let Some(r) = task.await else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                if n == this.edits {
                    this.take(r, window, cx);
                }
            });
        })
        .detach();
    }

    /// By hand, starting from the matrix that plays
    fn manual(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(st) = &self.state else { return };
        let mix = AuracastMix { auto: false, matrix: st.effective.clone(), ..st.mix.clone() };
        self.change(mix, window, cx);
    }

    fn choose_preset(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.preset = name.to_owned();
        let Some(p) = self.state.as_ref().and_then(|s| s.presets.iter().find(|x| x.name == name)).cloned() else {
            return;
        };
        if !p.builtin {
            self.preset_name.update(cx, |s, cx| s.set_value(p.name.clone(), window, cx));
        }
        self.change(p.mix, window, cx);
    }

    fn save_preset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mix) = self.mix().cloned() else { return };
        let name = self.preset_name.read(cx).value().trim().to_owned();
        let method = match self.target {
            MixTarget::Auracast => Method::AuracastMixPresetSave { name: name.clone(), mix: mix.clone() },
            MixTarget::Tws => Method::TwsMixPresetSave { name: name.clone(), mix: mix.clone() },
        };
        let task = app::run(app::request::<AuracastMixState>(method, cx), cx);
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
            let Some(r) = task.await else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                this.preset = name;
                this.take(AuracastMixState { mix, ..r }, window, cx);
            });
        })
        .detach();
    }

    fn delete_preset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mix) = self.mix().cloned() else { return };
        let name = self.preset.clone();
        let method = match self.target {
            MixTarget::Auracast => Method::AuracastMixPresetDelete { name },
            MixTarget::Tws => Method::TwsMixPresetDelete { name },
        };
        let task = app::run(app::request::<AuracastMixState>(method, cx), cx);
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
            let Some(r) = task.await else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                this.preset.clear();
                this.take(AuracastMixState { mix, ..r }, window, cx);
            });
        })
        .detach();
    }

    fn input(&self, kind: MixField) -> Entity<InputState> {
        self.fields.iter().find(|(k, _)| *k == kind).map(|(_, i)| i.clone()).expect("every field has an input")
    }

    /// The mixer's section with `children` (the broadcast's own settings) at
    /// the top of the frame; None until its state is read
    pub fn section(&mut self, children: Vec<AnyElement>, cx: &mut Context<Self>) -> Option<Div> {
        let st = self.state.clone()?;
        let p = *cx.palette();
        let mix = &st.mix;
        let builtin: Vec<&str> = st.presets.iter().filter(|p| p.builtin).map(|p| p.name.as_str()).collect();
        let preset_name = self.preset_name.read(cx).value().to_string();
        let can_save = can_save_preset(&preset_name, &builtin);
        let selected_mine = st.presets.iter().any(|p| p.name == self.preset && !p.builtin);

        let mut presets = vec![Choice::new(String::new(), t("ui.dev.auracast_mix_preset_pick")).disabled(true)];
        presets.extend(st.presets.iter().map(|p| {
            let group = if p.builtin { t("ui.dev.auracast_mix_builtin") } else { t("ui.dev.auracast_mix_mine") };
            Choice::new(p.name.clone(), preset_label(&p.name, p.builtin)).group(group)
        }));
        let this = cx.entity().downgrade();
        let preset_select = ui::select_labelled(
            "mix-preset",
            t("ui.dev.auracast_mix_preset"),
            &self.preset,
            presets,
            false,
            move |v: &String, window, cx| {
                let v = v.clone();
                let _ = this.update(cx, |m, cx| m.choose_preset(&v, window, cx));
            },
        );

        let unit_label = match self.unit {
            Unit::Ms => t("ui.dev.auracast_mix_ms"),
            Unit::M => t("ui.dev.auracast_mix_m"),
        };
        fn absent<E: Styled>(on: bool, d: E) -> E {
            if on { d } else { d.opacity(0.4) }
        }
        // The columns share the card's width: the gains and the delay grow and
        // shrink together between a least and a most, the names keep theirs.
        // The header's cells take the same widths as the rows', so they line
        // up; below the least width the grid scrolls sideways.
        fn name_col<E: Styled>(d: E) -> E {
            d.w(px(40.)).flex_none()
        }
        fn gain_col<E: Styled>(d: E) -> E {
            d.flex_1().min_w(px(48.)).max_w(px(72.))
        }
        fn delay_col<E: Styled>(d: E) -> E {
            d.flex_1().min_w(px(64.)).max_w(px(88.))
        }
        fn invert_col<E: Styled>(d: E) -> E {
            d.w(px(64.)).flex_none()
        }
        let head_cell = |text: SharedString| gain_col(div()).text_center().text_color(p.text_2).child(text);
        let grid = div()
            .flex()
            .flex_col()
            .min_w(px(40. + 8. * 48. + 64. + 64. + 10. * 4.))
            .gap(px(2.))
            .text_size(theme::FS_SMALL)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(name_col(div()).truncate().text_color(p.text_2).child(t("ui.dev.auracast_mix_output")))
                    .children(INPUT_NAMES.iter().enumerate().map(|(c, name)| {
                        absent(st.present.get(c).copied().unwrap_or(false), head_cell((*name).into()))
                    }))
                    .child(
                        delay_col(div())
                            .truncate()
                            .text_center()
                            .text_color(p.text_2)
                            .child(format!("{} ({unit_label})", t("ui.dev.auracast_mix_delay"))),
                    )
                    .child(
                        invert_col(div())
                            .truncate()
                            .text_center()
                            .text_color(p.text_2)
                            .child(t("ui.dev.auracast_mix_invert")),
                    ),
            )
            .children(self.outputs().iter().enumerate().map(|(r, out)| {
                let row_on = r < 2 || st.rear;
                let cells = INPUT_NAMES.iter().enumerate().map(|(c, inp)| {
                    let on = self.cell(r, c) != 0.;
                    let input = Input::new(&self.input(MixField::Cell(r, c)))
                        .readonly(mix.auto)
                        .aria_label(SharedString::from(format!("{inp} → {out}")))
                        .w_full()
                        .when(on, |i| i.border_color(p.accent_fill))
                        .when(mix.auto, |i| i.bg(p.surface_2));
                    absent(
                        st.present.get(c).copied().unwrap_or(false),
                        gain_col(div())
                            .id(("mix-cell", r * 8 + c))
                            .on_click(cx.listener(|this, ev: &gpui_kit::ClickEvent, window, cx| {
                                if ev.click_count() == 2 && this.mix().is_some_and(|m| m.auto) {
                                    this.manual(window, cx);
                                }
                            }))
                            .child(input),
                    )
                });
                let invert = mix.invert.get(r).copied().unwrap_or(false);
                absent(
                    row_on,
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .child(name_col(div()).text_center().child(*out))
                        .children(cells)
                        .child(
                            delay_col(div()).child(
                                Input::new(&self.input(MixField::Delay(r)))
                                    .aria_label(SharedString::from(format!("{} {out}", t("ui.dev.auracast_mix_delay"))))
                                    .w_full(),
                            ),
                        )
                        .child(
                            invert_col(div()).flex().justify_center().child(
                                Switch::new(("mix-invert", r))
                                    .small()
                                    .checked(invert)
                                    .accessibility_label(SharedString::from(format!(
                                        "{} {out}",
                                        t("ui.dev.auracast_mix_invert")
                                    )))
                                    .on_change(cx.listener(move |this, v: &bool, window, cx| {
                                        let Some(mut mix) = this.mix().cloned() else { return };
                                        if let Some(x) = mix.invert.get_mut(r) {
                                            *x = *v;
                                        }
                                        this.change(mix, window, cx);
                                    })),
                            ),
                        ),
                )
            }));

        let level = |label: String, kind: MixField| {
            div()
                .flex()
                .items_center()
                .gap(theme::S2)
                .child(label.clone())
                .child(Input::new(&self.input(kind)).aria_label(label).w(px(80.)))
        };
        let auto_row = ui::toggle(
            "mix-auto",
            t("ui.dev.auracast_mix_auto"),
            Some(t("ui.dev.auracast_mix_auto_hint").into()),
            mix.auto,
            false,
            cx.listener(|this, v: &bool, window, cx| {
                if *v {
                    if let Some(mix) = this.mix().cloned() {
                        this.change(AuracastMix { auto: true, ..mix }, window, cx);
                    }
                } else {
                    this.manual(window, cx);
                }
            }),
            cx,
        );
        let tws = self.tws();
        let live_hint = match (tws, st.live) {
            (false, true) => t("ui.dev.auracast_mix_live"),
            (false, false) => t("ui.dev.auracast_mix_next"),
            (true, true) => t("ui.dev.tws_mix_live"),
            (true, false) => t("ui.dev.tws_mix_next"),
        };
        let enabled_row = tws.then(|| {
            ui::toggle(
                "mix-enabled",
                t("ui.dev.tws_mix_enabled"),
                Some(t("ui.dev.tws_mix_enabled_hint").into()),
                st.enabled,
                false,
                cx.listener(|this, v: &bool, window, cx| this.set_enabled(*v, window, cx)),
                cx,
            )
        });
        let title = if tws { t("ui.dev.tws_mix") } else { t("ui.dev.auracast_mix") };
        let section = ui::section(title, None, cx)
            .children(children)
            .children(enabled_row)
            .child(ui::hint(live_hint, cx))
            .child(ui::row().child(div().child(t("ui.dev.auracast_mix_preset"))).child(preset_select))
            .child(
                ui::row()
                    .justify_start()
                    .child(Input::new(&self.preset_name).aria_label(t("ui.dev.auracast_mix_preset_name")).flex_1())
                    .child(
                        ui::button("mix-save", t("ui.dev.auracast_mix_save"), ButtonKind::Default)
                            .icon(icon(IconName::Checkmark))
                            .disabled(!can_save)
                            .on_click(cx.listener(|this, _, window, cx| this.save_preset(window, cx))),
                    )
                    .when(selected_mine, |d| {
                        d.child(
                            ui::button("mix-delete", t("ui.dev.auracast_mix_delete"), ButtonKind::Danger)
                                .icon(icon(IconName::Delete))
                                .on_click(cx.listener(|this, _, window, cx| this.delete_preset(window, cx))),
                        )
                    }),
            )
            .child(auto_row)
            .when(mix.auto, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(theme::S4)
                        .py(theme::S2)
                        .child(level(t("ui.dev.auracast_mix_center"), MixField::Center))
                        .child(level(t("ui.dev.auracast_mix_lfe"), MixField::Lfe))
                        .child(level(t("ui.dev.auracast_mix_surround"), MixField::Surround))
                        .child(ui::check(
                            "mix-normalize",
                            t("ui.dev.auracast_mix_normalize"),
                            mix.normalize,
                            false,
                            cx.listener(|this, v: &bool, window, cx| {
                                if let Some(mix) = this.mix().cloned() {
                                    this.change(AuracastMix { normalize: *v, ..mix }, window, cx);
                                }
                            }),
                        )),
                )
            })
            .child(div().id("mix-scroll").overflow_x_scroll().py(theme::S2).child(grid))
            .when(!st.rear && !tws, |d| d.child(ui::hint(t("ui.dev.auracast_mix_no_rear"), cx)))
            .child(
                ui::row()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(2.))
                            .child(t("ui.dev.auracast_mix_unit"))
                            .child(ui::hint(t("ui.dev.auracast_mix_delay_hint"), cx)),
                    )
                    .child(ui::segmented(
                        "mix-unit",
                        &self.unit,
                        vec![
                            Choice::new(Unit::Ms, t("ui.dev.auracast_mix_ms")),
                            Choice::new(Unit::M, t("ui.dev.auracast_mix_m")),
                        ],
                        {
                            let this = cx.entity().downgrade();
                            move |v: &Unit, window, cx| {
                                let v = *v;
                                let _ = this.update(cx, |m, cx| {
                                    m.unit = v;
                                    m.sync_fields(window, cx);
                                    cx.notify();
                                });
                            }
                        },
                    )),
            );
        Some(section)
    }
}

#[cfg(test)]
mod interaction_tests;
