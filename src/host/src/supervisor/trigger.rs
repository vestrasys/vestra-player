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
    // B3: a full backlog or lane.
    #[cfg_attr(not(test), allow(dead_code))]
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
    // B4: a crash hold (T20).
    #[cfg_attr(not(test), allow(dead_code))]
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
    /// The payload of each held trigger, as (event name, data), in order.
    fn order(backlog: &VecDeque<Trigger<serde_json::Value>>) -> Vec<(String, serde_json::Value)> {
        backlog
            .iter()
            .map(|t| {
                (
                    t.payload["event"].as_str().unwrap().to_owned(),
                    t.payload["data"].clone(),
                )
            })
            .collect()
    }
    fn message(event: &str, data: serde_json::Value) -> serde_json::Value {
        serde_json::json!({"event": event, "data": data})
    }

    /// Review M1 (MOVE of `lazy.rs` `held_events_keep_their_order_and_only_
    /// the_newest_of_each`): a woken plugin receives wake events in order,
    /// only the newest of each, after the trigger that came before them.
    #[test]
    fn held_events_keep_their_order_and_only_the_newest_of_each() {
        use serde_json::json;
        let mut queue = VecDeque::from([Trigger::interactive(message(
            "commands.execute",
            json!({"id":"a"}),
        ))]);
        let event = |name: &str, data| Trigger::event(name, message(name, data));
        hold(&mut queue, event("packages.changed", json!(1)));
        hold(&mut queue, event("appearance.changed", json!("x")));
        hold(&mut queue, event("packages.changed", json!(2)));
        assert_eq!(
            order(&queue),
            [
                ("commands.execute".to_owned(), json!({"id":"a"})),
                ("appearance.changed".to_owned(), json!("x")),
                ("packages.changed".to_owned(), json!(2)),
            ]
        );
    }

    /// B0 (MOVE of `lazy.rs` `b0_hold_event_replaces_only_its_own_event`):
    /// holding a wake event replaces only an older message of its own event
    /// name (newest wins, at the end) and keeps every other held message in
    /// its order, the `source.resolveCancel` that follows its request
    /// included. What `wake` holds end to end: tests/lifecycle_baseline.rs.
    #[test]
    fn b0_hold_event_replaces_only_its_own_event() {
        use serde_json::json;
        let mut queue = VecDeque::from([Trigger::interactive(message(
            "commands.execute",
            json!({"id":"a"}),
        ))]);
        hold(
            &mut queue,
            Trigger::event("packages.changed", message("packages.changed", json!(1))),
        );
        hold(
            &mut queue,
            Trigger::interactive(message("commands.execute", json!({"id":"b"}))),
        );
        hold(
            &mut queue,
            Trigger::interactive(message("source.resolveRequest", json!({"request":1}))),
        );
        hold(
            &mut queue,
            Trigger::follow(message("source.resolveCancel", json!({"request":1}))),
        );
        hold(
            &mut queue,
            Trigger::event("packages.changed", message("packages.changed", json!(2))),
        );
        assert_eq!(
            order(&queue),
            [
                ("commands.execute".to_owned(), json!({"id":"a"})),
                ("commands.execute".to_owned(), json!({"id":"b"})),
                ("source.resolveRequest".to_owned(), json!({"request":1})),
                ("source.resolveCancel".to_owned(), json!({"request":1})),
                ("packages.changed".to_owned(), json!(2)),
            ]
        );
    }
}
