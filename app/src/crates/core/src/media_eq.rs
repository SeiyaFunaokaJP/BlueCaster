//! The playback EQ: each device profile's own (`media_eq` in profiles.json),
//! applied to the audio before it is encoded. One stream runs at a time, so
//! the encode threads share one [`EqHandle`]: the profile that starts
//! streaming sets it, and a change saved (or tried out) while it streams
//! takes effect from the next block of audio, without reconnecting.

use std::sync::LazyLock;

use dsp::{BandType, EqBand};
use pipeline::eq::{EqHandle, EqSettings};
use serde::{Deserialize, Serialize};

/// A profile's EQ: on or off, and its points (kept while off)
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaEq {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub bands: Vec<EqBand>,
}

impl MediaEq {
    /// Off with no points: not written to the profile
    pub fn is_default(&self) -> bool {
        *self == MediaEq::default()
    }

    /// From the UI's form
    pub fn from_ipc(eq: &ipc::MediaEq) -> MediaEq {
        let kind = |t: ipc::EqBandType| match t {
            ipc::EqBandType::Peaking => BandType::Peaking,
            ipc::EqBandType::LowShelf => BandType::LowShelf,
            ipc::EqBandType::HighShelf => BandType::HighShelf,
            ipc::EqBandType::HighPass => BandType::HighPass,
            ipc::EqBandType::LowPass => BandType::LowPass,
        };
        // Held to the EQ's ranges as saved, so the file shows what plays
        let held = EqSettings::new(
            eq.enabled,
            &eq.bands
                .iter()
                .map(|b| EqBand { frequency: b.frequency, gain: b.gain, q: b.q, band_type: kind(b.band_type) })
                .collect::<Vec<_>>(),
        );
        MediaEq { enabled: eq.enabled, bands: held.bands().to_vec() }
    }

    /// As the UI gets it
    pub fn to_ipc(&self) -> ipc::MediaEq {
        let kind = |t: BandType| match t {
            BandType::Peaking => ipc::EqBandType::Peaking,
            BandType::LowShelf => ipc::EqBandType::LowShelf,
            BandType::HighShelf => ipc::EqBandType::HighShelf,
            BandType::HighPass => ipc::EqBandType::HighPass,
            BandType::LowPass => ipc::EqBandType::LowPass,
        };
        ipc::MediaEq {
            enabled: self.enabled,
            bands: self
                .bands
                .iter()
                .map(|b| ipc::EqPoint { frequency: b.frequency, gain: b.gain, q: b.q, band_type: kind(b.band_type) })
                .collect(),
        }
    }

    /// What the encode thread takes (held to the EQ's ranges)
    pub fn settings(&self) -> EqSettings {
        EqSettings::new(self.enabled, &self.bands)
    }
}

static HANDLE: LazyLock<EqHandle> = LazyLock::new(EqHandle::default);

/// The EQ the encode threads read
pub fn handle() -> EqHandle {
    HANDLE.clone()
}

/// The stream's EQ from now on (None: off, a stream without a profile)
pub fn apply(eq: Option<&MediaEq>) {
    let settings = eq.map_or(EqSettings::OFF, MediaEq::settings);
    if HANDLE.get() != settings {
        tracing::debug!(
            "playback EQ: {}, {} point(s)",
            if settings.enabled { "on" } else { "off" },
            settings.bands().len()
        );
        HANDLE.set(settings);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_as_its_points_and_left_out_when_unused() {
        let eq = MediaEq {
            enabled: true,
            bands: vec![EqBand { frequency: 100.0, gain: -3.0, q: 0.7, band_type: dsp::BandType::LowShelf }],
        };
        let json = serde_json::to_string(&eq).unwrap();
        assert_eq!(
            json,
            r#"{"enabled":true,"bands":[{"frequency":100.0,"gain":-3.0,"q":0.7,"band_type":"LowShelf"}]}"#
        );
        assert_eq!(serde_json::from_str::<MediaEq>(&json).unwrap(), eq);
        assert_eq!(serde_json::from_str::<MediaEq>("{}").unwrap(), MediaEq::default());
        assert!(MediaEq::default().is_default());
        assert_eq!(eq.settings().bands().len(), 1);
        assert_eq!(MediaEq::from_ipc(&eq.to_ipc()), eq);
        // Out of range from the UI: held, as it plays
        let wild = ipc::MediaEq {
            enabled: true,
            bands: vec![ipc::EqPoint { frequency: 5.0, gain: 40.0, q: 0.0, band_type: ipc::EqBandType::Peaking }],
        };
        assert_eq!(MediaEq::from_ipc(&wild).bands, [EqBand::peaking(20.0, 12.0, 0.1)]);
    }
}
