//! What the main window shows above or beside every screen: each device
//! and its state in the navigation, a newer release (the update check's
//! result, not only on Settings), a crash of BlueCaster itself and a running
//! HCI capture (v1 put " - [HCI capture running]" in the title bar).

use std::sync::Mutex;

use gpui_kit::component::Sizable as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AsyncApp, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use ipc::{Method, StreamState, UpdateInfo};
use ui_model::i18n::t;
use ui_model::profile::{AudioEntry, audio_entries};
use ui_model::t;
use ui_model::view::hid::hid_on;
use ui_model::view::stream::{self as model, capture_running_text, crash_head, note_lines};

use crate::app::{self, RouteName};
use crate::host::{self, UpdatePhase};
use crate::theme::{self, ActivePalette as _};
use crate::ui::stream::{self as stream, CodecBadge, dot, state_colors, tip};
use crate::ui::{self, BannerKind, ButtonKind, IconName, icon};

/// A small red sign while an HCI capture runs; it leads to Diagnostics
pub fn capture_sign(collapsed: bool, cx: &App) -> impl IntoElement + use<> {
    capture_sign_to(collapsed, |cx| app::navigate(RouteName::Diagnostics, cx), cx)
}

/// The capture sign leading elsewhere (the flyout opens the main window)
pub fn capture_sign_to<F: Fn(&mut App) + 'static>(collapsed: bool, on_open: F, cx: &App) -> impl IntoElement + use<F> {
    let p = *cx.palette();
    let core = app::core(cx);
    let capture = core.read(cx).snap().and_then(|s| s.capture.clone());
    div().when_some(capture, |d, c| {
        let text = capture_running_text();
        d.child(
            Button::new("capture-sign")
                .ghost()
                .small()
                .w_full()
                .when(!collapsed, |b| b.justify_start())
                .border_1()
                .border_color(p.bad_line)
                .text_color(p.bad_text)
                .text_size(theme::FS_SMALL)
                .accessibility_label(text.clone())
                .tooltip(format!("{text}\n{}", c.path))
                .child(dot(p.bad))
                .when(!collapsed, |b| b.child(div().min_w_0().truncate().child(text)))
                .on_click(move |_, _, cx| on_open(cx)),
        )
    })
}

/// Always visible (design principle 2): each device and its state, and the
/// codec and what it runs with for the one streaming, so other screens need
/// no trip to Home (issue #86)
pub fn status_pills(collapsed: bool, cx: &App) -> impl IntoElement + use<> {
    let core = app::core(cx);
    let entries = core.read(cx).snap().map(audio_entries).unwrap_or_default();
    let pills: Vec<AnyElement> = if entries.is_empty() {
        vec![device_pill(None, 0, collapsed, cx)]
    } else {
        entries.iter().enumerate().map(|(i, e)| device_pill(Some(e), i, collapsed, cx)).collect()
    };
    div().flex().flex_col().flex_none().gap(theme::S2).children(pills)
}

