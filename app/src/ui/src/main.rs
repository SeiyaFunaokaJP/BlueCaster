//! bluecaster.exe: the BlueCaster window process. It shows the state of
//! bluecaster-core.exe and sends it the user's requests; the core keeps
//! streaming when this process ends.
//!
//! Arguments: `--flyout` opens the tray flyout instead of the main window
//! (the core starts the UI this way when the tray icon is clicked);
//! `--quick-settings` opens it above Quick Settings, which just opened.

#![forbid(unsafe_code)]
// No console window in release builds
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod core;
mod demo;
mod flyout_window;
mod host;
mod screens;
mod shell;
#[cfg(test)]
mod testing;
mod theme;
mod ui;
mod ui_state;
mod windows;

use std::sync::Arc;

use gpui_kit::{App, AppContext as _, AsyncApp, KeyBinding, QuitMode, actions};
use ipc::ServerMessage;
use ui_host::HostEvent;
use ui_host::single_instance::{self, Instance, Launch};

use crate::app::RouteName;
use crate::core::{Core, Transport};

actions!(bluecaster, [OpenDiagnostics]);

fn main() {
    // The settings folder of 2.0.x is moved by whichever process comes
    // first, but not under a 2.0.x core or v1 that still writes to it (the
    // core stops them and moves it then)
    ui_host::logging::set_old_app_check();
    let log = ui_host::logging::init();
    let launch = Launch::from_args(std::env::args().skip(1));
    let demo = demo::from_env();
    // A second launch (from the tray or the Start menu) only forwards its
    // arguments to the first and ends before anything else starts
    let mut primary = None;
    if demo.is_none() {
        match single_instance::acquire_with(&single_instance::identifier(), launch) {
            Instance::Secondary => return,
            Instance::Primary(p) => primary = Some(p),
        }
    }
    tracing::info!("bluecaster {} starting ({launch:?})", host::VERSION);
    if primary.is_some() {
        ui_host::webview_data::remove_in_background(&single_instance::identifier());
    }
    let launches = primary.as_mut().and_then(|p| p.launches());

    gpui_kit::application().with_assets(gpui_kit::assets::Assets).with_quit_mode(QuitMode::Explicit).run(move |cx| {
        // The AppUserModelID of the installer's shortcuts, before any window:
        // the taskbar groups the windows under the pinned shortcut
        cx.set_app_identity(&single_instance::identifier(), "BlueCaster");
        gpui_kit::init(cx);
        let (transport, backend, events): (Arc<dyn Transport>, _, _) = match demo {
            Some(demo) => (demo, None, None),
            None => {
                let (backend, events) = ui_host::start();
                (Arc::new(host::Connection(backend.clone())), Some(backend), Some(events))
            }
        };
        let status = transport.status();
        let core = cx.new(|_| Core::new(transport));
        app::init(core.clone(), cx);
        host::init(backend, cx);
        theme::apply(theme::ThemeSetting::System, cx);
        core.update(cx, |core, cx| core.on_status(status, cx));

        if let Some(mut events) = events {
            cx.spawn(async move |cx: &mut AsyncApp| {
                while let Some(event) = events.recv().await {
                    cx.update(|cx| on_host_event(event, cx));
                }
            })
            .detach();
        }
        if let Some(mut launches) = launches {
            cx.spawn(async move |cx: &mut AsyncApp| {
                while let Some(launch) = launches.recv().await {
                    cx.update(|cx| on_launch(launch, cx));
                }
            })
            .detach();
        }

        bind_shortcuts(cx);

        if let Some(route) = demo::route() {
            // Demo only: "flyout" shows the tray flyout instead of the main window
            if route == "flyout" {
                flyout_window::show(cx);
            } else {
                if route == "setup" {
                    app::state(cx).update(cx, |s, cx| s.set_setup_active(true, cx));
                }
                app::navigate(app::parse_route(&route), cx);
                windows::show_main(None, false, cx);
            }
        } else {
            on_launch(launch, cx);
        }
        windows::start_idle_exit(cx);
    });

    tracing::info!("bluecaster exiting");
    // Before a relaunch: the next process must become the first one
    drop(primary);
    ui_host::recovery::relaunch_if_due();
    drop(log);
}

/// Ctrl+Shift+D opens the diagnostics (v1's Debug Console shortcut), not
/// while the first-run setup shows
fn bind_shortcuts(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("ctrl-shift-d", OpenDiagnostics, None)]);
    cx.on_action(|_: &OpenDiagnostics, cx| {
        if !app::state(cx).read(cx).setup_active {
            app::navigate(RouteName::Diagnostics, cx);
        }
    });
}

fn on_host_event(event: HostEvent, cx: &mut App) {
    match event {
        HostEvent::Message(value) => match serde_json::from_value::<ServerMessage>(value) {
            Ok(message) => app::core(cx).update(cx, |core, cx| core.on_message(message, cx)),
            // A message of a newer core: a state event among them leaves a
            // gap in the revisions, and the snapshot asked for then has it
            Err(e) => tracing::debug!("a message of the core was not read: {e}"),
        },
        HostEvent::Status(status) => app::core(cx).update(cx, |core, cx| core.on_status(status.into(), cx)),
        HostEvent::UpdatePhase(phase) => host::on_update_phase(phase, cx),
        HostEvent::Exit => cx.quit(),
    }
}

/// This launch, or a second one's arguments
fn on_launch(launch: Launch, cx: &mut App) {
    match launch {
        Launch::Main => windows::show_main(None, true, cx),
        Launch::Flyout => flyout_window::toggle(cx),
        Launch::QuickSettings => flyout_window::show_with_quick_settings(cx),
    }
}
