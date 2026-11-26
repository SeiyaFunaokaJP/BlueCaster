//! The ASHA encode thread (hearing aids, proof of concept):
//!
//! capture thread --[`CaptureInput`] (the same SPSC byte ring as A2DP)-->
//! ASHA encode thread --[`AshaOut`] (bt-host's `AshaSink`, one per hearing
//! aid)--> BTstack thread (an LE credit based channel each)
//!
//! Any captured PCM is downmixed to stereo, low-passed below 8 kHz and
//! resampled to 16 kHz when the capture runs faster, cut into 20 ms frames
//! and encoded with G.722 once per hearing aid: the left one takes the left
//! side, the right one the right side, a single one the average of both.
//! Every hearing aid gets one SDU per frame, a sequence counter byte (the
//! same on both sides) and 160 bytes of G.722.
//!
//! A delay (whole milliseconds) can be put before the encoder: with a
//! hearing aid on one ear and another Bluetooth device on the other
//! (bimodal), the two are heard together only when the faster path waits
//! for the slower one.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use bt_host::{ASHA_SDU_BYTES, AshaSink, LinkStats};
use codec::g722;

use crate::le::{LinearResampler, downmix, input_sample, to_i16};
use crate::{BufferPreset, CaptureInput, Control, InputFormat};

/// The rate of G.722
pub const ASHA_RATE: u32 = 16_000;
/// Samples of one 20 ms frame
pub const FRAME_SAMPLES: usize = 320;
/// The longest delay taken (bimodal alignment)
pub const MAX_DELAY_MS: u32 = 500;
const MAX_INPUT: usize = 8;

/// Where the SDUs of one hearing aid go
pub trait AshaOut: Send + 'static {
    fn push(&self, sdu: Vec<u8>);
}

impl AshaOut for AshaSink {
    fn push(&self, sdu: Vec<u8>) {
        AshaSink::push(self, sdu);
    }
}

/// What a hearing aid plays
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AshaChannel {
    Left,
    Right,
    /// The average of both sides (a single hearing aid)
    Mono,
}

/// A windowed-sinc low-pass for stereo, before the audio is resampled to a
/// lower rate
struct LowPass {
    taps: Vec<f32>,
    history: Vec<(f32, f32)>,
    at: usize,
}

impl LowPass {
    /// Passes up to `cutoff_hz` of audio at `rate`
    fn new(rate: u32, cutoff_hz: f32) -> LowPass {
        const TAPS: usize = 63;
        let fc = cutoff_hz / rate as f32;
        let mid = (TAPS - 1) as f32 / 2.0;
        let mut taps: Vec<f32> = (0..TAPS)
            .map(|n| {
                let x = n as f32 - mid;
                let sinc = if x == 0.0 {
                    2.0 * fc
                } else {
                    (2.0 * std::f32::consts::PI * fc * x).sin() / (std::f32::consts::PI * x)
                };
                // Hamming window
                sinc * (0.54 - 0.46 * (2.0 * std::f32::consts::PI * n as f32 / (TAPS - 1) as f32).cos())
            })
            .collect();
        let sum: f32 = taps.iter().sum();
        taps.iter_mut().for_each(|t| *t /= sum);
        LowPass { taps, history: vec![(0.0, 0.0); TAPS], at: 0 }
    }

    fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        let n = self.history.len();
        self.history[self.at] = (l, r);
        self.at = (self.at + 1) % n;
        let (mut ol, mut or) = (0.0, 0.0);
        for (i, t) in self.taps.iter().enumerate() {
            let (hl, hr) = self.history[(self.at + i) % n];
            ol += hl * t;
            or += hr * t;
        }
        (ol, or)
    }

    fn reset(&mut self) {
        self.history.fill((0.0, 0.0));
    }
}

struct Aid {
    channel: AshaChannel,
    encoder: g722::Encoder,
    out: Box<dyn AshaOut>,
}

/// The 16 kHz side: delay, framing, G.722, one SDU per hearing aid
struct Framer {
    aids: Vec<Aid>,
    /// Stereo samples waiting out the delay (as long as the delay, always)
    delay: VecDeque<(i16, i16)>,
    left: Vec<i16>,
    right: Vec<i16>,
    mono: Vec<i16>,
    sequence: u8,
    frames: u64,
}

