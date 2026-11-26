//! The service add-on (issue #413): an optional step for keyboards and mice
//! that makes them work on the sign-in screen and in administrator windows
//! too. Without it the feature works as before, in ordinary windows only.
//! Adding, updating and removing run the set-up tool as administrator
//! (Windows' own prompt; ui_host::service). Adding the service does not
//! turn keyboards and mice on: HidEnableDialog still does that.
//!
//! The window reaches the Developer screen only while a core serves it, so
//! "stopped" and "needs an update" show on the connection screen instead
//! ([`connection_notice`]), where no developer mode is needed.

use std::rc::Rc;

use gpui_kit::component::Disableable as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, AsyncApp, AsyncWindowContext, Context, Entity, FontWeight, IntoElement, ParentElement as _,
    Render, RenderOnce, Styled as _, Task, Window, div, px,
};
use ui_model::i18n::t;
use ui_model::links::LINKS;
use ui_model::t;
use ui_model::view::about::{
    ServiceButton, ServiceKind, SetupOutcome, service_buttons, service_state_text, setup_error,
};

use crate::host::{self, ServiceAction, ServiceOutcome, ServiceState};
use crate::theme::{self, ActivePalette as _};
use crate::ui::dialog::{self, Question};
use crate::ui::{self, ButtonKind, IconName, icon};

fn kind(s: &ServiceState) -> ServiceKind {
    match s {
        ServiceState::NotAdded => ServiceKind::NotAdded,
        ServiceState::Running { .. } => ServiceKind::Running,
        ServiceState::Stopped { .. } => ServiceKind::Stopped,
        ServiceState::NeedsUpdate { .. } => ServiceKind::NeedsUpdate,
        ServiceState::AppOlder { .. } => ServiceKind::AppOlder,
    }
}

fn outcome(o: &ServiceOutcome) -> SetupOutcome {
    match o {
        ServiceOutcome::Done => SetupOutcome::Done,
        ServiceOutcome::Declined => SetupOutcome::Declined,
        ServiceOutcome::Failed { code } => SetupOutcome::Failed(*code),
        ServiceOutcome::NotStarted { .. } => SetupOutcome::NotStarted,
    }
}

/// Runs one set-up at a time; the state and the last error
pub struct ServiceSetup {
    state: Option<ServiceState>,
    busy: bool,
    error: Option<String>,
}

impl ServiceSetup {
    fn new() -> ServiceSetup {
        ServiceSetup { state: None, busy: false, error: None }
    }

    fn read(&mut self, cx: &mut Context<Self>) {
        let task = host::service_state(cx);
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let state = task.await;
            let _ = this.update(cx, |s, cx| {
                s.state = Some(state);
                cx.notify();
            });
        })
        .detach();
    }

    /// -> whether it was done
    fn run(this: &Entity<Self>, action: ServiceAction, remove_data: bool, cx: &mut App) -> Task<bool> {
        this.update(cx, |s, cx| {
            s.busy = true;
            s.error = None;
            cx.notify();
        });
        let task = host::service_setup(action, remove_data, cx);
        let this = this.downgrade();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let r = task.await;
            let done = r.outcome == ServiceOutcome::Done;
            let _ = this.update(cx, |s, cx| {
                s.error = setup_error(outcome(&r.outcome));
                s.state = Some(r.service);
                s.busy = false;
                cx.notify();
            });
            done
        })
    }
}

/// Which question a dialog asks
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ask {
    Add,
    Remove,
}

