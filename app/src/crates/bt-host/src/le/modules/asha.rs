//! ASHA (Audio Streaming for Hearing Aids), proof of concept: the audio
//! channel of a hearing aid.
//!
//! A hearing aid has the ASHA GATT service (0xFDF0) and takes its audio on
//! an LE credit based L2CAP channel, on the PSM its LE_PSM_OUT
//! characteristic names: one SDU per 20 ms, a sequence counter byte and 160
//! bytes of G.722 (16 kHz mono). A pair is two devices, one channel each.
//!
//! This module keeps the channels. The GATT side (ReadOnlyProperties, the
//! PSM, AudioControlPoint, AudioStatus, Volume) is done by the caller with
//! the worker API ([`BtHost::le_read`], `le_write`, `le_watch`), as is the
//! G.722 encoding (pipeline's ASHA encode thread):
//!
//! encode thread --[`AshaSink::push`] (a short queue per hearing aid)-->
//! BTstack thread: one SDU at a time per channel, the next one when
//! L2CAP_EVENT_PACKET_SENT came. BTstack waits for the hearing aid's
//! credits; while it does, the queue fills and its oldest SDU is dropped,
//! so a hearing aid that stalls never delays the audio.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use btstack_sys::events::{cbm_channel_opened, l2cap_channel_closed, l2cap_packet_sent};
use tracing::{debug, info, warn};

use super::super::LeDeviceId;
use super::super::core::{LeCtx, LeDone, LePoster};
use super::super::module::{LeModule, LeModuleEntry, LeModuleSetup};
use crate::BtHost;

/// The ASHA service
pub const UUID_SERVICE_ASHA: u16 = 0xFDF0;
/// ReadOnlyProperties: version, capabilities, HiSyncId (8), feature map,
/// render delay (2, ms), reserved (2), supported codecs (2)
pub const UUID_READ_ONLY_PROPERTIES: [u8; 16] = uuid128(0x6333651e_c481_4a3e_9169_7c902aad37bb);
/// AudioControlPoint: start (1, codec, audio type, volume, other side
/// connected), stop (2), status (3, what changed)
pub const UUID_AUDIO_CONTROL_POINT: [u8; 16] = uuid128(0xf0d4de7e_4a88_476c_9d9f_1937b0996cc0);
/// AudioStatus: notified after start / stop, 0 = ok
pub const UUID_AUDIO_STATUS: [u8; 16] = uuid128(0x38663f1a_e711_4cac_b641_326b56404837);
/// Volume: one signed byte, -128 (muted) to 0
pub const UUID_VOLUME: [u8; 16] = uuid128(0x00e4ca9e_ab14_41e4_8823_f9e70c7e91df);
/// LE_PSM_OUT: the PSM of the audio channel (2)
pub const UUID_LE_PSM_OUT: [u8; 16] = uuid128(0x2d410339_82b6_42aa_b34e_e2e01df8cc1a);

/// One SDU: the sequence counter and 20 ms of G.722
pub const SDU_BYTES: usize = 161;
/// SDUs waiting for one hearing aid (80 ms); the oldest goes when full
const MAX_QUEUED: usize = 4;
/// The connection interval ASHA streams with: 20 ms, in 1.25 ms units
const INTERVAL_20_MS: u16 = 16;
/// Connection events a hearing aid may skip while nothing is sent
const LATENCY: u16 = 10;
/// Supervision timeout: 2 s, in 10 ms units
const SUPERVISION_TIMEOUT: u16 = 200;

const fn uuid128(v: u128) -> [u8; 16] {
    v.to_be_bytes()
}

/// What the encode thread and the BTstack thread share of one channel
#[derive(Default)]
struct Shared {
    frames: Mutex<VecDeque<Vec<u8>>>,
    /// A pump is posted and has not run yet
    kicked: AtomicBool,
    sent: AtomicU64,
    dropped: AtomicU64,
    closed: AtomicBool,
}

/// Where the encode thread puts the SDUs of one hearing aid
/// ([`BtHost::asha_open`])
#[derive(Clone)]
pub struct AshaSink {
    cid: u16,
    shared: Arc<Shared>,
    poster: LePoster,
}

impl AshaSink {
    /// One SDU ([`SDU_BYTES`]); the oldest one waiting is dropped when the
    /// hearing aid does not keep up
    pub fn push(&self, sdu: Vec<u8>) {
        {
            let mut frames = self.shared.frames.lock().unwrap_or_else(|e| e.into_inner());
            if frames.len() >= MAX_QUEUED {
                frames.pop_front();
                self.shared.dropped.fetch_add(1, Ordering::Relaxed);
            }
            frames.push_back(sdu);
        }
        if !self.shared.kicked.swap(true, Ordering::AcqRel) {
            let cid = self.cid;
            self.poster.post(move |ctx| AshaModule::pump(ctx, cid));
        }
    }