impl Framer {
    fn add(&mut self, l: f32, r: f32) {
        let mut s = (to_i16(l), to_i16(r));
        if !self.delay.is_empty() {
            self.delay.push_back(s);
            s = self.delay.pop_front().unwrap_or((0, 0));
        }
        self.left.push(s.0);
        self.right.push(s.1);
        if self.left.len() < FRAME_SAMPLES {
            return;
        }
        self.mono.clear();
        self.mono.extend(self.left.iter().zip(&self.right).map(|(l, r)| ((i32::from(*l) + i32::from(*r)) / 2) as i16));
        for aid in &mut self.aids {
            let pcm = match aid.channel {
                AshaChannel::Left => &self.left,
                AshaChannel::Right => &self.right,
                AshaChannel::Mono => &self.mono,
            };
            let mut sdu = vec![0u8; ASHA_SDU_BYTES];
            sdu[0] = self.sequence;
            aid.encoder.encode(pcm, &mut sdu[1..]);
            aid.out.push(sdu);
        }
        self.sequence = self.sequence.wrapping_add(1);
        self.frames += 1;
        self.left.clear();
        self.right.clear();
    }
}

/// Downmix, resampling and G.722 encoding for the hearing aids (no thread;
/// the ASHA encode thread drives it, tests call it directly)
pub struct AshaEncoder {
    input: InputFormat,
    lowpass: Option<LowPass>,
    resampler: LinearResampler,
    framer: Framer,
    delay_samples: usize,
}

impl AshaEncoder {
    /// `aids`: what each hearing aid plays and where its SDUs go;
    /// `delay_ms`: how much later the hearing aids get the audio
    pub fn new(
        input: InputFormat,
        aids: Vec<(AshaChannel, Box<dyn AshaOut>)>,
        delay_ms: u32,
    ) -> Result<AshaEncoder, String> {
        if input.channels == 0 || input.sample_rate == 0 {
            return Err(format!("capture format {} Hz, {} channels", input.sample_rate, input.channels));
        }
        if aids.is_empty() {
            return Err("no hearing aid".into());
        }
        let delay_samples = (delay_ms.min(MAX_DELAY_MS) * ASHA_RATE / 1000) as usize;
        Ok(AshaEncoder {
            input,
            // Up to 7 kHz, G.722's range: nothing above the new 8 kHz limit
            // is left to fold back
            lowpass: (input.sample_rate > ASHA_RATE).then(|| LowPass::new(input.sample_rate, 7000.0)),
            resampler: LinearResampler::new(input.sample_rate, ASHA_RATE),
            framer: Framer {
                aids: aids
                    .into_iter()
                    .map(|(channel, out)| Aid { channel, encoder: g722::Encoder::new(), out })
                    .collect(),
                delay: std::iter::repeat_n((0, 0), delay_samples).collect(),
                left: Vec::with_capacity(FRAME_SAMPLES),
                right: Vec::with_capacity(FRAME_SAMPLES),
                mono: Vec::with_capacity(FRAME_SAMPLES),
                sequence: 0,
                frames: 0,
            },
            delay_samples,
        })
    }

    /// 20 ms frames encoded so far
    pub fn frames(&self) -> u64 {
        self.framer.frames
    }

    /// Captured audio (whole frames of the input format): every 20 ms it
    /// completes goes out as one SDU per hearing aid
    pub fn push(&mut self, raw: &[u8]) {
        let in_ch = usize::from(self.input.channels);
        let t = self.input.sample;
        let used = in_ch.min(MAX_INPUT);
        let mut ch = [0.0f32; MAX_INPUT];
        let Self { lowpass, resampler, framer, .. } = self;
        for f in 0..raw.len() / (in_ch * t.bytes()) {
            for (c, v) in ch[..used].iter_mut().enumerate() {
                *v = input_sample(raw, f * in_ch + c, t);
            }
            let (mut l, mut r) = downmix(&ch[..used]);
            if let Some(lp) = lowpass.as_mut() {
                (l, r) = lp.process(l, r);
            }
            resampler.process(l, r, &mut |l, r| framer.add(l, r));
        }
    }

    /// Drops the partly filled frame and the filters' history (buffered
    /// audio of an old stream); the encoders and the counter go on
    pub fn reset(&mut self) {
        self.framer.left.clear();
        self.framer.right.clear();
        self.framer.delay.iter_mut().for_each(|s| *s = (0, 0));
        debug_assert_eq!(self.framer.delay.len(), self.delay_samples);
        self.resampler.reset();
        if let Some(lp) = self.lowpass.as_mut() {
            lp.reset();
        }
    }
}

