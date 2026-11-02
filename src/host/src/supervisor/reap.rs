//! Reap (systemd "control group empty" + `ExecStopPost=`): a deactivating
//! unit lands in its `then` once its process is reaped **and** the backend
//! cleaned up after it (T16), or after `STOP_WAIT` 20 s flagged stuck
//! (T18). The "one instance per data directory" rule is a runnability
//! condition: a job never starts a unit while it is `Deactivating`, nor
//! while the process of a stop that landed stuck is still tracked
//! (`Unit::stopping` outside `Deactivating`): it waits `STOP_WAIT` from its
//! own request for that process to go, then fails (§17.7). An uninstall's
//! `RemoveData` waits for that process's reap (today's `data_removals`),
//! landed or not.
#![forbid(unsafe_code)]

use super::job::{Cause, JobKind, JobResult};
use super::timers::{TimerKind, Timers};
use super::trigger::AnswerReason;
use super::unit::{
    DeactivatingSub, FailResult, Pid, StopReason, Stopping, Then, UnitId, UnitState,
};
use super::{Action, Supervisor};

/// Today's text of a wake whose stopped process did not go (`lazy.rs`).
pub const STUCK_WAKE: &str = "its stopped process did not go within 20 s";
/// Today's text of a reload / enable-after-stop that waited in vain.
pub const STUCK_RELOAD: &str = "the previous process did not stop within 20 s";

impl<P: Clone> Supervisor<P> {
    /// §5 `process_reaped`: the process is gone (`Event::Reaped`).
    pub fn process_reaped(&mut self, pid: Pid, _status: Option<i32>) {
        let Some(&u) = self.by_pid.get(&pid) else {
            return;
        };
        let deactivating = self
            .units
            .get(&u)
            .is_some_and(|unit| unit.stopping.is_some_and(|s| s.pid == pid));
        if !deactivating {
            // Reaped without a stop: it exited by itself.
            if self.units.get(&u).and_then(|unit| unit.pid()) == Some(pid) {
                self.crash(u, super::ExitCause::Exited, None);
            } else {
                return;
            }
        }
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        if let Some(stopping) = unit.stopping.as_mut()
            && stopping.pid == pid
        {
            stopping.reaped = true;
            if let UnitState::Deactivating { sub, .. } = &mut unit.state {
                *sub = DeactivatingSub::Cleanup;
            }
            Timers::cancel(&mut unit.gens, TimerKind::StopGrace);
        }
        self.flush_data_removal(u);
        self.land_if_done(u);
        self.sync(u);
    }

    /// Emits an owed `RemoveData` once no process may write the data any
    /// more: none tracked, or the tracked one reaped (today's
    /// `data_removals` at `Event::Reaped`; also cleaned up after with
    /// `remove_data_waits_cleanup`, B4).
    pub(super) fn flush_data_removal(&mut self, u: UnitId) {
        let waits = self.tunables.remove_data_waits_cleanup;
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        let gone = unit
            .stopping
            .is_none_or(|s| s.reaped && (s.backend_cleaned || !waits));
        if !unit.remove_data || !gone {
            return;
        }
        unit.remove_data = false;
        let data = unit.data.take();
        // Never after its data: the checkpoint of a unit being removed goes
        // first (today's uninstall drops the recording at the request).
        let removing = unit.removed
            || matches!(
                unit.state,
                UnitState::Deactivating {
                    then: Then::Removed,
                    ..
                }
            );
        if removing && unit.recorded {
            unit.recorded = false;
            self.pending.push(Action::DropCheckpoint { unit: u });
        }
        self.pending.push(Action::RemoveData { unit: u, data });
    }

    /// §5 `backend_cleaned` (`Event::BackendCleaned`).
    pub fn backend_cleaned(&mut self, pid: Pid) {
        let Some(&u) = self.by_pid.get(&pid) else {
            return;
        };
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        if let Some(stopping) = unit.stopping.as_mut()
            && stopping.pid == pid
        {
            stopping.backend_cleaned = true;
        }
        self.land_if_done(u);
        self.sync(u);
    }

