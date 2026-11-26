//! The tray flyout (360 wide): everyday control without the main window.
//! As Home's card (issue #86): the ring, and beside it the device chooser
//! in the title's place, battery and state (and headset mode in developer
//! mode), the codec (chosen right there) and link quality;
//! then the volume, the big Connect / Disconnect with "Open BlueCaster" beside it. No close
//! button: the flyout hides on Escape and when it loses the focus. Its
//! window is as tall as the page (issue #86: no empty space), so the page
//! keeps its natural height (flyout_window.rs measures it).
//!
//! Which parts show, the error shown and where a fix leads:
//! `ui_model::view::flyout`, with the tests.

use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AnyView, App, AppContext as _, Context, Div, Entity, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, Window, div, px,
};
use ipc::{ErrorInfo, StreamState};
use ui_model::i18n::{t, tm};
use ui_model::profile::{audio_entries, entry_of};
use ui_model::view::flyout::{FixTarget, FlyoutError, FlyoutParts, fix_target, flyout_error, flyout_parts};
use ui_model::view::stream::{stream_active, stream_status_text};

use crate::app;
use crate::core::{self, Core, WantKind};
use crate::screens::home::codec_picker;
use crate::screens::status::capture_sign_to;
use crate::shell;
use crate::theme::{self, ActivePalette as _};
use crate::ui::device_switcher::device_switcher;
use crate::ui::mixer::MixerList;
use crate::ui::stream::{
    CodecBadge, ConnectButton, QualityIndicator, VolumeControl, adapter_drivers, battery_level, codec_badge,
    copy_error, current_profile, dot, fix_for, fix_icon, is_app_fault, mic_row, report_error, ring_sized, run_fix,
    state_colors,
};
use crate::ui::{self, ButtonKind, IconName, icon};
use crate::windows;

/// The ring's size (`.fly-head .ring-small`: 3.5rem)
const FLY_RING: f32 = 56.;

/// Makes the flyout's page
pub fn open(window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| Flyout::new(window, cx)).into()
}

/// In the flyout a fix that needs a screen opens the main window there
/// (it closes the flyout)
fn open_main(route: Option<&str>, cx: &mut App) {
    windows::show_main(route.map(app::parse_route), true, cx);
}

pub struct Flyout {
    core: Entity<Core>,
    connect: Entity<ConnectButton>,
    volume: Entity<VolumeControl>,
    quality: Entity<QualityIndicator>,
    /// While the mixer shows (developer mode's volume mixer, issue #223)
    mixer: Option<Entity<MixerList>>,
    /// A failed Connect / Disconnect
    req_error: Option<ErrorInfo>,
    /// What clears it: the stream's state moving on, an adapter's driver
    /// changing (as on Home)
    seen: Option<(StreamState, String)>,
}

impl Flyout {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Flyout {
        let core = app::core(cx);
        cx.observe(&core, |this: &mut Self, core, cx| {
            let now = core.read(cx).snap().map(|s| (s.stream.state, adapter_drivers(s)));
            if this.seen != now {
                // As on Home: until the state moves on or an adapter's driver changes
                if this.seen.is_some() {
                    this.req_error = None;
                }
                this.seen = now;
            }
            cx.notify();
        })
        .detach();
        cx.observe(&app::state(cx), |_, _, cx| cx.notify()).detach();
        core::want(WantKind::Link, cx);
        let me = cx.weak_entity();
        let connect = cx.new(|cx| {
            ConnectButton::new(true, window, cx).on_error(move |e, cx| {
                let _ = me.update(cx, |this, cx| {
                    this.req_error = Some(e);
                    cx.notify();
                });
            })
        });
        let volume = cx.new(|cx| VolumeControl::new(false, window, cx));
        let quality = cx.new(|cx| QualityIndicator::new(window, cx));
        let seen = core.read(cx).snap().map(|s| (s.stream.state, adapter_drivers(s)));
        Flyout { core, connect, volume, quality, mixer: None, req_error: None, seen }
    }

    /// As on Home (issue #86): no device yet, the way to add one
    fn no_device(&self, cx: &App) -> Div {
        let p = cx.palette();
        ui::card(cx)
            .items_center()
            .gap(theme::S3)
            .py(theme::S8)
            .px(theme::S4)
            .text_color(p.text_2)
            .text_center()
            .child(icon(IconName::Headphones).size(px(40.)))
            .child(div().child(t("ui.home.no_device")))
            .child(
                ui::button("fly-add-device", t("ui.home.add_device"), ButtonKind::Strong)
                    .icon(icon(IconName::Add))
                    .on_click(|_, _, cx| open_main(Some("devices"), cx)),
            )
    }

