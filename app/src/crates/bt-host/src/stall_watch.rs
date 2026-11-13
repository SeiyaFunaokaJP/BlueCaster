//! A stream whose link stopped taking packets (#86). After a voice link, an
//! MT7925 with EarFun earbuds completed 28 media packets and then none for
//! 36 s while the link itself stayed up: the 8 packets the controller holds
//! filled its buffers, so nothing else could be sent either, not even the
//! AVDTP signaling to suspend or close the stream. Only dropping the link
//! frees them. [`StallWatch`] spots it: packets wait in the controller and
//! none completed for [`STALL_AFTER`]. A slow link still completes some each
//! second ([`crate::esco_watch`]).

use std::time::{Duration, Instant};

/// The watch's tick while streaming
pub const TICK: Duration = Duration::from_secs(1);
/// No packet completed for this long while some wait: the link is stalled.
/// A healthy stream completes one every few ms, a slow one dozens a second.
pub const STALL_AFTER: Duration = Duration::from_secs(5);

#[derive(Debug, Default)]
pub struct StallWatch {
    /// Since when nothing completed while packets waited, and the count then
    since: Option<(Instant, u64)>,
}

impl StallWatch {
    /// The stream started or stopped
    pub fn reset(&mut self) {
        self.since = None;
    }

    /// A tick: `completed` on the stream's link so far, `waiting` in the
    /// controller now. Some(how long) once the link stalled (then the watch
    /// starts over).
    pub fn tick(&mut self, now: Instant, completed: u64, waiting: u32) -> Option<Duration> {
        if waiting == 0 {
            self.since = None;
            return None;
        }
        match self.since {
            Some((then, at)) if at == completed => {
                let took = now.saturating_duration_since(then);
                if took < STALL_AFTER {
                    return None;
                }
                self.since = None;
                Some(took)
            }
            _ => {
                self.since = Some((now, completed));
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Duration = Duration::from_secs(1);

    #[test]
    fn packets_waiting_with_none_completed_stall_the_link() {
        let (mut w, mut t) = (StallWatch::default(), Instant::now());
        let mut stalled = None;
        for i in 0..10 {
            if let Some(d) = w.tick(t, 28, 8) {
                stalled = Some((i, d));
                break;
            }
            t += S;
        }
        assert_eq!(stalled, Some((5, STALL_AFTER)));
        // Reported once, then watched anew
        assert_eq!(w.tick(t, 28, 8), None);
    }

    #[test]
    fn a_slow_link_is_not_stalled() {
        let (mut w, mut t, mut n) = (StallWatch::default(), Instant::now(), 0);
        for _ in 0..60 {
            assert_eq!(w.tick(t, n, 6), None);
            t += S;
            n += 1;
        }
    }

    #[test]
    fn nothing_waiting_is_no_stall() {
        let (mut w, mut t) = (StallWatch::default(), Instant::now());
        for _ in 0..60 {
            assert_eq!(w.tick(t, 100, 0), None);
            t += S;
        }
        // Waiting from now on: the wait starts now, not with the first tick
        for _ in 0..5 {
            assert_eq!(w.tick(t, 100, 3), None);
            t += S;
        }
        assert_eq!(w.tick(t, 100, 3), Some(STALL_AFTER));
    }

    #[test]
    fn a_reset_starts_the_wait_over() {
        let (mut w, mut t) = (StallWatch::default(), Instant::now());
        for _ in 0..4 {
            assert_eq!(w.tick(t, 7, 8), None);
            t += S;
        }
        w.reset();
        for _ in 0..5 {
            assert_eq!(w.tick(t, 7, 8), None);
            t += S;
        }
        assert_eq!(w.tick(t, 7, 8), Some(STALL_AFTER));
    }
}
