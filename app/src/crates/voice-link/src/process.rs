//! The microphone's processing (gain, noise suppression, noise gate,
//! limiter; `dsp::mic`) on the PCM as it leaves the render queue.
//!
//! [`MicDsp`] holds the settings: the core changes them at any time and the
//! next samples use them (live, no reconnect). [`Processor`] runs the chain
//! where the queue is read: the render thread ("voice-render", MMCSS) or the
//! WAV writer, never the BTstack thread that pushes (the SCO sends wait on
//! it; noise suppression there cost about 2 % of the packets sent in the
//! emulator). With everything off the samples pass by untouched.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use dsp::mic::MicChain;
pub use dsp::mic::MicSettings;

#[derive(Debug)]
struct Shared {
    gain_db: AtomicU32,
    /// The strength in thousandths (0: off)
    suppression: AtomicU32,
    gate_db: AtomicU32,
}

/// The processing settings in force; clones share them
#[derive(Clone, Debug)]
pub struct MicDsp(Arc<Shared>);

impl Default for MicDsp {
    fn default() -> Self {
        let p = MicDsp(Arc::new(Shared {
            gain_db: AtomicU32::new(0),
            suppression: AtomicU32::new(0),
            gate_db: AtomicU32::new(0),
        }));
        p.set(MicSettings::default());
        p
    }
}

impl MicDsp {
    pub fn new() -> MicDsp {
        MicDsp::default()
    }

    /// From the next push on (clamped to the chain's ranges)
    pub fn set(&self, s: MicSettings) {
        let s = s.clamped();
        let r = Ordering::Relaxed;
        self.0.gain_db.store(s.gain_db.to_bits(), r);
        self.0.suppression.store((s.suppression * 1000.0).round() as u32, r);
        self.0.gate_db.store(s.gate_db.to_bits(), r);
    }

    pub fn get(&self) -> MicSettings {
        let r = Ordering::Relaxed;
        MicSettings {
            gain_db: f32::from_bits(self.0.gain_db.load(r)),
            suppression: self.0.suppression.load(r) as f32 / 1000.0,
            gate_db: f32::from_bits(self.0.gate_db.load(r)),
        }
    }
}

/// Samples converted per round (the buffer's size: a block of any length
/// goes through in pieces, nothing is allocated while processing)
const CHUNK: usize = 1024;

/// The chain at one rate with its buffer
pub(crate) struct Processor {
    settings: MicDsp,
    chain: Box<MicChain>,
    float: Vec<f32>,
}

impl Processor {
    pub fn new(settings: MicDsp, rate: u32) -> Processor {
        Processor { settings, chain: Box::new(MicChain::new(rate)), float: vec![0.0; CHUNK] }
    }

    /// Passes `pcm` through the chain in place; false (and `pcm` untouched)
    /// when the settings do nothing
    pub fn process(&mut self, pcm: &mut [i16]) -> bool {
        let settings = self.settings.get();
        if !settings.is_active() {
            // The chain notes it stopped: it starts clean next time
            self.chain.process(&mut [], &settings);
            return false;
        }
        for piece in pcm.chunks_mut(CHUNK) {
            let f = &mut self.float[..piece.len()];
            for (d, &s) in f.iter_mut().zip(piece.iter()) {
                *d = f32::from(s) / 32768.0;
            }
            self.chain.process(f, &settings);
            for (d, &s) in piece.iter_mut().zip(f.iter()) {
                *d = (s * 32768.0).round().clamp(-32768.0, 32767.0) as i16;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip() {
        let p = MicDsp::new();
        assert_eq!(p.get(), MicSettings::default());
        let s = MicSettings { gain_db: 6.0, suppression: 0.7, gate_db: -45.0 };
        p.clone().set(s);
        assert_eq!(p.get(), s);
        p.set(MicSettings { suppression: 0.0, ..s });
        assert_eq!(p.get(), MicSettings { suppression: 0.0, ..s });
    }

    #[test]
    fn off_leaves_the_samples_alone() {
        let mut proc = Processor::new(MicDsp::new(), 16_000);
        let mut pcm = [1, 2, 3];
        assert!(!proc.process(&mut pcm));
        assert_eq!(pcm, [1, 2, 3]);
    }

    #[test]
    fn long_blocks_go_through_in_pieces() {
        let settings = MicDsp::new();
        settings.set(MicSettings { gain_db: -6.0, ..MicSettings::default() });
        let mut proc = Processor::new(settings, 16_000);
        let mut pcm = vec![16_000i16; 5000];
        assert!(proc.process(&mut pcm));
        // Past the limiter's delay: all of it halved
        assert!(pcm[100..].iter().all(|&s| (s - 8019).abs() < 2), "{:?}", &pcm[4990..]);
    }
}
