//! The broadcast mixer (Auracast, developer mode), after MatrixMixer: the
//! output lanes (front left / right, rear left / right) are a matrix of the
//! input speakers, S' = M S, a column per input speaker and a row per
//! output. "Auto" computes the matrix from the input's speakers and a few
//! levels (centre, LFE, surround; optionally normalized so no row sums
//! above 1), else it is set cell by cell. Each output also has a delay (0
//! to [`MAX_DELAY_MS`], fractional: linear interpolation between samples)
//! and can be inverted. Auto sends the LFE to the outputs marked full range
//! (the fronts unless set). Solo and mute, per input and per output, are
//! for listening tests: the matrix stays as set, only what is heard
//! changes ([`heard_matrix`]).
//!
//! A 2.1 broadcast ([`Outputs::Sub`]) sends the third and fourth outputs
//! to a subwoofer: auto gives them what the fronts get, and a crossover
//! (Linkwitz-Riley, 24 dB per octave: two Butterworth sections) low passes
//! them and high passes the fronts at [`MixSettings::crossover_hz`], each
//! side on its own (the subwoofer gets the low end of the left and of the
//! right, and may mix them itself). With a mono subwoofer
//! ([`Outputs::SubMono`], [`Outputs::Center`]) the fourth output is the low
//! end of left and right together; 3.1 ([`Outputs::Center`]) has the
//! centre on the third output, high passed as the fronts.
//!
//! The control side sets [`MixSettings`] on a [`MixHandle`]; the encode
//! thread's [`Mixer`] takes them from its next block, without stopping the
//! broadcast (as the playback EQ). Nothing on the encode thread allocates
//! once the mixer is built.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use dsp::biquad::{Biquad, Coeffs};
use le_audio::plan::ChannelSource;

use crate::le::speaker::*;
use crate::le::{LANES, MAX_INPUT, MINUS_3DB, speakers};

/// The input columns: a speaker each
pub const INPUTS: usize = 8;
/// The speaker bits of the columns, in column order
pub const INPUT_SPEAKERS: [u32; INPUTS] =
    [FRONT_LEFT, FRONT_RIGHT, FRONT_CENTER, LOW_FREQUENCY, BACK_LEFT, BACK_RIGHT, SIDE_LEFT, SIDE_RIGHT];
/// Column names, as the UI shows them
pub const INPUT_NAMES: [&str; INPUTS] = ["FL", "FR", "FC", "LFE", "BL", "BR", "SL", "SR"];
/// Row names (the lanes)
pub const OUTPUT_NAMES: [&str; LANES] = ["FL", "FR", "RL", "RR"];
/// Longest delay of an output
pub const MAX_DELAY_MS: f32 = 500.0;
/// Highest gain of a cell, either sign (+12 dB)
pub const MAX_GAIN: f32 = 4.0;
/// The crossover of a 2.1 broadcast (Hz): its range and default
pub const CROSSOVER_MIN_HZ: f32 = 40.0;
pub const CROSSOVER_MAX_HZ: f32 = 500.0;
pub const CROSSOVER_DEFAULT_HZ: f32 = 80.0;
/// The centre into both fronts (dB) unless set
pub const CENTER_DEFAULT_DB: f32 = -3.0;
/// The LFE into both fronts (dB) unless set
pub const LFE_DEFAULT_DB: f32 = -6.0;
/// The outputs auto sends the LFE to unless set: the fronts
pub const FULL_RANGE_DEFAULT: [bool; LANES] = [true, true, false, false];

/// What the four outputs (lanes) of a broadcast feed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outputs {
    /// Only the fronts are sent: the rest folded into them
    Front,
    /// Front and rear speakers
    Rear,
    /// 2.1: the fronts, and the subwoofer's left and right on the third
    /// and fourth outputs
    Sub,
    /// 2.1, a mono subwoofer: the fronts, the subwoofer on the fourth
    /// output (the third unused)
    SubMono,
    /// 3.1: the fronts, the centre on the third output, a mono subwoofer
    /// on the fourth
    Center,
}

/// What a lane is to the crossover
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Band {
    /// As it is (no subwoofer, or unused)
    Full,
    /// A main speaker: high passed (unless full range)
    Main,
    /// The subwoofer: low passed
    Low,
}

impl Outputs {
    /// A subwoofer layout (the crossover runs)
    pub fn has_sub(self) -> bool {
        matches!(self, Outputs::Sub | Outputs::SubMono | Outputs::Center)
    }

    fn bands(self) -> [Band; LANES] {
        use Band::*;
        match self {
            Outputs::Front | Outputs::Rear => [Full; LANES],
            Outputs::Sub => [Main, Main, Low, Low],
            Outputs::SubMono => [Main, Main, Full, Low],
            Outputs::Center => [Main, Main, Main, Low],
        }
    }
}

