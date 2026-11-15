//! The discrete source events (events-and-triggers.md §4.6, first batch):
//! `source.opening`, `source.ready` and `source.ended`, derived on the UI
//! thread from two consecutive `source.state` broadcasts. The playback
//! thread is unchanged: every phase change is pushed in the iteration it
//! happens (`backend.rs`, `push.due`), so each transition is seen here, and
//! one that two mpv events collapse into a single push (a load that is
//! ready, or failed, before the first push) is filled in.
//!
//! A *playthrough* is one load of a source: `stamp.generation` (the
//! `sourceId`) and the load `cycle`. It is in playback while its phase is
//! `loading` or `ready`. The rules, from `previous` to `next`:
//!
//! - **Ended**: a playthrough in playback that `next` does not continue in
//!   playback leaves it. Its `state` is that source's last state: `next`
//!   when `next` is the same playthrough (an end, an error: the snapshot
//!   that says so, with the position at the end), else `previous` (a
//!   replacement, a close: the last snapshot of it). `reason`: `natural`,
//!   `stopped`, `replaced`, `error` (with `error`, the failure text).
//! - **Opening**: `next` begins a playthrough: another source id, or the
//!   same one loading again (`player.restart()`: mpv starts the file
//!   again). A new cycle of the same source that is not loading is a seek
//!   away from the end (`session.rs` `confirm_seek_from_end`): it plays
//!   on, nothing opens.
//! - **Ready**: the playthrough reached `ready` (or passed it: a file that
//!   ended before the first push).
//!
//! For one playthrough the order is `opening`, `ready`, `ended`; a
//! replaced one's `ended` comes before the next one's `opening`. A
//! playthrough that plays again after its end (a seek back) leaves
//! playback again later and gets a second `ended`, as the stream showed it
//! end twice. `source.ended` with `reason: "quit"` is not derived here: the
//! quit sends it (`Core::quit_source_ended`).
#![forbid(unsafe_code)]

use serde_json::{Value, json};

/// The three names, in their order for one playthrough.
pub(crate) const OPENING: &str = "source.opening";
pub(crate) const READY: &str = "source.ready";
pub(crate) const ENDED: &str = "source.ended";

/// What one snapshot says, as far as the events need it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Facts<'a> {
    id: u64,
    cycle: u64,
    phase: &'a str,
    /// A source is open (`source` is not null).
    open: bool,
}

fn facts(state: &Value) -> Facts<'_> {
    Facts {
        id: state["stamp"]["generation"].as_u64().unwrap_or(0),
        cycle: state["cycle"].as_u64().unwrap_or(0),
        phase: state["phase"].as_str().unwrap_or("empty"),
        open: !state["source"].is_null(),
    }
}

impl Facts<'_> {
    fn in_playback(&self) -> bool {
        self.open && matches!(self.phase, "loading" | "ready")
    }
    fn same_playthrough(&self, other: &Self) -> bool {
        self.id == other.id && self.cycle == other.cycle
    }
}

/// `source.ended`'s data: the playthrough's last state, why it ended and,
/// for `error`, the failure text when there is one.
fn ended(state: &Value, reason: &str, error: Option<&str>) -> Value {
    let mut data = json!({ "state": state, "reason": reason });
    if let Some(error) = error {
        data["error"] = json!(error);
    }
    data
}

/// The reason and failure text of a playthrough that ended in `state`
/// (phase `ended` or `error`). mpv's other end reasons (`Unknown`: a quit
/// or redirect of mpv itself) read as `stopped`.
fn end_reason(state: &Value) -> (&'static str, Option<&str>) {
    let reason = &state["terminalReason"];
    if state["phase"] == "error" {
        return ("error", reason["failed"].as_str());
    }
    match reason.as_str() {
        Some("natural") => ("natural", None),
        Some("replaced") => ("replaced", None),
        _ => ("stopped", None),
    }
}

