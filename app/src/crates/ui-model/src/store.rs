//! The core's state as the UI sees it: the last Snapshot with each Event
//! applied (each event replaces one part), plus the streams the UI
//! subscribes to (link samples, log lines, the receiver test's state). The
//! UI keeps no state of its own beyond this and view state.
//!
//! Revisions: every state event carries the next revision; link samples
//! and log lines are not state and carry none of their own. A gap means a
//! lost event, so the snapshot is fetched again (state.snapshot). Its answer
//! travels apart from the events and says nothing about which events it
//! already contains, so the state events that arrive while it is on its way
//! are kept and applied again on top of it, in order: each replaces a whole
//! part, so the newest one of each part ends up in place.

use std::collections::{HashSet, VecDeque};

use ipc::{Event, LinkSample, LogLine, ReceiverState, ServerMessage, Snapshot};

/// Link samples kept: one minute at 4 per second
pub const LINK_KEEP: usize = 240;
/// Log lines kept in the window (the core keeps its own files)
pub const LOG_KEEP: usize = 5000;

/// What a message changed, so that only the views showing it draw again
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Nothing,
    /// The snapshot or a part of it
    State,
    Link,
    Log,
    Receiver,
}

/// What [`Store::on_message`] did
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Applied {
    pub change: Change,
    /// An event was lost: ask for the snapshot ([`Store::begin_resync`])
    /// before the next message
    pub resync: bool,
}

/// A state.snapshot on its way: the revision when asked, and the state events since
struct Pending {
    token: u64,
    from: Option<u64>,
    events: Vec<(u64, Event)>,
}

#[derive(Default)]
pub struct Store {
    /// Revision of the snapshot plus the events applied; None before the first
    pub rev: Option<u64>,
    pub snap: Option<Snapshot>,
    link: VecDeque<LinkSample>,
    logs: VecDeque<LogLine>,
    /// Lines up to this time were cleared: the kept lines do not bring them back
    logs_cleared_at: Option<u64>,
    pub receiver: Option<ReceiverState>,
    pending: Option<Pending>,
    /// The event of the last message showed a gap: the revision before it,
    /// and the event, for the [`Store::begin_resync`] that follows
    gap: Option<(Option<u64>, u64, Event)>,
    next_token: u64,
}

impl Store {
    pub fn new() -> Store {
        Store::default()
    }

    /// The last [`LINK_KEEP`] samples
    pub fn link(&self) -> &VecDeque<LinkSample> {
        &self.link
    }

    /// The last [`LOG_KEEP`] lines
    pub fn logs(&self) -> &VecDeque<LogLine> {
        &self.logs
    }

    pub fn clear_link(&mut self) {
        self.link.clear();
    }

    pub fn clear_logs(&mut self) {
        if let Some(last) = self.logs.back() {
            self.logs_cleared_at = Some(last.time_ms);
        }
        self.logs.clear();
    }

    /// A snapshot is to be asked for -> its token for [`Store::finish_resync`];
    /// None while one is on its way already
    pub fn begin_resync(&mut self) -> Option<u64> {
        if self.pending.is_some() {
            return None;
        }
        // A window that opens late has no revision yet: the next event after
        // the answer then sets it, without a gap check
        self.next_token += 1;
        // After a gap the snapshot counts from the revision before it, and
        // the event that showed the gap goes on top of the answer too
        let (from, events) = match self.gap.take() {
            Some((from, rev, ev)) => (from, vec![(rev, ev)]),
            None => (self.rev, Vec::new()),
        };
        self.pending = Some(Pending { token: self.next_token, from, events });
        Some(self.next_token)
    }

    /// The answer to the state.snapshot of `token` (None: it failed; not
    /// connected, the next connection brings a snapshot anyway) -> whether
    /// the state changed
    pub fn finish_resync(&mut self, token: u64, snap: Option<Snapshot>) -> bool {
        // A snapshot message of the core came first and replaced this
        if self.pending.as_ref().is_none_or(|p| p.token != token) {
            return false;
        }
        let Some(p) = self.pending.take() else { return false };
        let Some(snap) = snap else { return false };
        self.snap = Some(snap);
        self.rev = p.from;
        for (rev, ev) in p.events {
            if p.from.is_none_or(|from| rev > from) {
                self.apply_state(ev);
                if rev > self.rev.unwrap_or(0) {
                    self.rev = Some(rev);
                }
            }
        }
        true
    }