/// Asks before adding (what the service means for this PC) or removing
/// (deleting the service's data is off unless chosen). The dialog stays,
/// busy, until the set-up ended, then closes however it ended; `on_end`
/// runs when it closes.
fn ask(ask: Ask, setup: Entity<ServiceSetup>, on_end: Rc<dyn Fn(&mut App)>, window: &mut Window, cx: &mut App) {
    // Each opening starts with the data kept
    let data = cx.new(|_| false);
    let question = {
        let (setup, on_end) = (setup.clone(), on_end.clone());
        move |_: &mut Window, cx: &mut App| {
            let on_end = on_end.clone();
            let q = match ask {
                Ask::Add => Question::new(t("ui.service.add_title")).confirm(t("ui.service.add")),
                Ask::Remove => Question::new(t("ui.service.remove_title")).warn().confirm(t("ui.service.remove")),
            };
            q.confirm_icon(IconName::Shield).busy(setup.read(cx).busy).on_cancel(move |_, cx| on_end(cx))
        }
    };
    let body = {
        let (setup, data) = (setup.clone(), data.clone());
        move |_: &mut Window, cx: &mut App| {
            let busy = setup.read(cx).busy;
            let remove_data = *data.read(cx);
            match ask {
                Ask::Add => div()
                    .flex()
                    .flex_col()
                    .gap(theme::S3)
                    .child(t("ui.service.add_all_users"))
                    .child(t("ui.service.add_headphones")),
                Ask::Remove => {
                    let data = data.clone();
                    div().flex().flex_col().gap(theme::S3).child(t("ui.service.remove_text")).child(ui::check(
                        "service-remove-data",
                        t("ui.service.remove_data"),
                        remove_data,
                        busy,
                        move |v, _, cx| {
                            let v = *v;
                            data.update(cx, |d, cx| {
                                *d = v;
                                cx.notify();
                            });
                        },
                    ))
                }
            }
            .into_any_element()
        }
    };
    // The dialog closes however the set-up ended
    let on_confirm = move |window: &mut Window, cx: &mut App| {
        if setup.read(cx).busy {
            return;
        }
        let task = match ask {
            Ask::Add => ServiceSetup::run(&setup, ServiceAction::Install, false, cx),
            Ask::Remove => ServiceSetup::run(&setup, ServiceAction::Remove, *data.read(cx), cx),
        };
        let on_end = on_end.clone();
        window
            .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                task.await;
                let _ = cx.update(|window, cx| {
                    window.close_dialog(cx);
                    on_end(cx);
                });
            })
            .detach();
    };
    dialog::show(question, body, on_confirm, window, cx);
}

/// Developer > keyboards and mice: the optional service, by what it gives
pub struct ServiceRow {
    setup: Entity<ServiceSetup>,
    adding: bool,
    removing: bool,
}

impl ServiceRow {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> ServiceRow {
        let setup = cx.new(|cx| {
            let mut s = ServiceSetup::new();
            s.read(cx);
            s
        });
        cx.observe(&setup, |_, _, cx| cx.notify()).detach();
        ServiceRow { setup, adding: false, removing: false }
    }

    /// The dialog closes however the set-up ended; the row shows an error
    fn open(&mut self, which: Ask, window: &mut Window, cx: &mut Context<Self>) {
        match which {
            Ask::Add => self.adding = true,
            Ask::Remove => self.removing = true,
        }
        cx.notify();
        let weak = cx.weak_entity();
        let on_end: Rc<dyn Fn(&mut App)> = Rc::new(move |cx| {
            let _ = weak.update(cx, |this, cx| {
                this.adding = false;
                this.removing = false;
                cx.notify();
            });
        });
        ask(which, self.setup.clone(), on_end, window, cx);
    }
}

impl Render for ServiceRow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let s = self.setup.read(cx);
        let Some(state) = s.state.as_ref() else { return div().into_any_element() };
        let (busy, k) = (s.busy, kind(state));
        let error = (!self.adding && !self.removing).then(|| s.error.clone()).flatten();
        let (first, remove) = service_buttons(k);
        let p = cx.palette();
        let setup = self.setup.clone();
        ui::setting_row(cx)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(t("ui.service.row"))
                    .child(ui::hint(t(service_state_text(k)), cx))
                    .when_some(error, |d, e| d.child(div().text_size(theme::FS_SMALL).text_color(p.bad_text).child(e))),
            )
            .child(
                ui::inline()
                    .when_some(first, |d, b| {
                        let button = match b {
                            ServiceButton::Add => ui::button("service-add", t("ui.service.add"), ButtonKind::Default)
                                .icon(icon(IconName::Shield))
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, window, cx| this.open(Ask::Add, window, cx))),
                            ServiceButton::Update | ServiceButton::Start => {
                                let label =
                                    if b == ServiceButton::Update { "ui.service.update" } else { "ui.service.start" };
                                let setup = setup.clone();
                                ui::button("service-update", t(label), ButtonKind::Default)
                                    .icon(icon(IconName::Shield))
                                    .loading(busy)
                                    .on_click(move |_, _, cx| {
                                        ServiceSetup::run(&setup, ServiceAction::Update, false, cx).detach();
                                    })
                            }
                        };
                        d.child(button)
                    })
                    .when(remove, |d| {
                        d.child(
                            ui::button("service-remove", t("ui.service.remove"), ButtonKind::Subtle)
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, window, cx| this.open(Ask::Remove, window, cx))),
                        )
                    }),
            )
            .into_any_element()
    }
}