/// The mixer's outputs for the channels a stream sends (`speakers`: it
/// mixes the four lanes, else only the fronts)
pub fn outputs_for(sources: impl Iterator<Item = ChannelSource>, speakers: bool) -> Outputs {
    if !speakers {
        return Outputs::Front;
    }
    let (mut center, mut mono_sub, mut sub) = (false, false, false);
    for s in sources {
        center |= s == ChannelSource::Center;
        mono_sub |= s == ChannelSource::SubMono;
        sub |= s.is_sub();
    }
    match (center, mono_sub, sub) {
        (true, _, _) => Outputs::Center,
        (_, true, _) => Outputs::SubMono,
        (_, _, true) => Outputs::Sub,
        _ => Outputs::Rear,
    }
}

/// A matrix: per output lane, the gain of each input column
pub type Matrix = [[f32; INPUTS]; LANES];

/// What the user set
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MixSettings {
    /// The matrix computed from the levels below (else `matrix` as set)
    pub auto: bool,
    /// Auto: the centre into both fronts (dB)
    pub center_db: f32,
    /// Auto: the LFE into both fronts (dB); with a subwoofer, into it alone
    pub lfe_db: f32,
    /// Auto: the back and side speakers (dB): into the rear outputs, or
    /// (a stream without rear outputs) into the fronts at a further -3 dB
    pub surround_db: f32,
    /// Auto: each row scaled down so its gains sum to 1 at most (no
    /// overflow, everything a little quieter)
    pub normalize: bool,
    /// The matrix when not auto
    pub matrix: Matrix,
    /// Per output: its delay (ms)
    pub delay_ms: [f32; LANES],
    /// Per output: polarity inverted
    pub invert: [bool; LANES],
    /// 2.1: where the subwoofer takes over (Hz)
    pub crossover_hz: f32,
    /// 2.1: the fronts play the full range (no high pass), for speakers
    /// with a low end of their own
    pub mains_full_range: bool,
    /// Auto without a subwoofer: per output, the LFE goes to it (at
    /// `lfe_db`)
    pub full_range: [bool; LANES],
    /// Listening tests: only the soloed inputs (columns) and outputs (rows)
    /// are heard when any is, the muted ones never
    pub solo_in: [bool; INPUTS],
    pub mute_in: [bool; INPUTS],
    pub solo_out: [bool; LANES],
    pub mute_out: [bool; LANES],
}

impl Default for MixSettings {
    fn default() -> Self {
        let mut s = MixSettings {
            auto: true,
            center_db: CENTER_DEFAULT_DB,
            lfe_db: LFE_DEFAULT_DB,
            surround_db: 0.0,
            normalize: false,
            matrix: [[0.0; INPUTS]; LANES],
            delay_ms: [0.0; LANES],
            invert: [false; LANES],
            crossover_hz: CROSSOVER_DEFAULT_HZ,
            mains_full_range: false,
            full_range: FULL_RANGE_DEFAULT,
            solo_in: [false; INPUTS],
            mute_in: [false; INPUTS],
            solo_out: [false; LANES],
            mute_out: [false; LANES],
        };
        s.matrix = auto_matrix(&[true; INPUTS], false, Outputs::Rear, &s);
        s
    }
}

impl MixSettings {
    /// Held to the ranges: gains within ±[`MAX_GAIN`], delays within 0 to
    /// [`MAX_DELAY_MS`], levels within -60 to +12 dB, NaN as 0
    pub fn held(mut self) -> MixSettings {
        let fin = |x: f32, lo: f32, hi: f32, nan: f32| if x.is_finite() { x.clamp(lo, hi) } else { nan };
        self.center_db = fin(self.center_db, -60.0, 12.0, CENTER_DEFAULT_DB);
        self.lfe_db = fin(self.lfe_db, -60.0, 12.0, LFE_DEFAULT_DB);
        self.surround_db = fin(self.surround_db, -60.0, 12.0, 0.0);
        for row in &mut self.matrix {
            for g in row.iter_mut() {
                *g = fin(*g, -MAX_GAIN, MAX_GAIN, 0.0);
            }
        }
        for d in &mut self.delay_ms {
            *d = fin(*d, 0.0, MAX_DELAY_MS, 0.0);
        }
        self.crossover_hz = fin(self.crossover_hz, CROSSOVER_MIN_HZ, CROSSOVER_MAX_HZ, CROSSOVER_DEFAULT_HZ);
        self
    }

    /// Any input or output soloed or muted
    pub fn monitoring(&self) -> bool {
        [&self.solo_in[..], &self.mute_in, &self.solo_out, &self.mute_out].iter().any(|v| v.contains(&true))
    }

    /// Without solo and mute (as saved)
    pub fn unmonitored(self) -> MixSettings {
        MixSettings {
            solo_in: [false; INPUTS],
            mute_in: [false; INPUTS],
            solo_out: [false; LANES],
            mute_out: [false; LANES],
            ..self
        }
    }

    /// Input column `c` is heard (not muted, soloed if any is)
    pub fn input_heard(&self, c: usize) -> bool {
        heard(&self.solo_in, &self.mute_in, c)
    }

    /// Output `l` is heard (not muted, soloed if any is)
    pub fn output_heard(&self, l: usize) -> bool {
        heard(&self.solo_out, &self.mute_out, l)
    }
}

fn heard(solo: &[bool], mute: &[bool], i: usize) -> bool {
    let soloed = !solo.contains(&true) || solo.get(i).copied().unwrap_or(false);
    soloed && !mute.get(i).copied().unwrap_or(false)
}

