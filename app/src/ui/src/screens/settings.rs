//! Settings: the general ones only: language, theme, startup, notifications,
//! updates, developer mode, the settings folder. (A device's buttons and
//! remembered volume are on Audio Devices; LE Audio and its GMAP low latency
//! are chosen per profile.)

use gpui_kit::component::Disableable as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AsyncApp, Context, Entity, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Window, div,
};
use ipc::{Method, Msg, Settings as CoreSettings, Snapshot, UpdateState};
use ui_model::i18n::{LANGUAGES, t, tm};
use ui_model::links::LINKS;
use ui_model::t;

use crate::app::{self, NoticeKind};
use crate::core::Core;
use crate::host::{self, HostState, UpdatePhase, UpdateUnavailable};
use crate::theme;
use crate::ui::dialog::{self, Question};
use crate::ui::{self, BannerKind, ButtonKind, Choice, IconName, icon};

/// The theme, here and on the setup's last step
pub fn theme_choice(settings: &CoreSettings, cx: &App) -> gpui_kit::Div {
    let choices = vec![
        Choice::new("system".to_owned(), t("settings.theme_system")),
        Choice::new("light".to_owned(), t("settings.theme_light")),
        Choice::new("dark".to_owned(), t("settings.theme_dark")),
    ];
    ui::setting_row(cx).child(div().child(t("settings.theme"))).child(ui::segmented(
        "theme",
        &settings.theme,
        choices,
        |v: &String, _, cx| {
            let v = v.clone();
            app::save_settings(move |s| s.theme = v, cx);
        },
    ))
}

/// When the low battery notice comes, here and on the setup's last step
pub fn battery_low_choice(settings: &CoreSettings, cx: &App) -> impl IntoElement + use<> {
    let mut choices: Vec<Choice<u8>> =
        [20u8, 10, 5].into_iter().map(|p| Choice::new(p, t!("ui.settings.battery_low_below", p))).collect();
    choices.push(Choice::new(0, t("ui.settings.battery_low_never")));
    ui::field(
        t("ui.settings.battery_low"),
        Some(t("ui.settings.battery_low_hint").into()),
        None,
        ui::select_labelled(
            "battery-low",
            t("ui.settings.battery_low"),
            &settings.battery_low_percent,
            choices,
            false,
            |v: &u8, _, cx| {
                let v = *v;
                app::save_settings(move |s| s.battery_low_percent = v, cx);
            },
        ),
        cx,
    )
}

/// Where the flyout opens from
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlyoutFrom {
    None,
    Tray,
    Quick,
}

pub struct Settings {
    core: Entity<Core>,
    host: Entity<HostState>,
}

impl Settings {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Settings {
        let core = app::core(cx);
        let host = host::state(cx);
        cx.observe(&core, |_, _, cx| cx.notify()).detach();
        cx.observe(&host, |_, _, cx| cx.notify()).detach();
        Settings { core, host }
    }

    fn check_now(&mut self, cx: &mut Context<Self>) {
        let task = app::request::<UpdateState>(Method::AppCheckUpdate, cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| match task.await {
            Ok(u) if u.available.is_none() => {
                cx.update(|cx| app::notify(t!("update.uptodate", &u.current_version), NoticeKind::Info, cx));
            }
            Ok(_) => {}
            // A failed check stays in the section (UpdateState.error) with the
            // link to the releases page; other failures (no core) are a notice
            Err(e) if e.code != "update.failed" => {
                cx.update(|cx| app::notify(app::error_text(&e), NoticeKind::Error, cx));
            }
            Err(_) => {}
        })
        .detach();
    }

    fn quit(window: &mut Window, cx: &mut App) {
        dialog::ask(
            Question::new(t("ui.settings.quit")).confirm(t("ui.settings.quit")).danger(),
            |_, _| div().child(t("ui.settings.quit_confirm")).into_any_element(),
            |_, cx| {
                let quit = app::request::<serde_json::Value>(Method::AppQuit, cx);
                cx.spawn(async move |cx: &mut AsyncApp| {
                    let Err(e) = quit.await else { return };
                    // A core that runs as a service stays: app.close closes
                    // the window and the tray icon instead
                    cx.update(|cx| {
                        if e.code == "ipc.not_in_service" {
                            app::send(Method::AppClose, cx);
                        } else {
                            app::notify(app::error_text(&e), NoticeKind::Error, cx);
                        }
                    });
                })
                .detach();
            },
            window,
            cx,
        );
    }

    fn appearance(&self, s: &CoreSettings, cx: &App) -> impl IntoElement + use<> {
        let language =
            if LANGUAGES.iter().any(|(code, _)| *code == s.language) { s.language.clone() } else { "en".into() };
        let languages = LANGUAGES.iter().map(|(code, name)| Choice::new((*code).to_owned(), *name)).collect();
        ui::section(t("ui.settings.appearance"), None, cx)
            .child(ui::field(
                t("settings.language"),
                None,
                None,
                ui::select("language", &language, languages, false, |v: &String, _, cx| {
                    let v = v.clone();
                    app::save_settings(move |s| s.language = v, cx);
                }),
                cx,
            ))
            .child(theme_choice(s, cx))
    }

