//! ASHA hearing aids (proof of concept, command line only): connecting,
//! starting and stopping a stream, and the audio into it.
//!
//! A hearing aid is an LE device with the ASHA service. [`connect`] secures
//! the link and reads what the hearing aid says about itself;
//! [`AshaStream::start`] opens the audio channel of each hearing aid and
//! tells it to play. The audio then comes from one of
//! - [`AshaAudioPath`]: a capture of its own at 16 kHz (hearing aids only);
//! - [`set_tap`]: the capture of the running A2DP stream, heard a second
//!   time (bimodal: a hearing aid on one ear, another Bluetooth device on
//!   the other). The A2DP stream is untouched; the hearing aid's audio can
//!   be delayed so both arrive together.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use audio_capture::{Capture, CaptureOptions, Format, Running, Sink};
use bt_host::{AshaSink, BtHost, LeAddr, LeDeviceId, LinkStats};
use pipeline::CaptureInput;
pub use pipeline::asha::MAX_DELAY_MS;
use pipeline::asha::{ASHA_RATE, AshaChannel, AshaOut, AshaPipeline, AshaPipelineConfig};

use crate::stream::SourceRequest;

const FIND_TIMEOUT: Duration = Duration::from_secs(8);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const CHANNEL_TIMEOUT: Duration = Duration::from_secs(10);
/// AudioStatus after a start
const STATUS_TIMEOUT: Duration = Duration::from_secs(3);

/// AudioControlPoint opcodes
const OP_START: u8 = 1;
const OP_STOP: u8 = 2;
const OP_STATUS: u8 = 3;
/// Start: G.722 at 16 kHz
const CODEC_G722: u8 = 1;
/// Start: the audio type Android sends for everything (unknown; 3 would be
/// media)
const AUDIO_TYPE_UNKNOWN: u8 = 0;
/// Status: the other side of the pair connected
const STATUS_OTHER_CONNECTED: u8 = 1;
/// Status: the connection parameters changed, followed by the new interval
/// (as Android sends it)
const STATUS_CONNECTION_UPDATE: u8 = 2;
/// How long the 20 ms connection interval is waited for before the start
const INTERVAL_TIMEOUT: Duration = Duration::from_secs(3);
/// ReadOnlyProperties: capabilities
const CAP_RIGHT: u8 = 0x01;
const CAP_BINAURAL: u8 = 0x02;
/// ReadOnlyProperties: supported codecs, G.722 at 16 kHz
const CODECS_G722: u16 = 1 << 1;

/// A connected hearing aid and what it says about itself
pub struct HearingAid {
    pub device: LeDeviceId,
    /// The right side (else the left)
    pub right: bool,
    /// One of a pair
    pub binaural: bool,
    /// The same on both sides of a pair
    pub hi_sync_id: [u8; 8],
    /// How long the hearing aid takes to play what it got
    pub render_delay_ms: u16,
    psm: u16,
    control: u16,
    volume: Option<u16>,
    status: Option<mpsc::Receiver<Vec<u8>>>,
}

impl HearingAid {
    pub fn side(&self) -> &'static str {
        if self.right { "right" } else { "left" }
    }
}

/// ReadOnlyProperties: (right, binaural, HiSyncId, render delay, G.722)
fn parse_properties(v: &[u8]) -> Option<(bool, bool, [u8; 8], u16, bool)> {
    if v.len() < 17 {
        return None;
    }
    let caps = v[1];
    let codecs = u16::from_le_bytes([v[15], v[16]]);
    Some((
        caps & CAP_RIGHT != 0,
        caps & CAP_BINAURAL != 0,
        v[2..10].try_into().ok()?,
        u16::from_le_bytes([v[11], v[12]]),
        codecs & CODECS_G722 != 0,
    ))
}