    /// The ring, and beside it the device chooser, battery | state, the
    /// codec and the microphone's line
    fn head(&self, cx: &App) -> Option<Div> {
        let p = *cx.palette();
        let snap = self.core.read(cx).snap()?;
        let s = &snap.stream;
        let parts = flyout_parts(snap);
        let entries = audio_entries(snap);
        let entry = entry_of(&entries, current_profile(snap));
        let (dot_color, text_color) = state_colors(s.state, &p);
        // An Auracast broadcast (developer mode) is no device: its name in
        // the title's place, no battery, codec or microphone. While it runs
        // or starts only: one that ended in an error keeps its name until
        // the next stream, and Connect is for the device again
        let broadcast = s.broadcast.as_ref().filter(|_| stream_active(s.state));
        let parts = if broadcast.is_some() { FlyoutParts { battery: false, ..parts } } else { parts };
        let entry = entry.filter(|_| broadcast.is_none());

        // The device chooser is the title: the device once (issue #86)
        let title = if let Some(name) = broadcast {
            div()
                .text_size(theme::FS_LARGE)
                .font_family(theme::FONT_DISPLAY)
                .font_weight(FontWeight::SEMIBOLD)
                .truncate()
                .child(format!("{} · {name}", t("ui.dev.auracast")))
        } else if parts.profile {
            device_switcher(cx)
        } else {
            div()
                .text_size(theme::FS_LARGE)
                .font_family(theme::FONT_DISPLAY)
                .font_weight(FontWeight::SEMIBOLD)
                .truncate()
                .child(t("profile.none_selected"))
        };
        // As on Home and in the sidebar: battery | state, the codec (issue #86)
        let state = div()
            .flex()
            .items_center()
            .gap(theme::S3)
            .min_w_0()
            .text_size(theme::FS_SMALL)
            .when(parts.battery, |d| {
                d.child(battery_level("fly-battery", s.battery, false, cx)).child(div().text_color(p.text_2).child("|"))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(theme::S2)
                    .min_w_0()
                    .text_color(text_color)
                    // No red dot for an error, as on Home (issue #86)
                    .when(s.state != StreamState::Error, |d| d.child(dot(dot_color)))
                    .child(div().min_w_0().truncate().child(stream_status_text(s))),
            );
        // The codec chosen right there, as on Home (issue #86); the link
        // quality beside its details
        let codec = broadcast
            .is_none()
            .then(|| {
                codec_badge(
                    CodecBadge {
                        aside: Some(self.quality.clone().into_any_element()),
                        picker: entry.map(|e| codec_picker(e, cx)),
                        stretch: true,
                        ..CodecBadge::default()
                    },
                    cx,
                )
            })
            .flatten();
        Some(
            div()
                .flex()
                .items_start()
                .gap(theme::S3)
                .child(ring_sized(
                    "fly-ring",
                    s.state,
                    s.volume,
                    FLY_RING,
                    true,
                    icon(if broadcast.is_some() { IconName::Pulse } else { IconName::Headphones }),
                    cx,
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(theme::S2)
                        .child(title)
                        .child(state)
                        .children(codec)
                        .children(entry.and_then(|e| mic_row(e, cx))),
                ),
        )
    }

    /// "Open BlueCaster" beside Connect, as tall as it: no row of its own
    /// (issue #86)
    fn actions(&self, cx: &App) -> Div {
        let p = cx.palette();
        div()
            .flex()
            .gap(theme::S2)
            .child(
                ui::icon_button("fly-open", IconName::Open, t("ui.flyout.open_app"))
                    .flex_none()
                    .w(px(52.))
                    .h(px(52.))
                    .rounded(theme::RADIUS_CARD)
                    .border_1()
                    .border_color(p.border)
                    .bg(p.surface)
                    .on_click(|_, _, cx| open_main(None, cx)),
            )
            .child(div().flex_1().min_w_0().child(self.connect.clone()))
    }

    /// The message has the width to itself, the buttons go below it
    fn error(&self, e: FlyoutError, cx: &App) -> Div {
        let p = cx.palette();
        let code = e.code.clone();
        let fix = fix_for(code.as_deref(), &e.message.key, cx);
        let (copy_msg, copy_code) = (e.message.clone(), code.clone());
        let (report_msg, report_code) = (e.message.clone(), code.clone());
        let actions = div()
            .flex()
            .items_center()
            .gap(theme::S1)
            // The fix first, copy and report at the end
            .when_some(fix, |d, f| {
                let target = fix_target(code.as_deref(), &e.message.key, &f);
                d.child(div().min_w_0().child(
                    ui::button("fly-fix", f.label(), ButtonKind::Subtle).icon(icon(fix_icon(&f))).on_click(
                        move |_, window, cx| match target {
                            // Fixes that are a screen of the main window go there
                            FixTarget::Main(route) => open_main(Some(route), cx),
                            // A failure shows as a notice
                            FixTarget::Here => run_fix(&f, window, cx).detach(),
                        },
                    ),
                ))
            })
            .child(div().flex_1())
            .child(
                ui::icon_button("fly-copy", IconName::Copy, t("ui.error.copy_hint"))
                    .on_click(move |_, _, cx| copy_error(&copy_msg, copy_code.as_deref(), cx)),
            )
            .when(is_app_fault(&e.message.key, code.as_deref()), |d| {
                d.child(
                    ui::icon_button("fly-report", IconName::Open, t("ui.error.report_hint"))
                        .on_click(move |_, _, cx| report_error(&report_msg, report_code.as_deref(), cx)),
                )
            });
        div()
            .flex()
            .flex_col()
            .gap(theme::S2)
            .p(theme::S2)
            .border_1()
            .border_color(p.bad_line)
            .rounded(theme::RADIUS)
            .text_size(theme::FS_SMALL)
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(theme::S2)
                    .child(div().flex_none().mt(px(1.)).text_color(p.bad_text).child(icon(IconName::ErrorCircle)))
                    .child(div().flex_1().min_w_0().child(tm(Some(&e.message)))),
            )
            .child(actions)
    }

    /// Only when there is something to show: an update, or "Open
    /// BlueCaster" with no Connect beside it
    fn footer(update: bool, open: bool) -> Div {
        div()
            .flex()
            .items_center()
            .justify_end()
            .when(update, |d| {
                d.child(
                    ui::button("fly-update", t("update.available_title"), ButtonKind::Subtle)
                        .icon(icon(IconName::ArrowSync))
                        .on_click(|_, _, cx| open_main(Some("settings"), cx)),
                )
                .child(div().flex_1())
            })
            .when(open, |d| {
                d.child(
                    ui::button("fly-open-app", t("ui.flyout.open_app"), ButtonKind::Default)
                        .icon(icon(IconName::Open))
                        .on_click(|_, _, cx| open_main(None, cx)),
                )
            })
    }
}

impl Render for Flyout {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let page = div()
            .relative()
            .w_full()
            .flex()
            .flex_col()
            .gap(theme::S3)
            .p(theme::S4)
            .border_1()
            .border_color(p.border)
            .rounded(theme::RADIUS_CARD)
            .bg(p.bg)
            .overflow_hidden();
        let seen = {
            let core = self.core.read(cx);
            core.snap().filter(|_| core.ready()).map(|snap| {
                (
                    flyout_parts(snap),
                    app::mixer_shown(Some(&snap.settings)),
                    flyout_error(self.req_error.as_ref(), snap),
                )
            })
        };
        // The connection screen while the core is not ready (Gate compact)
        let Some((parts, mixer_shown, error)) = seen else {
            self.mixer = None;
            let core = self.core.read(cx);
            return page.min_h(px(200.)).child(shell::connection_screen(&core.conn, true, cx)).into_any_element();
        };

