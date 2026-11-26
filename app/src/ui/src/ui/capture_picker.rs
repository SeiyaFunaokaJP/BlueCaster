//! The capture source choices of a device (its settings on Audio Devices):
//! System Loopback, a virtual device, a playing app, or one output device
//! whichever is the default (issue #193). The source is the device's: the
//! choice is saved into all the device's profiles (issue #86); a stream in
//! use takes it without reconnecting (profile.save in the core). Home chose
//! it too until issue #86 put the codec and the link latency there.
//!
//! The choices themselves (captureChoices, captureValue, withCapture) are in
//! ui_model::view::devices, with their tests.

use ipc::Profile;
use ui_model::i18n::t;

use super::IconName;

/// The profile's capture source, as the profile list and an idle Home show it
pub fn capture_text(p: &Profile) -> (IconName, String) {
    let or = |own: &str, key: &str| if own.is_empty() { t(key) } else { own.to_owned() };
    match p.capture_mode.as_str() {
        "virtual" => (IconName::DesktopSpeaker, or(&p.audio_device_name, "capture.virtual")),
        "output" => (IconName::Speaker2, or(&p.audio_device_name, "capture.output")),
        "app" => {
            let name = if p.app_name.is_empty() { &p.app_exe } else { &p.app_name };
            (IconName::WindowApps, or(name, "capture.app"))
        }
        _ => (IconName::Speaker2, t("capture.loopback")),
    }
}
