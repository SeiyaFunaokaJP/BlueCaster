//! A hearing aid session (ASHA, a proof of concept): the session thread of
//! [`crate::Service`] for a [`StreamRequest`] whose `asha` says the hearing
//! aids are the device.
//!
//! Same lifecycle as the LE Audio session ([`crate::le_stream`]), much
//! shorter: refuse while Bluetooth LE is off for the stack
//! (error.hearing_aid_needs_le); BTstack up with LE (wait for
//! `BtHost::asha_ready`, else error.hearing_aid_unavailable); connect the
//! hearing aid and, when it is one of a pair, look for the other side by
//! what both advertise and connect it too (not found: the stream plays to
//! the one, status.hearing_aid_one); start them ([`AshaStream::start`]);
//! capture at 16 kHz into the ASHA encode thread; the volume slider and
//! VolumeSync through `le_remote_volume` / `le_request_volume`, which the
//! ASHA module answers while its hearing aids stream. A hearing aid that
//! goes away ends the stream; with auto-reconnect it is set up again, a few
//! times.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bt_host::BtHost;

use crate::asha::{AshaAudioPath, AshaStream, HearingAid};
use crate::messages::Msg;
use crate::mute_output::OutputMute;
use crate::service::Inner;
use crate::stream::{DefaultDeviceGuard, SourceRequest, StreamInfo, StreamRequest, StreamState, VolumeSync};

/// What the stream is shown as
pub const CODEC_NAME: &str = "G.722 (ASHA)";
/// Waits for the LE core after BTstack started
const READY_WAIT: Duration = Duration::from_secs(5);
/// How long the other side of a pair is looked for
const PARTNER_SCAN: Duration = Duration::from_secs(4);
/// The volume the hearing aids start with (-128 muted to 0): the slider and
/// VolumeSync change it from there
const START_VOLUME: i8 = -20;
/// Set-ups after a hearing aid went away
const RECONNECT_ATTEMPTS: u32 = 3;
const RECONNECT_WAIT: Duration = Duration::from_secs(3);

/// A stream that plays
struct Playing {
    stream: AshaStream,
    audio: AshaAudioPath,
    /// One of a pair plays alone: the other side was not found
    alone: bool,
    _default: DefaultDeviceGuard,
}

/// The other side of `aid`'s pair, by what both advertise; None when it
/// does not show up in time (or `stopped`)
fn find_partner(host: &BtHost, aid: &HearingAid, stopped: &dyn Fn() -> bool) -> Option<[u8; 6]> {
    if !host.le_scan_start() {
        return None;
    }
    let until = Instant::now() + PARTNER_SCAN;
    let mut found = None;
    while found.is_none() && Instant::now() < until && !stopped() {
        std::thread::sleep(Duration::from_millis(200));
        found = host.le_scan_results().into_iter().find_map(|r| {
            let seen = r.hearing_aid?;
            (r.connectable && seen.binaural && seen.set == aid.hi_sync_id[..4] && seen.right != aid.right)
                .then_some(r.addr.addr)
        });
    }
    host.le_scan_stop();
    found
}

/// Connects the hearing aid of `req`, the other side of its pair when there
/// is one, and starts them
fn set_up(host: &Arc<BtHost>, req: &StreamRequest, stopped: &dyn Fn() -> bool) -> Result<Playing, Msg> {
    let connect_error = |e: String| Msg::with("error.hearing_aid_connect", &[&e]);
    let first = crate::asha::connect(host, req.device).map_err(connect_error)?;
    let mut aids = vec![first];
    let mut alone = false;
    if aids[0].binaural && !stopped() {
        match find_partner(host, &aids[0], stopped) {
            Some(address) => match crate::asha::connect(host, address) {
                Ok(other) if other.hi_sync_id == aids[0].hi_sync_id && other.right != aids[0].right => {
                    aids.push(other);
                }
                Ok(other) => {
                    tracing::warn!("ASHA: the hearing aid found is not the other side of the pair, left out");
                    host.le_disconnect(other.device);
                    alone = true;
                }
                Err(e) => {
                    tracing::warn!("ASHA: the other side of the pair did not connect ({e}), playing to one");
                    alone = true;
                }
            },
            None => {
                tracing::info!("ASHA: the other side of the pair was not found, playing to one");
                alone = true;
            }
        }
    }
    // Left first, as Android starts them
    aids.sort_by_key(|a| a.right);
    if stopped() {
        return Err(Msg::new("status.ready"));
    }
    let stream = AshaStream::start(host, aids, START_VOLUME, false, 0)
        .map_err(|e| Msg::with("error.hearing_aid_stream", &[&e]))?;
    host.asha_volume_targets(stream.volume_targets(), START_VOLUME);
    let default = match req.source.default_switch_device() {
        Some((id, name)) => DefaultDeviceGuard::switch_to(id, name),
        None => DefaultDeviceGuard(None),
    };
    match AshaAudioPath::start(&req.source, &stream, host.stats().clone()) {
        Ok(audio) => Ok(Playing { stream, audio, alone, _default: default }),
        Err(e) => {
            tracing::error!("ASHA: audio capture: {e}");
            host.asha_volume_targets(Vec::new(), 0);
            stream.stop(host);
            Err(Msg::new("error.wasapi_init"))
        }
    }
}

/// Ends what plays: the capture, the hearing aids, their links
fn tear_down(host: &Arc<BtHost>, playing: Playing) {
    let Playing { stream, audio, .. } = playing;
    audio.stop();
    host.asha_volume_targets(Vec::new(), 0);
    tracing::info!("ASHA: {}", stream.stats_line());
    stream.stop(host);
    host.le_disconnect_all();
}