/// Finds and connects a hearing aid, secures the link (pairing when needed)
/// and reads its ASHA service. Err: why, for the user.
pub fn connect(host: &BtHost, address: [u8; 6]) -> Result<HearingAid, String> {
    let (addr, name) = match host.le_find(address, FIND_TIMEOUT) {
        Some(found) => (found.addr, found.name),
        None => (LeAddr { addr: address, addr_type: LeAddr::PUBLIC }, String::new()),
    };
    let device = host.le_connect_bare(addr, &name, CONNECT_TIMEOUT)?;
    // In the background while its service is read: the 20 ms interval, the
    // 2M PHY and long link layer PDUs
    host.asha_prepare_link(device);
    let services = host.le_services(device);
    let Some(service) = services.iter().find(|s| s.uuid16 == bt_host::ASHA_SERVICE) else {
        host.le_disconnect(device);
        return Err("not a hearing aid (no ASHA service)".into());
    };
    let handle = |uuid: [u8; 16]| service.chars.iter().find(|c| c.uuid128 == uuid).map(|c| c.value_handle);
    let (Some(properties), Some(psm), Some(control)) = (
        handle(bt_host::ASHA_READ_ONLY_PROPERTIES),
        handle(bt_host::ASHA_LE_PSM_OUT),
        handle(bt_host::ASHA_AUDIO_CONTROL_POINT),
    ) else {
        host.le_disconnect(device);
        return Err("its ASHA service is incomplete".into());
    };
    let fail = |what: &str, status: u8| {
        host.le_disconnect(device);
        format!("{what} could not be read (0x{status:02x})")
    };
    let value = host.le_read(device, properties).map_err(|s| fail("its properties", s))?;
    let Some((right, binaural, hi_sync_id, render_delay_ms, g722)) = parse_properties(&value) else {
        host.le_disconnect(device);
        return Err("its properties are too short".into());
    };
    if !g722 {
        host.le_disconnect(device);
        return Err("it does not list G.722".into());
    }
    let psm = match host.le_read(device, psm).map_err(|s| fail("its audio channel", s))?.as_slice() {
        [a, b, ..] => u16::from_le_bytes([*a, *b]),
        _ => {
            host.le_disconnect(device);
            return Err("its audio channel number is too short".into());
        }
    };
    let status = handle(bt_host::ASHA_AUDIO_STATUS).and_then(|h| match host.le_watch(device, h) {
        Ok(rx) => Some(rx),
        Err(e) => {
            tracing::debug!("ASHA: device {device}: no AudioStatus notifications (0x{e:02x})");
            None
        }
    });
    tracing::info!(
        "ASHA: device {device}: {} side{}, render delay {render_delay_ms} ms",
        if right { "right" } else { "left" },
        if binaural { " of a pair" } else { "" }
    );
    Ok(HearingAid {
        device,
        right,
        binaural,
        hi_sync_id,
        render_delay_ms,
        psm,
        control,
        volume: handle(bt_host::ASHA_VOLUME),
        status,
    })
}

/// What each hearing aid of a stream plays: its side of the stereo sound
/// when there are two of them or `own_side` is set (bimodal: the other ear
/// has another device), else both sides mixed
fn channels(aids: &[HearingAid], own_side: bool) -> Vec<AshaChannel> {
    aids.iter()
        .map(|a| match (aids.len() > 1 || own_side, a.right) {
            (false, _) => AshaChannel::Mono,
            (true, false) => AshaChannel::Left,
            (true, true) => AshaChannel::Right,
        })
        .collect()
}

/// A started ASHA stream: the hearing aids play what their sinks get
pub struct AshaStream {
    aids: Vec<(HearingAid, AshaSink)>,
    channels: Vec<AshaChannel>,
    /// How much later the hearing aids get the audio (bimodal)
    pub delay_ms: u32,
}

