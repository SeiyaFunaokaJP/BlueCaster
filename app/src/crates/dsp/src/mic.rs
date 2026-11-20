//! The headset microphone's chain: gain → noise suppression → noise gate →
//! limiter, at the microphone's own rate (noise suppression goes through
//! 48 kHz inside [`Denoiser`]). Everything off is no processing at all: the
//! caller passes the samples by ([`MicSettings::is_active`]).

use crate::denoise::Denoiser;
use crate::dynamics::{GATE_OFF_DB, Limiter, NoiseGate, db_to_lin};

/// The gain's range in dB
pub const GAIN_MIN_DB: f32 = -12.0;
pub const GAIN_MAX_DB: f32 = 24.0;
/// The gate's highest threshold; [`GATE_OFF_DB`] is off
pub const GATE_MAX_DB: f32 = -20.0;
/// What the limiter holds the output to while the chain runs
pub const LIMITER_CEILING_DB: f32 = -1.0;

/// What the chain does
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MicSettings {
    pub gain_db: f32,
    /// 0..1: how much of the denoised signal (the rest is the dry one); 0:
    /// noise suppression off (the denoiser does not run)
    pub suppression: f32,
    /// The gate's threshold in dBFS; [`GATE_OFF_DB`] or lower: off
    pub gate_db: f32,
}

impl Default for MicSettings {
    fn default() -> Self {
        MicSettings { gain_db: 0.0, suppression: 0.0, gate_db: GATE_OFF_DB }
    }
}

impl MicSettings {
    /// Anything to do: off, the samples pass untouched
    pub fn is_active(&self) -> bool {
        self.gain_db != 0.0 || self.suppresses() || self.gate_db > GATE_OFF_DB
    }

    /// Noise suppression is on
    pub fn suppresses(&self) -> bool {
        self.suppression > 0.0
    }

    /// Within the ranges the chain takes
    pub fn clamped(self) -> MicSettings {
        MicSettings {
            gain_db: if self.gain_db.is_finite() { self.gain_db.clamp(GAIN_MIN_DB, GAIN_MAX_DB) } else { 0.0 },
            suppression: if self.suppression.is_finite() { self.suppression.clamp(0.0, 1.0) } else { 0.0 },
            gate_db: if self.gate_db.is_finite() { self.gate_db.clamp(GATE_OFF_DB, GATE_MAX_DB) } else { GATE_OFF_DB },
        }
    }
}

/// The chain's state for one microphone at one rate. Allocation-free after
/// [`MicChain::new`].
pub struct MicChain {
    rate: u32,
    /// The linear gain reached (ramped to a new one over a block)
    gain: f32,
    /// None: the rate has no whole 10 ms (no noise suppression there)
    denoiser: Option<Box<Denoiser>>,
    denoising: bool,
    gate: NoiseGate,
    limiter: Limiter,
    /// Ran the block before: a chain that starts again starts clean
    active: bool,
}