    /// SDUs handed to the link
    pub fn sent(&self) -> u64 {
        self.shared.sent.load(Ordering::Relaxed)
    }

    /// SDUs dropped because the hearing aid did not keep up
    pub fn dropped(&self) -> u64 {
        self.shared.dropped.load(Ordering::Relaxed)
    }

    /// The channel or its link is gone
    pub fn closed(&self) -> bool {
        self.shared.closed.load(Ordering::Acquire)
    }
}

struct Channel {
    dev: LeDeviceId,
    /// [`BtHost::asha_open`] waits for the channel
    opening: Option<LeDone>,
    open: bool,
    /// An SDU is going out (until L2CAP_EVENT_PACKET_SENT)
    busy: bool,
    shared: Arc<Shared>,
}

pub struct AshaModule {
    poster: LePoster,
    /// By local cid
    channels: BTreeMap<u16, Channel>,
}

impl AshaModule {
    /// Sends the next waiting SDU of the channel, if none is going out
    fn pump(ctx: &mut LeCtx<'_>, cid: u16) {
        let Some(m) = ctx.module_mut::<AshaModule>() else { return };
        let Some(ch) = m.channels.get_mut(&cid) else { return };
        ch.shared.kicked.store(false, Ordering::Release);
        if !ch.open || ch.busy {
            return;
        }
        let Some(frame) = ch.shared.frames.lock().unwrap_or_else(|e| e.into_inner()).pop_front() else { return };
        ch.busy = true;
        let shared = ch.shared.clone();
        let status = ctx.stack().cbm_send(cid, &frame);
        if status == 0 {
            shared.sent.fetch_add(1, Ordering::Relaxed);
            return;
        }
        debug!("ASHA: sending on channel 0x{cid:04x} failed (0x{status:02x})");
        shared.dropped.fetch_add(1, Ordering::Relaxed);
        if let Some(ch) = ctx.module_mut::<AshaModule>().and_then(|m| m.channels.get_mut(&cid)) {
            ch.busy = false;
        }
    }

    /// The channels' events ([`btstack_sys::Source::L2capCbm`])
    pub(crate) fn on_packet(ctx: &mut LeCtx<'_>, packet: &[u8]) {
        if let Some(opened) = cbm_channel_opened(packet) {
            let Some(m) = ctx.module_mut::<AshaModule>() else { return };
            let cid = opened.local_cid;
            if opened.status != 0 {
                warn!("ASHA: the audio channel did not open (0x{:02x})", opened.status);
                if let Some(ch) = m.channels.remove(&cid) {
                    ch.shared.closed.store(true, Ordering::Release);
                    if let Some(done) = ch.opening {
                        done(false);
                    }
                }
                return;
            }
            let Some(ch) = m.channels.get_mut(&cid) else { return };
            info!("ASHA: device {}: audio channel open (SDUs up to {} bytes)", ch.dev, opened.remote_mtu);
            ch.open = opened.remote_mtu as usize >= SDU_BYTES;
            if !ch.open {
                warn!("ASHA: device {}: the channel takes less than an audio SDU", ch.dev);
            }
            let open = ch.open;
            if let Some(done) = ch.opening.take() {
                done(open);
            }
        } else if let Some(cid) = l2cap_packet_sent(packet) {
            if let Some(ch) = ctx.module_mut::<AshaModule>().and_then(|m| m.channels.get_mut(&cid)) {
                ch.busy = false;
            }
            AshaModule::pump(ctx, cid);
        } else if let Some(cid) = l2cap_channel_closed(packet)
            && let Some(ch) = ctx.module_mut::<AshaModule>().and_then(|m| m.channels.remove(&cid))
        {
            info!("ASHA: device {}: audio channel closed", ch.dev);
            ch.shared.closed.store(true, Ordering::Release);
            if let Some(done) = ch.opening {
                done(false);
            }
        }
    }

    fn drop_channels(&mut self, of: Option<LeDeviceId>) {
        let cids: Vec<u16> =
            self.channels.iter().filter(|(_, ch)| of.is_none_or(|d| ch.dev == d)).map(|(cid, _)| *cid).collect();
        for cid in cids {
            if let Some(ch) = self.channels.remove(&cid) {
                ch.shared.closed.store(true, Ordering::Release);
                if let Some(done) = ch.opening {
                    done(false);
                }
            }
        }
    }
}

