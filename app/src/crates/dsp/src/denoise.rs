//! Noise suppression for a microphone at any rate whose 10 ms are a whole
//! number of samples (8, 16, 24, 32, 48 kHz): RNNoise (the pure-Rust
//! `nnnoiseless` port, as Easy-MIC-Booster's "Standard" method) runs on
//! 480-sample frames at 48 kHz, so each 10 ms block is resampled up to
//! 48 kHz, denoised, mixed with the dry signal by the strength, and
//! resampled back.
//!
//! Sample in, sample out: [`Denoiser::process`] takes any number of samples
//! and returns as many, with a fixed delay ([`Denoiser::latency`]): one block
//! of collection, RNNoise's one frame, and the two resamplers' look-ahead.
//! Allocation-free after [`Denoiser::new`].

use nnnoiseless::DenoiseState;

use crate::resample::BlockResampler;

/// RNNoise's frame: 10 ms at 48 kHz
pub const FRAME: usize = DenoiseState::FRAME_SIZE;
const MODEL_RATE: u32 = 48_000;
/// RNNoise works on 16-bit sample values
const SCALE: f32 = 32768.0;

pub struct Denoiser {
    rate: u32,
    /// Samples per block at `rate` (10 ms)
    block: usize,
    state: Box<DenoiseState<'static>>,
    /// None at 48 kHz
    up: Option<BlockResampler>,
    down: Option<BlockResampler>,
    /// The block being collected, and the processed one being handed out
    input: Vec<f32>,
    output: Vec<f32>,
    pos: usize,
    /// One frame at 48 kHz: what RNNoise reads (scaled) and writes
    frame: [f32; FRAME],
    wet: [f32; FRAME],
    /// The frame before, the dry signal lined up with RNNoise's output
    dry: [f32; FRAME],
    /// The first frame after a reset is the model's start-up transient
    warm: bool,
    vad: f32,
}

impl Denoiser {
    /// For a microphone at `rate` Hz (10 ms must be whole samples)
    pub fn new(rate: u32) -> Self {
        assert!(rate.is_multiple_of(100), "{rate} Hz");
        let block = rate as usize / 100;
        let (up, down) = if rate == MODEL_RATE {
            (None, None)
        } else {
            (Some(BlockResampler::new(rate, MODEL_RATE, block)), Some(BlockResampler::new(MODEL_RATE, rate, FRAME)))
        };
        Denoiser {
            rate,
            block,
            state: DenoiseState::new(),
            up,
            down,
            input: vec![0.0; block],
            output: vec![0.0; block],
            pos: 0,
            frame: [0.0; FRAME],
            wet: [0.0; FRAME],
            dry: [0.0; FRAME],
            warm: false,
            vad: 0.0,
        }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// The delay it adds, in samples at its rate: a block to collect,
    /// RNNoise's frame (10 ms) and the resamplers' look-ahead
    pub fn latency(&self) -> usize {
        let up = self.up.as_ref().map_or(0, |r| r.latency_in());
        // The down resampler's look-ahead is in 48 kHz samples
        let down = self.down.as_ref().map_or(0, |r| r.latency_in() * self.block / FRAME);
        2 * self.block + up + down
    }

    /// Speech probability of the last frame (0..1)
    pub fn vad(&self) -> f32 {
        self.vad
    }

    /// Back to silence, as if new (RNNoise's own state is kept: the frame
    /// after a reset is muted instead, which covers its transient)
    pub fn reset(&mut self) {
        self.input.fill(0.0);
        self.output.fill(0.0);
        self.pos = 0;
        self.dry.fill(0.0);
        self.warm = false;
        self.vad = 0.0;
        if let Some(r) = self.up.as_mut() {
            r.reset();
        }
        if let Some(r) = self.down.as_mut() {
            r.reset();
        }
    }

    /// Denoises `buf` in place, delayed by [`Denoiser::latency`].
    /// `strength`: 1 fully denoised, 0 the dry signal (still delayed, so the
    /// strength never shifts the timing).
    pub fn process(&mut self, buf: &mut [f32], strength: f32) {
        for s in buf.iter_mut() {
            self.input[self.pos] = *s;
            *s = self.output[self.pos];
            self.pos += 1;
            if self.pos == self.block {
                self.pos = 0;
                self.run_block(strength);
            }
        }
    }

    fn run_block(&mut self, strength: f32) {
        // Up to 48 kHz into `frame`
        match self.up.as_mut() {
            Some(r) => r.process(&self.input, &mut self.frame),
            None => self.frame.copy_from_slice(&self.input),
        }
        for s in self.frame.iter_mut() {
            *s *= SCALE;
        }
        self.vad = self.state.process_frame(&mut self.wet, &self.frame);
        // RNNoise's output lags its input by one frame: mix with the frame before
        let wet = if std::mem::replace(&mut self.warm, true) { strength.clamp(0.0, 1.0) / SCALE } else { 0.0 };
        let dry = 1.0 - strength.clamp(0.0, 1.0);
        for ((w, d), now) in self.wet.iter_mut().zip(self.dry.iter_mut()).zip(&self.frame) {
            *w = *w * wet + *d * dry;
            *d = *now / SCALE;
        }
        match self.down.as_mut() {
            Some(r) => r.process(&self.wet, &mut self.output),
            None => self.output.copy_from_slice(&self.wet),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(len: usize, amp: f32, mut seed: u32) -> Vec<f32> {
        (0..len)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((seed >> 8) as f32 / (1 << 24) as f32 - 0.5) * 2.0 * amp
            })
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    #[test]
    fn suppresses_steady_noise_at_every_voice_rate() {
        for rate in [8_000, 16_000, 32_000, 48_000] {
            let input = noise(rate as usize * 3, 0.03, 7);
            let mut d = Denoiser::new(rate);
            let mut out = input.clone();
            // Odd chunk sizes, as the codecs push them
            for c in out.chunks_mut(rate as usize * 3 / 400) {
                d.process(c, 1.0);
            }
            let tail = rate as usize * 2;
            let db = 20.0 * (rms(&out[tail..]) / rms(&input[tail..])).log10();
            assert!(db < -10.0, "{rate} Hz: {db:.1} dB");
        }
    }

    #[test]
    fn strength_zero_is_the_input_delayed() {
        for rate in [16_000, 48_000] {
            let input: Vec<f32> = (0..rate as usize)
                .map(|n| (2.0 * std::f32::consts::PI * 200.0 * n as f32 / rate as f32).sin() * 0.3)
                .collect();
            let mut d = Denoiser::new(rate);
            let lat = d.latency();
            let mut out = input.clone();
            d.process(&mut out, 0.0);
            let err: f32 =
                (rate as usize / 2..input.len()).map(|i| (out[i] - input[i - lat]).abs()).fold(0.0, f32::max);
            assert!(err < 0.02, "{rate} Hz: latency {lat}, error {err}");
        }
    }

    #[test]
    fn latency_is_about_twenty_milliseconds() {
        let d = Denoiser::new(16_000);
        // 10 ms block + 10 ms frame + 1 ms up + 1 ms down
        assert_eq!(d.latency(), 160 + 160 + 16 + 16);
        assert_eq!(Denoiser::new(48_000).latency(), 960);
    }
}
