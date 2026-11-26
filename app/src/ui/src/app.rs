//! What every screen reaches: the core store, navigation and the transient
//! notice line.

use std::time::Duration;

use gpui_kit::{App, AppContext as _, AsyncApp, Context, Entity, Global, SharedString, Task, WeakEntity};
use ipc::{ErrorInfo, Method, Settings, SettingsApplied};
use serde::de::DeserializeOwned;
use ui_model::i18n::{self, t, tm};

use crate::core::Core;
use crate::theme::{self, ThemeSetting};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RouteName {
    Home,
    Codecs,
    Devices,
    Diagnostics,
    Mixer,
    Developer,
    Adapter,
    Settings,
    About,
    Setup,
}

impl RouteName {
    pub const ALL: [RouteName; 10] = [
        RouteName::Home,
        RouteName::Codecs,
        RouteName::Devices,
        RouteName::Diagnostics,
        RouteName::Mixer,
        RouteName::Developer,
        RouteName::Adapter,
        RouteName::Settings,
        RouteName::About,
        RouteName::Setup,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            RouteName::Home => "home",
            RouteName::Codecs => "codecs",
            RouteName::Devices => "devices",
            RouteName::Diagnostics => "diagnostics",
            RouteName::Mixer => "mixer",
            RouteName::Developer => "developer",
            RouteName::Adapter => "adapter",
            RouteName::Settings => "settings",
            RouteName::About => "about",
            RouteName::Setup => "setup",
        }
    }
}

/// adapter: its channel hopping section, or the firmware
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AdapterSection {
    Afh,
    Firmware,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Route {
    pub name: RouteName,
    pub arg: Option<AdapterSection>,
}

impl From<RouteName> for Route {
    fn from(name: RouteName) -> Route {
        Route { name, arg: None }
    }
}

/// "adapter/afh", "#/settings", "home" -> the route (unknown: Home)
pub fn parse_route(text: &str) -> Route {
    let text = text.trim_start_matches('#').trim_start_matches('/');
    let mut parts = text.split('/');
    let first = parts.next().unwrap_or("");
    // Profiles became Codecs (issue #86): its links still lead there
    let name = if first == "profiles" {
        RouteName::Codecs
    } else {
        RouteName::ALL.into_iter().find(|n| n.as_str() == first).unwrap_or(RouteName::Home)
    };
    let arg = match (name, parts.next()) {
        (RouteName::Adapter, Some("afh")) => Some(AdapterSection::Afh),
        (RouteName::Adapter, Some("firmware")) => Some(AdapterSection::Firmware),
        _ => None,
    };
    Route { name, arg }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    /// Done ("Copied")
    Info,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub text: SharedString,
    pub kind: NoticeKind,
}

/// The main window's view state shared by its screens
pub struct AppState {
    pub route: Route,
    pub notice: Option<Notice>,
    /// The first-run setup is on screen
    pub setup_active: bool,
    notice_timer: Option<Task<()>>,
}

impl AppState {
    pub fn new() -> AppState {
        AppState { route: RouteName::Home.into(), notice: None, setup_active: false, notice_timer: None }
    }

    pub fn navigate(&mut self, route: impl Into<Route>, cx: &mut Context<Self>) {
        let route = route.into();
        if self.route != route {
            self.route = route;
            cx.notify();
        }
    }

    pub fn set_setup_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.setup_active != active {
            self.setup_active = active;
            cx.notify();
        }
    }

    pub fn notify(&mut self, text: impl Into<SharedString>, kind: NoticeKind, cx: &mut Context<Self>) {
        self.notice = Some(Notice { text: text.into(), kind });
        let shown = if kind == NoticeKind::Error { Duration::from_millis(8000) } else { Duration::from_millis(3500) };
        self.notice_timer = Some(cx.spawn(async move |this: WeakEntity<AppState>, cx: &mut AsyncApp| {
            cx.background_executor().timer(shown).await;
            let _ = this.update(cx, |state, cx| {
                state.notice = None;
                cx.notify();
            });
        }));
        cx.notify();
    }
}

struct Globals {
    core: Entity<Core>,
    state: Entity<AppState>,
}

impl Global for Globals {}

pub fn init(core: Entity<Core>, cx: &mut App) {
    let state = cx.new(|_| AppState::new());
    follow_settings(&core, cx);
    cx.set_global(Globals { core, state });
}

/// Language and theme come from the core's settings, applied here once for
/// every window (the main window and the flyout alike)
fn follow_settings(core: &Entity<Core>, cx: &mut App) {
    let (mut language, mut theme_name) = (String::new(), String::new());
    cx.observe(core, move |core, cx| {
        let Some(s) = core.read(cx).snap().map(|s| &s.settings) else { return };
        let (new_language, new_theme) = (s.language.clone(), s.theme.clone());
        if new_language != language {
            language = new_language;
            if i18n::set_language(&language) {
                cx.refresh_windows();
            }
        }
        if new_theme != theme_name {
            theme_name = new_theme;
            theme::apply(ThemeSetting::parse(&theme_name), cx);
        }
    })
    .detach();
}

