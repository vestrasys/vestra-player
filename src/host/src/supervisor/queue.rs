//! Runnable start jobs (systemd `manager_dispatch_run_queue`; Chrome
//! process-launch throttling): lanes in fixed order, FIFO within a lane.
//! Entries are validated when they reach the head (lazy deletion), so a
//! merged or canceled job costs nothing here.
#![forbid(unsafe_code)]

use std::collections::VecDeque;

use super::job::{Cause, JobId, Priority};
use super::unit::{Class, UnitId};

/// In the order they run (§4.4): built-ins first (their first runs after
/// their boot starts, as `StartQueue` then `FirstQueue`), then third-party
/// by priority; third-party first runs last (they are `Background`, after
/// the other background starts).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Lane {
    Builtin,
    BuiltinFirstRun,
    Interactive,
    Event,
    Background,
    FirstRun,
}
pub(super) const LANES: [Lane; 6] = [
    Lane::Builtin,
    Lane::BuiltinFirstRun,
    Lane::Interactive,
    Lane::Event,
    Lane::Background,
    Lane::FirstRun,
];
impl Lane {
    pub fn of(class: Class, cause: Option<Cause>, priority: Priority) -> Lane {
        let first = cause == Some(Cause::FirstRun);
        match (class, first) {
            (Class::Builtin, false) => Lane::Builtin,
            (Class::Builtin, true) => Lane::BuiltinFirstRun,
            (_, true) => Lane::FirstRun,
            _ => match priority {
                Priority::Interactive => Lane::Interactive,
                Priority::Event => Lane::Event,
                Priority::Background => Lane::Background,
            },
        }
    }
    pub fn builtin(self) -> bool {
        matches!(self, Lane::Builtin | Lane::BuiltinFirstRun)
    }
    pub fn first_run(self) -> bool {
        matches!(self, Lane::BuiltinFirstRun | Lane::FirstRun)
    }
    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Default)]
pub(super) struct Lanes {
    lanes: [VecDeque<(UnitId, JobId)>; 6],
}
impl Lanes {
    pub fn push(&mut self, lane: Lane, unit: UnitId, job: JobId) {
        self.lanes[lane.index()].push_back((unit, job));
    }
    pub fn front(&self, lane: Lane) -> Option<(UnitId, JobId)> {
        self.lanes[lane.index()].front().copied()
    }
    pub fn pop(&mut self, lane: Lane) -> Option<(UnitId, JobId)> {
        self.lanes[lane.index()].pop_front()
    }
    /// Entries, stale ones included (for bounds of the backing storage).
    pub fn len(&self) -> usize {
        self.lanes.iter().map(VecDeque::len).sum()
    }
}