/// Setup of the ASHA encode thread
pub struct AshaPipelineConfig {
    pub input: InputFormat,
    /// How much later the hearing aids get the audio (bimodal), at most
    /// [`MAX_DELAY_MS`]
    pub delay_ms: u32,
}

/// A running ASHA encode thread
pub struct AshaPipeline {
    control: Arc<Control>,
    thread: Option<JoinHandle<()>>,
}

impl AshaPipeline {
    /// Starts the ASHA encode thread; feed it through the returned
    /// [`CaptureInput`] as the A2DP pipeline
    pub fn start(
        config: AshaPipelineConfig,
        aids: Vec<(AshaChannel, Box<dyn AshaOut>)>,
        stats: Arc<LinkStats>,
    ) -> Result<(AshaPipeline, CaptureInput), String> {
        let AshaPipelineConfig { input, delay_ms } = config;
        let encoder = AshaEncoder::new(input, aids, delay_ms)?;
        let ring = BufferPreset::Standard.ring_bytes(&input);
        let (producer, consumer) = rtrb::RingBuffer::new(ring);
        let control = Control::new();
        let thread = {
            let control = control.clone();
            std::thread::Builder::new()
                .name("asha-encode".into())
                .spawn(move || asha_encode_thread(encoder, consumer, &control))
                .map_err(|e| format!("spawning the ASHA encode thread: {e}"))?
        };
        let capture = CaptureInput {
            producer,
            encode_thread: thread.thread().clone(),
            stats,
            control: control.clone(),
            frame_bytes: input.frame_bytes(),
        };
        Ok((AshaPipeline { control, thread: Some(thread) }, capture))
    }

    /// Stops and joins the encode thread
    pub fn stop(mut self) {
        self.stop_thread();
    }

    fn stop_thread(&mut self) {
        if let Some(t) = self.thread.take() {
            self.control.stop.store(true, Ordering::Release);
            t.thread().unpark();
            let _ = t.join();
        }
    }
}

impl Drop for AshaPipeline {
    fn drop(&mut self) {
        self.stop_thread();
    }
}

