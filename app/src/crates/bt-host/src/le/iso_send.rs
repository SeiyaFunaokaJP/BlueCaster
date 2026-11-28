//! ISO SDU sending on the BTstack thread (v1 Stream's sending half,
//! le_audio_stream.cpp).
//!
//! Unicast (CIS) sending is paced by the SDU interval on the run loop's
//! timer ([`TIMER_ISO`]). Each interval the sender takes the next SDU set
//! from the shared jitter buffer ([`le_audio::sdu_queue::SduQueue`],
//! silence on an underrun) and hands every CIS its SDU with that interval's
//! sequence number. Each CIS then sends on its own: an SDU goes to the
//! controller as one HCI ISO data packet (complete SDU, never fragmented:
//! plan_stream only takes SDUs that fit; no time stamp) while that CIS has
//! fewer than `depth` SDUs in the controller, counted from our sends and
//! HCI Number Of Completed Packets. The sender runs [`LEAD_SETS`] intervals
//! ahead of real time, so a late timer does not leave the controller
//! without an SDU.
//!
//! BTstack's CIS can-send-now is not used for sending: it grants the CIS of
//! a CIG in lockstep and stops at the first CIS whose queue is full, so a
//! CIS whose completions came late (a flush timeout of 10 ISO intervals) or
//! never held up every other CIS too. A CIS whose controller queue is full
//! keeps its SDUs waiting and sends them as soon as completions make room.
//! An SDU is dropped ("late") only once it can no longer be played in time:
//! its interval plus the wait budget, the CIG's max transport latency (at
//! least [`MIN_WAIT_BUDGET_MS`]), has passed. That also bounds the backlog
//! of a CIS that never completes. Sequence numbers count SDU intervals, so a
//! dropped SDU leaves a gap in the numbers (Core Vol 6 Part G 2: the
//! controller uses them to place SDUs in their intervals).
//!
//! BTstack has one outgoing HCI packet buffer for ACL and ISO. While it is
//! taken (an ISO packet or a GATT write still on its way to the controller)
//! a send is refused with BTSTACK_ACL_BUFFERS_FULL; the same SDU goes again,
//! with the same sequence number, on HCI_EVENT_TRANSPORT_PACKET_SENT (or the
//! next completion or interval).
//!
//! Depth per CIS: the CIS's share of the controller's ISO buffers
//! ([`cis_queue_share`]), within what the flush timeout of LE CIS
//! Established says the controller may hold ([`depth_for_flush`]).
//! MediaTek controllers keep [`MEDIATEK_QUEUE_DEPTH`] or the Developer >
//! Debug setting.
//!
//! Alignment: SDUs go without a time stamp, so the controller puts each
//! one in the next event of its CIS. When one CIS's first SDUs (or those
//! after an underrun) reach the controller just after its event while the
//! others' make theirs, that CIS plays a frame later than the others for
//! the rest of the stream: the sound of a set of earbuds moves to the other
//! side. Every [`ALIGN_WINDOW_MS`] the low completion delay (the
//! [`ALIGN_PERCENTILE`]th percentile: SDUs through at the first try, so
//! retransmissions, which move the median by up to an interval, do not
//! count) of each CIS is compared. In line, a CIS completes later than
//! another by the difference of their places in the CIG event
//! (CIG_Sync_Delay - CIS_Sync_Delay of LE CIS Established; seen on a
//! WF-1000XM6: 3.1 ms, and the CIS that comes first a frame behind
//! completing 7 ms after the other), so each low is taken from its place
//! first. A CIS then more than [`ALIGN_THRESHOLD_PCT`] % of a frame later
//! than the earliest one in [`ALIGN_CONFIRM`] windows in a row gets one SDU
//! left out for every frame it is behind (it holds that many more in the
//! controller than the others; [`ALIGN_FRAMES_MAX`] at most), which brings
//! it back in line. Completions less than the rest of a frame apart are
//! left alone whatever the places say: a controller that reports all CIS
//! together shows no gap while in line. Developer > Debug turns the
//! realignment off. A skew only comes from a start or
//! an underrun, so only those arm the check: the first window after one is
//! left out (the stream settling), and a realignment disarms it until the
//! next. At most [`ALIGN_MAX`] times a stream.
//!
//! Health: counters are published about once a second
//! ([`BtHost::le_stream_stats`](crate::BtHost)); every 10 s an "LE stream
//! health" line per CIS (SDUs per second, late SDUs, completion delay,
//! queue peak). A CIS without a completion for [`STALL_MS`] while SDUs wait
//! in the controller is stalled; when every CIS is, the stream is reported
//! lost ([`IsoSender::take_stall`]), else the others play on and
//! `LeStreamStats::cis_stalled` says how many do not.
//!
//! A broadcast (BIG) is sent in rounds on HCI_EVENT_BIS_CAN_SEND_NOW, which
//! BTstack gives for every BIS of the BIG in BIS order once all of them
//! have room. Several BIGs (broadcasts of their own, e.g. left and right)
//! share one round: each BIG is asked, and the round ends once every BIS
//! of all of them got its SDU, so all carry the same set and sequence
//! number. A BIS send refused while BTstack's packet buffer is taken is
//! neither sent nor failed: the same SDU goes again with the same sequence
//! number on HCI_EVENT_TRANSPORT_PACKET_SENT (or 1 ms later), and the round
//! waits for it. The [`le_audio::sdu_queue::SendGuard`] keeps those rounds
//! at most queue + 2 intervals ahead of real time. A stop in the middle of
//! a round lets that round finish first ([`IsoSender::finish_round`], for
//! [`FINISH_ROUND_INTERVALS`] at most), so every BIS ends on the same SDU.
//!
//! BTstack (1.8.3) refuses the CIS can-send-now request with Command
//! Disallowed until the CIG is active, which is only after the ISO data
//! paths are set up; the stream can start before that (the ASE reaches
//! Streaming in the same moment as the CIS), so the sender asks again every
//! 1 ms and starts once it is accepted.
//!
//! No allocation per interval: a set goes back to the queue's pool
//! ([`le_audio::sdu_queue::SduQueue::recycle`]) once its SDUs are copied to
//! the CIS, whose SDU buffers are reused.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use btstack_sys::consts::{
    BTSTACK_ACL_BUFFERS_FULL, ERROR_CODE_COMMAND_DISALLOWED, ERROR_CODE_SUCCESS, HCI_EVENT_NUMBER_OF_COMPLETED_PACKETS,
    HCI_EVENT_TRANSPORT_PACKET_SENT,
};
use btstack_sys::events::{self, LeCisEstablished};
use btstack_sys::{HighTimerResolution, Stack};
use le_audio::sdu_queue::{QueueStats, SduSet, SendCounters, SendGuard};

use super::LeStreamStats;
use super::core::SharedSduQueue;

/// SDUs queued per BIS in the controller (at most), and the jitter
/// buffer's prefill base
pub const QUEUE_DEPTH: u8 = 2;

/// SDUs queued per CIS at most (within the controller's ISO buffers)
pub const MAX_CIS_QUEUE_DEPTH: u8 = 16;

/// SDUs queued per CIS at least when the buffers allow: the lead, the one
/// on air and one more
const MIN_CIS_QUEUE_DEPTH: u8 = 4;

/// SDU sets the CIS sender runs ahead of real time
pub const LEAD_SETS: u64 = 2;

/// Intervals a stopping broadcast waits at most for the rest of its round:
/// each BIS gets room once an interval, so one is enough unless the
/// controller is held up
const FINISH_ROUND_INTERVALS: u32 = 2;

/// The least time an SDU may wait for room in the controller past its
/// interval: a few intervals of completions held up (a busy controller, a
/// GATT exchange) must not cost audio
pub const MIN_WAIT_BUDGET_MS: u32 = 40;

/// The wait budget when the CIG's max transport latency is not known
const DEFAULT_WAIT_BUDGET_MS: u32 = 100;

/// The most time an SDU may wait past its interval
const MAX_WAIT_BUDGET_MS: u32 = 200;

/// How long an SDU may wait for room in the controller past its interval:
/// the CIG's max transport latency (the time the stream allows for an SDU
/// to get through), within [`MIN_WAIT_BUDGET_MS`]..[`MAX_WAIT_BUDGET_MS`]
pub(crate) fn wait_budget_ms(max_transport_latency_ms: u16) -> u32 {
    if max_transport_latency_ms == 0 {
        return DEFAULT_WAIT_BUDGET_MS;
    }
    u32::from(max_transport_latency_ms).clamp(MIN_WAIT_BUDGET_MS, MAX_WAIT_BUDGET_MS)
}

/// SDUs a CIS may keep waiting: those of the lead and the wait budget, and
/// the one of the interval going on
fn pending_max(budget_ms: u32, frame_us: u32) -> usize {
    (budget_ms * 1000).div_ceil(frame_us.max(1)) as usize + LEAD_SETS as usize + 1
}

/// No SDU completed on a CIS for this long while SDUs wait in the
/// controller: the CIS is stalled
pub const STALL_MS: u32 = 2000;

/// Counters published this often
const STATS_MS: u32 = 1000;

/// The "LE stream health" line this often
const HEALTH_MS: u32 = 10_000;

/// Completion delays kept per CIS for one health line
const DELAY_SAMPLES_MAX: usize = 4096;

/// The window over which the CIS's completion delays are compared
const ALIGN_WINDOW_MS: u32 = 2000;
/// Completions a CIS needs in a window to be compared
const ALIGN_MIN_SAMPLES: usize = 50;
/// The percentile of a window's completion delays compared
const ALIGN_PERCENTILE: usize = 10;
/// A CIS whose low completion delay is this share of a frame (%) later
/// than the earliest CIS's is a frame behind
const ALIGN_THRESHOLD_PCT: u32 = 80;
/// Windows in a row a CIS must be behind in
const ALIGN_CONFIRM: u8 = 3;
/// Realignments a stream gets at most
const ALIGN_MAX: u32 = 4;
/// SDUs one realignment leaves out at most (frames a CIS is behind)
const ALIGN_FRAMES_MAX: u8 = 3;

/// BIG rounds are counted over windows this long
const RATE_WINDOW_MS: u32 = 1000;
/// A window with fewer rounds than this share of its intervals (%) is slow
const RATE_SLOW_PCT: u64 = 75;
/// Slow windows in a row that count as slow sending (reported once)
const RATE_SLOW_WINDOWS: u8 = 5;

/// LE CIS Established parameters kept (the latest per handle)
const LINKS_MAX: usize = 8;

/// The run loop timer of the sender
pub(crate) const TIMER_ISO: u32 = 4;

/// A CIS request refused while the CIG is not active yet is asked again
/// after this many ms
const CIG_NOT_ACTIVE_RETRY_MS: u32 = 1;

/// SDUs queued per CIS on a MediaTek controller (issue #86, measured on an
/// MT7925: with 2 it completed the first three ISO packets and then none;
/// with 12 it played; hci_transport's iso_intr has the rest)
pub const MEDIATEK_QUEUE_DEPTH: u8 = 12;

/// Developer > Debug: SDUs queued per CIS on a MediaTek controller instead
/// of [`MEDIATEK_QUEUE_DEPTH`]; 0: that default
static DEBUG_DEPTH: AtomicU8 = AtomicU8::new(0);
/// Whether the open controller is MediaTek's (set after HCI init)
static MEDIATEK: AtomicBool = AtomicBool::new(false);

/// The setting, for the next streams; 0: the default
pub fn set_debug_depth(depth: u8) {
    DEBUG_DEPTH.store(depth, Ordering::Relaxed);
}

/// After HCI init: whether the controller is MediaTek's
pub(crate) fn depth_for_controller(mediatek: bool) {
    MEDIATEK.store(mediatek, Ordering::Relaxed);
    let depth = DEBUG_DEPTH.load(Ordering::Relaxed);
    if mediatek && depth > 0 {
        tracing::info!("LE Audio: Developer > Debug: up to {depth} SDUs queued per CIS");
    }
}

fn depth_limit() -> u8 {
    depth_limit_for(MEDIATEK.load(Ordering::Relaxed), DEBUG_DEPTH.load(Ordering::Relaxed))
}

fn depth_limit_for(mediatek: bool, debug: u8) -> u8 {
    match (mediatek, debug) {
        (false, _) => QUEUE_DEPTH,
        (true, 0) => MEDIATEK_QUEUE_DEPTH,
        (true, d) => d,
    }
}

/// SDUs queued per BIS (and the jitter buffer's prefill base):
/// [`QUEUE_DEPTH`] ([`MEDIATEK_QUEUE_DEPTH`] or the Debug setting on
/// MediaTek's), fewer when the controller has fewer ISO buffers than that
/// for every stream (queue x streams must stay within
/// Total_Num_ISO_Data_Packets, which Number Of Completed Packets frees
/// again); 0: not even one per stream
pub(crate) fn queue_depth(iso_packets: u8, cis: usize) -> u8 {
    queue_depth_within(iso_packets, cis, depth_limit())
}

/// The most SDUs one of `cis` CIS may have queued in the controller: its
/// share of the ISO buffers, at most [`MAX_CIS_QUEUE_DEPTH`] (on MediaTek's:
/// [`MEDIATEK_QUEUE_DEPTH`] or the Debug setting); 0: not even one per CIS
pub(crate) fn cis_queue_share(iso_packets: u8, cis: usize) -> u8 {
    let limit = if MEDIATEK.load(Ordering::Relaxed) { depth_limit() } else { MAX_CIS_QUEUE_DEPTH };
    queue_depth_within(iso_packets, cis, limit)
}

