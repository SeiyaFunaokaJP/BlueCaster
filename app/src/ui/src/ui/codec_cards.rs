//! The codec cards of the Codecs screen: one codec with its three
//! comparison bars and whether the device supported it when it last
//! connected (green / red, issue #86), and behind "..." that codec's
//! quality, sample rate, bit depth and ABR (SBC and SBC XQ: bitpool, in
//! developer mode frame setup). The codecs offered and what they can be set
//! to come from ui_model::codecs, as far as the device listed them when it
//! last connected (issue #86: its rates, bit depths, largest bit rate and
//! bitpool range); the dialog's rules are ui_model::view::codecs.

use std::rc::Rc;

use gpui_kit::component::slider::{Slider, SliderEvent, SliderState, SliderValue};
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, BoxShadow, Context, Div, Entity, FocusHandle, FontWeight,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Pixels, Render, Role, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled as _, TextRun, Toggled, Window, div, point, px, relative,
};
use ipc::{Method, Profile};
use ui_model::codecs::{Bars, CodecCaps, CodecChoice, CodecSupport};
use ui_model::i18n::t;
use ui_model::sbc::SbcCaps;
use ui_model::t;
use ui_model::view::codecs::{
    BitpoolView, Change, Field, SettingsProps, SettingsView, apply_change, frame_change, settings_start, settings_view,
};

// The bitpool range the dialog offers, for the other screens
#[allow(unused_imports)]
pub use ui_model::view::codecs::bitpool_range;

use super::dialog::{self, Question};
use super::{ButtonKind, Choice, IconName, icon};
use crate::app;
use crate::theme::{self, ActivePalette as _};

/// The widest a bar's label gets; a longer one is cut short
const BAR_LABEL_MAX_W: f32 = 108.;
/// A bar's smallest width
const BAR_MIN_W: f32 = 48.;
/// A bar's row: the label's line
const BAR_ROW_H: f32 = 20.;
/// A codec card's smallest width whatever its labels: the codec's name beside
/// the settings button (`.codec-grid`: minmax(9.5rem, 1fr))
const CARD_FLOOR_W: f32 = 152.;

fn bar_labels() -> [String; 3] {
    [t("ui.codec.bar_quality"), t("ui.codec.bar_latency"), t("ui.codec.bar_stability")]
}