impl AshaStream {
    /// Opens the audio channel of every hearing aid, then starts them at
    /// `volume` (-128 muted to 0). `own_side`: a single hearing aid plays
    /// its side only. On an error the hearing aids stay connected.
    pub fn start(
        host: &BtHost,
        aids: Vec<HearingAid>,
        volume: i8,
        own_side: bool,
        delay_ms: u32,
    ) -> Result<AshaStream, String> {
        let channels = channels(&aids, own_side);
        let mut open = Vec::new();
        for aid in aids {
            match host.asha_open(aid.device, aid.psm, CHANNEL_TIMEOUT) {
                Ok(sink) => open.push((aid, sink)),
                Err(e) => {
                    let side = aid.side();
                    for (_, sink) in open {
                        host.asha_close(sink);
                    }
                    return Err(format!("{side} hearing aid: {e}"));
                }
            }
        }
        let stream = AshaStream { aids: open, channels, delay_ms };
        // The connection update is done before the start (the specification's
        // order); each hearing aid is told the interval, as Android does
        let until = Instant::now() + INTERVAL_TIMEOUT;
        for (aid, _) in &stream.aids {
            let mut interval = host.asha_link_interval(aid.device);
            while interval != Some(bt_host::ASHA_INTERVAL_20_MS) && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(50));
                interval = host.asha_link_interval(aid.device);
            }
            match interval {
                Some(bt_host::ASHA_INTERVAL_20_MS) => {
                    let update = vec![OP_STATUS, STATUS_CONNECTION_UPDATE, bt_host::ASHA_INTERVAL_20_MS as u8];
                    for (each, _) in &stream.aids {
                        let _ = host.le_write_command(each.device, each.control, update.clone());
                    }
                }
                other => tracing::warn!(
                    "ASHA: {} hearing aid: the connection interval is {:?} x 1.25 ms, not 20 ms; the sound may break up",
                    aid.side(),
                    other
                ),
            }
        }
        for (index, (aid, _)) in stream.aids.iter().enumerate() {
            // Old notifications are not this start's answer
            if let Some(rx) = &aid.status {
                while rx.try_recv().is_ok() {}
            }
            // The last byte: the other side streams already (the side
            // started first is told below)
            let start = vec![OP_START, CODEC_G722, AUDIO_TYPE_UNKNOWN, volume as u8, u8::from(index > 0)];
            if let Err(status) = host.le_write(aid.device, aid.control, start) {
                let e = format!("{} hearing aid: the start was refused (0x{status:02x})", aid.side());
                stream.stop(host);
                return Err(e);
            }
            if let Some(rx) = &aid.status {
                match rx.recv_timeout(STATUS_TIMEOUT) {
                    Ok(v) if v.first() == Some(&0) => {}
                    Ok(v) => {
                        let e =
                            format!("{} hearing aid: it did not start ({:?})", aid.side(), v.first().map(|b| *b as i8));
                        stream.stop(host);
                        return Err(e);
                    }
                    Err(_) => tracing::warn!("ASHA: {} hearing aid: no answer to the start, going on", aid.side()),
                }
            }
            // The sides that stream already learn that this one is there
            for (earlier, _) in &stream.aids[..index] {
                let _ = host.le_write_command(earlier.device, earlier.control, vec![OP_STATUS, STATUS_OTHER_CONNECTED]);
            }
        }
        Ok(stream)
    }

    pub fn aids(&self) -> impl Iterator<Item = &HearingAid> {
        self.aids.iter().map(|(a, _)| a)
    }

    /// Each hearing aid with the handle of its Volume characteristic (those
    /// that have one), for `BtHost::asha_volume_targets`
    pub fn volume_targets(&self) -> Vec<(LeDeviceId, u16)> {
        self.aids().filter_map(|a| Some((a.device, a.volume?))).collect()
    }

    /// The longest render delay of the hearing aids
    pub fn render_delay_ms(&self) -> u16 {
        self.aids().map(|a| a.render_delay_ms).max().unwrap_or(0)
    }

    /// Sets the volume of every hearing aid (-128 muted to 0)
    pub fn set_volume(&self, host: &BtHost, volume: i8) {
        for (aid, _) in &self.aids {
            if let Some(handle) = aid.volume
                && let Err(status) = host.le_write_command(aid.device, handle, vec![volume as u8])
            {
                tracing::debug!("ASHA: {} hearing aid: volume not set (0x{status:02x})", aid.side());
            }
        }
    }

    /// The encode thread for audio in `input`, and where to put that audio
    pub fn pipeline(
        &self,
        input: pipeline::InputFormat,
        stats: Arc<LinkStats>,
    ) -> Result<(AshaPipeline, CaptureInput), String> {
        let outs = self
            .aids
            .iter()
            .zip(&self.channels)
            .map(|((_, sink), channel)| (*channel, Box::new(sink.clone()) as Box<dyn AshaOut>))
            .collect();
        AshaPipeline::start(AshaPipelineConfig { input, delay_ms: self.delay_ms }, outs, stats)
    }

    /// A hearing aid's channel or link is gone
    pub fn lost(&self) -> bool {
        self.aids.iter().any(|(_, sink)| sink.closed())
    }

    /// "left 250 sent, 0 dropped; right ..."
    pub fn stats_line(&self) -> String {
        self.aids
            .iter()
            .map(|(aid, sink)| format!("{} {} sent, {} dropped", aid.side(), sink.sent(), sink.dropped()))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// Tells the hearing aids to stop and closes their channels (they stay
    /// connected)
    pub fn stop(self, host: &BtHost) {
        for (aid, sink) in self.aids {
            if !sink.closed() {
                let _ = host.le_write(aid.device, aid.control, vec![OP_STOP]);
            }
            host.asha_close(sink);
        }
    }
}

/// The capture's end of the ASHA encode thread's ring
struct InputSink(CaptureInput);

impl Sink for InputSink {
    fn audio(&mut self, data: &[u8], _frames: usize, _format: Format) {
        self.0.push(data);
    }

    fn silence(&mut self, frames: usize, _format: Format) {
        self.0.push_silence(frames);
    }

    fn discontinuity(&mut self, lost_frames: Option<u64>) {
        self.0.discontinuity(lost_frames);
    }
}