fn queue_depth_within(iso_packets: u8, cis: usize, limit: u8) -> u8 {
    if cis == 0 {
        return 0;
    }
    (iso_packets as usize / cis).min(limit as usize) as u8
}

/// SDUs a CIS needs queued with this flush timeout: the controller may
/// hold an SDU until FT ISO intervals after its first event (a PDU the
/// peer does not acknowledge completes only then), plus the lead, the one
/// on air and two for timer and completion jitter; within
/// [`MIN_CIS_QUEUE_DEPTH`]..[`MAX_CIS_QUEUE_DEPTH`]
pub(crate) fn depth_for_flush(ft: u8, iso_interval_us: u32, sdu_interval_us: u32) -> u8 {
    let held = (u32::from(ft) * iso_interval_us).div_ceil(sdu_interval_us.max(1));
    (held + LEAD_SETS as u32 + 3).clamp(u32::from(MIN_CIS_QUEUE_DEPTH), u32::from(MAX_CIS_QUEUE_DEPTH)) as u8
}

/// The depth a CIS stream uses: the share of the buffers, within the need
/// of the flush timeout when known (MediaTek's: the share as it is)
fn cis_depth(share: u8, mediatek: bool, flush_need: Option<u8>) -> u8 {
    match flush_need {
        Some(need) if !mediatek => share.min(need),
        _ => share,
    }
}

fn phy_name(phy: u8) -> &'static str {
    match phy {
        1 => "1M",
        2 => "2M",
        3 => "Coded",
        _ => "?",
    }
}

/// What the sender needs of BTstack (the Stack; a fake in the tests)
pub(crate) trait IsoOut {
    fn time_ms(&self) -> u32;
    fn set_queue(&self, packets: u8);
    /// CIS can-send-now for every CIS of the CIG of `cis`: BTstack's status
    /// (Command Disallowed while the CIG is not active)
    fn request_can_send(&self, cis: u16) -> u8;
    /// BIS can-send-now for every BIS of the BIG
    fn request_bis_can_send(&self, _big: u8) {}
    fn send(&self, cis: u16, sequence_number: u16, sdu: &[u8]) -> u8;
    fn timer_start(&self, ms: u32);
    fn timer_stop(&self);
}

impl IsoOut for Stack {
    fn time_ms(&self) -> u32 {
        Stack::time_ms(self)
    }

    fn set_queue(&self, packets: u8) {
        self.set_num_iso_packets_to_queue(packets);
    }

    fn request_can_send(&self, cis: u16) -> u8 {
        self.request_cis_can_send_now(cis)
    }

    fn request_bis_can_send(&self, big: u8) {
        self.request_bis_can_send_now(big);
    }

    fn send(&self, cis: u16, sequence_number: u16, sdu: &[u8]) -> u8 {
        self.iso_send(cis, sequence_number, sdu)
    }

    fn timer_start(&self, ms: u32) {
        Stack::timer_start(self, TIMER_ISO, ms);
    }

    fn timer_stop(&self) {
        Stack::timer_stop(self, TIMER_ISO);
    }
}

/// An SDU waiting for room in the controller
struct Pending {
    seq: u16,
    sdu: Vec<u8>,
    /// Run loop time after which it can no longer be played in time
    deadline_ms: u32,
}

/// Whether run loop time `t` is past `deadline` (wrapping)
fn past(t: u32, deadline: u32) -> bool {
    (t.wrapping_sub(deadline) as i32) > 0
}

/// One CIS of the running stream
struct CisOut {
    handle: u16,
    /// Not reported down
    alive: bool,
    pending: VecDeque<Pending>,
    /// SDU buffers to reuse
    free: Vec<Vec<u8>>,
    /// Run loop time of each SDU in the controller (oldest first)
    in_flight: VecDeque<u32>,
    /// Last completion, or the send that found the controller queue empty
    progress_ms: u32,
    stalled: bool,
    sent: u64,
    completed: u64,
    /// Dropped without being sent (late, or the send failed)
    late: u64,
    /// For the health line: since the last one
    window_sent: u64,
    window_late: u64,
    window_peak: usize,
    delays_ms: Vec<u32>,
    /// Completion delays of the alignment window
    align_ms: Vec<u32>,
    /// The next SDUs left out to bring the CIS back in line
    skip: u8,
}

impl CisOut {
    fn new(handle: u16, now: u32) -> CisOut {
        CisOut {
            handle,
            alive: true,
            pending: VecDeque::with_capacity(16),
            free: Vec::with_capacity(16),
            in_flight: VecDeque::with_capacity(usize::from(MAX_CIS_QUEUE_DEPTH)),
            progress_ms: now,
            stalled: false,
            sent: 0,
            completed: 0,
            late: 0,
            window_sent: 0,
            window_late: 0,
            window_peak: 0,
            delays_ms: Vec::with_capacity(256),
            align_ms: Vec::with_capacity(512),
            skip: 0,
        }
    }

    /// The SDU of the next interval: waits for room in the controller
    /// until `deadline_ms`; at most `max` wait (the oldest dropped beyond)
    fn give(&mut self, seq: u16, sdu: &[u8], deadline_ms: u32, now: u32, max: usize) {
        self.drop_expired(now);
        if self.pending.len() >= max.max(1) {
            self.drop_oldest();
        }
        let mut buf = self.free.pop().unwrap_or_default();
        buf.clear();
        buf.extend_from_slice(sdu);
        self.pending.push_back(Pending { seq, sdu: buf, deadline_ms });
    }

    /// Drops the waiting SDUs that can no longer be played in time
    fn drop_expired(&mut self, now: u32) {
        while self.pending.front().is_some_and(|p| past(now, p.deadline_ms)) {
            self.drop_oldest();
        }
    }

    fn drop_oldest(&mut self) {
        if let Some(old) = self.pending.pop_front() {
            self.late += 1;
            self.window_late += 1;
            self.free.push(old.sdu);
        }
    }

    /// Number Of Completed Packets: `n` of its SDUs left the controller
    fn on_completed(&mut self, n: u16, now: u32) {
        for _ in 0..n {
            let Some(at) = self.in_flight.pop_front() else { break };
            let delay = now.wrapping_sub(at);
            if self.delays_ms.len() < DELAY_SAMPLES_MAX {
                self.delays_ms.push(delay);
            }
            if self.align_ms.len() < DELAY_SAMPLES_MAX {
                self.align_ms.push(delay);
            }
            self.completed += 1;
        }
        self.progress_ms = now;
        if self.stalled {
            self.stalled = false;
            tracing::info!("LE Audio: CIS 0x{:04X}: SDUs complete again", self.handle);
        }
    }

    /// Time since the last progress; 0 when that is later than `now` (a
    /// clock read before it), not a wrapped 49 days
    fn idle_ms(&self, now: u32) -> u32 {
        let idle = now.wrapping_sub(self.progress_ms);
        if idle > i32::MAX as u32 { 0 } else { idle }
    }

    /// The low completion delay of the alignment window (None with too few
    /// completions); starts the next window
    fn take_align_low(&mut self) -> Option<u32> {
        let mut d = std::mem::take(&mut self.align_ms);
        let low = (d.len() >= ALIGN_MIN_SAMPLES).then(|| {
            d.sort_unstable();
            d[d.len() * ALIGN_PERCENTILE / 100]
        });
        d.clear();
        self.align_ms = d;
        low
    }

    fn down(&mut self) {
        self.alive = false;
        self.stalled = false;
        self.skip = 0;
        self.align_ms.clear();
        self.in_flight.clear();
        while let Some(p) = self.pending.pop_front() {
            self.free.push(p.sdu);
        }
    }

    /// "CIS 0x0060: 100.0 SDUs/s, 0 late, completion p50 12 ms p90 20 ms,
    /// queue peak 3 of 10"; starts the next window
    fn health(&mut self, secs: f64, depth: u8) -> String {
        let delays = if self.delays_ms.is_empty() {
            "no completion".to_string()
        } else {
            self.delays_ms.sort_unstable();
            let at = |pct: usize| self.delays_ms[(self.delays_ms.len() * pct / 100).min(self.delays_ms.len() - 1)];
            format!("completion p50 {} ms p90 {} ms", at(50), at(90))
        };
        let line = format!(
            "CIS 0x{:04X}{}: {:.1} SDUs/s, {} late, {delays}, queue peak {} of {depth}",
            self.handle,
            if !self.alive {
                " (down)"
            } else if self.stalled {
                " (stalled)"
            } else {
                ""
            },
            self.window_sent as f64 / secs.max(0.001),
            self.window_late,
            self.window_peak
        );
        self.window_sent = 0;
        self.window_late = 0;
        self.window_peak = self.in_flight.len();
        self.delays_ms.clear();
        line
    }
}

/// The sender of the running stream
#[derive(Default)]
pub struct IsoSender {
    sending: bool,
    /// 1 ms system timer resolution while sending (as v1), so that the
    /// timer and time_ms() don't move in 15.6 ms steps
    timer_resolution: Option<HighTimerResolution>,
    /// Sending to BIGs (broadcasts): each handle with the index of its
    /// first BIS in `cis_handles`; empty: a CIG
    bigs: Vec<(u8, usize)>,
    queue: Option<SharedSduQueue>,
    frame_us: u32,
    /// Run loop time of the first interval (CIS: once the CIG is active)
    t0_ms: Option<u32>,
    /// sets_sent, sdus_sent, send_errors (the rest is the queue's)
    counters: SendCounters,
    /// Bytes of the SDUs sent
    bytes_sent: u64,
    queue_stats: QueueStats,
    /// The jitter buffer's prefill (sets)
    prefill: usize,
    /// CIS requests asked again because the CIG was not active yet
    cig_retries: u32,
    /// Published for BtHost::le_stream_stats
    stats_out: Arc<Mutex<LeStreamStats>>,

    // ---- CIS ----
    cis: Vec<CisOut>,
    /// SDUs queued per CIS in the controller at most
    depth: u8,
    /// How long an SDU may wait for room past its interval
    budget_ms: u32,
    /// SDUs a CIS may keep waiting at most
    pending_max: usize,
    /// The CIG is active: pacing runs
    active: bool,
    /// SDU sets taken so far (the next one's sequence number)
    produced: u64,
    /// Intervals skipped after the run loop was held up
    skipped: u64,
    /// Sends refused while BTstack's packet buffer was taken (sent again)
    busy: u64,
    /// The CIS that sends first next time
    next_cis: usize,
    last_stats_ms: u32,
    last_health_ms: u32,
    /// Every CIS stalled: why, until taken
    stall: Option<String>,
    stall_reported: bool,
    /// Alignment: the last window's end, the CIS behind and in how many
    /// windows in a row, whether a start or an underrun armed the check,
    /// the windows still left out, the realignments of this stream
    last_align_ms: u32,
    align_suspect: Option<(u16, u8)>,
    align_armed: bool,
    align_settle: u8,
    realigns: u32,
    /// LE CIS Established of the recent CIS (any stream)
    links: Vec<LeCisEstablished>,

    // ---- BIG (rounds) ----
    /// The BIS handles in plan order
    cis_handles: Vec<u16>,
    seq: Vec<u16>,
    /// Per BIS: has its SDU of the current round
    served: Vec<bool>,
    /// BIS whose send of the current round was refused while BTstack's
    /// packet buffer was taken: sent again, same SDU and sequence number
    bis_retry: Vec<usize>,
    round_open: bool,
    /// Stopping: no further round
    stopping: bool,
    /// Stopping: the round going on finishes first, until this run loop
    /// time at most
    finish_by_ms: Option<u32>,
    /// That round is out (or its time ran out): the stop goes on, until
    /// taken
    round_finished: bool,
    current: SduSet,
    guard: SendGuard,
    /// Send guard waits: how many, the run loop time the current one is
    /// due, and how late the timer fired at most (ms)
    guard_waits: u64,
    guard_due_ms: Option<u32>,
    guard_late_ms: u32,
    /// Round rate: the current window's start and its rounds, the slow
    /// windows in a row, slow sending (why, until taken) and whether it
    /// was reported for this start
    rate_start_ms: Option<u32>,
    rate_rounds: u64,
    slow_windows: u8,
    slow: Option<String>,
    slow_reported: bool,
}

impl IsoSender {
    pub(crate) fn new(stats_out: Arc<Mutex<LeStreamStats>>) -> IsoSender {
        IsoSender { stats_out, ..IsoSender::default() }
    }

