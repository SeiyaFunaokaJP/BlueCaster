//! An LE Audio device's channels on Audio Devices: left and right swapped,
//! and, for a member of a set, what an earbud plays while the other one is
//! gone (Auto: its own side, as planned; Left; Right; Mono, the default). Kept in its
//! profile (`le_swap_channels`, `le_alone_channel`); a stream of it takes a
//! change in place, from the next frame.

use gpui_kit::{App, Div, IntoElement as _, ParentElement as _, Styled as _, div};
use ipc::{Device, Profile};
use ui_model::i18n::t;
use ui_model::profile::{AudioEntry, audio_entries, device_key};

use super::Choice;
use crate::app;
use crate::screens::codecs::save_entry;
use crate::theme;

/// What an earbud left alone plays, as `le_alone_channel` keeps it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AloneChannel {
    Auto,
    Left,
    Right,
    Mono,
}

impl AloneChannel {
    pub const ALL: [AloneChannel; 4] =
        [AloneChannel::Auto, AloneChannel::Left, AloneChannel::Right, AloneChannel::Mono];

    /// The profile's choice: "" and anything unknown are mono, as the core
    /// reads them
    pub fn of(p: Option<&Profile>) -> AloneChannel {
        match p.map(|p| p.le_alone_channel.as_str()) {
            Some("auto") => AloneChannel::Auto,
            Some("left") => AloneChannel::Left,
            Some("right") => AloneChannel::Right,
            _ => AloneChannel::Mono,
        }
    }

    /// As saved: mono, the default, as none
    pub fn saved(self) -> &'static str {
        match self {
            AloneChannel::Auto => "auto",
            AloneChannel::Left => "left",
            AloneChannel::Right => "right",
            AloneChannel::Mono => "",
        }
    }

    fn label(self) -> String {
        t(match self {
            AloneChannel::Auto => "ui.profile.le_alone_auto",
            AloneChannel::Left => "ui.profile.le_alone_left",
            AloneChannel::Right => "ui.profile.le_alone_right",
            AloneChannel::Mono => "ui.profile.le_alone_mono",
        })
    }
}

fn entry_of(key: &str, cx: &App) -> Option<AudioEntry> {
    let snap = app::core(cx).read(cx).snap()?;
    audio_entries(snap).into_iter().find(|e| e.key == key)
}

/// Saves the change into the device's profile (made when it has none yet)
fn save(key: &str, edit: impl FnOnce(&mut Profile) + 'static, cx: &mut App) {
    if let Some(e) = entry_of(key, cx) {
        save_entry(&e, edit, cx).detach();
    }
}

/// A member of a coordinated set: seen as one in a scan, or the other
/// members learned at a connection
fn in_set(device: &Device) -> bool {
    device.le_audio_set || !device.set_addresses.is_empty()
}

/// The settings of an LE Audio device (what an earbud alone plays only for
/// a member of a set); None for a classic one
pub fn le_channels(device: &Device, cx: &App) -> Option<Div> {
    if !device.le_audio {
        return None;
    }
    let key = device_key(&device.address, device.le_audio);
    let profile = entry_of(&key, cx).and_then(|e| e.profile);
    let swapped = profile.as_ref().is_some_and(|p| p.le_swap_channels);
    let alone = AloneChannel::of(profile.as_ref());
    let swap = {
        let key = key.clone();
        super::toggle(
            "le-swap-channels",
            t("ui.profile.le_swap_channels"),
            None,
            swapped,
            false,
            move |v: &bool, _, cx| {
                let v = *v;
                save(&key, move |p| p.le_swap_channels = v, cx);
            },
            cx,
        )
    };
    let choices = AloneChannel::ALL.iter().map(|&c| Choice::new(c, c.label())).collect();
    let alone_select = super::select_labelled(
        "le-alone-channel",
        t("ui.profile.le_alone_channel"),
        &alone,
        choices,
        false,
        move |v: &AloneChannel, _, cx| {
            let v = *v;
            save(&key, move |p| p.le_alone_channel = v.saved().to_owned(), cx);
        },
    );
    // Only a set has an earbud that can be left alone
    let alone_field = in_set(device).then(|| {
        super::field(
            t("ui.profile.le_alone_channel"),
            Some(t("ui.profile.le_alone_channel_hint").into()),
            None,
            div().min_w(gpui_kit::px(200.)).child(alone_select.into_any_element()),
            cx,
        )
    });
    Some(div().flex().flex_col().gap(theme::S3).child(swap).children(alone_field))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_or_empty_choice_is_mono_and_mono_is_saved_as_none() {
        let mut p = ui_model::profile::new_profile();
        assert_eq!(AloneChannel::of(Some(&p)), AloneChannel::Mono);
        assert_eq!(AloneChannel::of(None), AloneChannel::Mono);
        for c in AloneChannel::ALL {
            p.le_alone_channel = c.saved().to_owned();
            assert_eq!(AloneChannel::of(Some(&p)), c);
        }
        p.le_alone_channel = "both".into();
        assert_eq!(AloneChannel::of(Some(&p)), AloneChannel::Mono);
        assert_eq!(AloneChannel::Mono.saved(), "");
    }

    const ADDR: &str = "0A:00:00:00:00:02";

    #[test]
    fn only_a_set_member_can_be_left_alone() {
        let mut d = le_device(true);
        assert!(!in_set(&d));
        d.set_addresses = vec!["0A:00:00:00:00:03".into()];
        assert!(in_set(&d));
        d.set_addresses.clear();
        d.le_audio_set = true;
        assert!(in_set(&d));
    }

    fn le_device(le_audio: bool) -> Device {
        serde_json::from_value(serde_json::json!({
            "address": ADDR, "name": "Test Buds", "saved": true, "audio": true, "le_audio": le_audio
        }))
        .unwrap()
    }

    #[gpui_kit::test]
    fn a_change_is_saved_into_the_le_devices_profile(cx: &mut gpui_kit::TestAppContext) {
        use crate::testing::{Harness, device, profile, snapshot};
        let mut s = snapshot();
        // LE Audio devices are listed with LE Audio on (developer mode)
        s["settings"]["developer_mode"] = serde_json::json!(true);
        s["settings"]["le_audio_experimental"] = serde_json::json!(true);
        let mut d = device(ADDR, "Test Buds");
        d["le_audio"] = serde_json::json!(true);
        s["devices"] = serde_json::json!([d]);
        s["profiles"] =
            serde_json::json!([profile("Test Buds", ADDR, "Test Buds", serde_json::json!({"le_audio": true}))]);
        let mut h = Harness::start(cx, s);
        let key = device_key(ADDR, true);
        cx.update(|cx| {
            assert!(le_channels(&le_device(true), cx).is_some());
            assert!(le_channels(&le_device(false), cx).is_none());
            save(&key, |p| p.le_swap_channels = true, cx);
        });
        h.settle(cx);
        let saved = h.last_saved();
        assert_eq!(saved["le_swap_channels"], serde_json::json!(true));
        assert_eq!(saved["le_alone_channel"], serde_json::json!(""));
    }
}
