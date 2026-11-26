//! The peer receiver test's form: what stays chosen when the codec changes,
//! which codecs the receiver offers, how a receiver found is addressed. The
//! measurement itself is in crate::receiver.

use chrono::{Local, TimeZone as _, Timelike as _};
use ipc::{ReceiverInfo, ReceiverState};

use crate::codecs::{bit_depths, codec_spec, sample_rates};
use crate::i18n::t;
use crate::receiver::StepState;

/// The port a receiver listens on unless told otherwise
pub const RECEIVER_PORT: u16 = 51201;

/// What is sent: a test tone or what Windows plays
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Tone,
    Loopback,
}

impl Source {
    /// The value stream.start_custom takes
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Tone => "tone",
            Source::Loopback => "loopback",
        }
    }

    pub fn label_key(self) -> &'static str {
        match self {
            Source::Tone => "receiver.source_tone",
            Source::Loopback => "receiver.source_system",
        }
    }
}

/// The form's choices (0: Auto / as the receiver was started)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choices {
    pub codec: String,
    pub quality: String,
    pub rate: u32,
    pub depth: u32,
    pub source: Source,
    pub mtu: u32,
    pub buffer: u32,
}

impl Default for Choices {
    fn default() -> Choices {
        Choices {
            codec: "auto".into(),
            quality: "hq".into(),
            rate: 0,
            depth: 0,
            source: Source::Tone,
            mtu: 0,
            buffer: 0,
        }
    }
}

/// Another codec: the quality, rate and depth stay when it has them, else
/// they go back to their defaults
pub fn with_codec(c: &Choices, codec: &str) -> Choices {
    let spec = codec_spec(codec);
    Choices {
        codec: codec.to_owned(),
        quality: if spec.qualities.iter().any(|q| q.id == c.quality) { c.quality.clone() } else { "hq".into() },
        rate: if sample_rates(codec).contains(&c.rate) { c.rate } else { 0 },
        depth: if bit_depths(codec).contains(&c.depth) { c.depth } else { 0 },
        ..c.clone()
    }
}

/// Codecs the receiver offers (lower case, no spaces); empty: not known
pub fn offered(r: Option<&ReceiverState>) -> Vec<String> {
    let list = r
        .and_then(|r| r.stats.as_ref().map(|s| &s.codecs).or(r.receiver.as_ref().map(|i| &i.codecs)))
        .map(Vec::as_slice)
        .unwrap_or_default();
    list.iter()
        .map(|c| c.to_lowercase().chars().filter(|ch| !ch.is_whitespace() && *ch != '_' && *ch != '-').collect())
        .collect()
}

/// A codec the receiver does not offer (Auto always goes)
pub fn not_offered(codec: &str, offered: &[String]) -> bool {
    codec != "auto" && !offered.is_empty() && !offered.iter().any(|o| o == codec)
}

/// receiver.connect's target for a receiver found: its IP, with the port
/// when it is not the usual one
pub fn found_target(r: &ReceiverInfo) -> String {
    if r.port == RECEIVER_PORT { r.ip.clone() } else { format!("{}:{}", r.ip, r.port) }
}

/// The address field's text -> receiver.connect's target ("auto": the
/// first receiver found)
pub fn connect_target(address: &str) -> String {
    let a = address.trim();
    if a.is_empty() { "auto".into() } else { a.to_owned() }
}

/// A step's state as the list shows it
pub fn step_status(s: StepState) -> String {
    match s {
        StepState::Ok => "OK".into(),
        StepState::Failed => "NG".into(),
        StepState::Running => t("dbgc.status.running"),
        StepState::Pending => t("dbgc.status.pending"),
    }
}

/// "+1.2 s" after the start; empty before
pub fn step_time(at: Option<u64>) -> String {
    at.map_or_else(String::new, |ms| format!("+{:.1} s", ms as f64 / 1000.0))
}

/// A log line's local time of day, "hh:mm:ss" (`ms` since the Unix epoch)
pub fn time_text(ms: i64) -> String {
    match Local.timestamp_millis_opt(ms).earliest() {
        Some(d) => format!("{:02}:{:02}:{:02}", d.hour(), d.minute(), d.second()),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_change_keeps_what_fits() {
        let c = Choices { codec: "ldac".into(), quality: "sq".into(), rate: 96000, depth: 24, ..Choices::default() };
        let sbc = with_codec(&c, "sbc");
        assert_eq!(sbc.codec, "sbc");
        assert_eq!(sbc.rate, 0);
        assert_eq!(sbc.depth, 0);
        assert_eq!(with_codec(&Choices { rate: 48000, ..Choices::default() }, "sbc").rate, 48000);
        assert_eq!(with_codec(&Choices { quality: "nope".into(), ..Choices::default() }, "sbc").quality, "hq");
    }

    #[test]
    fn offered_codecs() {
        let r = ReceiverState {
            receiver: Some(ReceiverInfo {
                codecs: vec!["SBC".into(), "aptX HD".into(), "LDAC".into()],
                ..Default::default()
            }),
            ..Default::default()
        };
        let o = offered(Some(&r));
        assert_eq!(o, ["sbc", "aptxhd", "ldac"]);
        assert!(!not_offered("auto", &o));
        assert!(!not_offered("ldac", &o));
        assert!(not_offered("aac", &o));
        assert!(!not_offered("aac", &[]));
    }

    #[test]
    fn targets() {
        // ReceiverTest.test: a receiver found on the usual port connects by IP
        let mut r = ReceiverInfo { ip: "192.168.1.42".into(), port: RECEIVER_PORT, ..Default::default() };
        assert_eq!(found_target(&r), "192.168.1.42");
        r.port = 5000;
        assert_eq!(found_target(&r), "192.168.1.42:5000");
        // The saved address is empty: the local network
        assert_eq!(connect_target("  "), "auto");
        assert_eq!(connect_target(" host:1 "), "host:1");
    }

    #[test]
    fn steps() {
        assert_eq!(step_status(StepState::Ok), "OK");
        assert_eq!(step_status(StepState::Failed), "NG");
        assert_eq!(step_time(None), "");
        assert_eq!(step_time(Some(1234)), "+1.2 s");
        let t = time_text(1_700_000_000_000);
        assert_eq!(t.len(), 8);
        assert_eq!(&t[2..3], ":");
    }
}
