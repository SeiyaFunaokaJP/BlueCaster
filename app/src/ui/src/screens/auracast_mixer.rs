//! The matrix mixers (Developer screen): which source speakers (columns FL
//! FR FC LFE BL BR SL SR) make each output, plus a delay and polarity per
//! output. Automatic from three levels (the LFE to the outputs marked full
//! range), or each gain by hand; named presets. Solo and mute per row and
//! column for listening tests (heard at once, not saved).
//! Auracast's (under the broadcast form, rows FL FR RL RR, auracast.*) and
//! the earbuds' of an LE Audio stream to a device (the TWS tab, rows FL FR,
//! tws.*: used once turned on, from the next connection). Every change is
//! saved and, while a stream runs with the mixer, heard at once.

use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, AsyncWindowContext, Context, Div, Entity, Focusable as _, InteractiveElement as _,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled, Subscription, Task, WeakEntity, Window,
    div, px,
};
use ipc::{AuracastMix, AuracastMixState, AuracastSub, Method};
use ui_model::i18n::t;
use ui_model::view::auracast::{
    CROSSOVER_CHOICES, FULL_RANGE_DEFAULT, INPUT_NAMES, MAX_GAIN, MIX_SEND_MS, Unit, can_save_preset, crossover_choice,
    delay_from, delay_in, flag, full_range, gain_text, heard, num, output_names, preset_label, set_flag,
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

/// A switch of the mixer per input or output
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MixFlag {
    SoloIn,
    MuteIn,
    SoloOut,
    MuteOut,
    FullRange,
}

impl MixFlag {
    /// Its list in `mix`, the list's length and what a missing entry is
    fn of(self, mix: &mut AuracastMix) -> (&mut Vec<bool>, usize, &'static [bool]) {
        match self {
            MixFlag::SoloIn => (&mut mix.solo_in, INPUT_NAMES.len(), &[]),
            MixFlag::MuteIn => (&mut mix.mute_in, INPUT_NAMES.len(), &[]),
            MixFlag::SoloOut => (&mut mix.solo_out, 4, &[]),
            MixFlag::MuteOut => (&mut mix.mute_out, 4, &[]),
            MixFlag::FullRange => (&mut mix.full_range, 4, &FULL_RANGE_DEFAULT),
        }
    }

    fn get(self, mix: &AuracastMix, i: usize) -> bool {
        match self {
            MixFlag::SoloIn => flag(&mix.solo_in, i),
            MixFlag::MuteIn => flag(&mix.mute_in, i),
            MixFlag::SoloOut => flag(&mix.solo_out, i),
            MixFlag::MuteOut => flag(&mix.mute_out, i),
            MixFlag::FullRange => full_range(&mix.full_range, i),
        }
    }

    fn id(self) -> &'static str {
        match self {
            MixFlag::SoloIn => "mix-solo-in",
            MixFlag::MuteIn => "mix-mute-in",
            MixFlag::SoloOut => "mix-solo-out",
            MixFlag::MuteOut => "mix-mute-out",
            MixFlag::FullRange => "mix-full-range",
        }
    }
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
    /// The broadcast's rear, whether it is 2.1 and whether it is on air
    /// (None: not shown)
    props: Option<(bool, AuracastSub, bool)>,
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
            (0..4).flat_map(|r| (0..INPUT_NAMES.len()).map(move |c| MixField::Cell(r, c))).collect();
        kinds.extend((0..4).map(MixField::Delay));
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

    /// The broadcast's rear, 2.1 and live (None: the mixer is not shown,
    /// its state goes); a change reads the mixer again
    pub fn set_props(&mut self, props: Option<(bool, AuracastSub, bool)>, window: &mut Window, cx: &mut Context<Self>) {
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
            Some((rear, sub, _)) => self.load(rear, sub, window, cx),
        }
        cx.notify();
    }

    /// The broadcast's rear and 2.1
    fn layout(&self) -> Option<(bool, AuracastSub)> {
        self.props.map(|(rear, sub, _)| (rear, sub))
    }

    fn tws(&self) -> bool {
        self.target == MixTarget::Tws
    }

    /// The output rows shown: the earbuds' two, a broadcast's four, named
    /// after its layout
    fn output_names(&self, sub: AuracastSub) -> Vec<&'static str> {
        let all = output_names(sub);
        if self.tws() { all[..2].to_vec() } else { all.to_vec() }
    }

    fn get_method(&self, rear: bool, sub: AuracastSub) -> Method {
        match self.target {
            MixTarget::Auracast => Method::AuracastMixGet { channels: None, rear: Some(rear), sub: Some(sub) },
            MixTarget::Tws => Method::TwsMixGet { channels: None },
        }
    }

    fn set_method(&self, mix: AuracastMix, (rear, sub): (bool, AuracastSub), enabled: Option<bool>) -> Method {
        match self.target {
            MixTarget::Auracast => Method::AuracastMixSet { mix, channels: None, rear: Some(rear), sub: Some(sub) },
            MixTarget::Tws => Method::TwsMixSet { mix, enabled, channels: None },
        }
    }

    fn load(&mut self, rear: bool, sub: AuracastSub, window: &mut Window, cx: &mut Context<Self>) {
        let task = app::request::<AuracastMixState>(self.get_method(rear, sub), cx);
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
        let layout = self.layout();
        let (Some(st), Some(layout)) = (self.state.as_mut(), layout) else { return };
        st.mix = mix.clone();
        self.edits += 1;
        let n = self.edits;
        self.sync_fields(window, cx);
        cx.notify();
        let method = self.set_method(mix, layout, None);
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
        let (Some(mix), Some(layout)) = (self.mix().cloned(), self.layout()) else { return };
        self.timer = None;
        self.edits += 1;
        let n = self.edits;
        if let Some(st) = self.state.as_mut() {
            st.enabled = on;
        }
        cx.notify();
        let task = app::run(app::request::<AuracastMixState>(self.set_method(mix, layout, Some(on)), cx), cx);
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
        // Solo and mute stay as they are (a preset has none)
        let mix = match self.mix().cloned() {
            Some(cur) => AuracastMix {
                solo_in: cur.solo_in,
                mute_in: cur.mute_in,
                solo_out: cur.solo_out,
                mute_out: cur.mute_out,
                ..p.mix
            },
            None => p.mix,
        };
        self.change(mix, window, cx);
    }

    /// Solo, mute or full range of input or output `i` turned on or off
    fn set_mix_flag(&mut self, f: MixFlag, i: usize, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut mix) = self.mix().cloned() else { return };
        let (list, len, fill) = f.of(&mut mix);
        set_flag(list, len, fill, i, on);
        self.change(mix, window, cx);
    }

    /// The S and M buttons of input or output `i` (`name`)
    fn solo_mute(&self, (solo, mute): (MixFlag, MixFlag), i: usize, name: &str, cx: &mut Context<Self>) -> Div {
        let mut d = div().flex().justify_center().gap(px(2.));
        for (f, label, key) in [(solo, "S", "ui.dev.auracast_mix_solo"), (mute, "M", "ui.dev.auracast_mix_mute")] {
            let on = self.mix().is_some_and(|m| f.get(m, i));
            let what = SharedString::from(format!("{} {name}", t(key)));
            let b = Button::new((f.id(), i))
                .label(label)
                .xsmall()
                .compact()
                .toggled(on)
                .accessibility_label(what.clone())
                .tooltip(what);
            let b = match (on, f == solo) {
                (false, _) => b.outline(),
                (true, true) => b.primary(),
                (true, false) => b.danger(),
            };
            d = d.child(b.on_click(cx.listener(move |this, _, window, cx| this.set_mix_flag(f, i, !on, window, cx))));
        }
        d
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

    /// 2.1: the crossover's frequency, and the main pair's full range
    fn crossover_rows(&self, mix: &AuracastMix, cx: &mut Context<Self>) -> Div {
        let this = cx.entity().downgrade();
        let hz = crossover_choice(mix.crossover_hz);
        let freq = ui::select_labelled(
            "mix-crossover",
            t("ui.dev.auracast_mix_crossover"),
            &hz,
            CROSSOVER_CHOICES.iter().map(|hz| Choice::new(*hz, format!("{hz} Hz"))).collect(),
            false,
            move |hz: &u32, window, cx| {
                let hz = *hz as f32;
                let _ = this.update(cx, |m, cx| {
                    if let Some(mix) = m.mix().cloned() {
                        m.change(AuracastMix { crossover_hz: hz, ..mix }, window, cx);
                    }
                });
            },
        );
        let full_range = ui::toggle(
            "mix-mains-full-range",
            t("ui.dev.auracast_mix_mains_full_range"),
            Some(t("ui.dev.auracast_mix_mains_full_range_hint").into()),
            mix.mains_full_range,
            false,
            cx.listener(|this, v: &bool, window, cx| {
                if let Some(mix) = this.mix().cloned() {
                    this.change(AuracastMix { mains_full_range: *v, ..mix }, window, cx);
                }
            }),
            cx,
        );
        div()
            .flex()
            .flex_col()
            .gap(theme::S2)
            .child(
                ui::row()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(2.))
                            .child(t("ui.dev.auracast_mix_crossover"))
                            .child(ui::hint(t("ui.dev.auracast_mix_crossover_hint"), cx)),
                    )
                    .child(freq),
            )
            .child(full_range)
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
        fn full_col<E: Styled>(d: E) -> E {
            d.w(px(80.)).flex_none()
        }
        fn sm_col<E: Styled>(d: E) -> E {
            d.w(px(48.)).flex_none()
        }
        let in_heard = |c: usize| heard(&mix.solo_in, &mix.mute_in, c);
        let out_heard = |r: usize| heard(&mix.solo_out, &mix.mute_out, r);
        // The LFE's outputs: auto without a subwoofer (one takes it then)
        let show_full = mix.auto && st.sub == AuracastSub::None;
        let head_cell = |text: SharedString| gain_col(div()).text_center().text_color(p.text_2).child(text);
        let output_names = self.output_names(st.sub);
        let column_sm: Vec<Div> = INPUT_NAMES
            .iter()
            .enumerate()
            .map(|(c, name)| {
                absent(
                    st.present.get(c).copied().unwrap_or(false),
                    gain_col(self.solo_mute((MixFlag::SoloIn, MixFlag::MuteIn), c, name, cx)),
                )
            })
            .collect();
        let grid = div()
            .flex()
            .flex_col()
            .min_w(px(40. + 8. * 48. + 64. + 64. + 48. + if show_full { 84. } else { 0. } + 11. * 4.))
            .gap(px(2.))
            .text_size(theme::FS_SMALL)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(name_col(div()).truncate().text_color(p.text_2).child(t("ui.dev.auracast_mix_output")))
                    .children(INPUT_NAMES.iter().enumerate().map(|(c, name)| {
                        absent(st.present.get(c).copied().unwrap_or(false) && in_heard(c), head_cell((*name).into()))
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
                    )
                    .when(show_full, |d| {
                        d.child(
                            full_col(div())
                                .truncate()
                                .text_center()
                                .text_color(p.text_2)
                                .child(t("ui.dev.auracast_mix_full_range")),
                        )
                    })
                    .child(sm_col(div())),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(name_col(div()))
                    .children(column_sm)
                    .child(delay_col(div()))
                    .child(invert_col(div()))
                    .when(show_full, |d| d.child(full_col(div())))
                    .child(sm_col(div())),
            )
            .children(output_names.iter().enumerate().map(|(r, out)| {
                // A mono subwoofer leaves the third output unused
                let row_on = (r < 2 || st.rear) && !(r == 2 && st.sub == AuracastSub::Mono);
                let sm = self.solo_mute((MixFlag::SoloOut, MixFlag::MuteOut), r, out, cx);
                let cells = INPUT_NAMES.iter().enumerate().map(|(c, inp)| {
                    let on = self.cell(r, c) != 0.;
                    let input = Input::new(&self.input(MixField::Cell(r, c)))
                        .readonly(mix.auto)
                        .aria_label(SharedString::from(format!("{inp} → {out}")))
                        .w_full()
                        .when(on, |i| i.border_color(p.accent_fill))
                        .when(mix.auto, |i| i.bg(p.surface_2));
                    absent(
                        st.present.get(c).copied().unwrap_or(false) && in_heard(c) && out_heard(r),
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
                let lfe_here = full_range(&mix.full_range, r);
                absent(
                    row_on,
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .child(absent(out_heard(r), name_col(div()).text_center().child(*out)))
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
                        )
                        .when(show_full, |d| {
                            d.child(
                                full_col(div()).flex().justify_center().child(
                                    Switch::new(("mix-full-range", r))
                                        .small()
                                        .checked(lfe_here)
                                        .accessibility_label(SharedString::from(format!(
                                            "{} {out}",
                                            t("ui.dev.auracast_mix_full_range")
                                        )))
                                        .on_change(cx.listener(move |this, v: &bool, window, cx| {
                                            this.set_mix_flag(MixFlag::FullRange, r, *v, window, cx)
                                        })),
                                ),
                            )
                        })
                        .child(sm_col(sm)),
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
            .child(ui::hint(t("ui.dev.auracast_mix_solo_hint"), cx))
            .when(show_full, |d| d.child(ui::hint(t("ui.dev.auracast_mix_full_range_hint"), cx)))
            .when(st.sub != AuracastSub::None, |d| d.child(self.crossover_rows(mix, cx)))
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
