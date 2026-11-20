//! Audio processing shared by the media path (EQ before the encoder) and the
//! headset microphone path (gain, noise suppression, gate, limiter).
//!
//! `biquad` and `dynamics` come from Easy-MIC-Booster (same author, MIT OR
//! Apache-2.0), so its presets sound the same here; `denoise` and `resample`
//! follow its RNNoise and resampler, `mic` is the microphone's chain.

pub mod biquad;
pub mod denoise;
pub mod dynamics;
pub mod mic;
pub mod resample;

use serde::{Deserialize, Serialize};

/// Most EQ points one chain takes
pub const MAX_EQ_POINTS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BandType {
    #[default]
    Peaking,
    LowShelf,
    HighShelf,
    HighPass,
    LowPass,
}

/// One EQ point: frequency in Hz, gain in dB, Q (ignored by shelves)
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct EqBand {
    pub frequency: f32,
    pub gain: f32,
    pub q: f32,
    pub band_type: BandType,
}

impl EqBand {
    pub fn peaking(frequency: f32, gain: f32, q: f32) -> Self {
        EqBand { frequency, gain, q, band_type: BandType::Peaking }
    }
}
