//! The connection flow of the Diagnostics screen: eleven steps from "start"
//! to "connection upkeep", each with a status, derived from the log lines as
//! v1's debug console did (debug_log_model.cpp on the develop branch). The real core
//! follows every line itself (crates/core/src/diag.rs, method diag.flow);
//! this tracker is the mock core's, with rules for the messages of the v2
//! core (crates/core/src/stream.rs and service.rs, crates/bt-host/src).
//!
//! The patterns are spelled out as [`Needle`] variants (no regex crate).

use std::ops::{Index, IndexMut};

use ipc::{FlowStepView, FlowView, LogLine, Msg};

use crate::i18n::{english_text, tm};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StepStatus {
    #[default]
    Pending,
    Running,
    Done,
    Warning,
    Failed,
}

impl StepStatus {
    /// "pending", "running", "done", "warning", "failed" (FlowStepView.status)
    pub fn as_str(self) -> &'static str {
        match self {
            StepStatus::Pending => "pending",
            StepStatus::Running => "running",
            StepStatus::Done => "done",
            StepStatus::Warning => "warning",
            StepStatus::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Start,
    Btstack,
    Connect,
    Caps,
    Codec,
    Capture,
    Config,
    Encoder,
    Stream,
    Audio,
    Link,
}

pub const STEPS: [Step; 11] = [
    Step::Start,
    Step::Btstack,
    Step::Connect,
    Step::Caps,
    Step::Codec,
    Step::Capture,
    Step::Config,
    Step::Encoder,
    Step::Stream,
    Step::Audio,
    Step::Link,
];

impl Step {
    /// "start", "btstack", ... "link" (FlowStepView.step)
    pub fn as_str(self) -> &'static str {
        match self {
            Step::Start => "start",
            Step::Btstack => "btstack",
            Step::Connect => "connect",
            Step::Caps => "caps",
            Step::Codec => "codec",
            Step::Capture => "capture",
            Step::Config => "config",
            Step::Encoder => "encoder",
            Step::Stream => "stream",
            Step::Audio => "audio",
            Step::Link => "link",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FlowStep {
    pub status: StepStatus,
    /// Core time of the line that set the status
    pub time_ms: Option<u64>,
    pub detail: String,
}

/// The steps of a session, in [`STEPS`] order; indexed by [`Step`]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FlowSteps(pub [FlowStep; 11]);

impl Index<Step> for FlowSteps {
    type Output = FlowStep;
    fn index(&self, step: Step) -> &FlowStep {
        &self.0[step as usize]
    }
}

impl IndexMut<Step> for FlowSteps {
    fn index_mut(&mut self, step: Step) -> &mut FlowStep {
        &mut self.0[step as usize]
    }
}

impl FlowSteps {
    /// (step, its state) in [`STEPS`] order
    pub fn iter(&self) -> impl Iterator<Item = (Step, &FlowStep)> {
        STEPS.iter().copied().zip(self.0.iter())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowSession {
    pub id: u32,
    pub start_ms: u64,
    pub stop_ms: Option<u64>,
    pub steps: FlowSteps,
    pub reconnects: u32,
    /// Failed connection attempts before the signaling connection came up
    pub connect_retries: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Done,
    Running,
    Warning,
    Failed,
    Detail,
    Retry,
}

impl Action {
    /// The status a status-setting action sets
    fn status(self) -> Option<StepStatus> {
        match self {
            Action::Done => Some(StepStatus::Done),
            Action::Running => Some(StepStatus::Running),
            Action::Warning => Some(StepStatus::Warning),
            Action::Failed => Some(StepStatus::Failed),
            Action::Detail | Action::Retry => None,
        }
    }
}

/// Detail: the text after the needle, the whole message, or none
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DetailFrom {
    After,
    Whole,
    None,
}

/// Substring of the message, or a pattern where a substring would also
/// match other lines ("capture: " is in "HCI capture: started")
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Needle {
    /// Anywhere in the message
    Text(&'static str),
    /// `/^prefix/`
    Prefix(&'static str),
    /// `/^prefix.*mid/`: `mid` later on the first line (greedy: the last one)
    PrefixThen(&'static str, &'static str),
    /// `/^prefix.*suffix$/`: one line ending in `suffix`
    PrefixEnds(&'static str, &'static str),
    /// `/^connection attempt \d+\/\d+ failed/`
    ConnectionAttempt,
}

impl Needle {
    /// Where the match ends in `text` (byte index), None without a match
    fn match_end(self, text: &str) -> Option<usize> {
        match self {
            Needle::Text(n) => text.find(n).map(|i| i + n.len()),
            Needle::Prefix(p) => text.starts_with(p).then_some(p.len()),
            Needle::PrefixThen(p, mid) => {
                let rest = text.strip_prefix(p)?;
                let line = rest.split('\n').next().unwrap_or("");
                line.rfind(mid).map(|i| p.len() + i + mid.len())
            }
            Needle::PrefixEnds(p, suffix) => {
                let rest = text.strip_prefix(p)?;
                (!rest.contains('\n') && rest.ends_with(suffix)).then_some(text.len())
            }
            Needle::ConnectionAttempt => {
                const HEAD: &str = "connection attempt ";
                let rest = text.strip_prefix(HEAD)?;
                let digits = |s: &str| s.bytes().take_while(u8::is_ascii_digit).count();
                let a = digits(rest);
                let rest2 = rest[a..].strip_prefix('/').filter(|_| a > 0)?;
                let b = digits(rest2);
                let tail = rest2[b..].strip_prefix(" failed").filter(|_| b > 0)?;
                Some(text.len() - tail.len())
            }
        }
    }
}

struct Rule {
    step: Step,
    /// Only lines of this log target (or its submodules)
    target: Option<&'static str>,
    needle: Needle,
    action: Action,
    detail: DetailFrom,
}

const fn rule(step: Step, needle: Needle, action: Action, detail: DetailFrom) -> Rule {
    Rule { step, target: None, needle, action, detail }
}

const fn rule_for(step: Step, target: &'static str, needle: Needle, action: Action, detail: DetailFrom) -> Rule {
    Rule { step, target: Some(target), needle, action, detail }
}

use Action as A;
use DetailFrom as D;
use Needle as N;

const RULES: &[Rule] = &[
    // LE Audio, as the core maps it (diag.rs); before the classic rules,
    // whose needles its lines contain
    rule(Step::Connect, N::PrefixThen("LE Audio: connecting to ", " failed: "), A::Retry, D::Whole),
    rule(Step::Connect, N::Prefix("LE Audio: connecting to "), A::Running, D::After),
    rule(Step::Connect, N::Prefix("LE Audio: LE link to "), A::Detail, D::Whole),
    rule(Step::Connect, N::PrefixThen("LE Audio: ", " encryption on (0x00)"), A::Done, D::None),
    rule(Step::Caps, N::PrefixEnds("LE Audio: ", " characteristics"), A::Running, D::Whole),
    rule(Step::Caps, N::PrefixThen("LE Audio: ", ": ASCS "), A::Done, D::Whole),
    rule(Step::Codec, N::Prefix("LE Audio: starting unicast stream: "), A::Done, D::After),
    rule(Step::Capture, N::Prefix("LE Audio: audio capture: "), A::Failed, D::Whole),
    rule(Step::Capture, N::Prefix("LE Audio: capture "), A::Done, D::After),
    rule(Step::Config, N::Prefix("LE Audio: QoS: "), A::Running, D::After),
    rule(Step::Config, N::Prefix("LE Audio: unicast stream failed"), A::Failed, D::Whole),
    rule(Step::Config, N::PrefixThen("LE Audio: CIS ", " established (ISO interval"), A::Done, D::Whole),
    rule(Step::Encoder, N::Prefix("LE Audio: LC3 encoders ready: "), A::Done, D::After),
    rule(Step::Stream, N::Prefix("LE Audio: sending ISO SDUs"), A::Done, D::Whole),
    rule(Step::Audio, N::Prefix("LE Audio streaming: "), A::Done, D::After),
    rule(Step::Link, N::Prefix("LE Audio stream lost, reconnecting"), A::Running, D::Whole),
    rule(Step::Link, N::Prefix("LE Audio reconnect attempt "), A::Detail, D::Whole),
    rule(Step::Link, N::Prefix("LE Audio stream restored: "), A::Warning, D::Whole),
    // Before "connecting to ", which it contains
    rule(Step::Link, N::Text("reconnecting to "), A::Detail, D::Whole),
    rule(Step::Btstack, N::Text("state Connecting: Initializing BTstack"), A::Running, D::None),
    rule(Step::Btstack, N::Text("controller: "), A::Detail, D::After),
    rule(Step::Btstack, N::Text("Realtek firmware likely NOT loaded"), A::Warning, D::Whole),
    rule(Step::Btstack, N::Text("HCI initialization failed"), A::Failed, D::Whole),
    rule(Step::Btstack, N::Text("HCI initialization timed out"), A::Failed, D::Whole),
    rule(Step::Btstack, N::Text("could not open the Bluetooth adapter"), A::Failed, D::Whole),
    rule(Step::Btstack, N::Text("HCI ready"), A::Done, D::None),
    rule(Step::Connect, N::Text("connecting to "), A::Running, D::After),
    rule(Step::Connect, N::Text("HCI ACL connection established"), A::Detail, D::Whole),
    rule(Step::Connect, N::Text("signaling connection established"), A::Done, D::Whole),
    // A failed attempt is retried; the state Error line blames the step if
    // none succeeds
    rule(Step::Connect, N::Text("signaling connection failed"), A::Detail, D::Whole),
    rule_for(Step::Connect, "bt_host", N::ConnectionAttempt, A::Retry, D::Whole),
    rule(Step::Caps, N::Text("capability discovery complete"), A::Done, D::After),
    rule(Step::Codec, N::Text("codec fallback: "), A::Warning, D::After),
    rule(Step::Codec, N::Text("codec selected: "), A::Done, D::After),
    // audio_capture's own "audio capture: loopback of ..." describes the
    // source; the stream's "audio capture: ..." is the failure
    rule_for(Step::Capture, "audio_capture", N::Prefix("audio capture: "), A::Detail, D::After),
    rule_for(Step::Capture, "bluecaster_core::stream", N::Prefix("audio capture: "), A::Failed, D::After),
    rule(Step::Capture, N::Text("capturing app "), A::Detail, D::After),
    rule(Step::Capture, N::Prefix("capture: "), A::Done, D::After),
    rule(Step::Config, N::Text("codec configured"), A::Running, D::After),
    rule(Step::Config, N::Text("stream establishment failed"), A::Failed, D::Whole),
    rule(Step::Config, N::Text("stream established"), A::Done, D::After),
    rule(Step::Encoder, N::Text("encoder ready: "), A::Done, D::After),
    rule(Step::Encoder, N::Prefix("encoder: "), A::Failed, D::After),
    rule(Step::Stream, N::Text("streaming started"), A::Done, D::None),
    rule(Step::Audio, N::Text("audio capture start: "), A::Failed, D::After),
    rule(Step::Audio, N::Text("state Streaming"), A::Done, D::None),
    rule(Step::Link, N::Text("connection lost, reconnecting"), A::Running, D::Whole),
    rule(Step::Link, N::Text(", reconnecting"), A::Running, D::Whole),
    rule(Step::Link, N::Text("reconnect attempt "), A::Detail, D::Whole),
    rule(Step::Link, N::Text("reconnected on attempt "), A::Warning, D::Whole),
];

const START: &str = "stream: device ";
/// An LE Audio stream's first line (with "(experimental)" in older logs)
const LE_STARTS: [&str; 2] = ["LE Audio stream to ", "LE Audio (experimental) stream to "];
const STOP: &str = "streaming stopped";
const ERROR_STATE: &str = "state Error: ";

fn target_matches(target: &str, prefix: &str) -> bool {
    target == prefix || target.strip_prefix(prefix).is_some_and(|rest| rest.starts_with("::"))
}

fn after_needle(text: &str, needle: Needle) -> String {
    let rest = needle.match_end(text).map_or("", |end| &text[end..]);
    let mut s = rest.trim();
    if let Some(cut) = s.strip_suffix("...") {
        s = cut;
    }
    if s.len() >= 2 && s.starts_with('(') && s.ends_with(')') {
        s = &s[1..s.len() - 1];
    }
    s.to_owned()
}

fn finished(s: StepStatus) -> bool {
    s == StepStatus::Done || s == StepStatus::Warning
}

/// Follows the log lines of a connection; `session` is the current (or last)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowTracker {
    pub session: Option<FlowSession>,
    next_id: u32,
}

impl Default for FlowTracker {
    fn default() -> Self {
        FlowTracker { session: None, next_id: 1 }
    }
}

impl FlowTracker {
    pub fn new() -> FlowTracker {
        FlowTracker::default()
    }

    /// Takes one log line; true if the flow changed
    pub fn feed(&mut self, line: &LogLine) -> bool {
        let t = line.message.as_str();
        let start = if t.contains(START) { Some(START) } else { LE_STARTS.into_iter().find(|p| t.starts_with(p)) };
        if let Some(start) = start {
            let mut session = FlowSession {
                id: self.next_id,
                start_ms: line.time_ms,
                stop_ms: None,
                steps: FlowSteps::default(),
                reconnects: 0,
                connect_retries: 0,
            };
            self.next_id += 1;
            set(&mut session, Step::Start, StepStatus::Done, line, &after_needle(t, Needle::Text(start)));
            self.session = Some(session);
            return true;
        }
        let Some(s) = self.session.as_mut().filter(|s| s.stop_ms.is_none()) else { return false };
        if t.contains(STOP) {
            s.stop_ms = Some(line.time_ms);
            return true;
        }
        if t.contains(ERROR_STATE) {
            // Blame the first step that did not complete
            let detail = after_needle(t, Needle::Text(ERROR_STATE));
            let step = STEPS.into_iter().find(|&st| !finished(s.steps[st].status) || st == Step::Link);
            if let Some(step) = step {
                set(s, step, StepStatus::Failed, line, &detail);
            }
            return true;
        }
        for r in RULES {
            if let Some(target) = r.target
                && !target_matches(&line.target, target)
            {
                continue;
            }
            if r.needle.match_end(t).is_none() {
                continue;
            }
            let detail = match r.detail {
                DetailFrom::After => after_needle(t, r.needle),
                DetailFrom::Whole => t.to_owned(),
                DetailFrom::None => String::new(),
            };
            // While reconnecting, the steps keep the first connection's results:
            // what goes wrong in an attempt belongs to the upkeep step
            if s.steps[Step::Link].status == StepStatus::Running && r.step != Step::Link {
                if r.action == Action::Failed || r.action == Action::Warning {
                    s.steps[Step::Link].detail = t.to_owned();
                }
                return true;
            }
            match r.action {
                Action::Detail => {
                    if !detail.is_empty() {
                        s.steps[r.step].detail = detail;
                    }
                }
                Action::Retry => {
                    s.connect_retries += 1;
                    if !detail.is_empty() {
                        s.steps[r.step].detail = detail;
                    }
                }
                Action::Done if r.step == Step::Connect && s.connect_retries > 0 => {
                    let detail = format!("{detail} (after {} failed attempt(s))", s.connect_retries);
                    set(s, r.step, StepStatus::Warning, line, &detail);
                }
                action => {
                    if let Some(status) = action.status() {
                        set(s, r.step, status, line, &detail);
                    }
                }
            }
            if r.step == Step::Audio && r.action == Action::Done && s.steps[Step::Link].status == StepStatus::Pending {
                set(s, Step::Link, StepStatus::Done, line, "");
            }
            if r.step == Step::Link && r.action == Action::Running {
                s.reconnects += 1;
            }
            return true;
        }
        false
    }
}

fn set(session: &mut FlowSession, step: Step, status: StepStatus, line: &LogLine, detail: &str) {
    let st = &mut session.steps[step];
    match status {
        StepStatus::Failed => {
            if st.status == StepStatus::Failed {
                return; // keep the first cause
            }
        }
        StepStatus::Warning => {
            if st.status == StepStatus::Failed {
                return;
            }
        }
        StepStatus::Done => {
            if st.status == StepStatus::Failed {
                return;
            }
            if st.status == StepStatus::Warning {
                // Completed, but the warning stays visible
                if !detail.is_empty() {
                    st.detail = detail.to_owned();
                }
                return;
            }
        }
        StepStatus::Running => {
            // Only the upkeep step goes back from finished to running
            if st.status == StepStatus::Failed {
                return;
            }
            if finished(st.status) && step != Step::Link {
                return;
            }
        }
        StepStatus::Pending => {}
    }
    st.status = status;
    st.time_ms = Some(line.time_ms);
    if !detail.is_empty() {
        st.detail = detail.to_owned();
    }
}

/// A step's detail in the UI language: the message the core computed for it
/// (FlowStepView.detail_msg: the audio rate, the link's reconnects)
/// translated, with what the core put after that message in `detail`
/// ("2 reconnect(s) - <the line's detail>"); else the log line's own detail
pub fn step_detail(detail: &str, detail_msg: Option<&Msg>) -> String {
    let Some(msg) = detail_msg else { return detail.to_owned() };
    let text = tm(Some(msg));
    let en = english_text(msg);
    if detail.starts_with(&format!("{en} - ")) { text + &detail[en.len()..] } else { text }
}

/// A session as the core's diag.flow answers it
pub fn flow_view(s: Option<&FlowSession>) -> FlowView {
    let Some(s) = s else {
        return FlowView { session: 0, first_seq: None, start_ms: 0, stop_ms: None, reconnects: 0, steps: Vec::new() };
    };
    FlowView {
        session: s.id,
        first_seq: None,
        start_ms: s.start_ms,
        stop_ms: s.stop_ms,
        reconnects: s.reconnects,
        steps: s
            .steps
            .iter()
            .map(|(step, st)| FlowStepView {
                step: step.as_str().to_owned(),
                status: st.status.as_str().to_owned(),
                elapsed_ms: st.time_ms.map(|t| t.saturating_sub(s.start_ms)),
                detail: st.detail.clone(),
                detail_msg: None,
                seq: None,
                time_ms: None,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipc::LogLevel;

    fn line_at(time_ms: u64, target: &str, message: &str, level: LogLevel) -> LogLine {
        LogLine { time_ms, level, target: target.to_owned(), message: message.to_owned(), seq: None, session: None }
    }

    fn line(time_ms: u64, message: &str) -> LogLine {
        line_at(time_ms, "test", message, LogLevel::Info)
    }

    #[test]
    fn follows_a_connection_from_start_to_streaming() {
        let mut f = FlowTracker::new();
        let lines = [
            "stream: device AC:80:0A:12:34:56 codec Ldac quality Hq",
            "state Connecting: Starting Bluetooth...",
            "controller: company 0x005D (Realtek), HCI version 10",
            "HCI ready after 412 ms",
            "connecting to AC:80:0A:12:34:56",
            "HCI ACL connection established to AC:80:0A:12:34:56",
            "signaling connection established (cid 0x0041, handle 0x000b)",
            "capability discovery complete (LDAC=true SBC=true)",
            "codec selected: LDAC",
            "capture: 48000 Hz, 2 channel(s), F32 -> 2 channel(s)",
            "HCI capture: started -> C:\\x.pklg",
            "codec configured (local 1, remote 3)",
            "stream established (local 1, remote 3, mtu 1005)",
            "encoder ready: LDAC 990 kbps, mtu 1005, 24-bit, ABR false",
            "streaming started",
            "state Streaming: Streaming",
        ];
        for (i, m) in lines.iter().enumerate() {
            f.feed(&line(1000 + i as u64, m));
        }
        let s = f.session.as_ref().unwrap();
        for (step, st) in s.steps.iter() {
            assert_eq!(st.status, StepStatus::Done, "{}", step.as_str());
        }
        assert!(s.steps[Step::Btstack].detail.contains("Realtek"));
        assert_eq!(s.steps[Step::Codec].detail, "LDAC");
        assert!(s.steps[Step::Capture].detail.contains("48000 Hz"));
    }

    #[test]
    fn blames_the_first_unfinished_step_on_an_error_and_counts_reconnects() {
        let mut f = FlowTracker::new();
        f.feed(&line(1, "stream: device AA:BB:CC:DD:EE:FF"));
        f.feed(&line(2, "HCI ready after 300 ms"));
        f.feed(&line(3, "connecting to AA:BB:CC:DD:EE:FF"));
        f.feed(&line_at(4, "test", "state Error: The device did not answer.", LogLevel::Error));
        let s = f.session.as_ref().unwrap();
        assert_eq!(s.steps[Step::Connect].status, StepStatus::Failed);
        assert_eq!(s.steps[Step::Connect].detail, "The device did not answer.");

        f.feed(&line(10, "stream: device AA:BB:CC:DD:EE:FF"));
        assert_eq!(f.session.as_ref().unwrap().id, 2);
        f.feed(&line(11, "state Streaming: Streaming"));
        f.feed(&line_at(12, "test", "connection lost, reconnecting", LogLevel::Warn));
        assert_eq!(f.session.as_ref().unwrap().steps[Step::Link].status, StepStatus::Running);
        f.feed(&line(13, "reconnected on attempt 1"));
        assert_eq!(f.session.as_ref().unwrap().steps[Step::Link].status, StepStatus::Warning);
        assert_eq!(f.session.as_ref().unwrap().reconnects, 1);
        f.feed(&line(14, "streaming stopped, cleaning up"));
        assert_eq!(f.session.as_ref().unwrap().stop_ms, Some(14));
    }

    #[test]
    fn keeps_the_first_connections_steps_through_a_reconnect_with_a_failed_attempt() {
        let mut f = FlowTracker::new();
        let mut at = |time_ms: u64, target: &str, message: &str| {
            f.feed(&line_at(time_ms, target, message, LogLevel::Info));
        };
        at(1, "bluecaster_core::stream", "stream: device 58:18:62:02:7A:D7 codec LDAC");
        at(2, "bt_host", "HCI ready after 69 ms");
        at(3, "bt_host", "connecting to 58:18:62:02:7A:D7");
        at(4, "bt_host", "signaling connection established (cid 0x0041, handle 0x0001)");
        at(5, "bluecaster_core::stream", "codec selected: LDAC");
        at(6, "audio_capture::loopback", "audio capture: loopback of \"CABLE Input\": mix 96000 Hz 2 ch 32 bit");
        at(7, "bluecaster_core::stream", "capture: 96000 Hz, 2 channel(s), F32 -> 2 channel(s)");
        at(8, "bt_host::app", "stream established (local 1, remote 3, mtu 895)");
        at(9, "bluecaster_core::service", "state Streaming: Streaming");
        assert_eq!(f.session.as_ref().unwrap().steps[Step::Capture].status, StepStatus::Done);

        let mut at = |time_ms: u64, target: &str, message: &str| {
            f.feed(&line_at(time_ms, target, message, LogLevel::Warn));
        };
        at(20, "bluecaster_core::stream", "connection lost, reconnecting");
        at(21, "bt_host", "reconnecting to 58:18:62:02:7A:D7");
        at(40, "bt_host::app", "signaling connection failed (0x67)");
        at(40, "bt_host::app", "stream establishment failed (0x67)");
        at(40, "bluecaster_core::stream", "reconnect attempt 1 failed");
        at(43, "bt_host", "reconnecting to 58:18:62:02:7A:D7");
        at(44, "bluecaster_core::stream", "reconnected on attempt 2");
        let s = f.session.as_ref().unwrap();
        assert_eq!(s.steps[Step::Connect].status, StepStatus::Done);
        assert_eq!(s.steps[Step::Config].status, StepStatus::Done);
        assert_eq!(s.steps[Step::Link].status, StepStatus::Warning);
        assert_eq!(s.reconnects, 1);
    }

    #[test]
    fn shows_a_connection_that_needed_a_second_attempt_as_a_warning() {
        let mut f = FlowTracker::new();
        let mut at = |time_ms: u64, target: &str, message: &str| {
            f.feed(&line_at(time_ms, target, message, LogLevel::Info));
        };
        at(1, "bluecaster_core::stream", "stream: device AA:BB:CC:DD:EE:FF");
        at(2, "bt_host", "connecting to AA:BB:CC:DD:EE:FF");
        at(3, "bt_host::app", "signaling connection failed (0x04)");
        at(4, "bt_host", "connection attempt 1/3 failed (PageTimeout), trying again");
        at(5, "bt_host::app", "signaling connection established (cid 0x0041, handle 0x000b)");
        let s = f.session.as_ref().unwrap();
        assert_eq!(s.steps[Step::Connect].status, StepStatus::Warning);
        assert!(s.steps[Step::Connect].detail.contains("after 1 failed attempt(s)"));
    }

    #[test]
    fn shows_the_cores_computed_step_details_in_the_ui_language() {
        // English only: the language is process-wide and tests run in
        // parallel
        let live = Msg { key: "dbgc.audio_live".to_owned(), args: vec!["120".to_owned(), "990".to_owned()] };
        assert_eq!(step_detail("120 packets/s, 990 kbps", Some(&live)), "120 packets/s, 990 kbps");
        assert_eq!(step_detail("reconnecting to AA:BB", None), "reconnecting to AA:BB");
        let reconnects = Msg { key: "dbgc.link_reconnects".to_owned(), args: vec!["2".to_owned()] };
        // What the core puts after its message stays
        assert_eq!(
            step_detail("2 reconnect(s) - reconnecting to AA:BB", Some(&reconnects)),
            format!("{} - reconnecting to AA:BB", crate::t!("dbgc.link_reconnects", "2"))
        );
    }

    #[test]
    fn matches_the_patterns_as_the_regular_expressions_did() {
        let end = |n: Needle, t: &str| n.match_end(t);
        assert_eq!(end(N::ConnectionAttempt, "connection attempt 1/3 failed (x)"), Some(29));
        assert_eq!(end(N::ConnectionAttempt, "connection attempt /3 failed"), None);
        assert_eq!(end(N::ConnectionAttempt, "connection attempt 1/ failed"), None);
        assert_eq!(end(N::ConnectionAttempt, "a connection attempt 1/3 failed"), None);
        assert!(end(N::PrefixEnds("LE Audio: ", " characteristics"), "LE Audio: found 5 characteristics").is_some());
        assert!(end(N::PrefixEnds("LE Audio: ", " characteristics"), "LE Audio: 5 characteristics read").is_none());
        assert!(end(N::PrefixThen("LE Audio: ", ": ASCS "), "LE Audio: dev: ASCS 2 sinks").is_some());
        assert!(end(N::PrefixThen("LE Audio: ", ": ASCS "), "LE Audio: dev\n: ASCS 2 sinks").is_none());
        let mut f = FlowTracker::new();
        f.feed(&line(1, "LE Audio (experimental) stream to AA:BB"));
        assert_eq!(f.session.as_ref().unwrap().steps[Step::Start].detail, "AA:BB");
        f.feed(&line(2, "LE Audio: connecting to AA:BB failed: timeout"));
        assert_eq!(f.session.as_ref().unwrap().connect_retries, 1);
        f.feed(&line(3, "LE Audio: connecting to AA:BB..."));
        let s = f.session.as_ref().unwrap();
        assert_eq!(
            (s.steps[Step::Connect].status, s.steps[Step::Connect].detail.as_str()),
            (StepStatus::Running, "AA:BB")
        );
    }

    #[test]
    fn views_a_session_as_the_core_answers_diag_flow() {
        assert_eq!(flow_view(None).steps.len(), 0);
        let mut f = FlowTracker::new();
        f.feed(&line(100, "stream: device AA:BB"));
        f.feed(&line(150, "HCI ready after 3 ms"));
        let v = flow_view(f.session.as_ref());
        assert_eq!((v.session, v.start_ms, v.steps.len()), (1, 100, 11));
        assert_eq!(
            (v.steps[1].step.as_str(), v.steps[1].status.as_str(), v.steps[1].elapsed_ms),
            ("btstack", "done", Some(50))
        );
        assert_eq!(v.steps[2].elapsed_ms, None);
    }
}
