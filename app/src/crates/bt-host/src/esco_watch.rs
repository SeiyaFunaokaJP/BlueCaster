//! The stream's rate after a voice link (#86). An RTL8761BU with a
//! WH-1000XM4 came out of some eSCO links completing less than half the ACL
//! packets of before (83 of 187 a second), with every packet type allowed
//! and 5-slot packets again. Neither a short eSCO link nor a stream restart
//! put it right; the next whole voice link did, and once it came right by
//! itself after 26 s. [`EscoWatch`] spots the slow rate after a voice link:
//! the encoder lowers its bit rate ([`crate::LinkStats::slow_link`]) and
//! tries the full one again after [`PROBE_AFTER`], longer each time it is
//! still slow.

use std::time::{Duration, Instant};

/// How long after a voice link the rate is watched
pub const WATCH: Duration = Duration::from_secs(15);
/// The watch's tick
pub const TICK: Duration = Duration::from_secs(1);
/// How long the bit rate stays lowered before the full one is tried again,
/// by the times it was lowered in a row (the last one from then on). Each
/// try that finds the link still slow costs about a second of audio, and
/// the slow rate was seen lasting minutes, until the next voice link.
pub const PROBE_AFTER: [Duration; 4] =
    [Duration::from_secs(20), Duration::from_secs(60), Duration::from_secs(120), Duration::from_secs(300)];
/// Seconds below [`SLOW_PERCENT`] of the rate before that count as slow
/// after a voice link; one while the full bit rate is tried again (it was
/// slow just before)
const SLOW_TICKS: u8 = 2;
/// Seconds at the rate before that end the watch
const GOOD_TICKS: u8 = 3;
/// Slow: below this share of the rate before the voice link
const SLOW_PERCENT: u64 = 60;
/// A stream part shorter than this gives no rate
const MIN_PART: Duration = Duration::from_secs(3);
/// Rates below this a second are no stream to compare with
const MIN_RATE: u64 = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing to do (another tick follows while [`EscoWatch::watching`])
    Wait,
    /// The rate is as before: the watch ends (after a lowered bit rate:
    /// the full one stays)
    Good { rate: u64 },
    /// Slow: the bit rate lowered until [`Verdict::Probe`]
    Lower { rate: u64, before: u64 },
    /// The full bit rate tried again, the rate watched
    Probe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    /// The rate of `acl` watched until `until`
    Watching {
        acl: u16,
        until: Instant,
    },
    /// The bit rate lowered on `acl` until `probe`
    Lowered {
        acl: u16,
        probe: Instant,
    },
}

#[derive(Debug)]
pub struct EscoWatch {
    /// The best rate (completed ACL packets a second) of the stream's
    /// parts with this configuration
    before: Option<u64>,
    /// Since when the stream runs, and the completed packets then
    part: Option<(Instant, u64)>,
    state: State,
    /// The last tick's time and completed packets
    last: Option<(Instant, u64)>,
    slow: u8,
    good: u8,
    /// Times lowered in a row
    lowered: usize,
}

impl Default for EscoWatch {
    fn default() -> Self {
        EscoWatch { before: None, part: None, state: State::Idle, last: None, slow: 0, good: 0, lowered: 0 }
    }
}

impl EscoWatch {
    /// Another stream configuration: the rate before means nothing now
    pub fn reconfigured(&mut self) {
        self.before = None;
        self.part = None;
    }

    /// The stream started (`completed`: on its link so far)
    pub fn stream_started(&mut self, now: Instant, completed: u64) {
        self.part = Some((now, completed));
        self.last = None;
    }

    /// The stream stopped: its part's rate counts towards the one before
    /// (not while lowered: that rate is the encoder's, not the link's)
    pub fn stream_stopped(&mut self, now: Instant, completed: u64) {
        let Some((since, at)) = self.part.take() else { return };
        let took = now.saturating_duration_since(since);
        if took < MIN_PART || self.is_lowered() {
            return;
        }
        let rate = per_second(completed.saturating_sub(at), took);
        if rate >= MIN_RATE {
            self.before = Some(self.before.map_or(rate, |b| b.max(rate)));
        }
    }

