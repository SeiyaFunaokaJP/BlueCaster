//! VB-Audio's virtual cables: the Hi-Fi Cable for the sound (it adds the
//! least delay), VB-CABLE for the headphones' microphone. The pages show one
//! amber line while a cable is missing, a green one once both are installed
//! (issue #86: kept, with its Learn more); Learn more opens the dialog with both
//! cables, each with Install (Download for the Hi-Fi Cable) or Uninstall, and
//! VB-Audio's links. Nothing else in the app talks about the cables (issue
//! #86). Install VB-CABLE has the core fetch VB-Audio's own pack from
//! VB-Audio and run its setup (BlueCaster ships none of it); Uninstall runs
//! VB-Audio's installed setup. VB-Audio asks that the user can tell it is
//! their donationware, so its origin is said under each cable
//! (https://vb-audio.com/Services/licensing.htm). The line cannot be closed
//! (issue #86: other ways to the microphone only bring bug reports);
//! Developer > Hide the virtual audio device warning removes it, green too. About >
//! Third-party software opens the same dialog, the only place that names
//! the cables' origin. After an update from a version without it, the dialog
//! opens by itself once on the main window (issue #86: the setup, which shows
//! the line, does not run again).

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, AsyncApp, Context, Entity, FontWeight, IntoElement, ParentElement as _, Render, RenderOnce,
    SharedString, Styled as _, Task, WeakEntity, Window, div, px,
};
use ipc::{AudioDevice, Method};
use ui_model::i18n::t;
use ui_model::links::LINKS;
use ui_model::view::about::{
    CableButton, CablesOnceStart, POLL_MS, POLLS, cable_after, cable_buttons, cable_installed, cable_line,
    cables_once_opens, cables_once_start, link_text,
};

use crate::app::{self, NoticeKind};
use crate::host;
use crate::theme::{self, ActivePalette as _};
use crate::ui::dialog::{self, Question};
use crate::ui::{self, ButtonKind, IconName, icon};

/// Reads Windows' outputs again (the owner's list)
pub type Reload = Rc<dyn Fn(&mut App)>;

/// A link to a page, opened in the browser (a link button: assistive
/// technology finds and presses it, which it cannot with a plain `Link`)
fn link(id: impl Into<gpui_kit::ElementId>, url: &'static str, label: impl Into<SharedString>) -> Button {
    Button::new(id).link().label(label.into()).on_click(move |_, _, cx| host::open_url(url, cx))
}

/// VB-Audio's name for it, and that it is donationware: a link to its site
fn origin(id: &'static str, text: String, url: &'static str, cx: &App) -> impl IntoElement + use<> {
    ui::hint(String::new(), cx).flex().flex_wrap().gap(theme::S1).mt(theme::S1).child(text).child(link(
        id,
        url,
        link_text(url).to_owned(),
    ))
}

/// One cable: its name with Installed or Not installed, its origin, and
/// Install or Uninstall. `installed` None until Windows' outputs are read.
pub struct CableRow {
    hifi: bool,
    installed: Option<bool>,
    on_changed: Reload,
    busy: bool,
    /// Done, but Windows still lists it, or does not list it yet
    restart: bool,
    /// Install failed (no download, VB-Audio changed the pack, ...): their page
    failed: bool,
    /// Windows' outputs read again for a while: a cable may show up (or go)
    /// after the answer (issue #86: installed, yet the error and Download stayed)
    polling: Option<Task<()>>,
}

impl CableRow {
    fn new(hifi: bool, on_changed: Reload) -> CableRow {
        CableRow { hifi, installed: None, on_changed, busy: false, restart: false, failed: false, polling: None }
    }

    /// The cable came or went: what the last try said no longer holds
    fn set_installed(&mut self, installed: Option<bool>, cx: &mut Context<Self>) {
        if self.installed != installed {
            self.installed = installed;
            self.polling = None;
            self.restart = false;
            self.failed = false;
            cx.notify();
        }
    }