/// Audio into an ASHA stream from a capture of its own (or the test tone),
/// at 16 kHz when Windows converts to it
pub struct AshaAudioPath {
    capture: Option<Running>,
    pipeline: Option<AshaPipeline>,
    format: Format,
}

impl AshaAudioPath {
    pub fn start(source: &SourceRequest, stream: &AshaStream, stats: Arc<LinkStats>) -> Result<AshaAudioPath, String> {
        let test_tone = matches!(source, SourceRequest::TestTone);
        let options = CaptureOptions { preferred_rate: Some(ASHA_RATE), fill_silence: !test_tone };
        let capture = Capture::open(&crate::le_stream::LeAudioPath::capture_source(source)?, &options)
            .map_err(|e| e.to_string())?;
        let format = capture.format();
        let (pipeline, input) = stream.pipeline(crate::stream::input_format(format), stats)?;
        let running = match capture.start(Box::new(InputSink(input))) {
            Ok(r) => r,
            Err(e) => {
                pipeline.stop();
                return Err(e.to_string());
            }
        };
        Ok(AshaAudioPath { capture: Some(running), pipeline: Some(pipeline), format })
    }

    pub fn format(&self) -> Format {
        self.format
    }

    /// The capture ended on its own (its device went away)
    pub fn ended(&self) -> bool {
        self.capture.as_ref().is_none_or(Running::ended)
    }

    /// Producer first, then the encode thread
    pub fn stop(mut self) {
        self.stop_now();
    }

    fn stop_now(&mut self) {
        if let Some(c) = self.capture.take() {
            c.stop();
        }
        if let Some(p) = self.pipeline.take() {
            p.stop();
        }
    }
}

impl Drop for AshaAudioPath {
    fn drop(&mut self) {
        self.stop_now();
    }
}

/// Bimodal: the hearing aids' share of the A2DP stream's capture. The
/// encode thread starts with the first audio, for the format it comes in.
struct Tap {
    outs: Vec<(AshaChannel, AshaSink)>,
    delay_ms: u32,
    /// Its own counters: the A2DP stream's count that capture once
    stats: Arc<LinkStats>,
    running: Option<(Format, AshaPipeline, CaptureInput)>,
    started: Instant,
}

static TAP: Mutex<Option<Tap>> = Mutex::new(None);

/// From now on the hearing aids of `stream` also get what the A2DP stream
/// captures ([`clear_tap`] ends it)
pub fn set_tap(stream: &AshaStream) {
    let outs = stream.aids.iter().zip(&stream.channels).map(|((_, sink), c)| (*c, sink.clone())).collect();
    *TAP.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(Tap { outs, delay_ms: stream.delay_ms, stats: Arc::default(), running: None, started: Instant::now() });
}

pub fn clear_tap() {
    let tap = TAP.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some((_, pipeline, _)) = tap.and_then(|t| t.running) {
        pipeline.stop();
    }
}

