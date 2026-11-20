//! RBJ "Audio EQ Cookbook" biquads. Coefficients match NAudio's `BiQuadFilter`
//! (used by v1.x), so presets made with the C# version sound the same.

use std::f64::consts::PI;

use crate::{BandType, EqBand};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coeffs {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Coeffs {
    pub const IDENTITY: Coeffs = Coeffs { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0 };

    fn normalized(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        Coeffs { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 }
    }

    pub fn peaking(sample_rate: f64, freq: f64, q: f64, gain_db: f64) -> Self {
        let w0 = 2.0 * PI * freq / sample_rate;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q);
        let a = 10f64.powf(gain_db / 40.0);
        Self::normalized(1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a)
    }

    /// `slope` is the cookbook's shelf slope S (1.0 = steepest without overshoot).
    pub fn low_shelf(sample_rate: f64, freq: f64, slope: f64, gain_db: f64) -> Self {
        let w0 = 2.0 * PI * freq / sample_rate;
        let (sin, cos) = w0.sin_cos();
        let a = 10f64.powf(gain_db / 40.0);
        let alpha = sin / 2.0 * ((a + 1.0 / a) * (1.0 / slope - 1.0) + 2.0).sqrt();
        let sa = 2.0 * a.sqrt() * alpha;
        Self::normalized(
            a * ((a + 1.0) - (a - 1.0) * cos + sa),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
            a * ((a + 1.0) - (a - 1.0) * cos - sa),
            (a + 1.0) + (a - 1.0) * cos + sa,
            -2.0 * ((a - 1.0) + (a + 1.0) * cos),
            (a + 1.0) + (a - 1.0) * cos - sa,
        )
    }

    pub fn high_shelf(sample_rate: f64, freq: f64, slope: f64, gain_db: f64) -> Self {
        let w0 = 2.0 * PI * freq / sample_rate;
        let (sin, cos) = w0.sin_cos();
        let a = 10f64.powf(gain_db / 40.0);
        let alpha = sin / 2.0 * ((a + 1.0 / a) * (1.0 / slope - 1.0) + 2.0).sqrt();
        let sa = 2.0 * a.sqrt() * alpha;
        Self::normalized(
            a * ((a + 1.0) + (a - 1.0) * cos + sa),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
            a * ((a + 1.0) + (a - 1.0) * cos - sa),
            (a + 1.0) - (a - 1.0) * cos + sa,
            2.0 * ((a - 1.0) - (a + 1.0) * cos),
            (a + 1.0) - (a - 1.0) * cos - sa,
        )
    }

    pub fn high_pass(sample_rate: f64, freq: f64, q: f64) -> Self {
        let w0 = 2.0 * PI * freq / sample_rate;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q);
        Self::normalized((1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
    }

    pub fn low_pass(sample_rate: f64, freq: f64, q: f64) -> Self {
        let w0 = 2.0 * PI * freq / sample_rate;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q);
        Self::normalized((1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
    }

    /// Coefficients for one EQ band, or `None` when the band has no audible effect
    /// (v1.x skipped bands under 0.05 dB too, which keeps the phase untouched).
    pub fn for_band(band: &EqBand, sample_rate: f64) -> Option<Self> {
        let nyquist = sample_rate / 2.0;
        let freq = (band.frequency as f64).clamp(10.0, nyquist * 0.98);
        let q = (band.q as f64).clamp(0.05, 30.0);
        let gain = band.gain as f64;
        match band.band_type {
            BandType::HighPass => Some(Self::high_pass(sample_rate, freq, q)),
            BandType::LowPass => Some(Self::low_pass(sample_rate, freq, q)),
            _ if gain.abs() < 0.05 => None,
            BandType::Peaking => Some(Self::peaking(sample_rate, freq, q, gain)),
            // v1.x always used slope 1.0 for shelves; Q is ignored for them.
            BandType::LowShelf => Some(Self::low_shelf(sample_rate, freq, 1.0, gain)),
            BandType::HighShelf => Some(Self::high_shelf(sample_rate, freq, 1.0, gain)),
        }
    }

    /// Magnitude response in dB at `freq`.
    pub fn response_db(&self, freq: f64, sample_rate: f64) -> f64 {
        let w = 2.0 * PI * freq / sample_rate;
        let (s1, c1) = w.sin_cos();
        let (s2, c2) = (2.0 * w).sin_cos();
        let nr = self.b0 + self.b1 * c1 + self.b2 * c2;
        let ni = -(self.b1 * s1 + self.b2 * s2);
        let dr = 1.0 + self.a1 * c1 + self.a2 * c2;
        let di = -(self.a1 * s1 + self.a2 * s2);
        let num = nr * nr + ni * ni;
        let den = (dr * dr + di * di).max(1e-30);
        10.0 * (num / den).max(1e-30).log10()
    }
}

/// Transposed direct form II with f64 state: stable even for the very wide
/// (Q = 0.1) low-frequency bands that v1.x presets use.
#[derive(Clone, Copy, Debug)]
pub struct Biquad {
    c: Coeffs,
    z1: f64,
    z2: f64,
}

impl Default for Biquad {
    fn default() -> Self {
        Biquad { c: Coeffs::IDENTITY, z1: 0.0, z2: 0.0 }
    }
}

impl Biquad {
    /// Swaps coefficients but keeps the state, so dragging a point while audio
    /// runs does not click.
    pub fn set(&mut self, c: Coeffs) {
        self.c = c;
    }

    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

    #[inline]
    pub fn process(&mut self, x: f64) -> f64 {
        let c = &self.c;
        let y = c.b0 * x + self.z1;
        self.z1 = c.b1 * x - c.a1 * y + self.z2;
        self.z2 = c.b2 * x - c.a2 * y;
        y
    }
}

/// Total response of an EQ (sum of the bands' dB responses).
pub fn eq_response_db(bands: &[EqBand], freq: f64, sample_rate: f64) -> f64 {
    bands.iter().filter_map(|b| Coeffs::for_band(b, sample_rate)).map(|c| c.response_db(freq, sample_rate)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f64 = 48_000.0;

    fn band(t: BandType, f: f32, g: f32, q: f32) -> EqBand {
        EqBand { frequency: f, gain: g, q, band_type: t }
    }

    #[test]
    fn peaking_hits_gain_at_center() {
        let c = Coeffs::peaking(SR, 1000.0, 1.0, 6.0);
        assert!((c.response_db(1000.0, SR) - 6.0).abs() < 0.01);
        assert!(c.response_db(20.0, SR).abs() < 0.2);
    }

    #[test]
    fn shelves_reach_gain_in_their_band() {
        let lo = Coeffs::low_shelf(SR, 200.0, 1.0, -9.0);
        assert!((lo.response_db(20.0, SR) + 9.0).abs() < 0.3);
        assert!(lo.response_db(10_000.0, SR).abs() < 0.1);
        let hi = Coeffs::high_shelf(SR, 4000.0, 1.0, 5.0);
        assert!((hi.response_db(20_000.0, SR) - 5.0).abs() < 0.3);
    }

    #[test]
    fn high_pass_cuts_lows() {
        let c = Coeffs::high_pass(SR, 100.0, 0.707);
        assert!(c.response_db(20.0, SR) < -20.0);
        assert!(c.response_db(2000.0, SR).abs() < 0.1);
    }

    #[test]
    fn flat_bands_are_skipped() {
        assert!(Coeffs::for_band(&band(BandType::Peaking, 1000.0, 0.01, 1.0), SR).is_none());
        assert!(Coeffs::for_band(&band(BandType::HighPass, 80.0, 0.0, 0.7), SR).is_some());
    }

    #[test]
    fn filter_matches_predicted_response() {
        let c = Coeffs::peaking(SR, 1000.0, 2.0, 12.0);
        let mut f = Biquad::default();
        f.set(c);
        let mut peak: f64 = 0.0;
        for n in 0..48_000 {
            let x = (2.0 * PI * 1000.0 * n as f64 / SR).sin();
            let y = f.process(x);
            if n > 24_000 {
                peak = peak.max(y.abs());
            }
        }
        let db = 20.0 * peak.log10();
        assert!((db - 12.0).abs() < 0.1, "{db}");
    }
}