    pub fn on_message(&mut self, m: ServerMessage) -> Applied {
        // Only the message right before begin_resync counts
        self.gap = None;
        match m {
            // The core's own snapshot (after Hello, or when this client fell
            // behind) has its true revision: a requested one is not needed
            ServerMessage::Snapshot { rev, state } => {
                self.pending = None;
                self.snap = Some(*state);
                self.rev = Some(rev);
                Applied { change: Change::State, resync: false }
            }
            ServerMessage::Event { rev, event } => self.apply_event(rev, *event),
            // welcome / reject / response: the connection handles them
            _ => Applied { change: Change::Nothing, resync: false },
        }
    }

    fn apply_event(&mut self, rev: u64, ev: Event) -> Applied {
        let only = |change| Applied { change, resync: false };
        match ev {
            Event::Link(sample) => {
                self.link.push_back(sample);
                while self.link.len() > LINK_KEEP {
                    self.link.pop_front();
                }
                return only(Change::Link);
            }
            Event::Log(line) => {
                self.logs.push_back(line);
                while self.logs.len() > LOG_KEEP {
                    self.logs.pop_front();
                }
                return only(Change::Log);
            }
            Event::Receiver(state) => {
                self.receiver = Some(*state);
                return only(Change::Receiver);
            }
            Event::Bye => {
                self.snap = None;
                return only(Change::State);
            }
            // The window closes (the shell ends this process); not state.
            // Headphone buttons and serial data are for other clients: not
            // state, no revision
            Event::UiClose | Event::MediaButton(_) | Event::RfcommData(_) | Event::RfcommClosed(_) => {
                return only(Change::Nothing);
            }
            _ => {}
        }
        let mut resync = false;
        if let Some(cur) = self.rev {
            if rev <= cur {
                return only(Change::Nothing); // already in the snapshot
            }
            resync = rev > cur + 1; // lost one: the snapshot fixes it
        }
        if let Some(p) = &mut self.pending {
            p.events.push((rev, ev.clone()));
        } else if resync {
            self.gap = Some((self.rev, rev, ev.clone()));
        }
        self.rev = Some(rev);
        let changed = self.apply_state(ev);
        Applied { change: if changed { Change::State } else { Change::Nothing }, resync }
    }

    fn apply_state(&mut self, ev: Event) -> bool {
        // Before the first snapshot there is nothing to patch: it comes next
        let Some(snap) = &mut self.snap else { return false };
        match ev {
            Event::Adapter(v) => snap.adapter = v,
            Event::Devices(v) => snap.devices = v,
            Event::Scanning(v) => snap.scanning = v,
            Event::LeScanning(v) => snap.le_scanning = v,
            Event::Stream(v) => snap.stream = v,
            Event::Profiles { profiles, active_profile } => {
                snap.profiles = profiles;
                snap.active_profile = active_profile;
            }
            Event::Settings(v) => snap.settings = v,
            Event::Update(v) => snap.update = v,
            Event::Capture(v) => snap.capture = v,
            Event::V1Running(v) => snap.v1_running = v,
            Event::Verbose(v) => snap.verbose = v,
            Event::Crashes(v) => snap.crashes = v,
            Event::AudioEndpoints(v) => snap.audio_endpoints = v,
            Event::HidDevices(v) => snap.hid_devices = v,
            Event::HidCandidates(v) => snap.hid_candidates = v,
            Event::HidPasskey(v) => snap.hid_passkey = v,
            Event::HidCheck { checks, rolled_back } => {
                snap.hid_checks = checks;
                snap.hid_rolled_back = rolled_back;
            }
            Event::HidHealth(v) => snap.hid_health = v,
            _ => return false,
        }
        true
    }