/// A hearing aid beside a running A2DP stream (bimodal), on a thread of its
/// own: connected and started once Bluetooth LE is up, fed from the A2DP
/// stream's capture ([`set_tap`]), and looked for again when it goes away.
/// The A2DP stream never waits for it.
pub struct Beside {
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Beside {
    /// The volume the hearing aid plays at (-128 muted to 0)
    const VOLUME: i8 = -20;
    /// Waits for the LE core after the A2DP stream started
    const READY_WAIT: Duration = Duration::from_secs(5);
    /// The pause before the hearing aid is looked for again
    const RETRY: Duration = Duration::from_secs(5);

    pub fn start(host: Arc<BtHost>, aid: [u8; 6], delay_ms: u32) -> Beside {
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopped = {
            let stop = stop.clone();
            move || stop.load(std::sync::atomic::Ordering::Acquire)
        };
        let thread = std::thread::Builder::new()
            .name("asha-beside".into())
            .spawn(move || Beside::run(&host, aid, delay_ms, &stopped))
            .map_err(|e| tracing::error!("ASHA: the hearing aid's thread did not start: {e}"))
            .ok();
        Beside { stop, thread }
    }

    fn run(host: &BtHost, aid: [u8; 6], delay_ms: u32, stopped: &dyn Fn() -> bool) {
        let pause = |d: Duration| {
            let until = Instant::now() + d;
            while Instant::now() < until && !stopped() {
                std::thread::sleep(Duration::from_millis(100));
            }
        };
        let until = Instant::now() + Beside::READY_WAIT;
        while !host.le_ready() && Instant::now() < until && !stopped() {
            std::thread::sleep(Duration::from_millis(100));
        }
        if !host.le_ready() {
            if !stopped() {
                tracing::warn!("ASHA: Bluetooth LE is not running, the hearing aid beside this stream is not used");
            }
            return;
        }
        let mut failures = 0u32;
        while !stopped() {
            let started =
                connect(host, aid).and_then(|aid| AshaStream::start(host, vec![aid], Beside::VOLUME, true, delay_ms));
            let stream = match started {
                Ok(s) => s,
                Err(e) => {
                    // Said once: the hearing aid may be off for the whole stream
                    if failures == 0 && !stopped() {
                        tracing::warn!("ASHA: the hearing aid beside this stream is not playing: {e}");
                    }
                    failures += 1;
                    pause(Beside::RETRY);
                    continue;
                }
            };
            failures = 0;
            tracing::info!("ASHA: a hearing aid plays beside this stream, {delay_ms} ms later");
            set_tap(&stream);
            while !stopped() && !stream.lost() {
                std::thread::sleep(Duration::from_millis(100));
            }
            clear_tap();
            tracing::info!("ASHA: {}", stream.stats_line());
            let devices: Vec<LeDeviceId> = stream.aids().map(|a| a.device).collect();
            stream.stop(host);
            for device in devices {
                host.le_disconnect(device);
            }
            if !stopped() {
                tracing::info!("ASHA: the hearing aid went away, looking for it again");
                pause(Beside::RETRY);
            }
        }
    }

    /// Ends it: the hearing aid stops and is disconnected
    pub fn stop(mut self) {
        self.halt();
    }

    fn halt(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Beside {
    fn drop(&mut self) {
        self.halt();
    }
}

/// The tap's input for audio in `format` (capture thread; never waits)
fn with_tap(format: Format, f: impl FnOnce(&mut CaptureInput)) {
    let Ok(mut guard) = TAP.try_lock() else { return };
    let Some(tap) = guard.as_mut() else { return };
    if tap.running.as_ref().is_some_and(|(running, ..)| *running != format)
        && let Some((_, pipeline, _)) = tap.running.take()
    {
        pipeline.stop();
    }
    if tap.running.is_none() {
        let outs = tap.outs.iter().map(|(c, sink)| (*c, Box::new(sink.clone()) as Box<dyn AshaOut>)).collect();
        let config = AshaPipelineConfig { input: crate::stream::input_format(format), delay_ms: tap.delay_ms };
        match AshaPipeline::start(config, outs, tap.stats.clone()) {
            Ok((pipeline, input)) => {
                tracing::info!(
                    "ASHA: hearing the A2DP stream's capture ({} Hz, {} channels) after {} ms",
                    format.sample_rate,
                    format.channels,
                    tap.started.elapsed().as_millis()
                );
                tap.running = Some((format, pipeline, input));
            }
            Err(e) => {
                tracing::error!("ASHA: encode thread: {e}");
                *guard = None;
                return;
            }
        }
    }
    if let Some((_, _, input)) = tap.running.as_mut() {
        f(input);
    }
}

/// The A2DP stream's capture: audio for the hearing aids too
pub(crate) fn tap_audio(data: &[u8], format: Format) {
    with_tap(format, |input| input.push(data));
}

pub(crate) fn tap_silence(frames: usize, format: Format) {
    with_tap(format, |input| input.push_silence(frames));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aid(right: bool) -> HearingAid {
        HearingAid {
            device: LeDeviceId(0),
            right,
            binaural: true,
            hi_sync_id: [0; 8],
            render_delay_ms: 0,
            psm: 0x80,
            control: 1,
            volume: None,
            status: None,
        }
    }

    #[test]
    fn properties_are_read_as_the_hearing_aid_sends_them() {
        let mut v = vec![1, CAP_RIGHT | CAP_BINAURAL];
        v.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        v.extend_from_slice(&[1, 160, 0, 0, 0, 2, 0]);
        assert_eq!(parse_properties(&v), Some((true, true, [1, 2, 3, 4, 5, 6, 7, 8], 160, true)));
        v[1] = 0;
        v[15] = 0;
        assert_eq!(parse_properties(&v), Some((false, false, [1, 2, 3, 4, 5, 6, 7, 8], 160, false)));
        assert_eq!(parse_properties(&v[..16]), None);
    }

    #[test]
    fn a_pair_plays_stereo_and_one_hearing_aid_both_sides() {
        assert_eq!(channels(&[aid(false), aid(true)], false), [AshaChannel::Left, AshaChannel::Right]);
        assert_eq!(channels(&[aid(true)], false), [AshaChannel::Mono]);
        assert_eq!(channels(&[aid(true)], true), [AshaChannel::Right]);
        assert_eq!(channels(&[aid(false)], true), [AshaChannel::Left]);
    }
}