/// A codec card's smallest width in the language shown: its border and
/// padding (13 px a side), the widest bar label, the gap and the bar. Narrower,
/// the bars would reach past the card; with a width fixed for the longest
/// translation, short labels (Japanese) left the cards mostly empty.
pub fn card_min_w(window: &Window) -> f32 {
    let font = gpui_kit::font(theme::FONT);
    let widest = bar_labels()
        .into_iter()
        .map(|label| {
            let run = TextRun {
                len: label.len(),
                font: font.clone(),
                color: gpui_kit::Hsla::default(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let line = window.text_system().layout_line(SharedString::from(label), theme::FS_SMALL, &[run], None);
            f32::from(line.width)
        })
        .fold(0., f32::max);
    (26. + widest.ceil().min(BAR_LABEL_MAX_W) + 8. + BAR_MIN_W).max(CARD_FLOOR_W)
}

/// What a card shows of its codec: the call codecs and LE Audio's LC3 have
/// cards like the A2DP ones (issue #86)
#[derive(Debug, Clone, PartialEq)]
pub struct CardSpec {
    pub id: &'static str,
    /// The codec's name, translated
    pub name: SharedString,
    pub bars: Option<Bars>,
}

/// How a card is shown
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CardState {
    pub selected: bool,
    pub disabled: bool,
    /// Auto chooses (issue #86): the card only shows whether Auto picked it
    pub read_only: bool,
    /// Its tag (None: untagged)
    pub support: Option<CodecSupport>,
    /// The card can be dragged (and moved with Alt+arrow keys) into the
    /// device's order (issue #86 round 8)
    pub orderable: bool,
    /// Its place while it is being dragged (the card itself follows the
    /// pointer)
    pub dragging: bool,
    /// It has the keyboard focus
    pub focused: bool,
}

pub type Act = Rc<dyn Fn(&mut Window, &mut App)>;
pub type ReadOnlyAct = Rc<dyn Fn(bool, &mut Window, &mut App)>;
pub type MoveAct = Rc<dyn Fn(i32, &mut Window, &mut App)>;
type OnClose = Box<dyn FnOnce(&mut App)>;

/// What a card does
#[derive(Clone)]
pub struct CardActions {
    pub on_pick: Act,
    /// The card or its "..." (`true`) clicked while read-only: offers to
    /// turn Auto-select off (issue #86 round 8)
    pub on_read_only: ReadOnlyAct,
    pub on_settings: Act,
    /// Alt+arrow key: one place earlier (-1) or later (1)
    pub on_move: Option<MoveAct>,
}

/// The three comparison bars (`.cbar`): the labels in a column as wide as
/// the widest of them, the bars beside it in the rest of the card
fn bars(b: Bars, on: bool, cx: &App) -> Div {
    let p = cx.palette();
    let labels = bar_labels();
    let values = [b.quality, b.latency, b.stability];
    let column = || div().flex().flex_col().gap(theme::S2);
    div()
        .flex()
        .gap(theme::S2)
        .text_size(theme::FS_SMALL)
        .text_color(p.text_2)
        .child(
            column().flex_shrink(1.).min_w_0().max_w(px(BAR_LABEL_MAX_W)).children(
                labels
                    .iter()
                    .map(|label| div().h(px(BAR_ROW_H)).line_height(px(BAR_ROW_H)).truncate().child(label.clone())),
            ),
        )
        .child(column().flex_1().min_w(px(BAR_MIN_W)).children(labels.into_iter().zip(values).map(|(label, value)| {
            div().flex().items_center().h(px(BAR_ROW_H)).child(
                div()
                    .id(SharedString::from(format!("cbar-{label}")))
                    .role(Role::Meter)
                    .aria_label(label)
                    .aria_numeric_value(f64::from(value))
                    .aria_min_numeric_value(0.)
                    .aria_max_numeric_value(5.)
                    .w_full()
                    .h(px(4.))
                    .rounded(px(2.))
                    .bg(p.surface_3)
                    .overflow_hidden()
                    .child(div().h_full().w(relative(f32::from(value) * 0.2)).bg(if on {
                        p.accent_fill
                    } else {
                        p.text_2
                    })),
            )
        })))
}

/// The card's tag: unknown until the device's first connection, else
/// supported or not, only the mark in colour (issue #86)
fn support_tag(s: CodecSupport, cx: &App) -> Div {
    let p = cx.palette();
    let (name, color, text) = match s {
        CodecSupport::Unknown => (IconName::QuestionCircle, p.text_2, t("ui.profile.codec_support_unknown")),
        CodecSupport::Offered => (IconName::Checkmark, p.good_text, t("ui.profile.codec_supported")),
        CodecSupport::NotOffered => (IconName::Dismiss, p.bad_text, t("ui.profile.codec_not_supported")),
    };
    // As device_picker::tag, its mark first
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(theme::S1)
        .pl(px(6.))
        .pr(theme::S2)
        .min_h(px(20.))
        .border_1()
        .border_color(p.border_strong)
        .rounded_full()
        .text_size(theme::FS_SMALL)
        .text_color(p.text_2)
        .whitespace_nowrap()
        .child(div().flex().text_color(color).child(icon(name).xsmall()))
        .child(text)
}

/// The card's face (`.codec-card`): its name, tag and bars
pub fn card_face(spec: &CardSpec, state: CardState, cx: &App) -> Div {
    let p = cx.palette();
    div()
        .flex()
        .flex_col()
        .flex_1()
        .gap(theme::S2)
        .map(|d| if state.focused { d.p(px(11.)).border_2().border_color(p.focus) } else { d.p(theme::S3).border_1() })
        .when(!state.focused, |d| d.border_color(if state.selected { p.accent_fill } else { p.border }))
        .rounded(theme::RADIUS)
        .bg(if state.selected { p.surface_2 } else { p.surface })
        // Room for the settings button in the corner
        .child(div().pr(px(28.)).font_weight(FontWeight::SEMIBOLD).child(spec.name.clone()))
        .when_some(state.support, |d, s| d.child(div().flex().flex_wrap().gap(theme::S1).child(support_tag(s, cx))))
        .when_some(spec.bars, |d, b| d.child(bars(b, state.selected, cx)))
}

/// One codec choice with its three comparison bars, and "..." for its
/// settings. `id` is unique in the window; `focus` makes the card take the
/// keyboard (not while read-only or disabled); `description` says how to
/// move it (screen readers). The caller adds dragging to the card (the
/// returned element's first child is the "..." button's sibling).
pub fn codec_card(
    id: SharedString,
    spec: &CardSpec,
    state: CardState,
    actions: CardActions,
    focus: Option<&FocusHandle>,
    description: Option<SharedString>,
    cx: &App,
) -> Stateful<Div> {
    let p = *cx.palette();
    let name = spec.name.clone();
    let takes_focus = !(state.disabled || state.read_only);
    let face = card_face(spec, state, cx)
        .when(state.read_only && !state.selected, |d| d.opacity(0.75))
        .when(state.dragging, |d| d.opacity(0.35).border_dashed())
        .when(!state.disabled && !state.read_only && !state.selected && !state.focused, |d| {
            d.hover(|s| s.border_color(p.border_strong))
        });
    let a = actions.clone();
    let selected = state.selected;
    let card = div()
        .id(SharedString::from(format!("{id}-card")))
        .role(Role::RadioButton)
        .aria_label(name.clone())
        .aria_toggled(if selected { Toggled::True } else { Toggled::False })
        .when_some(description.filter(|_| state.orderable && takes_focus), |d, text| d.aria_description(text))
        .flex()
        .flex_col()
        .flex_1()
        .rounded(theme::RADIUS)
        .cursor_pointer()
        .when(takes_focus, |d| {
            let d = match focus {
                Some(f) => d.track_focus(&f.clone().tab_stop(true)),
                None => d,
            };
            d.on_key_down(move |ev: &KeyDownEvent, window, cx| {
                let k = &ev.keystroke;
                let m = k.modifiers;
                if m.alt && !m.control && !m.platform && !m.shift {
                    let by = match k.key.as_str() {
                        "left" | "up" => -1,
                        "right" | "down" => 1,
                        _ => return,
                    };
                    // Not the radio group's own arrow keys
                    if let Some(on_move) = &a.on_move {
                        cx.stop_propagation();
                        on_move(by, window, cx);
                    }
                }
                // Enter and Space on the focused card are a click (below), once
            })
        })
        .on_click({
            let a = actions.clone();
            move |_, window, cx| {
                if state.read_only {
                    (a.on_read_only)(false, window, cx);
                } else if !state.disabled && !selected {
                    (a.on_pick)(window, cx);
                }
            }
        })
        .child(face);
    // The button is beside the card, not in it: the card's name stays the
    // codec's, and pressing it starts no drag
    // A gear in a button of its own, as Windows shows an item's settings:
    // three dots read as a menu, and without an outline as no button at all
    let label = t!("ui.profile.codec_settings", name.to_string());
    let more = gpui_kit::component::button::Button::new(SharedString::from(format!("{id}-more")))
        .icon(icon(IconName::Settings))
        .accessibility_label(label.clone())
        .tooltip(label)
        .small()
        .disabled(state.disabled && !state.read_only)
        .on_click(move |_, window, cx| {
            if state.read_only {
                (actions.on_read_only)(true, window, cx);
            } else {
                (actions.on_settings)(window, cx);
            }
        });
    div()
        .id(id)
        .relative()
        .flex()
        .flex_col()
        .min_w_0()
        .child(card)
        .child(div().absolute().top(theme::S1).right(theme::S1).occlude().child(more))
}

/// What a card dragged carries (the device it belongs to, its codec), and
/// draws while it follows the pointer: the card lifted
#[derive(Debug, Clone)]
pub struct DraggedCodec {
    pub device: String,
    pub id: &'static str,
    pub spec: CardSpec,
    pub state: CardState,
    pub width: Pixels,
}

impl Render for DraggedCodec {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let state = CardState { dragging: false, ..self.state };
        div().w(self.width).flex().flex_col().child(
            card_face(&self.spec, state, cx).border_color(p.border_strong).shadow(vec![BoxShadow {
                color: gpui_kit::hsla(0., 0., 0., 0.35),
                offset: point(px(0.), px(8.)),
                blur_radius: px(24.),
                spread_radius: px(0.),
                inset: false,
            }]),
        )
    }
}

// ---- a codec's "..." dialog -------------------------------------------------------

/// What the dialog goes by, read again whenever it draws
pub struct SettingsInput {
    /// The profile shown (the device's, or what it would start from)
    pub profile: Profile,
    /// Device.codec_caps as the settings take them (empty: not known)
    pub caps: Vec<CodecCaps>,
    pub sbc_caps: Option<SbcCaps>,
    pub developer: bool,
    pub match_source: bool,
    pub source_format: Option<(u32, u32)>,
    pub device_name: String,
    pub address: String,
    /// Turning "Show only supported options" off asks first (until accepted
    /// once, for every device and codec)
    pub warn_unlisted: bool,
}

impl SettingsInput {
    pub fn props(&self) -> SettingsProps<'_> {
        SettingsProps {
            caps: &self.caps,
            sbc_caps: self.sbc_caps.as_ref(),
            developer: self.developer,
            match_source: self.match_source,
            source_format: self.source_format,
            device_name: &self.device_name,
        }
    }
}