    pub(super) fn land_if_done(&mut self, u: UnitId) {
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        if !unit.stopping.is_some_and(|s| s.done()) {
            return;
        }
        if unit.state.is_deactivating() {
            self.land(u, false);
        } else {
            // The process of a stop that landed stuck went after all: a
            // start waiting for it goes on, an owed `RemoveData` goes first
            // if it still is owed (§17.7).
            unit.stopping = None;
            Timers::cancel(&mut unit.gens, TimerKind::StopWait);
            self.flush_data_removal(u);
            self.run.push_back(u);
        }
    }

    /// A start that waited `STOP_WAIT` from its request for the process of
    /// a stop that landed stuck: failed with today's text, never kept in
    /// standby (§17.7).
    pub(super) fn fail_stuck(&mut self, u: UnitId) {
        let wake = self.units.get(&u).is_some_and(|unit| {
            matches!(
                unit.job.as_ref().map(|job| &job.kind),
                Some(JobKind::Start {
                    cause: Cause::Trigger(_)
                })
            )
        });
        let text = if wake { STUCK_WAKE } else { STUCK_RELOAD };
        self.answer_backlog(u, AnswerReason::CouldNotStart);
        self.finish(u, JobResult::Failed(text.to_owned()));
        // Today's `wake_failed` shows "<name> could not start".
        if wake {
            self.pending.push(Action::Notify {
                unit: Some(u),
                notice: super::Notice::CouldNotStart,
            });
        }
        // Never kept in standby, not even from B3
        // (`start_failure_keeps_registrations` is for a failed spawn): the
        // old process may still run, so the next trigger would start a
        // second instance on the same data.
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        if let UnitState::Standby { holder, .. } = unit.state {
            self.pending.push(Action::Revoke {
                unit: u,
                pid: holder,
            });
        }
        unit.stuck = true;
        unit.failure = Some(text.to_owned());
        unit.state = UnitState::Failed {
            result: FailResult::Stuck,
            text: text.to_owned(),
        };
    }

    // ---- B2b–B2d: stops the Host executes itself (§17.9) ----------------

    /// The Host stopped process `pid` of `u` itself (`lifecycle.rs`
    /// `stop`; today's `Host.cleanup` entry): T24's `Deactivating`,
    /// without a `Terminate` action (the Host ran it) and without the
    /// `TimeoutStopSec` timer (the reaper times the grace and kills, A3).
    /// It lands as any stop does: at `process_reaped` + `backend_cleaned`
    /// (T16), or stuck after `STOP_WAIT` and still tracked until that
    /// process goes (T18). Every hand-off start waits for it
    /// (`host_start`). `then` `Standby` keeps the registrations (a park).
    /// The process it displaced from tracking, if one still went: a Host
    /// bug, since every start waits for the previous process (§17.9).
    /// Removed in B2d: the Host's stops become `stop` jobs and the
    /// `Terminate` executor.
    pub fn process_stopping(
        &mut self,
        u: UnitId,
        pid: Pid,
        reason: StopReason,
        then: Then,
    ) -> Option<Pid> {
        let now = self.now;
        let stop_wait = self.tunables.stop_wait;
        let unit = self.units.get_mut(&u)?;
        let displaced = unit
            .stopping
            .filter(|s| s.pid != pid && !s.done())
            .map(|s| s.pid);
        debug_assert!(
            displaced.is_none(),
            "unit {} stops {pid:?} while {displaced:?} still goes",
            unit.id
        );
        Self::cancel_run_timers(unit);
        let last = unit.last_pid.replace(pid);
        unit.state = UnitState::Deactivating {
            pid,
            reason,
            then,
            sub: DeactivatingSub::Term,
        };
        unit.stopping = Some(Stopping {
            pid,
            reaped: false,
            backend_cleaned: false,
            deadline: now + stop_wait,
            kept: then == Then::Standby,
        });
        Timers::cancel(&mut unit.gens, TimerKind::StopGrace);
        self.timers
            .arm(&mut unit.gens, u, TimerKind::StopWait, now + stop_wait);
        self.map_pid(u, pid);
        for old in [last, displaced].into_iter().flatten() {
            if old != pid {
                self.unmap_pid(u, old);
            }
        }
        self.sync(u);
        displaced
    }

