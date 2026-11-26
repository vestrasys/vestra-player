//! The host's backstop against a plugin's request flood: each plugin may
//! have at most [`MAX_UNANSWERED_REQUESTS`] request lines and
//! [`MAX_UNANSWERED_BYTES`] of request bytes unanswered; a request past
//! either is answered at once with `limit` on the reader thread, never
//! queued for the UI thread ("Request
//! limits").
//!
//! Counted per plugin (an own process's, or one shared-runner member's),
//! from the reader thread that read the line until the host's answer
//! (`Host::respond`, [`Unanswered::answered`]). A request the host never
//! answers because the plugin stopped goes with its counter: a restart is a
//! new plugin generation with a counter of its own. Notifications (lines
//! without an `id`) are not counted: there is no answer to decrement on and
//! no id to refuse with, and what a plugin may notify is two lines per run
//! (`runtime.ready`, `runtime.failed`); a flood of them is the UI thread's
//! drain budget's (`host::events::DRAIN_BUDGET`).
#![forbid(unsafe_code)]

use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

/// Request lines one plugin may have unanswered (user ruling 2026-11-11,
/// after measuring the built-ins): the SDK keeps at most 256 in flight and
/// queues the rest itself, so a plugin on the SDK never reaches this; it
/// stops clients that bypass the SDK.
///
/// The 64 lines over the SDK's 256 are for the requests the SDK never
/// queues (its `UNQUEUED_METHODS`: answers to the player's own calls, which
/// a held request may be waiting for), and their number is the player's:
/// `sourceResolvers.respond` answers one of at most 32 waiting resolutions
/// (`playback/source_backend.rs`), `producer.ackInput` the front of at most
/// `CAPACITY` (32) queued input notices (`playback/producer_input_delivery.rs`;
/// the SDK acknowledges one at a time) and `producer.ack` the one pending
/// control. Raising one of those bounds, or a new unqueued method in the
/// SDK, needs this margin checked again; the SDK agreement test
/// (`tests/sdk/host-agreement.test.ts`) reads these figures and fails first.
pub const MAX_UNANSWERED_REQUESTS: usize = 320;
/// Request bytes (whole lines as read, newline included) one plugin may
/// have unanswered (same ruling): four of the largest lines
/// (`MAX_MESSAGE_BYTES`, 1 MiB).
pub const MAX_UNANSWERED_BYTES: usize = 4 * 1024 * 1024;
/// Refusals of one plugin's that may wait to be written to it. A client
/// that floods without reading its input gets no more queued (the excess
/// is dropped unanswered), so its refusals never fill the queue that its
/// answers and events share (`OUTBOUND_CAPACITY`, the shared runner's
/// `MEMBER_QUEUE`), which would stop the plugin or restart the runner.
pub const REFUSALS_QUEUED: usize = 256;

/// One plugin's unanswered requests, shared by its reader thread (which
/// counts and refuses), the Host (which answers) and its writer (which
/// counts refusals written).
pub struct Unanswered {
    /// The manifest id, for the log line.
    plugin: String,
    state: Mutex<State>,
    /// Refusals queued and not yet written ([`REFUSALS_QUEUED`]).
    refusals: Arc<AtomicUsize>,
}

#[derive(Default)]
struct State {
    lines: usize,
    bytes: usize,
    /// By request id: how many lines with it are unanswered and their
    /// bytes. A plugin on the SDK never repeats an id; a client that does
    /// has each answer take one line and an even share of the bytes.
    ids: HashMap<u64, (u32, usize)>,
    /// Refusing since the log line; over once the plugin is back under half
    /// of both limits ([`Unanswered::answered`]), so the next burst logs
    /// again even while a request stays pending (an open dialog's), and a
    /// flood held at the limit logs once rather than at every answer.
    refusing: bool,
}

/// What [`Unanswered::admit`] makes of a line.
#[derive(Debug, PartialEq)]
pub enum Admission {
    /// A request within the limits, counted, or a line that is no request:
    /// for the UI thread.
    Forward,
    /// A request over a limit: this answer (`{ id, error }`) goes back at
    /// once; the request is not forwarded.
    Refuse(Value),
    /// Over a limit, and [`REFUSALS_QUEUED`] refusals wait already: the
    /// request is dropped unanswered.
    Drop,
}