    /// The voice link on `acl` went down; true when the watch starts (a
    /// tick to schedule). Whatever was lowered is over: the voice link may
    /// have put the link right.
    pub fn voice_link_ended(&mut self, now: Instant, acl: u16) -> bool {
        self.lowered = 0;
        self.watch(now, acl)
    }

    fn watch(&mut self, now: Instant, acl: u16) -> bool {
        self.last = None;
        self.slow = 0;
        self.good = 0;
        if self.before.is_none() {
            self.state = State::Idle;
            return false;
        }
        self.state = State::Watching { acl, until: now + WATCH };
        true
    }

    /// The link watched or lowered on, if any
    pub fn watching(&self) -> Option<u16> {
        match self.state {
            State::Idle => None,
            State::Watching { acl, .. } | State::Lowered { acl, .. } => Some(acl),
        }
    }

    /// The bit rate is lowered now
    pub fn is_lowered(&self) -> bool {
        matches!(self.state, State::Lowered { .. })
    }

    /// The link went down or the stream ended for good
    pub fn stop(&mut self) {
        self.state = State::Idle;
        self.last = None;
        self.lowered = 0;
    }

    /// A tick: `completed` on the watched link, None while it does not
    /// stream
    pub fn tick(&mut self, now: Instant, completed: Option<u64>) -> Verdict {
        let (acl, until) = match self.state {
            State::Idle => return Verdict::Wait,
            State::Lowered { acl, probe } => {
                if now < probe {
                    return Verdict::Wait;
                }
                self.watch(now, acl);
                return Verdict::Probe;
            }
            State::Watching { acl, until } => (acl, until),
        };
        if now >= until {
            self.stop();
            return Verdict::Wait;
        }
        let Some(completed) = completed else {
            self.last = None;
            return Verdict::Wait;
        };
        let Some((then, at)) = self.last.replace((now, completed)) else { return Verdict::Wait };
        let Some(before) = self.before else { return Verdict::Wait };
        let rate = per_second(completed.saturating_sub(at), now.saturating_duration_since(then));
        if rate * 100 < before * SLOW_PERCENT {
            self.good = 0;
            self.slow += 1;
            if self.slow < if self.lowered > 0 { 1 } else { SLOW_TICKS } {
                return Verdict::Wait;
            }
            let wait = PROBE_AFTER[self.lowered.min(PROBE_AFTER.len() - 1)];
            self.lowered += 1;
            self.state = State::Lowered { acl, probe: now + wait };
            self.last = None;
            return Verdict::Lower { rate, before };
        }
        self.slow = 0;
        self.good += 1;
        if self.good >= GOOD_TICKS {
            self.stop();
            return Verdict::Good { rate };
        }
        Verdict::Wait
    }
}