pub type SettingsSource = Rc<dyn Fn(&App) -> Option<SettingsInput>>;
pub type SettingsChange = Rc<dyn Fn(CodecChoice, &mut App)>;

/// One codec's quality, sample rate, bit depth and ABR (what "Advanced" had
/// for the chosen codec); SBC and SBC XQ also their bitpool and, in developer
/// mode, frame setup, offering only what the device listed when it last
/// connected, with the bit rate that gives; with "Show only supported
/// options" turned off at its top, everything the codec has (the profile's
/// unlisted_options, issue #86), what the device did not list marked. A codec
/// with one quality shows it rather than a choice. It starts from the
/// profile's values fitted to the codec, or from what that codec was last set
/// to (the profile's codec_settings); each change goes to `on_change` at once
/// (issue #86: no OK or Cancel).
pub struct CodecSettings {
    v: CodecChoice,
    source: SettingsSource,
    on_change: SettingsChange,
    on_close: Option<OnClose>,
    slider: Entity<SliderState>,
    /// The slider's range and handles as last set
    slider_at: (u32, u32, u32, u32),
}

impl CodecSettings {
    fn new(start: CodecChoice, source: SettingsSource, on_change: SettingsChange, cx: &mut Context<Self>) -> Self {
        let core = app::core(cx);
        cx.observe(&core, |_, _, cx| cx.notify()).detach();
        let slider = cx.new(|_| SliderState::new().min(0.).max(1.).step(1.).default_value((0., 1.)));
        cx.subscribe(&slider, |this: &mut Self, _, ev: &SliderEvent, cx| {
            let SliderEvent::Change(SliderValue::Range(a, b)) = ev else { return };
            let (a, b) = (a.round().max(0.) as u32, b.round().max(0.) as u32);
            let (lo, hi) = (this.slider_at.2, this.slider_at.3);
            if a != lo {
                this.change(Change::LowBitpool(a), cx);
            } else if b != hi {
                this.change(Change::HighBitpool(b), cx);
            }
        })
        .detach();
        cx.on_release(|this: &mut Self, cx| {
            if let Some(f) = this.on_close.take() {
                f(cx);
            }
        })
        .detach();
        CodecSettings { v: start, source, on_change, on_close: None, slider, slider_at: (0, 0, 0, 0) }
    }