/// The matrix as heard: the cells of inputs and outputs not heard (solo,
/// mute) at 0
pub fn heard_matrix(m: &Matrix, s: &MixSettings) -> Matrix {
    std::array::from_fn(|l| std::array::from_fn(|c| if s.output_heard(l) && s.input_heard(c) { m[l][c] } else { 0.0 }))
}

fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// The column of a speaker bit (the front left / right of centre count as
/// front left / right); None for one without a column
pub fn speaker_column(bit: u32) -> Option<usize> {
    match bit {
        FRONT_LEFT_OF_CENTER => Some(0),
        FRONT_RIGHT_OF_CENTER => Some(1),
        b => INPUT_SPEAKERS.iter().position(|&s| s == b),
    }
}

/// The column of each interleaved input channel of a source of `channels`
/// with the speakers of `mask` (0: the usual ones of the count)
pub fn input_columns(channels: usize, mask: u32) -> [Option<usize>; MAX_INPUT] {
    let sp = speakers(channels, mask);
    std::array::from_fn(|i| if i < channels { speaker_column(sp[i]) } else { None })
}

/// The interleaved input channels that are the back centre speaker (6.1):
/// it has no column of its own and plays as the back left and right, at
/// -3 dB in each (as [`crate::le::speaker_matrix`] sends it to both rears)
pub fn back_center_inputs(channels: usize, mask: u32) -> [bool; MAX_INPUT] {
    let sp = speakers(channels, mask);
    std::array::from_fn(|i| i < channels && sp[i] == BACK_CENTER)
}

/// The speakers of the source that no column takes (they are not heard):
/// their mask bits
pub fn unused_speakers(channels: usize, mask: u32) -> u32 {
    let sp = speakers(channels, mask);
    sp.iter()
        .take(channels)
        .filter(|&&bit| bit != 0 && bit != BACK_CENTER && speaker_column(bit).is_none())
        .fold(0, |all, bit| all | bit)
}

/// The columns the source has (a back centre speaker counts as the back
/// left and right)
pub fn present_columns(channels: usize, mask: u32) -> [bool; INPUTS] {
    let mut present = [false; INPUTS];
    for c in input_columns(channels, mask).into_iter().flatten() {
        present[c] = true;
    }
    if back_center_inputs(channels, mask).contains(&true) {
        (present[4], present[5]) = (true, true);
    }
    present
}

/// The auto matrix for a source with the columns `present` (`mono`: a
/// single channel, played on every output) and the stream's outputs.
/// Fronts: front left / right as they are, the centre and the LFE into
/// both. Rear outputs: back and side make the rear (-3 dB each when a
/// source has both), a source without them plays its front on the rear
/// too. No rear outputs: back and side folded into the fronts at -3 dB
/// (ITU-R BS.775, as the stereo downmix). The LFE goes to the outputs
/// marked full range. 2.1: the subwoofer outputs get what the fronts get
/// (the crossover keeps their low end), and the LFE goes to the subwoofer
/// alone.
pub fn auto_matrix(present: &[bool; INPUTS], mono: bool, outputs: Outputs, s: &MixSettings) -> Matrix {
    let half = |a: &[f32; INPUTS], b: &[f32; INPUTS]| -> [f32; INPUTS] { std::array::from_fn(|c| 0.5 * (a[c] + b[c])) };
    // With a subwoofer the LFE is its alone, not the other speakers': as
    // loud as the source has it at the default LFE level, louder or quieter
    // with that level. The rows changed are normalized again.
    let lfe_to_sub = |m: &mut Matrix, subs: &[usize]| {
        if mono || !present[3] {
            return;
        }
        for (l, row) in m.iter_mut().enumerate() {
            let sub = subs.contains(&l);
            row[3] = if sub { db_to_lin(s.lfe_db - LFE_DEFAULT_DB).min(MAX_GAIN) } else { 0.0 };
            let sum: f32 = row.iter().map(|g| g.abs()).sum();
            if sub && s.normalize && sum > 1.0 {
                row.iter_mut().for_each(|g| *g /= sum);
            }
        }
    };
    match outputs {
        Outputs::Sub => {
            let mut m = auto_matrix(present, mono, Outputs::Front, s);
            (m[2], m[3]) = (m[0], m[1]);
            lfe_to_sub(&mut m, &[2, 3]);
            return m;
        }
        // The subwoofer: half of each front (a sound on both at once as
        // loud as on each)
        Outputs::SubMono => {
            let mut m = auto_matrix(present, mono, Outputs::Front, s);
            m[2] = [0.0; INPUTS];
            m[3] = half(&m[0], &m[1]);
            lfe_to_sub(&mut m, &[3]);
            return m;
        }
        // The centre speaker on its own output (not in the fronts), as
        // loud as the source has it at the default centre level and
        // louder or quieter with that level; a source without one: half
        // of each front. The subwoofer gets everything the fronts and the
        // centre get.
        Outputs::Center => {
            let mut m = auto_matrix(present, mono, Outputs::Front, s);
            let sub = half(&m[0], &m[1]);
            if !mono && present[2] {
                (m[0][2], m[1][2]) = (0.0, 0.0);
                m[2] = [0.0; INPUTS];
                m[2][2] = db_to_lin(s.center_db - CENTER_DEFAULT_DB).min(MAX_GAIN);
            } else {
                m[2] = half(&m[0], &m[1]);
            }
            m[3] = sub;
            lfe_to_sub(&mut m, &[3]);
            return m;
        }
        Outputs::Front | Outputs::Rear => {}
    }
    let rear_out = outputs == Outputs::Rear;
    let mut m = [[0.0f32; INPUTS]; LANES];
    if mono {
        // The single channel's column is the centre's (the usual mask of
        // one channel); any present column plays everywhere
        for (c, _) in present.iter().enumerate().filter(|(_, p)| **p) {
            for lane in &mut m {
                lane[c] = 1.0;
            }
        }
        return m;
    }
    let (c, lfe, sur) = (db_to_lin(s.center_db), db_to_lin(s.lfe_db), db_to_lin(s.surround_db));
    m[0][0] = 1.0;
    m[1][1] = 1.0;
    (m[0][2], m[1][2]) = (c, c);
    (m[0][3], m[1][3]) = (lfe, lfe);
    let (bl, br, sl, sr) = (present[4], present[5], present[6], present[7]);
    if rear_out {
        let kl = if bl && sl { MINUS_3DB } else { 1.0 };
        let kr = if br && sr { MINUS_3DB } else { 1.0 };
        (m[2][4], m[2][6]) = (sur * kl, sur * kl);
        (m[3][5], m[3][7]) = (sur * kr, sur * kr);
        if !(bl || br || sl || sr) {
            (m[2], m[3]) = (m[0], m[1]);
        }
    } else {
        let k = MINUS_3DB * sur;
        (m[0][4], m[0][6]) = (k, k);
        (m[1][5], m[1][7]) = (k, k);
    }
    // The LFE to the outputs marked full range (the rear ones only when
    // sent)
    for (l, row) in m.iter_mut().enumerate() {
        row[3] = if s.full_range[l] && (l < 2 || rear_out) { lfe } else { 0.0 };
    }
    for row in &mut m {
        for (g, p) in row.iter_mut().zip(present) {
            if !p {
                *g = 0.0;
            }
        }
        if s.normalize {
            let sum: f32 = row.iter().map(|g| g.abs()).sum();
            if sum > 1.0 {
                row.iter_mut().for_each(|g| *g /= sum);
            }
        }
    }
    m
}