impl MicChain {
    pub fn new(rate: u32) -> Self {
        let sr = rate as f32;
        MicChain {
            rate,
            gain: 1.0,
            denoiser: rate.is_multiple_of(100).then(|| Box::new(Denoiser::new(rate))),
            denoising: false,
            gate: NoiseGate::new(sr),
            limiter: Limiter::new(sr),
            active: false,
        }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// The delay `settings` add, in samples (the limiter's look-ahead, and
    /// noise suppression's when on)
    pub fn latency(&self, settings: &MicSettings) -> usize {
        if !settings.is_active() {
            return 0;
        }
        let limiter = ((0.0015 * self.rate as f32) as usize).max(1);
        let ns = match &self.denoiser {
            Some(d) if settings.suppresses() => d.latency(),
            _ => 0,
        };
        limiter + ns
    }

    /// Processes `buf` (mono, ±1.0 full scale) in place; returns false (and
    /// leaves it untouched) when `settings` do nothing
    pub fn process(&mut self, buf: &mut [f32], settings: &MicSettings) -> bool {
        if !settings.is_active() {
            self.active = false;
            return false;
        }
        let target = db_to_lin(settings.gain_db);
        if !std::mem::replace(&mut self.active, true) {
            self.gain = target;
            self.denoising = false;
        }

        // Linear ramp to the new gain over the block: no zipper noise while dragging
        if !buf.is_empty() {
            let step = (target - self.gain) / buf.len() as f32;
            for s in buf.iter_mut() {
                self.gain += step;
                *s *= self.gain;
            }
        }
        self.gain = target;

        match self.denoiser.as_mut() {
            Some(d) if settings.suppresses() => {
                if !std::mem::replace(&mut self.denoising, true) {
                    d.reset();
                }
                d.process(buf, settings.suppression);
            }
            _ => self.denoising = false,
        }

        self.gate.process(buf, settings.gate_db);
        self.limiter.process(buf, LIMITER_CEILING_DB);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dynamics::lin_to_db;

    fn tone(rate: u32, amp: f32, len: usize) -> Vec<f32> {
        (0..len).map(|n| (2.0 * std::f32::consts::PI * 300.0 * n as f32 / rate as f32).sin() * amp).collect()
    }

    fn peak(x: &[f32]) -> f32 {
        x.iter().fold(0.0f32, |m, &v| m.max(v.abs()))
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    fn run(rate: u32, input: &[f32], settings: &MicSettings, chunk: usize) -> Vec<f32> {
        let mut chain = MicChain::new(rate);
        let mut out = input.to_vec();
        for c in out.chunks_mut(chunk) {
            chain.process(c, settings);
        }
        out
    }

    #[test]
    fn off_leaves_the_samples_alone() {
        let input = tone(16_000, 0.5, 16_000);
        let mut chain = MicChain::new(16_000);
        let mut out = input.clone();
        assert!(!chain.process(&mut out, &MicSettings::default()));
        assert_eq!(out, input);
        assert_eq!(chain.latency(&MicSettings::default()), 0);
    }

    #[test]
    fn gain_raises_the_level_by_its_decibels() {
        for gain_db in [6.0, -6.0, 12.0] {
            let input = tone(16_000, 0.05, 16_000);
            let s = MicSettings { gain_db, ..MicSettings::default() };
            let out = run(16_000, &input, &s, 120);
            let db = lin_to_db(rms(&out[8_000..]) / rms(&input[8_000..]));
            assert!((db - gain_db).abs() < 0.1, "{gain_db} dB asked, {db:.2} dB got");
        }
    }

    #[test]
    fn limiter_holds_the_ceiling_under_full_gain() {
        let input = tone(16_000, 0.9, 16_000);
        let s = MicSettings { gain_db: GAIN_MAX_DB, ..MicSettings::default() };
        let out = run(16_000, &input, &s, 120);
        let ceiling = db_to_lin(LIMITER_CEILING_DB);
        assert!(peak(&out) <= ceiling + 1e-6, "{}", peak(&out));
        assert!(peak(&out) > ceiling * 0.95);
    }

    #[test]
    fn suppression_lowers_noise() {
        let mut seed = 3u32;
        let noise: Vec<f32> = (0..16_000 * 3)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((seed >> 8) as f32 / (1 << 24) as f32 - 0.5) * 0.04
            })
            .collect();
        let s = MicSettings { suppression: 1.0, ..MicSettings::default() };
        let out = run(16_000, &noise, &s, 120);
        let db = lin_to_db(rms(&out[32_000..]) / rms(&noise[32_000..]));
        assert!(db < -10.0, "{db:.1} dB");
    }

    #[test]
    fn gate_mutes_what_stays_below_it() {
        let input = tone(16_000, 0.002, 32_000);
        let s = MicSettings { gate_db: -40.0, ..MicSettings::default() };
        let out = run(16_000, &input, &s, 120);
        assert!(peak(&out[24_000..]) < 1e-4, "{}", peak(&out[24_000..]));
    }

    #[test]
    fn clamped_keeps_values_in_range() {
        let s = MicSettings { gain_db: 99.0, suppression: 7.0, gate_db: 0.0 }.clamped();
        assert_eq!((s.gain_db, s.suppression, s.gate_db), (GAIN_MAX_DB, 1.0, GATE_MAX_DB));
        assert_eq!(MicSettings { suppression: f32::NAN, ..MicSettings::default() }.clamped().suppression, 0.0);
    }

    #[test]
    fn zero_strength_is_no_noise_suppression() {
        let s = MicSettings { suppression: 0.0, ..MicSettings::default() };
        assert!(!s.is_active());
        let mut chain = MicChain::new(16_000);
        assert_eq!(chain.latency(&s), 0);
        let input = tone(16_000, 0.5, 1_600);
        let mut out = input.clone();
        assert!(!chain.process(&mut out, &s));
        assert_eq!(out, input);
        // With the gain on, the denoiser still stays out (no added delay)
        let g = MicSettings { gain_db: 6.0, ..s };
        assert_eq!(chain.latency(&g), (0.0015f32 * 16_000.0) as usize);
    }

    /// `cargo test -p dsp --release -- --ignored --nocapture bench_mic_chain`
    #[test]
    #[ignore = "benchmark"]
    fn bench_mic_chain() {
        for rate in [16_000u32, 32_000, 48_000] {
            let secs = 30;
            let input = tone(rate, 0.2, rate as usize * secs);
            let s = MicSettings { gain_db: 12.0, suppression: 1.0, gate_db: -50.0 };
            let mut chain = MicChain::new(rate);
            let mut buf = input.clone();
            let t = std::time::Instant::now();
            for c in buf.chunks_mut(rate as usize * 75 / 10_000) {
                chain.process(c, &s);
            }
            let cpu = t.elapsed().as_secs_f64() / secs as f64 * 100.0;
            println!("{rate} Hz, everything on: {cpu:.2}% of one core");
        }
    }
}
