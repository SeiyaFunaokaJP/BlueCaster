//! LHDC V5: Savitech's high-resolution Bluetooth codec (44.1 / 48 / 96 /
//! 192 kHz, 16 or 24-bit stereo, 5 ms frames, 64 to 1000 kbps over A2DP),
//! encoded by a portable C port of the encoder Android ships (crate
//! `lhdcv5-sys`; the port and the AOSP original are Apache-2.0; LHDC itself
//! is Savitech's, whose licensing terms for products advertising LHDC are
//! separate from the code licence).
//!
//! The encoder takes one 5 ms frame of interleaved stereo PCM per call
//! (packed s16le or s24le: mono is duplicated, 24-bit input is repacked
//! from the top of the int32 container) and returns a complete packet of
//! one or more frames, each with its own 2-byte frame header, every few
//! calls. Parameters follow Android's a2dp_vendor_lhdcv5_encoder.cc as v1
//! did: 20 ms encode interval, packets sized for the MTU minus the 2-byte
//! media payload header, the sink's bit rate limits applied after init, and
//! the ABR stages of lhdcv5BT_enc.c ([`LhdcV5Encoder::abr_adjust`]; the
//! climb back up is faster than Android's).

use std::time::{Duration, Instant};

use lhdcv5_sys::{LhdcV5Enc, ffi};

use crate::abr::StallMemory;
use crate::{CodecKind, Encoded, Encoder, EncoderConfig, EncoderError, Pcm, PcmFormat, Quality};

const CODEC: CodecKind = CodecKind::LhdcV5;

/// The A2DP media payload header in front of the frames
/// (A2DP_LHDC_MPL_HDR_LEN)
const MPL_HDR_LEN: usize = 2;

// ABR. Going down follows Android's lhdcv5BT_enc.c: every 4 packets sent,
// an average queue above 0 drops the bit rate to the lowest stage. Going up
// does not: Android waits 3000 packets per stage (a minute of 20 ms
// packets, four minutes from 160 back to 400 kbps, #600). Here a stage
// takes 5 s of an empty queue, longer back to a stage the queue backed up
// at ([`StallMemory`]).
/// Packets between checks for going down
const ABR_DOWN_RATE_TIME_CNT: u32 = 4;
/// Average queue (packets) above which a check goes down
const ABR_DOWN_QUEUE_LENGTH_THRESHOLD: u32 = 0;
/// Average queue that goes down in the first [`ABR_START_GRACE`] of a
/// stream: the queue that builds up as it starts is not the link's limit
const ABR_START_DOWN_AVG: u32 = 4;
const ABR_START_GRACE: Duration = Duration::from_secs(3);
/// One step of the climb: calm when no packet sent in it saw a queue
const ABR_TICK: Duration = Duration::from_millis(100);
/// Calm steps for one stage up: 5 s
const ABR_UP_TICKS: u32 = 50;
const ABR_TABLE_44K: [u32; 6] = [160, 192, 240, 320, 400, 400];
const ABR_TABLE_48K: [u32; 6] = [160, 192, 256, 320, 400, 400];
const ABR_TABLE_96K: [u32; 6] = [256, 320, 400, 400, 400, 400];
const ABR_TABLE_192K: [u32; 6] = [256, 320, 400, 400, 400, 400];

// The queue guard of low latency without ABR. With the
// link at its limit the send queue fills and never drains (packets go out
// only as fast as they are made), so the delay it holds stays until the
// next connection. The guard lowers the bit rate one step while the queue
// stays backed up, which gives the link room to empty it, and goes back up
// after a long calm. Android's ABR does the job too, but never above 400 kbps.
/// The queue's average over this long decides a step
const GUARD_WINDOW: Duration = Duration::from_secs(1);
/// Average depth (packets) that steps down: 60 ms of 5 ms frames
const GUARD_DOWN_AVG: u64 = 12;
/// After a step down the queue gets this long to drain before another
const GUARD_DOWN_HOLD: Duration = Duration::from_secs(2);
/// Average depth at or below this counts as calm
const GUARD_CALM_AVG: u64 = 1;
/// Calm this long steps back up (doubled after a step up that did not hold)
const GUARD_UP_FIRST: Duration = Duration::from_secs(60);
const GUARD_UP_MAX: Duration = Duration::from_secs(600);
/// A step down this soon after a step up: the step up did not hold
const GUARD_UP_FAILED: Duration = Duration::from_secs(30);

