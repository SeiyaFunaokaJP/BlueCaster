//! Noise gate and look-ahead peak limiter.

#[inline]
fn coef(ms: f32, sample_rate: f32) -> f32 {
    (-1.0 / (0.001 * ms * sample_rate)).exp()
}

#[inline]
pub fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

#[inline]
pub fn lin_to_db(lin: f32) -> f32 {
    20.0 * lin.max(1e-9).log10()
}

/// Lowest threshold; the gate is bypassed at this value.
pub const GATE_OFF_DB: f32 = -80.0;

/// Noise gate with hysteresis and hold, so it does not chatter on word endings
/// the way the v1.x gate (a steep expander) did.
pub struct NoiseGate {
    env: f32,
    gain: f32,
    hold_left: u32,
    open: bool,
    env_attack: f32,
    env_release: f32,
    gain_open: f32,
    gain_close: f32,
    hold_samples: u32,
}

impl NoiseGate {
    const HYSTERESIS_DB: f32 = 4.0;
    const FLOOR: f32 = 0.0;

    pub fn new(sample_rate: f32) -> Self {
        NoiseGate {
            env: 0.0,
            gain: 1.0,
            hold_left: 0,
            open: true,
            env_attack: coef(0.5, sample_rate),
            env_release: coef(40.0, sample_rate),
            gain_open: coef(1.5, sample_rate),
            gain_close: coef(120.0, sample_rate),
            hold_samples: (0.120 * sample_rate) as u32,
        }
    }

    /// Returns whether the gate was open at the end of the block.
    pub fn process(&mut self, buf: &mut [f32], threshold_db: f32) -> bool {
        if threshold_db <= GATE_OFF_DB {
            self.open = true;
            self.gain = 1.0;
            return true;
        }
        let open_at = db_to_lin(threshold_db);
        let close_at = db_to_lin(threshold_db - Self::HYSTERESIS_DB);
        for s in buf.iter_mut() {
            let a = s.abs();
            let c = if a > self.env { self.env_attack } else { self.env_release };
            self.env = c * self.env + (1.0 - c) * a;

            if self.env >= open_at {
                self.open = true;
                self.hold_left = self.hold_samples;
            } else if self.env < close_at {
                if self.hold_left > 0 {
                    self.hold_left -= 1;
                } else {
                    self.open = false;
                }
            }

            let (target, c) = if self.open { (1.0, self.gain_open) } else { (Self::FLOOR, self.gain_close) };
            self.gain = c * self.gain + (1.0 - c) * target;
            *s *= self.gain;
        }
        self.open
    }

    /// Detector level in dBFS, for the UI meter.
    pub fn level_db(&self) -> f32 {
        lin_to_db(self.env)
    }
}

/// Look-ahead brick-wall limiter: min-hold of the required gain over the
/// look-ahead window, then a box filter of the same length. The box average at
/// the time a peak leaves the delay line only contains values at or below that
/// peak's required gain, so the output never exceeds the ceiling, without the
/// hard clipping v1.x relied on.
pub struct Limiter {
    delay: Vec<f32>,
    targets: Vec<f32>,
    smooth: Vec<f32>,
    pos: usize,
    sum: f64,
    release_gain: f32,
    release: f32,
}

impl Limiter {
    pub fn new(sample_rate: f32) -> Self {
        let lookahead = ((0.0015 * sample_rate) as usize).max(1);
        Limiter {
            delay: vec![0.0; lookahead],
            targets: vec![1.0; lookahead + 1],
            smooth: vec![1.0; lookahead],
            pos: 0,
            sum: lookahead as f64,
            release_gain: 1.0,
            release: coef(80.0, sample_rate),
        }
    }

    #[cfg(test)]
    pub fn latency(&self) -> usize {
        self.delay.len()
    }

    /// Returns the deepest gain applied in the block (1.0 = no limiting).
    pub fn process(&mut self, buf: &mut [f32], ceiling_db: f32) -> f32 {
        let ceiling = db_to_lin(ceiling_db.min(0.0));
        let n = self.delay.len();
        let mut min_gain: f32 = 1.0;
        for s in buf.iter_mut() {
            let x = *s;
            let a = x.abs();
            let target = if a > ceiling { ceiling / a } else { 1.0 };

            // Min-hold over the window (n + 1 samples). n is tiny (72 @ 48 kHz).
            let tpos = self.pos % self.targets.len();
            self.targets[tpos] = target;
            let held = self.targets.iter().copied().fold(1.0f32, f32::min);

            // Instant attack, smooth release.
            self.release_gain =
                if held < self.release_gain { held } else { held + (self.release_gain - held) * self.release };

            // Box filter over n samples.
            let spos = self.pos % n;
            self.sum += self.release_gain as f64 - self.smooth[spos] as f64;
            self.smooth[spos] = self.release_gain;
            let g = (self.sum / n as f64) as f32;

            let delayed = self.delay[spos];
            self.delay[spos] = x;
            self.pos = self.pos.wrapping_add(1);

            min_gain = min_gain.min(g);
            *s = (delayed * g).clamp(-ceiling, ceiling);
        }
        // Keep the running sum from drifting over hours of audio.
        if self.pos % (1 << 16) < buf.len() {
            self.sum = self.smooth.iter().map(|&v| v as f64).sum();
        }
        min_gain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_never_exceeds_ceiling() {
        let mut l = Limiter::new(48_000.0);
        let mut buf: Vec<f32> =
            (0..48_000).map(|n| (n as f32 * 0.05).sin() * if n % 4800 < 300 { 8.0 } else { 0.3 }).collect();
        l.process(&mut buf, -1.0);
        let ceiling = db_to_lin(-1.0);
        let peak = buf.iter().fold(0.0f32, |m, &v| m.max(v.abs()));
        assert!(peak <= ceiling + 1e-6, "{peak}");
    }

    #[test]
    fn limiter_is_transparent_below_ceiling() {
        let mut l = Limiter::new(48_000.0);
        let input: Vec<f32> = (0..4800).map(|n| (n as f32 * 0.01).sin() * 0.5).collect();
        let mut buf = input.clone();
        l.process(&mut buf, -1.0);
        let lat = l.latency();
        for i in lat..input.len() {
            assert!((buf[i] - input[i - lat]).abs() < 1e-5);
        }
    }

    #[test]
    fn gate_closes_on_silence_and_opens_on_voice() {
        let mut g = NoiseGate::new(48_000.0);
        let mut quiet = vec![0.001f32; 48_000];
        assert!(!g.process(&mut quiet, -40.0));
        assert!(quiet[47_999].abs() < 1e-4);
        let mut loud: Vec<f32> = (0..4800).map(|n| (n as f32 * 0.1).sin() * 0.5).collect();
        assert!(g.process(&mut loud, -40.0));
    }

    #[test]
    fn gate_off_is_passthrough() {
        let mut g = NoiseGate::new(48_000.0);
        let mut buf = vec![0.0001f32; 480];
        g.process(&mut buf, GATE_OFF_DB);
        assert!(buf.iter().all(|&v| v == 0.0001));
    }
}