/// One device's block (issue #86): the device and its state; battery and
/// codec for the one streaming, and its microphone's button and line as on
/// Home's card. Without devices: the state alone. The whole block opens
/// Home (its dot is the button), the microphone's button apart.
fn device_pill(entry: Option<&AudioEntry>, index: usize, collapsed: bool, cx: &App) -> AnyElement {
    let p = *cx.palette();
    let core = app::core(cx);
    let Some(snap) = core.read(cx).snap() else { return div().into_any_element() };
    let stream = &snap.stream;
    let state = model::pill_state(snap, entry);
    let live = state != StreamState::Idle;
    let label = model::pill_label(snap, entry);
    let (st, st_text) = state_colors(state, &p);
    // No red dot beside the device's name for an error: it read as recording
    // (issue #86); the red "Error" says it. Collapsed to the dot, it stays.
    let dot_color = if state == StreamState::Error && !collapsed { gpui_kit::transparent_black() } else { st };
    let open = Button::new(("status-pill-open", index))
        .ghost()
        .xsmall()
        .w(px(16.))
        .h(px(16.))
        .accessibility_label(label.clone())
        .child(dot(dot_color))
        .on_click(|_, _, cx| app::navigate(RouteName::Home, cx));
    let block = div()
        .id(("status-pill", index))
        .flex()
        .gap(theme::S2)
        .border_1()
        .border_color(p.border)
        .rounded(theme::RADIUS)
        .bg(p.surface)
        .text_color(p.text)
        .cursor_pointer()
        .hover(|s| s.bg(p.surface_2))
        .tooltip(tip(label))
        .on_click(|_, _, cx| app::navigate(RouteName::Home, cx));
    if collapsed {
        return block.min_h(px(36.)).items_center().justify_center().child(open).into_any_element();
    }
    let device = entry.map(|e| model::entry_name(e).to_owned()).filter(|d| !d.is_empty());
    let battery = (live && stream::shows_battery(stream)).then(|| {
        div()
            .flex()
            .items_center()
            .gap(theme::S1)
            .child(stream::battery_level(("pill-battery", index), stream.battery, true, cx))
            .child(div().text_color(p.text_2).child("|"))
    });
    let info = live && stream.info.is_some();
    let text = div()
        .flex()
        .flex_col()
        .flex_1()
        .gap(px(2.))
        .min_w_0()
        .text_size(theme::FS_SMALL)
        // As Home's card: the device, battery and state, the codec
        .when_some(device, |d, name| {
            // Two lines at most, cut with an ellipsis inside the block
            d.child(
                div()
                    .w_full()
                    .min_w_0()
                    .text_size(theme::FS_BODY)
                    .font_weight(FontWeight::SEMIBOLD)
                    .line_clamp(2)
                    .text_ellipsis()
                    .child(name),
            )
        })
        .child(
            div()
                .flex()
                // A long state goes under the battery, not out of the block
                .flex_wrap()
                .items_center()
                .gap_x(theme::S1)
                .min_w_0()
                .children(battery)
                .child(div().min_w_0().max_w_full().text_color(st_text).child(model::pill_state_text(snap, state))),
        )
        // The codec, then its details below it, as on Home and in the tray (issue #86)
        .when(info, |d| {
            d.children(stream::codec_badge(CodecBadge { pill: true, ..CodecBadge::default() }, cx))
                .children(entry.and_then(|e| stream::mic_row_under_dot(e, cx)))
        });
    block.items_start().py(theme::S2).px(theme::S3).child(div().pt(px(1.)).child(open)).child(text).into_any_element()
}

/// The head of a crash's report, for the report and the copy
fn crash_notice(cx: &App) -> Option<AnyElement> {
    let p = *cx.palette();
    let core = app::core(cx);
    let crashes = &core.read(cx).snap()?.crashes;
    let c = crashes.first()?.clone();
    let more = crashes.len().saturating_sub(1);
    let mut when = t!("ui.crash.when", &c.process, &c.time);
    if more > 0 {
        when = format!("{when} {}", t!("ui.crash.more", more));
    }
    let body = div().flex().flex_col().gap(theme::S1).child(div().child(when)).when(!c.reason.is_empty(), |d| {
        d.child(
            div()
                .id("crash-reason")
                .min_w_0()
                .truncate()
                .text_color(p.text_2)
                .tooltip(tip(c.reason.clone()))
                .child(c.reason.clone()),
        )
    });
    let (head, text) = (crash_head(&c), c.text.clone());
    let (head2, text2) = (head.clone(), text.clone());
    let actions = ui::inline()
        .child(
            ui::button("crash-report", t("ui.diag.report_github"), ButtonKind::Strong)
                .icon(icon(IconName::Open))
                .on_click(move |_, _, cx| stream::open_report(head.clone(), text.clone(), cx)),
        )
        .child(
            ui::button("crash-copy", t("ui.error.copy"), ButtonKind::Default)
                .icon(icon(IconName::Copy))
                .on_click(move |_, _, cx| stream::copy_report(head2.clone(), text2.clone(), cx)),
        );
    let dismiss: ui::ClickHandler = Box::new(|_, _, cx| {
        drop(app::request::<serde_json::Value>(Method::DiagCrashesSeen, cx));
    });
    Some(
        ui::banner(
            BannerKind::Error,
            Some(t("ui.crash.title").into()),
            Some(body.into_any_element()),
            Some(actions.into_any_element()),
            Some(("crash-dismiss".into(), dismiss)),
            cx,
        )
        .into_any_element(),
    )
}

/// The version whose update dialog opened in this run: it does not open
/// again while it is open or after it was answered. The answer (Later,
/// Update now, or closed) is kept across starts in crate::ui_state.
static UPDATE_SHOWN: Mutex<Option<String>> = Mutex::new(None);

