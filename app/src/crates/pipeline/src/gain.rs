//! A volume applied to the audio before encoding (the Auracast broadcast's:
//! its receivers have no volume of ours to set). One [`GainHandle`] is shared
//! with the encode thread, which moves to a new gain within a few
//! milliseconds ([`GainRamp`]) so a change does not click.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

/// How long a change of gain takes (the ramp's time constant)
const RAMP_SECONDS: f32 = 0.005;
/// Closer than this to the gain asked for: there
const SNAP: f32 = 1e-4;

/// The gain asked for (an amplitude factor, 0.0..=1.0), shared with the
/// encode thread; 1.0 (unchanged) at first
#[derive(Clone, Debug)]
pub struct GainHandle(Arc<AtomicU32>);

impl Default for GainHandle {
    fn default() -> Self {
        GainHandle(Arc::new(AtomicU32::new(1.0f32.to_bits())))
    }
}

impl GainHandle {
    /// The gain from now on (clamped to 0.0..=1.0)
    pub fn set(&self, gain: f32) {
        let gain = if gain.is_finite() { gain.clamp(0.0, 1.0) } else { 1.0 };
        self.0.store(gain.to_bits(), Ordering::Relaxed);
    }

    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
}

/// The encode thread's side of a [`GainHandle`]: the gain each frame gets,
/// moving smoothly to the one asked for
#[derive(Debug)]
pub struct GainRamp {
    handle: GainHandle,
    current: f32,
    target: f32,
    /// The share of the distance to the target one frame covers
    step: f32,
}

impl GainRamp {
    /// For audio at `sample_rate`; starts at the handle's gain
    pub fn new(handle: GainHandle, sample_rate: u32) -> GainRamp {
        let gain = handle.get();
        let frames = (sample_rate.max(1) as f32 * RAMP_SECONDS).max(1.0);
        GainRamp { handle, current: gain, target: gain, step: 1.0 - (-1.0 / frames).exp() }
    }

    /// Takes the gain asked for now (once per block of audio)
    pub fn refresh(&mut self) {
        let target = self.handle.get();
        if target != self.target {
            tracing::debug!("broadcast volume: gain {:.3} -> {target:.3}", self.target);
            self.target = target;
        }
    }

    /// One frame of `lanes` (every channel the same gain)
    pub fn apply(&mut self, lanes: &mut [f32]) {
        if self.current != self.target {
            self.current += (self.target - self.current) * self.step;
            if (self.target - self.current).abs() < SNAP {
                self.current = self.target;
            }
        }
        if self.current != 1.0 {
            lanes.iter_mut().for_each(|v| *v *= self.current);
        }
    }

    /// Straight to the gain asked for (a new stream: nothing to ramp from)
    pub fn reset(&mut self) {
        self.refresh();
        self.current = self.target;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_handle_starts_at_full_and_clamps() {
        let h = GainHandle::default();
        assert_eq!(h.get(), 1.0);
        h.set(0.25);
        assert_eq!(h.get(), 0.25);
        h.set(3.0);
        assert_eq!(h.get(), 1.0);
        h.set(-1.0);
        assert_eq!(h.get(), 0.0);
        h.set(f32::NAN);
        assert_eq!(h.get(), 1.0);
    }

    #[test]
    fn full_gain_leaves_the_audio_alone() {
        let mut r = GainRamp::new(GainHandle::default(), 48_000);
        let mut f = [0.5, -0.25, 1.0, 0.0];
        r.apply(&mut f);
        assert_eq!(f, [0.5, -0.25, 1.0, 0.0]);
    }

    #[test]
    fn a_change_ramps_in_a_few_milliseconds() {
        let h = GainHandle::default();
        let mut r = GainRamp::new(h.clone(), 48_000);
        h.set(0.0);
        r.refresh();
        let mut seen = Vec::new();
        for _ in 0..48 * 50 {
            let mut f = [1.0f32; 2];
            r.apply(&mut f);
            assert_eq!(f[0], f[1]);
            seen.push(f[0]);
        }
        // No jump: the first frame is still near full, each one lower
        assert!(seen[0] > 0.9, "{}", seen[0]);
        assert!(seen.windows(2).all(|w| w[1] <= w[0]));
        // 5 ms in: about a third left; 50 ms in: silent
        assert!((seen[240] - (-1.0f32).exp()).abs() < 0.02, "{}", seen[240]);
        assert_eq!(*seen.last().unwrap(), 0.0);
    }

    #[test]
    fn a_reset_goes_straight_to_the_gain() {
        let h = GainHandle::default();
        let mut r = GainRamp::new(h.clone(), 48_000);
        h.set(0.5);
        r.reset();
        let mut f = [1.0f32];
        r.apply(&mut f);
        assert_eq!(f[0], 0.5);
    }
}
