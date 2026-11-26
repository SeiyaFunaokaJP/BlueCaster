//! The log of the Diagnostics screen: level, text and "this attempt only"
//! filters, the line text copies use, and the window of lines drawn, as
//! v1's debug console (wx_debug_console_dialog.cpp on the develop branch) had them.

use chrono::{Local, TimeZone, Timelike};
use ipc::{FlowView, LogLevel, LogLine};

use crate::format::mask_identifiers;

/// Most severe first
pub const LEVEL_ORDER: [LogLevel; 5] =
    [LogLevel::Error, LogLevel::Warn, LogLevel::Info, LogLevel::Debug, LogLevel::Trace];

/// A level's name as the core sends it ("error", "warn", ...)
pub fn level_name(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Error => "error",
        LogLevel::Warn => "warn",
        LogLevel::Info => "info",
        LogLevel::Debug => "debug",
        LogLevel::Trace => "trace",
    }
}

/// The level filter: every line, or a level and those more severe
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelFilter {
    All,
    Level(LogLevel),
}

impl From<LogLevel> for LevelFilter {
    fn from(l: LogLevel) -> Self {
        LevelFilter::Level(l)
    }
}

/// An attempt as the filter needs it (Pick<FlowView, "session" | "first_seq">)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attempt {
    pub session: u32,
    pub first_seq: Option<u64>,
}

impl From<&FlowView> for Attempt {
    fn from(f: &FlowView) -> Self {
        Attempt { session: f.session, first_seq: f.first_seq }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogFilter {
    pub level: LevelFilter,
    /// Text in the message or the target (any case)
    pub query: String,
    /// Only the lines of this attempt (None: every line)
    pub attempt: Option<Attempt>,
}

fn level_index(level: LogLevel) -> usize {
    LEVEL_ORDER.iter().position(|l| *l == level).unwrap_or(LEVEL_ORDER.len())
}

/// The line is at the filter's level or more severe
pub fn level_shown(level: LogLevel, filter: LevelFilter) -> bool {
    match filter {
        LevelFilter::All => true,
        LevelFilter::Level(f) => level_index(level) <= level_index(f),
    }
}

/// The line belongs to the attempt: it came at or after the attempt's first
/// line, or the core tagged it with the attempt. A line that carries neither
/// (an older core) is kept.
pub fn in_attempt(line: &LogLine, attempt: Attempt) -> bool {
    let by_seq = line.seq.zip(attempt.first_seq);
    if by_seq.is_none() && line.session.is_none() {
        return true;
    }
    by_seq.is_some_and(|(seq, first)| seq >= first) || line.session.is_some_and(|s| s == attempt.session)
}

pub fn filter_log<'a>(lines: &'a [LogLine], f: &LogFilter) -> Vec<&'a LogLine> {
    let q = crate::format::js_trim(&f.query).to_lowercase();
    lines
        .iter()
        .filter(|l| {
            level_shown(l.level, f.level)
                && f.attempt.is_none_or(|a| in_attempt(l, a))
                && (q.is_empty() || l.message.to_lowercase().contains(&q) || l.target.to_lowercase().contains(&q))
        })
        .collect()
}

/// A key that stays with a line while the buffer moves (seq when the core numbers its lines)
pub fn log_key(l: &LogLine) -> String {
    match l.seq {
        Some(seq) => format!("#{seq}"),
        None => format!("{}\u{0}{}\u{0}{}", l.time_ms, l.target, l.message),
    }
}

/// One line as copied: time, level, target, message
pub fn format_log_line(l: &LogLine, time: &str, mask: bool) -> String {
    let msg = if mask { mask_identifiers(&l.message) } else { l.message.clone() };
    format!("{time} {:<5} {}: {msg}", level_name(l.level).to_uppercase(), l.target)
}

/// Which of `count` filtered lines to draw, at most `limit`: the newest ones,
/// or a window around `focus` (the index of a line jumped to) when that is
/// older. Returns (start, end).
pub fn drawn_range(count: usize, limit: usize, focus: Option<i64>) -> (usize, usize) {
    let start = count.saturating_sub(limit);
    match focus {
        Some(f) if f >= 0 && (f as usize) < start => {
            let from = (f as usize).saturating_sub(limit / 4);
            (from, count.min(from + limit))
        }
        _ => (start, count),
    }
}

/// The keys of the lines from `a` to `b` (either order), for a shift-click
pub fn keys_between(lines: &[LogLine], a: &str, b: &str) -> Vec<String> {
    let i = lines.iter().position(|l| log_key(l) == a);
    let j = lines.iter().position(|l| log_key(l) == b);
    match (i, j) {
        (Some(i), Some(j)) => lines[i.min(j)..=i.max(j)].iter().map(log_key).collect(),
        (None, Some(_)) => vec![b.to_owned()],
        _ => Vec::new(),
    }
}