    /// Puts the core's kept lines before the live ones (the live lines that
    /// came in meanwhile may be among them)
    pub fn merge_recent(&mut self, recent: Vec<LogLine>) {
        let key = |l: &LogLine| (l.time_ms, l.target.clone(), l.message.clone());
        let kept: Vec<LogLine> =
            recent.into_iter().filter(|l| self.logs_cleared_at.is_none_or(|at| l.time_ms > at)).collect();
        let last = kept.last().map(|l| l.time_ms);
        let seen: HashSet<_> = kept.iter().filter(|l| Some(l.time_ms) == last).map(key).collect();
        let live: Vec<LogLine> = self
            .logs
            .drain(..)
            .filter(|l| match last {
                None => true,
                Some(last) => l.time_ms > last || (l.time_ms == last && !seen.contains(&key(l))),
            })
            .collect();
        self.logs.extend(kept);
        self.logs.extend(live);
        while self.logs.len() > LOG_KEEP {
            self.logs.pop_front();
        }
    }
}

/// The cases about the state itself. A resync the store asks for (a
/// state.snapshot request) is `Applied::resync` and the caller's
/// [`Store::begin_resync`]; its answer is [`Store::finish_resync`].
#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    const XM5: &str = "AA:BB:CC:DD:EE:FF";

    /// The smallest snapshot that reads: only the fields without a serde default
    fn snapshot_json() -> Value {
        json!({
            "adapter": {"status": "ready", "adapters": []},
            "devices": [{"address": XM5, "name": "WH-1000XM5", "saved": true, "audio": true}],
            "scanning": false,
            "stream": {"state": "idle", "message": {"key": "status.idle", "args": []}},
            "profiles": [],
            "active_profile": "WH-1000XM5 (LDAC)",
            "settings": {
                "language": "en",
                "theme": "system",
                "start_with_windows": false,
                "start_minimized": false,
                "minimize_to_tray": false,
                "check_updates_on_startup": false,
                "include_prereleases": false,
                "media_keys": false,
                "developer_mode": false,
                "bt_chip_pid": 0,
                "bt_chip_fw_stem": "",
                "afh": "auto",
                "remote_sink": "auto"
            },
            "update": {"checking": false, "current_version": "3.0.4"},
            "config_dir": ""
        })
    }

    fn sample_snapshot() -> Snapshot {
        serde_json::from_value(snapshot_json()).unwrap()
    }

    fn snapshot_msg(rev: u64, state: Snapshot) -> ServerMessage {
        ServerMessage::Snapshot { rev, state: Box::new(state) }
    }

    /// An event message as the core writes it: {"kind": ..., "value": ...}
    fn event(rev: u64, ev: Value) -> ServerMessage {
        serde_json::from_value(json!({"type": "event", "rev": rev, "event": ev})).unwrap()
    }

    fn scanning(v: bool) -> Value {
        json!({"kind": "scanning", "value": v})
    }

    fn line(time_ms: u64, message: &str) -> LogLine {
        serde_json::from_value(json!({"time_ms": time_ms, "level": "info", "target": "t", "message": message})).unwrap()
    }

    fn log(time_ms: u64, message: &str) -> Value {
        json!({"kind": "log", "value": line(time_ms, message)})
    }

    fn link(time_ms: u64) -> Value {
        json!({"kind": "link", "value": {
            "time_ms": time_ms,
            "rssi": -3,
            "afh_in_use": 79,
            "afh_usable": 79,
            "afh_avoided": "",
            "bytes_per_second": 1000.0,
            "packets_per_second": 10.0,
            "queue_depth": 1,
            "dropped": 0,
            "capture_dropped": 0,
            "airtime": 0.3
        }})
    }

    fn messages(store: &Store) -> Vec<&str> {
        store.logs().iter().map(|l| l.message.as_str()).collect()
    }

    fn snap(store: &Store) -> &Snapshot {
        store.snap.as_ref().unwrap()
    }

    /// Applies `m`; a message that asks for a resync starts one, as core.rs
    /// does -> whether it asked
    fn send(store: &mut Store, m: ServerMessage) -> bool {
        let applied = store.on_message(m);
        if applied.resync {
            store.begin_resync();
        }
        applied.resync
    }

    #[test]
    fn takes_the_snapshot_and_applies_events_in_order() {
        let mut s = Store::new();
        assert!(s.snap.is_none());
        let mut asked = send(&mut s, snapshot_msg(10, sample_snapshot()));
        assert_eq!(s.rev, Some(10));
        assert!(!snap(&s).scanning);

        asked |= send(&mut s, event(11, scanning(true)));
        assert!(snap(&s).scanning);
        assert_eq!(s.rev, Some(11));

        asked |= send(
            &mut s,
            event(
                12,
                json!({"kind": "stream", "value": {
                    "state": "connecting",
                    "message": {"key": "status.connecting", "args": []},
                    "profile": "X",
                    "info": null,
                    "volume": null,
                    "battery": null,
                    "battery_refused": false,
                    "voice_codecs": [],
                    "mic_rates": []
                }}),
            ),
        );
        assert_eq!(snap(&s).stream.state, ipc::StreamState::Connecting);

        // The LE Audio part of a scan (state, with a revision)
        asked |= send(&mut s, event(13, json!({"kind": "le_scanning", "value": true})));
        assert!(snap(&s).le_scanning);
        assert_eq!(s.rev, Some(13));
        asked |= send(&mut s, event(14, json!({"kind": "le_scanning", "value": false})));
        assert!(!snap(&s).le_scanning);

        asked |=
            send(&mut s, event(15, json!({"kind": "profiles", "value": {"profiles": [], "active_profile": null}})));
        assert!(snap(&s).profiles.is_empty());
        assert_eq!(snap(&s).active_profile, None);
        // No resync was needed
        assert!(!asked);
    }

    #[test]
    fn asks_for_a_new_snapshot_when_a_revision_is_missing() {
        let mut s = Store::new();
        send(&mut s, snapshot_msg(1, sample_snapshot()));
        assert!(!send(&mut s, event(2, scanning(true))));
        // Revision 3 was lost
        let applied = s.on_message(event(4, scanning(false)));
        assert!(applied.resync);
        let token = s.begin_resync().unwrap();
        // The newer event is applied meanwhile
        assert!(!snap(&s).scanning);
        // Only one request while it is on its way
        assert!(send(&mut s, event(6, scanning(true))));
        assert_eq!(s.begin_resync(), None);
        // The answer was taken before events 4 and 6 (it says scanning: false
        // and has the lost event 3's devices): those events go on top again
        let mut answer = sample_snapshot();
        answer.scanning = false;
        answer.devices = Vec::new();
        assert!(s.finish_resync(token, Some(answer)));
        assert!(snap(&s).scanning);
        assert!(snap(&s).devices.is_empty());
        assert_eq!(s.rev, Some(6));
        // Older events after it are ignored, the next one applies
        assert!(!send(&mut s, event(5, scanning(false))));
        assert!(snap(&s).scanning);
        assert!(!send(&mut s, event(7, scanning(false))));
        assert!(!snap(&s).scanning);
    }

    /// The event that showed the gap is applied again on top of the answer,
    /// like the ones that came after it
    #[test]
    fn the_event_that_shows_a_gap_goes_on_top_of_the_answer() {
        let mut s = Store::new();
        send(&mut s, snapshot_msg(1, sample_snapshot()));
        assert!(send(&mut s, event(3, json!({"kind": "le_scanning", "value": true}))));
        let mut answer = sample_snapshot();
        answer.le_scanning = false;
        // The token begin_resync gave in `send`
        assert!(s.finish_resync(1, Some(answer)));
        assert!(snap(&s).le_scanning);
        assert_eq!(s.rev, Some(3));
    }

    #[test]
    fn a_failed_resync_lets_the_next_gap_ask_again() {
        let mut s = Store::new();
        send(&mut s, snapshot_msg(1, sample_snapshot()));
        assert!(s.on_message(event(3, scanning(true))).resync);
        let token = s.begin_resync().unwrap();
        // Not connected: the next connection brings a snapshot anyway
        assert!(!s.finish_resync(token, None));
        assert!(snap(&s).scanning);
        assert_eq!(s.rev, Some(3));
        assert!(s.on_message(event(5, scanning(false))).resync);
        assert!(s.begin_resync().is_some());
    }

    #[test]
    fn prefers_the_cores_own_snapshot_to_a_requested_one() {
        let mut s = Store::new();
        // Connected: core.rs asks for the snapshot
        let token = s.begin_resync().unwrap();
        let mut own = sample_snapshot();
        own.scanning = true;
        send(&mut s, snapshot_msg(10, own));
        assert!(!s.finish_resync(token, Some(sample_snapshot())));
        assert!(snap(&s).scanning);
        assert_eq!(s.rev, Some(10));
    }

    #[test]
    fn a_window_that_opens_late_takes_the_requested_snapshot() {
        let mut s = Store::new();
        let token = s.begin_resync().unwrap();
        // Before the answer there is nothing to patch; the revision is taken
        assert_eq!(s.on_message(event(7, scanning(true))).change, Change::Nothing);
        assert!(s.finish_resync(token, Some(sample_snapshot())));
        // The event goes on top of the answer, without a gap check
        assert!(snap(&s).scanning);
        assert_eq!(s.rev, Some(7));
        assert!(!send(&mut s, event(8, scanning(false))));
    }

    #[test]
    fn keeps_link_samples_and_log_lines_without_advancing_past_state_events() {
        let mut s = Store::new();
        send(&mut s, snapshot_msg(5, sample_snapshot()));
        // Subscription data may carry the current revision...
        assert_eq!(s.on_message(event(5, log(1, "stream: device AA codec Ldac"))).change, Change::Log);
        assert_eq!(s.on_message(event(5, link(2))).change, Change::Link);
        assert_eq!(s.logs().len(), 1);
        assert_eq!(s.link().len(), 1);
        // ...and the next state event is still in order
        assert!(!send(&mut s, event(6, scanning(true))));
    }

    #[test]
    fn keeps_only_the_last_link_samples_and_log_lines() {
        let mut s = Store::new();
        for t in 0..LINK_KEEP as u64 + 3 {
            s.on_message(event(0, link(t)));
        }
        assert_eq!(s.link().len(), LINK_KEEP);
        assert_eq!(s.link().front().unwrap().time_ms, 3);
        for t in 0..LOG_KEEP as u64 + 2 {
            s.on_message(event(0, log(t, "x")));
        }
        assert_eq!(s.logs().len(), LOG_KEEP);
        assert_eq!(s.logs().front().unwrap().time_ms, 2);
        // The core's kept lines do not make the window longer either
        s.merge_recent(vec![line(0, "old")]);
        assert_eq!(s.logs().len(), LOG_KEEP);
        assert_eq!(s.logs().back().unwrap().time_ms, LOG_KEEP as u64 + 1);
    }

    #[test]
    fn ignores_headphone_buttons_which_carry_no_revision_of_their_own() {
        let mut s = Store::new();
        send(&mut s, snapshot_msg(5, sample_snapshot()));
        let button = json!({"kind": "media_button", "value": {"button": "play", "pressed": true, "le_audio": false}});
        assert_eq!(s.on_message(event(5, button)).change, Change::Nothing);
        assert_eq!(s.rev, Some(5));
        assert!(!send(&mut s, event(6, scanning(true))));
        assert!(snap(&s).scanning);
    }

    /// Not state and no revision either: closing the window, serial data
    #[test]
    fn ignores_events_that_are_not_state() {
        let mut s = Store::new();
        send(&mut s, snapshot_msg(5, sample_snapshot()));
        for ev in [
            json!({"kind": "ui_close"}),
            json!({"kind": "rfcomm_closed", "value": {"channel_id": 1, "reason": "remote"}}),
        ] {
            let applied = s.on_message(event(9, ev));
            assert_eq!(applied, Applied { change: Change::Nothing, resync: false });
            assert_eq!(s.rev, Some(5));
        }
        // Welcome, reject and response are the connection's
        let welcome = ServerMessage::Welcome { protocol: 1, server: "core".into(), features: Vec::new() };
        assert_eq!(s.on_message(welcome).change, Change::Nothing);
        assert!(!send(&mut s, event(6, scanning(true))));
    }

    /// An event kind of a newer core does not read (main.rs drops the
    /// message): its revision is missing, and the snapshot asked for has it
    #[test]
    fn an_unknown_event_kind_leaves_a_gap_the_snapshot_fills() {
        let unknown = json!({"type": "event", "rev": 6, "event": {"kind": "from_a_newer_core", "value": 1}});
        assert!(serde_json::from_value::<ServerMessage>(unknown).is_err());
        let mut s = Store::new();
        send(&mut s, snapshot_msg(5, sample_snapshot()));
        assert!(send(&mut s, event(7, scanning(true))));
    }

    #[test]
    fn bye_drops_the_state() {
        let mut s = Store::new();
        send(&mut s, snapshot_msg(5, sample_snapshot()));
        assert_eq!(s.on_message(event(5, json!({"kind": "bye"}))).change, Change::State);
        assert!(s.snap.is_none());
        // Nothing to patch until the next connection's snapshot
        assert_eq!(s.on_message(event(6, scanning(true))).change, Change::Nothing);
        send(&mut s, snapshot_msg(1, sample_snapshot()));
        assert_eq!(s.rev, Some(1));
        assert!(!snap(&s).scanning);
    }

    #[test]
    fn follows_the_receiver_test_without_a_revision() {
        let mut s = Store::new();
        send(&mut s, snapshot_msg(1, sample_snapshot()));
        let first = json!({
            "connection": "connecting",
            "detail": "",
            "target": "auto",
            "receiver": null,
            "stats": null,
            "stats_age_ms": null,
            "sent": null,
            "stream_starts": 0
        });
        let mut connected = first.clone();
        connected["connection"] = json!("connected");
        // Its events are data: they take no revision
        let applied = s.on_message(event(1, json!({"kind": "receiver", "value": connected})));
        assert_eq!(applied.change, Change::Receiver);
        assert_eq!(s.receiver.as_ref().unwrap().connection, ipc::ReceiverConnection::Connected);
        assert!(!send(&mut s, event(2, scanning(true))));
    }

    #[test]
    fn keeps_the_hci_capture_state() {
        let mut s = Store::new();
        send(&mut s, snapshot_msg(1, sample_snapshot()));
        assert_eq!(snap(&s).capture, None);
        let c = json!({"path": "C:\\x\\hci_20261017_101500.pklg", "bytes": 64, "started_ms": 1_760_000_000_000u64});
        send(&mut s, event(2, json!({"kind": "capture", "value": c})));
        assert_eq!(snap(&s).capture, Some(serde_json::from_value(c.clone()).unwrap()));
        let mut more = c;
        more["bytes"] = json!(4096);
        send(&mut s, event(3, json!({"kind": "capture", "value": more})));
        assert_eq!(snap(&s).capture.as_ref().unwrap().bytes, 4096);
        send(&mut s, event(4, json!({"kind": "capture", "value": null})));
        assert_eq!(snap(&s).capture, None);
        assert_eq!(s.rev, Some(4));
    }

    #[test]
    fn puts_the_cores_kept_lines_before_the_live_ones_once_each() {
        let mut s = Store::new();
        // A live line that is also among the kept ones, and a newer one
        s.on_message(event(0, log(3, "c")));
        s.on_message(event(0, log(4, "d")));
        s.merge_recent(vec![line(1, "a"), line(2, "b"), line(3, "c")]);
        assert_eq!(messages(&s), ["a", "b", "c", "d"]);
    }

    #[test]
    fn cleared_lines_do_not_come_back_with_the_kept_ones() {
        let mut s = Store::new();
        s.on_message(event(0, log(1, "a")));
        s.on_message(event(0, log(2, "b")));
        s.clear_logs();
        assert!(s.logs().is_empty());
        s.on_message(event(0, log(4, "d")));
        s.merge_recent(vec![line(1, "a"), line(2, "b"), line(3, "c"), line(4, "d")]);
        assert_eq!(messages(&s), ["c", "d"]);
    }
}