    /// A shared-runner member stopped because the Host killed its runner
    /// (today's `restart_shared`): reaped with the runner, and cleaned up
    /// after unless `runner_restart_waits_cleanup` (B4), as `runner_failed`
    /// does for its members (§17.7 item 9c): it lands now, so its restart
    /// does not wait for it. Removed in B4 (`runner_failed`, T29).
    pub fn member_killed(&mut self, pid: Pid) {
        let cleaned = !self.tunables.runner_restart_waits_cleanup;
        let Some(&u) = self.by_pid.get(&pid) else {
            return;
        };
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        let Some(stopping) = unit.stopping.as_mut().filter(|s| s.pid == pid) else {
            return;
        };
        stopping.reaped = true;
        stopping.backend_cleaned |= cleaned;
        if let UnitState::Deactivating { sub, .. } = &mut unit.state {
            *sub = DeactivatingSub::Cleanup;
        }
        self.flush_data_removal(u);
        self.land_if_done(u);
        self.sync(u);
    }

    /// An uninstall asks for `data` (what the Host resolved at the request)
    /// to be removed once no process of `u` may write it any more (today's
    /// `data_removals`: at the reap under `legacy()`, also after the
    /// playback worker's cleanup with `remove_data_waits_cleanup`, §17.5,
    /// §17.7 item 18). `Some(data)` back when that is now (no process
    /// tracked, or the tracked one gone that far): the Host removes it at
    /// once. Otherwise it is owed and comes back as `RemoveData { data }`
    /// at that process's reap. Removed in B2d (`stop(u, Uninstall,
    /// Removed)` with the data resolved at the request, §13 B2d).
    pub fn remove_data_after(&mut self, u: UnitId, data: P) -> Option<P> {
        let waits = self.tunables.remove_data_waits_cleanup;
        let Some(unit) = self.units.get_mut(&u) else {
            return Some(data);
        };
        let gone = unit
            .stopping
            .is_none_or(|s| s.reaped && (s.backend_cleaned || !waits));
        if gone {
            return Some(data);
        }
        unit.remove_data = true;
        unit.data = Some(data);
        None
    }

    /// The process of `u` that may still run after its stop: tracked and
    /// not yet reaped and cleaned up after (today's `!process_gone`,
    /// `parked_process`, `!cleanup_done`). A start waits for it.
    pub fn previous_process(&self, u: UnitId) -> Option<Pid> {
        self.units
            .get(&u)?
            .stopping
            .filter(|s| !s.done())
            .map(|s| s.pid)
    }

    /// Whether `pid` is such a process (today's `!process_gone(owner)`).
    pub fn process_going(&self, pid: Pid) -> bool {
        self.by_pid
            .get(&pid)
            .and_then(|u| self.previous_process(*u))
            == Some(pid)
    }

