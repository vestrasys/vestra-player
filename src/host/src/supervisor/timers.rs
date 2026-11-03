//! Deadline min-heap (systemd `sd-event` timer sources): `(due, unit,
//! kind, generation)` with lazy deletion. Arming or canceling bumps the
//! unit's generation of that kind, so a stale entry is skipped when popped:
//! `run_jobs` costs O(due), never a scan over all units (A1).
#![forbid(unsafe_code)]

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use super::unit::{Ms, UnitId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum TimerKind {
    /// Idle standby (re-armed lazily: `touch` only moves `last_active`).
    Idle,
    /// `TimeoutStartSec` (spawn → ready).
    StartTimeout,
    /// `TimeoutStopSec`: `Term` → `Kill`.
    StopGrace,
    /// `STOP_WAIT` 20 s: reap + backend cleanup.
    StopWait,
    /// `VERIFY_WAIT` 3 s.
    Verify,
    /// A job's `not_before` (backoff, `hold_until`).
    NotBefore,
    /// `"Starting <name>…"` after 400 ms.
    Notice,
    /// `CLASSIFY_WAIT_MS` 5 s.
    Classify,
    /// The resources answer of a Standby job.
    Resources,
    /// Request timeout of in-flight work.
    Request,
}
const KINDS: usize = 10;
impl TimerKind {
    fn index(self) -> usize {
        self as usize
    }
}

/// Per-unit generation of each timer kind.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Gens([u64; KINDS]);
impl Gens {
    pub fn get(&self, kind: TimerKind) -> u64 {
        self.0[kind.index()]
    }
    fn bump(&mut self, kind: TimerKind) -> u64 {
        self.0[kind.index()] += 1;
        self.0[kind.index()]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Target {
    Unit(UnitId, TimerKind),
    /// The one shutdown deadline.
    // B4: `shutdown`.
    #[cfg_attr(not(test), allow(dead_code))]
    Shutdown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Entry {
    pub due: Ms,
    seq: u64,
    pub target: Target,
    pub generation: u64,
}

#[derive(Default)]
pub(super) struct Timers {
    heap: BinaryHeap<Reverse<Entry>>,
    seq: u64,
}
impl Timers {
    /// Arms `kind` for `unit` at `due`, replacing any earlier one.
    pub fn arm(&mut self, gens: &mut Gens, unit: UnitId, kind: TimerKind, due: Ms) {
        let generation = gens.bump(kind);
        self.push(due, Target::Unit(unit, kind), generation);
    }
    pub fn cancel(gens: &mut Gens, kind: TimerKind) {
        gens.bump(kind);
    }
    pub fn push(&mut self, due: Ms, target: Target, generation: u64) {
        self.seq += 1;
        self.heap.push(Reverse(Entry {
            due,
            seq: self.seq,
            target,
            generation,
        }));
    }
    /// The earliest entry due at `now`, stale or not (the caller checks).
    pub fn pop_due(&mut self, now: Ms) -> Option<Entry> {
        if self.heap.peek().is_some_and(|Reverse(e)| e.due <= now) {
            self.heap.pop().map(|Reverse(e)| e)
        } else {
            None
        }
    }
    pub fn peek(&self) -> Option<Entry> {
        self.heap.peek().map(|Reverse(e)| *e)
    }
    pub fn pop(&mut self) -> Option<Entry> {
        self.heap.pop().map(|Reverse(e)| e)
    }
    /// The earliest due time (`run_jobs` drops stale entries at the top;
    /// one armed after it may still be stale: an early wake-up at most,
    /// never a late one).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn next(&self) -> Option<Ms> {
        self.heap.peek().map(|Reverse(e)| e.due)
    }
    // B3's per-tick budget (A1) bounds the heap with it.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.heap.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rearming_makes_the_old_entry_stale() {
        let mut timers = Timers::default();
        let mut gens = Gens::default();
        let u = UnitId(1);
        timers.arm(&mut gens, u, TimerKind::Idle, 100);
        timers.arm(&mut gens, u, TimerKind::Idle, 50);
        assert_eq!(timers.next(), Some(50));
        let first = timers.pop_due(200).unwrap();
        assert_eq!(first.generation, gens.get(TimerKind::Idle));
        let second = timers.pop_due(200).unwrap();
        assert_ne!(second.generation, gens.get(TimerKind::Idle));
        assert!(timers.pop_due(200).is_none());
    }
}