    /// Starts sending on these CIS (plan order) from `queue`, one set per
    /// `frame_us`; `share`: [`cis_queue_share`]; `max_latency_ms`: the
    /// CIG's max transport latency C->P ([`wait_budget_ms`])
    pub(crate) fn start(
        &mut self,
        out: &dyn IsoOut,
        cis_handles: &[u16],
        queue: SharedSduQueue,
        frame_us: u32,
        share: u8,
        max_latency_ms: u16,
    ) {
        self.bigs.clear();
        self.reset(queue, frame_us);
        self.budget_ms = wait_budget_ms(max_latency_ms);
        self.pending_max = pending_max(self.budget_ms, self.frame_us);
        let need = self.flush_need(cis_handles, frame_us);
        let mediatek = MEDIATEK.load(Ordering::Relaxed);
        self.depth = cis_depth(share, mediatek, need).max(1);
        let now = out.time_ms();
        self.cis = cis_handles.iter().map(|&h| CisOut::new(h, now)).collect();
        self.cis_handles = cis_handles.to_vec();
        self.publish();
        out.set_queue(self.depth);
        self.timer_resolution = Some(HighTimerResolution::new());
        self.sending = true;
        tracing::info!("LE Audio: sending ISO SDUs on {} CIS (queue {})", cis_handles.len(), self.depth);
        tracing::debug!(
            "LE Audio: ISO sender: CIS {cis_handles:04X?}, frame {} us, prefill {} set(s), {share} buffer(s) per \
             CIS, flush timeout needs {}, lead {LEAD_SETS} set(s), SDUs wait up to {} ms past their interval              ({} at most)",
            self.frame_us,
            self.prefill,
            need.map_or("?".to_string(), |n| n.to_string()),
            self.budget_ms,
            self.pending_max
        );
        self.activate(out);
    }

    /// As [`IsoSender::start_bigs`] for one BIG
    #[cfg(test)]
    pub(crate) fn start_big(
        &mut self,
        out: &dyn IsoOut,
        big: u8,
        bis_handles: &[u16],
        queue: SharedSduQueue,
        frame_us: u32,
        queue_depth: u8,
    ) {
        self.start_bigs(out, &[(big, bis_handles.to_vec())], queue, frame_us, queue_depth);
    }

    /// Starts sending on the BIS of several BIGs (each in BIS order; the
    /// SDU set has one SDU per BIS in this order), in rounds of BTstack's
    /// BIS can-send-now: each BIG is asked, and the round ends once every
    /// BIS of all of them got its SDU (one set per interval for all)
    pub(crate) fn start_bigs(
        &mut self,
        out: &dyn IsoOut,
        bigs: &[(u8, Vec<u16>)],
        queue: SharedSduQueue,
        frame_us: u32,
        queue_depth: u8,
    ) {
        let mut bis_handles = Vec::new();
        self.bigs.clear();
        for (big, bis) in bigs {
            self.bigs.push((*big, bis_handles.len()));
            bis_handles.extend_from_slice(bis);
        }
        let bis_handles = &bis_handles[..];
        self.reset(queue, frame_us);
        self.cis.clear();
        self.cis_handles = bis_handles.to_vec();
        self.seq = vec![0; bis_handles.len()];
        self.served = vec![false; bis_handles.len()];
        self.bis_retry.clear();
        self.round_open = false;
        self.stopping = false;
        self.finish_by_ms = None;
        self.round_finished = false;
        self.current = SduSet::new();
        self.guard = SendGuard::new(self.frame_us, u64::from(queue_depth) + 2);
        self.guard_waits = 0;
        self.guard_due_ms = None;
        self.guard_late_ms = 0;
        self.rate_start_ms = None;
        self.rate_rounds = 0;
        self.slow_windows = 0;
        self.slow = None;
        self.slow_reported = false;
        self.publish();
        out.set_queue(queue_depth);
        self.timer_resolution = Some(HighTimerResolution::new());
        self.sending = true;
        tracing::info!("LE Audio: sending ISO SDUs on {} BIS (queue {queue_depth})", bis_handles.len());
        tracing::debug!(
            "LE Audio: ISO sender: BIS {bis_handles:04X?}, frame {} us, prefill {} set(s), guard lead {} set(s)",
            self.frame_us,
            self.prefill,
            u64::from(queue_depth) + 2
        );
        self.request_next(out);
    }

    fn reset(&mut self, queue: SharedSduQueue, frame_us: u32) {
        self.prefill = queue.lock().unwrap_or_else(|e| e.into_inner()).prefill();
        self.queue = Some(queue);
        self.frame_us = frame_us.max(1);
        self.t0_ms = None;
        self.counters = SendCounters::default();
        self.bytes_sent = 0;
        self.queue_stats = QueueStats::default();
        self.cig_retries = 0;
        self.active = false;
        self.produced = 0;
        self.skipped = 0;
        self.busy = 0;
        self.next_cis = 0;
        self.stall = None;
        self.stall_reported = false;
        self.align_suspect = None;
        self.align_armed = false;
        self.align_settle = 0;
        self.realigns = 0;
    }

    /// A start or an underrun: a CIS may have fallen a frame behind
    fn arm_alignment(&mut self) {
        self.align_armed = true;
        self.align_settle = 1;
        self.align_suspect = None;
    }

    /// Broadcast stop, first step: a round that only some BIS got goes on
    /// until every BIS has its SDU, for [`FINISH_ROUND_INTERVALS`] at most,
    /// and no further round starts. True while it does: the caller stops
    /// once [`IsoSender::take_round_finished`] says so. False: no round to
    /// finish, the caller stops now.
    pub(crate) fn finish_round(&mut self, out: &dyn IsoOut) -> bool {
        if self.finish_by_ms.is_some() {
            return true;
        }
        if self.stopping || !self.sending || self.bigs.is_empty() || !self.round_open || self.served.iter().all(|&s| s)
        {
            return false;
        }
        self.stopping = true;
        let wait = (FINISH_ROUND_INTERVALS * self.frame_us).div_ceil(1000).max(1);
        self.finish_by_ms = Some(out.time_ms().wrapping_add(wait));
        if self.bis_retry.is_empty() {
            // With a BIS waiting to go again, its 1 ms timer runs already
            out.timer_start(wait);
        }
        tracing::debug!(
            "LE Audio: stopping after the round going on ({} of {} BIS still to send, {wait} ms at most)",
            self.served.iter().filter(|&&s| !s).count(),
            self.served.len()
        );
        true
    }

    /// The round of [`IsoSender::finish_round`] is out or its time ran out:
    /// true once
    pub(crate) fn take_round_finished(&mut self) -> bool {
        std::mem::take(&mut self.round_finished)
    }

    /// Stops sending (logs the totals)
    pub(crate) fn stop(&mut self, out: &dyn IsoOut) {
        self.stopping = false;
        self.finish_by_ms = None;
        self.round_finished = false;
        if !self.sending {
            return;
        }
        self.sending = false;
        self.active = false;
        self.timer_resolution = None;
        out.timer_stop();
        if let Some(q) = &self.queue {
            let mut q = q.lock().unwrap_or_else(|e| e.into_inner());
            self.queue_stats = q.stats();
            self.prefill = q.prefill();
            let current = std::mem::take(&mut self.current);
            if !current.is_empty() {
                q.recycle(current);
            }
        }
        for c in &self.cis {
            tracing::info!(
                "LE Audio: CIS 0x{:04X}: {} SDUs sent, {} completed, {} late, {} in the controller at the end",
                c.handle,
                c.sent,
                c.completed,
                c.late,
                c.in_flight.len()
            );
        }
        for c in &mut self.cis {
            c.down();
        }
        self.publish();
        tracing::info!("{}", self.send_counters().stopped_message());
        if self.busy > 0 || self.skipped > 0 {
            tracing::debug!(
                "LE Audio: {} send(s) waited for BTstack's packet buffer, {} interval(s) skipped",
                self.busy,
                self.skipped
            );
        }
        if self.guard_waits > 0 {
            // The run loop's timer follows the system timer resolution
            // (v1 raised it to 1 ms while sending)
            tracing::info!(
                "LE Audio: send guard waited {} time(s), its timer at most {} ms late",
                self.guard_waits,
                self.guard_late_ms
            );
        }
        if self.cig_retries > 0 {
            tracing::info!("LE Audio: asked {} time(s) again for the CIG to become active", self.cig_retries);
        }
    }

    /// Every CIS stalled (no SDU completed for [`STALL_MS`]): why, once;
    /// the caller reports the stream lost
    pub(crate) fn take_stall(&mut self) -> Option<String> {
        self.stall.take()
    }

    /// BIG rounds well below the frame rate for [`RATE_SLOW_WINDOWS`]
    /// windows in a row (the controller takes only some of the SDUs, the
    /// jitter buffer drops the rest): why, once per start
    pub(crate) fn take_slow(&mut self) -> Option<String> {
        self.slow.take()
    }

    /// A round started at `now`: counted in the rate window
    fn count_round(&mut self, now: u32) {
        let Some(start) = self.rate_start_ms else {
            self.rate_start_ms = Some(now);
            self.rate_rounds = 0;
            return;
        };
        self.rate_rounds += 1;
        let span = now.wrapping_sub(start);
        if span < RATE_WINDOW_MS {
            return;
        }
        let intervals = u64::from(span) * 1000 / u64::from(self.frame_us.max(1));
        let pct = self.rate_rounds * 100 / intervals.max(1);
        if pct < RATE_SLOW_PCT {
            self.slow_windows = self.slow_windows.saturating_add(1);
            tracing::debug!(
                "LE Audio: {} BIG round(s) in {span} ms ({pct} % of the intervals), {} slow window(s) in a row",
                self.rate_rounds,
                self.slow_windows
            );
        } else {
            self.slow_windows = 0;
        }
        self.rate_start_ms = Some(now);
        self.rate_rounds = 0;
        if self.slow_windows >= RATE_SLOW_WINDOWS && !self.slow_reported && !self.stopping {
            self.slow_reported = true;
            let why = format!(
                "BIG rounds at {pct} % of the frame rate for {} s",
                u32::from(self.slow_windows) * RATE_WINDOW_MS / 1000
            );
            tracing::warn!("LE Audio: ISO sending slow: {why}");
            self.slow = Some(why);
        }
    }

    /// HCI events (all of them): LE CIS Established (logged and kept for
    /// the depth), Number Of Completed Packets and the transport's packet
    /// sent (room again: send)
    pub(crate) fn on_hci_event(&mut self, out: &dyn IsoOut, packet: &[u8]) {
        if let Some(e) = events::le_cis_established(packet) {
            self.on_cis_established(e);
            return;
        }
        if self.sending && !self.bigs.is_empty() {
            // A BIS send refused while the packet buffer was taken
            if packet.first() == Some(&HCI_EVENT_TRANSPORT_PACKET_SENT) && !self.bis_retry.is_empty() {
                self.retry_bis(out);
            }
            return;
        }
        if !self.sending || !self.active {
            return;
        }
        match packet.first() {
            Some(&HCI_EVENT_NUMBER_OF_COMPLETED_PACKETS) => {
                let now = out.time_ms();
                let mut ours = false;
                for (handle, n) in events::completed_packets(packet) {
                    if let Some(c) = self.cis.iter_mut().find(|c| c.handle == handle && c.alive) {
                        c.on_completed(n, now);
                        ours = true;
                    }
                }
                if ours {
                    self.flush(out);
                }
            }
            Some(&HCI_EVENT_TRANSPORT_PACKET_SENT) => self.flush(out),
            _ => {}
        }
    }

    fn on_cis_established(&mut self, e: LeCisEstablished) {
        if e.status != ERROR_CODE_SUCCESS {
            return;
        }
        tracing::info!(
            "LE Audio: CIS 0x{:04X} parameters: NSE {}, BN {} / {}, FT {} / {}, max PDU {} / {}, ISO interval \
             {:.2} ms, transport latency {:.1} / {:.1} ms, PHY {} / {} (C->P / P->C)",
            e.cis_handle,
            e.nse,
            e.bn_c_to_p,
            e.bn_p_to_c,
            e.ft_c_to_p,
            e.ft_p_to_c,
            e.max_pdu_c_to_p,
            e.max_pdu_p_to_c,
            f64::from(e.iso_interval) * 1.25,
            f64::from(e.transport_latency_c_to_p) / 1000.0,
            f64::from(e.transport_latency_p_to_c) / 1000.0,
            phy_name(e.phy_c_to_p),
            phy_name(e.phy_p_to_c)
        );
        self.links.retain(|l| l.cis_handle != e.cis_handle);
        if self.links.len() >= LINKS_MAX {
            self.links.remove(0);
        }
        self.links.push(e);
    }

    /// The most SDUs the flush timeouts of these CIS need queued; None when
    /// none of them is known
    fn flush_need(&self, handles: &[u16], frame_us: u32) -> Option<u8> {
        handles
            .iter()
            .filter_map(|h| self.links.iter().find(|l| l.cis_handle == *h))
            .filter(|l| l.bn_c_to_p > 0 && l.iso_interval > 0)
            .map(|l| depth_for_flush(l.ft_c_to_p, u32::from(l.iso_interval) * 1250, frame_us))
            .max()
    }

    /// HCI_EVENT_BIS_CAN_SEND_NOW: the last BIS of all BIGs completes the
    /// round
    pub(crate) fn on_bis_can_send_now(&mut self, out: &dyn IsoOut, packet: &[u8]) {
        if !self.sending {
            return;
        }
        let Some(ev) = events::bis_can_send_now(packet) else { return };
        let Some(k) = self.bigs.iter().position(|&(big, _)| big == ev.big_handle) else { return };
        // By index: BTstack's event lacks the handle
        let first = self.bigs[k].1;
        let end = self.bigs.get(k + 1).map_or(self.cis_handles.len(), |&(_, next)| next);
        let index = first + usize::from(ev.bis_index);
        if index >= end {
            return;
        }
        let handle = self.cis_handles[index];
        self.on_bis_can_send(out, handle);
    }