    fn poll(&mut self, cx: &mut Context<Self>) {
        let on_changed = self.on_changed.clone();
        self.polling = Some(cx.spawn(async move |_, cx: &mut AsyncApp| {
            for _ in 0..POLLS {
                cx.background_executor().timer(Duration::from_millis(POLL_MS)).await;
                cx.update(|cx| on_changed(cx));
            }
        }));
    }

    fn act(&mut self, uninstall: bool, cx: &mut Context<Self>) {
        self.busy = true;
        self.restart = false;
        self.failed = false;
        self.polling = None;
        cx.notify();
        let method =
            if uninstall { Method::AudioUninstallCable { hifi: self.hifi } } else { Method::AudioInstallVbCable };
        let task = app::request::<bool>(method, cx);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let answer = task.await;
            let after = cable_after(answer.as_ref().map(|done| *done).map_err(|e| e.code.as_str()), uninstall);
            let error = answer.err().filter(|_| after.notify).map(|e| app::error_text(&e));
            let _ = this.update(cx, |row, cx| {
                row.busy = false;
                row.restart = after.restart;
                row.failed = after.failed;
                if after.restart {
                    row.poll(cx);
                }
                if let Some(e) = error {
                    app::notify(e, NoticeKind::Error, cx);
                }
                cx.notify();
                // Always: an answer that came too late or said "not yet" may be wrong
                (row.on_changed)(cx);
            });
        })
        .detach();
    }
}

impl Render for CableRow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let (name, origin_text, origin_url, id) = if self.hifi {
            ("Hi-Fi Cable", t("hificable.origin"), LINKS.vb_audio_home, "hifi")
        } else {
            ("VB-CABLE", t("vbcable.origin"), LINKS.vb_cable_home, "vb")
        };
        let page = if self.hifi { LINKS.hifi_cable } else { LINKS.vb_cable };
        let tag = self.installed.map(|on| {
            let (text, color) = if on {
                (t("cables.installed"), (p.good_text, p.good_line))
            } else {
                (t("cables.not_installed"), (p.warn_text, p.warn_line))
            };
            div()
                .flex()
                .items_center()
                .px(theme::S2)
                .min_h(px(20.))
                .border_1()
                .border_color(color.1)
                .rounded_full()
                .text_size(theme::FS_SMALL)
                .text_color(color.0)
                .whitespace_nowrap()
                .child(text)
        });
        let buttons = cable_buttons(self.installed, self.hifi, self.failed).into_iter().map(|b| match b {
            CableButton::Uninstall => ui::button("uninstall", t("cables.uninstall"), ButtonKind::Default)
                .loading(self.busy)
                .on_click(cx.listener(|this, _, _, cx| this.act(true, cx)))
                .into_any_element(),
            CableButton::Install => ui::button("install", t("cables.install"), ButtonKind::Default)
                .icon(icon(IconName::ArrowDownload))
                .loading(self.busy)
                .on_click(cx.listener(|this, _, _, cx| this.act(false, cx)))
                .into_any_element(),
            CableButton::Download => ui::button("download", t("cables.download"), ButtonKind::Default)
                .icon(icon(IconName::Open))
                .on_click(move |_, _, cx| host::open_url(page, cx))
                .into_any_element(),
        });
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x(theme::S4)
            .gap_y(theme::S2)
            .child(
                div()
                    .flex_1()
                    .min_w(px(16. * 14.))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(theme::S2)
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(name))
                            .children(tag),
                    )
                    .child(origin(id, origin_text, origin_url, cx)),
            )
            .child(div().flex().flex_wrap().items_center().justify_end().gap(theme::S2).children(buttons))
            .when(self.restart, |d| {
                d.child(
                    div()
                        .w_full()
                        .flex()
                        .items_center()
                        .gap(theme::S1)
                        .text_size(theme::FS_SMALL)
                        .text_color(p.warn_text)
                        .child(icon(IconName::Warning))
                        .child(t("cables.restart")),
                )
            })
    }
}

