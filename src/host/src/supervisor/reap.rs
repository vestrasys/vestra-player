//! Reap (systemd "control group empty" + `ExecStopPost=`): a deactivating
//! unit lands in its `then` once its process is reaped **and** the backend
//! cleaned up after it (T16), or after `STOP_WAIT` 20 s flagged stuck
//! (T18). The "one instance per data directory" rule is a runnability
//! condition: a job never starts a unit while it is `Deactivating`, nor
//! while the process of a stop that landed stuck is still tracked
//! (`Unit::stopping` outside `Deactivating`): it waits `STOP_WAIT` from its
//! own request for that process to go, then fails (§17.7).
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
        self.land_if_done(u);
        self.sync(u);
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
            // start waiting for it goes on, an uninstall removes the data
            // now (§17.7).
            unit.stopping = None;
            Timers::cancel(&mut unit.gens, TimerKind::StopWait);
            if unit.removed {
                self.pending.push(Action::RemoveData { unit: u });
            }
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
                unit.state = UnitState::Inactive;
                if !stuck {
                    if unit.recorded {
                        actions.push(Action::DropCheckpoint { unit: u });
                    }
                    unit.recorded = false;
                    unit.removed = true;
                    actions.push(Action::RemoveData { unit: u });
                }
            }
        }
        self.pending.extend(actions);
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
