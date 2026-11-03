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
    // B3 bounds the backlog with it.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.lanes.iter().map(VecDeque::len).sum()
    }
}

/// The start-up queues' tests (`StartQueue`, `FirstQueue`; package_manager.rs
/// and host/supervisor/first_runs.rs until B2d), ported to the lanes.
#[cfg(test)]
mod tests {
    use super::super::{Action, Class, ListState, Pid, Supervisor, Tunables, UnitId, UnitSpec};

    type S = Supervisor<u32>;

    fn spawns(actions: &[Action<u32>]) -> Vec<UnitId> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::Spawn { unit, .. } => Some(*unit),
                _ => None,
            })
            .collect()
    }

    fn pid(s: &S, u: UnitId) -> Pid {
        s.unit(u).unwrap().pid().unwrap()
    }

    /// B0 (`b0_start_queue_lets_others_pass_a_held_builtin`,
    /// `start_queue_starts_builtins_first_and_counts_them`): loaded in the
    /// start order, a built-in still being checked keeps its place while the
    /// others start, counted as pending (`boot_pending`: today's
    /// `to_start`), listed `stopped`; it starts once checked.
    #[test]
    fn b0_others_pass_a_builtin_being_checked() {
        let mut s = S::new(Tunables::legacy(), 1);
        let slow = s.add_unit(
            UnitSpec::new("vestra.slow", Class::Builtin)
                .unrecordable()
                .verify(),
        );
        let fast = s.add_unit(UnitSpec::new("vestra.fast", Class::Builtin).unrecordable());
        let other = s.add_unit(UnitSpec::new("a.one", Class::Normal).unrecordable());
        let actions = s.run_jobs(0);
        assert_eq!(actions.first(), Some(&Action::Verify { unit: slow }));
        assert_eq!(spawns(&actions), [fast, other]);
        assert!(s.boot_pending());
        assert_eq!(s.list_state(slow), Some(ListState::Stopped));
        s.verify_done(slow, true);
        assert_eq!(spawns(&s.run_jobs(10)), [slow]);
        assert!(!s.boot_pending());
    }

    /// `FirstQueue` and `FIRST_RUNS` (`first_queue_puts_builtins_first_and_
    /// counts_them`, `eight_first_runs_at_a_time`): at most 8 first runs at
    /// a time, built-ins first; a third-party first run waits while a
    /// built-in waits (`BuiltinsFirst::Queued`); a place frees once a run is
    /// classified resident.
    #[test]
    fn eight_first_runs_at_a_time_builtins_first() {
        let mut s = S::new(Tunables::legacy(), 1);
        let third = s.add_unit(UnitSpec::new("a.two", Class::Normal));
        let builtins = (0..10)
            .map(|n| s.add_unit(UnitSpec::new(format!("vestra.b{n}"), Class::Builtin)))
            .collect::<Vec<_>>();
        let actions = s.run_jobs(0);
        assert_eq!(spawns(&actions), builtins[..8]);
        assert_eq!(s.first_runs_queued(), (true, true));
        assert_eq!(s.list_state(builtins[8]), Some(ListState::Starting));
        assert_eq!(s.list_state(third), Some(ListState::Starting));
        // A place frees: the next built-in, never the third-party one first.
        let first = pid(&s, builtins[0]);
        s.process_spawned(first, Ok(()));
        s.process_ready(first);
        s.classified(first, Some("a panel".into()));
        assert_eq!(spawns(&s.run_jobs(1)), [builtins[8]]);
        let second = pid(&s, builtins[1]);
        s.process_spawned(second, Ok(()));
        s.process_ready(second);
        s.classified(second, Some("a panel".into()));
        assert_eq!(spawns(&s.run_jobs(2)), [builtins[9]]);
        assert_eq!(s.first_runs_queued(), (true, false));
        // The built-ins started: the third-party first run waits for a place.
        let third_place = pid(&s, builtins[2]);
        s.process_spawned(third_place, Ok(()));
        s.process_ready(third_place);
        s.classified(third_place, Some("a panel".into()));
        assert_eq!(spawns(&s.run_jobs(3)), [third]);
        assert_eq!(s.first_runs_queued(), (false, false));
    }

    /// `verify_before_first_run_place` (today a verifying built-in waits in
    /// the start queue and asks for a first-run place once checked): nine
    /// built-ins being checked take no place while they are; the first 8
    /// checked start, the ninth waits at the end of its lane.
    #[test]
    fn a_first_run_is_checked_before_it_takes_a_place() {
        let mut s = S::new(Tunables::legacy(), 1);
        let units = (0..9)
            .map(|n| s.add_unit(UnitSpec::new(format!("vestra.b{n}"), Class::Builtin).verify()))
            .collect::<Vec<_>>();
        let actions = s.run_jobs(0);
        assert!(spawns(&actions).is_empty());
        assert_eq!(
            actions
                .iter()
                .filter(|a| matches!(a, Action::Verify { .. }))
                .count(),
            9
        );
        assert_eq!(s.list_state(units[0]), Some(ListState::Stopped));
        assert!(s.boot_pending());
        // Checked last first: it gets a place before the others.
        for u in units.iter().rev() {
            s.verify_done(*u, true);
        }
        let started = spawns(&s.run_jobs(10));
        assert_eq!(
            started,
            units[1..].iter().rev().copied().collect::<Vec<_>>()
        );
        assert_eq!(s.first_runs_queued(), (true, true));
        assert_eq!(s.list_state(units[0]), Some(ListState::Starting));
        assert!(!s.boot_pending());
        let first = pid(&s, units[8]);
        s.process_spawned(first, Ok(()));
        s.process_ready(first);
        s.classified(first, Some("a panel".into()));
        assert_eq!(spawns(&s.run_jobs(20)), [units[0]]);
        // From B3 the check runs on the place (T4).
        let mut b3 = S::new(Tunables::legacy().with_b3(8), 1);
        let u = b3.add_unit(UnitSpec::new("vestra.b", Class::Builtin).verify());
        assert_eq!(b3.run_jobs(0), [Action::Verify { unit: u }]);
        assert!(!b3.boot_pending() || b3.list_state(u) == Some(ListState::Starting));
    }

    /// Built-ins the adapter has still to load (kept in place in its start
    /// order while their digest is checked, as today's `StartQueue`) hold
    /// third-party first runs back like a queued built-in.
    #[test]
    fn builtins_still_to_load_hold_third_party_first_runs() {
        let mut s = S::new(Tunables::legacy(), 1);
        s.set_builtins_to_load(1);
        let third = s.add_unit(UnitSpec::new("a.one", Class::Normal));
        assert!(spawns(&s.run_jobs(0)).is_empty());
        assert_eq!(s.list_state(third), Some(ListState::Starting));
        s.set_builtins_to_load(0);
        assert_eq!(spawns(&s.run_jobs(1)), [third]);
    }
}