/// Both cables and VB-Audio's links (`devices`: Windows' outputs, None
/// until read; `reload` reads them again)
pub struct CablesDialog {
    vb: Entity<CableRow>,
    hifi: Entity<CableRow>,
}

impl CablesDialog {
    pub fn new(reload: Reload, cx: &mut Context<Self>) -> CablesDialog {
        let vb = cx.new(|_| CableRow::new(false, reload.clone()));
        let hifi = cx.new(|_| CableRow::new(true, reload));
        CablesDialog { vb, hifi }
    }

    /// Windows' outputs as the owner read them (None: not yet)
    pub fn set_devices(&mut self, devices: Option<&[AudioDevice]>, cx: &mut Context<Self>) {
        let (vb, hifi) = (cable_installed(devices, false), cable_installed(devices, true));
        self.vb.update(cx, |r, cx| r.set_installed(vb, cx));
        self.hifi.update(cx, |r, cx| r.set_installed(hifi, cx));
    }

    /// Opens the dialog; `on_close` runs when it closes
    pub fn open(
        this: &Entity<Self>,
        on_close: impl Fn(&mut Window, &mut App) + 'static,
        window: &mut Window,
        cx: &mut App,
    ) {
        let view = this.clone();
        dialog::ask(
            Question::new(t("cables.title")).on_cancel(on_close),
            move |_, _| view.clone().into_any_element(),
            |_, _| {},
            window,
            cx,
        );
    }
}

impl Render for CablesDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let rule = || div().w_full().h(px(1.)).bg(p.border);
        let bullet = |text: String| {
            div().flex().gap(theme::S2).child(div().flex_none().child("•")).child(div().flex_1().min_w_0().child(text))
        };
        div()
            .flex()
            .flex_col()
            .gap(theme::S4)
            .child(
                div().flex().flex_col().gap(theme::S1).child(t("cables.intro")).child(
                    div()
                        .flex()
                        .flex_col()
                        .pl(theme::S2)
                        .child(bullet(t("cables.reason_separate")))
                        .child(bullet(t("cables.reason_features"))),
                ),
            )
            .child(rule())
            .child(self.vb.clone())
            .child(self.hifi.clone())
            .child(rule())
            .child(
                div().child(div().font_weight(FontWeight::SEMIBOLD).child("VB-Audio")).child(
                    ui::hint(String::new(), cx)
                        .flex()
                        .flex_wrap()
                        .gap(theme::S4)
                        .mt(theme::S1)
                        .child(link("vb-audio", LINKS.vb_audio_home, "vb-audio.com"))
                        .child(link("vb-cable", LINKS.vb_cable_home, "vb-cable.com"))
                        .child(link("licensing", LINKS.vb_audio_licensing, t("cables.licensing"))),
                ),
            )
    }
}

/// One amber line while a cable is missing, a green one once both are
/// installed (`devices`: Windows' outputs, None until read); Learn more
/// opens both cables. It cannot be closed. Use as
/// `.child(cable_notes(devices, reload))` in a screen that reads the outputs.
pub fn cable_notes(devices: Option<&[AudioDevice]>, reload: impl Fn(&mut App) + 'static) -> CableNotes {
    CableNotes { devices: devices.map(<[AudioDevice]>::to_vec), reload: Rc::new(reload) }
}

#[derive(IntoElement)]
pub struct CableNotes {
    devices: Option<Vec<AudioDevice>>,
    reload: Reload,
}