    /// T16 / T18: the unit leaves `Deactivating` for its `then`.
    pub(super) fn land(&mut self, u: UnitId, stuck: bool) {
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        let UnitState::Deactivating { pid, then, .. } = unit.state else {
            return;
        };
        let now = self.now;
        let kept = unit.stopping.is_some_and(|s| s.kept);
        // Stuck: the process is still tracked (reap / cleanup may come
        // later), and every start waits for it (`step_start`, §17.7).
        unit.stopping = unit
            .stopping
            .filter(|_| stuck)
            .map(|s| super::unit::Stopping { kept: false, ..s });
        let hold = unit.hold_next.take().map(|delay| now + delay);
        if let Some(delay) = unit.after_reap.take()
            && let Some(job) = unit.job.as_mut().filter(|job| job.will_start())
        {
            job.not_before = Some(now + delay);
        }
        unit.stuck = stuck;
        unit.first_run = false;
        Timers::cancel(&mut unit.gens, TimerKind::StopGrace);
        Timers::cancel(&mut unit.gens, TimerKind::StopWait);
        let mut actions = Vec::new();
        if kept && then != Then::Standby {
            actions.push(Action::Revoke { unit: u, pid });
        }
        match then {
            Then::Standby => {
                unit.state = UnitState::Standby {
                    holder: pid,
                    recorded: true,
                    hold_until: hold,
                };
            }
            Then::Inactive => unit.state = UnitState::Inactive,
            Then::Failed => {
                let (result, text) = unit.failing.take().unwrap_or((
                    FailResult::Exit(super::ExitCause::Exited),
                    super::ExitCause::Exited.text().to_owned(),
                ));
                unit.failure = Some(text.clone());
                unit.state = UnitState::Failed { result, text };
            }
            Then::Removed => {
                // Removed even when stuck (today's uninstall removes the
                // plugin at once); its data goes at the reap of the process
                // still tracked (`flush_data_removal`, today's
                // `data_removals`, §17.7).
                unit.state = UnitState::Inactive;
                if unit.recorded {
                    actions.push(Action::DropCheckpoint { unit: u });
                }
                unit.recorded = false;
                unit.removed = true;
            }
        }
        self.pending.extend(actions);
        self.flush_data_removal(u);
        let replay = self
            .units
            .get_mut(&u)
            .is_some_and(|unit| std::mem::take(&mut unit.replay_after));
        if replay && then == Then::Inactive && !stuck {
            self.replay(u);
        }
        let stop_wait = self.tunables.stop_wait;
        let job = self
            .units
            .get(&u)
            .and_then(|unit| unit.job.as_ref())
            .map(|job| (job.kind.clone(), job.will_start(), job.created));
        match job {
            Some((kind @ (JobKind::Stop { .. } | JobKind::Standby), _, _)) => {
                let result = if stuck {
                    JobResult::Failed(STUCK_RELOAD.to_owned())
                } else if kind == JobKind::Standby && then != Then::Standby {
                    // It did not go to standby (a crash landed it in
                    // `Failed`, a stop elsewhere).
                    JobResult::Canceled
                } else {
                    JobResult::Done
                };
                self.finish(u, result);
            }
            // A start that waited its own `STOP_WAIT` fails now; one asked
            // later in the stop waits the rest of its 20 s (`step_start`;
            // today each wait counts from its request, §17.7).
            Some((_, true, created)) if stuck && now >= created + stop_wait => {
                self.fail_stuck(u);
            }
            _ => {}
        }
        self.run.push_back(u);
    }
}

/// The B2b inputs (§17.9): stops the Host runs itself, the hand-off waits
/// of the starts it runs, and an uninstall's data removal. The Host's side
/// is pinned end to end by tests/lifecycle_baseline.rs.
#[cfg(test)]
mod tests {
    use super::super::{
        Action, Busy, HostStart, HostStarted, JobId, JobKind, JobResult, Priority, Supervisor,
        Tunables,
    };
    use super::*;

    type S = Supervisor<u32>;

    fn new() -> S {
        Supervisor::new(Tunables::legacy(), 1)
    }

    /// A unit running `pid` (the Host's `add_plugin`).
    fn running(s: &mut S, id: &str, pid: u64) -> UnitId {
        let u = s.load_unit(id);
        s.adopt_process(u, Pid(pid));
        u
    }

    /// The Host's `stop` of `pid`.
    fn stop(s: &mut S, u: UnitId, pid: u64, reason: StopReason, then: Then) {
        assert_eq!(s.process_stopping(u, Pid(pid), reason, then), None);
        s.release_process(u, Pid(pid));
    }

