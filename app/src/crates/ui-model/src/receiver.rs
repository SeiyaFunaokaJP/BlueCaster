//! The peer receiver test's logic, as v1's dialog (wx_receiver_dialog.cpp
//! at b819893): the six OK / NG steps of a measurement and the comparison of
//! what this PC sent with what the receiver (tools/linux_sink) counted. The
//! screen (the window's screens/receiver_test.rs) feeds it the core's ReceiverState, the
//! stream status and the last link sample; everything here is plain data so
//! the tests can drive it without a core.
//!
//! Times are milliseconds on the screen's own clock.

use std::ops::{Index, IndexMut};

use ipc::{
    LinkSample, LinkTotals, ReceivedStream, ReceiverConnection, ReceiverState, ReceiverStats, StreamInfo, StreamState,
    StreamStatus,
};

use crate::format::{kbps, khz};
use crate::i18n::t;
use crate::t;

/// The receiver reports every second; older data means it stopped
pub const STATS_STALE_MS: u64 = 3000;
/// A step not confirmed within this long after it started is NG
pub const CONFIGURE_TIMEOUT_MS: u64 = 5000;
pub const AUDIO_TIMEOUT_MS: u64 = 8000;
/// Log lines kept (v1 kept 64 KB)
pub const LOG_KEEP: usize = 1000;

/// Receiver L2CAP MTU: 0 = as the receiver was started; 672 = BlueZ default,
/// 679 = smallest LDAC accepts, 895 / 1005 = common in headphones
pub const MTUS: [u32; 6] = [0, 672, 679, 895, 1005, 2048];
pub const BUFFERS_MS: [u32; 5] = [0, 100, 200, 300, 500];

// ---------------------------------------------------------------------------
// Steps
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepId {
    Discover,
    Stats,
    Configure,
    Link,
    Codec,
    Audio,
}

pub const STEP_IDS: [StepId; 6] =
    [StepId::Discover, StepId::Stats, StepId::Configure, StepId::Link, StepId::Codec, StepId::Audio];

