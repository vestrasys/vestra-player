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
use super::unit::{DeactivatingSub, FailResult, Pid, Then, UnitId, UnitState};
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
        self.pending.push(Action::RemoveData { unit: u });
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

    // ---- B2b–B4: the runner restart the Host still runs ------------------

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

/// The waits for stopped processes (§17.9), driven as the adapter drives
/// them since B2d: every start and stop is a job. The Host's side is pinned
/// end to end by tests/lifecycle_baseline.rs.
#[cfg(test)]
mod tests {
    use super::super::{
        Action, Busy, Class, Delivery, JobId, JobKind, JobMode, JobResult, Origin, Priority,
        StopReason, Supervisor, Trigger, Tunables, UnitSpec,
    };
    use super::*;

    type S = Supervisor<u32>;

    fn new() -> S {
        Supervisor::new(Tunables::legacy(), 1)
    }

    /// The pid of the spawn in `actions`.
    fn spawned(actions: &[Action<u32>]) -> Option<Pid> {
        actions.iter().find_map(|a| match a {
            Action::Spawn { pid, .. } => Some(*pid),
            _ => None,
        })
    }

    /// A unit running `pid` (its start job spawned it, it is ready).
    fn running(s: &mut S, id: &str, pid: u64) -> UnitId {
        let u = s.add_unit(UnitSpec::new(id, Class::Normal).unrecordable());
        let now = s.now();
        assert_eq!(spawned(&s.run_jobs(now)), Some(Pid(pid)));
        s.process_spawned(Pid(pid), Ok(()));
        s.process_ready(Pid(pid));
        s.run_jobs(now);
        u
    }

    /// A start of `u` (an enable) ran: spawned as `pid` and ready.
    fn started(s: &mut S, pid: u64) {
        s.process_spawned(Pid(pid), Ok(()));
        s.process_ready(Pid(pid));
    }

    /// The core stops `u`'s process (`Terminate`): a stop job, or for
    /// standby the idle Standby job (`QueryResources` answered idle).
    fn stop(s: &mut S, u: UnitId, reason: StopReason, then: Then) {
        if then == Then::Standby {
            // Not used since its ready.
            let later = s.now() + 1;
            s.set_now(later);
            s.standby(u);
            let now = s.now();
            let pid = s.unit(u).unwrap().pid().unwrap();
            assert!(
                s.run_jobs(now)
                    .contains(&Action::QueryResources { unit: u, pid })
            );
            s.resources_answered(pid, false);
        } else {
            s.stop(u, reason, then).unwrap();
        }
        let now = s.now();
        assert!(
            s.run_jobs(now)
                .iter()
                .any(|a| matches!(a, Action::Terminate { .. }))
        );
    }