fn asha_encode_thread(mut encoder: AshaEncoder, mut ring: rtrb::Consumer<u8>, control: &Control) {
    let _priority = crate::thread_priority();
    let frame_bytes = encoder.input.frame_bytes();
    // ~10 ms rounds, at least 128 frames (as the A2DP encode thread)
    let round_frames = (encoder.input.sample_rate as usize / 100).max(128);
    let mut read_buf = vec![0u8; round_frames * frame_bytes];
    let mut overflow_report = Instant::now();
    tracing::debug!("ASHA encode thread: running, rounds of {round_frames} frames");
    while !control.stop.load(Ordering::Acquire) {
        if control.reset.swap(false, Ordering::AcqRel) {
            let n = ring.slots();
            if let Ok(chunk) = ring.read_chunk(n) {
                chunk.commit_all();
            }
            encoder.reset();
        }
        control.report_overflows("ASHA encode thread", &mut overflow_report);
        let available = ring.slots() / frame_bytes;
        if available == 0 {
            std::thread::park_timeout(Duration::from_millis(10));
            continue;
        }
        let bytes = available.min(round_frames) * frame_bytes;
        if let Ok(chunk) = ring.read_chunk(bytes) {
            let (a, b) = chunk.as_slices();
            read_buf[..a.len()].copy_from_slice(a);
            read_buf[a.len()..bytes].copy_from_slice(b);
            chunk.commit_all();
        }
        encoder.push(&read_buf[..bytes]);
    }
    tracing::debug!("ASHA encode thread: stopped after {} frames", encoder.frames());
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::SampleType;

    #[derive(Clone, Default)]
    struct Collect(Arc<Mutex<Vec<Vec<u8>>>>);

    impl AshaOut for Collect {
        fn push(&self, sdu: Vec<u8>) {
            self.0.lock().unwrap().push(sdu);
        }
    }

    fn stereo_i16(rate: u32, ms: u32, left_hz: f64, right_hz: f64) -> Vec<u8> {
        let mut raw = Vec::new();
        for n in 0..(rate * ms / 1000) {
            let t = f64::from(n) / f64::from(rate);
            for hz in [left_hz, right_hz] {
                let s = if hz == 0.0 { 0.0 } else { 8000.0 * (2.0 * std::f64::consts::PI * hz * t).sin() };
                raw.extend_from_slice(&(s as i16).to_le_bytes());
            }
        }
        raw
    }

    fn input(rate: u32) -> InputFormat {
        InputFormat { sample_rate: rate, channels: 2, sample: SampleType::I16 }
    }

    fn decode(sdus: &[Vec<u8>]) -> Vec<i16> {
        let mut decoder = g722::Decoder::new();
        let mut pcm = Vec::new();
        for sdu in sdus {
            decoder.decode(&sdu[1..], &mut pcm);
        }
        pcm
    }

    fn rms(pcm: &[i16]) -> f64 {
        (pcm.iter().map(|s| f64::from(*s) * f64::from(*s)).sum::<f64>() / pcm.len().max(1) as f64).sqrt()
    }

    #[test]
    fn one_sdu_per_20_ms_and_hearing_aid_with_the_same_counter() {
        let (l, r) = (Collect::default(), Collect::default());
        let aids: Vec<(AshaChannel, Box<dyn AshaOut>)> =
            vec![(AshaChannel::Left, Box::new(l.clone())), (AshaChannel::Right, Box::new(r.clone()))];
        let mut e = AshaEncoder::new(input(16_000), aids, 0).unwrap();
        e.push(&stereo_i16(16_000, 1000, 440.0, 880.0));
        let (l, r) = (l.0.lock().unwrap(), r.0.lock().unwrap());
        assert_eq!((l.len(), r.len()), (50, 50));
        assert!(l.iter().all(|s| s.len() == ASHA_SDU_BYTES));
        for (n, (a, b)) in l.iter().zip(r.iter()).enumerate() {
            assert_eq!((a[0], b[0]), (n as u8, n as u8));
        }
        assert_ne!(l[10][1..], r[10][1..], "each side has its own audio");
    }

    #[test]
    fn each_side_gets_its_channel_and_one_hearing_aid_both() {
        let (l, r, m) = (Collect::default(), Collect::default(), Collect::default());
        let aids: Vec<(AshaChannel, Box<dyn AshaOut>)> = vec![
            (AshaChannel::Left, Box::new(l.clone())),
            (AshaChannel::Right, Box::new(r.clone())),
            (AshaChannel::Mono, Box::new(m.clone())),
        ];
        let mut e = AshaEncoder::new(input(48_000), aids, 0).unwrap();
        // A tone on the left only
        e.push(&stereo_i16(48_000, 1000, 1000.0, 0.0));
        let (l, r, m) = (decode(&l.0.lock().unwrap()), decode(&r.0.lock().unwrap()), decode(&m.0.lock().unwrap()));
        assert_eq!(l.len(), 16_000);
        let (l, r, m) = (rms(&l[4000..]), rms(&r[4000..]), rms(&m[4000..]));
        assert!(l > 4000.0 && l < 7000.0, "left {l}");
        assert!(r < 100.0, "right {r}");
        assert!(m > l * 0.4 && m < l * 0.6, "mono {m} of left {l}");
    }

    #[test]
    fn the_delay_holds_the_audio_back() {
        let out = Collect::default();
        let mut e = AshaEncoder::new(input(16_000), vec![(AshaChannel::Mono, Box::new(out.clone()))], 100).unwrap();
        e.push(&stereo_i16(16_000, 400, 1000.0, 1000.0));
        let pcm = decode(&out.0.lock().unwrap());
        assert!(rms(&pcm[..1400]) < 100.0, "silence first: {}", rms(&pcm[..1400]));
        assert!(rms(&pcm[2400..]) > 4000.0, "then the tone: {}", rms(&pcm[2400..]));
    }

    #[test]
    fn audio_above_the_new_range_is_removed_before_resampling() {
        let out = Collect::default();
        let mut e = AshaEncoder::new(input(48_000), vec![(AshaChannel::Mono, Box::new(out.clone()))], 0).unwrap();
        // 12 kHz would fold back to 4 kHz at 16 kHz
        e.push(&stereo_i16(48_000, 500, 12_000.0, 12_000.0));
        let pcm = decode(&out.0.lock().unwrap());
        assert!(rms(&pcm[2000..]) < 300.0, "{}", rms(&pcm[2000..]));
    }
}