/// The session thread for `req` (`req.asha` says the hearing aids are the
/// device). Returns when stopped or on an error (state reported), as
/// `stream::run` does.
pub(crate) fn run(inner: &Arc<Inner>, req: StreamRequest) {
    tracing::info!("hearing aid stream ({})", crate::diag::quoted(&req.device_name));
    let stopped = || inner.stop_requested();
    let ready = || inner.notify(StreamState::Idle, Msg::new("status.ready"));
    if !inner.asha_wanted() {
        return inner.notify(StreamState::Error, Msg::new("error.hearing_aid_needs_le"));
    }
    if matches!(req.source, SourceRequest::Application { .. }) {
        return inner.notify(StreamState::Error, Msg::new("error.le_audio_capture"));
    }
    inner.notify(StreamState::Connecting, Msg::new("status.initializing_btstack"));
    let host = match inner.ensure_host() {
        Ok(h) => h,
        Err(m) => return inner.notify(StreamState::Error, m),
    };
    let until = Instant::now() + READY_WAIT;
    while !host.asha_ready() && Instant::now() < until && !stopped() {
        inner.wait_tick(Duration::from_millis(100));
    }
    if stopped() {
        return ready();
    }
    if !host.asha_ready() {
        return inner.notify(StreamState::Error, Msg::new("error.hearing_aid_unavailable"));
    }

    inner.notify(StreamState::Connecting, Msg::new("status.connecting_device"));
    let mut playing = match set_up(&host, &req, &stopped) {
        Ok(p) => Some(p),
        Err(m) => {
            host.le_disconnect_all();
            return if stopped() { ready() } else { inner.notify(StreamState::Error, m) };
        }
    };
    let report = |p: &Playing| {
        let aids = p.stream.aids().count() as u32;
        inner.notify(
            StreamState::Streaming,
            Msg::new(if p.alone { "status.hearing_aid_one" } else { "status.connected" }),
        );
        let format = p.audio.format();
        inner.stream_info(StreamInfo {
            codec: CODEC_NAME.into(),
            bitrate_kbps: 64 * aids,
            sample_rate: pipeline::asha::ASHA_RATE,
            channels: aids,
            bits: 0,
            source_sample_rate: format.sample_rate,
            source_channels: u32::from(format.channels),
            source_bits: (format.sample_format.bytes_per_sample() * 8) as u32,
            requested_codec: None,
            requested_in_use: false,
            abr: false,
            bitrate_capped: false,
            hi_res_set: false,
            low_latency: false,
            source: req.source.info(),
            details: None,
        });
    };
    if let Some(p) = &playing {
        report(p);
    }
    inner.save_le_device(&bt_host::addr_str(&req.device), &req.device_name);

    let sync_volume = req.source != SourceRequest::TestTone && req.volume_sync != VolumeSync::Off;
    let mut volume = crate::volume_sync::EndpointChoice::for_stream(&req.volume_sync, &req.source)
        .map(crate::volume_sync::VolumeSyncer::new);
    let mutes_output = req.mutes_output(&req.source);
    let mut output_mute = OutputMute::new();
    let mut shown_volume = None;
    // The end was reported already (the hearing aids are off, or could not
    // be reached again)
    let mut reported = false;
    'session: while !stopped() {
        inner.wait_tick(Duration::from_millis(200));
        if stopped() {
            break;
        }
        if let Some(v) = volume.as_mut() {
            v.tick(sync_volume, &crate::volume_sync::LeHeadphones(&host));
        }
        if let Some(level) = host.le_remote_volume().filter(|&l| Some(l) != shown_volume) {
            shown_volume = Some(level);
            inner.send_event(crate::service::ServiceEvent::Volume(level));
        }
        output_mute.tick(mutes_output);
        if playing.as_ref().is_some_and(|p| !p.stream.lost() && !p.audio.ended()) {
            continue;
        }
        // A hearing aid (or the capture) went away
        tracing::warn!("ASHA: the stream was lost");
        if let Some(p) = playing.take() {
            tear_down(&host, p);
        }
        if !inner.auto_reconnect() {
            inner.notify(StreamState::Idle, Msg::new("status.device_off"));
            reported = true;
            break 'session;
        }
        inner.notify(StreamState::Reconnecting, Msg::new("status.connection_lost"));
        let mut again = None;
        for attempt in 1..=RECONNECT_ATTEMPTS {
            let wait = Instant::now() + RECONNECT_WAIT;
            while Instant::now() < wait && !stopped() {
                inner.wait_tick(Duration::from_millis(100));
            }
            if stopped() {
                break;
            }
            inner.notify(
                StreamState::Reconnecting,
                Msg::with("status.reconnect_attempt", &[&attempt, &RECONNECT_ATTEMPTS]),
            );
            match set_up(&host, &req, &stopped) {
                Ok(p) => {
                    again = Some(p);
                    break;
                }
                Err(m) => {
                    tracing::warn!("ASHA: set-up {attempt} failed: {}", m.english());
                    host.le_disconnect_all();
                }
            }
        }
        match again {
            Some(p) => {
                shown_volume = None;
                report(&p);
                playing = Some(p);
            }
            None => {
                if !stopped() {
                    inner.notify(StreamState::Error, Msg::new("error.reconnect_failed"));
                    reported = true;
                }
                break 'session;
            }
        }
    }
    if let Some(p) = playing.take() {
        tear_down(&host, p);
    }
    if let Some(mut v) = volume.take() {
        v.stop();
    }
    drop(output_mute);
    if !reported {
        ready();
    }
}