/// Queue guard state: bit rate indices of the encoder's table
#[derive(Debug)]
struct Guard {
    /// Where the quality mode put the bit rate (after the sink's and the
    /// MTU's limits): the guard never goes above it
    target: u32,
    /// The lowest it goes (500 kbps, or the target if that is lower)
    floor: u32,
    window_start: Instant,
    sum: u64,
    count: u64,
    calm_since: Option<Instant>,
    last_down: Option<Instant>,
    last_up: Option<Instant>,
    up_wait: Duration,
}

impl Guard {
    fn new(target: u32, now: Instant) -> Guard {
        Guard {
            target,
            floor: target.min(ffi::LHDC_QUALITY_MID),
            window_start: now,
            sum: 0,
            count: 0,
            calm_since: None,
            last_down: None,
            last_up: None,
            up_wait: GUARD_UP_FIRST,
        }
    }

    /// One packet sent with `depth` packets queued before its round, at
    /// `now`, the encoder at index `current`: the index to switch to, if any
    fn step(&mut self, now: Instant, depth: u32, current: u32) -> Option<u32> {
        self.sum += u64::from(depth);
        self.count += 1;
        if now.saturating_duration_since(self.window_start) < GUARD_WINDOW {
            return None;
        }
        // Average x10, to tell 1.0 from 1.9
        let avg10 = self.sum * 10 / self.count.max(1);
        self.sum = 0;
        self.count = 0;
        self.window_start = now;
        if avg10 >= GUARD_DOWN_AVG * 10 {
            self.calm_since = None;
            let held = self.last_down.is_none_or(|t| now.saturating_duration_since(t) >= GUARD_DOWN_HOLD);
            if current > self.floor && held {
                if self.last_up.is_some_and(|t| now.saturating_duration_since(t) < GUARD_UP_FAILED) {
                    self.up_wait = (self.up_wait * 2).min(GUARD_UP_MAX);
                }
                self.last_down = Some(now);
                return Some(current - 1);
            }
        } else if avg10 <= GUARD_CALM_AVG * 10 {
            let since = *self.calm_since.get_or_insert(now);
            if current < self.target && now.saturating_duration_since(since) >= self.up_wait {
                self.last_up = Some(now);
                self.calm_since = Some(now);
                return Some(current + 1);
            }
        } else {
            self.calm_since = None;
        }
        None
    }
}

/// Adaptive bit rate state: stages of Android's table (lhdcv5BT_enc.c)
#[derive(Debug)]
struct Abr {
    table: &'static [u32; 6],
    /// The first stage at the table's highest rate (the tables repeat
    /// 400 kbps at the end)
    top: usize,
    index: usize,
    /// The first packet since the encoder was made or ABR was reset
    started: Option<Instant>,
    down_count: u32,
    down_sum: u32,
    tick_start: Instant,
    /// A packet in this step saw a queue
    tick_busy: bool,
    /// Calm steps in a row
    calm: u32,
    stalls: StallMemory,
}

impl Abr {
    fn new(table: &'static [u32; 6]) -> Abr {
        let max = table.iter().copied().max().unwrap_or(0);
        let top = table.iter().position(|&r| r == max).unwrap_or(table.len() - 1);
        Abr {
            table,
            top,
            index: top,
            started: None,
            down_count: 0,
            down_sum: 0,
            tick_start: Instant::now(),
            tick_busy: false,
            calm: 0,
            stalls: StallMemory::default(),
        }
    }

    /// One packet sent at `now` with `depth` packets queued before its
    /// round: the stage to switch to, if any (already noted as current)
    fn step(&mut self, now: Instant, depth: u32) -> Option<usize> {
        let started = match self.started {
            Some(t) => t,
            None => {
                self.started = Some(now);
                self.tick_start = now;
                now
            }
        };
        self.down_sum = self.down_sum.saturating_add(depth);
        self.down_count += 1;
        if depth > 0 {
            self.tick_busy = true;
        }
        if self.down_count >= ABR_DOWN_RATE_TIME_CNT {
            let avg = self.down_sum / self.down_count;
            self.down_count = 0;
            self.down_sum = 0;
            let starting = now.saturating_duration_since(started) < ABR_START_GRACE;
            let backed_up = if starting { avg >= ABR_START_DOWN_AVG } else { avg > ABR_DOWN_QUEUE_LENGTH_THRESHOLD };
            if backed_up {
                self.calm = 0;
                if self.index > 0 {
                    self.stalls.stepped_down("LHDC V5", self.index as u8);
                    self.index = 0;
                    return Some(0);
                }
            }
        }
        if now.saturating_duration_since(self.tick_start) < ABR_TICK {
            return None;
        }
        self.tick_start = now;
        if std::mem::take(&mut self.tick_busy) {
            self.calm = 0;
        } else {
            self.calm += 1;
        }
        let here = self.index as u8;
        self.stalls.tick("LHDC V5", here, self.calm, ABR_UP_TICKS);
        if self.index < self.top && self.calm >= self.stalls.wait(here, here + 1, ABR_UP_TICKS) {
            self.calm = 0;
            self.index += 1;
            return Some(self.index);
        }
        None
    }
}