impl LeModule for AshaModule {
    fn name(&self) -> &'static str {
        "ASHA"
    }

    fn on_disconnected(&mut self, _ctx: &mut LeCtx<'_>, dev: LeDeviceId, _reason: u8) {
        self.drop_channels(Some(dev));
    }

    fn on_stack_deinit(&mut self) {
        self.drop_channels(None);
    }
}

pub fn create(setup: &mut LeModuleSetup) -> LeModuleEntry {
    LeModuleEntry {
        module: Box::new(AshaModule { poster: setup.poster.clone(), channels: BTreeMap::new() }),
        hooks: None,
    }
}

impl BtHost {
    /// Opens the audio channel of a connected hearing aid on `psm` (its
    /// LE_PSM_OUT) and asks for the 20 ms connection interval ASHA streams
    /// with. Err: why, for the log.
    pub fn asha_open(&self, device: LeDeviceId, psm: u16, timeout: Duration) -> Result<AshaSink, String> {
        let out = Arc::new(Mutex::new(None::<Result<AshaSink, String>>));
        let o = out.clone();
        let ok = self.le_call(timeout, move |ctx, done| {
            let set = |r| *o.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
            let Some(con) = ctx.device(device).filter(|d| d.connected()).map(|d| d.acl) else {
                set(Err("not connected".into()));
                return done(false);
            };
            let status = ctx.stack().le_update_connection(con, INTERVAL_20_MS, LATENCY, SUPERVISION_TIMEOUT);
            if status != 0 {
                debug!("ASHA: device {device}: no connection update (0x{status:02x})");
            }
            let cid = match ctx.stack().cbm_connect(con, psm) {
                Ok(cid) => cid,
                Err(status) => {
                    set(Err(format!("the audio channel could not be opened (0x{status:02x})")));
                    return done(false);
                }
            };
            let Some(m) = ctx.module_mut::<AshaModule>() else {
                set(Err("ASHA is not running".into()));
                return done(false);
            };
            let shared = Arc::new(Shared::default());
            set(Ok(AshaSink { cid, shared: shared.clone(), poster: m.poster.clone() }));
            m.channels.insert(cid, Channel { dev: device, opening: Some(done), open: false, busy: false, shared });
        });
        let result = out.lock().unwrap_or_else(|e| e.into_inner()).take();
        match (ok, result) {
            (true, Some(Ok(sink))) => Ok(sink),
            (_, Some(Err(e))) => Err(e),
            (_, Some(Ok(sink))) => {
                self.asha_close(sink);
                Err("the hearing aid did not open the audio channel".into())
            }
            (_, None) => Err("no answer (timeout)".into()),
        }
    }

    /// Closes an audio channel; SDUs still waiting are dropped
    pub fn asha_close(&self, sink: AshaSink) {
        let cid = sink.cid;
        sink.shared.closed.store(true, Ordering::Release);
        self.le_run(Duration::from_secs(3), move |ctx| {
            let known = ctx.module_mut::<AshaModule>().is_some_and(|m| m.channels.contains_key(&cid));
            if known {
                let status = ctx.stack().cbm_disconnect(cid);
                debug!("ASHA: closing channel 0x{cid:04x}: 0x{status:02x}");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sink() -> AshaSink {
        AshaSink { cid: 0x40, shared: Arc::new(Shared::default()), poster: LePoster::detached() }
    }

    #[test]
    fn uuids_are_in_printed_order() {
        assert_eq!(UUID_LE_PSM_OUT[..4], [0x2d, 0x41, 0x03, 0x39]);
        assert_eq!(UUID_LE_PSM_OUT[12..], [0x1d, 0xf8, 0xcc, 0x1a]);
    }

    #[test]
    fn the_oldest_sdu_is_dropped_when_the_queue_is_full() {
        let s = sink();
        for n in 0..(MAX_QUEUED as u8 + 2) {
            s.push(vec![n; SDU_BYTES]);
        }
        assert_eq!(s.dropped(), 2);
        let frames = s.shared.frames.lock().unwrap();
        assert_eq!(frames.len(), MAX_QUEUED);
        assert_eq!(frames.front().unwrap()[0], 2);
    }

    #[test]
    fn one_pump_is_posted_for_many_pushes() {
        let s = sink();
        s.push(vec![0; SDU_BYTES]);
        assert!(s.shared.kicked.load(Ordering::Acquire));
        s.push(vec![1; SDU_BYTES]);
        assert_eq!(s.sent(), 0);
        assert!(!s.closed());
    }
}
