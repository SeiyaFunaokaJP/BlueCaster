//! The controls the screens are made of: the component library's buttons,
//! switches, menus and dialogs in Windows 11's shapes (theme.rs), and the
//! lines and cards the screens share: [`button`], [`icon_button`],
//! [`toggle`], [`check`], [`segmented`], [`select`], [`field`], [`section`],
//! [`banner`], [`nav_row`] and [`help_tip`].

pub mod cable_notes;
pub mod capture_picker;
pub mod charts;
pub mod codec_cards;
pub mod codec_details;
pub mod device_picker;
pub mod device_settings;
pub mod device_switcher;
pub mod dialog;
pub mod icons;
pub mod import_pairings;
pub mod input_source;
pub mod le_channels;
pub mod media_eq;
pub mod mic_processing;
pub mod mixer;
pub mod paired_devices;
pub mod pairing_dialog;
pub mod stream;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::TestSupportExt as _;
use gpui_kit::accesskit::Live;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, DispatchPhase, Div, ElementId, FontWeight, InputEvent as _, InteractiveElement as _,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _, Pixels, Point,
    RenderOnce, Role, ScrollHandle, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Text,
    Toggled, Window, canvas, div, point, px,
};

pub use icons::IconName;

use crate::theme::{self, ActivePalette as _};

/// A Fluent icon (20 px grid); takes the text colour
pub fn icon(name: IconName) -> Icon {
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor"><path d="{}"/></svg>"#,
        name.path()
    );
    Icon::default().data(svg.as_bytes())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonKind {
    /// Windows' standard button
    #[default]
    Default,
    /// No fill until hovered (links to pages, less important actions)
    Subtle,
    Danger,
    /// The accent colour: the one thing to do next
    Strong,
}

/// A button with its text; add `.icon(icon(..))`, `.loading(busy)`,
/// `.disabled(..)` and `.on_click(..)` as needed
pub fn button(id: impl Into<ElementId>, label: impl Into<SharedString>, kind: ButtonKind) -> Button {
    let b = Button::new(id).label(label);
    match kind {
        ButtonKind::Default => b,
        ButtonKind::Subtle => b.ghost(),
        ButtonKind::Danger => b.danger(),
        ButtonKind::Strong => b.primary(),
    }
}

/// Icon-only button: the label is for screen readers and the tooltip
pub fn icon_button(id: impl Into<ElementId>, name: IconName, label: impl Into<SharedString>) -> Button {
    let label = label.into();
    Button::new(id).ghost().icon(icon(name)).accessibility_label(label.clone()).tooltip(label)
}

/// One choice of a [`select`] or [`segmented`]
#[derive(Debug, Clone, PartialEq)]
pub struct Choice<T> {
    pub value: T,
    pub label: SharedString,
    pub icon: Option<IconName>,
    pub disabled: bool,
    /// A select's choices with the same group (next to each other) go under
    /// one heading
    pub group: Option<SharedString>,
}