pub struct LhdcV5Encoder {
    enc: LhdcV5Enc,
    format: PcmFormat,
    channels: usize,
    /// PCM frames per LHDC frame (240 / 480 / 960)
    block_size: usize,
    /// MTU the encoder sizes its packets for (media MTU - payload header)
    enc_mtu: usize,
    /// Input repacked for the encoder: interleaved stereo s16le or packed
    /// s24le, one frame
    packed: Vec<u8>,
    abr: Option<Abr>,
    /// Low latency without ABR lowers the bit rate while
    /// the send queue stays backed up ([`Guard`])
    guard: Option<Guard>,
    /// The sink's maximum bit rate (P7) is below the quality mode's
    peer_capped: bool,
}

/// Sink capability P7[5:4] -> encoder quality index
/// (A2DP_MaxBitRatetoQualityLevelLhdcV5)
fn peer_max_bitrate_index(p7: u8) -> u32 {
    match p7 & 0x30 {
        0x10 => ffi::LHDC_QUALITY_LOW,  // 400 kbps
        0x20 => ffi::LHDC_QUALITY_MID,  // 500 kbps
        0x30 => ffi::LHDC_QUALITY_HIGH, // 900 kbps
        _ => ffi::LHDC_QUALITY_HIGH1,   // 1000 kbps = no upper limit
    }
}

/// Whether the sink's maximum bit rate (P7) lowers the fixed bit rate index
/// a quality mode asks for. Never under ABR: it stays at 400 kbps or below
/// whatever the quality, as on Android.
fn peer_caps_bitrate(abr: bool, bitrate_inx: u32, p7: u8) -> bool {
    !abr && peer_max_bitrate_index(p7) < bitrate_inx
}

/// Sink capability P7[7:6] -> encoder quality index
/// (A2DP_MinBitRatetoQualityLevelLhdcV5)
fn peer_min_bitrate_index(p7: u8) -> u32 {
    match p7 & 0xC0 {
        0x40 => ffi::LHDC_QUALITY_LOW1, // 160 kbps
        0x80 => ffi::LHDC_QUALITY_LOW3, // 256 kbps
        0xC0 => ffi::LHDC_QUALITY_LOW,  // 400 kbps
        _ => ffi::LHDC_QUALITY_LOW0,    // 64 kbps = no lower limit
    }
}

/// The encoder's bit rate table (kbps) from 64 kbps up to
/// LHDC_QUALITY_HIGH1 (lhdcv5_api.c g_bitrate_table_*)
fn bitrate_table(sample_rate: u32) -> [u32; 9] {
    if sample_rate == 44100 {
        [64, 160, 192, 240, 320, 400, 480, 900, 1000]
    } else {
        [64, 160, 192, 256, 320, 400, 500, 900, 1000]
    }
}

/// Highest bit rate index up to LHDC_QUALITY_HIGH1 whose frames fit `mtu`.
/// The encoder sizes a frame for the bit rate over the frame's duration
/// (block_size / sample_rate), and the frame header rounds it up to an even
/// size plus 2 bytes; this estimate is that, rounded up once more.
fn fitting_bitrate_index(sample_rate: u32, block_size: usize, mtu: usize) -> u32 {
    let table = bitrate_table(sample_rate);
    let mut inx = ffi::LHDC_QUALITY_HIGH1;
    while inx > ffi::LHDC_QUALITY_LOW {
        let payload = (u64::from(table[inx as usize]) * 125 * block_size as u64).div_ceil(u64::from(sample_rate));
        if 2 + 2 * payload.div_ceil(2) as usize <= mtu {
            break;
        }
        inx -= 1;
    }
    inx
}