    fn on_bis_can_send(&mut self, out: &dyn IsoOut, handle: u16) {
        let Some(index) = self.cis_handles.iter().position(|&h| h == handle) else { return };
        if !self.round_open || self.served[index] {
            if self.stopping {
                // Stopping: no further round
                return;
            }
            // First BIS of a round: the next frame interval for all BIS
            self.start_round();
            self.count_round(out.time_ms());
        }
        self.bis_retry.retain(|&i| i != index);
        if !self.send_bis(out, index) {
            // Refused while BTstack's packet buffer is taken: again once it
            // is free (transport packet sent; the timer if that is missed)
            self.bis_retry.push(index);
            out.timer_start(1);
            return;
        }
        self.end_round_if_done(out);
    }

    /// Sends the BIS its SDU of the current round; false: refused while
    /// BTstack's packet buffer was taken (nothing sent, nothing counted:
    /// the same SDU goes again with the same sequence number)
    fn send_bis(&mut self, out: &dyn IsoOut, index: usize) -> bool {
        let handle = self.cis_handles[index];
        if self.current.len() == self.cis_handles.len() {
            let status = out.send(handle, self.seq[index], &self.current[index]);
            tracing::trace!(
                "LE Audio: ISO SDU on 0x{:04X}: seq {}, {} bytes, status 0x{status:02X}",
                handle,
                self.seq[index],
                self.current[index].len()
            );
            if status == BTSTACK_ACL_BUFFERS_FULL {
                self.busy += 1;
                return false;
            }
            self.seq[index] = self.seq[index].wrapping_add(1);
            if status == ERROR_CODE_SUCCESS {
                self.counters.sdus_sent += 1;
                self.bytes_sent += self.current[index].len() as u64;
            } else {
                self.send_error(handle, status);
            }
        }
        self.served[index] = true;
        true
    }

    /// The BIS refused before go again (in order) until one is refused
    fn retry_bis(&mut self, out: &dyn IsoOut) {
        while let Some(&index) = self.bis_retry.first() {
            if !self.send_bis(out, index) {
                out.timer_start(1);
                return;
            }
            self.bis_retry.remove(0);
        }
        self.end_round_if_done(out);
    }

    /// Every BIS of every BIG has its SDU: the next round
    fn end_round_if_done(&mut self, out: &dyn IsoOut) {
        if self.round_open && self.served.iter().all(|&s| s) {
            self.round_open = false;
            self.publish();
            if self.finish_by_ms.take().is_some() {
                // Stopping: every BIS ends on this round's SDU
                self.round_finished = true;
                return;
            }
            self.request_next(out);
        }
    }

    fn send_error(&mut self, handle: u16, status: u8) {
        self.counters.send_errors += 1;
        let n = self.counters.send_errors;
        if n <= 10 || n.is_power_of_two() {
            tracing::debug!("LE Audio: ISO send on 0x{handle:04X} failed (0x{status:02X}), {n} send error(s) so far");
        }
    }

    /// The next set of the jitter buffer, or the silence set on an underrun
    fn take_set(&mut self) -> SduSet {
        let Some(q) = &self.queue else { return SduSet::new() };
        let mut q = q.lock().unwrap_or_else(|e| e.into_inner());
        tracing::trace!("LE Audio: SDU set {}, {} set(s) buffered", self.counters.sets_sent + 1, q.len());
        let set = match q.pop() {
            Some(set) => set,
            None => {
                let mut set = q.spare().unwrap_or_default();
                let silence = q.silence();
                let same_shape = set.len() == silence.len() && set.iter().zip(silence).all(|(a, b)| a.len() == b.len());
                if same_shape {
                    for (dst, src) in set.iter_mut().zip(silence) {
                        dst.copy_from_slice(src);
                    }
                } else {
                    set = silence.clone();
                }
                set
            }
        };
        let underrun = q.take_underrun();
        self.queue_stats = q.stats();
        self.prefill = q.prefill();
        drop(q);
        if let Some(u) = underrun {
            tracing::warn!("{u}");
            self.arm_alignment();
        }
        self.counters.sets_sent += 1;
        if self.counters.sets_sent.is_multiple_of(1000) {
            tracing::debug!("{}", self.send_counters().stats_message());
        }
        set
    }

    fn recycle(&self, set: SduSet) {
        if let Some(q) = &self.queue
            && !set.is_empty()
        {
            q.lock().unwrap_or_else(|e| e.into_inner()).recycle(set);
        }
    }

    fn start_round(&mut self) {
        let next = self.take_set();
        let old = std::mem::replace(&mut self.current, next);
        self.recycle(old);
        self.guard.round_started();
        self.round_open = true;
        self.served.fill(false);
        self.bis_retry.clear();
    }

    /// A CIS of the stream disconnected: the others go on
    pub(crate) fn on_cis_down(&mut self, out: &dyn IsoOut, handle: u16) {
        if !self.sending {
            return;
        }
        for c in self.cis.iter_mut().filter(|c| c.handle == handle) {
            c.down();
        }
        tracing::debug!(
            "LE Audio: ISO sender: CIS 0x{handle:04X} down, {} of {} still up",
            self.cis.iter().filter(|c| c.alive).count(),
            self.cis.len()
        );
        if self.active {
            self.flush(out);
        }
    }

    /// TIMER_ISO: the next interval (CIS), the CIG asked again, or the
    /// guard's wait is over (BIG)
    pub(crate) fn on_timer(&mut self, out: &dyn IsoOut) {
        let guard_wait = self.guard_due_ms.take();
        if let Some(due) = guard_wait {
            let late = out.time_ms().wrapping_sub(due) as i32;
            tracing::trace!("LE Audio: send guard timer, {late} ms late");
            self.guard_late_ms = self.guard_late_ms.max(late.max(0) as u32);
        }
        if !self.sending {
            return;
        }
        if !self.bigs.is_empty() {
            if !self.bis_retry.is_empty() {
                self.retry_bis(out);
            } else if guard_wait.is_some() && self.finish_by_ms.is_none() {
                self.request_bigs(out);
            }
            if let Some(by) = self.finish_by_ms {
                let now = out.time_ms();
                if !past(by, now) {
                    // Never waits on a controller that gives no room
                    tracing::debug!(
                        "LE Audio: the round going on did not finish in time ({} BIS without its SDU), stopping",
                        self.served.iter().filter(|&&s| !s).count()
                    );
                    self.finish_by_ms = None;
                    self.round_open = false;
                    self.bis_retry.clear();
                    self.round_finished = true;
                } else if self.bis_retry.is_empty() {
                    out.timer_start(by.wrapping_sub(now));
                }
            }
        } else if self.active {
            self.tick(out);
        } else {
            self.activate(out);
        }
    }

    /// Starts pacing once BTstack accepts a CIS request (the CIG is active)
    fn activate(&mut self, out: &dyn IsoOut) {
        let Some(handle) = self.cis.iter().find(|c| c.alive).map(|c| c.handle) else { return };
        let status = out.request_can_send(handle);
        if status == ERROR_CODE_COMMAND_DISALLOWED {
            // The CIG is not active yet (ISO data paths being set up)
            self.cig_retries += 1;
            tracing::debug!("LE Audio: CIG not active yet, asking again in {CIG_NOT_ACTIVE_RETRY_MS} ms");
            out.timer_start(CIG_NOT_ACTIVE_RETRY_MS);
            return;
        }
        let now = out.time_ms();
        self.active = true;
        self.t0_ms = Some(now);
        self.last_stats_ms = now;
        self.last_health_ms = now;
        self.last_align_ms = now;
        self.arm_alignment();
        for c in &mut self.cis {
            c.progress_ms = now;
        }
        self.tick(out);
    }

    /// One interval (or more after a late timer): the due SDU sets to the
    /// CIS, sending, the health checks, the timer for the next one
    fn tick(&mut self, out: &dyn IsoOut) {
        let now = out.time_ms();
        let t0 = *self.t0_ms.get_or_insert(now);
        let elapsed_ms = u64::from(now.wrapping_sub(t0));
        let frame = u64::from(self.frame_us);
        let due = elapsed_ms * 1000 / frame + LEAD_SETS;
        // After the run loop was held up: the intervals whose SDUs could no
        // longer be played in time are skipped, the others made at once
        let catch_up = self.pending_max.max(1) as u64;
        if due > self.produced + catch_up {
            let skip = due - catch_up - self.produced;
            tracing::debug!("LE Audio: ISO sender held up {elapsed_ms} ms in: {skip} interval(s) skipped");
            self.skipped += skip;
            self.produced += skip;
        }
        while self.produced < due {
            self.produce(now);
        }
        self.flush(out);
        // flush() reads the clock again and may set a CIS's progress later
        // than `now`
        self.check(out.time_ms());
        let next_ms = ((self.produced + 1 - LEAD_SETS) * frame).div_ceil(1000);
        let wait = next_ms.saturating_sub(elapsed_ms).clamp(1, 1000) as u32;
        out.timer_start(wait);
    }

    /// The next SDU set to every CIS that is up, with its interval's
    /// sequence number
    fn produce(&mut self, now: u32) {
        let seq = self.produced as u16;
        // Its interval (the lead ahead of when it is made), plus the budget
        let t0 = self.t0_ms.unwrap_or(now);
        let interval_end_ms = ((self.produced + 1) * u64::from(self.frame_us) / 1000) as u32;
        let deadline = t0.wrapping_add(interval_end_ms).wrapping_add(self.budget_ms);
        self.produced += 1;
        let set = self.take_set();
        if set.len() == self.cis.len() {
            let max = self.pending_max;
            for (c, sdu) in self.cis.iter_mut().zip(&set) {
                if c.skip > 0 {
                    // Left out: the CIS holds one SDU less from now on
                    c.skip -= 1;
                    continue;
                }
                if c.alive {
                    c.give(seq, sdu, deadline, now, max);
                }
            }
        } else {
            tracing::trace!("LE Audio: SDU set has {} SDUs for {} CIS, nothing sent", set.len(), self.cis.len());
        }
        self.recycle(set);
    }

    /// Sends what waits, one SDU per CIS in turn, while each has room in
    /// the controller; stops while BTstack's packet buffer is taken
    fn flush(&mut self, out: &dyn IsoOut) {
        let n = self.cis.len();
        if !self.active || n == 0 {
            return;
        }
        let depth = usize::from(self.depth);
        let now = out.time_ms();
        let mut sent = false;
        loop {
            let mut any = false;
            for k in 0..n {
                let i = (self.next_cis + k) % n;
                let c = &mut self.cis[i];
                if !c.alive || c.in_flight.len() >= depth {
                    continue;
                }
                c.drop_expired(now);
                let Some(p) = c.pending.front() else { continue };
                let status = out.send(c.handle, p.seq, &p.sdu);
                tracing::trace!(
                    "LE Audio: ISO SDU on 0x{:04X}: seq {}, {} bytes, status 0x{status:02X}",
                    c.handle,
                    p.seq,
                    p.sdu.len()
                );
                if status == BTSTACK_ACL_BUFFERS_FULL {
                    // Sent again (same SDU, same number) once the buffer
                    // is free; this CIS first
                    self.busy += 1;
                    self.next_cis = i;
                    return;
                }
                let Some(p) = c.pending.pop_front() else { continue };
                if status == ERROR_CODE_SUCCESS {
                    if c.in_flight.is_empty() {
                        c.progress_ms = now;
                    }
                    c.in_flight.push_back(now);
                    c.window_peak = c.window_peak.max(c.in_flight.len());
                    c.sent += 1;
                    c.window_sent += 1;
                    self.counters.sdus_sent += 1;
                    self.bytes_sent += p.sdu.len() as u64;
                } else {
                    c.late += 1;
                    c.window_late += 1;
                    let handle = c.handle;
                    self.send_error(handle, status);
                }
                self.cis[i].free.push(p.sdu);
                any = true;
            }
            if !any {
                break;
            }
            sent = true;
        }
        // The next CIS goes first next time
        if sent {
            self.next_cis = (self.next_cis + 1) % n;
        }
    }

    /// Stall watchdog, counters once a second, the health line every 10 s
    fn check(&mut self, now: u32) {
        let mut up = 0;
        let mut stalled = 0;
        for c in self.cis.iter_mut().filter(|c| c.alive) {
            up += 1;
            let idle = c.idle_ms(now);
            if !c.stalled && !c.in_flight.is_empty() && idle >= STALL_MS {
                c.stalled = true;
                tracing::warn!(
                    "LE Audio: CIS 0x{:04X}: no SDU completed for {} s ({} SDU(s) in the controller)",
                    c.handle,
                    idle / 1000,
                    c.in_flight.len()
                );
            }
            stalled += usize::from(c.stalled);
        }
        if up > 0 && stalled == up && !self.stall_reported {
            self.stall_reported = true;
            let why = format!("no SDU completed on any CIS for {} s", STALL_MS / 1000);
            tracing::warn!("LE Audio: ISO sending stalled: {why}");
            self.stall = Some(why);
        } else if stalled > 0 && stalled < up && self.stall.is_none() {
            tracing::trace!("LE Audio: {stalled} of {up} CIS stalled, the others play on");
        }
        if now.wrapping_sub(self.last_align_ms) >= ALIGN_WINDOW_MS {
            self.last_align_ms = now;
            self.check_alignment();
        }
        if now.wrapping_sub(self.last_stats_ms) >= STATS_MS {
            self.last_stats_ms = now;
            self.refresh_queue_stats();
            self.publish();
        }
        let since = now.wrapping_sub(self.last_health_ms);
        if since >= HEALTH_MS {
            self.last_health_ms = now;
            let secs = f64::from(since) / 1000.0;
            let depth = self.depth;
            let per_cis: Vec<String> = self.cis.iter_mut().map(|c| c.health(secs, depth)).collect();
            let s = self.stats();
            tracing::info!(
                "LE stream health: {} | {} silence set(s), {} underrun(s), {} jitter buffer overflow(s), {} send \
                 error(s)",
                per_cis.join(" | "),
                s.silence_sets,
                s.underruns,
                s.overflows,
                s.send_errors
            );
        }
    }