        if !mixer_shown {
            self.mixer = None;
        } else if self.mixer.is_none() {
            self.mixer = Some(cx.new(|cx| MixerList::new(window, cx)));
        }

        let body: Vec<AnyElement> = if parts.no_device {
            vec![self.no_device(cx).into_any_element()]
        } else {
            let mut body: Vec<AnyElement> = self.head(cx).into_iter().map(IntoElement::into_any_element).collect();
            if parts.profile {
                // As on Home: the volume while a device is in use
                if parts.volume {
                    body.push(self.volume.clone().into_any_element());
                }
                body.push(self.actions(cx).into_any_element());
            } else {
                body.push(ui::muted(t("profile.empty"), cx).into_any_element());
            }
            body
        };

        page.children(body)
            // One click from the taskbar (issue #223)
            .when_some(self.mixer.clone(), |d, mixer| {
                d.child(div().id("fly-mixer").max_h(px(176.)).overflow_y_scrollbar().child(mixer))
            })
            // Kept simple: the capture source is chosen in the main window (issue #86)
            .when(parts.capture, |d| {
                d.child(
                    div()
                        .flex()
                        .justify_between()
                        .items_center()
                        .gap(theme::S2)
                        .min_w_0()
                        .text_size(theme::FS_SMALL)
                        .child(capture_sign_to(false, |cx| open_main(Some("diagnostics"), cx), cx)),
                )
            })
            .when_some(error, |d, e| d.child(self.error(e, cx)))
            .when(parts.footer, |d| d.child(Self::footer(parts.update, !parts.actions)))
            .child(shell::notice_line(cx))
            .into_any_element()
    }
}