impl LhdcV5Encoder {
    pub fn new(config: &EncoderConfig) -> Result<LhdcV5Encoder, EncoderError> {
        let unsupported = |what: String| EncoderError::Unsupported { codec: CODEC, what };
        if !matches!(config.sample_rate, 44100 | 48000 | 96000 | 192000) {
            return Err(unsupported(format!(
                "sample rate {} Hz (use 44100, 48000, 96000 or 192000)",
                config.sample_rate
            )));
        }
        if !matches!(config.channels, 1 | 2) {
            return Err(unsupported(format!("{} channels", config.channels)));
        }
        let (format, encoder_bits) = match config.bits_per_sample {
            16 => (PcmFormat::I16, ffi::LHDCBT_SMPL_FMT_S16),
            24 => (PcmFormat::I32, ffi::LHDCBT_SMPL_FMT_S24),
            n => return Err(unsupported(format!("{n}-bit samples (use 16 or 24)"))),
        };

        // Quality mode -> target bit rate index (clamped to the sink's
        // limits below), the four fixed levels Android offers. ABR starts at
        // 400 kbps regardless (LHDC_QUALITY_AUTO).
        let bitrate_inx = if config.abr {
            ffi::LHDC_QUALITY_AUTO
        } else {
            match config.quality {
                Quality::High | Quality::LowLatency => ffi::LHDC_QUALITY_HIGH1, // 1000 kbps
                Quality::MediumHigh => ffi::LHDC_QUALITY_HIGH,                  // 900 kbps
                Quality::Standard => ffi::LHDC_QUALITY_MID,                     // 500 kbps (480 at 44.1 kHz)
                Quality::Mobile => ffi::LHDC_QUALITY_LOW,                       // 400 kbps
            }
        };

        // The encoder sizes its packets for this MTU: the media MTU (RTP
        // header already excluded) minus the 2-byte LHDC media payload header
        let enc_mtu = usize::from(config.mtu).saturating_sub(MPL_HDR_LEN);
        if enc_mtu < ffi::LHDC_MTU_MIN as usize {
            return Err(unsupported(format!(
                "media MTU {} (LHDC V5 needs at least {})",
                config.mtu,
                ffi::LHDC_MTU_MIN as usize + MPL_HDR_LEN
            )));
        }
        let enc_mtu = enc_mtu.min(ffi::LHDC_MTU_MAX as usize);

        let init_err = |e: lhdcv5_sys::Error| EncoderError::Init { codec: CODEC, what: e.to_string() };
        let mut enc = LhdcV5Enc::new().map_err(init_err)?;
        // Android encodes every 10 ms instead of 20 with low latency on
        let interval =
            if config.peer.lhdcv5_low_latency { ffi::LHDC_ENC_INTERVAL_10MS } else { ffi::LHDC_ENC_INTERVAL_20MS };
        enc.init_encoder(config.sample_rate, encoder_bits, bitrate_inx, enc_mtu as u32, interval).map_err(init_err)?;
        tracing::info!("LHDC V5: encode interval {interval} ms");

        let block_size = match enc.block_size() {
            Ok(n) if n > 0 => n as usize,
            r => {
                return Err(EncoderError::Init { codec: CODEC, what: format!("no frame size ({r:?})") });
            }
        };

        // Peer bit rate limits, applied after init as Android does; a
        // failure leaves the encoder's own limits (as v1)
        let p7 = config.peer.lhdcv5_bitrate_limits;
        let (mut max_inx, min_inx) = (peer_max_bitrate_index(p7), peer_min_bitrate_index(p7));
        let peer_capped = peer_caps_bitrate(config.abr, bitrate_inx, p7);
        // The encoder never splits a frame: one that does not fit the MTU
        // goes out whole, over the sink's MTU, and is dropped. Only 1000 kbps
        // at 44.1 kHz with the smallest MTUs gets there (a 5 ms frame sized
        // for 1000 kbps lasts 5.44 ms there: 682 bytes), so cap the bit rate
        // at what fits (900 kbps then). v1 sent those packets anyway.
        let fitting = fitting_bitrate_index(config.sample_rate, block_size, enc_mtu);
        tracing::debug!(
            "LHDC V5: quality {:?}, abr {} -> bitrate index {bitrate_inx}; sink limits byte 0x{p7:02x} -> index \
             {min_inx}..{max_inx}, fits the {enc_mtu}-byte mtu up to index {fitting}, frame {block_size} samples",
            config.quality,
            config.abr
        );
        if fitting < max_inx {
            tracing::info!(
                "LHDC V5: {} kbps frames do not fit the {enc_mtu}-byte MTU at {} Hz, capping the bit rate",
                bitrate_table(config.sample_rate)[max_inx as usize],
                config.sample_rate
            );
            max_inx = fitting;
        }
        if let Err(e) = enc.set_max_bitrate_index(max_inx) {
            tracing::warn!("LHDC V5: set_max_bitrate_index({max_inx}) failed: {e}");
        }
        if let Err(e) = enc.set_min_bitrate_index(min_inx) {
            tracing::warn!("LHDC V5: set_min_bitrate_index({min_inx}) failed: {e}");
        }

        let packed = vec![0u8; block_size * 2 * (encoder_bits as usize / 8)];

        // ABR table for this sample rate; start at its top (400 kbps)
        let abr = config.abr.then(|| {
            let table = match config.sample_rate {
                44100 => &ABR_TABLE_44K,
                96000 => &ABR_TABLE_96K,
                192000 => &ABR_TABLE_192K,
                _ => &ABR_TABLE_48K,
            };
            Abr::new(table)
        });

        let mut encoder = LhdcV5Encoder {
            enc,
            format,
            channels: config.channels as usize,
            block_size,
            enc_mtu,
            packed,
            abr,
            guard: None,
            peer_capped,
        };
        if config.peer.lhdcv5_low_latency
            && !config.abr
            && let Some(target) = encoder.current_index()
        {
            encoder.guard = Some(Guard::new(target, Instant::now()));
            tracing::info!(
                "LHDC V5: queue guard on (low latency): down to {} kbps while the send queue stays backed up",
                bitrate_table(config.sample_rate)[target.min(ffi::LHDC_QUALITY_MID) as usize]
            );
        }
        tracing::info!(
            "LHDC V5: initialized (rate={}, ch={}, {}-bit, mtu={}, frame={} samples, bitrate={} kbps{}, peer limits idx {}..{}{})",
            config.sample_rate,
            config.channels,
            encoder_bits,
            enc_mtu,
            block_size,
            encoder.bitrate_kbps(),
            if config.abr { " ABR" } else { "" },
            min_inx,
            max_inx,
            if peer_capped { ", capped by the sink's maximum" } else { "" }
        );
        Ok(encoder)
    }