fn per_second(n: u64, d: Duration) -> u64 {
    let ms = d.as_millis().max(1) as u64;
    n * 1000 / ms
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Duration = Duration::from_secs(1);

    /// A stream of `rate` a second for `secs`, from `t` and `n`
    fn stream(w: &mut EscoWatch, t: &mut Instant, n: &mut u64, rate: u64, secs: u32) {
        w.stream_started(*t, *n);
        *t += S * secs;
        *n += rate * u64::from(secs);
        w.stream_stopped(*t, *n);
    }

    /// Ticks at `rate` a second until a verdict other than Wait (or the
    /// watch ends), with the ticks it took
    fn run(w: &mut EscoWatch, t: &mut Instant, n: &mut u64, rate: u64) -> (Verdict, u32) {
        for i in 0..400 {
            let v = w.tick(*t, Some(*n));
            if v != Verdict::Wait || w.watching().is_none() {
                return (v, i);
            }
            *t += S;
            *n += rate;
        }
        (Verdict::Wait, 400)
    }

    fn slow_after_a_voice_link() -> (EscoWatch, Instant, u64) {
        let (mut w, mut t, mut n) = (EscoWatch::default(), Instant::now(), 0);
        stream(&mut w, &mut t, &mut n, 187, 10);
        assert!(w.voice_link_ended(t, 1));
        w.stream_started(t, n);
        (w, t, n)
    }

    #[test]
    fn the_slow_rate_after_a_voice_link_lowers_the_bit_rate() {
        let (mut w, mut t, mut n) = slow_after_a_voice_link();
        let (v, ticks) = run(&mut w, &mut t, &mut n, 83);
        assert_eq!(v, Verdict::Lower { rate: 83, before: 187 });
        assert_eq!(ticks, 2, "two slow seconds after the first tick");
        assert!(w.is_lowered());
        assert_eq!(w.watching(), Some(1));
    }

    #[test]
    fn the_full_bit_rate_is_tried_again_later_each_time() {
        let (mut w, mut t, mut n) = slow_after_a_voice_link();
        for (i, wait) in [20, 60, 120, 300, 300].into_iter().enumerate() {
            let ticks = run(&mut w, &mut t, &mut n, 83).1;
            // Two slow seconds after a voice link, one when tried again
            assert_eq!(ticks, if i == 0 { 2 } else { 1 });
            assert!(w.is_lowered());
            let lowered_at = t;
            // Lowered: 62 packets a second (LDAC MQ), neither slow nor good
            assert_eq!(run(&mut w, &mut t, &mut n, 62).0, Verdict::Probe);
            assert_eq!(t - lowered_at, S * wait);
        }
        // The link came right: the full bit rate stays
        assert_eq!(run(&mut w, &mut t, &mut n, 187).0, Verdict::Good { rate: 187 });
        assert!(!w.is_lowered());
        assert_eq!(w.watching(), None);
    }

    #[test]
    fn a_voice_link_starts_the_waits_over() {
        let (mut w, mut t, mut n) = slow_after_a_voice_link();
        assert!(matches!(run(&mut w, &mut t, &mut n, 83).0, Verdict::Lower { .. }));
        assert_eq!(run(&mut w, &mut t, &mut n, 62).0, Verdict::Probe);
        assert!(matches!(run(&mut w, &mut t, &mut n, 83).0, Verdict::Lower { .. }));
        // Lowered for 40 s now; a voice link in between: 20 s again
        assert!(w.voice_link_ended(t, 1));
        assert!(!w.is_lowered());
        assert!(matches!(run(&mut w, &mut t, &mut n, 83).0, Verdict::Lower { .. }));
        let at = t;
        assert_eq!(run(&mut w, &mut t, &mut n, 62).0, Verdict::Probe);
        assert_eq!(t - at, S * 20);
    }

    #[test]
    fn the_rate_as_before_ends_the_watch() {
        let (mut w, mut t, mut n) = slow_after_a_voice_link();
        assert_eq!(run(&mut w, &mut t, &mut n, 185).0, Verdict::Good { rate: 185 });
    }

    #[test]
    fn the_rate_before_is_the_best_part_and_slow_or_lowered_parts_do_not_change_it() {
        let (mut w, mut t, mut n) = (EscoWatch::default(), Instant::now(), 0);
        stream(&mut w, &mut t, &mut n, 187, 10);
        stream(&mut w, &mut t, &mut n, 83, 10);
        stream(&mut w, &mut t, &mut n, 300, 1); // too short to count
        assert!(w.voice_link_ended(t, 1));
        assert!(matches!(run(&mut w, &mut t, &mut n, 83).0, Verdict::Lower { before: 187, .. }));
        // A part while lowered counts for nothing
        stream(&mut w, &mut t, &mut n, 300, 10);
        assert!(w.voice_link_ended(t, 1));
        assert!(matches!(run(&mut w, &mut t, &mut n, 83).0, Verdict::Lower { before: 187, .. }));
    }

    #[test]
    fn nothing_is_watched_without_a_rate_before() {
        let (mut w, mut t, mut n) = (EscoWatch::default(), Instant::now(), 0);
        assert!(!w.voice_link_ended(t, 1));
        stream(&mut w, &mut t, &mut n, 187, 10);
        w.reconfigured();
        assert!(!w.voice_link_ended(t, 1));
        assert_eq!(w.tick(t, Some(n)), Verdict::Wait);
    }

    #[test]
    fn seconds_without_the_stream_are_not_slow_and_the_watch_expires() {
        let (mut w, mut t, mut n) = (EscoWatch::default(), Instant::now(), 0);
        stream(&mut w, &mut t, &mut n, 187, 10);
        assert!(w.voice_link_ended(t, 1));
        for _ in 0..16 {
            assert_eq!(w.tick(t, None), Verdict::Wait);
            t += S;
        }
        assert_eq!(w.watching(), None);
    }
}