impl<T> Choice<T> {
    pub fn new(value: T, label: impl Into<SharedString>) -> Choice<T> {
        Choice { value, label: label.into(), icon: None, disabled: false, group: None }
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn group(mut self, group: impl Into<SharedString>) -> Self {
        self.group = Some(group.into());
        self
    }
}

/// A drop-down list: the chosen value on a button, the choices in a menu
/// with a check mark on the chosen one
pub fn select<T: Clone + PartialEq + 'static>(
    id: impl Into<ElementId>,
    value: &T,
    choices: Vec<Choice<T>>,
    disabled: bool,
    on_change: impl Fn(&T, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    select_named(id.into(), None, value, choices, disabled, on_change)
}

/// A [`select`] with a name for screen readers,
/// for a list without a visible label of its own beside it: it is announced
/// as "<label>: <the chosen value>"
pub fn select_labelled<T: Clone + PartialEq + 'static>(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    value: &T,
    choices: Vec<Choice<T>>,
    disabled: bool,
    on_change: impl Fn(&T, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    select_named(id.into(), Some(label.into()), value, choices, disabled, on_change)
}

fn select_named<T: Clone + PartialEq + 'static>(
    id: ElementId,
    label: Option<SharedString>,
    value: &T,
    choices: Vec<Choice<T>>,
    disabled: bool,
    on_change: impl Fn(&T, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let current = choices.iter().find(|c| c.value == *value).map(|c| c.label.clone()).unwrap_or_default();
    let name = label.map(|l| if current.is_empty() { l } else { format!("{l}: {current}").into() });
    let value = value.clone();
    let on_change = Rc::new(on_change);
    // The menu opens on the press itself; the click handler does nothing but
    // tell assistive technology that the list can be pressed
    let button = Button::new(id).label(current).dropdown_caret(true).disabled(disabled).on_click(|_, _, _| {});
    let button = match name {
        Some(name) => button.accessibility_label(name),
        None => button,
    };
    button.dropdown_menu(move |menu, window, _| {
        // Not a narrow strip under a wide button. A list taller than the
        // window scrolls (the menu scrolls by itself only past 20 entries):
        // the flyout is short, and its device list long
        let tall = window.viewport_size().height * 0.7;
        let mut menu = menu.min_w(px(160.)).scrollable(true).max_h(tall.min(px(450.)));
        // Nor wider than a long name (an audio device's) leaves readable
        menu = menu.max_w(px(480.));
        let mut group: Option<SharedString> = None;
        for c in &choices {
            if c.group != group {
                group = c.group.clone();
                menu = menu.separator();
                if let Some(g) = &group {
                    menu = menu.label(g.clone());
                }
            }
            let (v, on_change) = (c.value.clone(), on_change.clone());
            let chosen = c.value == value;
            let mut item = menu_choice(c.label.clone(), chosen, c.disabled);
            if let Some(i) = c.icon {
                item = item.icon(icon(i));
            }
            // The value it has chosen again changes nothing (as a list in a
            // browser): the flyout's device list would start its stream again
            menu = menu.item(item.on_click(move |_: &ClickEvent, window, cx| {
                if !chosen {
                    on_change(&v, window, cx);
                }
            }));
        }
        menu
    })
}

// The menu reports each of its entries as highlighted or not, and for
// assistive technology that leaves an entry nothing to press. The entries
// below carry their text in a node of their own that can be chosen or
// pressed; the press lands on the entry, which runs its handler and closes
// the menu, and the menu draws the check mark.

/// A menu entry that is one of several options, `chosen` with a check mark;
/// add `.icon(..)` and `.on_click(..)`
pub fn menu_choice(label: SharedString, chosen: bool, disabled: bool) -> PopupMenuItem {
    PopupMenuItem::element(move |_, _| {
        div()
            .id("choice")
            .test_support()
            .role(Role::MenuItemRadio)
            .aria_label(label.clone())
            .aria_toggled(if chosen { Toggled::True } else { Toggled::False })
            .when(disabled, |d| d.a11y_synthetic_children(|b| b.parent_node().set_disabled()))
            .flex_1()
            // The entry does not shrink with its menu: a width of its own
            .max_w(px(420.))
            .truncate()
            .child(label.clone())
    })
    .checked(chosen)
    .disabled(disabled)
}

/// A menu entry that does something; add `.on_click(..)`
pub fn menu_command(label: SharedString, disabled: bool) -> PopupMenuItem {
    PopupMenuItem::element(move |_, _| {
        div()
            .id("command")
            .test_support()
            .role(Role::MenuItem)
            .aria_label(label.clone())
            .when(disabled, |d| d.a11y_synthetic_children(|b| b.parent_node().set_disabled()))
            // Tells assistive technology that the entry can be pressed: the
            // press itself goes on to the entry
            .on_click(|_, _, _| {})
            .flex_1()
            .child(label.clone())
    })
    .disabled(disabled)
}

#[cfg(test)]
mod page_tests;
#[cfg(test)]
mod select_tests;

/// A few choices side by side, one chosen
pub fn segmented<T: Clone + PartialEq + 'static>(
    id: impl Into<ElementId>,
    value: &T,
    choices: Vec<Choice<T>>,
    on_change: impl Fn(&T, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let selected = choices.iter().position(|c| c.value == *value).unwrap_or(0);
    let values: Vec<T> = choices.iter().map(|c| c.value.clone()).collect();
    TabBar::new(id)
        .segmented()
        .selected_index(selected)
        .children(choices.into_iter().map(|c| {
            let mut tab = Tab::new().label(c.label).disabled(c.disabled);
            if let Some(i) = c.icon {
                tab = tab.prefix(icon(i).small());
            }
            tab
        }))
        .on_click(move |index: &usize, window, cx| {
            if let Some(v) = values.get(*index) {
                on_change(v, window, cx);
            }
        })
}

/// A page's tabs, each with its icon before its name (Diagnostics, Developer)
pub fn tab_bar<T: Clone + PartialEq + 'static>(
    id: impl Into<ElementId>,
    current: &T,
    tabs: Vec<Choice<T>>,
    on_select: impl Fn(&T, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let selected = tabs.iter().position(|c| c.value == *current).unwrap_or(0);
    let values: Vec<T> = tabs.iter().map(|c| c.value.clone()).collect();
    TabBar::new(id)
        .underline()
        // Many tabs (the developer screen's): those that do not fit a narrow
        // window are in a menu at the end, not cut off
        .menu(tabs.len() > 6)
        .selected_index(selected)
        .children(tabs.into_iter().map(|c| {
            let tab = Tab::new().label(c.label);
            // The prefix sits outside the tab's padding: the icon gets its own
            match c.icon {
                Some(i) => tab.prefix(div().flex().items_center().pl(theme::S2).child(icon(i))),
                None => tab,
            }
        }))
        .on_click(move |index: &usize, window, cx| {
            if let Some(v) = values.get(*index) {
                on_select(v, window, cx);
            }
        })
}

/// Text that assistive technology reads (plain text is only drawn)
pub fn text(s: impl Into<SharedString>) -> Text {
    let s = s.into();
    Text::new(child_id("text", &s), s)
}

thread_local! {
    /// How often each kind and words were given an id in the frame being
    /// drawn (see [`frame_drawn`])
    static IDS_GIVEN: RefCell<HashMap<(&'static str, SharedString), u64>> = RefCell::new(HashMap::new());
}

/// An id of an element known by its words: the same words again in one
/// frame get the next number, as assistive technology needs every element's
/// id to be its own; drawn in the same order, they get the same ids each
/// frame
fn child_id(kind: &'static str, words: &SharedString) -> ElementId {
    let n = IDS_GIVEN.with(|ids| {
        let mut ids = ids.borrow_mut();
        let n = ids.entry((kind, words.clone())).or_insert(0);
        *n += 1;
        *n - 1
    });
    ElementId::NamedChild(Arc::new(ElementId::NamedInteger(kind.into(), n)), words.clone())
}

/// Put once in a window's root: when the window's frame is painted (every
/// element made), the ids of [`child_id`] start again for the next frame
pub fn frame_drawn() -> impl IntoElement {
    canvas(|_, _, _| {}, |_, _, _, _| IDS_GIVEN.with(|ids| ids.borrow_mut().clear())).absolute().size_0()
}

/// Secondary text under a label or in a card (`.hint`)
pub fn hint(s: impl Into<SharedString>, cx: &App) -> Div {
    div().text_size(theme::FS_SMALL).text_color(cx.palette().text_2).child(text(s))
}

/// Dimmed text (`.muted`)
pub fn muted(s: impl Into<SharedString>, cx: &App) -> Div {
    div().text_color(cx.palette().text_2).child(text(s))
}

/// Text in the monospace font (addresses, versions, log lines)
pub fn mono(text: impl Into<SharedString>) -> Div {
    div().font_family(theme::FONT_MONO).child(text.into())
}

/// A line of controls side by side that wraps (`.inline.wrap`)
pub fn inline() -> Div {
    div().flex().flex_wrap().items_center().gap(theme::S2)
}

/// A line of a card: its text on the left, the control on the right (`.row`)
pub fn row() -> Div {
    div().flex().items_center().justify_between().gap(theme::S4).min_h(px(40.))
}

/// A setting's line in a card: a [`row`] with a hairline above it from edge
/// to edge of the card, as the lines of Windows' Settings
pub fn setting_row(cx: &App) -> Div {
    row().mx(-theme::S4).px(theme::S4).pt(theme::S3).min_h(px(52.)).border_t_1().border_color(cx.palette().border)
}

/// A setting's line that is the first thing in its card (a card without a
/// heading): without the hairline, which would double the card's edge
pub fn first_row(line: Div) -> Div {
    line.border_t_0().pt_0().min_h(px(40.))
}

/// A line's label and hint; `read`: the label is read out too (a switch
/// carries its label itself, other controls may not)
fn row_text(label: SharedString, hint_text: Option<SharedString>, read: bool, cx: &App) -> Div {
    let label = if read { div().child(text(label)) } else { div().child(label) };
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(px(2.))
        .child(label)
        .when_some(hint_text, |d, h| d.child(hint(h, cx)))
}

/// A switch with its label and hint on the left (a setting's line). The
/// switch shows `checked`: the owner writes the new value and draws again.
pub fn toggle(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    hint_text: Option<SharedString>,
    checked: bool,
    disabled: bool,
    on_change: impl Fn(&bool, &mut Window, &mut App) + 'static,
    cx: &App,
) -> Div {
    let label = label.into();
    setting_row(cx)
        .child(row_text(label.clone(), hint_text, false, cx))
        .child(Switch::new(id).checked(checked).disabled(disabled).accessibility_label(label).on_change(on_change))
}

/// A small switch with its label after it, for options inside a line or a
/// dialog (every on/off option is a switch, issue #86)
pub fn check(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    checked: bool,
    disabled: bool,
    on_change: impl Fn(&bool, &mut Window, &mut App) + 'static,
) -> Switch {
    Switch::new(id).small().checked(checked).disabled(disabled).label(label.into()).on_change(on_change)
}

/// A labelled control with an optional hint and error: the label and hint
/// on the left and the control on the right, as a [`toggle`] (issue #86)
pub fn field(
    label: impl Into<SharedString>,
    hint_text: Option<SharedString>,
    error: Option<SharedString>,
    control: impl IntoElement,
    cx: &App,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(theme::S1)
        .child(setting_row(cx).child(row_text(label.into(), hint_text, true, cx)).child(control))
        .when_some(error, |d, e| d.child(field_error(e, cx)))
}

/// As [`field`] with the control below the label, for one that needs the width
pub fn field_stacked(
    label: impl Into<SharedString>,
    hint_text: Option<SharedString>,
    error: Option<SharedString>,
    control: impl IntoElement,
    cx: &App,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(theme::S2)
        .child(div().child(text(label)))
        .child(control)
        .when_some(hint_text, |d, h| d.child(hint(h, cx)))
        .when_some(error, |d, e| d.child(field_error(e, cx)))
}

/// A field's error, told at once by a screen reader as it appears
fn field_error(message: SharedString, cx: &App) -> Stateful<Div> {
    alert(child_id("field-error", &message), message.clone(), true)
        .flex()
        .items_center()
        .gap(theme::S1)
        .text_size(theme::FS_SMALL)
        .text_color(cx.palette().bad_text)
        .child(icon(IconName::ErrorCircle).small())
        .child(message)
}

/// A region a screen reader reads out when it appears or its words change:
/// an error (`urgent`: at once) or a status, named by its words
pub fn alert(id: impl Into<ElementId>, words: impl Into<SharedString>, urgent: bool) -> Stateful<Div> {
    let live = if urgent { Live::Assertive } else { Live::Polite };
    div()
        .id(id)
        .role(if urgent { Role::Alert } else { Role::Status })
        .aria_label(words.into())
        .a11y_synthetic_children(move |b| b.parent_node().set_live(live))
}

/// A line that opens more settings: its name, its state, an arrow. The
/// whole line is the button.
pub fn nav_row(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    disabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> impl IntoElement {
    let p = *cx.palette();
    let (label, value): (SharedString, SharedString) = (label.into(), value.into());
    let line = row()
        .id(id)
        // A button for assistive technology, named "<label>: <value>"
        .role(gpui_kit::Role::Button)
        .aria_label(SharedString::from(format!("{label}: {value}")))
        .flex_1()
        .px(theme::S2)
        .rounded(theme::RADIUS)
        .when(!disabled, |d| d.cursor_pointer().hover(|s| s.bg(p.surface_2)).on_click(on_click))
        .when(disabled, |d| d.text_color(p.disabled_text))
        .child(div().child(label))
        .child(
            div()
                .flex()
                .items_center()
                .gap(theme::S2)
                .text_color(if disabled { p.disabled_text } else { p.text_2 })
                .child(value)
                .child(icon(IconName::ChevronRight).small()),
        );
    div().flex().mx(-theme::S4).px(theme::S2).pt(theme::S3).border_t_1().border_color(p.border).child(line)
}

/// A card (`.card`): children one below the other
pub fn card(cx: &App) -> Div {
    let p = cx.palette();
    div()
        .flex()
        .flex_col()
        .gap(theme::S3)
        .p(theme::S4)
        .bg(p.surface)
        .border_1()
        .border_color(p.border)
        .rounded(theme::RADIUS_CARD)
}

/// A card section with a heading, and its actions at the heading's right
pub fn section(title: impl Into<SharedString>, actions: Option<AnyElement>, cx: &App) -> Div {
    card(cx).child(
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(theme::S3)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(theme::FS_LARGE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title.into()),
            )
            .when_some(actions, |d, a| d.child(a)),
    )
}

/// A screen: its heading, then its cards, scrolling as one
pub fn page(id: impl Into<ElementId>, title: impl Into<SharedString>) -> Page {
    Page { id: id.into(), title: title.into(), children: Vec::new() }
}

/// What [`page`] makes; add the cards with `.child(..)`
#[derive(IntoElement)]
pub struct Page {
    id: ElementId,
    title: SharedString,
    children: Vec<AnyElement>,
}

impl gpui_kit::ParentElement for Page {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Page {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        // The page keeps where it was scrolled to while it is shown
        let scroll = window.use_keyed_state(self.id.clone(), cx, |_, _| ScrollHandle::new()).read(cx).clone();
        div()
            .relative()
            .size_full()
            .child(
                div()
                    .id(self.id)
                    .size_full()
                    .flex()
                    .flex_col()
                    .gap(theme::S4)
                    .p(theme::S6)
                    .overflow_y_scroll()
                    .track_scroll(&scroll)
                    .child(reveal_presses(&scroll))
                    .child(
                        div()
                            .font_family(theme::FONT_DISPLAY)
                            .text_size(theme::FS_PAGE)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(text(self.title)),
                    )
                    .children(self.children),
            )
            .vertical_scrollbar(&scroll)
    }
}

thread_local! {
    /// Where the pointer last moved to, in its window's coordinates
    static POINTER_AT: Cell<Option<Point<Pixels>>> = const { Cell::new(None) };
}

/// Lets a press assistive technology makes on a control scrolled out of
/// view reach it. Such a press arrives as a click at the control's centre,
/// outside the window: the scrolling element (`scroll`'s, which this is put
/// in) scrolls the control into view and the press goes to it there.
///
/// The pointer can be outside the page too: on Windows a press on the title
/// bar, its buttons or the window's edge comes as a press above or below
/// the page. That one is the pointer's own, at the place it last moved to,
/// and is left alone, or the window would not maximize or resize.
pub fn reveal_presses(scroll: &ScrollHandle) -> impl IntoElement {
    let scroll = scroll.clone();
    canvas(
        |_, _, _| {},
        move |_, _, window, _| {
            window.on_mouse_event(|e: &MouseMoveEvent, phase, _, _| {
                if phase == DispatchPhase::Capture {
                    POINTER_AT.set(Some(e.position));
                }
            });
            let swallow_up = Rc::new(Cell::new(false));
            let (down_scroll, up_swallow) = (scroll.clone(), swallow_up.clone());
            window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture || e.button != MouseButton::Left {
                    return;
                }
                if POINTER_AT.get() == Some(e.position) {
                    return;
                }
                let Some(at) = revealed_at(&down_scroll, e.position, window) else { return };
                cx.stop_propagation();
                swallow_up.set(true);
                window.refresh();
                // Pressed once the frame with the new scroll position is drawn:
                // what runs on the next frame runs before that frame is drawn
                window.on_next_frame(move |window, _| {
                    window.on_next_frame(move |window, cx| press_at_now(window, at, cx));
                });
            });
            window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                if phase == DispatchPhase::Capture && up_swallow.replace(false) {
                    cx.stop_propagation();
                }
            });
        },
    )
    .absolute()
    .size_0()
}