    /// Whether adaptive bit rate is active
    pub fn abr_enabled(&self) -> bool {
        self.abr.is_some()
    }

    /// Index in the encoder's bit rate table of its current target
    fn current_index(&self) -> Option<u32> {
        self.enc.bitrate_index(self.enc.last_bitrate()).ok()
    }

    /// One packet of the queue guard ([`Guard`]): a step down or up when
    /// the queue asks for it
    fn guard_adjust(&mut self, queue_depth: u32) {
        let Some(current) = self.current_index() else { return };
        let Some(guard) = self.guard.as_mut() else { return };
        let Some(next) = guard.step(Instant::now(), queue_depth, current) else { return };
        let before = self.enc.last_bitrate();
        match self.enc.set_bitrate_index(next, false) {
            Ok(()) => tracing::info!(
                "LHDC V5: queue guard {before} -> {} kbps ({})",
                self.enc.last_bitrate(),
                if next < current { "send queue backed up" } else { "queue calm" }
            ),
            Err(e) => tracing::warn!("LHDC V5: queue guard to index {next} failed: {e}"),
        }
    }

    /// [`Encoder::abr_adjust`] at `now`
    fn abr_adjust_at(&mut self, now: Instant, queue_depth: u32) {
        let Some(abr) = self.abr.as_mut() else {
            self.guard_adjust(queue_depth);
            return;
        };
        let before = abr.table[abr.index];
        let Some(stage) = abr.step(now, queue_depth) else { return };
        let rate = abr.table[stage];
        match self.enc.bitrate_index(rate).and_then(|inx| self.enc.set_bitrate_index(inx, false)) {
            Ok(()) => tracing::debug!(
                "LHDC V5: ABR {} {before} -> {rate} kbps (queue {queue_depth})",
                if rate < before { "down" } else { "up" }
            ),
            Err(e) => tracing::warn!("LHDC V5: ABR to {rate} kbps failed: {e}"),
        }
    }

    /// Repacks one frame of input into `packed`: interleaved stereo, s16le
    /// or packed s24le (the top 24 bits of the MSB-aligned int32)
    fn pack(&mut self, pcm: Pcm<'_>) {
        let channels = self.channels;
        match pcm {
            Pcm::I16(s) => {
                for (frame, dst) in s.chunks_exact(channels).zip(self.packed.as_chunks_mut::<4>().0) {
                    let (l, r) = (frame[0], frame[channels - 1]);
                    dst[..2].copy_from_slice(&l.to_le_bytes());
                    dst[2..].copy_from_slice(&r.to_le_bytes());
                }
            }
            Pcm::I32(s) => {
                for (frame, dst) in s.chunks_exact(channels).zip(self.packed.as_chunks_mut::<6>().0) {
                    let (l, r) = (frame[0].to_le_bytes(), frame[channels - 1].to_le_bytes());
                    dst[..3].copy_from_slice(&l[1..]);
                    dst[3..].copy_from_slice(&r[1..]);
                }
            }
        }
    }
}

