//! Held triggers (systemd socket activation: the listening socket's
//! backlog). MOVE `lazy::hold_event` (only wake events merge, newest
//! wins), bounds and explicit answers (A9, §4.6).
#![forbid(unsafe_code)]

use std::collections::VecDeque;

use super::job::Priority;
use super::unit::{Ms, Pid};

/// What a trigger is (today: `TRIGGERS` / `WAKE_EVENTS` / the
/// `source.resolveCancel` that follows its request).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TriggerKind {
    /// `commands.execute`, `source.resolveRequest`, `drops.drop`,
    /// `ui.callback`, a settings page shown: answered when it cannot be
    /// delivered.
    Interactive,
    /// A wake-event broadcast, by event name: merged (newest wins),
    /// dropped silently.
    Event(String),
    /// A message that follows a held trigger (`source.resolveCancel`): held
    /// only behind one, never starts anything.
    Follow,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trigger<P> {
    pub kind: TriggerKind,
    pub payload: P,
}
impl<P> Trigger<P> {
    pub fn interactive(payload: P) -> Self {
        Self {
            kind: TriggerKind::Interactive,
            payload,
        }
    }
    pub fn event(name: impl Into<String>, payload: P) -> Self {
        Self {
            kind: TriggerKind::Event(name.into()),
            payload,
        }
    }
    pub fn follow(payload: P) -> Self {
        Self {
            kind: TriggerKind::Follow,
            payload,
        }
    }
    /// The start priority it asks for (§4.4).
    pub fn priority(&self) -> Priority {
        match self.kind {
            TriggerKind::Interactive => Priority::Interactive,
            TriggerKind::Event(_) => Priority::Event,
            TriggerKind::Follow => Priority::Background,
        }
    }
    /// Answered (not dropped silently) when it cannot be delivered.
    pub fn answered(&self) -> bool {
        self.kind == TriggerKind::Interactive
    }
}

/// Result of `wake` (§5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    /// The unit is active with that pid: send it now.
    Now,
    /// A start job is installed; the trigger is in the backlog.
    Held,
    /// A new start job was enqueued; the trigger is in the backlog.
    Started,
    /// An old pid of a unit that moved on: send to this pid.
    Forward(Pid),
    /// Disabled / failed / removed unit, no entry point, or a muted event.
    Dropped,
    /// Bounds (backlog, lane) or a hold: answered per §4.6.
    Refused,
}

/// Why an `Answer` replies failed (§4.6, i18n texts in the adapter).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnswerReason {
    /// `"<name> is busy"` (backlog or lane full).
    Busy,
    /// `"<name> could not start"`.
    CouldNotStart,
    /// `"<name> is disabled"`.
    Disabled,
    /// Uninstalled.
    Removed,
    /// The start was canceled by another stop (reload of a removed image,
    /// quit...).
    Stopped,
    /// In standby with a restart hold (T20, B4): until then.
    Held { until: Ms },
}

/// Holds `trigger` (MOVE `hold_event`: a wake event replaces a held one of
/// the same name). Whether it was added without growing the backlog.
pub(super) fn hold<P>(backlog: &mut VecDeque<Trigger<P>>, trigger: Trigger<P>) -> bool {
    let merged = if let TriggerKind::Event(name) = &trigger.kind {
        let before = backlog.len();
        backlog.retain(|held| !matches!(&held.kind, TriggerKind::Event(other) if other == name));
        backlog.len() < before
    } else {
        false
    };
    backlog.push_back(trigger);
    merged
}

/// Whether holding `trigger` would grow the backlog past `max`.
pub(super) fn would_overflow<P>(
    backlog: &VecDeque<Trigger<P>>,
    trigger: &Trigger<P>,
    max: Option<usize>,
) -> bool {
    let Some(max) = max else {
        return false;
    };
    if let TriggerKind::Event(name) = &trigger.kind
        && backlog
            .iter()
            .any(|held| matches!(&held.kind, TriggerKind::Event(other) if other == name))
    {
        return false;
    }
    backlog.len() >= max
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_wake_events_merge_newest_wins() {
        let mut backlog = VecDeque::new();
        hold(&mut backlog, Trigger::event("a", 1));
        hold(&mut backlog, Trigger::interactive(2));
        hold(&mut backlog, Trigger::interactive(3));
        assert!(hold(&mut backlog, Trigger::event("a", 4)));
        let payloads = backlog.iter().map(|t| t.payload).collect::<Vec<_>>();
        assert_eq!(payloads, [2, 3, 4]);
        assert!(!would_overflow(&backlog, &Trigger::event("a", 5), Some(3)));
        assert!(would_overflow(&backlog, &Trigger::interactive(5), Some(3)));
        assert!(!would_overflow(&backlog, &Trigger::interactive(5), None));
    }
}