/// A left click at `at`, as the platform delivers one
fn press_at_now(window: &mut Window, at: Point<Pixels>, cx: &mut App) {
    let modifiers = window.modifiers();
    let down =
        MouseDownEvent { button: MouseButton::Left, position: at, modifiers, click_count: 1, first_mouse: false };
    window.dispatch_event(down.to_platform_input(), cx);
    let up = MouseUpEvent { button: MouseButton::Left, position: at, modifiers, click_count: 1 };
    window.dispatch_event(up.to_platform_input(), cx);
}

/// For a press at `at` outside the window, over the element `scroll`
/// scrolls: scrolls it so that the place comes into view and says where it
/// is then (None: not such a press, or the place cannot be shown)
fn revealed_at(scroll: &ScrollHandle, at: Point<Pixels>, window: &Window) -> Option<Point<Pixels>> {
    let height = window.viewport_size().height;
    let view = scroll.bounds();
    let outside_window = at.y < px(0.) || at.y > height;
    if !outside_window || at.x < view.left() || at.x > view.right() {
        return None;
    }
    // A little room from the edge, as when the user scrolls to it
    let margin = px(48.).min(view.size.height / 2.);
    let to = if at.y > view.bottom() { view.bottom() - margin } else { view.top() + margin };
    let (offset, max) = (scroll.offset(), scroll.max_offset());
    let y = (offset.y + (to - at.y)).clamp(-max.y, px(0.));
    scroll.set_offset(point(offset.x, y));
    let shown = point(at.x, at.y + (y - offset.y));
    (shown.y >= view.top() && shown.y <= view.bottom() && shown.y <= height && shown.y >= px(0.)).then_some(shown)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BannerKind {
    Info,
    Warn,
    Error,
    Good,
}

/// What a click runs (a banner's close button)
pub type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// A message in a card of its own: an icon, a title, text below it, then
/// buttons; `on_dismiss` adds a close button
pub fn banner(
    kind: BannerKind,
    title: Option<SharedString>,
    body: Option<AnyElement>,
    actions: Option<AnyElement>,
    on_dismiss: Option<(ElementId, ClickHandler)>,
    cx: &App,
) -> Stateful<Div> {
    let p = cx.palette();
    let (name, line, text) = match kind {
        BannerKind::Info => (IconName::Info, p.border_strong, p.text_2),
        BannerKind::Warn => (IconName::Warning, p.warn_line, p.warn_text),
        BannerKind::Error => (IconName::ErrorCircle, p.bad_line, p.bad_text),
        BannerKind::Good => (IconName::CheckmarkCircle, p.good_line, p.good_text),
    };
    // An error is told at once, the others as a status when they appear; the
    // title names it (a banner without one is read by its text)
    let words = title.clone().unwrap_or_default();
    alert(child_id("banner", &words), words, kind == BannerKind::Error)
        .flex()
        .items_start()
        .gap(theme::S3)
        .p(theme::S3)
        .bg(p.surface)
        .border_1()
        .border_color(line)
        .rounded(theme::RADIUS_CARD)
        .child(div().text_color(text).pt(px(2.)).child(icon(name)))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(theme::S2)
                .when_some(title, |d, t| d.child(div().font_weight(FontWeight::SEMIBOLD).child(t)))
                .when_some(body, |d, b| d.child(b))
                .when_some(actions, |d, a| d.child(a)),
        )
        .when_some(on_dismiss, |d, (id, f)| {
            d.child(icon_button(id, IconName::Dismiss, ui_model::i18n::t("modal.close")).on_click(f))
        })
}

/// What changes when Windows' audio devices change (a cable installed by
/// its own setup, a device plugged in or out), or a profile's source or
/// microphone output does (the core gave it a cable that came: the list read
/// before does not have it, issue #86). A view keeps the last one and reads
/// the devices again when it differs (not at first).
pub fn audio_devices_key(snap: Option<&ipc::Snapshot>) -> String {
    let Some(snap) = snap else { return String::new() };
    let mut key = snap.audio_endpoints.to_string();
    for p in &snap.profiles {
        key.push('\n');
        key.push_str(&p.audio_device_id);
        key.push('|');
        key.push_str(&p.mic_device_id);
    }
    key
}
