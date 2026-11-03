//! Runnable start jobs (supervisor.md §5 "Start order and limits"): lanes in
//! fixed order, FIFO within a lane.
//! Entries are validated when they reach the head (lazy deletion), so a
//! merged or canceled job costs nothing here.
#![forbid(unsafe_code)]

use std::collections::VecDeque;

use super::job::{Cause, JobId, Priority};
use super::unit::{Class, UnitId};

/// In the order they run: built-ins first (their first runs after their boot
/// starts, as `StartQueue` then `FirstQueue`), then third-party by priority;
/// third-party first runs last (they are `Background`, after the other
/// background starts).
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
}

/// The start-up queues' tests (`StartQueue`, `FirstQueue`; package_manager.rs
/// and host/supervisor/first_runs.rs before the lanes), ported to the lanes.
#[cfg(test)]
mod tests {
    use super::super::{Action, Class, ListState, Supervisor, Tunables, UnitId, UnitSpec};

    type S = Supervisor<u32>;

    fn new() -> S {
        S::new(Tunables::new(8, 16_384), 1)
    }

    fn spawns(actions: &[Action<u32>]) -> Vec<UnitId> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::Spawn { unit, .. } => Some(*unit),
                _ => None,
            })
            .collect()
    }

    /// Characterization tests (`b0_start_queue_lets_others_pass_a_held_builtin`,
    /// `start_queue_starts_builtins_first_and_counts_them`): loaded in the
    /// start order, a built-in still being checked keeps its place while the
    /// others start, counted as pending (`boot_pending`: today's `to_start`),
    /// listed `starting`; it starts once checked.
    #[test]
    fn b0_others_pass_a_builtin_being_checked() {
        let mut s = new();
        let slow = s.add_unit(
            UnitSpec::new("vestra.slow", Class::Builtin)
                .unrecordable()
                .verify(),
        );
        let fast = s.add_unit(UnitSpec::new("vestra.fast", Class::Builtin).unrecordable());
        let actions = s.run_jobs(0);
        assert_eq!(actions.first(), Some(&Action::Verify { unit: slow }));
        assert_eq!(spawns(&actions), [fast]);
        assert!(s.boot_pending());
        assert_eq!(s.list_state(slow), Some(ListState::Starting));
        s.verify_done(slow, true);
        assert_eq!(spawns(&s.run_jobs(10)), [slow]);
        assert!(!s.boot_pending());
    }

    /// A built-in's first run is checked on its place: the check runs first,
    /// its start lists `starting`.
    #[test]
    fn a_first_run_is_checked_on_its_place() {
        let mut s = new();
        let u = s.add_unit(UnitSpec::new("vestra.b", Class::Builtin).verify());
        assert_eq!(s.run_jobs(0), [Action::Verify { unit: u }]);
        assert_eq!(s.list_state(u), Some(ListState::Starting));
    }

    /// Built-ins the adapter has still to load (kept in place in its start
    /// order while their digest is checked, as today's `StartQueue`) hold
    /// third-party first runs back like a built-in with a start installed.
    #[test]
    fn builtins_still_to_load_hold_third_party_first_runs() {
        let mut s = new();
        s.set_builtins_to_load(1);
        let third = s.add_unit(UnitSpec::new("a.one", Class::Normal));
        assert!(spawns(&s.run_jobs(0)).is_empty());
        assert_eq!(s.list_state(third), Some(ListState::Starting));
        s.set_builtins_to_load(0);
        assert_eq!(spawns(&s.run_jobs(1)), [third]);
    }
}
