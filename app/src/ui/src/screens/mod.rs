//! The screens of the main window, one module each, and the pieces around
//! them (status.rs: what the navigation and the top of every screen show).

pub mod about;
pub mod adapter;
pub mod auracast;
pub mod auracast_mixer;
pub mod chip_tools;
pub mod codecs;
pub mod developer;
pub mod devices;
pub mod diagnostics;
pub mod flyout;
pub mod hid_enable_dialog;
pub mod hid_host;
pub mod home;
pub mod mixer;
pub mod placeholder;
pub mod receiver_test;
pub mod service_addon;
pub mod settings;
pub mod setup;
pub mod status;
pub mod tws;

use gpui_kit::{AnyView, App, AppContext as _, Window};

use crate::app::RouteName;

/// Makes the screen of `name`
pub fn open(name: RouteName, window: &mut Window, cx: &mut App) -> AnyView {
    match name {
        RouteName::Settings => cx.new(|cx| settings::Settings::new(window, cx)).into(),
        RouteName::Adapter => cx.new(|cx| adapter::Adapter::new(window, cx)).into(),
        RouteName::Developer => cx.new(|cx| developer::Developer::new(window, cx)).into(),
        RouteName::About => cx.new(|cx| about::About::new(window, cx)).into(),
        RouteName::Mixer => cx.new(|cx| mixer::Mixer::new(window, cx)).into(),
        RouteName::Diagnostics => cx.new(|cx| diagnostics::Diagnostics::new(window, cx)).into(),
        RouteName::Home => cx.new(|cx| home::Home::new(window, cx)).into(),
        RouteName::Devices => cx.new(|cx| devices::Devices::new(window, cx)).into(),
        RouteName::Codecs => cx.new(|cx| codecs::Codecs::new(window, cx)).into(),
        _ => cx.new(|_| placeholder::Placeholder::new(name)).into(),
    }
}