    fn startup(&self, snap: &Snapshot, cx: &App) -> impl IntoElement + use<> {
        let s = &snap.settings;
        let from = if s.flyout_with_quick_settings {
            FlyoutFrom::Quick
        } else if s.tray_click_opens_window {
            FlyoutFrom::None
        } else {
            FlyoutFrom::Tray
        };
        let hint: Vec<String> = [
            (from != FlyoutFrom::Tray).then(|| t("ui.settings.flyout_from_hint")),
            (!host::mica()).then(|| t("ui.settings.flyout_from_win11")),
        ]
        .into_iter()
        .flatten()
        .collect();
        let hint = (!hint.is_empty()).then(|| SharedString::from(hint.join(" ")));
        ui::section(t("ui.settings.startup"), None, cx)
            // The service add-on starts with Windows whatever this says: on
            // and not changeable while connected to it
            .child(ui::toggle(
                "start-with-windows",
                t("settings.start_with_windows"),
                None,
                s.start_with_windows || snap.service_mode,
                snap.service_mode,
                |v, _, cx| {
                    let v = *v;
                    app::save_settings(move |s| s.start_with_windows = v, cx);
                },
                cx,
            ))
            // At logon: on starts BlueCaster with --background (only the tray
            // icon), off opens the window; it applies only with Windows
            .child(ui::toggle(
                "start-minimized",
                t("ui.settings.start_minimized"),
                Some(t("ui.settings.start_minimized_logon").into()),
                s.start_minimized,
                !s.start_with_windows || snap.service_mode,
                |v, _, cx| {
                    let v = *v;
                    app::save_settings(move |s| s.start_minimized = v, cx);
                },
                cx,
            ))
            .child(ui::toggle(
                "minimize-to-tray",
                t("settings.minimize_to_tray"),
                None,
                s.minimize_to_tray,
                false,
                |v, _, cx| {
                    let v = *v;
                    app::save_settings(move |s| s.minimize_to_tray = v, cx);
                },
                cx,
            ))
            // Off the tray icon, the icon's click opens the main window.
            // Quick Settings are Windows 11's, as Mica is
            .child(ui::field(
                t("ui.settings.flyout_from"),
                hint,
                None,
                ui::select(
                    "flyout-from",
                    &from,
                    vec![
                        Choice::new(FlyoutFrom::None, t("ui.settings.flyout_from_none")),
                        Choice::new(FlyoutFrom::Tray, t("ui.settings.flyout_from_tray")),
                        Choice::new(FlyoutFrom::Quick, t("ui.settings.flyout_from_quick")).disabled(!host::mica()),
                    ],
                    false,
                    |v: &FlyoutFrom, _, cx| {
                        let v = *v;
                        app::save_settings(
                            move |s| {
                                s.tray_click_opens_window = v != FlyoutFrom::Tray;
                                s.flyout_with_quick_settings = v == FlyoutFrom::Quick;
                            },
                            cx,
                        );
                    },
                ),
                cx,
            ))
    }

    fn notifications(&self, s: &CoreSettings, cx: &App) -> impl IntoElement + use<> {
        ui::section(t("ui.settings.notifications"), None, cx)
            .child(ui::toggle(
                "notify-connected",
                t("ui.settings.notify_connected"),
                None,
                s.notify_connected,
                false,
                |v, _, cx| {
                    let v = *v;
                    app::save_settings(move |s| s.notify_connected = v, cx);
                },
                cx,
            ))
            .child(battery_low_choice(s, cx))
    }