/// The matrix in force for a source (`channels`, `mask`) and the stream's
/// outputs: auto, or the one set (its columns the source lacks at 0)
pub fn effective_matrix(channels: usize, mask: u32, outputs: Outputs, s: &MixSettings) -> Matrix {
    let present = present_columns(channels, mask);
    if s.auto {
        return auto_matrix(&present, channels == 1, outputs, s);
    }
    let mut m = s.matrix;
    for row in &mut m {
        for (g, p) in row.iter_mut().zip(&present) {
            if !p {
                *g = 0.0;
            }
        }
    }
    m
}

struct Shared {
    version: AtomicU64,
    settings: Mutex<MixSettings>,
    /// For the log: "Auracast mixer", "TWS mixer"
    name: &'static str,
}

/// The mixer settings of the broadcast running, shared by the control side
/// (which sets them) and the encode thread (which reads them)
#[derive(Clone)]
pub struct MixHandle(Arc<Shared>);

impl Default for MixHandle {
    fn default() -> Self {
        MixHandle::named("Auracast mixer")
    }
}

impl std::fmt::Debug for MixHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("MixHandle").field(&self.get()).finish()
    }
}

impl MixHandle {
    /// Default settings, logged as `name`
    pub fn named(name: &'static str) -> MixHandle {
        MixHandle(Arc::new(Shared { version: AtomicU64::new(0), settings: Mutex::new(MixSettings::default()), name }))
    }