    /// Alignment window over: a CIS a frame behind the earliest one in
    /// [`ALIGN_CONFIRM`] windows in a row gets one SDU left out
    fn check_alignment(&mut self) {
        let lows: Vec<(usize, u32)> = self
            .cis
            .iter_mut()
            .enumerate()
            .filter(|(_, c)| c.alive)
            .filter_map(|(i, c)| c.take_align_low().map(|m| (i, m)))
            .collect();
        if lows.len() < 2 || !self.align_armed || self.realigns >= ALIGN_MAX || super::bap::realign_off() {
            self.align_suspect = None;
            return;
        }
        if self.align_settle > 0 {
            self.align_settle -= 1;
            return;
        }
        // In line, a CIS completes later than another by the difference of
        // their places in the CIG event: each low is taken from its place
        let frame_us = i64::from(self.frame_us);
        let in_line: Vec<(usize, u32, i64)> = lows
            .iter()
            .map(|&(i, ms)| (i, ms, i64::from(ms) * 1000 - i64::from(self.anchor_offset_us(self.cis[i].handle))))
            .collect();
        let Some(&(_, earliest, first)) = in_line.iter().min_by_key(|&&(_, _, us)| us) else { return };
        let Some(&(late, latest, last)) = in_line.iter().max_by_key(|&&(_, _, us)| us) else { return };
        let threshold_us = (frame_us * i64::from(ALIGN_THRESHOLD_PCT) / 100).max(1000);
        let handle = self.cis[late].handle;
        let skew_us = last - first;
        // A controller that reports the completions of all CIS together
        // shows no gap while in line, whatever their places: completions
        // that close together are left alone
        let together = i64::from(latest.abs_diff(earliest)) * 1000 < frame_us - threshold_us;
        if skew_us < threshold_us || together {
            self.align_suspect = None;
            return;
        }
        // Whole frames behind (a CIS may hold more than one SDU too many)
        let frames = ((skew_us + frame_us / 2) / frame_us).clamp(1, i64::from(ALIGN_FRAMES_MAX)) as u8;
        let windows = match self.align_suspect {
            Some((h, n)) if h == handle => n + 1,
            _ => 1,
        };
        if windows < ALIGN_CONFIRM {
            tracing::debug!(
                "LE Audio: CIS 0x{handle:04X} completes {:.1} ms later than in line with the earliest CIS ({latest} \
                 vs {earliest} ms, p{ALIGN_PERCENTILE}, window {windows} of {ALIGN_CONFIRM})",
                skew_us as f64 / 1000.0
            );
            self.align_suspect = Some((handle, windows));
            return;
        }
        self.align_suspect = None;
        self.align_armed = false;
        self.realigns += 1;
        self.cis[late].skip = frames;
        tracing::info!(
            "LE Audio: CIS 0x{handle:04X} plays {frames} frame(s) behind the others (completion {latest} vs \
             {earliest} ms, p{ALIGN_PERCENTILE}, {:.1} ms later than in line): {frames} SDU(s) left out to bring it \
             in line ({} of {ALIGN_MAX})",
            skew_us as f64 / 1000.0,
            self.realigns
        );
        if self.realigns == ALIGN_MAX {
            tracing::warn!("LE Audio: CIS realigned {ALIGN_MAX} times: no further realignment in this stream");
        }
    }

    /// How long after the CIG event's start the events of the CIS `handle`
    /// begin (LE CIS Established), us; 0 while not known
    fn anchor_offset_us(&self, handle: u16) -> u32 {
        self.links
            .iter()
            .find(|l| l.cis_handle == handle)
            .map_or(0, |l| l.cig_sync_delay.saturating_sub(l.cis_sync_delay))
    }

    fn refresh_queue_stats(&mut self) {
        if let Some(q) = &self.queue {
            let q = q.lock().unwrap_or_else(|e| e.into_inner());
            self.queue_stats = q.stats();
            self.prefill = q.prefill();
        }
    }

    pub fn stats(&self) -> LeStreamStats {
        let c = self.send_counters();
        LeStreamStats {
            sets_encoded: c.sets_encoded,
            sets_sent: c.sets_sent,
            sdus_sent: c.sdus_sent,
            bytes_sent: self.bytes_sent,
            silence_sets: c.silence_sets,
            underruns: c.underruns,
            overflows: c.overflows,
            send_errors: c.send_errors,
            prefill: self.prefill as u32,
            late_sdus: self.cis.iter().map(|c| c.late).sum(),
            cis_stalled: self.cis_stalled(),
            transport_latency_us: self
                .cis_handles
                .iter()
                .filter_map(|h| self.links.iter().find(|l| l.cis_handle == *h))
                .map(|l| l.transport_latency_c_to_p)
                .max()
                .unwrap_or(0),
        }
    }

    /// Stalled CIS while others play; 0 when none plays (the stream is
    /// lost then, not partial)
    fn cis_stalled(&self) -> u32 {
        let up = self.cis.iter().filter(|c| c.alive).count();
        let stalled = self.cis.iter().filter(|c| c.alive && c.stalled).count();
        if stalled < up { stalled as u32 } else { 0 }
    }

    fn send_counters(&self) -> SendCounters {
        SendCounters {
            sets_encoded: self.queue_stats.sets_pushed,
            silence_sets: self.queue_stats.silence_sets,
            underruns: self.queue_stats.underruns,
            overflows: self.queue_stats.overflows,
            ..self.counters
        }
    }

    fn publish(&self) {
        *self.stats_out.lock().unwrap_or_else(|e| e.into_inner()) = self.stats();
    }

    /// BIS can-send-now of every BIG (one round for all)
    fn request_bigs(&self, out: &dyn IsoOut) {
        for &(big, _) in &self.bigs {
            out.request_bis_can_send(big);
        }
    }

