//! Which subscription (diag.subscribe) the window sends: views ask for what
//! they show while they live; the window asks for nothing while none of its
//! windows shows. The backend merges both windows' subscriptions.

use ipc::LogLevel;

/// What the views ask for now
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Wants {
    pub link: bool,
    pub receiver: bool,
    /// The most verbose level asked for
    pub logs: Option<LogLevel>,
}

/// One subscription: what diag.subscribe asks for
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SubKey {
    pub logs: Option<LogLevel>,
    pub link: bool,
    pub receiver: bool,
}

/// What to do for a subscription that changed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubPlan {
    pub key: SubKey,
    /// Samples from an earlier subscription would join the new ones across
    /// the gap (a step in the charts, the drops meanwhile in one sample):
    /// the link samples start afresh
    pub clear_link: bool,
    /// Lines logged while nobody listened (the window was closed, or opened
    /// later): once subscribed, ask the core for its kept lines at this level
    /// (only when the level rose)
    pub recent_logs: Option<LogLevel>,
}

/// What was sent last
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SubState {
    pub last_sent: Option<SubKey>,
    /// The log level of the last subscription sent
    pub last_logs: Option<LogLevel>,
    /// Whether the last subscription sent asked for link samples
    pub link_on: bool,
}

impl SubState {
    /// The subscription to send for `wants` -> None when nothing changed or
    /// the core is not connected. `hidden`: no window of ours shows, so it
    /// asks for nothing.
    pub fn plan(&self, wants: Wants, hidden: bool, connected: bool) -> Option<SubPlan> {
        let key = if hidden {
            SubKey::default()
        } else {
            SubKey { logs: wants.logs, link: wants.link, receiver: wants.receiver }
        };
        if self.last_sent == Some(key) || !connected {
            return None;
        }
        let rose = |l: &LogLevel| self.last_logs.is_none_or(|b| *l > b);
        Some(SubPlan { key, clear_link: key.link && !self.link_on, recent_logs: key.logs.filter(rose) })
    }

    /// `plan` is being sent
    pub fn sent(&mut self, plan: &SubPlan) {
        self.last_sent = Some(plan.key);
        self.last_logs = plan.key.logs;
        self.link_on = plan.key.link;
    }

    /// A fresh connection, or a subscription that failed: the next one is
    /// sent whatever it asks for
    pub fn reset(&mut self) {
        *self = SubState::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wants(logs: Option<LogLevel>, link: bool, receiver: bool) -> Wants {
        Wants { link, receiver, logs }
    }

    fn key(logs: Option<LogLevel>, link: bool, receiver: bool) -> SubKey {
        SubKey { logs, link, receiver }
    }

    /// Plans and records what would be sent
    fn send(s: &mut SubState, w: Wants, hidden: bool) -> Option<SubPlan> {
        let plan = s.plan(w, hidden, true)?;
        s.sent(&plan);
        Some(plan)
    }

    #[test]
    fn merges_subscriptions_once_connected() {
        let mut s = SubState::default();
        // Link, info and debug logs wanted; not connected yet: nothing sent
        let w = wants(Some(LogLevel::Debug), true, false);
        assert_eq!(s.plan(w, false, false), None);
        // Connected: the most verbose level, and the kept lines at it
        let first = send(&mut s, w, false).unwrap();
        assert_eq!(first.key, key(Some(LogLevel::Debug), true, false));
        assert_eq!(first.recent_logs, Some(LogLevel::Debug));
        // Debug and link released
        let second = send(&mut s, wants(Some(LogLevel::Info), false, false), false).unwrap();
        assert_eq!(second.key, key(Some(LogLevel::Info), false, false));
        assert_eq!(second.recent_logs, None);
        // The logs released too
        let third = send(&mut s, Wants::default(), false).unwrap();
        assert_eq!(third.key, key(None, false, false));
        assert_eq!(third.recent_logs, None);
    }

    #[test]
    fn sends_nothing_when_nothing_changed() {
        let mut s = SubState::default();
        let w = wants(Some(LogLevel::Info), true, false);
        send(&mut s, w, false).unwrap();
        assert_eq!(s.plan(w, false, true), None);
    }

    #[test]
    fn starts_the_link_samples_afresh_when_asked_for_again() {
        let mut s = SubState::default();
        let link = wants(None, true, false);
        // The first time: nothing to join
        assert!(send(&mut s, link, false).unwrap().clear_link);
        // Another view taking over in the same update: nothing is sent, the
        // samples stay
        assert_eq!(s.plan(link, false, true), None);
        // Released: they stay too
        let off = send(&mut s, Wants::default(), false).unwrap();
        assert!(!off.clear_link);
        assert!(!s.link_on);
        // Asked for again later: the old ones would join across the gap
        assert!(send(&mut s, link, false).unwrap().clear_link);
    }

    #[test]
    fn follows_the_receiver_test_while_wanted() {
        let mut s = SubState::default();
        let on = send(&mut s, wants(None, false, true), false).unwrap();
        assert_eq!(on.key, key(None, false, true));
        let off = send(&mut s, Wants::default(), false).unwrap();
        assert_eq!(off.key, key(None, false, false));
    }

    #[test]
    fn asks_for_recent_logs_only_when_the_level_rises() {
        let mut s = SubState::default();
        assert_eq!(
            send(&mut s, wants(Some(LogLevel::Info), false, false), false).unwrap().recent_logs,
            Some(LogLevel::Info)
        );
        assert_eq!(
            send(&mut s, wants(Some(LogLevel::Debug), false, false), false).unwrap().recent_logs,
            Some(LogLevel::Debug)
        );
        assert_eq!(send(&mut s, wants(Some(LogLevel::Warn), false, false), false).unwrap().recent_logs, None);
        // A failed subscription forgets what was sent: asked again
        s.reset();
        assert_eq!(
            send(&mut s, wants(Some(LogLevel::Warn), false, false), false).unwrap().recent_logs,
            Some(LogLevel::Warn)
        );
    }

    #[test]
    fn asks_for_nothing_while_hidden() {
        let mut s = SubState::default();
        let w = wants(Some(LogLevel::Info), true, true);
        send(&mut s, w, false).unwrap();
        // Every window minimized: an empty subscription
        let hidden = send(&mut s, w, true).unwrap();
        assert_eq!(hidden.key, key(None, false, false));
        assert_eq!(hidden.recent_logs, None);
        assert!(!hidden.clear_link);
        // Still hidden: nothing more
        assert_eq!(s.plan(w, true, true), None);
        // Shown again: the samples start afresh, the lines missed meanwhile come
        let shown = send(&mut s, w, false).unwrap();
        assert_eq!(shown.key, key(Some(LogLevel::Info), true, true));
        assert!(shown.clear_link);
        assert_eq!(shown.recent_logs, Some(LogLevel::Info));
    }
}
