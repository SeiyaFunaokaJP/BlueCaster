//! The decisions behind the Volume Mixer: each app's name and mute button,
//! the requests, how often a dragged slider is sent, and how a refresh
//! keeps the rows.

use ipc::{Method, MixerApp};

use crate::i18n::t;

/// Sessions come and go without an event
pub const REFRESH_MS: u64 = 2000;
/// At most ten requests a second while dragging
pub const SEND_EVERY_MS: u64 = 100;
/// How long after the slider is let go a refresh may move it again
pub const RELEASE_MS: u64 = 600;

/// An app's name ("System sounds" for the system's own)
pub fn app_name(a: &MixerApp) -> String {
    if a.key == "*system" { t("ui.mixer.system") } else { a.name.clone() }
}

/// The speaker button's label: what a click does
pub fn mute_label(muted: bool) -> String {
    if muted { t("ui.volume.unmute") } else { t("ui.volume.mute") }
}

/// How long to wait before sending a volume (0: now), `since_last_ms` after
/// the one before
pub fn send_wait_ms(since_last_ms: u64) -> u64 {
    SEND_EVERY_MS.saturating_sub(since_last_ms)
}

/// The value the slider shows: the one under the pointer, which a refresh
/// does not move
pub fn shown_volume(held: Option<u8>, volume: u8) -> u8 {
    held.unwrap_or(volume)
}

/// audio.mixer_set with a volume
pub fn volume_request(key: &str, volume: u8) -> Method {
    Method::AudioMixerSet { app: key.to_owned(), volume: Some(volume), muted: None }
}

/// audio.mixer_set with a mute
pub fn mute_request(key: &str, muted: bool) -> Method {
    Method::AudioMixerSet { app: key.to_owned(), volume: None, muted: Some(muted) }
}

/// A refresh by key: for each new app, the index of its row before (None:
/// a new row). A row kept keeps its slider, so a refresh does not take the
/// thumb out of the pointer's hand.
pub fn reconcile(before: &[String], after: &[MixerApp]) -> Vec<Option<usize>> {
    after.iter().map(|a| before.iter().position(|k| *k == a.key)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(key: &str, name: &str) -> MixerApp {
        MixerApp { key: key.to_owned(), name: name.to_owned(), volume: 80, muted: false }
    }

    #[test]
    fn names_and_labels() {
        assert_eq!(app_name(&app("*system", "")), t("ui.mixer.system"));
        assert_eq!(app_name(&app("player.exe", "Music Player")), "Music Player");
        assert_eq!(mute_label(false), t("ui.volume.mute"));
        assert_eq!(mute_label(true), t("ui.volume.unmute"));
    }

    // Sets an app's volume and mute
    #[test]
    fn requests() {
        assert_eq!(
            volume_request("player.exe", 30),
            Method::AudioMixerSet { app: "player.exe".into(), volume: Some(30), muted: None }
        );
        assert_eq!(
            mute_request("player.exe", true),
            Method::AudioMixerSet { app: "player.exe".into(), volume: None, muted: Some(true) }
        );
    }

    // "keeps the slider in the hand through a move and a refresh"
    #[test]
    fn a_refresh_keeps_the_rows_and_the_held_value() {
        let before = vec!["*system".to_owned(), "player.exe".to_owned()];
        let after = [app("player.exe", "Music Player"), app("game.exe", "Game")];
        assert_eq!(reconcile(&before, &after), vec![Some(1), None]);
        assert_eq!(shown_volume(Some(31), 80), 31);
        assert_eq!(shown_volume(None, 80), 80);
    }

    #[test]
    fn at_most_ten_sends_a_second() {
        assert_eq!(send_wait_ms(0), 100);
        assert_eq!(send_wait_ms(40), 60);
        assert_eq!(send_wait_ms(100), 0);
        assert_eq!(send_wait_ms(5000), 0);
    }
}