impl Encoder for LhdcV5Encoder {
    fn kind(&self) -> CodecKind {
        CODEC
    }

    fn pcm_format(&self) -> PcmFormat {
        self.format
    }

    /// One LHDC frame (5 ms: 240 / 480 / 960 PCM frames) per call
    fn pcm_frames_per_encode(&self) -> usize {
        self.block_size
    }

    /// Packets never exceed the MTU the encoder was given (media MTU - 2)
    fn max_output_bytes(&self) -> usize {
        self.enc_mtu
    }

    fn encode(&mut self, pcm: Pcm<'_>, out: &mut [u8]) -> Result<Encoded, EncoderError> {
        let expected = self.block_size * self.channels;
        if pcm.samples() != expected {
            return Err(EncoderError::InputLength { expected, got: pcm.samples() });
        }
        let format_ok = matches!((pcm, self.format), (Pcm::I16(_), PcmFormat::I16) | (Pcm::I32(_), PcmFormat::I32));
        if !format_ok {
            return Err(EncoderError::Encode { codec: CODEC, what: format!("input is not {:?} PCM", self.format) });
        }
        if out.len() < self.enc_mtu {
            return Err(EncoderError::OutputTooSmall { have: out.len(), need: self.enc_mtu });
        }
        self.pack(pcm);
        let r = self.enc.encode(&self.packed, out).map_err(|e| EncoderError::Encode {
            codec: CODEC,
            what: format!("{e} (output buffer {} bytes)", out.len()),
        })?;
        Ok(Encoded { bytes: r.bytes, frames: r.frames })
    }

    /// The current target bit rate (changes under ABR)
    fn bitrate_kbps(&self) -> u32 {
        self.enc.last_bitrate()
    }

    fn bitrate_capped(&self) -> bool {
        self.peer_capped
    }

    /// ABR ([`Abr`]). Call once per media packet sent, with the send queue
    /// depth (packets) seen before this round's packets were enqueued, as
    /// Android does (lhdcv5BT_adjust_bitrate) and v1 did. Every 4 calls an
    /// average depth above 0 drops the bit rate to the lowest stage of the
    /// table (160 kbps, 256 at 96 / 192 kHz); 5 s with the queue empty
    /// throughout raise it one stage, up to 400 kbps. Does nothing when ABR
    /// is off.
    fn abr_adjust(&mut self, queue_depth: u32) {
        self.abr_adjust_at(Instant::now(), queue_depth);
    }

    fn abr_active(&self) -> bool {
        self.abr.is_some()
    }

    fn queue_guard_active(&self) -> bool {
        self.guard.is_some()
    }

    /// Back to the top of the ABR table (400 kbps), counters cleared; the
    /// queue guard back to the quality mode's bit rate
    fn abr_reset(&mut self) {
        if let Some(guard) = self.guard.as_mut() {
            let target = guard.target;
            *guard = Guard::new(target, Instant::now());
            if self.current_index() != Some(target)
                && let Err(e) = self.enc.set_bitrate_index(target, false)
            {
                tracing::warn!("LHDC V5: queue guard reset to index {target} failed: {e}");
            }
            return;
        }
        let Some(abr) = self.abr.as_mut() else { return };
        *abr = Abr::new(abr.table);
        let rate = abr.table[abr.top];
        match self.enc.bitrate_index(rate) {
            Ok(inx) if self.current_index() != Some(inx) => {
                if let Err(e) = self.enc.set_bitrate_index(inx, false) {
                    tracing::warn!("LHDC V5: ABR reset to {rate} kbps failed: {e}");
                }
            }
            Ok(_) => {}
            Err(e) => tracing::warn!("LHDC V5: ABR reset to {rate} kbps failed: {e}"),
        }
    }
}

#[cfg(test)]
mod abr_tests {
    use super::*;

    /// Feeds `ms` milliseconds of packets (one per 20 ms) with `depth`
    /// queued, from `t`; returns the time after and each switch as
    /// (milliseconds from `t`, stage)
    fn feed(abr: &mut Abr, t: Instant, ms: u64, depth: u32) -> (Instant, Vec<(u64, usize)>) {
        let mut switches = Vec::new();
        for i in 1..=ms / 20 {
            if let Some(stage) = abr.step(t + Duration::from_millis(i * 20), depth) {
                switches.push((i * 20, stage));
            }
        }
        (t + Duration::from_millis(ms), switches)
    }

