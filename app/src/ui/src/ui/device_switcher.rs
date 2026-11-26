//! Choosing the device for the big button (the flyout), one entry per audio
//! device (its saved settings, internally a Profile, since the Codecs screen,
//! issue #86). While a stream runs, choosing another device streams to that
//! one (stream.start stops the running one); otherwise it becomes the active one.

use gpui_kit::{App, Div, ParentElement as _, Styled as _, div};
use ipc::{Method, StreamState};
use ui_model::i18n::t;
use ui_model::view::devices::switcher_choices;
use ui_model::view::stream::current_profile;

use super::{Choice, select_labelled};
use crate::app;

/// The flyout's device list (`.device-switcher`: the list takes the width)
pub fn device_switcher(cx: &App) -> Div {
    let core = app::core(cx);
    let Some(snap) = core.read(cx).snap() else { return div() };
    let value = current_profile(snap).map(|p| p.name.clone()).unwrap_or_default();
    let choices = switcher_choices(snap).into_iter().map(|(name, label)| Choice::new(name, label)).collect();
    div().flex().flex_col().child(select_labelled(
        "device-switcher",
        t("ui.home.device"),
        &value,
        choices,
        false,
        |name: &String, _, cx| {
            let Some(snap) = app::core(cx).read(cx).snap() else { return };
            let running = matches!(snap.stream.state, StreamState::Streaming | StreamState::Connecting);
            let method = if running {
                Method::StreamStart { profile: name.clone() }
            } else {
                Method::ProfileSetActive { name: name.clone() }
            };
            app::send(method, cx);
        },
    ))
}