impl RenderOnce for CableNotes {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let reload = self.reload.clone();
        let dialog = window.use_keyed_state("cable-notes-dialog", cx, move |_, cx| CablesDialog::new(reload, cx));
        dialog.update(cx, |d, cx| d.set_devices(self.devices.as_deref(), cx));
        let settings = app::core(cx).read(cx).snap().map(|s| s.settings.clone());
        let line = settings.and_then(|s| cable_line(self.devices.as_deref(), &s));
        let Some(missing) = line else { return div().into_any_element() };
        let p = cx.palette();
        let (name, line_color, text_color, text) = if missing {
            (IconName::Warning, p.warn_line, p.warn_text, t("cables.recommended"))
        } else {
            (IconName::CheckmarkCircle, p.good_line, p.good_text, t("cables.installed_both"))
        };
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(theme::S3)
            .px(theme::S4)
            .py(theme::S3)
            .bg(p.surface)
            .border_1()
            .border_color(line_color)
            .rounded(theme::RADIUS_CARD)
            .child(div().text_color(text_color).child(icon(name)))
            .child(div().flex_1().min_w(px(16. * 14.)).child(text))
            .child(
                ui::button("cables-learn-more", t("cables.learn_more"), ButtonKind::Default)
                    .on_click(move |_, window, cx| CablesDialog::open(&dialog, |_, _| {}, window, cx)),
            )
            .into_any_element()
    }
}

/// The dialog once by itself after an update (settings.cables_dialog_shown:
/// false in a folder set up before it). Both cables installed, or the
/// warning hidden in developer mode: nothing to show, it only counts as
/// shown. Closed: never again.
pub struct CablesOnce {
    dialog: Entity<CablesDialog>,
    open: bool,
    devices_key: String,
}

/// The main window's `CablesOnce`: make it once with the window and keep it
/// (it draws nothing itself; the dialog opens over the window)
pub fn cables_once(window: &mut Window, cx: &mut App) -> Entity<CablesOnce> {
    cx.new(|cx| CablesOnce::new(window, cx))
}

fn shown(cx: &App) {
    app::save_settings(|s| s.cables_dialog_shown = true, cx);
}

impl CablesOnce {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> CablesOnce {
        let core = app::core(cx);
        let weak = cx.weak_entity();
        let reload: Reload = Rc::new(move |cx: &mut App| {
            let _ = weak.update(cx, |this: &mut CablesOnce, cx| this.load(cx));
        });
        let dialog = cx.new(|cx| CablesDialog::new(reload, cx));
        let devices_key = ui::audio_devices_key(core.read(cx).snap());
        cx.observe(&core, |this, core, cx| {
            let key = ui::audio_devices_key(core.read(cx).snap());
            if key != this.devices_key {
                this.devices_key = key;
                if this.open {
                    this.load(cx);
                }
            }
        })
        .detach();
        let this = CablesOnce { dialog, open: false, devices_key };
        let settings = core.read(cx).snap().map(|s| s.settings.clone());
        match settings.as_ref().map(cables_once_start) {
            None | Some(CablesOnceStart::Nothing) => {}
            Some(CablesOnceStart::CountAsShown) => shown(cx),
            Some(CablesOnceStart::Read) => {
                let task = app::request::<Vec<AudioDevice>>(Method::AudioDevices, cx);
                cx.spawn_in(window, async move |this, cx| {
                    // Not read: next time
                    let Ok(list) = task.await else { return };
                    let _ = this.update_in(cx, |this, window, cx| {
                        let opens = cables_once_opens(&list);
                        this.set_devices(list, cx);
                        if opens {
                            this.show(window, cx);
                        } else {
                            shown(cx);
                        }
                    });
                })
                .detach();
            }
        }
        this
    }

    fn set_devices(&mut self, list: Vec<AudioDevice>, cx: &mut Context<Self>) {
        self.dialog.update(cx, |d, cx| d.set_devices(Some(&list), cx));
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let task = app::request::<Vec<AudioDevice>>(Method::AudioDevices, cx);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let Ok(list) = task.await else { return };
            let _ = this.update(cx, |this, cx| this.set_devices(list, cx));
        })
        .detach();
    }

    fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = true;
        let weak = cx.weak_entity();
        CablesDialog::open(
            &self.dialog,
            move |_, cx| {
                let _ = weak.update(cx, |this, _| this.open = false);
                shown(cx);
            },
            window,
            cx,
        );
    }
}

impl Render for CablesOnce {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