/// A newer release (issue #86): a dialog with its short notes and Update
/// now (Open download page for a portable copy), once per version; Later
/// leaves it to Settings, which has the details
fn update_notice(cx: &App) {
    let core = app::core(cx);
    let state = app::state(cx);
    let Some(snap) = core.read(cx).snap() else { return };
    let Some(u) = snap.update.available.clone() else { return };
    let (route, setup) = (state.read(cx).route.name, state.read(cx).setup_active);
    let answered = crate::ui_state::update_later().as_deref() == Some(u.version.as_str());
    if route == RouteName::Settings || route == RouteName::Setup || setup {
        // Settings has the update itself: an open dialog goes away, unanswered,
        // and comes back when the user leaves (closing it in code answers nothing)
        let mut shown = UPDATE_SHOWN.lock().unwrap_or_else(|e| e.into_inner());
        if !answered && shown.as_deref() == Some(u.version.as_str()) {
            *shown = None;
            on_main_window(cx, |window, cx| window.close_dialog(cx));
        }
        return;
    }
    if answered {
        return;
    }
    {
        let mut shown = UPDATE_SHOWN.lock().unwrap_or_else(|e| e.into_inner());
        if shown.as_deref() == Some(u.version.as_str()) {
            return;
        }
        *shown = Some(u.version.clone());
    }
    let current = snap.update.current_version.clone();
    on_main_window(cx, move |window, cx| open_update_dialog(u.clone(), current.clone(), window, cx));
}

/// Runs `f` on the main window once the frame being drawn is done (every
/// root is the component library's Root)
fn on_main_window(cx: &App, f: impl Fn(&mut Window, &mut App) + 'static) {
    cx.spawn(async move |cx: &mut AsyncApp| {
        cx.update(|cx| {
            let main = crate::windows::main_hwnd(cx);
            for w in cx.windows() {
                let done = w.update(cx, |_, window, cx| {
                    let ours = main != 0 && crate::windows::hwnd(window) == main;
                    if ours {
                        f(window, cx);
                    }
                    ours
                });
                if done.unwrap_or(false) {
                    break;
                }
            }
        });
    })
    .detach();
}

fn open_update_dialog(u: UpdateInfo, current: String, window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, move |dialog, window, cx| {
        let p = *cx.palette();
        // Whether this copy updates itself
        let phase = host::state(cx).read(cx).update.clone();
        let can_install = matches!(phase, UpdatePhase::Idle | UpdatePhase::Failed { .. });
        let (manifest, url, url2) = (u.manifest_url.clone(), u.url.clone(), u.url.clone());
        // Closed in any way, it was answered: it shows again only for a newer release
        let (v1, v2, v3) = (u.version.clone(), u.version.clone(), u.version.clone());
        let footer = DialogFooter::new()
            .child(Button::new("dialog-cancel").label(t("ui.update.later")).on_click(move |_, window, cx| {
                crate::ui_state::set_update_later(&v1);
                window.close_dialog(cx);
            }))
            .child(
                ui::button(
                    "dialog-confirm",
                    t(if can_install { "ui.update.install" } else { "ui.settings.open_download" }),
                    ButtonKind::Strong,
                )
                .on_click(move |_, window, cx| {
                    crate::ui_state::set_update_later(&v2);
                    window.close_dialog(cx);
                    if can_install {
                        // Settings shows the download and the install
                        app::navigate(RouteName::Settings, cx);
                        host::update_install(&manifest, cx);
                    } else {
                        host::open_url(&url, cx);
                    }
                }),
            );
        let notes = (!u.notes.trim().is_empty()).then(|| {
            div().flex().flex_col().gap(theme::S1).children(note_lines(&u.notes).into_iter().map(|l| {
                div()
                    .flex()
                    .gap(theme::S2)
                    .when(l.bullet, |d| d.child(div().flex_none().text_color(p.text_2).child("•")))
                    .child(div().flex_1().min_w_0().child(l.text))
            }))
        });
        let body = div()
            .flex()
            .flex_col()
            .gap(theme::S3)
            .child(div().child(ui::text(t!("ui.settings.update_available", &u.version, &current))))
            .children(notes)
            .child(
                div().flex().child(
                    Button::new("release-notes")
                        .link()
                        .label(t("ui.update.release_notes"))
                        .on_click(move |_, _, cx| host::open_url(&url2, cx)),
                ),
            );
        dialog
            .width(ui::dialog::width(window, cx))
            .title(ui::text(t("update.available_title")))
            .child(body)
            .footer(footer)
            .on_close(move |_, _, _| crate::ui_state::set_update_later(&v3))
    });
}

/// The notices above every screen: input devices, a crash, an update
pub fn app_notices(cx: &App) -> impl IntoElement + use<> {
    let core = app::core(cx);
    let hid = core.read(cx).snap().is_some_and(|s| hid_on(Some(&s.settings)));
    let hid = hid.then(|| crate::screens::hid_host::hid_notice(cx).into_any_element());
    let notices: Vec<AnyElement> = [hid, crash_notice(cx)].into_iter().flatten().collect();
    update_notice(cx);
    div().when(!notices.is_empty(), |d| {
        d.flex()
            .flex_col()
            .gap(theme::S2)
            .w_full()
            .max_w(px(1152.))
            .mx_auto()
            .px(theme::S6)
            .pt(theme::S4)
            .children(notices)
    })
}