/// A connection screen's heading
fn title(text: String) -> impl IntoElement {
    div().text_size(theme::FS_TITLE).font_weight(FontWeight::SEMIBOLD).child(text)
}

/// A connection screen's paragraph
fn para(text: String) -> impl IntoElement {
    div().max_w(px(34. * 16.)).child(text)
}

/// The app's and the service's versions, when both are known
fn versions(app: &str, version: &str, cx: &App) -> Option<impl IntoElement + use<>> {
    (!version.is_empty() && !app.is_empty())
        .then(|| ui::mono(t!("ui.service.versions", app, version)).text_color(cx.palette().text_2))
}

/// The connection screen while the service add-on is older or stopped: no
/// core serves this window until the service is updated or started
/// (`versions`: the service's and the app's)
pub fn connection_notice(stopped: bool, versions: Option<(&String, &String)>, _cx: &App) -> impl IntoElement + use<> {
    ServiceConnectionNotice {
        stopped,
        version: versions.map(|(v, _)| v.clone()).unwrap_or_default(),
        app: versions.map(|(_, a)| a.clone()).unwrap_or_default(),
    }
}

#[derive(IntoElement)]
struct ServiceConnectionNotice {
    stopped: bool,
    version: String,
    app: String,
}

/// The notice's own state: the set-up, and whether the remove dialog is open
struct NoticeState {
    setup: Entity<ServiceSetup>,
    removing: bool,
}

impl RenderOnce for ServiceConnectionNotice {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state("service-connection-notice", cx, |_, cx| {
            let setup = cx.new(|_| ServiceSetup::new());
            cx.observe(&setup, |_, _, cx| cx.notify()).detach();
            NoticeState { setup, removing: false }
        });
        let (setup, removing) = {
            let n = state.read(cx);
            (n.setup.clone(), n.removing)
        };
        let (busy, error) = {
            let s = setup.read(cx);
            (s.busy, (!removing).then(|| s.error.clone()).flatten())
        };
        let p = cx.palette();
        let head = if self.stopped {
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(theme::S3)
                .child(title(t("ui.service.stopped_title")))
                .child(para(t("ui.service.stopped_body")))
        } else {
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(theme::S3)
                .child(title(t("ui.service.outdated_title")))
                .child(para(t("ui.service.outdated_body")))
                .children(versions(&self.app, &self.version, cx))
        };
        let update = {
            let setup = setup.clone();
            ui::button(
                "service-update",
                if self.stopped { t("ui.service.start") } else { t("ui.service.update") },
                ButtonKind::Strong,
            )
            .icon(icon(IconName::Shield))
            .loading(busy)
            .on_click(move |_, _, cx| ServiceSetup::run(&setup, ServiceAction::Update, false, cx).detach())
        };
        let remove = {
            let state = state.clone();
            ui::button("service-remove", t("ui.service.remove"), ButtonKind::Subtle).disabled(busy).on_click(
                move |_, window, cx| {
                    state.update(cx, |n, cx| {
                        n.removing = true;
                        cx.notify();
                    });
                    let weak = state.downgrade();
                    let on_end: Rc<dyn Fn(&mut App)> = Rc::new(move |cx| {
                        let _ = weak.update(cx, |n, cx| {
                            n.removing = false;
                            cx.notify();
                        });
                    });
                    ask(Ask::Remove, setup.clone(), on_end, window, cx);
                },
            )
        };
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(theme::S3)
            .child(head)
            .child(ui::inline().justify_center().child(update).child(remove))
            .when_some(error, |d, e| d.child(div().text_size(theme::FS_SMALL).text_color(p.bad_text).child(e)))
    }
}

/// The connection screen while the service is newer than the app: only
/// the app's update helps (setting the service up from here would take it
/// back to this older version for everyone)
pub fn newer_notice(version: &str, app: &str, cx: &App) -> impl IntoElement + use<> {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(theme::S3)
        .child(title(t("ui.service.newer_title")))
        .child(para(t("ui.service.newer_body")))
        .children(versions(app, version, cx))
        .child(
            ui::inline().justify_center().child(
                ui::button("open-download", t("ui.settings.open_download"), ButtonKind::Strong)
                    .icon(icon(IconName::Open))
                    .on_click(|_, _, cx| host::open_url(LINKS.releases, cx)),
            ),
        )
}