    /// An ABR past its start grace
    fn running(table: &'static [u32; 6]) -> (Abr, Instant) {
        let mut abr = Abr::new(table);
        let (t, sw) = feed(&mut abr, Instant::now(), 4000, 0);
        assert!(sw.is_empty(), "{sw:?}");
        (abr, t)
    }

    #[test]
    fn the_top_is_the_first_stage_at_the_highest_rate() {
        assert_eq!(Abr::new(&ABR_TABLE_44K).top, 4);
        assert_eq!(Abr::new(&ABR_TABLE_48K).top, 4);
        assert_eq!(Abr::new(&ABR_TABLE_96K).top, 2);
        assert_eq!(Abr::new(&ABR_TABLE_192K).top, 2);
        assert_eq!(Abr::new(&ABR_TABLE_48K).index, 4);
    }

    #[test]
    fn a_queue_drops_to_the_bottom_and_the_climb_takes_seconds() {
        let (mut abr, t) = running(&ABR_TABLE_48K);
        let (t, sw) = feed(&mut abr, t, 80, 1);
        assert_eq!(sw, vec![(80, 0)], "4 packets with a queue: down to 160 kbps");
        let (_, sw) = feed(&mut abr, t, 60_000, 0);
        let stages: Vec<usize> = sw.iter().map(|s| s.1).collect();
        assert_eq!(stages, vec![1, 2, 3, 4]);
        // 5 s per stage, 10 s back to the stage the queue backed up at
        let at: Vec<u64> = sw.iter().map(|s| s.0).collect();
        for (got, want) in at.iter().zip([5000, 10_000, 15_000, 25_000]) {
            assert!(got.abs_diff(want) <= 200, "{at:?}");
        }
    }

    #[test]
    fn a_stall_at_the_same_stage_doubles_the_last_wait() {
        let (mut abr, t) = running(&ABR_TABLE_96K);
        let (t, _) = feed(&mut abr, t, 80, 2);
        let (t, sw) = feed(&mut abr, t, 30_000, 0);
        assert_eq!(sw.last().map(|s| s.1), Some(2));
        let (t, sw) = feed(&mut abr, t, 80, 2);
        assert_eq!(sw, vec![(80, 0)]);
        let (_, sw) = feed(&mut abr, t, 60_000, 0);
        let at: Vec<u64> = sw.iter().map(|s| s.0).collect();
        assert_eq!(sw.iter().map(|s| s.1).collect::<Vec<_>>(), vec![1, 2]);
        assert!(at[1].abs_diff(25_000) <= 200, "5 s, then 20 s: {at:?}");
    }

    #[test]
    fn one_queued_packet_starts_the_wait_again() {
        let (mut abr, t) = running(&ABR_TABLE_48K);
        let (t, _) = feed(&mut abr, t, 80, 3);
        assert_eq!(abr.index, 0);
        let (t, sw) = feed(&mut abr, t, 4000, 0);
        assert!(sw.is_empty());
        // Too few to step down, enough to break the calm
        assert_eq!(abr.step(t + Duration::from_millis(20), 1), None);
        let (_, sw) = feed(&mut abr, t + Duration::from_millis(20), 4500, 0);
        assert!(sw.is_empty(), "{sw:?}");
    }

    #[test]
    fn the_queue_of_a_starting_stream_is_not_a_stall() {
        let mut abr = Abr::new(&ABR_TABLE_48K);
        let t = Instant::now();
        let (t, sw) = feed(&mut abr, t, 2900, 3);
        assert!(sw.is_empty(), "{sw:?}");
        let (_, sw) = feed(&mut abr, t, 200, 1);
        assert_eq!(sw.first().map(|s| s.1), Some(0), "after 3 s a queue counts again");

        // A real backlog counts at once
        let mut abr = Abr::new(&ABR_TABLE_48K);
        let (_, sw) = feed(&mut abr, Instant::now(), 80, 4);
        assert_eq!(sw, vec![(80, 0)]);
    }

    #[test]
    fn the_encoder_follows_the_climb() {
        let cfg = EncoderConfig {
            sample_rate: 48000,
            channels: 2,
            quality: Quality::High,
            mtu: 1011,
            bits_per_sample: 24,
            abr: true,
            peer: Default::default(),
        };
        let mut enc = LhdcV5Encoder::new(&cfg).unwrap();
        let t = Instant::now();
        let at = |i: u64| t + Duration::from_millis(i * 20);
        for i in 1..=4 {
            enc.abr_adjust_at(at(i), 6);
        }
        let mut seen = vec![enc.bitrate_kbps()];
        for i in 5..=2000 {
            enc.abr_adjust_at(at(i), 0);
            if seen.last() != Some(&enc.bitrate_kbps()) {
                seen.push(enc.bitrate_kbps());
            }
        }
        assert_eq!(seen, vec![160, 192, 256, 320, 400]);
    }
}