    fn change(&mut self, change: Change, cx: &mut Context<Self>) {
        let Some(input) = (self.source)(cx) else { return };
        if apply_change(&mut self.v, change, &input.props()) {
            (self.on_change)(self.v.clone(), cx);
        }
        cx.notify();
    }

    /// "Show only supported options" switched; turning it off asks first
    /// (issue #86: a device may reboot or break), once for every device
    fn toggle_listed(&mut self, on: bool, warn: bool, window: &mut Window, cx: &mut Context<Self>) {
        if on || !warn {
            self.change(Change::Unlisted(!on), cx);
            return;
        }
        let this = cx.entity().downgrade();
        dialog::ask(
            Question::new(t("ui.profile.unlisted_warn_title")).warn().confirm(t("ui.profile.unlisted_warn_accept")),
            |_, _| div().child(t("ui.profile.unlisted_warn")).into_any_element(),
            move |_, cx| {
                app::save_settings(|s| s.unlisted_options_warned = true, cx);
                let _ = this.update(cx, |this, cx| this.change(Change::Unlisted(true), cx));
            },
            window,
            cx,
        );
    }

    /// The slider follows the values (Auto, a rate that moved the range)
    fn sync_slider(&mut self, bp: &BitpoolView, window: &mut Window, cx: &mut Context<Self>) {
        // A range of one value still needs a track
        let max = bp.max.max(bp.min + 1);
        let at = (bp.min, max, bp.low, bp.high);
        if at == self.slider_at {
            return;
        }
        let range_moved = (at.0, at.1) != (self.slider_at.0, self.slider_at.1);
        self.slider_at = at;
        let value = (bp.low as f32, bp.high as f32);
        self.slider.update(cx, |s, cx| {
            if range_moved {
                *s = SliderState::new().min(bp.min as f32).max(max as f32).step(1.).default_value(value);
                cx.notify();
            } else {
                s.set_value(value, window, cx);
            }
        });
    }
}