/// The events `next` adds to `previous`, in order, as (name, data).
/// `previous` is `Value::Null` before the first broadcast.
pub(crate) fn derive(previous: &Value, next: &Value) -> Vec<(&'static str, Value)> {
    let was = facts(previous);
    let now = facts(next);
    let mut events = Vec::new();
    // A seek away from the end: the same source, a new cycle, not loading.
    let resumed = was.open
        && was.id == now.id
        && was.cycle != now.cycle
        && was.phase == "ended"
        && now.phase != "loading";
    let begins = now.open && !now.same_playthrough(&was) && !resumed;
    if was.in_playback() && !(now.same_playthrough(&was) && now.in_playback()) {
        if now.same_playthrough(&was) && now.open {
            // It ended or failed: `next` says how, at its last position.
            let (reason, error) = end_reason(next);
            events.push((ENDED, ended(next, reason, error)));
        } else if !now.open && now.id == was.id {
            events.push((ENDED, ended(previous, "stopped", None)));
        } else {
            events.push((ENDED, ended(previous, "replaced", None)));
        }
    }
    if begins {
        let before = was.open.then(|| json!({ "sourceId": was.id }));
        events.push((
            OPENING,
            json!({ "state": next, "previous": before.unwrap_or(Value::Null) }),
        ));
        // Loaded before the first push: ready, or past it.
        if matches!(now.phase, "ready" | "ended") {
            events.push((READY, json!({ "state": next })));
        }
        if matches!(now.phase, "ended" | "error") {
            let (reason, error) = end_reason(next);
            events.push((ENDED, ended(next, reason, error)));
        }
    } else if now.same_playthrough(&was) && was.phase == "loading" && now.open {
        if matches!(now.phase, "ready" | "ended") {
            events.push((READY, json!({ "state": next })));
        }
        // Loading straight to its end within one push: ready, then ended
        // (the ended was added above, before it); keep the order.
        if now.phase == "ended"
            && let Some(at) = events.iter().position(|(name, _)| *name == ENDED)
        {
            let end = events.remove(at);
            events.push(end);
        }
    }
    events
}

/// `source.ended` with `reason: "quit"` for the source `state` is, when it
/// is in playback (the player quits; running plugins only).
pub(crate) fn at_quit(state: &Value) -> Option<Value> {
    facts(state)
        .in_playback()
        .then(|| ended(state, "quit", None))
}

