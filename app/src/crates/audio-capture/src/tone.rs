//! Test tone: a known signal instead of captured audio.
//!
//! 1 kHz on the left, 1.5 kHz on the right, amplitude 0.5, delivered like
//! WASAPI delivers captured audio: float32 stereo in 10 ms blocks, paced in
//! real time (v1 test_tone.cpp). Used by the CLI test-tone option, the
//! emulator tests and the receiver test, where a steady source keeps the
//! measurement independent of what the PC happens to play (loopback capture
//! delivers nothing while it is silent).
//!
//! Four channels instead ([`set_quad`], the CLI's --test-tone-quad, for
//! the Auracast matrix layouts): front left / right and back left / right
//! (mask [`QUAD_MASK`]), each its own tone ([`QUAD_HZ`]) in beeps: channel
//! n (1-4) beeps n times every [`QUAD_CYCLE_S`] s, so a listener tells by
//! ear which channel a speaker plays.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use crate::{Format, SampleFormat, Shared, Sink};

pub const LEFT_HZ: f64 = 1000.0;
pub const RIGHT_HZ: f64 = 1500.0;
pub const AMPLITUDE: f64 = 0.5;
pub const BLOCK: Duration = Duration::from_millis(10);

/// The four-channel tone: the frequency of FL, FR, BL, BR
pub const QUAD_HZ: [f64; 4] = [500.0, 750.0, 1000.0, 1500.0];
/// Its speakers (WAVEFORMATEXTENSIBLE dwChannelMask): FL FR BL BR
pub const QUAD_MASK: u32 = 0x33;
/// Its cycle: channel n beeps n times, then silence until the next
pub const QUAD_CYCLE_S: u32 = 2;
/// A beep and the gap after it
const BEEP_MS: u32 = 150;
/// Fade in and out of a beep (no clicks)
const RAMP_MS: u32 = 5;

static QUAD: AtomicBool = AtomicBool::new(false);

/// The test tone in four channels from the next source opened
pub fn set_quad(on: bool) {
    QUAD.store(on, Ordering::Relaxed);
}

pub(crate) fn quad() -> bool {
    QUAD.load(Ordering::Relaxed)
}

pub(crate) fn format(sample_rate: u32) -> Format {
    Format { sample_rate, channels: if quad() { 4 } else { 2 }, sample_format: SampleFormat::F32 }
}

/// The level (0..1) of quad channel `ch` (0-3) `ms` into its cycle
fn beep_level(ch: usize, ms: f64) -> f64 {
    let period = f64::from(2 * BEEP_MS);
    let k = (ms / period).floor();
    if k >= (ch + 1) as f64 {
        return 0.0;
    }
    let into = ms - k * period;
    let beep = f64::from(BEEP_MS);
    let ramp = f64::from(RAMP_MS);
    if into >= beep {
        0.0
    } else if into < ramp {
        into / ramp
    } else if into > beep - ramp {
        (beep - into) / ramp
    } else {
        1.0
    }
}

/// The tone's samples, block after block
#[derive(Debug)]
pub(crate) struct ToneGen {
    sample_rate: u32,
    /// The four-channel beeps ([`set_quad`])
    quad: bool,
    /// Frame index within the current second. Both frequencies are whole
    /// numbers of Hz, so the signal repeats every second exactly; wrapping
    /// here keeps `t` small and the sine precise however long it runs.
    n: u32,
}

impl ToneGen {
    pub fn new(sample_rate: u32) -> ToneGen {
        ToneGen { sample_rate, quad: false, n: 0 }
    }

    /// The four-channel beeps (`out` of [`ToneGen::fill`] in 16-byte frames)
    pub fn new_quad(sample_rate: u32) -> ToneGen {
        ToneGen { sample_rate, quad: true, n: 0 }
    }

    /// Frames per 10 ms block (v1: sample_rate / 100)
    pub fn block_frames(&self) -> usize {
        (self.sample_rate / 100) as usize
    }

    /// Fill `out` with interleaved stereo f32 samples (little-endian bytes,
    /// as WASAPI delivers them); `out.len()` must be a multiple of 8
    pub fn fill(&mut self, out: &mut [u8]) {
        const TWO_PI: f64 = std::f64::consts::TAU;
        let rate = self.sample_rate as f64;
        if self.quad {
            // Whole numbers of Hz and a cycle of whole seconds: wrapping at
            // the cycle keeps every tone's phase
            let cycle = self.sample_rate * QUAD_CYCLE_S;
            for frame in out.as_chunks_mut::<16>().0 {
                let t = self.n as f64 / rate;
                for (ch, hz) in QUAD_HZ.iter().enumerate() {
                    let v = (AMPLITUDE * beep_level(ch, t * 1000.0) * (TWO_PI * hz * t).sin()) as f32;
                    frame[ch * 4..ch * 4 + 4].copy_from_slice(&v.to_le_bytes());
                }
                self.n += 1;
                if self.n >= cycle {
                    self.n = 0;
                }
            }
            return;
        }
        for frame in out.as_chunks_mut::<8>().0 {
            let t = self.n as f64 / rate;
            let l = (AMPLITUDE * (TWO_PI * LEFT_HZ * t).sin()) as f32;
            let r = (AMPLITUDE * (TWO_PI * RIGHT_HZ * t).sin()) as f32;
            frame[..4].copy_from_slice(&l.to_le_bytes());
            frame[4..].copy_from_slice(&r.to_le_bytes());
            self.n += 1;
            if self.n >= self.sample_rate {
                self.n = 0;
            }
        }
    }
}

/// If the thread falls further behind than this (system sleep, debugger),
/// it restarts its clock instead of catching up with a burst of blocks
const MAX_LAG: Duration = Duration::from_millis(200);