/// The core store
pub fn core(cx: &App) -> Entity<Core> {
    cx.global::<Globals>().core.clone()
}

pub fn try_core(cx: &App) -> Option<Entity<Core>> {
    cx.try_global::<Globals>().map(|g| g.core.clone())
}

/// Route, notice and whether the setup runs
pub fn state(cx: &App) -> Entity<AppState> {
    cx.global::<Globals>().state.clone()
}

pub fn navigate(route: impl Into<Route>, cx: &mut App) {
    let route = route.into();
    state(cx).update(cx, |s, cx| s.navigate(route, cx));
}

/// Shows the transient notice ("Copied", a button's error)
pub fn notify(text: impl Into<SharedString>, kind: NoticeKind, cx: &mut App) {
    let text = text.into();
    state(cx).update(cx, |s, cx| s.notify(text, kind, cx));
}

/// The text of an error from a request
pub fn error_text(e: &ErrorInfo) -> String {
    let text = tm(Some(&e.message));
    if text.is_empty() { t("status.error") } else { text }
}

/// Sends a request to the core -> its result
pub fn request<R: DeserializeOwned + Send + 'static>(method: Method, cx: &App) -> Task<Result<R, ErrorInfo>> {
    core(cx).read(cx).request(method, cx)
}

/// Runs a request for a button: shows the error as a notice -> the result,
/// None when it failed (already told)
pub fn run<R: 'static>(task: Task<Result<R, ErrorInfo>>, cx: &App) -> Task<Option<R>> {
    cx.spawn(async move |cx: &mut AsyncApp| match task.await {
        Ok(r) => Some(r),
        Err(e) => {
            cx.update(|cx| notify(error_text(&e), NoticeKind::Error, cx));
            None
        }
    })
}

/// Sends a request whose answer nobody waits for; an error shows as a notice
pub fn send(method: Method, cx: &App) {
    run(request::<serde_json::Value>(method, cx), cx).detach();
}

// ---- Settings ------------------------------------------------------------------

/// The Volume Mixer (screen, flyout list, output mute): Developer > Volume
/// Mixer on, in developer mode (issue #86)
pub fn mixer_shown(s: Option<&Settings>) -> bool {
    s.is_some_and(|s| s.developer_mode && s.volume_mixer)
}

/// The Output and Input Source lists offer every device (each output, the
/// 16-channel cable, the other side's cable): Developer > Show every audio
/// device on, in developer mode (issue #86: the lists read as chaos)
pub fn all_audio_devices(s: Option<&Settings>) -> bool {
    s.is_some_and(|s| s.developer_mode && s.all_audio_devices)
}

/// A device's "Set as Windows default output/input while connected" in
/// force: shown and as saved in developer mode, on for everyone else (issue
/// #86: apps without an output picker and the microphone need them; the
/// core does the same)
pub fn devices_switch_shown(s: Option<&Settings>) -> bool {
    s.is_some_and(|s| s.developer_mode)
}

/// A capture mode the default output switch applies to: a virtual or a
/// physical output device (issue #86)
pub fn switchable_source(mode: &str) -> bool {
    mode == "virtual" || mode == "output"
}

pub fn default_device_switch(s: Option<&Settings>, saved: Option<bool>) -> bool {
    saved != Some(false) || !devices_switch_shown(s)
}

/// Changes some settings: the core takes the whole object and sends it back
/// as an event -> what the core did besides saving (None: it failed,
/// already told)
pub fn apply_settings(patch: impl FnOnce(&mut Settings), cx: &App) -> Task<Option<SettingsApplied>> {
    let Some(mut settings) = core(cx).read(cx).snap().map(|s| s.settings.clone()) else {
        return Task::ready(None);
    };
    patch(&mut settings);
    let task = request::<Option<SettingsApplied>>(Method::SettingsSet { settings: Box::new(settings) }, cx);
    let task = run(task, cx);
    // An older core answers null
    cx.spawn(async move |_: &mut AsyncApp| {
        task.await.map(|applied| applied.unwrap_or(SettingsApplied { le_audio_deferred: false }))
    })
}

/// As [`apply_settings`], for a control that does not wait for the answer
pub fn save_settings(patch: impl FnOnce(&mut Settings), cx: &App) {
    apply_settings(patch, cx).detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes() {
        assert_eq!(parse_route("#/adapter/afh"), Route { name: RouteName::Adapter, arg: Some(AdapterSection::Afh) });
        assert_eq!(parse_route("settings"), RouteName::Settings.into());
        assert_eq!(parse_route("#/profiles"), RouteName::Codecs.into());
        assert_eq!(parse_route("nowhere"), RouteName::Home.into());
        assert_eq!(parse_route("home/afh"), RouteName::Home.into());
    }
}
