//! A hearing aid for the other ear, on a classic device's card on Audio
//! Devices (ASHA, a proof of concept): one of the saved hearing aids plays
//! its own side beside this device's stream, and how much later it plays,
//! so that both ears hear the sound together. Kept in the device's profile
//! (`hearing_aid_beside`, `hearing_aid_delay_ms`); shown while LE Audio is
//! on and a hearing aid is saved (or one is chosen).

use gpui_kit::{App, Div, IntoElement as _, ParentElement as _, Styled as _, div};
use ipc::{Device, Profile};
use ui_model::i18n::t;
use ui_model::profile::{AudioEntry, audio_entries, device_key, saved_hearing_aids};
use ui_model::view::devices::device_label;

use super::Choice;
use crate::app;
use crate::screens::codecs::save_entry;
use crate::theme;

/// The delays offered, in milliseconds (the core takes 0 to 500)
pub const DELAYS: [u32; 12] = [0, 20, 40, 60, 80, 100, 120, 150, 200, 300, 400, 500];

/// The delay offered that is nearest to a saved one
pub fn nearest_delay(ms: u32) -> u32 {
    DELAYS.iter().copied().min_by_key(|d| d.abs_diff(ms)).unwrap_or(0)
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

/// The hearing aid of a classic device; None for an LE entry, with LE Audio
/// off, or while no hearing aid is saved and none is chosen
pub fn beside(device: &Device, cx: &App) -> Option<Div> {
    if device.le_audio {
        return None;
    }
    let snap = app::core(cx).read(cx).snap()?;
    let key = device_key(&device.address, false);
    let profile = entry_of(&key, cx).and_then(|e| e.profile);
    let chosen = profile.as_ref().map(|p| p.hearing_aid_beside.to_uppercase()).unwrap_or_default();
    let aids = saved_hearing_aids(snap);
    if aids.is_empty() && chosen.is_empty() {
        return None;
    }
    let mut choices = vec![Choice::new(String::new(), t("ui.profile.hearing_aid_none"))];
    choices.extend(aids.iter().map(|d| Choice::new(d.address.to_uppercase(), device_label(d))));
    // A chosen one that is no longer saved stays visible, to be taken off
    if !chosen.is_empty() && !aids.iter().any(|d| d.address.eq_ignore_ascii_case(&chosen)) {
        choices.push(Choice::new(chosen.clone(), chosen.clone()));
    }
    let aid_select = {
        let key = key.clone();
        super::select_labelled(
            "hearing-aid-beside",
            t("ui.profile.hearing_aid_beside"),
            &chosen,
            choices,
            false,
            move |v: &String, _, cx| {
                let v = v.clone();
                save(&key, move |p| p.hearing_aid_beside = v, cx);
            },
        )
    };
    let aid_field = super::field(
        t("ui.profile.hearing_aid_beside"),
        Some(t("ui.profile.hearing_aid_beside_hint").into()),
        None,
        div().min_w(gpui_kit::px(200.)).child(aid_select.into_any_element()),
        cx,
    );
    let delay_field = (!chosen.is_empty()).then(|| {
        let delay = nearest_delay(profile.as_ref().map_or(0, |p| p.hearing_aid_delay_ms));
        let choices = DELAYS.iter().map(|&ms| Choice::new(ms, format!("{ms} ms"))).collect();
        let select = super::select_labelled(
            "hearing-aid-delay",
            t("ui.profile.hearing_aid_delay"),
            &delay,
            choices,
            false,
            move |v: &u32, _, cx| {
                let v = *v;
                save(&key, move |p| p.hearing_aid_delay_ms = v, cx);
            },
        );
        super::field(
            t("ui.profile.hearing_aid_delay"),
            Some(t("ui.profile.hearing_aid_delay_hint").into()),
            None,
            div().min_w(gpui_kit::px(200.)).child(select.into_any_element()),
            cx,
        )
    });
    Some(div().flex().flex_col().gap(theme::S3).child(aid_field).children(delay_field))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLASSIC: &str = "0A:00:00:00:00:02";
    const AID: &str = "0A:00:00:00:00:22";

    fn classic() -> Device {
        serde_json::from_value(
            serde_json::json!({"address": CLASSIC, "name": "Headphones", "saved": true, "audio": true}),
        )
        .unwrap()
    }

    #[gpui_kit::test]
    fn a_saved_hearing_aid_can_be_put_beside_a_classic_device(cx: &mut gpui_kit::TestAppContext) {
        use crate::testing::{Harness, device, profile, snapshot};
        let mut s = snapshot();
        s["devices"] = serde_json::json!([device(CLASSIC, "Headphones")]);
        s["profiles"] = serde_json::json!([profile("Headphones", CLASSIC, "Headphones", serde_json::json!({}))]);
        let mut h = Harness::start(cx, s.clone());
        // No hearing aid saved: nothing to choose
        cx.update(|cx| assert!(beside(&classic(), cx).is_none()));
        drop(h);
        let mut aid = device(AID, "Aid");
        aid["le_audio"] = serde_json::json!(true);
        aid["hearing_aid"] = serde_json::json!(true);
        s["devices"] = serde_json::json!([device(CLASSIC, "Headphones"), aid.clone()]);
        // With LE Audio off the saved hearing aid is not offered either
        h = Harness::start(cx, s.clone());
        cx.update(|cx| assert!(beside(&classic(), cx).is_none()));
        drop(h);
        s["settings"]["developer_mode"] = serde_json::json!(true);
        s["settings"]["le_audio_experimental"] = serde_json::json!(true);
        h = Harness::start(cx, s);
        let key = device_key(CLASSIC, false);
        cx.update(|cx| {
            assert!(beside(&classic(), cx).is_some());
            // Never on the hearing aid's own card
            let aid: Device = serde_json::from_value(aid.clone()).unwrap();
            assert!(beside(&aid, cx).is_none());
            save(
                &key,
                |p| {
                    p.hearing_aid_beside = AID.to_owned();
                    p.hearing_aid_delay_ms = 60;
                },
                cx,
            );
        });
        h.settle(cx);
        let saved = h.last_saved();
        assert_eq!(saved["hearing_aid_beside"], serde_json::json!(AID));
        assert_eq!(saved["hearing_aid_delay_ms"], serde_json::json!(60));
    }

    #[test]
    fn a_saved_delay_shows_as_the_nearest_one_offered() {
        assert_eq!(nearest_delay(0), 0);
        assert_eq!(nearest_delay(40), 40);
        assert_eq!(nearest_delay(55), 60);
        assert_eq!(nearest_delay(175), 150);
        assert_eq!(nearest_delay(9000), 500);
    }
}