    fn finished(actions: &[Action<u32>]) -> Vec<(JobId, JobKind, JobResult)> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::JobFinished {
                    job, kind, result, ..
                } => Some((*job, kind.clone(), result.clone())),
                _ => None,
            })
            .collect()
    }

    fn waits(started: Result<HostStarted, Busy>) -> JobId {
        match started {
            Ok(HostStarted::Waits { job, .. }) => job,
            other => panic!("{other:?}"),
        }
    }

    const WAKE: JobKind = JobKind::Start {
        cause: Cause::Trigger(Priority::Interactive),
    };
    const ENABLE: JobKind = JobKind::Start {
        cause: Cause::Enable,
    };

    /// A18 (2): what `Host.cleanup` kept for every stop of a run is one
    /// `Stopping` per unit, gone once its process is reaped and cleaned up
    /// after: a thousand stops leave nothing behind, and the pid index keeps
    /// only the last pid.
    #[test]
    fn stop_bookkeeping_is_one_per_unit_and_goes() {
        let mut s = new();
        let u = s.load_unit("a.one");
        for n in 1..=1000 {
            s.set_now(n * 10);
            s.adopt_process(u, Pid(n));
            stop(&mut s, u, n, StopReason::Disable, Then::Inactive);
            assert_eq!(s.previous_process(u), Some(Pid(n)));
            assert!(s.process_going(Pid(n)));
            // Either order (the playback worker is often first).
            if n % 2 == 0 {
                s.backend_cleaned(Pid(n));
                s.process_reaped(Pid(n), None);
            } else {
                s.process_reaped(Pid(n), None);
                s.backend_cleaned(Pid(n));
            }
            assert_eq!(s.previous_process(u), None);
            assert!(!s.process_going(Pid(n)));
            assert!(s.unit(u).unwrap().stopping.is_none());
            s.run_jobs(n * 10);
        }
        assert!(s.by_pid.len() <= 1, "{:?}", s.by_pid);
        assert_eq!(s.unit(u).unwrap().state, UnitState::Inactive);
    }

    /// T18: a stop not reaped within 20 s lands stuck, its process still
    /// tracked; a start asked later waits 20 s from its own request, and
    /// goes on once the process goes after all.
    #[test]
    fn a_stuck_stop_stays_tracked_until_its_process_goes() {
        let mut s = new();
        let u = running(&mut s, "a.one", 1);
        stop(&mut s, u, 1, StopReason::Idle, Then::Standby);
        s.adopt_holder(u, Pid(1));
        s.run_jobs(20_000);
        assert!(matches!(
            s.unit(u).unwrap().state,
            UnitState::Standby { .. }
        ));
        assert_eq!(s.previous_process(u), Some(Pid(1)));
        s.set_now(25_000);
        let job = waits(s.host_start(u, HostStart::Wake(Priority::Interactive)));
        assert!(finished(&s.run_jobs(44_999)).is_empty());
        s.process_reaped(Pid(1), None);
        s.backend_cleaned(Pid(1));
        assert_eq!(
            finished(&s.run_jobs(45_000)),
            [(job, WAKE, JobResult::Done)]
        );
    }

    /// Today's wake (`lazy.rs`): it waits for the process its plugin was
    /// parked from, starts once that is gone (reaped and cleaned up after),
    /// and fails with today's text 20 s after it was asked; a second wake
    /// meanwhile neither starts a second job nor moves the deadline.
    #[test]
    fn a_wake_waits_for_the_parked_process_or_fails_after_20_s() {
        let mut s = new();
        let u = running(&mut s, "a.one", 1);
        s.set_now(100);
        stop(&mut s, u, 1, StopReason::Idle, Then::Standby);
        s.adopt_holder(u, Pid(1));
        s.set_now(1_000);
        let job = waits(s.host_start(u, HostStart::Wake(Priority::Interactive)));
        s.set_now(5_000);
        assert_eq!(
            s.host_start(u, HostStart::Wake(Priority::Event)),
            Ok(HostStarted::Waits { job, new: false })
        );
        assert!(finished(&s.run_jobs(20_999)).is_empty());
        assert_eq!(
            finished(&s.run_jobs(21_000)),
            [(job, WAKE, JobResult::Failed(STUCK_WAKE.into()))]
        );
        // A wake whose process goes in time starts.
        let v = running(&mut s, "b.two", 3);
        stop(&mut s, v, 3, StopReason::Idle, Then::Standby);
        s.adopt_holder(v, Pid(3));
        let job = waits(s.host_start(v, HostStart::Wake(Priority::Interactive)));
        s.process_reaped(Pid(3), None);
        assert!(finished(&s.run_jobs(21_100)).is_empty(), "not cleaned");
        s.backend_cleaned(Pid(3));
        assert_eq!(
            finished(&s.run_jobs(21_200)),
            [(job, WAKE, JobResult::Done)]
        );
    }

    /// Nothing to wait for: the Host starts at once, no job.
    #[test]
    fn a_start_without_a_previous_process_is_now() {
        let mut s = new();
        let u = s.load_unit("a.one");
        for start in [
            HostStart::Wake(Priority::Interactive),
            HostStart::Enable,
            HostStart::Reload { verify_until: None },
            HostStart::Install { image: "d".into() },
        ] {
            assert_eq!(s.host_start(u, start), Ok(HostStarted::Now));
        }
        assert!(s.job(u).is_none());
    }

    /// D13: disable then enable of a running plugin: the enable waits for
    /// the old process (reaped and cleaned up after), is a package RPC
    /// meanwhile (today's `packages.reloads` entry: a second enable or a
    /// reload is `busy`), then hands off; with the process not gone 20 s
    /// after the enable, it fails with today's reload text.
    #[test]
    fn disable_then_enable_waits_for_the_old_process() {
        let mut s = new();
        let u = running(&mut s, "a.one", 1);
        stop(&mut s, u, 1, StopReason::Disable, Then::Inactive);
        let job = waits(s.host_start(u, HostStart::Enable));
        assert!(s.package_busy(u));
        assert_eq!(s.host_start(u, HostStart::Enable), Err(Busy));
        assert_eq!(
            s.host_start(u, HostStart::Reload { verify_until: None }),
            Err(Busy)
        );
        s.process_reaped(Pid(1), None);
        assert!(finished(&s.run_jobs(10)).is_empty());
        s.backend_cleaned(Pid(1));
        assert_eq!(finished(&s.run_jobs(20)), [(job, ENABLE, JobResult::Done)]);
        assert!(!s.package_busy(u));
        // Never gone: failed 20 s after the enable.
        s.adopt_process(u, Pid(2));
        s.set_now(1_000);
        stop(&mut s, u, 2, StopReason::Disable, Then::Inactive);
        s.set_now(2_000);
        let job = waits(s.host_start(u, HostStart::Enable));
        assert!(finished(&s.run_jobs(21_999)).is_empty());
        assert_eq!(
            finished(&s.run_jobs(22_000)),
            [(job, ENABLE, JobResult::Failed(STUCK_RELOAD.into()))]
        );
    }

    /// Today's reload of a built-in also waits for its digest check: until
    /// it is over, or its 3 s, whichever comes first once the process is
    /// gone.
    #[test]
    fn a_reload_waits_for_the_process_and_the_digest_check() {
        let mut s = new();
        let u = running(&mut s, "a.one", 1);
        stop(&mut s, u, 1, StopReason::Reload, Then::Inactive);
        let job = waits(s.host_start(
            u,
            HostStart::Reload {
                verify_until: Some(3_000),
            },
        ));
        s.process_reaped(Pid(1), None);
        s.backend_cleaned(Pid(1));
        assert!(finished(&s.run_jobs(100)).is_empty(), "still checking");
        assert!(s.package_busy(u));
        s.verify_finished(u);
        let done = finished(&s.run_jobs(150));
        assert_eq!(done.len(), 1);
        assert_eq!((done[0].0, &done[0].2), (job, &JobResult::Done));
        // Its 3 s over: it goes on without the check.
        s.adopt_process(u, Pid(2));
        stop(&mut s, u, 2, StopReason::Reload, Then::Inactive);
        let job = waits(s.host_start(
            u,
            HostStart::Reload {
                verify_until: Some(3_000),
            },
        ));
        s.process_reaped(Pid(2), None);
        s.backend_cleaned(Pid(2));
        assert!(finished(&s.run_jobs(2_999)).is_empty());
        assert_eq!(finished(&s.run_jobs(3_000))[0].0, job);
    }

    /// Review note B2-2: an install right after the uninstall of a running
    /// plugin (a reinstall reconnecting to its data directory) waits for
    /// the uninstalled process; until B2b it waited only for a running or
    /// parked version.
    #[test]
    fn an_install_after_an_uninstall_waits_for_its_process() {
        let mut s = new();
        let u = running(&mut s, "a.one", 1);
        stop(&mut s, u, 1, StopReason::Uninstall, Then::Inactive);
        let job = waits(s.host_start(u, HostStart::Install { image: "d".into() }));
        assert!(s.package_busy(u));
        s.process_reaped(Pid(1), None);
        assert!(finished(&s.run_jobs(10)).is_empty());
        s.backend_cleaned(Pid(1));
        assert_eq!(finished(&s.run_jobs(20))[0].0, job);
    }

    /// A package operation's wait merges with a waiting reload (today the
    /// install went on and the reload's start was dropped): never `Busy`,
    /// one job, ending as the operation's.
    #[test]
    fn an_install_takes_over_a_waiting_reload() {
        let mut s = new();
        let u = running(&mut s, "a.one", 1);
        stop(&mut s, u, 1, StopReason::Reload, Then::Inactive);
        let reload = waits(s.host_start(u, HostStart::Reload { verify_until: None }));
        let install = waits(s.host_start(u, HostStart::Install { image: "d".into() }));
        assert_eq!(install, reload);
        s.process_reaped(Pid(1), None);
        s.backend_cleaned(Pid(1));
        let done = finished(&s.run_jobs(10));
        assert!(
            matches!(
                &done[..],
                [(job, JobKind::Restart { image: Some(_), .. }, JobResult::Done)] if *job == install
            ),
            "{done:?}"
        );
    }

    /// A parked plugin forgotten (disabled, reloaded, uninstalled) while its
    /// wake waits: the wake goes with it (canceled), as today's
    /// `Parked.waiting` went with the parked entry, and the unit no longer
    /// lands in standby.
    #[test]
    fn forgetting_a_parked_unit_cancels_its_waiting_wake() {
        let mut s = new();
        let u = running(&mut s, "a.one", 1);
        stop(&mut s, u, 1, StopReason::Idle, Then::Standby);
        s.adopt_holder(u, Pid(1));
        let job = waits(s.host_start(u, HostStart::Wake(Priority::Interactive)));
        s.release_holder(u, Pid(1));
        assert!(s.job(u).is_none());
        assert!(finished(&s.take_actions()).contains(&(job, WAKE, JobResult::Canceled)));
        s.process_reaped(Pid(1), None);
        s.backend_cleaned(Pid(1));
        s.run_jobs(10);
        assert_eq!(s.unit(u).unwrap().state, UnitState::Inactive);
    }

    /// Today's `data_removals`: an uninstall's data goes at once without a
    /// process that may write it, or once that was reaped (its cleanup not
    /// awaited under `legacy()`); else at that process's reap, with what
    /// the Host resolved at the request.
    #[test]
    fn uninstall_data_goes_at_the_reap() {
        let mut s = new();
        let u = s.load_unit("a.one");
        assert_eq!(s.remove_data_after(u, 7), Some(7));
        s.adopt_process(u, Pid(1));
        stop(&mut s, u, 1, StopReason::Uninstall, Then::Inactive);
        assert_eq!(s.remove_data_after(u, 8), None);
        s.backend_cleaned(Pid(1));
        let removed = |a: &Action<u32>| matches!(a, Action::RemoveData { .. });
        assert!(!s.take_actions().iter().any(removed));
        s.process_reaped(Pid(1), None);
        assert!(s.take_actions().contains(&Action::RemoveData {
            unit: u,
            data: Some(8)
        }));
        // Reaped, not cleaned up after: at once (legacy).
        s.adopt_process(u, Pid(2));
        stop(&mut s, u, 2, StopReason::Uninstall, Then::Inactive);
        s.process_reaped(Pid(2), None);
        assert_eq!(s.remove_data_after(u, 9), Some(9));
    }

    /// The members of a runner the Host killed land at once (today's
    /// `restart_shared`): their restart does not wait for them, and their
    /// late reap and cleanup change nothing.
    #[test]
    fn members_killed_with_their_runner_are_gone_at_once() {
        let mut s = new();
        let u = running(&mut s, "vestra.one", 1);
        stop(&mut s, u, 1, StopReason::Failed, Then::Failed);
        s.member_killed(Pid(1));
        assert_eq!(s.previous_process(u), None);
        assert_eq!(s.host_start(u, HostStart::Enable), Ok(HostStarted::Now));
        s.adopt_process(u, Pid(2));
        s.process_reaped(Pid(1), None);
        s.backend_cleaned(Pid(1));
        assert!(matches!(
            s.unit(u).unwrap().state,
            UnitState::Active { pid: Pid(2), .. }
        ));
    }
}