/// A value shown where a list would be (`output.input.readonly`)
pub fn value_box(text: impl Into<SharedString>, cx: &App) -> Div {
    let p = cx.palette();
    div()
        .flex()
        .items_center()
        .min_h(gpui_kit::rems(2.))
        .min_w(px(96.))
        .px(theme::S3)
        .rounded(theme::RADIUS)
        .border_1()
        .border_color(p.border)
        .bg(p.surface_2)
        .text_color(p.text)
        .whitespace_nowrap()
        .child(text.into())
}

fn hint_of(key: &str) -> Option<SharedString> {
    Some(t(key).into())
}

/// A setting as a value or a list (named by its field's `label` for
/// assistive technology)
fn field_control<T: Clone + PartialEq + 'static>(
    id: &'static str,
    label: impl Into<SharedString>,
    f: &Field<T>,
    on_change: impl Fn(&T, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    match f {
        Field::Fixed(text) => value_box(text.clone(), cx).into_any_element(),
        Field::Choice { value, options, disabled } => {
            let choices = options.iter().map(|o| Choice::new(o.value.clone(), o.label.clone())).collect();
            super::select_labelled(id, label, value, choices, *disabled, on_change).into_any_element()
        }
    }
}

impl Render for CodecSettings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(input) = (self.source)(cx) else { return div() };
        let props = input.props();
        let view: SettingsView = settings_view(&self.v, &props);
        if let Some(bp) = &view.bitpool {
            self.sync_slider(bp, window, cx);
        }
        let warn = input.warn_unlisted;
        let address = input.address.clone();
        div()
            .flex()
            .flex_col()
            .gap(theme::S3)
            // What the device listed, or everything the codec has (issue #86:
            // the user's choice, the device may refuse it)
            .when_some(view.listed.clone(), |d, (label, on)| {
                d.child(super::toggle(
                    "codec-listed",
                    label,
                    None,
                    on,
                    false,
                    cx.listener(move |this, v: &bool, window, cx| this.toggle_listed(*v, warn, window, cx)),
                    cx,
                ))
            })
            // SBC and SBC XQ: the bitpool below decides the bit rate, shown
            // here as worked out (issue #86); one quality: shown, not a choice
            .child(super::field(
                t("connection.quality"),
                hint_of("tooltip.quality"),
                None,
                field_control(
                    "codec-quality",
                    t("connection.quality"),
                    &view.quality,
                    cx.listener(|this, q: &String, _, cx| this.change(Change::Quality(q.clone()), cx)),
                    cx,
                ),
                cx,
            ))
            // One rate the device takes: shown as a value, without Auto
            .child(super::field(
                t("connection.sample_rate"),
                hint_of("tooltip.sample_rate"),
                None,
                field_control(
                    "codec-rate",
                    t("connection.sample_rate"),
                    &view.rate,
                    cx.listener(|this, r: &u32, _, cx| this.change(Change::SampleRate(*r), cx)),
                    cx,
                ),
                cx,
            ))
            // One bit depth: shown as a value, not as a list to open (issue #86)
            .child(super::field(
                t("connection.bit_depth"),
                hint_of("tooltip.bit_depth"),
                None,
                field_control(
                    "codec-depth",
                    t("connection.bit_depth"),
                    &view.depth,
                    cx.listener(|this, b: &u32, _, cx| this.change(Change::BitDepth(*b), cx)),
                    cx,
                ),
                cx,
            ))
            .when_some(view.abr, |d, on| {
                d.child(super::toggle(
                    "codec-abr",
                    t("connection.abr"),
                    hint_of("tooltip.abr"),
                    on,
                    false,
                    cx.listener(|this, v: &bool, _, cx| this.change(Change::Abr(*v), cx)),
                    cx,
                ))
            })
            .when_some(view.vbr, |d, on| {
                d.child(super::toggle(
                    "codec-vbr",
                    t("connection.vbr"),
                    hint_of("tooltip.vbr"),
                    on,
                    false,
                    cx.listener(|this, v: &bool, _, cx| this.change(Change::Abr(*v), cx)),
                    cx,
                ))
            })
            .when_some(view.low_latency, |d, on| {
                d.child(super::toggle(
                    "codec-ll",
                    t("connection.lhdcv5_low_latency"),
                    hint_of("tooltip.lhdcv5_low_latency"),
                    on,
                    false,
                    cx.listener(|this, v: &bool, _, cx| this.change(Change::Lhdcv5LowLatency(*v), cx)),
                    cx,
                ))
            })
            .when_some(view.bitpool.clone(), |d, bp| d.child(self.bitpool_field(&bp, cx)))
            .when(!view.frame.is_empty(), |d| {
                let p = cx.palette();
                let fields = view.frame.iter().map(|f| {
                    let key = f.key;
                    let id: &'static str = match key {
                        ui_model::view::codecs::FrameKey::Blocks => "sbc-blocks",
                        ui_model::view::codecs::FrameKey::Subbands => "sbc-subbands",
                        ui_model::view::codecs::FrameKey::Allocation => "sbc-allocation",
                        ui_model::view::codecs::FrameKey::ChannelMode => "sbc-channel-mode",
                    };
                    div().flex_1().min_w(px(220.)).child(super::field_stacked(
                        f.title.clone(),
                        None,
                        None,
                        field_control(
                            id,
                            f.title.clone(),
                            &f.field,
                            cx.listener(move |this, v: &String, _, cx| this.change(frame_change(key, v), cx)),
                            cx,
                        ),
                        cx,
                    ))
                });
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(theme::S2)
                        .child(div().text_color(p.text_2).child(t("ui.profile.sbc_frame")))
                        .child(div().flex().flex_wrap().gap(theme::S3).children(fields)),
                )
            })
            // Fixed for each codec, unless chosen above (issue #86)
            .when_some(view.channel_mode.clone(), |d, mode| {
                d.child(super::field(
                    t("ui.diag.cd_channel_mode"),
                    hint_of("tooltip.channel_mode"),
                    None,
                    value_box(mode, cx),
                    cx,
                ))
            })
            // OPPO's Hi-Res request: niche, so developer mode (issue #86)
            .when(view.ask_hi_res, |d| {
                d.child(
                    div().flex().child(
                        super::button("ask-hi-res", t("ui.profile.ask_hi_res_again"), ButtonKind::Default)
                            .icon(icon(IconName::ArrowReset))
                            .on_click(move |_, _, cx| {
                                app::send(Method::DeviceForgetCodecs { address: address.clone() }, cx);
                            }),
                    ),
                )
            })
    }
}