/// The language key of a step's name (STEP_KEYS)
pub fn step_key(id: StepId) -> &'static str {
    match id {
        StepId::Discover => "receiver.step_discover",
        StepId::Stats => "receiver.step_stats",
        StepId::Configure => "receiver.step_configure",
        StepId::Link => "receiver.step_link",
        StepId::Codec => "receiver.step_codec",
        StepId::Audio => "receiver.step_audio",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StepState {
    #[default]
    Pending,
    Running,
    Ok,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Step {
    pub state: StepState,
    /// What was found, or why it failed
    pub detail: String,
    /// ms after Start when it became OK / NG (measurement steps only)
    pub at: Option<u64>,
}

/// The six steps; indexed by [`StepId`]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Steps {
    pub discover: Step,
    pub stats: Step,
    pub configure: Step,
    pub link: Step,
    pub codec: Step,
    pub audio: Step,
}

impl Index<StepId> for Steps {
    type Output = Step;
    fn index(&self, id: StepId) -> &Step {
        match id {
            StepId::Discover => &self.discover,
            StepId::Stats => &self.stats,
            StepId::Configure => &self.configure,
            StepId::Link => &self.link,
            StepId::Codec => &self.codec,
            StepId::Audio => &self.audio,
        }
    }
}

impl IndexMut<StepId> for Steps {
    fn index_mut(&mut self, id: StepId) -> &mut Step {
        match id {
            StepId::Discover => &mut self.discover,
            StepId::Stats => &mut self.stats,
            StepId::Configure => &mut self.configure,
            StepId::Link => &mut self.link,
            StepId::Codec => &mut self.codec,
            StepId::Audio => &mut self.audio,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TestModel {
    pub steps: Steps,
    /// When Start was pressed (ms); None before the first measurement
    pub started_at: Option<u64>,
    /// Receiver settings asked for at Start (0: as it was started)
    pub mtu: u32,
    pub buffer_ms: u32,
    /// The receiver's stream when Start was pressed: a new one is ours
    pub stream_id_at_start: u32,
    pub had_stats_at_start: bool,
    /// When the Bluetooth connection came up (for the audio timeout)
    pub link_ok_at: Option<u64>,
    /// This screen started the stream and it has not ended
    pub measuring: bool,
    /// The stream left Idle since Start (Idle before that is not the end)
    pub left_idle: bool,
    /// Why the last stream this screen started ended
    pub failure: String,
    /// The core's status text last logged
    pub service_text: String,
    /// The last stream's info: its counts stay shown after it stopped
    pub last_info: Option<StreamInfo>,
}

pub fn new_model() -> TestModel {
    TestModel::default()
}

/// The receiver's latest report, if it is recent
pub fn fresh_stats(r: Option<&ReceiverState>) -> Option<&ReceiverStats> {
    let r = r?;
    let stats = r.stats.as_ref()?;
    match r.stats_age_ms {
        Some(age) if age < STATS_STALE_MS => Some(stats),
        _ => None,
    }
}

fn done(state: StepState) -> bool {
    state == StepState::Ok || state == StepState::Failed
}

/// Sets a step; a change to OK / NG goes to `log` ("OK Codec: SBC ...")
pub fn set_step(m: &mut TestModel, id: StepId, state: StepState, detail: &str, now: u64, log: &mut Vec<String>) {
    let started_at = m.started_at;
    let s = &mut m.steps[id];
    let changed = s.state != state;
    s.state = state;
    s.detail = detail.to_owned();
    if !changed {
        return;
    }
    let measurement = id != StepId::Discover && id != StepId::Stats;
    s.at = match started_at {
        Some(start) if done(state) && measurement => Some(now.saturating_sub(start)),
        _ => None,
    };
    if done(state) {
        let mark = if state == StepState::Ok { "OK" } else { "NG" };
        let detail = if detail.is_empty() { String::new() } else { format!(": {detail}") };
        log.push(format!("{mark} {}{detail}", t(step_key(id))));
    }
}

/// What [`start_measurement`] takes
#[derive(Debug, Clone, Copy)]
pub struct StartOptions<'a> {
    pub receiver: Option<&'a ReceiverState>,
    pub mtu: u32,
    pub buffer_ms: u32,
    pub now: u64,
}

/// Start: the measurement steps from the beginning; configure and link run
pub fn start_measurement(m: &mut TestModel, o: &StartOptions) -> Vec<String> {
    let mut log = Vec::new();
    for id in [StepId::Configure, StepId::Link, StepId::Codec, StepId::Audio] {
        m.steps[id] = Step::default();
    }
    let s = fresh_stats(o.receiver);
    m.started_at = Some(o.now);
    m.mtu = o.mtu;
    m.buffer_ms = o.buffer_ms;
    m.had_stats_at_start = s.is_some();
    m.stream_id_at_start = s.and_then(|s| s.stream.as_ref()).map_or(0, |s| s.id);
    m.link_ok_at = None;
    m.measuring = true;
    m.left_idle = false;
    m.failure = String::new();
    m.service_text = String::new();
    m.last_info = None;
    set_step(m, StepId::Configure, StepState::Running, "", o.now, &mut log);
    set_step(m, StepId::Link, StepState::Running, "", o.now, &mut log);
    log
}

/// Stop pressed (the stream is stopped by the caller)
pub fn stop_measurement(m: &mut TestModel) {
    m.measuring = false;
}

/// The stream could not be started at all (the request failed)
pub fn start_failed(m: &mut TestModel, text: &str, now: u64) -> Vec<String> {
    let mut log = Vec::new();
    m.measuring = false;
    m.failure = text.to_owned();
    if m.steps.link.state == StepState::Running {
        set_step(m, StepId::Link, StepState::Failed, text, now, &mut log);
    }
    log
}

/// The receiver did not take the settings (receiver.configure failed)
pub fn configure_failed(m: &mut TestModel, text: &str, now: u64) -> Vec<String> {
    let mut log = Vec::new();
    if m.steps.configure.state == StepState::Running {
        set_step(m, StepId::Configure, StepState::Failed, text, now, &mut log);
    }
    log
}

/// What [`update`] takes
#[derive(Debug, Clone, Copy)]
pub struct Inputs<'a> {
    pub receiver: Option<&'a ReceiverState>,
    pub stream: &'a StreamStatus,
    /// The stream's status text, translated
    pub service_text: &'a str,
    pub now: u64,
}

/// One refresh (v1's refresh + refresh_steps): returns the new log lines
pub fn update(m: &mut TestModel, i: &Inputs) -> Vec<String> {
    let mut log = Vec::new();
    let log = &mut log;
    let now = i.now;
    let state = i.stream.state;
    if state == StreamState::Streaming
        && let Some(info) = &i.stream.info
    {
        m.last_info = Some(info.clone());
    }

    // Whether the stream this screen started has ended
    if m.measuring && state != StreamState::Idle {
        m.left_idle = true;
    }
    if m.measuring && (state == StreamState::Error || (state == StreamState::Idle && m.left_idle)) {
        m.measuring = false;
        if state == StreamState::Error {
            m.failure = i.service_text.to_owned();
        }
    }

    // 1-2: finding the receiver and its statistics
    let r = i.receiver;
    let conn = r.map_or(ReceiverConnection::Off, |r| r.connection);
    let info = r.and_then(|r| r.receiver.as_ref());
    let stats = fresh_stats(r);
    let automatic = r.is_some_and(|r| r.target == "auto");
    let r_detail = r.map_or("", |r| r.detail.as_str());
    match conn {
        ReceiverConnection::Off => {
            set_step(m, StepId::Discover, StepState::Pending, "", now, log);
            set_step(m, StepId::Stats, StepState::Pending, "", now, log);
        }
        ReceiverConnection::Connecting => {
            if info.is_none() {
                let discover = if automatic { StepState::Running } else { StepState::Pending };
                set_step(m, StepId::Discover, discover, "", now, log);
            }
            set_step(m, StepId::Stats, StepState::Running, "", now, log);
        }
        ReceiverConnection::Failed => {
            if info.is_none() && automatic {
                set_step(m, StepId::Discover, StepState::Failed, r_detail, now, log);
            } else {
                set_step(m, StepId::Stats, StepState::Failed, r_detail, now, log);
            }
        }
        ReceiverConnection::Connected => {
            if let Some(info) = info {
                let detail = format!("{} ({}), Bluetooth {}", info.host, info.ip, info.bt_address);
                set_step(m, StepId::Discover, StepState::Ok, &detail, now, log);
            }
            if stats.is_some() {
                set_step(m, StepId::Stats, StepState::Ok, "", now, log);
            }
        }
    }

    let Some(started_at) = m.started_at else { return std::mem::take(log) };
    let elapsed = now.saturating_sub(started_at);

    // 3: the receiver took the settings (it reports them in its statistics)
    if m.steps.configure.state == StepState::Running {
        let mtu_ok = m.mtu == 0 || stats.is_some_and(|s| s.mtu == m.mtu);
        let buffer_ok = m.buffer_ms == 0 || stats.is_some_and(|s| s.buffer_target_ms == m.buffer_ms);
        if let Some(stats) = stats
            && mtu_ok
            && buffer_ok
        {
            let detail = format!("MTU {}, {} ms", stats.mtu, stats.buffer_target_ms);
            set_step(m, StepId::Configure, StepState::Ok, &detail, now, log);
        } else if elapsed > CONFIGURE_TIMEOUT_MS {
            set_step(m, StepId::Configure, StepState::Failed, &t("receiver.log_configure_failed"), now, log);
        }
    }

    // 4: this PC's own connection, with its status messages in the log
    if m.measuring && m.left_idle && i.service_text != m.service_text {
        m.service_text = i.service_text.to_owned();
        if !i.service_text.is_empty() {
            log.push(format!("BlueCaster: {}", i.service_text));
        }
    }
    let link = m.steps.link.state;
    if link == StepState::Running {
        if state == StreamState::Streaming {
            set_step(m, StepId::Link, StepState::Ok, "", now, log);
            m.link_ok_at = Some(now);
            set_step(m, StepId::Codec, StepState::Running, "", now, log);
            set_step(m, StepId::Audio, StepState::Running, "", now, log);
        } else if state == StreamState::Error || (!m.measuring && m.left_idle && state == StreamState::Idle) {
            set_step(m, StepId::Link, StepState::Failed, i.service_text, now, log);
        }
    } else if link == StepState::Ok && state == StreamState::Reconnecting {
        set_step(m, StepId::Link, StepState::Running, "", now, log);
        log.push(t("receiver.log_link_lost"));
    }

    // 5-6: what the receiver sees of it
    let s = stats.and_then(|st| st.stream.as_ref());
    let new_stream = s.filter(|s| s.id != m.stream_id_at_start || !m.had_stats_at_start);
    let timed_out = m.link_ok_at.is_some_and(|at| now.saturating_sub(at) > AUDIO_TIMEOUT_MS);
    if m.steps.codec.state == StepState::Running
        && let Some(s) = new_stream
    {
        set_step(m, StepId::Codec, StepState::Ok, &s.config, now, log);
    }
    if m.steps.audio.state == StepState::Running {
        let arriving = new_stream.filter(|s| stats.is_some_and(|st| st.streaming) && s.interval_packets > 0);
        if let Some(s) = arriving {
            let detail = t!("receiver.bitrate_value", (s.interval_bytes as f64 * 8.0) / 1000.0, s.interval_packets);
            set_step(m, StepId::Audio, StepState::Ok, &detail, now, log);
        } else if timed_out {
            set_step(m, StepId::Audio, StepState::Failed, &t("receiver.log_no_audio"), now, log);
        }
    }
    if m.steps.codec.state == StepState::Running && timed_out {
        set_step(m, StepId::Codec, StepState::Failed, &t("receiver.log_no_audio"), now, log);
    }
    std::mem::take(log)
}

// ---------------------------------------------------------------------------
// Status line
// ---------------------------------------------------------------------------

/// What [`status_text`] returns
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusText {
    pub lines: Vec<String>,
    pub bad: bool,
}

/// The text under the address (v1's refresh_status): the receiver, then what happens
pub fn status_text(m: &TestModel, r: Option<&ReceiverState>, stream: &StreamStatus) -> StatusText {
    let conn = r.map_or(ReceiverConnection::Off, |r| r.connection);
    let info = if conn != ReceiverConnection::Off { r.and_then(|r| r.receiver.as_ref()) } else { None };
    let mut lines = Vec::new();
    if let Some(info) = info {
        lines.push(t!("receiver.found", &info.host, &info.ip, &info.bt_address, info.mtu));
    }
    let mut bad = false;
    match conn {
        ReceiverConnection::Off => lines.push(t("receiver.off")),
        ReceiverConnection::Connecting => {
            let auto = r.is_some_and(|r| r.target == "auto");
            lines.push(t(if auto { "receiver.searching" } else { "receiver.connecting" }));
        }
        ReceiverConnection::Failed => {
            lines.push(t!("receiver.failed", r.map_or("", |r| r.detail.as_str())));
            bad = true;
        }
        ReceiverConnection::Connected => {
            let stats = fresh_stats(r);
            if stats.is_some_and(|s| s.streaming) {
                lines.push(t("receiver.state_receiving"));
            } else if m.measuring && matches!(stream.state, StreamState::Connecting | StreamState::Reconnecting) {
                lines.push(t("receiver.state_connecting"));
            } else if m.measuring {
                lines.push(t("receiver.state_waiting"));
            } else if !m.failure.is_empty() {
                lines.push(t!("receiver.state_failed", &m.failure));
                bad = true;
            } else {
                lines.push(t("receiver.state_ready"));
            }
        }
    }
    StatusText { lines, bad }
}

/// Start is possible: a receiver with a Bluetooth address is known
pub fn can_start(r: Option<&ReceiverState>) -> bool {
    r.is_some_and(|r| {
        r.connection != ReceiverConnection::Off && r.receiver.as_ref().is_some_and(|i| !i.bt_address.is_empty())
    })
}

// ---------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowId {
    Codec,
    Bitrate,
    Total,
    Loss,
    Timing,
    Buffer,
    Audio,
    Errors,
    Signal,
}

pub const ROW_IDS: [RowId; 9] = [
    RowId::Codec,
    RowId::Bitrate,
    RowId::Total,
    RowId::Loss,
    RowId::Timing,
    RowId::Buffer,
    RowId::Audio,
    RowId::Errors,
    RowId::Signal,
];

/// The language key of a row's name (ROW_KEYS)
pub fn row_key(id: RowId) -> &'static str {
    match id {
        RowId::Codec => "receiver.row_codec",
        RowId::Bitrate => "receiver.row_bitrate",
        RowId::Total => "receiver.row_total",
        RowId::Loss => "receiver.row_loss",
        RowId::Timing => "receiver.row_timing",
        RowId::Buffer => "receiver.row_buffer",
        RowId::Audio => "receiver.row_audio",
        RowId::Errors => "receiver.row_errors",
        RowId::Signal => "receiver.row_signal",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub text: String,
    pub bad: bool,
}

impl Cell {
    fn new(text: String) -> Cell {
        Cell { text, bad: false }
    }

    fn marked(text: String, bad: bool) -> Cell {
        Cell { text, bad }
    }
}

/// "-"
pub fn dash() -> Cell {
    Cell::new("-".to_owned())
}

impl Default for Cell {
    fn default() -> Cell {
        dash()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Row {
    pub sent: Cell,
    pub received: Cell,
    pub diff: Cell,
}

/// The comparison table's rows; indexed by [`RowId`]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Rows {
    pub codec: Row,
    pub bitrate: Row,
    pub total: Row,
    pub loss: Row,
    pub timing: Row,
    pub buffer: Row,
    pub audio: Row,
    pub errors: Row,
    pub signal: Row,
}

impl Index<RowId> for Rows {
    type Output = Row;
    fn index(&self, id: RowId) -> &Row {
        match id {
            RowId::Codec => &self.codec,
            RowId::Bitrate => &self.bitrate,
            RowId::Total => &self.total,
            RowId::Loss => &self.loss,
            RowId::Timing => &self.timing,
            RowId::Buffer => &self.buffer,
            RowId::Audio => &self.audio,
            RowId::Errors => &self.errors,
            RowId::Signal => &self.signal,
        }
    }
}

/// Counts start with each stream and at Reset: what Reset took
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Baseline {
    /// ReceiverState.stream_starts at Reset (None: no Reset); another stream
    /// starts from zero
    pub sent_starts: Option<u32>,
    /// `sent` at Reset
    pub sent: Option<LinkTotals>,
    /// The receiver's stream and its counters at Reset
    pub received: Option<ReceivedStream>,
}

pub fn no_baseline() -> Baseline {
    Baseline::default()
}

/// Reset: the counts from now on
pub fn take_baseline(r: Option<&ReceiverState>) -> Baseline {
    let s = fresh_stats(r).and_then(|st| st.stream.clone());
    Baseline { sent_starts: r.map(|r| r.stream_starts), sent: r.and_then(|r| r.sent), received: s }
}

pub fn pct(part: f64, whole: f64) -> f64 {
    if whole > 0.0 { (100.0 * part) / whole } else { 0.0 }
}

/// "RSSI +0 dB vs. golden range, AFH 45/79 ch (avoiding Wi-Fi 6, 11)"
pub fn radio_text(rssi: Option<i32>, afh: Option<u32>, avoided: &str) -> String {
    let mut text = rssi.map_or_else(String::new, |r| t!("radio.rssi", r));
    if let Some(afh) = afh {
        if !text.is_empty() {
            text += &t("radio.separator");
        }
        text += &t!("radio.afh", afh);
    }
    if !avoided.is_empty() && !text.is_empty() {
        text += &t!("radio.afh_avoid", avoided);
    }
    if text.is_empty() { "-".to_owned() } else { text }
}

/// `a - b`, never below 0 (b absent: 0)
fn sub(a: u64, b: Option<u64>) -> u64 {
    a.saturating_sub(b.unwrap_or(0))
}

/// What this PC sent since the stream started or Reset
pub fn sent_since(r: Option<&ReceiverState>, base: &Baseline) -> Option<LinkTotals> {
    let r = r?;
    let now = r.sent.as_ref()?;
    let b = base.sent.filter(|_| base.sent_starts == Some(r.stream_starts)).unwrap_or_default();
    Some(now.since(&b))
}

fn norm_codec(name: &str) -> String {
    name.to_lowercase().chars().filter(|c| !c.is_whitespace() && *c != '_' && *c != '-').collect()
}

/// What [`compare`] takes
#[derive(Debug, Clone, Copy)]
pub struct CompareInputs<'a> {
    pub receiver: Option<&'a ReceiverState>,
    pub stream: &'a StreamStatus,
    /// The last link sample (rate, queue, radio of this PC)
    pub link: Option<&'a LinkSample>,
    pub base: &'a Baseline,
    /// The last stream's info (kept after it stopped)
    pub last_info: Option<&'a StreamInfo>,
}

/// The comparison table: sent | received | difference, per row
pub fn compare(i: &CompareInputs) -> Rows {
    let mut rows = Rows::default();
    let stream_info = i.stream.info.as_ref().filter(|_| i.stream.state == StreamState::Streaming);
    let streaming = stream_info.is_some();
    let sent = sent_since(i.receiver, i.base);

    // ---- Sent (this PC) ----
    let mut sent_kbps: Option<f64> = None;
    let sent_info = if streaming { stream_info } else { i.last_info };
    if let Some(info) = sent_info {
        let codec = format!("{}, {} kHz", info.codec, khz(info.sample_rate));
        rows.codec.sent = Cell::new(if streaming { codec } else { t!("receiver.final_value", codec) });
        if let Some(sent) = &sent {
            rows.total.sent = Cell::new(sent.packets.to_string());
            rows.loss.sent = Cell::marked(
                t!(
                    "receiver.dropped_value",
                    sent.dropped,
                    pct(sent.dropped as f64, (sent.packets + sent.dropped) as f64)
                ),
                sent.dropped > 0,
            );
        }
    }
    if let Some(info) = stream_info {
        let l = i.link;
        if let Some(l) = l.filter(|l| l.packets_per_second > 0.0) {
            let k = kbps(l.bytes_per_second);
            sent_kbps = Some(k);
            rows.bitrate.sent = Cell::new(t!("receiver.bitrate_value", k, l.packets_per_second));
        }
        if let Some(l) = l {
            rows.buffer.sent = Cell::new(t!("receiver.queue_value", l.queue_depth, l.queue_capacity.unwrap_or(0)));
            rows.signal.sent = Cell::new(radio_text(l.rssi.map(i32::from), l.afh_in_use, &l.afh_avoided));
        }
        if let Some(sent) = &sent {
            let link_rate = l.and_then(|l| l.source_rate).unwrap_or(0);
            let rate = if info.source_sample_rate != 0 { info.source_sample_rate } else { link_rate };
            let gaps = if rate > 0 { (1000.0 * sent.capture_dropped as f64) / rate as f64 } else { 0.0 };
            rows.audio.sent = Cell::marked(t!("receiver.capture_value", gaps), sent.capture_dropped > 0);
        }
    }

    // ---- Received (the receiver) ----
    let stats = fresh_stats(i.receiver);
    let s = stats.and_then(|st| st.stream.as_ref());
    let (Some(stats), Some(s)) = (stats, s) else {
        if let Some(stats) = stats
            && stats.connected
            && let Some(rssi) = stats.rssi
        {
            rows.signal.received = Cell::new(t!("receiver.rssi_value", rssi));
        }
        return rows;
    };
    let b = i.base.received.as_ref().filter(|b| b.id == s.id);
    let packets = sub(s.packets, b.map(|b| b.packets));
    let lost = sub(s.lost, b.map(|b| b.lost));
    let late = sub(s.late, b.map(|b| b.late));
    let underruns = sub(s.underruns, b.map(|b| b.underruns));
    let overflows = sub(s.overflows, b.map(|b| b.overflows));
    let pauses = sub(s.pauses, b.map(|b| b.pauses));
    let errors = sub(s.frame_errors, b.map(|b| b.frame_errors))
        + sub(s.decode_errors, b.map(|b| b.decode_errors))
        + sub(s.ts_errors, b.map(|b| b.ts_errors));
    rows.codec.received =
        Cell::new(if stats.streaming { s.config.clone() } else { t!("receiver.final_value", &s.config) });
    let mut recv_kbps: Option<f64> = None;
    if stats.streaming && s.interval_packets > 0 {
        let k = (s.interval_bytes as f64 * 8.0) / 1000.0;
        recv_kbps = Some(k);
        rows.bitrate.received = Cell::new(t!("receiver.bitrate_value", k, s.interval_packets));
        rows.timing.received = Cell::new(t!("receiver.timing_value", s.jitter_ms, s.interval_max_gap_ms));
    }
    rows.total.received = Cell::new(packets.to_string());
    rows.loss.received = Cell::marked(
        t!("receiver.lost_value", lost, pct(lost as f64, (packets + lost) as f64), late),
        lost > 0 || late > 0,
    );
    rows.buffer.received = match s.buffer_ms {
        None => dash(),
        Some(buffer) => Cell::new(t!("receiver.buffer_value", buffer, stats.buffer_target_ms)),
    };
    let underrun_ms = (s.underrun_ms - b.map_or(0.0, |b| b.underrun_ms)).max(0.0);
    let mut audio = t!("receiver.dropouts_value", underruns, underrun_ms, overflows);
    if pauses > 0 {
        audio += &t!("receiver.pauses_value", pauses);
    }
    if s.idle {
        audio = t("receiver.idle_value") + &audio;
    }
    rows.audio.received = Cell::marked(audio, underruns > 0 || overflows > 0);
    let errors_key = if s.decoding { "receiver.errors_value" } else { "receiver.errors_value_headers" };
    rows.errors.received = Cell::marked(t!(errors_key, errors), errors > 0);
    rows.signal.received = if stats.rssi.is_some() || stats.afh_channels.is_some() {
        Cell::new(radio_text(stats.rssi, stats.afh_channels, ""))
    } else {
        Cell::new(if stats.rssi_note.is_empty() { "-".to_owned() } else { stats.rssi_note.clone() })
    };

    // ---- Differences ----
    if let Some(info) = sent_info {
        let same = norm_codec(&info.codec) == norm_codec(&s.codec);
        rows.codec.diff = if same {
            Cell::new(t("ui.receiver.codec_same"))
        } else {
            Cell::marked(t("ui.receiver.codec_differs"), true)
        };
    }
    if let (Some(sent_kbps), Some(recv_kbps)) = (sent_kbps, recv_kbps) {
        rows.bitrate.diff = Cell::new(t!("ui.receiver.diff_kbps", recv_kbps - sent_kbps));
    }
    if let Some(sent) = &sent {
        // While streaming, the packets on their way (up to a second's worth)
        // are among the missing ones; at the end it is the loss
        let missing = sent.packets as i64 - packets as i64;
        rows.total.diff = if missing < 0 {
            dash()
        } else {
            Cell::marked(
                t!("ui.receiver.diff_missing", missing, pct(missing as f64, sent.packets as f64)),
                !streaming && missing > 0,
            )
        };
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipc::{Msg, ReceiverInfo};

    fn info() -> ReceiverInfo {
        ReceiverInfo {
            ip: "192.168.1.42".to_owned(),
            port: 51201,
            host: "sink-pi".to_owned(),
            bt_address: "DC:A6:32:01:02:03".to_owned(),
            bt_name: "bluecaster-sink".to_owned(),
            codecs: vec!["SBC".to_owned(), "LDAC".to_owned()],
            mtu: 1005,
            buffer_ms: 200,
        }
    }

    fn sbc() -> StreamInfo {
        StreamInfo {
            codec: "SBC".to_owned(),
            bitrate_kbps: 328,
            sample_rate: 48000,
            channels: 2,
            bits: 16,
            source_sample_rate: 48000,
            source_channels: 2,
            source_bits: 32,
            requested_codec: None,
            requested_in_use: false,
            abr: false,
            bitrate_capped: false,
            hi_res_set: false,
            low_latency: false,
            source: None,
            le: None,
            details: None,
        }
    }

    fn recv(id: u32, f: impl FnOnce(&mut ReceivedStream)) -> ReceivedStream {
        let mut s = ReceivedStream {
            id,
            codec: "SBC".to_owned(),
            config: "SBC 48000 Hz, joint stereo".to_owned(),
            decoding: true,
            jitter_ms: 1.5,
            buffer_ms: Some(190.0),
            interval_max_gap_ms: 12.0,
            ..Default::default()
        };
        f(&mut s);
        s
    }

    /// A test's state: the stream, streaming, mtu, buffer and the rest
    #[derive(Default)]
    struct St {
        stream: Option<ReceivedStream>,
        streaming: bool,
        mtu: Option<u32>,
        buffer: Option<u32>,
        sent: Option<LinkTotals>,
        stream_starts: u32,
    }

    fn state(o: St) -> ReceiverState {
        let streaming = o.streaming;
        ReceiverState {
            connection: ReceiverConnection::Connected,
            detail: String::new(),
            target: "auto".to_owned(),
            receiver: Some(info()),
            stats: Some(ReceiverStats {
                connected: streaming,
                streaming,
                device: String::new(),
                buffer_target_ms: o.buffer.unwrap_or(200),
                mtu: o.mtu.unwrap_or(1005),
                codecs: info().codecs,
                rssi: if streaming { Some(-3) } else { None },
                tx_power: None,
                rssi_note: if streaming { String::new() } else { "no connection".to_owned() },
                afh_channels: if streaming { Some(60) } else { None },
                stream: o.stream,
            }),
            stats_age_ms: Some(300),
            sent: o.sent,
            stream_starts: o.stream_starts,
        }
    }

    fn plain() -> ReceiverState {
        state(St::default())
    }

    fn stream(s: StreamState, info: Option<StreamInfo>, key: &str) -> StreamStatus {
        StreamStatus { state: s, message: Msg::new(key), info, ..Default::default() }
    }

    fn idle() -> StreamStatus {
        stream(StreamState::Idle, None, "status.idle")
    }

    fn streaming() -> StreamStatus {
        stream(StreamState::Streaming, Some(sbc()), "status.streaming")
    }

    fn run(m: &mut TestModel, r: Option<&ReceiverState>, s: &StreamStatus, now: u64) -> Vec<String> {
        update(m, &Inputs { receiver: r, stream: s, service_text: &s.message.key, now })
    }

    fn start(m: &mut TestModel, r: &ReceiverState, mtu: u32, buffer_ms: u32, now: u64) -> Vec<String> {
        start_measurement(m, &StartOptions { receiver: Some(r), mtu, buffer_ms, now })
    }

    fn step(state: StepState, detail: &str) -> (StepState, String) {
        (state, detail.to_owned())
    }

    fn of(s: &Step) -> (StepState, String) {
        (s.state, s.detail.clone())
    }

    // ---- receiver steps ----

    #[test]
    fn finding_the_receiver_and_its_statistics() {
        let mut m = new_model();
        let searching = ReceiverState {
            connection: ReceiverConnection::Connecting,
            receiver: None,
            stats: None,
            stats_age_ms: None,
            ..plain()
        };
        run(&mut m, Some(&searching), &idle(), 0);
        assert_eq!(m.steps.discover.state, StepState::Running);
        assert_eq!(m.steps.stats.state, StepState::Running);

        // An address given: nothing to find
        let mut m2 = new_model();
        let given = ReceiverState {
            target: "sink-pi".to_owned(),
            connection: ReceiverConnection::Connecting,
            receiver: None,
            stats: None,
            ..plain()
        };
        run(&mut m2, Some(&given), &idle(), 0);
        assert_eq!(m2.steps.discover.state, StepState::Pending);

        let log = run(&mut m, Some(&plain()), &idle(), 100);
        assert_eq!(of(&m.steps.discover), step(StepState::Ok, "sink-pi (192.168.1.42), Bluetooth DC:A6:32:01:02:03"));
        assert_eq!(m.steps.stats.state, StepState::Ok);
        assert_eq!(log, ["OK Find receiver: sink-pi (192.168.1.42), Bluetooth DC:A6:32:01:02:03", "OK Statistics"]);
        // No change, no new lines
        assert!(run(&mut m, Some(&plain()), &idle(), 200).is_empty());
    }

    #[test]
    fn not_found_on_the_network_or_not_reachable() {
        let mut m = new_model();
        let none = ReceiverState {
            connection: ReceiverConnection::Failed,
            detail: "no receiver found".to_owned(),
            receiver: None,
            stats: None,
            ..plain()
        };
        let log = run(&mut m, Some(&none), &idle(), 0);
        assert_eq!(of(&m.steps.discover), step(StepState::Failed, "no receiver found"));
        assert_eq!(log, ["NG Find receiver: no receiver found"]);

        let mut m2 = new_model();
        let refused = ReceiverState {
            target: "10.0.0.9".to_owned(),
            connection: ReceiverConnection::Failed,
            detail: "connection refused".to_owned(),
            receiver: None,
            stats: None,
            ..plain()
        };
        run(&mut m2, Some(&refused), &idle(), 0);
        assert_eq!(m2.steps.discover.state, StepState::Pending);
        assert_eq!(of(&m2.steps.stats), step(StepState::Failed, "connection refused"));
    }

    #[test]
    fn stale_statistics_are_not_counted() {
        let mut m = new_model();
        run(&mut m, Some(&ReceiverState { stats_age_ms: Some(5000), ..plain() }), &idle(), 0);
        assert_eq!(m.steps.discover.state, StepState::Ok);
        assert_eq!(m.steps.stats.state, StepState::Pending);
    }

    #[test]
    fn a_measurement_from_start_to_audio() {
        let mut m = new_model();
        let before = state(St { stream: Some(recv(4, |s| s.packets = 900)), ..Default::default() });
        run(&mut m, Some(&before), &idle(), 0);
        let start_log = start(&mut m, &before, 679, 300, 1000);
        assert!(start_log.is_empty());
        assert_eq!(m.steps.configure.state, StepState::Running);
        assert_eq!(m.steps.link.state, StepState::Running);
        assert!(m.measuring);

        // The receiver has not taken the settings yet
        let connecting = stream(StreamState::Connecting, None, "status.connecting_device");
        let log = run(&mut m, Some(&before), &connecting, 1200);
        assert_eq!(m.steps.configure.state, StepState::Running);
        // The core's status messages go to the log once each
        assert_eq!(log, ["BlueCaster: status.connecting_device"]);
        let configured = state(St {
            stream: Some(recv(4, |s| s.packets = 900)),
            mtu: Some(679),
            buffer: Some(300),
            ..Default::default()
        });
        let log = run(&mut m, Some(&configured), &connecting, 1500);
        assert_eq!(of(&m.steps.configure), step(StepState::Ok, "MTU 679, 300 ms"));
        assert_eq!(m.steps.configure.at, Some(500));
        assert_eq!(log, ["OK Receiver settings: MTU 679, 300 ms"]);

        let log = run(&mut m, Some(&configured), &streaming(), 2000);
        assert_eq!(m.steps.link.state, StepState::Ok);
        assert_eq!(m.steps.codec.state, StepState::Running);
        assert_eq!(m.steps.audio.state, StepState::Running);
        assert!(log.contains(&"OK Bluetooth connection".to_owned()));

        // Still the old stream at the receiver: not ours
        run(&mut m, Some(&configured), &streaming(), 2500);
        assert_eq!(m.steps.codec.state, StepState::Running);

        let arriving = state(St {
            streaming: true,
            mtu: Some(679),
            buffer: Some(300),
            stream: Some(recv(5, |s| {
                s.packets = 120;
                s.interval_packets = 120;
                s.interval_bytes = 41000;
            })),
            ..Default::default()
        });
        let log = run(&mut m, Some(&arriving), &streaming(), 3000);
        assert_eq!(of(&m.steps.codec), step(StepState::Ok, "SBC 48000 Hz, joint stereo"));
        assert_eq!(of(&m.steps.audio), step(StepState::Ok, "328 kbps, 120 packets/s"));
        assert_eq!(log, ["OK Codec: SBC 48000 Hz, joint stereo", "OK Audio arrives: 328 kbps, 120 packets/s"]);
        assert_eq!(
            status_text(&m, Some(&arriving), &streaming()),
            StatusText {
                lines: vec![
                    "sink-pi (192.168.1.42), Bluetooth DC:A6:32:01:02:03, L2CAP MTU 1005".to_owned(),
                    "Measuring.".to_owned(),
                ],
                bad: false,
            }
        );

        // Stop
        stop_measurement(&mut m);
        run(&mut m, Some(&arriving), &idle(), 4000);
        assert!(!m.measuring);
        assert_eq!(
            status_text(&m, Some(&plain()), &idle()).lines[1],
            "Ready. Click Start to stream to the receiver's Bluetooth adapter."
        );
    }

    #[test]
    fn the_receiver_does_not_confirm_the_settings() {
        let mut m = new_model();
        let r = plain();
        run(&mut m, Some(&r), &idle(), 0);
        start(&mut m, &r, 679, 0, 0);
        run(&mut m, Some(&r), &idle(), CONFIGURE_TIMEOUT_MS - 1);
        assert_eq!(m.steps.configure.state, StepState::Running);
        let log = run(&mut m, Some(&r), &idle(), CONFIGURE_TIMEOUT_MS + 1);
        assert_eq!(m.steps.configure.state, StepState::Failed);
        assert!(log[0].starts_with("NG Receiver settings: the receiver did not confirm"), "{}", log[0]);
    }

    #[test]
    fn default_settings_are_confirmed_by_any_report() {
        let mut m = new_model();
        let r = state(St { mtu: Some(895), buffer: Some(150), ..Default::default() });
        start(&mut m, &r, 0, 0, 0);
        run(&mut m, Some(&r), &idle(), 10);
        assert_eq!(of(&m.steps.configure), step(StepState::Ok, "MTU 895, 150 ms"));
    }

    #[test]
    fn nothing_is_arriving_at_the_receiver() {
        let mut m = new_model();
        let r = plain();
        start(&mut m, &r, 0, 0, 0);
        run(&mut m, Some(&r), &stream(StreamState::Connecting, None, "status.idle"), 100);
        run(&mut m, Some(&r), &streaming(), 1000);
        run(&mut m, Some(&r), &streaming(), 1000 + AUDIO_TIMEOUT_MS);
        assert_eq!(m.steps.audio.state, StepState::Running);
        let log = run(&mut m, Some(&r), &streaming(), 1001 + AUDIO_TIMEOUT_MS);
        assert_eq!(of(&m.steps.audio), step(StepState::Failed, "nothing is arriving at the receiver"));
        assert_eq!(m.steps.codec.state, StepState::Failed);
        assert_eq!(log.len(), 2);
    }

    #[test]
    fn the_stream_fails_or_the_connection_is_lost() {
        let mut m = new_model();
        let r = plain();
        start(&mut m, &r, 0, 0, 0);
        run(&mut m, Some(&r), &stream(StreamState::Connecting, None, "status.idle"), 100);
        let err = stream(StreamState::Error, None, "error.connect_failed");
        let log = run(&mut m, Some(&r), &err, 200);
        assert_eq!(of(&m.steps.link), step(StepState::Failed, "error.connect_failed"));
        assert!(!m.measuring);
        assert_eq!(m.failure, "error.connect_failed");
        assert!(log.contains(&"NG Bluetooth connection: error.connect_failed".to_owned()));
        assert!(status_text(&m, Some(&r), &err).bad);
        let line = &status_text(&m, Some(&r), &err).lines[1];
        assert!(line.starts_with("Sending failed: error.connect_failed"), "{line}");

        // Lost while streaming: running again until it is back
        let mut m2 = new_model();
        start(&mut m2, &r, 0, 0, 0);
        run(&mut m2, Some(&r), &streaming(), 100);
        let lost = run(&mut m2, Some(&r), &stream(StreamState::Reconnecting, None, "status.connection_lost"), 200);
        assert_eq!(m2.steps.link.state, StepState::Running);
        assert!(lost.contains(&"Bluetooth connection lost, reconnecting".to_owned()));
        run(&mut m2, Some(&r), &streaming(), 300);
        assert_eq!(m2.steps.link.state, StepState::Ok);
    }

    #[test]
    fn the_stream_request_is_refused() {
        let mut m = new_model();
        start(&mut m, &plain(), 0, 0, 0);
        let log = start_failed(&mut m, "No adapter", 50);
        assert_eq!(of(&m.steps.link), step(StepState::Failed, "No adapter"));
        assert_eq!(m.steps.link.at, Some(50));
        assert_eq!(log, ["NG Bluetooth connection: No adapter"]);
        assert!(!m.measuring);
    }

    #[test]
    fn start_needs_a_receiver_with_a_bluetooth_address() {
        assert!(!can_start(None));
        assert!(!can_start(Some(&ReceiverState { connection: ReceiverConnection::Off, ..plain() })));
        assert!(!can_start(Some(&ReceiverState { receiver: None, ..plain() })));
        assert!(can_start(Some(&plain())));
    }

    // ---- receiver comparison ----

    fn link() -> LinkSample {
        LinkSample {
            time_ms: 0,
            rssi: Some(-2),
            afh_in_use: Some(58),
            afh_usable: Some(79),
            afh_avoided: "6".to_owned(),
            bytes_per_second: 41000.0,
            packets_per_second: 120.0,
            queue_depth: 3,
            dropped: 0,
            capture_dropped: 0,
            airtime: 0.3,
            queue_capacity: Some(64),
            ..Default::default()
        }
    }

    fn totals(packets: u64, bytes: u64, dropped: u64, capture_frames: u64, capture_dropped: u64) -> LinkTotals {
        LinkTotals { packets, bytes, dropped, capture_frames, capture_dropped, ..Default::default() }
    }

    fn cell(text: &str, bad: bool) -> Cell {
        Cell { text: text.to_owned(), bad }
    }

    #[test]
    fn sets_sent_beside_received_with_the_differences() {
        let r = state(St {
            streaming: true,
            stream: Some(recv(5, |s| {
                s.packets = 990;
                s.lost = 10;
                s.late = 1;
                s.interval_packets = 120;
                s.interval_bytes = 40000;
                s.underruns = 2;
                s.underrun_ms = 40.0;
            })),
            sent: Some(totals(1000, 345000, 2, 480000, 480)),
            stream_starts: 3,
            ..Default::default()
        });
        let (st, l, base) = (streaming(), link(), no_baseline());
        let rows =
            compare(&CompareInputs { receiver: Some(&r), stream: &st, link: Some(&l), base: &base, last_info: None });
        assert_eq!(rows.codec.sent.text, "SBC, 48 kHz");
        assert_eq!(rows.codec.received.text, "SBC 48000 Hz, joint stereo");
        assert_eq!(rows.codec.diff.text, t("ui.receiver.codec_same"));
        assert_eq!(rows.bitrate.sent.text, "328 kbps, 120 packets/s");
        assert_eq!(rows.bitrate.received.text, "320 kbps, 120 packets/s");
        assert_eq!(rows.bitrate.diff.text, t!("ui.receiver.diff_kbps", -8));
        assert_eq!((rows.total.sent.text.as_str(), rows.total.received.text.as_str()), ("1000", "990"));
        // In flight while streaming: not marked
        assert_eq!(rows.total.diff, Cell { text: t!("ui.receiver.diff_missing", 10, 1), bad: false });
        assert_eq!(rows.loss.sent, cell("2 dropped before sending (0.20%)", true));
        assert_eq!(rows.loss.received, cell("10 (1.00%), late 1", true));
        assert_eq!(rows.timing.received.text, "jitter 1.5 ms, gap 12 ms");
        assert_eq!(rows.buffer.sent.text, "send queue 3 / 64");
        assert_eq!(rows.buffer.received.text, "190 / 200 ms (model)");
        assert_eq!(rows.audio.sent, cell("capture gaps 10 ms", true));
        assert_eq!(rows.audio.received, cell("dropouts 2 (40 ms), skips 0", true));
        assert_eq!(rows.errors.received, cell("0", false));
        assert_eq!(rows.signal.sent.text, "RSSI -2 dB vs. golden range, AFH 58/79 ch (avoiding Wi-Fi 6)");
        assert_eq!(rows.signal.received.text, "RSSI -3 dB vs. golden range, AFH 60/79 ch");
    }

    #[test]
    fn keeps_the_final_counts_after_the_stream_stopped() {
        let r = state(St {
            stream: Some(recv(5, |s| {
                s.packets = 995;
                s.idle = true;
                s.pauses = 1;
                s.decoding = false;
                s.frame_errors = 2;
            })),
            sent: Some(totals(1000, 0, 0, 0, 0)),
            stream_starts: 3,
            ..Default::default()
        });
        let (st, l, base, last) = (idle(), link(), no_baseline(), sbc());
        let rows = compare(&CompareInputs {
            receiver: Some(&r),
            stream: &st,
            link: Some(&l),
            base: &base,
            last_info: Some(&last),
        });
        assert_eq!(rows.codec.sent.text, "SBC, 48 kHz (at stop)");
        assert_eq!(rows.codec.received.text, "SBC 48000 Hz, joint stereo (at stop)");
        assert_eq!(rows.bitrate.sent.text, "-");
        assert_eq!(rows.total.diff, Cell { text: t!("ui.receiver.diff_missing", 5, 0.5), bad: true });
        assert_eq!(rows.audio.received.text, "source idle - dropouts 0 (0 ms), skips 0, source paused 1 time(s)");
        assert_eq!(rows.errors.received, cell("2 (headers only)", true));
        assert_eq!(rows.signal.received.text, "no connection");
    }

    #[test]
    fn reset_counts_from_then_on_until_another_stream_starts() {
        let r = state(St {
            streaming: true,
            stream: Some(recv(5, |s| {
                s.packets = 600;
                s.lost = 3;
            })),
            sent: Some(totals(610, 0, 1, 0, 0)),
            stream_starts: 3,
            ..Default::default()
        });
        let base = take_baseline(Some(&r));
        let later = state(St {
            streaming: true,
            stream: Some(recv(5, |s| {
                s.packets = 700;
                s.lost = 4;
            })),
            sent: Some(totals(712, 0, 1, 0, 0)),
            stream_starts: 3,
            ..Default::default()
        });
        let (st, l) = (streaming(), link());
        let rows = compare(&CompareInputs {
            receiver: Some(&later),
            stream: &st,
            link: Some(&l),
            base: &base,
            last_info: None,
        });
        assert_eq!(rows.total.sent.text, "102");
        assert_eq!(rows.total.received.text, "100");
        assert!(!rows.loss.sent.bad);
        assert_eq!(rows.loss.received.text, "1 (0.99%), late 0");

        // A new stream on both sides: from zero again
        let next = state(St {
            streaming: true,
            stream: Some(recv(6, |s| s.packets = 50)),
            sent: Some(totals(52, 0, 0, 0, 0)),
            stream_starts: 4,
            ..Default::default()
        });
        let rows = compare(&CompareInputs {
            receiver: Some(&next),
            stream: &st,
            link: Some(&l),
            base: &base,
            last_info: None,
        });
        assert_eq!(rows.total.sent.text, "52");
        assert_eq!(rows.total.received.text, "50");
    }

    #[test]
    fn a_different_codec_at_the_receiver_is_marked() {
        let r = state(St {
            streaming: true,
            stream: Some(recv(1, |s| {
                s.codec = "AAC".to_owned();
                s.config = "AAC 44.1 kHz".to_owned();
            })),
            ..Default::default()
        });
        let (st, l, base) = (streaming(), link(), no_baseline());
        let rows =
            compare(&CompareInputs { receiver: Some(&r), stream: &st, link: Some(&l), base: &base, last_info: None });
        assert_eq!(rows.codec.diff, Cell { text: t("ui.receiver.codec_differs"), bad: true });
    }

    #[test]
    fn nothing_known_dashes() {
        let (st, base) = (idle(), no_baseline());
        let rows = compare(&CompareInputs { receiver: None, stream: &st, link: None, base: &base, last_info: None });
        for id in ROW_IDS {
            let row = &rows[id];
            assert_eq!([row.sent.text.as_str(), row.received.text.as_str(), row.diff.text.as_str()], ["-", "-", "-"]);
        }
    }

    #[test]
    fn radio_text_joins_what_is_known() {
        assert_eq!(radio_text(None, None, ""), "-");
        assert_eq!(radio_text(Some(0), None, ""), "RSSI +0 dB vs. golden range");
        assert_eq!(radio_text(None, Some(45), "6, 11"), "AFH 45/79 ch (avoiding Wi-Fi 6, 11)");
    }
}
