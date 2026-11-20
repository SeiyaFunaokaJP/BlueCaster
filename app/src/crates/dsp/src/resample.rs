//! Fixed-ratio windowed-sinc resampler for blocks of a fixed length: every
//! block of `input_len` samples gives exactly `output_len`, so a 10 ms block
//! at the microphone's rate maps onto a 10 ms block at 48 kHz and back.
//!
//! The kernel (Blackman-windowed sinc, cutoff 0.94 of the lower Nyquist) is
//! Easy-MIC-Booster's `resample.rs` (same author, MIT OR Apache-2.0), 32
//! taps at the lower rate (a downsampler's kernel is that many times longer
//! at its input rate). Here the ratio is rational, so each output sample has
//! an exact phase and no interpolation between table rows is needed.
//! Allocation-free after [`BlockResampler::new`].

use std::f64::consts::PI;

/// Half the kernel's taps at the lower of the two rates
const HALF_TAPS: usize = 16;

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

pub struct BlockResampler {
    /// Half the kernel's length, in input samples
    half: usize,
    /// `phases` rows of `2 * half` coefficients
    table: Vec<f32>,
    /// Input step per output sample as `step / phases` (reduced ratio)
    step: usize,
    phases: usize,
    /// The last `2 * half` input samples of the block before, then the block
    history: Vec<f32>,
    input_len: usize,
    output_len: usize,
}

impl BlockResampler {
    /// From `from_rate` to `to_rate` in blocks of `input_len` samples, which
    /// must map onto a whole number of output samples
    pub fn new(from_rate: u32, to_rate: u32, input_len: usize) -> Self {
        assert!(from_rate > 0 && to_rate > 0);
        let g = gcd(from_rate, to_rate);
        let (step, phases) = ((from_rate / g) as usize, (to_rate / g) as usize);
        assert_eq!(
            input_len * phases % step,
            0,
            "{input_len} samples at {from_rate} Hz are no whole block at {to_rate}"
        );
        let ratio = to_rate as f64 / from_rate as f64;
        let cutoff = ratio.min(1.0) * 0.94;
        let half = HALF_TAPS * step.div_ceil(phases);
        let taps = 2 * half;
        let mut table = vec![0f32; phases * taps];
        let mut row64 = vec![0f64; taps];
        for (p, row) in table.chunks_exact_mut(taps).enumerate() {
            let frac = p as f64 / phases as f64;
            let mut sum = 0.0;
            for (k, c) in row64.iter_mut().enumerate() {
                // Tap k sits at input offset k - (half - 1) from the integer index
                let x = k as f64 - (half as f64 - 1.0) - frac;
                let sinc = if x.abs() < 1e-9 { 1.0 } else { (PI * cutoff * x).sin() / (PI * cutoff * x) };
                let t = (x / half as f64 + 1.0) / 2.0;
                let w = if (0.0..=1.0).contains(&t) {
                    0.42 - 0.5 * (2.0 * PI * t).cos() + 0.08 * (4.0 * PI * t).cos()
                } else {
                    0.0
                };
                *c = cutoff * sinc * w;
                sum += *c;
            }
            // Unity DC gain for every phase
            for (c, v) in row.iter_mut().zip(&row64) {
                *c = (v / sum) as f32;
            }
        }
        BlockResampler {
            half,
            table,
            step,
            phases,
            history: vec![0.0; taps + input_len],
            input_len,
            output_len: input_len * phases / step,
        }
    }

    pub fn output_len(&self) -> usize {
        self.output_len
    }

    /// Delay in input samples (the kernel's look-ahead)
    pub fn latency_in(&self) -> usize {
        self.half
    }

    pub fn reset(&mut self) {
        self.history.fill(0.0);
    }

    /// One block: `input.len()` is `input_len`, `out.len()` is
    /// [`BlockResampler::output_len`]
    pub fn process(&mut self, input: &[f32], out: &mut [f32]) {
        debug_assert_eq!(input.len(), self.input_len);
        debug_assert_eq!(out.len(), self.output_len);
        let (half, taps) = (self.half, 2 * self.half);
        self.history[taps..].copy_from_slice(input);
        if self.step == 1 && self.phases == 1 {
            // The same rate: only the delay, as the other ratios have
            out.copy_from_slice(&self.history[half..half + self.input_len]);
        } else {
            for (j, o) in out.iter_mut().enumerate() {
                let pos = j * self.step;
                let (idx, phase) = (pos / self.phases, pos % self.phases);
                let row = &self.table[phase * taps..(phase + 1) * taps];
                // Output j lies `half` input samples behind its position,
                // so every tap is already in `history`
                let src = &self.history[idx + 1..idx + 1 + taps];
                *o = row.iter().zip(src).map(|(c, s)| c * s).sum();
            }
        }
        self.history.copy_within(self.input_len.., 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32, freq: f64, len: usize) -> Vec<f32> {
        (0..len).map(|n| (2.0 * PI * freq * n as f64 / rate as f64).sin() as f32).collect()
    }

    fn run(from: u32, to: u32, input: &[f32]) -> Vec<f32> {
        let n = from as usize / 100;
        let mut r = BlockResampler::new(from, to, n);
        let mut out = vec![0.0; input.len() / n * r.output_len()];
        let m = r.output_len();
        for (i, o) in input.chunks_exact(n).zip(out.chunks_exact_mut(m)) {
            r.process(i, o);
        }
        out
    }

    #[test]
    fn block_lengths_match_the_ratio() {
        for (from, to, m) in [(8_000, 48_000, 480), (16_000, 48_000, 480), (32_000, 48_000, 480), (48_000, 16_000, 160)]
        {
            assert_eq!(BlockResampler::new(from, to, from as usize / 100).output_len(), m);
        }
    }

    #[test]
    fn passband_tone_keeps_amplitude_up_and_down() {
        for rate in [8_000, 16_000, 24_000, 32_000, 48_000] {
            let input = tone(rate, 1000.0, rate as usize);
            let up = run(rate, 48_000, &input);
            let down = run(48_000, rate, &up);
            let peak = down[down.len() / 2..].iter().fold(0.0f32, |m, &v| m.max(v.abs()));
            assert!((peak - 1.0).abs() < 0.03, "{rate}: {peak}");
        }
    }

    #[test]
    fn downsampling_rejects_aliases() {
        // 12 kHz cannot exist at 16 kHz
        let up = tone(48_000, 12_000.0, 48_000);
        let out = run(48_000, 16_000, &up);
        let peak = out[4000..].iter().fold(0.0f32, |m, &v| m.max(v.abs()));
        assert!(peak < 0.01, "{peak}");
    }
}