    /// The name it is logged with
    pub fn name(&self) -> &'static str {
        self.0.name
    }

    /// New settings (held to the ranges): the encode thread takes them from
    /// its next block
    pub fn set(&self, settings: MixSettings) {
        let settings = settings.held();
        let mut s = self.0.settings.lock().unwrap_or_else(|e| e.into_inner());
        if *s != settings {
            *s = settings;
            self.0.version.fetch_add(1, Ordering::Release);
        }
    }

    pub fn get(&self) -> MixSettings {
        *self.0.settings.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn version(&self) -> u64 {
        self.0.version.load(Ordering::Acquire)
    }

    fn try_get(&self) -> Option<MixSettings> {
        match self.0.settings.try_lock() {
            Ok(s) => Some(*s),
            Err(std::sync::TryLockError::Poisoned(e)) => Some(*e.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => None,
        }
    }
}

/// The mixer on the encode thread, for one source format and stream
pub struct Mixer {
    handle: MixHandle,
    seen: u64,
    channels: usize,
    mask: u32,
    outputs: Outputs,
    columns: [Option<usize>; MAX_INPUT],
    /// The input channels that are the back centre speaker
    back_center: [bool; MAX_INPUT],
    rate: f32,
    /// Per lane, the gain of each interleaved input channel
    gains: [[f32; MAX_INPUT]; LANES],
    /// Per lane: delay in samples, polarity
    delay: [f32; LANES],
    sign: [f32; LANES],
    /// Per lane, the delay line (capacity `cap`), written at `pos`
    lines: [Vec<f32>; LANES],
    pos: usize,
    cap: usize,
    /// 2.1: per lane, the crossover's two sections (none: the lane as it is)
    crossover: [Option<[Biquad; 2]>; LANES],
}

impl Mixer {
    /// For a source of `channels` with the speakers of `mask` at `rate`
    /// (Hz), a stream with those outputs. Allocates.
    pub fn new(handle: MixHandle, channels: usize, mask: u32, outputs: Outputs, rate: u32) -> Mixer {
        let cap = (f64::from(MAX_DELAY_MS) / 1000.0 * f64::from(rate.max(1))).ceil() as usize + 2;
        let unused = unused_speakers(channels, mask);
        if unused != 0 {
            tracing::warn!("{}: no column for the source's speakers 0x{unused:x}, they are not heard", handle.name());
        }
        let mut m = Mixer {
            handle,
            seen: u64::MAX,
            channels,
            mask,
            outputs,
            columns: input_columns(channels, mask),
            back_center: back_center_inputs(channels, mask),
            rate: rate.max(1) as f32,
            gains: [[0.0; MAX_INPUT]; LANES],
            delay: [0.0; LANES],
            sign: [1.0; LANES],
            lines: std::array::from_fn(|_| vec![0.0; cap]),
            pos: 0,
            cap,
            crossover: [None; LANES],
        };
        m.refresh();
        m
    }

    /// Takes changed settings (unless the control side holds them now)
    pub fn refresh(&mut self) {
        let version = self.handle.version();
        if version == self.seen {
            return;
        }
        let Some(s) = self.handle.try_get() else { return };
        self.seen = version;
        let m = heard_matrix(&effective_matrix(self.channels, self.mask, self.outputs, &s), &s);
        for (lane, row) in self.gains.iter_mut().zip(&m) {
            for ((g, col), back_center) in lane.iter_mut().zip(&self.columns).zip(self.back_center) {
                *g = if back_center { MINUS_3DB * (row[4] + row[5]) } else { col.map_or(0.0, |c| row[c]) };
            }
        }
        for l in 0..LANES {
            self.delay[l] = (s.delay_ms[l] / 1000.0 * self.rate).min((self.cap - 2) as f32);
            self.sign[l] = if s.invert[l] { -1.0 } else { 1.0 };
        }
        self.set_crossover(&s);
        tracing::debug!(
            "{}: {} matrix {:?}{}, delays {:?} ms, inverted {:?}{}",
            self.handle.name(),
            if s.auto { "auto" } else { "manual" },
            m,
            if s.monitoring() {
                format!(" (solo in {:?} out {:?}, mute in {:?} out {:?})", s.solo_in, s.solo_out, s.mute_in, s.mute_out)
            } else {
                String::new()
            },
            s.delay_ms,
            s.invert,
            if !self.outputs.has_sub() {
                String::new()
            } else if s.mains_full_range {
                format!(", crossover {} Hz (the subwoofer only, the fronts full range)", s.crossover_hz)
            } else {
                format!(", crossover {} Hz", s.crossover_hz)
            }
        );
    }

    /// 2.1: the crossover's sections, keeping their state when only the
    /// frequency changes (no click while it is moved)
    fn set_crossover(&mut self, s: &MixSettings) {
        if !self.outputs.has_sub() {
            return;
        }
        let (rate, f) = (f64::from(self.rate), f64::from(s.crossover_hz));
        let q = std::f64::consts::FRAC_1_SQRT_2;
        let (low, high) = (Coeffs::low_pass(rate, f, q), Coeffs::high_pass(rate, f, q));
        for (x, band) in self.crossover.iter_mut().zip(self.outputs.bands()) {
            let c = match band {
                Band::Full => None,
                Band::Main if s.mains_full_range => None,
                Band::Main => Some(high),
                Band::Low => Some(low),
            };
            match (c, x.as_mut()) {
                (None, _) => *x = None,
                (Some(c), Some(sections)) => sections.iter_mut().for_each(|b| b.set(c)),
                (Some(c), None) => {
                    let mut b = Biquad::default();
                    b.set(c);
                    *x = Some([b; 2]);
                }
            }
        }
    }

    /// The lanes of one input frame (its channels, interleaved order)
    #[inline]
    pub fn apply(&self, ch: &[f32]) -> [f32; LANES] {
        std::array::from_fn(|lane| ch.iter().zip(&self.gains[lane]).map(|(x, g)| x * g).sum())
    }

    /// Delay and polarity of one frame of lanes, in place
    #[inline]
    pub fn post(&mut self, s: &mut [f32; LANES]) {
        let (pos, cap) = (self.pos, self.cap);
        for (l, x) in s.iter_mut().enumerate() {
            let line = &mut self.lines[l];
            line[pos] = *x;
            let d = self.delay[l];
            let out = if d <= 0.0 {
                *x
            } else {
                let di = d as usize;
                let fr = d - di as f32;
                let a = line[(pos + cap - di) % cap];
                let b = line[(pos + cap - di - 1) % cap];
                a + (b - a) * fr
            };
            let out = match self.crossover[l].as_mut() {
                Some([a, b]) => b.process(a.process(f64::from(out))) as f32,
                None => out,
            };
            *x = out * self.sign[l];
        }
        self.pos = (pos + 1) % cap;
    }

    /// Drops the delayed audio (an old stream's)
    pub fn reset(&mut self) {
        for line in &mut self.lines {
            line.fill(0.0);
        }
        self.pos = 0;
        for b in self.crossover.iter_mut().flatten().flatten() {
            b.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 2e-3
    }

    #[test]
    fn auto_matches_the_downmix_and_the_speaker_matrix() {
        let s = MixSettings::default();
        // 5.1 to stereo: as the downmix (centre -3 dB, LFE -6 dB, back -3 dB)
        let m = effective_matrix(6, 0x3F, Outputs::Front, &s);
        let f = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6];
        let lanes = |m: &Matrix, f: &[f32], cols: &[usize]| -> [f32; LANES] {
            std::array::from_fn(|l| f.iter().zip(cols).map(|(x, &c)| x * m[l][c]).sum())
        };
        let cols = [0, 1, 2, 3, 4, 5];
        let (dl, dr) = crate::le::downmix(&f);
        let out = lanes(&m, &f, &cols);
        assert!(close(out[0], dl) && close(out[1], dr), "{out:?} vs {dl} {dr}");
        assert_eq!((out[2], out[3]), (0.0, 0.0));
        // 5.1 to four outputs: as speaker_matrix
        let m = effective_matrix(6, 0x3F, Outputs::Rear, &s);
        let (sm, _) = crate::le::speaker_matrix(6, 0x3F);
        for l in 0..LANES {
            for i in 0..6 {
                assert!(close(m[l][cols[i]], sm[l][i]), "lane {l} input {i}");
            }
        }
        // Stereo source, rear outputs: the rear plays the front
        let m = effective_matrix(2, 0x3, Outputs::Rear, &s);
        assert_eq!((m[2], m[3]), (m[0], m[1]));
        // Mono: everywhere
        let m = effective_matrix(1, 0x4, Outputs::Rear, &s);
        assert!(m.iter().all(|row| row[2] == 1.0));
    }

    #[test]
    fn the_lfe_goes_to_the_outputs_marked_full_range() {
        // 5.1 on four outputs: the fronts by default
        let s = MixSettings::default();
        let m = effective_matrix(6, 0x3F, Outputs::Rear, &s);
        let lfe = db_to_lin(LFE_DEFAULT_DB);
        assert_eq!([m[0][3], m[1][3], m[2][3], m[3][3]], [lfe, lfe, 0.0, 0.0]);
        // The rears only
        let s = MixSettings { full_range: [false, false, true, true], ..s };
        let m = effective_matrix(6, 0x3F, Outputs::Rear, &s);
        assert_eq!([m[0][3], m[1][3], m[2][3], m[3][3]], [0.0, 0.0, lfe, lfe]);
        // Normalized: a rear row with the LFE sums to 1 at most
        let n = effective_matrix(6, 0x3F, Outputs::Rear, &MixSettings { normalize: true, ..s });
        assert!(n[2].iter().map(|g| g.abs()).sum::<f32>() <= 1.0 + 1e-6);
        assert!(n[2][3] > 0.0);
        // No rear outputs: the rear flags count for nothing
        let m = effective_matrix(6, 0x3F, Outputs::Front, &s);
        assert!(m.iter().all(|row| row[3] == 0.0));
        // A subwoofer takes the LFE whatever the flags
        let m = effective_matrix(6, 0x3F, Outputs::Sub, &s);
        assert_eq!((m[0][3], m[2][3]), (0.0, 1.0));
    }

    #[test]
    fn solo_and_mute_change_what_is_heard_only() {
        let s = MixSettings::default();
        let m = effective_matrix(6, 0x3F, Outputs::Rear, &s);
        assert_eq!(heard_matrix(&m, &s), m);
        assert!(!s.monitoring());
        // The centre soloed: only its column; the rear right muted
        let mut t = s;
        t.solo_in[2] = true;
        t.mute_out[3] = true;
        assert!(t.monitoring());
        let h = heard_matrix(&m, &t);
        for (l, row) in h.iter().enumerate() {
            for (c, g) in row.iter().enumerate() {
                let want = if c == 2 && l != 3 { m[l][c] } else { 0.0 };
                assert_eq!(*g, want, "lane {l} column {c}");
            }
        }
        // A soloed output, a muted one wins over its solo
        let mut u = s;
        u.solo_out = [true, true, false, false];
        u.mute_out[1] = true;
        let h = heard_matrix(&m, &u);
        assert_eq!((h[0], h[1], h[2]), (m[0], [0.0; INPUTS], [0.0; INPUTS]));
        // As saved: without them
        assert!(!t.unmonitored().monitoring());
        assert_eq!(t.unmonitored().matrix, t.matrix);
    }

    #[test]
    fn levels_normalize_and_manual() {
        let mut s = MixSettings { center_db: 0.0, lfe_db: -60.0, normalize: true, ..MixSettings::default() };
        let m = effective_matrix(6, 0x3F, Outputs::Rear, &s);
        // FL + FC at 0 dB + LFE at -60 dB, normalized
        let sum: f32 = m[0].iter().map(|g| g.abs()).sum();
        assert!(close(sum, 1.0));
        assert!(close(m[0][0], m[0][2]));
        // Manual: as set, columns the source lacks dropped
        s.auto = false;
        s.matrix = [[0.0; INPUTS]; LANES];
        s.matrix[0][1] = 1.0; // FR into the front left
        s.matrix[3][7] = 1.0; // SR into the rear right: a 5.1 back source lacks it
        let m = effective_matrix(6, 0x3F, Outputs::Rear, &s);
        assert_eq!(m[0][1], 1.0);
        assert_eq!(m[3][7], 0.0);
        assert_eq!(present_columns(6, 0x60F), [true, true, true, true, false, false, true, true]);
    }

    #[test]
    fn a_back_center_speaker_plays_as_the_back_left_and_right() {
        // 6.1: FL FR FC LFE BC SL SR
        let mask = 0x70F;
        assert_eq!(present_columns(7, mask), [true, true, true, true, true, true, true, true]);
        assert_eq!(back_center_inputs(7, mask)[..7], [false, false, false, false, true, false, false]);
        assert_eq!(unused_speakers(7, mask), 0);
        let mx = Mixer::new(MixHandle::default(), 7, mask, Outputs::Rear, 48000);
        // The back centre alone: on both rears, -3 dB for being split and
        // -3 dB more beside the sides
        let out = mx.apply(&[0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        assert_eq!((out[0], out[1]), (0.0, 0.0));
        assert!(close(out[2], 0.5) && close(out[3], 0.5), "{out:?}");
        // The side left: the rear left only
        let out = mx.apply(&[0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
        assert!(close(out[2], MINUS_3DB) && out[3] == 0.0, "{out:?}");
        // Without rear outputs: folded into both fronts
        let mx = Mixer::new(MixHandle::default(), 7, mask, Outputs::Front, 48000);
        let out = mx.apply(&[0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        assert!(close(out[0], 0.5) && close(out[1], 0.5) && out[2] == 0.0, "{out:?}");
        // A top speaker has no column
        assert_eq!(unused_speakers(3, 0x1003), 0x1000);
    }

    #[test]
    fn held_to_the_ranges() {
        let mut s = MixSettings::default();
        s.matrix[0][0] = 100.0;
        s.matrix[1][1] = f32::NAN;
        s.delay_ms = [-1.0, 1e6, f32::INFINITY, 2.5];
        let h = s.held();
        assert_eq!((h.matrix[0][0], h.matrix[1][1]), (MAX_GAIN, 0.0));
        assert_eq!(h.delay_ms, [0.0, MAX_DELAY_MS, 0.0, 2.5]);
    }

    #[test]
    fn mixer_follows_the_handle_and_delays_fractionally() {
        let handle = MixHandle::default();
        let mut mx = Mixer::new(handle.clone(), 4, 0x33, Outputs::Rear, 1000);
        // Quad, auto: the four speakers as they are
        assert_eq!(mx.apply(&[0.1, 0.2, 0.3, 0.4]), [0.1, 0.2, 0.3, 0.4]);
        // Swap left and right, delay the rear right 2.5 samples (2.5 ms at
        // 1 kHz), invert the front right
        let mut s = handle.get();
        s.auto = false;
        s.matrix = [[0.0; INPUTS]; LANES];
        (s.matrix[0][1], s.matrix[1][0], s.matrix[2][4], s.matrix[3][5]) = (1.0, 1.0, 1.0, 1.0);
        s.delay_ms[3] = 2.5;
        s.invert[1] = true;
        handle.set(s);
        mx.refresh();
        assert_eq!(mx.apply(&[0.1, 0.2, 0.3, 0.4]), [0.2, 0.1, 0.3, 0.4]);
        // An impulse on every lane, then silence
        let mut out = Vec::new();
        for n in 0..6 {
            let x = if n == 0 { 1.0 } else { 0.0 };
            let mut lanes = [x; LANES];
            mx.post(&mut lanes);
            out.push(lanes);
        }
        assert_eq!(out[0][..3], [1.0, -1.0, 1.0]);
        let rr: Vec<f32> = out.iter().map(|l| l[3]).collect();
        // 2.5 samples: half at 2, half at 3
        assert_eq!(rr, [0.0, 0.0, 0.5, 0.5, 0.0, 0.0]);
        mx.reset();
        let mut lanes = [0.0; LANES];
        mx.post(&mut lanes);
        assert_eq!(lanes, [0.0; LANES]);
    }

    /// The peak of a tone of `hz` through the mixer's post stage (after a
    /// second to settle), per lane
    fn tone_peaks(mx: &mut Mixer, hz: f32, rate: u32) -> [f32; LANES] {
        let mut peak = [0.0f32; LANES];
        for n in 0..rate * 2 {
            let x = (2.0 * std::f32::consts::PI * hz * n as f32 / rate as f32).sin();
            let mut lanes = [x; LANES];
            mx.post(&mut lanes);
            if n >= rate {
                for (p, v) in peak.iter_mut().zip(lanes) {
                    *p = p.max(v.abs());
                }
            }
        }
        peak
    }

    #[test]
    fn the_2_1_crossover_splits_the_low_end_to_the_subwoofer() {
        let rate = 48000;
        let handle = MixHandle::default();
        let mut mx = Mixer::new(handle.clone(), 2, 0x3, Outputs::Sub, rate);
        // Auto: the subwoofer gets the fronts
        assert_eq!(mx.apply(&[0.1, 0.2]), [0.1, 0.2, 0.1, 0.2]);
        // 80 Hz: a 30 Hz tone is the subwoofer's, 1 kHz the fronts'
        let low = tone_peaks(&mut mx, 30.0, rate);
        assert!(low[2] > 0.9 && low[3] > 0.9 && low[0] < 0.2, "{low:?}");
        mx.reset();
        let high = tone_peaks(&mut mx, 1000.0, rate);
        assert!(high[0] > 0.9 && high[1] > 0.9 && high[2] < 0.01, "{high:?}");
        // At the crossover each side is half (-6 dB)
        mx.reset();
        let at = tone_peaks(&mut mx, 80.0, rate);
        assert!((at[0] - 0.5).abs() < 0.05 && (at[2] - 0.5).abs() < 0.05, "{at:?}");
        // The fronts full range: no high pass
        let mut s = handle.get();
        s.mains_full_range = true;
        handle.set(s);
        mx.refresh();
        mx.reset();
        let low = tone_peaks(&mut mx, 30.0, rate);
        assert!(low[0] > 0.99 && low[2] > 0.9, "{low:?}");
        // A mono subwoofer: half of each front on the fourth output; 3.1:
        // a stereo source's centre is half of each front, high passed
        let mut mono = Mixer::new(MixHandle::default(), 2, 0x3, Outputs::SubMono, rate);
        assert_eq!(mono.apply(&[0.2, 0.4]), [0.2, 0.4, 0.0, 0.3]);
        let low = tone_peaks(&mut mono, 30.0, rate);
        assert!(low[3] > 0.9 && low[0] < 0.2, "{low:?}");
        let mut center = Mixer::new(MixHandle::default(), 2, 0x3, Outputs::Center, rate);
        assert_eq!(center.apply(&[0.2, 0.4]), [0.2, 0.4, 0.3, 0.3]);
        let high = tone_peaks(&mut center, 1000.0, rate);
        assert!(high[2] > 0.9 && high[3] < 0.01, "{high:?}");
        // 5.1: the centre on its own output, not in the fronts
        let five = Mixer::new(MixHandle::default(), 6, 0x3f, Outputs::Center, rate);
        let lanes = five.apply(&[0.0, 0.0, 0.5, 0.0, 0.0, 0.0]);
        assert_eq!(lanes[..3], [0.0, 0.0, 0.5]);
        assert!(lanes[3] > 0.0);
        // Its level follows the centre level: 6 dB down, half as loud
        let quiet = MixHandle::default();
        quiet.set(MixSettings { center_db: CENTER_DEFAULT_DB - 6.0, ..quiet.get() });
        let five = Mixer::new(quiet, 6, 0x3f, Outputs::Center, rate);
        let lanes = five.apply(&[0.0, 0.0, 0.5, 0.0, 0.0, 0.0]);
        assert!(lanes[0] == 0.0 && (lanes[2] - 0.25).abs() < 0.001, "{lanes:?}");
        // The LFE is the subwoofer's alone, as loud as the source has it;
        // its level follows the LFE level
        for (outputs, subs) in [(Outputs::Sub, &[2, 3][..]), (Outputs::SubMono, &[3]), (Outputs::Center, &[3])] {
            let mx = Mixer::new(MixHandle::default(), 6, 0x3f, outputs, rate);
            let lanes = mx.apply(&[0.0, 0.0, 0.0, 0.5, 0.0, 0.0]);
            for (l, x) in lanes.iter().enumerate() {
                assert_eq!(*x, if subs.contains(&l) { 0.5 } else { 0.0 }, "{outputs:?} {lanes:?}");
            }
            let quiet = MixHandle::default();
            quiet.set(MixSettings { lfe_db: LFE_DEFAULT_DB - 6.0, ..quiet.get() });
            let lanes = Mixer::new(quiet, 6, 0x3f, outputs, rate).apply(&[0.0, 0.0, 0.0, 0.5, 0.0, 0.0]);
            assert!((lanes[3] - 0.25).abs() < 0.001 && lanes[0] == 0.0, "{outputs:?} {lanes:?}");
        }
        // Without a subwoofer no lane is filtered
        let mut rear = Mixer::new(MixHandle::default(), 4, 0x33, Outputs::Rear, rate);
        let low = tone_peaks(&mut rear, 30.0, rate);
        assert!(low.iter().all(|p| *p > 0.99), "{low:?}");
    }
}