#[cfg(test)]
mod guard_tests {
    use super::*;

    /// Feeds `secs` seconds of packets (100 per second) with `depth`
    /// queued, starting at `t`, applying each switch to `current`; returns
    /// the time after and the switches made
    fn feed(g: &mut Guard, t: Instant, secs: u64, depth: u32, current: &mut u32) -> (Instant, Vec<u32>) {
        let mut switches = Vec::new();
        let mut now = t;
        for _ in 0..secs * 100 {
            now += Duration::from_millis(10);
            if let Some(next) = g.step(now, depth, *current) {
                *current = next;
                switches.push(next);
            }
        }
        (now, switches)
    }

    #[test]
    fn a_backed_up_queue_steps_down_to_the_floor_and_no_further() {
        let t = Instant::now();
        let mut g = Guard::new(ffi::LHDC_QUALITY_HIGH1, t);
        let mut cur = ffi::LHDC_QUALITY_HIGH1;
        let (_, sw) = feed(&mut g, t, 10, 20, &mut cur);
        // 1000 -> 900 -> 500, at least GUARD_DOWN_HOLD apart, then it stays
        assert_eq!(sw, vec![ffi::LHDC_QUALITY_HIGH, ffi::LHDC_QUALITY_MID]);
        assert_eq!(cur, ffi::LHDC_QUALITY_MID);
    }

    #[test]
    fn a_short_queue_changes_nothing() {
        let t = Instant::now();
        let mut g = Guard::new(ffi::LHDC_QUALITY_HIGH1, t);
        let mut cur = ffi::LHDC_QUALITY_HIGH1;
        let (_, sw) = feed(&mut g, t, 30, 4, &mut cur);
        assert!(sw.is_empty(), "{sw:?}");
    }

    #[test]
    fn a_calm_minute_steps_back_up_to_the_target_only() {
        let t = Instant::now();
        let mut g = Guard::new(ffi::LHDC_QUALITY_HIGH1, t);
        let mut cur = ffi::LHDC_QUALITY_HIGH1;
        let (t, _) = feed(&mut g, t, 4, 20, &mut cur);
        assert_eq!(cur, ffi::LHDC_QUALITY_MID);
        let (t, sw) = feed(&mut g, t, 59, 0, &mut cur);
        assert!(sw.is_empty(), "not before a calm minute: {sw:?}");
        let (t, sw) = feed(&mut g, t, 2, 0, &mut cur);
        assert_eq!(sw, vec![ffi::LHDC_QUALITY_HIGH]);
        let (_, sw) = feed(&mut g, t, 200, 0, &mut cur);
        assert_eq!(sw, vec![ffi::LHDC_QUALITY_HIGH1], "back to the target, never above it");
    }

    #[test]
    fn a_step_up_that_does_not_hold_doubles_the_wait() {
        let t = Instant::now();
        let mut g = Guard::new(ffi::LHDC_QUALITY_HIGH1, t);
        let mut cur = ffi::LHDC_QUALITY_HIGH1;
        let (t, _) = feed(&mut g, t, 2, 20, &mut cur); // down to 900
        assert_eq!(cur, ffi::LHDC_QUALITY_HIGH);
        let (t, _) = feed(&mut g, t, 62, 0, &mut cur); // up to 1000
        assert_eq!(cur, ffi::LHDC_QUALITY_HIGH1);
        let (t, _) = feed(&mut g, t, 2, 20, &mut cur); // backed up again at once
        assert_eq!(cur, ffi::LHDC_QUALITY_HIGH);
        assert_eq!(g.up_wait, GUARD_UP_FIRST * 2);
        let (t, sw) = feed(&mut g, t, 100, 0, &mut cur);
        assert!(sw.is_empty(), "waits two minutes now: {sw:?}");
        let (_, sw) = feed(&mut g, t, 30, 0, &mut cur);
        assert_eq!(sw, vec![ffi::LHDC_QUALITY_HIGH1]);
    }

    #[test]
    fn a_target_below_500_kbps_is_its_own_floor() {
        let t = Instant::now();
        let mut g = Guard::new(ffi::LHDC_QUALITY_LOW, t);
        let mut cur = ffi::LHDC_QUALITY_LOW;
        let (_, sw) = feed(&mut g, t, 10, 30, &mut cur);
        assert!(sw.is_empty(), "{sw:?}");
    }
}