/// The source id an event's data belongs to (the backlog merges
/// per-source events by name and source id, design §4.9).
pub(crate) fn source_id(data: &Value) -> u64 {
    data["state"]["stamp"]["generation"].as_u64().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wire snapshot: source id, cycle, phase, position.
    fn state(id: u64, cycle: u64, phase: &str, position: Option<f64>) -> Value {
        json!({
            "stamp": {"generation": id, "intentVersion": id + cycle},
            "cycle": cycle,
            "phase": phase,
            "source": if phase == "empty" { Value::Null } else { json!(format!("/m/{id}.mkv")) },
            "terminalReason": Value::Null,
            "position": position,
        })
    }
    fn with_reason(mut state: Value, reason: Value) -> Value {
        state["terminalReason"] = reason;
        state
    }
    /// The names of what a run of snapshots derives, each with its
    /// source id and, for `ended`, its reason.
    fn run(states: &[Value]) -> Vec<String> {
        let mut previous = Value::Null;
        let mut out = Vec::new();
        for next in states {
            for (name, data) in derive(&previous, next) {
                let id = source_id(&data);
                out.push(match name {
                    ENDED => format!("ended {id} {}", data["reason"].as_str().unwrap()),
                    _ => format!("{} {id}", name.trim_start_matches("source.")),
                });
            }
            previous = next.clone();
        }
        out
    }

    #[test]
    fn open_then_natural_end() {
        let end = with_reason(state(1, 1, "ended", Some(60.0)), json!("natural"));
        assert_eq!(
            run(&[
                state(0, 0, "empty", None),
                state(1, 1, "loading", None),
                state(1, 1, "ready", Some(0.0)),
                state(1, 1, "ready", Some(30.0)),
                end.clone(),
            ]),
            ["opening 1", "ready 1", "ended 1 natural"]
        );
        // The end's state is the snapshot that says so: its last position.
        let events = derive(&state(1, 1, "ready", Some(59.9)), &end);
        assert_eq!(events[0].1["state"]["position"], 60.0);
    }

    #[test]
    fn replace_ends_the_old_one_before_the_new_one_opens() {
        let events = derive(
            &state(1, 1, "ready", Some(42.5)),
            &state(2, 2, "loading", None),
        );
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].0, ENDED);
        assert_eq!(events[0].1["reason"], "replaced");
        // The old source's last state, with its last position.
        assert_eq!(events[0].1["state"]["position"], 42.5);
        assert_eq!(source_id(&events[0].1), 1);
        assert_eq!(events[1].0, OPENING);
        assert_eq!(events[1].1["previous"], json!({"sourceId": 1}));
        assert_eq!(
            run(&[
                state(1, 1, "loading", None),
                state(1, 1, "ready", Some(1.0)),
                state(2, 2, "loading", None),
                state(2, 2, "ready", Some(0.0)),
            ]),
            [
                "opening 1",
                "ready 1",
                "ended 1 replaced",
                "opening 2",
                "ready 2"
            ]
        );
    }

    #[test]
    fn a_failure_ends_with_its_text_and_no_ready() {
        let failed = with_reason(
            state(1, 1, "error", None),
            json!({"failed": "no such file"}),
        );
        assert_eq!(
            run(&[state(1, 1, "loading", None), failed.clone()]),
            ["opening 1", "ended 1 error"]
        );
        let events = derive(&state(1, 1, "loading", None), &failed);
        assert_eq!(events[0].1["error"], "no such file");
        // Failed before the first push: opening, then ended.
        assert_eq!(
            run(&[state(0, 0, "empty", None), failed]),
            ["opening 1", "ended 1 error"]
        );
    }

    #[test]
    fn stop_ends_with_the_last_state() {
        let closed = with_reason(state(1, 1, "empty", None), json!("stopped"));
        let events = derive(&state(1, 1, "ready", Some(12.0)), &closed);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].1["reason"], "stopped");
        assert_eq!(events[0].1["state"]["position"], 12.0);
        assert_eq!(events[0].1["state"]["phase"], "ready");
        // Closing an ended source ends nothing more.
        let end = with_reason(state(1, 1, "ended", Some(60.0)), json!("natural"));
        assert!(derive(&end, &closed).is_empty());
        // Nor opens anything.
        assert!(derive(&closed, &state(1, 1, "empty", None)).is_empty());
    }

    #[test]
    fn loaded_before_the_first_push_is_filled_in() {
        assert_eq!(
            run(&[state(0, 0, "empty", None), state(1, 1, "ready", Some(0.0))]),
            ["opening 1", "ready 1"]
        );
        // A replacement that is ready at once.
        assert_eq!(
            run(&[
                state(1, 1, "ready", Some(5.0)),
                state(2, 2, "ready", Some(0.0))
            ]),
            [
                "opening 1",
                "ready 1",
                "ended 1 replaced",
                "opening 2",
                "ready 2"
            ]
        );
        // Loading, then ended within one push: ready before ended.
        let end = with_reason(state(1, 1, "ended", Some(0.5)), json!("natural"));
        assert_eq!(
            run(&[state(1, 1, "loading", None), end]),
            ["opening 1", "ready 1", "ended 1 natural"]
        );
    }

    #[test]
    fn seek_from_the_end_plays_on_and_ends_again() {
        let end = with_reason(state(1, 1, "ended", Some(60.0)), json!("natural"));
        assert_eq!(
            run(&[
                state(1, 1, "loading", None),
                state(1, 1, "ready", Some(0.0)),
                end,
                state(1, 2, "ready", Some(10.0)),
                state(2, 3, "loading", None),
            ]),
            [
                "opening 1",
                "ready 1",
                "ended 1 natural",
                "ended 1 replaced",
                "opening 2"
            ]
        );
    }

    #[test]
    fn restart_is_a_new_playthrough_of_the_same_source() {
        assert_eq!(
            run(&[
                state(1, 1, "loading", None),
                state(1, 1, "ready", Some(30.0)),
                state(1, 2, "loading", None),
                state(1, 2, "ready", Some(0.0)),
            ]),
            [
                "opening 1",
                "ready 1",
                "ended 1 replaced",
                "opening 1",
                "ready 1"
            ]
        );
        let events = derive(
            &state(1, 1, "ready", Some(30.0)),
            &state(1, 2, "loading", None),
        );
        assert_eq!(events[1].1["previous"], json!({"sourceId": 1}));
    }

    #[test]
    fn repeated_snapshots_derive_nothing() {
        for phase in ["empty", "loading", "ready", "ended", "error"] {
            let same = state(1, 1, phase, Some(1.0));
            assert!(derive(&same, &same).is_empty(), "{phase}");
        }
        // Position and pause changes are not events.
        assert!(
            derive(
                &state(1, 1, "ready", Some(1.0)),
                &state(1, 1, "ready", Some(2.0))
            )
            .is_empty()
        );
    }

    #[test]
    fn quit_ends_only_a_source_in_playback() {
        let data = at_quit(&state(1, 1, "ready", Some(7.0))).unwrap();
        assert_eq!(data["reason"], "quit");
        assert_eq!(data["state"]["position"], 7.0);
        assert!(at_quit(&state(1, 1, "loading", None)).is_some());
        assert!(at_quit(&state(1, 1, "ended", Some(9.0))).is_none());
        assert!(at_quit(&state(1, 1, "error", None)).is_none());
        assert!(at_quit(&state(0, 0, "empty", None)).is_none());
        assert!(at_quit(&Value::Null).is_none());
    }

    #[test]
    fn an_unknown_end_reads_as_stopped() {
        let end = with_reason(state(1, 1, "ended", Some(3.0)), json!("unknown"));
        assert_eq!(
            derive(&state(1, 1, "ready", Some(2.0)), &end)[0].1["reason"],
            "stopped"
        );
        let stopped = with_reason(state(1, 1, "ended", Some(3.0)), json!("stopped"));
        assert_eq!(
            derive(&state(1, 1, "ready", Some(2.0)), &stopped)[0].1["reason"],
            "stopped"
        );
    }
}