    fn updates(&self, snap: &Snapshot, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let (s, update) = (&snap.settings, &snap.update);
        let phase = self.host.read(cx).update.clone();
        let check = ui::button("check-update", t("update.check"), ButtonKind::Default)
            .icon(icon(IconName::ArrowSync))
            .loading(update.checking)
            .disabled(update.checking)
            .on_click(cx.listener(|this, _, _, cx| this.check_now(cx)));
        let failed = (!update.checking).then_some(update.error.as_ref()).flatten();
        ui::section(t("ui.settings.updates"), Some(check.into_any_element()), cx)
            .when_some(failed, |d, message| d.child(update_check_failed(message, cx)))
            .when_some(update.available.as_ref(), |d, u| {
                let can_install = matches!(phase, UpdatePhase::Idle | UpdatePhase::Failed { .. });
                let unavailable = matches!(phase, UpdatePhase::Unavailable { .. });
                let (manifest, url) = (u.manifest_url.clone(), u.url.clone());
                let mb = |bytes: u64| format!("{:.1}", bytes as f64 / 1_048_576.);
                let progress = match &phase {
                    UpdatePhase::Unavailable { reason: UpdateUnavailable::Portable } => t("ui.update.portable"),
                    UpdatePhase::Unavailable { reason: UpdateUnavailable::Unsigned } => t("ui.update.unsigned"),
                    UpdatePhase::Idle => String::new(),
                    UpdatePhase::Downloading { version, downloaded, total } => {
                        let version = if version.is_empty() { &u.version } else { version };
                        let size = match total {
                            Some(total) => format!("{} / {} MB", mb(*downloaded), mb(*total)),
                            None => format!("{} MB", mb(*downloaded)),
                        };
                        t!("ui.update.downloading", version, size)
                    }
                    UpdatePhase::Waiting { version } => t!("ui.update.waiting", version),
                    UpdatePhase::Installing { version } => t!("ui.update.installing", version),
                    UpdatePhase::Failed { code, .. } => t!("ui.update.failed", code),
                };
                let body = div()
                    .flex()
                    .flex_col()
                    .gap(theme::S1)
                    .child(t!("ui.settings.update_available", &u.version, &update.current_version))
                    .when(!progress.is_empty(), |d| d.child(ui::hint(progress, cx)));
                let actions = ui::inline()
                    .when(can_install, |d| {
                        d.child(
                            ui::button("install-update", t("ui.update.install"), ButtonKind::Strong)
                                .icon(icon(IconName::ArrowSync))
                                .on_click(move |_, _, cx| host::update_install(&manifest, cx)),
                        )
                    })
                    .child(
                        ui::button(
                            "open-download",
                            t("ui.settings.open_download"),
                            if unavailable { ButtonKind::Strong } else { ButtonKind::Default },
                        )
                        .icon(icon(IconName::Open))
                        .on_click(move |_, _, cx| host::open_url(&url, cx)),
                    );
                d.child(ui::banner(
                    BannerKind::Info,
                    Some(t("update.available_title").into()),
                    Some(body.into_any_element()),
                    Some(actions.into_any_element()),
                    None,
                    cx,
                ))
            })
            .child(ui::toggle(
                "check-updates",
                t("settings.check_updates_on_startup"),
                None,
                s.check_updates_on_startup,
                false,
                |v, _, cx| {
                    let v = *v;
                    app::save_settings(move |s| s.check_updates_on_startup = v, cx);
                },
                cx,
            ))
            .child(ui::toggle(
                "include-prereleases",
                t("settings.include_prereleases"),
                None,
                s.include_prereleases,
                false,
                |v, _, cx| {
                    let v = *v;
                    app::save_settings(move |s| s.include_prereleases = v, cx);
                },
                cx,
            ))
    }

    fn advanced(&self, snap: &Snapshot, cx: &App) -> impl IntoElement + use<> {
        let config_dir = snap.config_dir.clone();
        ui::section(t("ui.settings.advanced"), None, cx)
            .child(ui::toggle(
                "developer-mode",
                t("ui.settings.developer_mode"),
                Some(t("ui.settings.developer_mode_hint").into()),
                snap.settings.developer_mode,
                false,
                |v, _, cx| {
                    let v = *v;
                    app::save_settings(move |s| s.developer_mode = v, cx);
                },
                cx,
            ))
            // A core that runs as a service keeps its folder protected
            .when(!snap.service_mode, |d| {
                d.child(
                    ui::setting_row(cx)
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .child(t("ui.settings.config_folder"))
                                .child(ui::hint(config_dir.clone(), cx).font_family(theme::FONT_MONO)),
                        )
                        .child(
                            ui::button("open-config", t("ui.settings.open_config_folder"), ButtonKind::Default)
                                .icon(icon(IconName::FolderOpen))
                                .on_click(move |_, _, cx| host::open_folder(&config_dir, cx)),
                        ),
                )
            })
            .child(
                ui::setting_row(cx).child(div().child(t("ui.setup.title"))).child(
                    ui::button("run-setup", t("ui.settings.run_setup"), ButtonKind::Default)
                        .icon(icon(IconName::Play))
                        .on_click(|_, _, cx| app::state(cx).update(cx, |s, cx| s.set_setup_active(true, cx))),
                ),
            )
    }
}

impl Render for Settings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(snap) = self.core.read(cx).snap().cloned() else { return div().into_any_element() };
        ui::page("settings", t("settings.header"))
            .child(self.appearance(&snap.settings, cx))
            .child(self.startup(&snap, cx))
            .child(self.notifications(&snap.settings, cx))
            .child(self.updates(&snap, cx))
            .child(self.advanced(&snap, cx))
            .child(
                div().flex().child(
                    ui::button("quit", t("ui.settings.quit"), ButtonKind::Danger)
                        .icon(icon(IconName::Power))
                        .on_click(|_, window, cx| Settings::quit(window, cx)),
                ),
            )
            .into_any_element()
    }
}

/// A failed update check (update.error) and the way out: the releases page
pub fn update_check_failed(message: &Msg, cx: &App) -> impl IntoElement + use<> {
    let text = tm(Some(message));
    let text = if text.is_empty() { t("update.error") } else { text };
    let body: AnyElement = div().child(text).into_any_element();
    let open = ui::button("open-releases", t("ui.update.open_releases"), ButtonKind::Default)
        .icon(icon(IconName::Open))
        .on_click(|_, _, cx| host::open_url(LINKS.releases, cx));
    ui::banner(BannerKind::Error, Some(t("update.check").into()), Some(body), Some(open.into_any_element()), None, cx)
}