    /// BIG: the next round now, or once the guard allows it
    fn request_next(&mut self, out: &dyn IsoOut) {
        if self.bigs.is_empty() || !self.sending || self.cis_handles.is_empty() {
            return;
        }
        let now = out.time_ms();
        let t0 = *self.t0_ms.get_or_insert(now);
        let elapsed_ms = u64::from(now.wrapping_sub(t0));
        if self.guard.may_send(elapsed_ms * 1000) {
            self.request_bigs(out);
            return;
        }
        // Ahead of real time (a controller that completes SDUs early): wait
        let due_ms = self.guard.next_due_us() / 1000;
        let wait = if due_ms > elapsed_ms { due_ms - elapsed_ms } else { u64::from(self.frame_us / 1000) };
        let wait = wait.clamp(1, 1000) as u32;
        self.guard_waits += 1;
        tracing::trace!("LE Audio: send guard: {elapsed_ms} ms in, ahead of real time, waiting {wait} ms");
        self.guard_due_ms = Some(now.wrapping_add(wait));
        out.timer_start(wait);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use btstack_sys::consts::HCI_EVENT_BIS_CAN_SEND_NOW;
    use le_audio::sdu_queue::SduQueue;

    use super::*;

    #[derive(Default)]
    struct Fake {
        now: Cell<u32>,
        log: RefCell<Vec<String>>,
        /// CIS requests still refused with Command Disallowed (CIG not active)
        refuse: Cell<u32>,
        /// Sends still refused with BTSTACK_ACL_BUFFERS_FULL
        busy: Cell<u32>,
        /// Added to the clock after each read (a clock that moves on while
        /// the sender works)
        step: Cell<u32>,
    }

    impl Fake {
        fn take(&self) -> Vec<String> {
            self.log.borrow_mut().drain(..).collect()
        }

        /// The sends of the log so far, the rest dropped
        fn sends(&self) -> Vec<String> {
            self.take().into_iter().filter(|l| l.starts_with("send")).collect()
        }
    }

    impl IsoOut for Fake {
        fn time_ms(&self) -> u32 {
            let now = self.now.get();
            self.now.set(now.wrapping_add(self.step.get()));
            now
        }
        fn set_queue(&self, packets: u8) {
            self.log.borrow_mut().push(format!("queue {packets}"));
        }
        fn request_can_send(&self, cis: u16) -> u8 {
            self.log.borrow_mut().push(format!("request {cis:x}"));
            if self.refuse.get() > 0 {
                self.refuse.set(self.refuse.get() - 1);
                return ERROR_CODE_COMMAND_DISALLOWED;
            }
            ERROR_CODE_SUCCESS
        }
        fn request_bis_can_send(&self, big: u8) {
            self.log.borrow_mut().push(format!("request BIG {big}"));
        }
        fn send(&self, cis: u16, seq: u16, sdu: &[u8]) -> u8 {
            if self.busy.get() > 0 {
                self.busy.set(self.busy.get() - 1);
                self.log.borrow_mut().push(format!("busy {cis:x} #{seq}"));
                return BTSTACK_ACL_BUFFERS_FULL;
            }
            self.log.borrow_mut().push(format!("send {cis:x} #{seq} {:x}", sdu[0]));
            0
        }
        fn timer_start(&self, ms: u32) {
            self.log.borrow_mut().push(format!("timer {ms}"));
        }
        fn timer_stop(&self) {
            self.log.borrow_mut().push("timer stop".into());
        }
    }

    /// As BTstack 1.8.1 emits it: without the BIS handle
    fn bis_can_send(big: u8, index: u8, _bis: u16) -> [u8; 4] {
        [HCI_EVENT_BIS_CAN_SEND_NOW, 2, big, index]
    }

    fn completed(handle: u16, n: u16) -> [u8; 7] {
        [HCI_EVENT_NUMBER_OF_COMPLETED_PACKETS, 5, 1, handle as u8, (handle >> 8) as u8, n as u8, (n >> 8) as u8]
    }

    const PACKET_SENT: [u8; 2] = [HCI_EVENT_TRANSPORT_PACKET_SENT, 0];

    fn queue(cis: usize) -> SharedSduQueue {
        let q: SharedSduQueue = Arc::new(Mutex::new(SduQueue::new()));
        q.lock().unwrap().configure(QUEUE_DEPTH, vec![vec![0xEE; 4]; cis]);
        q
    }

    fn set(tag: u8) -> SduSet {
        vec![vec![tag; 4], vec![tag; 4]]
    }

    /// Advances the fake clock to `ms` and fires the timer
    fn at(s: &mut IsoSender, out: &Fake, ms: u32) {
        out.now.set(ms);
        s.on_timer(out);
    }

    #[test]
    fn big_rounds_end_with_the_last_bis() {
        let out = Fake::default();
        let stats = Arc::new(Mutex::new(LeStreamStats::default()));
        let mut s = IsoSender::new(stats.clone());
        let q = queue(2);
        s.start_big(&out, 0, &[0x10, 0x11], q.clone(), 10_000, 2);
        assert_eq!(out.take(), ["queue 2", "request BIG 0"]);
        // Another BIG's events do not count
        s.on_bis_can_send_now(&out, &bis_can_send(1, 0, 0x10));
        assert!(out.take().is_empty());
        s.on_bis_can_send_now(&out, &bis_can_send(0, 0, 0x10));
        assert_eq!(out.take(), ["send 10 #0 ee"]);
        s.on_bis_can_send_now(&out, &bis_can_send(0, 1, 0x11));
        assert_eq!(out.take(), ["send 11 #0 ee", "request BIG 0"]);
        s.on_bis_can_send_now(&out, &bis_can_send(0, 0, 0x10));
        s.on_bis_can_send_now(&out, &bis_can_send(0, 1, 0x11));
        assert_eq!(out.take(), ["send 10 #1 ee", "send 11 #1 ee", "request BIG 0"]);
        s.stop(&out);
        assert_eq!(stats.lock().unwrap().sdus_sent, 4);
    }

    #[test]
    fn a_refused_bis_send_goes_again_in_the_same_round() {
        let out = Fake::default();
        let stats = Arc::new(Mutex::new(LeStreamStats::default()));
        let mut s = IsoSender::new(stats.clone());
        s.start_big(&out, 0, &[0x10, 0x11], queue(2), 10_000, 2);
        assert_eq!(out.take(), ["queue 2", "request BIG 0"]);
        s.on_bis_can_send_now(&out, &bis_can_send(0, 0, 0x10));
        // BTstack's packet buffer is still taken for the second BIS
        out.busy.set(1);
        s.on_bis_can_send_now(&out, &bis_can_send(0, 1, 0x11));
        assert_eq!(out.take(), ["send 10 #0 ee", "busy 11 #0", "timer 1"]);
        // Free again: the same SDU with the same number, then the round ends
        s.on_hci_event(&out, &PACKET_SENT);
        assert_eq!(out.take(), ["send 11 #0 ee", "request BIG 0"]);
        // The retry's timer fires late: no extra request
        at(&mut s, &out, 1);
        assert!(out.take().is_empty());
        s.on_bis_can_send_now(&out, &bis_can_send(0, 0, 0x10));
        s.on_bis_can_send_now(&out, &bis_can_send(0, 1, 0x11));
        assert_eq!(out.take(), ["send 10 #1 ee", "send 11 #1 ee", "request BIG 0"]);
        s.stop(&out);
        let st = *stats.lock().unwrap();
        assert_eq!((st.sets_sent, st.sdus_sent, st.send_errors), (2, 4, 0));
    }

    #[test]
    fn a_refused_send_on_the_second_big_keeps_both_bigs_on_one_set() {
        let out = Fake::default();
        let stats = Arc::new(Mutex::new(LeStreamStats::default()));
        let mut s = IsoSender::new(stats.clone());
        let q = queue(2);
        s.start_bigs(&out, &[(0, vec![0x10]), (1, vec![0x20])], q.clone(), 10_000, 2);
        out.take();
        s.on_bis_can_send_now(&out, &bis_can_send(0, 0, 0x10));
        out.busy.set(2);
        s.on_bis_can_send_now(&out, &bis_can_send(1, 0, 0x20));
        // Still taken when the timer fires, free on the packet sent
        at(&mut s, &out, 1);
        assert_eq!(out.take(), ["send 10 #0 ee", "busy 20 #0", "timer 1", "busy 20 #0", "timer 1"]);
        s.on_hci_event(&out, &PACKET_SENT);
        assert_eq!(out.take(), ["send 20 #0 ee", "request BIG 0", "request BIG 1"]);
        // The next round: both BIGs carry the next set with the next number
        for tag in 1..=5 {
            q.lock().unwrap().push(set(tag));
        }
        s.on_bis_can_send_now(&out, &bis_can_send(1, 0, 0x20));
        s.on_bis_can_send_now(&out, &bis_can_send(0, 0, 0x10));
        assert_eq!(out.take(), ["send 20 #1 1", "send 10 #1 1", "request BIG 0", "request BIG 1"]);
        s.stop(&out);
        let st = *stats.lock().unwrap();
        assert_eq!((st.sdus_sent, st.send_errors), (4, 0));
        assert_eq!(s.busy, 2);
    }

    #[test]
    fn rounds_over_two_bigs_end_once_both_sent() {
        let out = Fake::default();
        let stats = Arc::new(Mutex::new(LeStreamStats::default()));
        let mut s = IsoSender::new(stats.clone());
        let q: SharedSduQueue = Arc::new(Mutex::new(SduQueue::new()));
        q.lock().unwrap().configure(2, vec![vec![0xEE; 4], vec![0xEE; 4]]);
        // Left in BIG 0, right in BIG 1: each its BIS 0
        s.start_bigs(&out, &[(0, vec![0x10]), (1, vec![0x20])], q.clone(), 10_000, 2);
        assert_eq!(out.take(), ["queue 2", "request BIG 0", "request BIG 1"]);
        // An index beyond a BIG's own BIS is ignored
        s.on_bis_can_send_now(&out, &bis_can_send(0, 1, 0));
        assert!(out.take().is_empty());
        s.on_bis_can_send_now(&out, &bis_can_send(1, 0, 0x20));
        assert_eq!(out.take(), ["send 20 #0 ee"]);
        s.on_bis_can_send_now(&out, &bis_can_send(0, 0, 0x10));
        assert_eq!(out.take(), ["send 10 #0 ee", "request BIG 0", "request BIG 1"]);
        // The next round, the BIGs in the other order: same set for both
        s.on_bis_can_send_now(&out, &bis_can_send(0, 0, 0x10));
        s.on_bis_can_send_now(&out, &bis_can_send(1, 0, 0x20));
        assert_eq!(out.take(), ["send 10 #1 ee", "send 20 #1 ee", "request BIG 0", "request BIG 1"]);
        s.stop(&out);
        assert_eq!(stats.lock().unwrap().sets_sent, 2);
        assert_eq!(stats.lock().unwrap().sdus_sent, 4);
    }

    /// Two BIGs of one BIS each, the first round sent on both
    fn two_bigs_one_round_in(out: &Fake, q: SharedSduQueue) -> IsoSender {
        let mut s = IsoSender::new(Arc::new(Mutex::new(LeStreamStats::default())));
        s.start_bigs(out, &[(0, vec![0x10]), (1, vec![0x20])], q, 10_000, 2);
        s.on_bis_can_send_now(out, &bis_can_send(0, 0, 0x10));
        s.on_bis_can_send_now(out, &bis_can_send(1, 0, 0x20));
        out.take();
        s
    }

    #[test]
    fn a_stop_mid_round_finishes_the_round_on_every_bis() {
        let out = Fake::default();
        let mut s = two_bigs_one_round_in(&out, queue(2));
        // The next round: BIG 0 got its SDU, BIG 1 has no room yet
        s.on_bis_can_send_now(&out, &bis_can_send(0, 0, 0x10));
        assert_eq!(out.take(), ["send 10 #1 ee"]);
        assert!(s.finish_round(&out));
        assert_eq!(out.take(), ["timer 20"]);
        // Asked again (a second stop): still waiting, no second timer
        assert!(s.finish_round(&out));
        assert!(out.take().is_empty());
        // BIG 0 has room again first: no further round
        s.on_bis_can_send_now(&out, &bis_can_send(0, 0, 0x10));
        assert!(out.take().is_empty());
        assert!(!s.take_round_finished());
        // BIG 1 gets its SDU of the round: both end on #1, no further request
        s.on_bis_can_send_now(&out, &bis_can_send(1, 0, 0x20));
        assert_eq!(out.take(), ["send 20 #1 ee"]);
        assert!(s.take_round_finished());
        assert!(!s.take_round_finished());
        s.on_bis_can_send_now(&out, &bis_can_send(0, 0, 0x10));
        at(&mut s, &out, 20);
        assert!(out.take().is_empty());
        s.stop(&out);
        assert_eq!(out.take(), ["timer stop"]);
    }

    #[test]
    fn a_stop_between_rounds_stops_at_once() {
        let out = Fake::default();
        let mut s = two_bigs_one_round_in(&out, queue(2));
        assert!(!s.finish_round(&out));
        assert!(out.take().is_empty());
        assert!(!s.take_round_finished());
        // Not sending
        s.stop(&out);
        assert!(!s.finish_round(&out));
    }

    #[test]
    fn a_stop_waits_for_a_refused_bis_send() {
        let out = Fake::default();
        let mut s = two_bigs_one_round_in(&out, queue(2));
        // BIG 0 refused while BTstack's packet buffer is taken, BIG 1 no
        // room yet
        out.busy.set(1);
        s.on_bis_can_send_now(&out, &bis_can_send(0, 0, 0x10));
        assert_eq!(out.take(), ["busy 10 #1", "timer 1"]);
        // The retry's 1 ms timer runs: no timer of its own
        assert!(s.finish_round(&out));
        assert!(out.take().is_empty());
        s.on_hci_event(&out, &PACKET_SENT);
        assert_eq!(out.take(), ["send 10 #1 ee"]);
        // The retry's timer: the rest of the wait
        at(&mut s, &out, 1);
        assert_eq!(out.take(), ["timer 19"]);
        s.on_bis_can_send_now(&out, &bis_can_send(1, 0, 0x20));
        assert_eq!(out.take(), ["send 20 #1 ee"]);
        assert!(s.take_round_finished());
    }

    #[test]
    fn a_stop_does_not_wait_on_a_controller_without_room() {
        let out = Fake::default();
        let mut s = two_bigs_one_round_in(&out, queue(2));
        s.on_bis_can_send_now(&out, &bis_can_send(0, 0, 0x10));
        out.take();
        assert!(s.finish_round(&out));
        assert_eq!(out.take(), ["timer 20"]);
        // A timer that fires early waits the rest
        at(&mut s, &out, 5);
        assert_eq!(out.take(), ["timer 15"]);
        assert!(!s.take_round_finished());
        at(&mut s, &out, 20);
        assert!(out.take().is_empty());
        assert!(s.take_round_finished());
        // Room after that: nothing more
        s.on_bis_can_send_now(&out, &bis_can_send(1, 0, 0x20));
        assert!(out.take().is_empty());
    }

    #[test]
    fn a_stop_does_not_wait_on_a_send_refused_to_the_end() {
        let out = Fake::default();
        let mut s = IsoSender::new(Arc::new(Mutex::new(LeStreamStats::default())));
        s.start_big(&out, 0, &[0x10, 0x11], queue(2), 10_000, 2);
        s.on_bis_can_send_now(&out, &bis_can_send(0, 0, 0x10));
        out.busy.set(u32::MAX);
        s.on_bis_can_send_now(&out, &bis_can_send(0, 1, 0x11));
        assert!(s.finish_round(&out));
        out.take();
        for ms in 1..=20 {
            assert!(!s.take_round_finished());
            at(&mut s, &out, ms);
        }
        assert!(s.take_round_finished());
        assert_eq!(out.sends(), Vec::<String>::new());
    }

    #[test]
    fn a_start_after_a_stop_mid_round_begins_afresh() {
        let out = Fake::default();
        let q = queue(2);
        let mut s = two_bigs_one_round_in(&out, q.clone());
        out.busy.set(1);
        s.on_bis_can_send_now(&out, &bis_can_send(0, 0, 0x10));
        assert!(s.finish_round(&out));
        // The time runs out with BIS 0x10 still to send again
        out.busy.set(u32::MAX);
        at(&mut s, &out, 20);
        assert!(s.take_round_finished());
        s.stop(&out);
        out.busy.set(0);
        out.take();
        // Late events of the old BIGs do nothing
        s.on_hci_event(&out, &PACKET_SENT);
        s.on_bis_can_send_now(&out, &bis_can_send(1, 0, 0x20));
        at(&mut s, &out, 21);
        assert!(out.take().is_empty());
        // Started again (quickly): numbers from 0, a full first round, no
        // leftover retry
        s.start_bigs(&out, &[(0, vec![0x30]), (1, vec![0x31])], q.clone(), 10_000, 2);
        assert_eq!(out.take(), ["queue 2", "request BIG 0", "request BIG 1"]);
        s.on_hci_event(&out, &PACKET_SENT);
        at(&mut s, &out, 22);
        assert!(out.take().is_empty());
        s.on_bis_can_send_now(&out, &bis_can_send(1, 0, 0x31));
        s.on_bis_can_send_now(&out, &bis_can_send(0, 0, 0x30));
        assert_eq!(out.take(), ["send 31 #0 ee", "send 30 #0 ee", "request BIG 0", "request BIG 1"]);
        assert!(!s.take_round_finished());
    }

    #[test]
    fn cis_paced_by_the_sdu_interval_with_a_lead() {
        let out = Fake::default();
        let stats = Arc::new(Mutex::new(LeStreamStats::default()));
        let mut s = IsoSender::new(stats.clone());
        let q = queue(2);
        s.start(&out, &[0x60, 0x61], q.clone(), 10_000, 8, 100);
        // Two sets at once (the lead); BTstack's buffer is taken after
        // every packet in real life, not in this fake
        assert_eq!(
            out.take(),
            ["queue 8", "request 60", "send 60 #0 ee", "send 61 #0 ee", "send 60 #1 ee", "send 61 #1 ee", "timer 10"]
        );
        // Nothing more before the next interval
        at(&mut s, &out, 5);
        assert_eq!(out.take(), ["timer 5"]);
        for tag in 1..=5 {
            q.lock().unwrap().push(set(tag));
        }
        at(&mut s, &out, 10);
        assert_eq!(out.take(), ["send 61 #2 1", "send 60 #2 1", "timer 10"]);
        // A timer 25 ms late: the intervals it missed go out at once
        at(&mut s, &out, 45);
        assert_eq!(
            out.sends(),
            ["send 60 #3 2", "send 61 #3 2", "send 60 #4 3", "send 61 #4 3", "send 60 #5 4", "send 61 #5 4"]
        );
        s.stop(&out);
        let st = *stats.lock().unwrap();
        assert_eq!((st.sets_sent, st.sdus_sent, st.silence_sets, st.sets_encoded, st.late_sdus), (6, 12, 2, 5, 0));
    }

    /// Two CIS sending each SDU at once; the second's SDUs complete one
    /// interval later than the first's (its stream a frame behind). The
    /// sender, the fake, the second CIS's sequence numbers.
    fn run_skewed(until_ms: u32) -> (IsoSender, Fake, Vec<u16>) {
        let out = Fake::default();
        let mut s = IsoSender::new(Arc::default());
        s.start(&out, &[0x60, 0x61], queue(2), 10_000, 8, 100);
        out.take();
        let mut right = Vec::new();
        let mut t = 10;
        while t <= until_ms {
            at(&mut s, &out, t);
            for l in out.sends() {
                if let Some(seq) = l.strip_prefix("send 61 #") {
                    right.push(seq.split(' ').next().unwrap().parse().unwrap());
                }
            }
            // One SDU leaves each controller queue an interval; the second
            // CIS started an interval later (it holds one more SDU)
            s.on_hci_event(&out, &completed(0x60, 1));
            if t > 10 {
                s.on_hci_event(&out, &completed(0x61, 1));
            }
            t += 10;
        }
        (s, out, right)
    }

    /// LE CIS Established of `handle` with its sync delays (us)
    fn established(handle: u8, cig_sync_delay: u32, cis_sync_delay: u32) -> Vec<u8> {
        let mut ev = vec![0x3E, 29, 0x19, 0x00, handle, 0x00];
        ev.extend_from_slice(&cig_sync_delay.to_le_bytes()[..3]);
        ev.extend_from_slice(&cis_sync_delay.to_le_bytes()[..3]);
        ev.extend_from_slice(&[0x10, 0x27, 0x00, 0x10, 0x27, 0x00]); // 10 ms both ways
        // BN 0: the queue depth stays the one given to start
        ev.extend_from_slice(&[2, 2, 2, 0, 0, 1, 1, 155, 0, 0, 0, 8, 0]);
        ev
    }

    /// 10 s of two CIS (0x60, 0x61) on a controller with a CIG event every
    /// 10 ms, from when the host sends: the events of each CIS begin
    /// `place_us` into it, and each event takes the oldest SDU the
    /// controller holds for its CIS. A CIS `behind` by n frames had its
    /// first n events without an SDU, so it holds n more from then on.
    /// `together`: the controller reports the completions of both CIS at
    /// the end of the CIG event instead of at each CIS's own events. The
    /// sender and the SDUs left out of each CIS.
    fn run_placed(place_us: [u32; 2], behind: [u32; 2], together: bool) -> (IsoSender, [usize; 2]) {
        const HANDLES: [u8; 2] = [0x60, 0x61];
        let cig_sync_delay = 10_000;
        let out = Fake::default();
        let mut s = IsoSender::new(Arc::default());
        for (h, place) in HANDLES.iter().zip(place_us) {
            s.on_hci_event(&out, &established(*h, cig_sync_delay, cig_sync_delay - place));
        }
        s.start(&out, &[0x60, 0x61], queue(2), 10_000, 8, 100);
        out.take();
        let mut held = [0u32; 2];
        let mut last: [Option<u16>; 2] = [None; 2];
        let mut left_out = [0usize; 2];
        for t in 10..=10_000u32 {
            if t.is_multiple_of(10) {
                at(&mut s, &out, t);
                for l in out.sends() {
                    for (i, h) in HANDLES.iter().enumerate() {
                        let Some(rest) = l.strip_prefix(&format!("send {h:x} #")) else { continue };
                        let seq: u16 = rest.split(' ').next().unwrap().parse().unwrap();
                        // Sequence numbers skipped are SDUs left out
                        left_out[i] += last[i].map_or(0, |n| usize::from(seq - n - 1));
                        last[i] = Some(seq);
                        held[i] += 1;
                    }
                }
            }
            for i in 0..2 {
                let event_ms = if together { 9 } else { place_us[i] / 1000 };
                let first = 10 + 10 * behind[i] + event_ms;
                if t >= first && (t - event_ms).is_multiple_of(10) && held[i] > 0 {
                    held[i] -= 1;
                    out.now.set(t);
                    s.on_hci_event(&out, &completed(u16::from(HANDLES[i]), 1));
                }
            }
        }
        (s, left_out)
    }

    #[test]
    fn a_cis_a_frame_behind_is_brought_in_line_once() {
        let (s, _, right) = run_skewed(10_000);
        // A window left out, three behind: one SDU of the second CIS left
        // out (its numbers skip one), then they are in line and stay so
        assert_eq!(s.realigns, 1);
        let gaps: Vec<u16> = right.windows(2).filter(|w| w[1] != w[0] + 1).map(|w| w[1]).collect();
        assert_eq!(gaps.len(), 1, "{gaps:?}");
        assert!((790..=810).contains(&gaps[0]), "{gaps:?}");
    }

    #[test]
    fn cis_are_compared_from_their_places_in_the_cig_event() {
        // As measured with a WF-1000XM6: the second CIS's events begin
        // 3.1 ms after the first one's. In line: it completes 3 ms later
        let place = [0, 3147];
        let (s, left_out) = run_placed(place, [0, 0], false);
        assert_eq!((s.realigns, left_out), (0, [0, 0]));
        // The CIS that comes first a frame behind completes only 7 ms after
        // the other one, under 80 % of a frame, and is a frame from its
        // place all the same
        let (s, left_out) = run_placed(place, [1, 0], false);
        assert_eq!((s.realigns, left_out), (1, [1, 0]));
        // The one that comes later a frame behind: 13 ms after
        let (s, left_out) = run_placed(place, [0, 1], false);
        assert_eq!((s.realigns, left_out), (1, [0, 1]));
        // A place of most of a frame is not a frame behind
        let (s, left_out) = run_placed([0, 9000], [0, 0], false);
        assert_eq!((s.realigns, left_out), (0, [0, 0]));
    }

    #[test]
    fn a_cis_two_frames_behind_gets_two_sdus_left_out_at_once() {
        let (s, left_out) = run_placed([0, 3147], [2, 0], false);
        assert_eq!((s.realigns, left_out), (1, [2, 0]));
    }

    #[test]
    fn completions_reported_together_are_not_taken_from_their_places() {
        // Both CIS complete at the end of the CIG event, whatever their
        // places: no gap, so nothing is behind, even with places most of a
        // frame apart
        let (s, left_out) = run_placed([0, 9000], [0, 0], true);
        assert_eq!((s.realigns, left_out), (0, [0, 0]));
        let (s, left_out) = run_placed([0, 3147], [0, 0], true);
        assert_eq!((s.realigns, left_out), (0, [0, 0]));
    }

    #[test]
    fn retransmissions_do_not_count_as_a_frame_behind() {
        // The second CIS's SDUs often need the next event (FT 2): its
        // median completion moves an interval later, its first tries not
        let out = Fake::default();
        let mut s = IsoSender::new(Arc::default());
        s.start(&out, &[0x60, 0x61], queue(2), 10_000, 8, 100);
        out.take();
        let mut held = 0u16;
        for i in 1..=1500u32 {
            at(&mut s, &out, i * 10);
            s.on_hci_event(&out, &completed(0x60, 1));
            // 6 of 10 SDUs through a retry an interval later
            let retried = i % 10 < 6;
            let n = held + u16::from(!retried);
            if n > 0 {
                s.on_hci_event(&out, &completed(0x61, n));
            }
            held = u16::from(retried);
        }
        assert_eq!(s.realigns, 0);
    }

    #[test]
    fn a_realignment_waits_for_the_next_underrun() {
        let (mut s, _, _) = run_skewed(10_000);
        assert_eq!(s.realigns, 1);
        assert!(!s.align_armed);
        s.arm_alignment();
        assert!(s.align_armed && s.align_settle == 1);
    }

    #[test]
    fn cis_in_line_are_left_alone() {
        let out = Fake::default();
        let mut s = IsoSender::new(Arc::default());
        s.start(&out, &[0x60, 0x61], queue(2), 10_000, 8, 100);
        out.take();
        for i in 1..=1000u32 {
            at(&mut s, &out, i * 10);
            s.on_hci_event(&out, &completed(0x60, 1));
            s.on_hci_event(&out, &completed(0x61, 1));
        }
        assert_eq!(s.realigns, 0);
        assert_eq!(s.cis[0].sent, s.cis[1].sent);
    }

    #[test]
    fn a_cis_that_never_completes_does_not_stop_the_other() {
        let out = Fake::default();
        let stats = Arc::new(Mutex::new(LeStreamStats::default()));
        let mut s = IsoSender::new(stats.clone());
        s.start(&out, &[0x60, 0x61], queue(2), 10_000, 4, 100);
        out.take();
        let mut right = Vec::new();
        for i in 1..=100u32 {
            at(&mut s, &out, i * 10);
            // The left CIS completes its SDUs, the right one never
            s.on_hci_event(&out, &completed(0x60, 1));
            for l in out.sends() {
                if l.starts_with("send 61") {
                    right.push(l);
                }
            }
        }
        // The right CIS stops at its queue depth; the left one goes on
        assert_eq!(right.len(), 2, "{right:?}");
        let left = s.cis[0].sent;
        assert!(left >= 100, "{left}");
        // Its SDUs wait as long as they could still be played (100 ms past
        // their interval), older ones are dropped as late; the numbers of
        // the right one keep counting intervals
        assert!(s.cis[1].late >= 80, "{}", s.cis[1].late);
        let seqs: Vec<u16> = s.cis[1].pending.iter().map(|p| p.seq).collect();
        assert_eq!(seqs, (89..=101).collect::<Vec<u16>>());
        // 2 s without a completion: stalled, the other plays on (no failure)
        at(&mut s, &out, 2010);
        assert!(s.take_stall().is_none());
        assert_eq!(stats.lock().unwrap().cis_stalled, 1);
        // Completions again: it sends the oldest SDUs still in time, not
        // those of before
        s.on_hci_event(&out, &completed(0x61, 4));
        assert_eq!(
            out.sends().iter().filter(|l| l.starts_with("send 61")).cloned().collect::<Vec<_>>(),
            ["send 61 #190 ee", "send 61 #191 ee", "send 61 #192 ee", "send 61 #193 ee"]
        );
        assert_eq!(s.stats().cis_stalled, 0);
    }

    /// One CIS completing each SDU 15 ms after it was sent, except that
    /// nothing completes from `stall_from` for `stall` ms (all at once
    /// then; None: never again). The sequence numbers sent and the sender.
    fn run_with_stall(depth: u8, stall_from: u32, stall: Option<u32>, until: u32) -> (Vec<u16>, IsoSender, Fake) {
        let out = Fake::default();
        let mut s = IsoSender::new(Arc::default());
        s.start(&out, &[0x60], queue(1), 10_000, depth, 100);
        let mut seqs = Vec::new();
        for t in 0..=until {
            out.now.set(t);
            let held = t >= stall_from && stall.is_none_or(|d| t < stall_from + d);
            if !held {
                let n = s.cis[0].in_flight.iter().filter(|&&at| t.wrapping_sub(at) >= 15).count();
                if n > 0 {
                    s.on_hci_event(&out, &completed(0x60, n as u16));
                }
            }
            s.on_timer(&out);
            for l in out.take() {
                if let Some(rest) = l.strip_prefix("send 60 #") {
                    seqs.push(rest.split(' ').next().unwrap().parse().unwrap());
                }
            }
        }
        (seqs, s, out)
    }

    #[test]
    fn a_completion_stall_within_the_budget_costs_no_sdu() {
        // FT 2 and FT 10 at a 10 ms ISO interval: the depth they get
        for depth in [depth_for_flush(2, 10_000, 10_000), depth_for_flush(10, 10_000, 10_000)] {
            for stall in [30, 60, 100] {
                let (seqs, s, _) = run_with_stall(depth, 1000, Some(stall), 2000);
                assert_eq!(s.cis[0].late, 0, "depth {depth}, {stall} ms");
                // Every interval's SDU, in order, none skipped
                assert!(seqs.windows(2).all(|w| w[1] == w[0] + 1), "depth {depth}, {stall} ms");
                assert!(seqs.len() >= 200, "depth {depth}, {stall} ms: {}", seqs.len());
            }
        }
    }

    #[test]
    fn a_stall_past_the_budget_drops_the_oldest_and_stays_bounded() {
        let depth = depth_for_flush(2, 10_000, 10_000);
        // 200 ms without a completion: the SDUs past 100 ms + their interval
        // are dropped, the rest go out once completions are back
        let (seqs, s, _) = run_with_stall(depth, 1000, Some(200), 2000);
        let late = s.cis[0].late;
        assert!((1..=10).contains(&late), "{late}");
        let gaps: Vec<_> = seqs.windows(2).filter(|w| w[1] != w[0] + 1).collect();
        assert_eq!(gaps.len(), 1, "{gaps:?}");
        // Never again: bounded backlog, dropped as late, reported stalled
        let (_, mut s, out) = run_with_stall(depth, 1000, None, 3500);
        assert!(s.cis[0].pending.len() <= s.pending_max, "{}", s.cis[0].pending.len());
        assert!(s.cis[0].late > 200, "{}", s.cis[0].late);
        assert_eq!(s.take_stall().as_deref(), Some("no SDU completed on any CIS for 2 s"));
        s.stop(&out);
    }

    #[test]
    fn the_wait_budget_follows_the_max_transport_latency() {
        assert_eq!(wait_budget_ms(100), 100);
        assert_eq!(wait_budget_ms(15), MIN_WAIT_BUDGET_MS);
        assert_eq!(wait_budget_ms(0), DEFAULT_WAIT_BUDGET_MS);
        assert_eq!(wait_budget_ms(4000), MAX_WAIT_BUDGET_MS);
        // 100 ms of 10 ms intervals, the lead and the interval going on
        assert_eq!(pending_max(100, 10_000), 13);
        assert_eq!(pending_max(40, 7_500), 9);
    }

    #[test]
    fn every_cis_stalled_reports_the_stream_lost_once() {
        let out = Fake::default();
        let stats = Arc::new(Mutex::new(LeStreamStats::default()));
        let mut s = IsoSender::new(stats.clone());
        s.start(&out, &[0x60, 0x61], queue(2), 10_000, 4, 100);
        let mut bytes = Vec::new();
        for i in 1..=250u32 {
            at(&mut s, &out, i * 10);
            bytes.push(stats.lock().unwrap().bytes_sent);
            if i < 200 {
                assert!(s.take_stall().is_none(), "at {i}");
            }
        }
        // Counters published while nothing completes; nothing sent after
        // the queues filled (a live bit rate of 0)
        assert_eq!(bytes[100], bytes[249]);
        assert!(stats.lock().unwrap().late_sdus > 300);
        assert!(s.stats().late_sdus > 400);
        assert_eq!(s.take_stall().as_deref(), Some("no SDU completed on any CIS for 2 s"));
        at(&mut s, &out, 2600);
        assert!(s.take_stall().is_none());
    }

    #[test]
    fn a_send_a_millisecond_after_the_tick_is_no_stall() {
        let out = Fake::default();
        let stats = Arc::new(Mutex::new(LeStreamStats::default()));
        let mut s = IsoSender::new(stats.clone());
        s.start(&out, &[0x60], queue(1), 10_000, 4, 100);
        for i in 1..=300u32 {
            // Everything in the controller completed, then the clock moves
            // on between the tick's read and the send's
            let n = s.cis[0].in_flight.len() as u16;
            s.on_hci_event(&out, &completed(0x60, n));
            out.now.set(i * 10);
            out.step.set(1);
            s.on_timer(&out);
            assert!(!s.cis[0].stalled, "at {i}");
            assert!(s.take_stall().is_none(), "at {i}");
        }
        s.stop(&out);
    }

    #[test]
    fn a_refused_send_goes_again_with_the_same_sequence_number() {
        let out = Fake::default();
        let mut s = IsoSender::new(Arc::default());
        out.busy.set(1);
        s.start(&out, &[0x60, 0x61], queue(2), 10_000, 4, 100);
        // The first send found BTstack's packet buffer taken (a GATT write)
        assert_eq!(out.take(), ["queue 4", "request 60", "busy 60 #0", "timer 10"]);
        s.on_hci_event(&out, &PACKET_SENT);
        assert_eq!(out.sends(), ["send 60 #0 ee", "send 61 #0 ee", "send 60 #1 ee", "send 61 #1 ee"]);
        // The second CIS's turn is kept while the buffer is taken
        out.busy.set(1);
        at(&mut s, &out, 10);
        s.on_hci_event(&out, &completed(0x60, 2));
        assert_eq!(out.take(), ["busy 61 #2", "timer 10", "send 61 #2 ee", "send 60 #2 ee"]);
        assert_eq!((s.busy, s.counters.send_errors), (2, 0));
    }

    #[test]
    fn a_failed_send_is_dropped_and_counted() {
        struct Failing(Fake);
        impl IsoOut for Failing {
            fn time_ms(&self) -> u32 {
                self.0.time_ms()
            }
            fn set_queue(&self, _: u8) {}
            fn request_can_send(&self, _: u16) -> u8 {
                0
            }
            fn send(&self, cis: u16, seq: u16, sdu: &[u8]) -> u8 {
                self.0.send(cis, seq, sdu);
                if cis == 0x61 { 0x02 } else { 0 }
            }
            fn timer_start(&self, _: u32) {}
            fn timer_stop(&self) {}
        }
        let out = Failing(Fake::default());
        let mut s = IsoSender::new(Arc::default());
        s.start(&out, &[0x60, 0x61], queue(2), 10_000, 4, 100);
        assert_eq!(out.0.sends(), ["send 60 #0 ee", "send 61 #0 ee", "send 60 #1 ee", "send 61 #1 ee"]);
        assert_eq!((s.counters.send_errors, s.cis[1].late, s.cis[1].sent), (2, 2, 0));
    }

    #[test]
    fn request_refused_before_the_cig_is_active_is_asked_again() {
        let out = Fake::default();
        out.refuse.set(2);
        let mut s = IsoSender::new(Arc::new(Mutex::new(LeStreamStats::default())));
        s.start(&out, &[0x60], queue(1), 10_000, 2, 100);
        assert_eq!(out.take(), ["queue 2", "request 60", "timer 1"]);
        at(&mut s, &out, 1);
        assert_eq!(out.take(), ["request 60", "timer 1"]);
        // Active now: pacing starts from here
        at(&mut s, &out, 2);
        assert_eq!(out.take(), ["request 60", "send 60 #0 ee", "send 60 #1 ee", "timer 10"]);
        assert_eq!(s.cig_retries, 2);
    }

    #[test]
    fn a_cis_going_down_leaves_the_others_sending() {
        let out = Fake::default();
        let mut s = IsoSender::new(Arc::default());
        s.start(&out, &[0x60, 0x61], queue(2), 10_000, 4, 100);
        out.take();
        s.on_cis_down(&out, 0x61);
        at(&mut s, &out, 10);
        s.on_hci_event(&out, &completed(0x60, 1));
        assert_eq!(out.sends(), ["send 60 #2 ee"]);
        // A CIS that is down is not stalled either
        for i in 2..=300u32 {
            at(&mut s, &out, i * 10);
            s.on_hci_event(&out, &completed(0x60, 1));
        }
        assert!(s.take_stall().is_none());
        assert_eq!(s.stats().cis_stalled, 0);
    }

    #[test]
    fn health_line_and_completion_delays() {
        let mut c = CisOut::new(0x60, 0);
        for t in 0..10 {
            c.in_flight.push_back(t * 10);
            c.window_sent += 1;
        }
        c.window_peak = 10;
        for t in 0..10u32 {
            c.on_completed(1, t * 10 + 5 + t);
        }
        assert_eq!(c.completed, 10);
        let line = c.health(0.1, 10);
        assert_eq!(line, "CIS 0x0060: 100.0 SDUs/s, 0 late, completion p50 10 ms p90 14 ms, queue peak 10 of 10");
        assert_eq!(c.health(1.0, 10), "CIS 0x0060: 0.0 SDUs/s, 0 late, no completion, queue peak 0 of 10");
    }

    #[test]
    fn mediatek_queues_more_unless_debug_says_otherwise() {
        assert_eq!(depth_limit_for(false, 0), QUEUE_DEPTH);
        assert_eq!(depth_limit_for(false, 6), QUEUE_DEPTH);
        assert_eq!(depth_limit_for(true, 0), MEDIATEK_QUEUE_DEPTH);
        assert_eq!(depth_limit_for(true, 2), 2);
    }

    #[test]
    fn debug_depth_stays_within_the_controller_buffers() {
        // 24 buffers for 2 CIS: a limit of 6 is taken, 20 is cut to 12
        assert_eq!(queue_depth_within(24, 2, 6), 6);
        assert_eq!(queue_depth_within(24, 2, 20), 12);
        assert_eq!(queue_depth_within(24, 0, 6), 0);
        assert_eq!(queue_depth_within(64, 2, QUEUE_DEPTH), 2);
        assert_eq!(queue_depth_within(3, 2, QUEUE_DEPTH), 1);
        assert_eq!(queue_depth_within(1, 2, QUEUE_DEPTH), 0);
    }

    #[test]
    fn cis_depth_from_buffers_and_flush_timeout() {
        // FT 10 at a 10 ms ISO interval holds an SDU up to 100 ms: 10 + lead
        // 2 + 3, cut to the 10 buffers each of 2 CIS has of 20
        assert_eq!(depth_for_flush(10, 10_000, 10_000), 15);
        assert_eq!(cis_depth(queue_depth_within(20, 2, MAX_CIS_QUEUE_DEPTH), false, Some(15)), 10);
        // FT 2: 7 are enough
        assert_eq!(depth_for_flush(2, 10_000, 10_000), 7);
        assert_eq!(cis_depth(10, false, Some(7)), 7);
        // FT 1 at 7.5 ms
        assert_eq!(depth_for_flush(1, 7_500, 7_500), 6);
        assert_eq!(depth_for_flush(0, 7_500, 7_500), 5);
        assert_eq!(cis_depth(MIN_CIS_QUEUE_DEPTH, false, Some(5)), MIN_CIS_QUEUE_DEPTH);
        // A long flush timeout is capped
        assert_eq!(depth_for_flush(30, 20_000, 10_000), MAX_CIS_QUEUE_DEPTH);
        // Unknown flush timeout: the share; MediaTek's: the share as it is
        assert_eq!(cis_depth(10, false, None), 10);
        assert_eq!(cis_depth(12, true, Some(5)), 12);
        // 64 buffers for 2 CIS: 16 at most
        assert_eq!(queue_depth_within(64, 2, MAX_CIS_QUEUE_DEPTH), 16);
    }

    #[test]
    fn depth_follows_the_flush_timeout_of_cis_established() {
        let out = Fake::default();
        let mut s = IsoSender::new(Arc::default());
        // LE CIS Established v1: handle 0x60, NSE 2, BN 1/0, FT 10/10,
        // max PDU 155/0, ISO interval 8 x 1.25 ms
        let mut ev = vec![0x3E, 29, 0x19, 0x00, 0x60, 0x00];
        ev.extend_from_slice(&[0; 6]); // sync delays
        ev.extend_from_slice(&[0x40, 0x73, 0x01, 0x40, 0x73, 0x01]); // 95 ms both ways
        ev.extend_from_slice(&[2, 2, 2, 1, 0, 10, 10, 155, 0, 0, 0, 8, 0]);
        let e = events::le_cis_established(&ev).unwrap();
        assert_eq!((e.cis_handle, e.nse, e.ft_c_to_p, e.max_pdu_c_to_p, e.iso_interval), (0x60, 2, 10, 155, 8));
        assert_eq!(e.transport_latency_c_to_p, 95_040);
        s.on_hci_event(&out, &ev);
        s.start(&out, &[0x60], queue(1), 10_000, 16, 100);
        assert_eq!(s.depth, 15);
        s.stop(&out);
        // Without parameters: the share
        let mut s = IsoSender::new(Arc::default());
        s.start(&out, &[0x61], queue(1), 10_000, 16, 100);
        assert_eq!(s.depth, 16);
    }

    #[test]
    fn underrun_sends_silence() {
        let out = Fake::default();
        let mut s = IsoSender::new(Arc::default());
        let q = queue(1);
        s.start(&out, &[0x60], q.clone(), 10_000, 8, 100);
        out.take();
        for tag in 1..=5 {
            q.lock().unwrap().push(vec![vec![tag; 4]]);
        }
        for i in 1..=6u32 {
            at(&mut s, &out, i * 10);
        }
        assert_eq!(
            out.sends(),
            ["send 60 #2 1", "send 60 #3 2", "send 60 #4 3", "send 60 #5 4", "send 60 #6 5", "send 60 #7 ee"]
        );
        let st = s.stats();
        assert_eq!((st.underruns, st.silence_sets), (1, 3));
    }

    /// One BIG round of two BIS at `ms`; the slow report, if any
    fn round_at(s: &mut IsoSender, out: &Fake, ms: u32) -> Option<String> {
        out.now.set(ms);
        s.on_bis_can_send_now(out, &bis_can_send(0, 0, 0x10));
        s.on_bis_can_send_now(out, &bis_can_send(0, 1, 0x11));
        s.take_slow()
    }

    #[test]
    fn big_rounds_at_the_frame_rate_are_not_slow() {
        let out = Fake::default();
        let mut s = IsoSender::new(Arc::default());
        s.start_big(&out, 0, &[0x10, 0x11], queue(2), 10_000, 2);
        for i in 0..2000u32 {
            assert_eq!(round_at(&mut s, &out, i * 10), None, "at {i}");
        }
        s.stop(&out);
    }

    #[test]
    fn big_rounds_at_half_the_rate_are_reported_once_after_five_seconds() {
        let out = Fake::default();
        let mut s = IsoSender::new(Arc::default());
        s.start_big(&out, 0, &[0x10, 0x11], queue(2), 10_000, 2);
        let mut reports = Vec::new();
        for i in 0..1000u32 {
            if let Some(why) = round_at(&mut s, &out, i * 20) {
                reports.push((i * 20, why));
            }
        }
        assert_eq!(reports.len(), 1, "{reports:?}");
        let (at, why) = &reports[0];
        assert_eq!(*at, 5000);
        assert_eq!(why, "BIG rounds at 50 % of the frame rate for 5 s");
        // A new start reports again
        s.stop(&out);
        s.start_big(&out, 0, &[0x10, 0x11], queue(2), 10_000, 2);
        let n = (0..400u32).filter_map(|i| round_at(&mut s, &out, 30_000 + i * 20)).count();
        assert_eq!(n, 1);
        s.stop(&out);
    }

    #[test]
    fn a_few_slow_seconds_between_full_rate_ones_are_not_reported() {
        let out = Fake::default();
        let mut s = IsoSender::new(Arc::default());
        s.start_big(&out, 0, &[0x10, 0x11], queue(2), 10_000, 2);
        let mut t = 0;
        for _ in 0..5 {
            // 4 s at half the rate, then 1 s at the full rate
            for _ in 0..200 {
                assert_eq!(round_at(&mut s, &out, t), None, "at {t}");
                t += 20;
            }
            for _ in 0..100 {
                assert_eq!(round_at(&mut s, &out, t), None, "at {t}");
                t += 10;
            }
        }
        s.stop(&out);
    }
}
