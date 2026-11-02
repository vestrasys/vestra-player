//! Reap (systemd "control group empty" + `ExecStopPost=`): a deactivating
//! unit lands in its `then` once its process is reaped **and** the backend
//! cleaned up after it (T16), or after `STOP_WAIT` 20 s flagged stuck
//! (T18). The "one instance per data directory" rule is a runnability
//! condition: a job never starts a unit while it is `Deactivating`.
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
        if self
            .units
            .get(&u)
            .and_then(|unit| unit.stopping)
            .is_some_and(|s| s.done())
        {
            self.land(u, false);
        }
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
        unit.stopping = None;
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
        let job = self
            .units
            .get(&u)
            .and_then(|unit| unit.job.as_ref())
            .map(|job| (job.kind.clone(), job.will_start()));
        match job {
            Some((kind @ (JobKind::Stop { .. } | JobKind::Standby), _)) => {
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
            Some((kind, true)) if stuck => {
                let wake = matches!(
                    kind,
                    JobKind::Start {
                        cause: Cause::Trigger(_)
                    }
                );
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
                // (`start_failure_keeps_registrations` is for a failed
                // spawn): the old process may still run, so the next
                // trigger would start a second instance on the same data.
                if let Some(unit) = self.units.get_mut(&u) {
                    if let UnitState::Standby { holder, .. } = unit.state {
                        self.pending.push(Action::Revoke {
                            unit: u,
                            pid: holder,
                        });
                    }
                    unit.failure = Some(text.to_owned());
                    unit.state = UnitState::Failed {
                        result: FailResult::Stuck,
                        text: text.to_owned(),
                    };
                }
            }
            _ => {}
        }
        self.run.push_back(u);
    }
}
