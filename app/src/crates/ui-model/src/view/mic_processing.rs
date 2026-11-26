//! The headset microphone's processing: the sliders' ranges and the texts
//! of their values.

use ipc::MicProcessing;

use crate::i18n::t;

pub const MIC_GAIN_RANGE: (i32, i32) = (-12, 24);
pub const MIC_SUPPRESSION_RANGE: (i32, i32) = (0, 100);
pub const MIC_GATE_RANGE: (i32, i32) = (-80, -20);

/// The switch over the processing is on
pub fn mic_processing_on(p: &MicProcessing) -> bool {
    p.enabled
}

/// "+6 dB"
pub fn gain_text(db: i32) -> String {
    format!("{}{db} dB", if db > 0 { "+" } else { "" })
}

/// "-50 dB", Off at its lowest
pub fn gate_text(db: i32) -> String {
    if db <= MIC_GATE_RANGE.0 { t("ui.mic_gate_off") } else { format!("{db} dB") }
}

/// "70%", Off at 0
pub fn suppression_text(v: i32) -> String {
    if v <= 0 { t("ui.mic_suppression_off") } else { format!("{v}%") }
}

/// The sliders: off (the values kept, greyed out) while the processing or
/// the microphone is off, or a change is on its way
pub fn sliders_disabled(p: &MicProcessing, disabled: bool) -> bool {
    disabled || !p.enabled
}

/// The gate's slider position (a threshold below its range is off: its lowest)
pub fn gate_shown(p: &MicProcessing) -> i32 {
    i32::from(p.gate_db).max(MIC_GATE_RANGE.0)
}

/// One of the processing's settings
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicSetting {
    Enabled(bool),
    Gain(i32),
    Suppression(i32),
    Gate(i32),
}

/// The processing with one setting changed, the others kept
pub fn with_setting(p: &MicProcessing, s: MicSetting) -> MicProcessing {
    let mut out = *p;
    match s {
        MicSetting::Enabled(v) => out.enabled = v,
        MicSetting::Gain(v) => out.gain_db = v.clamp(MIC_GAIN_RANGE.0, MIC_GAIN_RANGE.1) as i8,
        MicSetting::Suppression(v) => {
            out.suppression = v.clamp(MIC_SUPPRESSION_RANGE.0, MIC_SUPPRESSION_RANGE.1) as u8;
        }
        MicSetting::Gate(v) => out.gate_db = v.clamp(MIC_GATE_RANGE.0, MIC_GATE_RANGE.1) as i8,
    }
    out
}

/// What a slider shows: the value while dragged, then the one saved until the
/// profile has it, else the profile's
pub fn slider_shown(held: Option<i32>, pending: Option<i32>, value: i32) -> i32 {
    held.or(pending).unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn off() -> MicProcessing {
        MicProcessing { enabled: false, gain_db: 0, suppression: 0, gate_db: -80 }
    }

    #[test]
    fn processes_the_microphone_saved_with_the_devices_profile() {
        // All off by default: the switch, and the sliders greyed out under it
        let p = off();
        assert!(!mic_processing_on(&p));
        assert!(sliders_disabled(&p, false));
        let p = with_setting(&p, MicSetting::Enabled(true));
        assert!(!sliders_disabled(&p, false));
        // ... and while the microphone is Off or a change is on its way
        assert!(sliders_disabled(&p, true));
        assert_eq!(gate_text(gate_shown(&p)), t("ui.mic_gate_off"));
        assert_eq!(suppression_text(i32::from(p.suppression)), t("ui.mic_suppression_off"));
        // A slider shows what it holds while dragged, and saves when let go
        assert_eq!(slider_shown(Some(6), None, 0), 6);
        assert_eq!(gain_text(6), "+6 dB");
        let p = with_setting(&p, MicSetting::Gain(6));
        assert_eq!(p, MicProcessing { enabled: true, gain_db: 6, suppression: 0, gate_db: -80 });
        assert_eq!(slider_shown(None, Some(70), 0), 70);
        let p = with_setting(&p, MicSetting::Suppression(70));
        assert_eq!(p, MicProcessing { enabled: true, gain_db: 6, suppression: 70, gate_db: -80 });
        assert_eq!(suppression_text(70), "70%");
        let p = with_setting(&p, MicSetting::Gate(-50));
        assert_eq!(p.gate_db, -50);
        assert_eq!(gate_text(-50), "-50 dB");
        // Switched off: the values stay, greyed out
        let p = with_setting(&p, MicSetting::Enabled(false));
        assert_eq!(p, MicProcessing { enabled: false, gain_db: 6, suppression: 70, gate_db: -50 });
        assert!(sliders_disabled(&p, false));
        assert_eq!(slider_shown(None, None, i32::from(p.gain_db)), 6);
    }

    #[test]
    fn texts_and_ranges() {
        assert_eq!(gain_text(0), "0 dB");
        assert_eq!(gain_text(-12), "-12 dB");
        // A gate below its range is off, at its lowest
        assert_eq!(gate_shown(&MicProcessing { gate_db: -100, ..off() }), -80);
        assert_eq!(with_setting(&off(), MicSetting::Gain(99)).gain_db, 24);
    }
}