impl CodecSettings {
    /// The SBC / SBC XQ bitpool as one slider with two handles (issue #86):
    /// the lowest bitpool (the SET_CONFIGURATION's minimum and the floor of
    /// ABR) and the highest (the bitpool the stream starts with), each 0 for
    /// Auto, with the values beside it and Auto to put both back
    fn bitpool_field(&self, bp: &BitpoolView, cx: &mut Context<Self>) -> Div {
        let p = *cx.palette();
        let num =
            |text: String| div().flex_none().min_w(px(64.)).text_color(p.text_2).text_size(theme::FS_SMALL).child(text);
        let slider = div()
            .flex()
            .flex_1()
            .min_w(px(200.))
            .items_center()
            .gap(theme::S2)
            .child(num(bp.low_text.clone()).text_right())
            // Room for the handles, which reach past the slider's ends
            .child(div().flex_1().px(theme::S2).child(
                Slider::new(&self.slider).accessibility_label(t("connection.sbc_bitpool")).disabled(bp.max <= bp.min),
            ))
            .child(num(bp.high_shown.clone()));
        let auto = super::button("bitpool-auto", t("codec.auto"), ButtonKind::Subtle)
            .disabled(bp.auto)
            .on_click(cx.listener(|this, _, _, cx| this.change(Change::BitpoolAuto, cx)));
        super::field_stacked(
            t("connection.sbc_bitpool"),
            hint_of("tooltip.sbc_bitpool"),
            bp.error.clone().map(SharedString::from),
            super::inline().child(slider).child(auto),
            cx,
        )
    }
}

/// Opens a codec's settings (`card`) in a dialog. `source` gives the
/// profile and what the device listed whenever it draws; each change goes to
/// `on_change` at once, and `on_close` runs once the dialog is gone.
pub fn open_codec_settings(
    card: &'static str,
    source: SettingsSource,
    on_change: SettingsChange,
    on_close: impl FnOnce(&mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(input) = source(cx) else { return };
    let profile = &input.profile;
    let kept = ui_model::profile::kept_for(profile, card);
    let start = settings_start(card, profile, kept, &input.props());
    let title = settings_view(&start, &input.props()).title;
    let entity = cx.new(|cx| {
        let mut s = CodecSettings::new(start, source, on_change, cx);
        s.on_close = Some(Box::new(on_close));
        s
    });
    dialog::ask(Question::new(title), move |_, _| entity.clone().into_any_element(), |_, _| {}, window, cx);
}