impl Unanswered {
    pub fn new(plugin: &str) -> Arc<Self> {
        Arc::new(Self {
            plugin: plugin.to_owned(),
            state: Mutex::new(State::default()),
            refusals: Arc::new(AtomicUsize::new(0)),
        })
    }
    /// The plugin's manifest id.
    pub fn plugin(&self) -> &str {
        &self.plugin
    }
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
    /// A line of `bytes` the plugin wrote, parsed as `message`: a request
    /// (an `id`, no `event`, as `Host::handle` tells them apart) is counted
    /// or refused.
    pub fn admit(&self, message: &Value, bytes: usize) -> Admission {
        let Some(id) = message.get("id").and_then(Value::as_u64) else {
            return Admission::Forward;
        };
        if message.get("event").is_some() {
            return Admission::Forward;
        }
        let mut state = self.state();
        if state.lines < MAX_UNANSWERED_REQUESTS && state.bytes + bytes <= MAX_UNANSWERED_BYTES {
            state.lines += 1;
            state.bytes += bytes;
            let entry = state.ids.entry(id).or_default();
            entry.0 += 1;
            entry.1 += bytes;
            return Admission::Forward;
        }
        if !state.refusing {
            state.refusing = true;
            logged!(
                "{}: {} requests ({} KiB) unanswered, at the limit of {} and {} MiB: \
                 further requests are refused (limit) until they are answered",
                self.plugin,
                state.lines,
                state.bytes / 1024,
                MAX_UNANSWERED_REQUESTS,
                MAX_UNANSWERED_BYTES >> 20
            );
        }
        drop(state);
        if self.refusals.fetch_add(1, Ordering::AcqRel) >= REFUSALS_QUEUED {
            self.refusals.fetch_sub(1, Ordering::AcqRel);
            return Admission::Drop;
        }
        let error = crate::host::RpcError::new(
            "limit",
            format!(
                "too many unanswered requests: at most {MAX_UNANSWERED_REQUESTS} and \
                 {} MiB per plugin",
                MAX_UNANSWERED_BYTES >> 20
            ),
        );
        Admission::Refuse(json!({"id": id, "error": error}))
    }
    /// The counter of queued refusals, decremented by the writer once it
    /// wrote one (or by the sender when it could not be queued).
    pub fn refusals(&self) -> &Arc<AtomicUsize> {
        &self.refusals
    }
    /// The host answered request `id`: it no longer counts. An id not
    /// counted (refused, or not this plugin's) changes nothing.
    pub fn answered(&self, id: u64) {
        let mut state = self.state();
        let Some(entry) = state.ids.get_mut(&id) else {
            return;
        };
        let share = entry.1 / entry.0 as usize;
        entry.0 -= 1;
        entry.1 -= share;
        if entry.0 == 0 {
            state.ids.remove(&id);
        }
        state.lines -= 1;
        state.bytes -= share;
        if state.lines <= MAX_UNANSWERED_REQUESTS / 2 && state.bytes <= MAX_UNANSWERED_BYTES / 2 {
            state.refusing = false;
        }
    }
    /// Unanswered request lines and bytes.
    #[cfg(test)]
    pub fn counts(&self) -> (usize, usize) {
        let state = self.state();
        (state.lines, state.bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(id: u64) -> Value {
        json!({"id": id, "method": "ui.set", "params": {}})
    }

    #[test]
    fn requests_count_until_answered_and_past_the_limit_are_refused() {
        let plugin = Unanswered::new("a.b");
        for id in 0..MAX_UNANSWERED_REQUESTS as u64 {
            assert_eq!(plugin.admit(&request(id), 100), Admission::Forward);
        }
        assert_eq!(plugin.counts(), (MAX_UNANSWERED_REQUESTS, 32_000));
        let Admission::Refuse(answer) = plugin.admit(&request(999), 100) else {
            panic!("not refused");
        };
        assert_eq!(answer["id"], 999);
        assert_eq!(answer["error"]["code"], "limit");
        assert!(answer.get("result").is_none());
        // Not counted, and its answer frees nothing.
        assert_eq!(plugin.counts(), (MAX_UNANSWERED_REQUESTS, 32_000));
        plugin.refusals().fetch_sub(1, Ordering::AcqRel);
        plugin.answered(999);
        assert_eq!(plugin.counts(), (MAX_UNANSWERED_REQUESTS, 32_000));
        // One answered: one more in.
        plugin.answered(7);
        assert_eq!(plugin.counts(), (MAX_UNANSWERED_REQUESTS - 1, 31_900));
        assert_eq!(plugin.admit(&request(1000), 100), Admission::Forward);
        assert!(matches!(
            plugin.admit(&request(1001), 100),
            Admission::Refuse(_)
        ));
        plugin.refusals().fetch_sub(1, Ordering::AcqRel);
        // A second answer to the same id changes nothing.
        plugin.answered(7);
        assert_eq!(plugin.counts(), (MAX_UNANSWERED_REQUESTS, 32_000));
        for id in (0..MAX_UNANSWERED_REQUESTS as u64).chain([1000]) {
            plugin.answered(id);
        }
        assert_eq!(plugin.counts(), (0, 0));
        assert!(!plugin.state().refusing);
    }

    #[test]
    fn a_burst_ends_under_half_the_limits_though_a_request_stays_pending() {
        let plugin = Unanswered::new("a.b");
        // Request 0 stays pending throughout (an open dialog's).
        for id in 0..MAX_UNANSWERED_REQUESTS as u64 {
            plugin.admit(&request(id), 10);
        }
        let refuse = |id| {
            assert!(matches!(
                plugin.admit(&request(id), 10),
                Admission::Refuse(_)
            ));
            plugin.refusals().fetch_sub(1, Ordering::AcqRel);
        };
        refuse(1000);
        assert!(plugin.state().refusing);
        // Answers down to just over half: still the same burst, so a flood
        // held at the limit logs once.
        let half = MAX_UNANSWERED_REQUESTS as u64 / 2;
        for id in half + 1..MAX_UNANSWERED_REQUESTS as u64 {
            plugin.answered(id);
        }
        assert_eq!(plugin.counts().0, half as usize + 1);
        assert!(plugin.state().refusing);
        plugin.answered(half);
        assert!(!plugin.state().refusing);
        // The next burst logs again, though request 0 was never answered.
        for id in 2000..2000 + (MAX_UNANSWERED_REQUESTS as u64 - half) {
            plugin.admit(&request(id), 10);
        }
        refuse(3000);
        assert!(plugin.state().refusing);
        assert_eq!(plugin.counts().0, MAX_UNANSWERED_REQUESTS);
    }

    #[test]
    fn bytes_count_too() {
        let plugin = Unanswered::new("a.b");
        let line = 1024 * 1024;
        for id in 0..4 {
            assert_eq!(plugin.admit(&request(id), line), Admission::Forward);
        }
        // Four lines, 4 MiB: one byte more is refused.
        assert!(matches!(plugin.admit(&request(4), 1), Admission::Refuse(_)));
        plugin.refusals().fetch_sub(1, Ordering::AcqRel);
        plugin.answered(0);
        assert_eq!(plugin.counts(), (3, 3 * line));
        assert!(matches!(
            plugin.admit(&request(5), line + 1),
            Admission::Refuse(_)
        ));
        plugin.refusals().fetch_sub(1, Ordering::AcqRel);
        assert_eq!(plugin.admit(&request(6), line), Admission::Forward);
    }

    #[test]
    fn notifications_are_not_counted_and_a_repeated_id_counts_each_line() {
        let plugin = Unanswered::new("a.b");
        for _ in 0..1000 {
            let ready = json!({"event": "runtime.ready", "data": {}});
            assert_eq!(plugin.admit(&ready, 50), Admission::Forward);
            // An `event` line is one, whatever else it has.
            assert_eq!(
                plugin.admit(&json!({"event": "x", "id": 1}), 50),
                Admission::Forward
            );
        }
        assert_eq!(plugin.counts(), (0, 0));
        for bytes in [10, 30] {
            assert_eq!(plugin.admit(&request(3), bytes), Admission::Forward);
        }
        assert_eq!(plugin.counts(), (2, 40));
        plugin.answered(3);
        assert_eq!(plugin.counts(), (1, 20));
        plugin.answered(3);
        assert_eq!(plugin.counts(), (0, 0));
        plugin.answered(3);
        assert_eq!(plugin.counts(), (0, 0));
    }

    #[test]
    fn refusals_waiting_to_be_written_are_capped() {
        let plugin = Unanswered::new("a.b");
        for id in 0..MAX_UNANSWERED_REQUESTS as u64 {
            plugin.admit(&request(id), 10);
        }
        for id in 0..REFUSALS_QUEUED as u64 {
            assert!(matches!(
                plugin.admit(&request(1000 + id), 10),
                Admission::Refuse(_)
            ));
        }
        assert_eq!(plugin.admit(&request(9999), 10), Admission::Drop);
        // One written: one more refusal.
        plugin.refusals().fetch_sub(1, Ordering::AcqRel);
        assert!(matches!(
            plugin.admit(&request(10_000), 10),
            Admission::Refuse(_)
        ));
        assert_eq!(plugin.refusals().load(Ordering::Acquire), REFUSALS_QUEUED);
    }
}