/// Body of the test tone thread
pub(crate) fn run(sample_rate: u32, mut sink: Box<dyn Sink>, shared: Arc<Shared>) {
    // Multimedia priority like the WASAPI capture thread it stands in for:
    // at normal priority a busy PC can hold it back for tens of ms
    let _mmcss = crate::register_mmcss();
    let fmt = format(sample_rate);
    let mut tone = if fmt.channels == 4 { ToneGen::new_quad(sample_rate) } else { ToneGen::new(sample_rate) };
    let frames = tone.block_frames();
    let mut buf = vec![0u8; frames * fmt.bytes_per_frame()];
    let mut next = Instant::now();
    tracing::debug!(
        "test tone: {sample_rate} Hz, {} channel(s), {frames} frames per {} ms block",
        fmt.channels,
        BLOCK.as_millis()
    );
    while !shared.stop.load(Ordering::Acquire) {
        tone.fill(&mut buf);
        sink.audio(&buf, frames, fmt);
        shared.frames.fetch_add(frames as u64, Ordering::Relaxed);
        next += BLOCK;
        let now = Instant::now();
        if now < next {
            // std's sleep uses a high-resolution waitable timer on Windows
            // 10 1803+, so v1's timeBeginPeriod(1) is not needed for
            // 10 ms pacing
            thread::sleep(next - now);
        } else if now - next > MAX_LAG {
            tracing::debug!("test tone: {} ms behind, restarting the clock", (now - next).as_millis());
            next = now;
        }
    }
    tracing::debug!("test tone: stopped after {} frames", shared.frames.load(Ordering::Relaxed));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples(bytes: &[u8]) -> Vec<f32> {
        bytes.as_chunks::<4>().0.iter().map(|&c| f32::from_le_bytes(c)).collect()
    }

    #[test]
    fn first_samples_match_v1() {
        let mut g = ToneGen::new(48_000);
        assert_eq!(g.block_frames(), 480);
        let mut buf = vec![0u8; 480 * 8];
        g.fill(&mut buf);
        let s = samples(&buf);
        for (i, frame) in s.as_chunks::<2>().0.iter().enumerate() {
            let t = i as f64 / 48_000.0;
            let l = (0.5 * (std::f64::consts::TAU * 1000.0 * t).sin()) as f32;
            let r = (0.5 * (std::f64::consts::TAU * 1500.0 * t).sin()) as f32;
            assert_eq!(frame[0], l, "left {i}");
            assert_eq!(frame[1], r, "right {i}");
        }
    }

    #[test]
    fn amplitude_and_frequency() {
        let rate = 44_100;
        let mut g = ToneGen::new(rate);
        let mut buf = vec![0u8; rate as usize * 8]; // one second
        g.fill(&mut buf);
        let s = samples(&buf);
        let left: Vec<f32> = s.iter().step_by(2).copied().collect();
        let right: Vec<f32> = s.iter().skip(1).step_by(2).copied().collect();
        let peak = |v: &[f32]| v.iter().fold(0f32, |m, x| m.max(x.abs()));
        assert!((peak(&left) - 0.5).abs() < 1e-3);
        assert!((peak(&right) - 0.5).abs() < 1e-3);
        // Rising zero crossings in one second = frequency
        let crossings = |v: &[f32]| v.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count() as i64;
        assert!((crossings(&left) - 1000).abs() <= 1);
        assert!((crossings(&right) - 1500).abs() <= 1);
    }

    #[test]
    fn quad_channels_beep_their_number_of_times_at_their_own_tone() {
        let rate = 48_000;
        let mut g = ToneGen::new_quad(rate);
        let mut buf = vec![0u8; (rate * QUAD_CYCLE_S) as usize * 16];
        g.fill(&mut buf);
        let s = samples(&buf);
        for (ch, hz) in QUAD_HZ.iter().enumerate() {
            let v: Vec<f32> = s.iter().skip(ch).step_by(4).copied().collect();
            // Beeps: runs of sound split by silence longer than a period
            let ms = |i: usize| i as f64 * 1000.0 / f64::from(rate);
            let loud: Vec<usize> = (0..v.len()).filter(|&i| v[i].abs() > 0.05).collect();
            let beeps = 1 + loud.windows(2).filter(|w| ms(w[1] - w[0]) > 50.0).count();
            assert_eq!(beeps, ch + 1, "channel {ch}");
            assert!(ms(*loud.last().unwrap()) < f64::from((ch as u32 + 1) * 2 * BEEP_MS));
            // Its own tone: rising zero crossings within the first beep
            let first: Vec<f32> = v[..(rate as usize * 100 / 1000)].to_vec();
            let crossings = first.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count() as f64;
            assert!((crossings - hz / 10.0).abs() <= 2.0, "channel {ch}: {crossings}");
            let peak = v.iter().fold(0f32, |m, x| m.max(x.abs()));
            assert!((peak - 0.5).abs() < 1e-3);
        }
    }

    #[test]
    fn phase_continues_across_blocks_and_seconds() {
        // Block by block over 2.5 s equals one long fill
        let rate = 48_000;
        let mut a = ToneGen::new(rate);
        let mut b = ToneGen::new(rate);
        let mut long = vec![0u8; 120_000 * 8];
        a.fill(&mut long);
        let mut pieces = Vec::new();
        let mut block = vec![0u8; 480 * 8];
        for _ in 0..250 {
            b.fill(&mut block);
            pieces.extend_from_slice(&block);
        }
        assert_eq!(long, pieces);
        // and the second second repeats the first exactly
        assert_eq!(long[..48_000 * 8], long[48_000 * 8..96_000 * 8]);
    }
}