    /// The process a start of `u` waits for (today's `!process_gone`).
    fn going(s: &S, u: UnitId) -> Option<Pid> {
        s.unit(u)?.stopping.filter(|s| !s.done()).map(|s| s.pid)
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

    fn spawns(actions: &[Action<u32>]) -> Vec<UnitId> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::Spawn { unit, .. } => Some(*unit),
                _ => None,
            })
            .collect()
    }

    /// A package operation's start (`Restart{fresh, image}`, merged with
    /// what is installed: the operation is the busy RPC already).
    fn install(s: &mut S, u: UnitId) -> JobId {
        s.enqueue_job(
            u,
            JobKind::Restart {
                fresh: true,
                image: Some("d".into()),
            },
            JobMode::Replace,
            Priority::Interactive,
            Origin::PackageRpc,
        )
        .unwrap()
    }

    const WAKE: JobKind = JobKind::Start {
        cause: Cause::Trigger(Priority::Interactive),
    };

    /// A18 (2): what `Host.cleanup` kept for every stop of a run is one
    /// `Stopping` per unit, gone once its process is reaped and cleaned up
    /// after: a thousand stops leave nothing behind, and the pid index keeps
    /// only the last pid.
    #[test]
    fn stop_bookkeeping_is_one_per_unit_and_goes() {
        let mut s = new();
        let u = running(&mut s, "a.one", 1);
        for n in 1..=1000 {
            stop(&mut s, u, StopReason::Disable, Then::Inactive);
            assert_eq!(going(&s, u), Some(Pid(n)));
            // Either order (the playback worker is often first).
            if n % 2 == 0 {
                s.backend_cleaned(Pid(n));
                s.process_reaped(Pid(n), None);
            } else {
                s.process_reaped(Pid(n), None);
                s.backend_cleaned(Pid(n));
            }
            assert_eq!(going(&s, u), None);
            assert!(s.unit(u).unwrap().stopping.is_none());
            s.run_jobs(n * 10);
            if n < 1000 {
                s.enable(u).unwrap();
                assert_eq!(spawned(&s.run_jobs(n * 10)), Some(Pid(n + 1)));
                started(&mut s, n + 1);
            }
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
        stop(&mut s, u, StopReason::Disable, Then::Inactive);
        s.run_jobs(20_000);
        assert_eq!(s.unit(u).unwrap().state, UnitState::Inactive);
        assert_eq!(going(&s, u), Some(Pid(1)));
        s.set_now(25_000);
        s.enable(u).unwrap();
        assert!(spawns(&s.run_jobs(25_000)).is_empty());
        assert!(spawns(&s.run_jobs(44_999)).is_empty());
        s.process_reaped(Pid(1), None);
        s.backend_cleaned(Pid(1));
        assert_eq!(spawns(&s.run_jobs(45_000)), [u]);
    }

    /// Today's wake (`lazy.rs`): it waits for the process its plugin was
    /// parked from, starts once that is gone (reaped and cleaned up after),
    /// and fails with today's text 20 s after it was asked ("could not
    /// start", entries revoked); a second trigger meanwhile neither starts
    /// a second job nor moves the deadline.
    #[test]
    fn a_wake_waits_for_the_parked_process_or_fails_after_20_s() {
        let mut s = new();
        let u = running(&mut s, "a.one", 1);
        s.set_now(100);
        stop(&mut s, u, StopReason::Idle, Then::Standby);
        s.set_now(1_000);
        assert_eq!(s.wake(Pid(1), &Trigger::interactive(7)), Delivery::Started);
        let job = s.job(u).unwrap().id;
        s.set_now(5_000);
        assert_eq!(
            s.wake(Pid(1), &Trigger::event("packages.changed", 8)),
            Delivery::Held
        );
        assert_eq!(s.job(u).unwrap().id, job);
        assert!(
            !finished(&s.run_jobs(20_999))
                .iter()
                .any(|(j, ..)| *j == job)
        );
        let actions = s.run_jobs(21_000);
        assert_eq!(
            finished(&actions),
            [(job, WAKE, JobResult::Failed(STUCK_WAKE.into()))]
        );
        assert!(actions.contains(&Action::Revoke {
            unit: u,
            pid: Pid(1)
        }));
        // A wake whose process goes in time starts.
        let v = running(&mut s, "b.two", 2);
        stop(&mut s, v, StopReason::Idle, Then::Standby);
        assert_eq!(s.wake(Pid(2), &Trigger::interactive(9)), Delivery::Started);
        s.process_reaped(Pid(2), None);
        assert!(spawns(&s.run_jobs(21_100)).is_empty(), "not cleaned");
        s.backend_cleaned(Pid(2));
        assert_eq!(spawns(&s.run_jobs(21_200)), [v]);
    }

    /// Nothing to wait for: an enable, a reload or an install starts at
    /// once.
    #[test]
    fn a_start_without_a_previous_process_is_now() {
        let mut s = new();
        let u = s.add_unit(UnitSpec::new("a.one", Class::Normal).disabled());
        s.enable(u).unwrap();
        assert_eq!(spawns(&s.run_jobs(0)), [u]);
        let v = s.add_unit(UnitSpec::new("b.two", Class::Normal).disabled());
        s.set_enabled(v, true);
        s.restart(v, true, None).unwrap();
        assert_eq!(spawns(&s.run_jobs(0)), [v]);
        let w = s.add_unit(UnitSpec::new("c.three", Class::Normal).disabled());
        s.set_enabled(w, true);
        install(&mut s, w);
        assert_eq!(spawns(&s.run_jobs(0)), [w]);
    }

    /// D13: disable then enable of a running plugin: the enable waits for
    /// the old process (reaped and cleaned up after), is a package RPC
    /// meanwhile (today's `packages.reloads` entry: a second enable or a
    /// reload is `busy`), then starts; with the process not gone 20 s after
    /// the enable, it fails with today's reload text.
    #[test]
    fn disable_then_enable_waits_for_the_old_process() {
        let mut s = new();
        let u = running(&mut s, "a.one", 1);
        s.disable(u).unwrap();
        s.run_jobs(0);
        let job = s.enable(u).unwrap();
        assert!(s.package_busy(u));
        assert_eq!(s.enable(u), Err(Busy));
        assert_eq!(s.restart(u, true, None), Err(Busy));
        s.process_reaped(Pid(1), None);
        assert!(spawns(&s.run_jobs(10)).is_empty());
        s.backend_cleaned(Pid(1));
        assert_eq!(spawns(&s.run_jobs(20)), [u]);
        assert_eq!(s.job(u).unwrap().id, job);
        started(&mut s, 2);
        assert!(finished(&s.run_jobs(20)).iter().any(|(j, ..)| *j == job));
        assert!(!s.package_busy(u));
        // Never gone: failed 20 s after the enable.
        s.set_now(1_000);
        s.disable(u).unwrap();
        s.run_jobs(1_000);
        s.set_now(2_000);
        let job = s.enable(u).unwrap();
        assert!(finished(&s.run_jobs(21_999)).is_empty());
        let done = finished(&s.run_jobs(22_000));
        assert_eq!(done.len(), 1);
        assert_eq!(
            (done[0].0, &done[0].2),
            (job, &JobResult::Failed(STUCK_RELOAD.into()))
        );
    }

    /// A reload of a built-in waits for its process, then for its digest
    /// check (`Activating{Verify}`, T4): until it is over, or its 3 s.
    #[test]
    fn a_reload_waits_for_the_process_and_the_digest_check() {
        let mut s = new();
        let u = s.add_unit(
            UnitSpec::new("vestra.one", Class::Builtin)
                .unrecordable()
                .verify(),
        );
        assert_eq!(s.run_jobs(0), [Action::Verify { unit: u }]);
        s.verify_done(u, true);
        assert_eq!(spawned(&s.run_jobs(0)), Some(Pid(1)));
        started(&mut s, 1);
        s.restart(u, true, None).unwrap();
        s.run_jobs(0);
        s.process_reaped(Pid(1), None);
        s.backend_cleaned(Pid(1));
        assert_eq!(s.run_jobs(100), [Action::Verify { unit: u }]);
        assert!(s.package_busy(u));
        s.verify_done(u, true);
        assert_eq!(spawns(&s.run_jobs(150)), [u]);
        started(&mut s, 2);
        // Its 3 s over: it goes on without the check (its own process).
        s.restart(u, true, None).unwrap();
        s.run_jobs(150);
        s.process_reaped(Pid(2), None);
        s.backend_cleaned(Pid(2));
        assert_eq!(s.run_jobs(200), [Action::Verify { unit: u }]);
        assert!(spawns(&s.run_jobs(3_199)).is_empty());
        let actions = s.run_jobs(3_200);
        assert!(actions.iter().any(|a| matches!(
            a,
            Action::Spawn {
                placement: super::super::Placement::Own,
                ..
            }
        )));
    }

    /// Review note B2-2: an install right after the uninstall of a running
    /// plugin (a reinstall reconnecting to its data directory) waits for
    /// the uninstalled process.
    #[test]
    fn an_install_after_an_uninstall_waits_for_its_process() {
        let mut s = new();
        let u = running(&mut s, "a.one", 1);
        stop(&mut s, u, StopReason::Uninstall, Then::Removed);
        let v = s.add_unit(UnitSpec::new("a.one", Class::Normal).disabled());
        assert_eq!(v, u, "a reinstall: the same unit");
        s.set_enabled(u, true);
        let job = install(&mut s, u);
        assert!(s.package_busy(u));
        s.process_reaped(Pid(1), None);
        assert!(spawns(&s.run_jobs(10)).is_empty());
        s.backend_cleaned(Pid(1));
        assert_eq!(spawns(&s.run_jobs(20)), [u]);
        assert_eq!(s.job(u).unwrap().id, job);
    }

    /// A package operation's start merges with a waiting reload (today the
    /// install went on and the reload's start was dropped): never `Busy`,
    /// one job, the operation's image.
    #[test]
    fn an_install_takes_over_a_waiting_reload() {
        let mut s = new();
        let u = running(&mut s, "a.one", 1);
        let reload = s.restart(u, true, None).unwrap();
        s.run_jobs(0);
        let job = install(&mut s, u);
        assert_eq!(job, reload);
        s.process_reaped(Pid(1), None);
        s.backend_cleaned(Pid(1));
        let actions = s.run_jobs(10);
        assert!(actions.iter().any(|a| matches!(
            a,
            Action::Spawn { image: Some(image), .. } if image == "d"
        )));
    }

    /// A parked plugin disabled while its wake waits: the wake goes with it
    /// (canceled), as today's `Parked.waiting` went with the parked entry,
    /// and the unit no longer lands in standby.
    #[test]
    fn a_disable_cancels_the_waiting_wake_of_a_parked_unit() {
        let mut s = new();
        let u = running(&mut s, "a.one", 1);
        stop(&mut s, u, StopReason::Idle, Then::Standby);
        assert_eq!(s.wake(Pid(1), &Trigger::interactive(7)), Delivery::Started);
        let job = s.job(u).unwrap().id;
        s.disable(u).unwrap();
        let actions = s.run_jobs(0);
        assert!(finished(&actions).contains(&(job, WAKE, JobResult::Canceled)));
        s.process_reaped(Pid(1), None);
        s.backend_cleaned(Pid(1));
        s.run_jobs(10);
        assert_eq!(s.unit(u).unwrap().state, UnitState::Inactive);
        assert!(s.job(u).is_none());
    }

    /// Today's `data_removals`: an uninstall's data goes at once without a
    /// process that may write it, or once that was reaped (its cleanup not
    /// awaited under `legacy()`); else at that process's reap. The
    /// recording goes at the request (review note B2-4).
    #[test]
    fn uninstall_data_goes_at_the_reap() {
        let mut s = new();
        let removed = |a: &Action<u32>| matches!(a, Action::RemoveData { .. });
        let u = s.add_unit(UnitSpec::new("a.one", Class::Normal).disabled());
        s.stop(u, StopReason::Uninstall, Then::Removed).unwrap();
        assert!(s.run_jobs(0).iter().any(removed));
        let v = running(&mut s, "b.two", 1);
        stop(&mut s, v, StopReason::Uninstall, Then::Removed);
        s.backend_cleaned(Pid(1));
        assert!(!s.run_jobs(0).iter().any(removed));
        s.process_reaped(Pid(1), None);
        assert!(s.run_jobs(0).contains(&Action::RemoveData { unit: v }));
        assert!(s.unit(v).unwrap().removed);
    }

    /// Review note B2-4: the checkpoint of a unit being uninstalled goes at
    /// the request; its landing, after a reinstall took the unit, drops
    /// nothing of the new install.
    #[test]
    fn an_uninstalls_checkpoint_goes_at_the_request() {
        let mut s = new();
        let u = s.add_unit(UnitSpec::new("a.one", Class::Normal).checkpoint(true));
        assert_eq!(spawned(&s.run_jobs(0)), Some(Pid(1)));
        started(&mut s, 1);
        s.stop(u, StopReason::Uninstall, Then::Removed).unwrap();
        let actions = s.run_jobs(0);
        assert!(actions.contains(&Action::DropCheckpoint { unit: u }));
        s.add_unit(UnitSpec::new("a.one", Class::Normal));
        s.process_reaped(Pid(1), None);
        s.backend_cleaned(Pid(1));
        let actions = s.run_jobs(10);
        assert!(
            !actions.contains(&Action::DropCheckpoint { unit: u }),
            "{actions:?}"
        );
        assert_eq!(spawns(&actions), [u]);
    }

    /// The members of a runner the Host killed land at once (today's
    /// `restart_shared`): their restart does not wait for them, and their
    /// late reap and cleanup change nothing.
    #[test]
    fn members_killed_with_their_runner_are_gone_at_once() {
        let mut s = new();
        let u = running(&mut s, "vestra.one", 1);
        stop(&mut s, u, StopReason::Failed, Then::Inactive);
        s.member_killed(Pid(1));
        assert_eq!(going(&s, u), None);
        // Queued again: loaded again with its spec, started at once.
        s.add_unit(UnitSpec::new("vestra.one", Class::Normal).unrecordable());
        assert_eq!(spawned(&s.run_jobs(0)), Some(Pid(2)));
        started(&mut s, 2);
        s.process_reaped(Pid(1), None);
        s.backend_cleaned(Pid(1));
        assert!(matches!(
            s.unit(u).unwrap().state,
            UnitState::Active { pid: Pid(2), .. }
        ));
    }
}