// ---- Time of day ------------------------------------------------------------
// The core stamps lines with milliseconds since it started; v1 showed the
// time of day. The core tells its local time of day at time 0
// (Snapshot.clock_ms), which gives the exact time. Without it (an older
// core) the wall clock at the core's time 0 is estimated from live samples:
// now - time_ms is the epoch plus the delivery delay, so the smallest value
// seen is the best estimate.

/// The better of two epoch estimates (None: none yet); times in ms
/// (`now_ms` since the Unix epoch)
pub fn next_epoch(prev: Option<i64>, time_ms: u64, now_ms: i64) -> i64 {
    let e = now_ms - time_ms as i64;
    prev.map_or(e, |p| p.min(e))
}

/// "hh:mm:ss.mmm" of the local time of day at core time `time_ms`
/// (`epoch`: ms since the Unix epoch at core time 0)
pub fn time_of_day(time_ms: u64, epoch: i64) -> String {
    match Local.timestamp_millis_opt(epoch + time_ms as i64).earliest() {
        Some(d) => format!("{:02}:{:02}:{:02}.{:03}", d.hour(), d.minute(), d.second(), d.timestamp_subsec_millis()),
        None => String::new(),
    }
}

const DAY_MS: i64 = 86_400_000;

/// "hh:mm:ss.mmm" of core time `time_ms` from the core's own clock: `clock_ms`
/// is its local time of day at time 0 (Snapshot.clock_ms); past midnight
/// the time of day starts again from 00:00
pub fn core_time_of_day(time_ms: u64, clock_ms: u64) -> String {
    let t = (clock_ms as i64 + time_ms as i64).rem_euclid(DAY_MS);
    format!("{:02}:{:02}:{:02}.{:03}", t / 3_600_000, t / 60_000 % 60, t / 1000 % 60, t % 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(seq: Option<u64>, session: Option<u32>, message: Option<&str>, level: LogLevel) -> LogLine {
        LogLine {
            time_ms: seq.unwrap_or(0) * 10,
            level,
            target: "bluecaster_core::stream".into(),
            message: message
                .map_or_else(|| format!("line {}", seq.map_or("null".into(), |s| s.to_string())), str::to_owned),
            seq,
            session,
        }
    }

    fn l(seq: Option<u64>, session: Option<u32>) -> LogLine {
        line(seq, session, None, LogLevel::Info)
    }

    fn at(session: u32, first_seq: Option<u64>) -> Attempt {
        Attempt { session, first_seq }
    }

    #[test]
    fn levels() {
        use LogLevel::*;
        assert!(level_shown(Debug, LevelFilter::All));
        assert!(!level_shown(Debug, Info.into()));
        assert!(level_shown(Info, Info.into()));
        assert!(level_shown(Warn, Info.into()));
        assert!(!level_shown(Warn, Error.into()));
        assert!(level_shown(Error, Warn.into()));
    }

    #[test]
    fn this_attempt_only_from_its_first_line_or_tagged_with_it() {
        let attempt = at(2, Some(10));
        assert!(!in_attempt(&l(Some(9), Some(1)), attempt));
        assert!(in_attempt(&l(Some(10), Some(2)), attempt));
        assert!(in_attempt(&l(Some(12), Some(2)), attempt));
        // Only one of the two known
        assert!(in_attempt(&l(Some(11), None), attempt));
        assert!(!in_attempt(&l(Some(5), None), attempt));
        assert!(in_attempt(&l(None, Some(2)), attempt));
        assert!(!in_attempt(&l(None, Some(1)), attempt));
        assert!(in_attempt(&l(Some(5), Some(2)), at(2, None)));
        // Neither: an older core, kept
        assert!(in_attempt(&l(None, None), attempt));
        // No attempt yet: the lines before the first
        assert!(in_attempt(&l(Some(3), Some(0)), at(0, None)));
        assert!(!in_attempt(&l(Some(3), Some(1)), at(0, None)));
    }

    #[test]
    fn combines_level_text_and_attempt() {
        let lines = [
            line(Some(1), Some(1), Some("connecting to AC:80:0A:12:34:56"), LogLevel::Info),
            line(Some(2), Some(1), Some("HCI ready"), LogLevel::Debug),
            line(Some(3), Some(2), Some("connecting to AC:80:0A:12:34:56"), LogLevel::Info),
            line(Some(4), Some(2), Some("codec selected: LDAC"), LogLevel::Warn),
        ];
        let seqs = |level: LevelFilter, query: &str, attempt: Option<Attempt>| {
            filter_log(&lines, &LogFilter { level, query: query.into(), attempt })
                .iter()
                .map(|l| l.seq.unwrap())
                .collect::<Vec<_>>()
        };
        let info = LevelFilter::Level(LogLevel::Info);
        assert_eq!(seqs(LevelFilter::All, "", None), [1, 2, 3, 4]);
        assert_eq!(seqs(info, "", None), [1, 3, 4]);
        assert_eq!(seqs(info, "CONNECTING", None), [1, 3]);
        assert_eq!(seqs(info, "stream", None), [1, 3, 4]); // the target matches
        assert_eq!(seqs(LevelFilter::All, "", Some(at(2, Some(3)))), [3, 4]);
        assert_eq!(seqs(LevelFilter::Level(LogLevel::Warn), " ", Some(at(2, Some(3)))), [4]);
    }

    #[test]
    fn keys_and_copied_text() {
        assert_eq!(log_key(&l(Some(7), Some(1))), "#7");
        let old = line(None, None, Some("x"), LogLevel::Info);
        assert_eq!(log_key(&old), log_key(&old.clone()));
        let masked = format_log_line(
            &line(Some(1), Some(1), Some("to AC:80:0A:12:34:56"), LogLevel::Info),
            "12:00:00.000",
            true,
        );
        let n = masked.strip_prefix("12:00:00.000 INFO  bluecaster_core::stream: to <addr-").unwrap();
        assert!(n.strip_suffix('>').unwrap().parse::<u32>().is_ok());
        assert_eq!(
            format_log_line(&line(Some(1), Some(1), Some("to AC:80:0A:12:34:56"), LogLevel::Error), "t", false),
            "t ERROR bluecaster_core::stream: to AC:80:0A:12:34:56"
        );
    }

    #[test]
    fn selects_a_range_in_either_direction() {
        let lines = [l(Some(1), Some(1)), l(Some(2), Some(1)), l(Some(3), Some(1)), l(Some(4), Some(1))];
        assert_eq!(keys_between(&lines, "#2", "#4"), ["#2", "#3", "#4"]);
        assert_eq!(keys_between(&lines, "#4", "#2"), ["#2", "#3", "#4"]);
        assert_eq!(keys_between(&lines, "#9", "#2"), ["#2"]);
        assert!(keys_between(&lines, "#2", "#9").is_empty());
    }

    #[test]
    fn draws_the_newest_lines_or_a_window_around_a_line_jumped_to() {
        assert_eq!(drawn_range(100, 1500, None), (0, 100));
        assert_eq!(drawn_range(5000, 1500, None), (3500, 5000));
        assert_eq!(drawn_range(5000, 1500, Some(4000)), (3500, 5000));
        assert_eq!(drawn_range(5000, 1500, Some(1000)), (625, 2125));
        assert_eq!(drawn_range(5000, 1500, Some(100)), (0, 1500));
        assert_eq!(drawn_range(5000, 1500, Some(-1)), (3500, 5000));
    }

    #[test]
    fn keeps_the_smallest_epoch_estimate() {
        let mut e: Option<i64> = None;
        e = Some(next_epoch(e, 1000, 1_000_000_050)); // 50 ms late
        assert_eq!(e, Some(999_999_050));
        e = Some(next_epoch(e, 2000, 1_000_001_010)); // 10 ms late
        assert_eq!(e, Some(999_999_010));
        e = Some(next_epoch(e, 3000, 1_000_002_100)); // later again: kept
        assert_eq!(e, Some(999_999_010));
    }

    #[test]
    fn formats_the_local_time() {
        let epoch = Local.with_ymd_and_hms(2026, 10, 17, 13, 4, 5).earliest().unwrap().timestamp_millis() + 6;
        assert_eq!(time_of_day(0, epoch), "13:04:05.006");
        assert_eq!(time_of_day(3_600_000 + 994, epoch), "14:04:06.000");
    }

    #[test]
    fn takes_the_time_of_day_from_the_cores_clock_past_midnight_too() {
        let at = ((23 * 60 + 59) * 60 + 58) * 1000 + 500; // 23:59:58.500 at time 0
        assert_eq!(core_time_of_day(0, at), "23:59:58.500");
        assert_eq!(core_time_of_day(1_499, at), "23:59:59.999");
        assert_eq!(core_time_of_day(1_500, at), "00:00:00.000");
        assert_eq!(core_time_of_day(86_400_000 + 3_723_004, 0), "01:02:03.004");
    }
}
