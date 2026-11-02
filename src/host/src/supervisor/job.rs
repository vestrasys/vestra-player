//! Jobs (systemd `job.c`): nothing changes a unit's state except a job;
//! one installed job per unit; a new request merges with or supersedes it
//! by the table of doc/architecture/supervisor.md §4.2.
#![forbid(unsafe_code)]

use super::trigger::AnswerReason;
use super::unit::{Ms, StopReason, Then, UnitId, UnitState};
use super::{Action, Supervisor};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JobId(pub u64);

/// Third-party start priority (§4.4), lowest first so `max` is the higher.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    Background,
    Event,
    Interactive,
}

/// Why a unit is started (§4.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    Boot,
    FirstRun,
    /// A wake (`wake`), with its trigger's priority.
    Trigger(Priority),
    Enable,
    AutoRestart,
}
impl Cause {
    // B2d: `start`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn priority(self) -> Priority {
        match self {
            Cause::Trigger(priority) => priority,
            Cause::Enable => Priority::Interactive,
            Cause::Boot | Cause::FirstRun | Cause::AutoRestart => Priority::Background,
        }
    }
}

/// §4.1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobKind {
    Start {
        cause: Cause,
    },
    // B2d: the Host's stops as jobs.
    #[cfg_attr(not(test), allow(dead_code))]
    Stop {
        reason: StopReason,
        then: Then,
    },
    /// Stop, wait for the old process, start. `fresh`: drop the checkpoint
    /// (and re-verify a built-in); `image`: start a different package.
    Restart {
        fresh: bool,
        image: Option<String>,
    },
    Standby,
}

/// systemd `--job-mode` (§4.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobMode {
    /// The merge table.
    Replace,
    /// Rejected with `Busy` while a `PackageRpc` job is installed.
    Fail,
}

/// Who asked (decides `JobMode::Fail` conflicts).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Internal,
    /// `packages.setEnabled` (not "busy" for a later RPC, as today).
    User,
    /// `packages.reload`, install, upgrade, uninstall (today `package_busy`).
    PackageRpc,
    Trigger,
    Timer,
    Crash,
    Pressure,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobState {
    Waiting,
    Running,
}

/// How a job ended (systemd job results).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobResult {
    Done,
    /// Includes a start that passed `TimeoutStartSec` and a wait for the
    /// previous process that passed `STOP_WAIT` (systemd reports those as
    /// `timeout`; ours carries today's failure text instead).
    Failed(String),
    Canceled,
    /// Refused by the merge table (a trigger never re-enables, start of a
    /// disabled unit, restart of a disabled unit...).
    Dropped,
}

/// Today's `RpcError("busy", ...)` (`JobMode::Fail`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Busy;
impl std::fmt::Display for Busy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the package is being installed or reloaded")
    }
}
impl std::error::Error for Busy {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub id: JobId,
    pub unit: UnitId,
    pub kind: JobKind,
    pub mode: JobMode,
    pub priority: Priority,
    pub origin: Origin,
    pub state: JobState,
    /// The deadline of the phase it is in, for display.
    pub deadline: Option<Ms>,
    /// Backoff / `hold_until`: not runnable before.
    pub not_before: Option<Ms>,
    pub created: Ms,
    /// Restart: its stop half is done, it now runs as a start.
    pub(super) restarting: bool,
    /// Restart: the stop reason of its stop half.
    pub(super) reason: StopReason,
    /// Start-like and waiting in a lane.
    pub(super) queued: bool,
    /// B2b–B2d (transitional, supervisor.md §17.9): the Host starts the
    /// unit itself once this job may start (`host_start`): where it would
    /// take a lane and `Spawn`, it ends `Done` and the adapter runs
    /// today's start. Goes when the Host's starts become jobs: wakes in
    /// B2c, enable, reload and package operations in B2d.
    pub(super) handoff: Option<Handoff>,
}

/// A hand-off job's own extra wait (B2b–B2d, §17.9): a reload of a
/// built-in also waits for the digest check it asked for, as today's
/// `reload_tick` waits for `!verifying(id)`: until `verify_finished`, or
/// `verify_until` (`VERIFY_WAIT` after the request). B2d's
/// `Activating{Verify}` (T4) replaces it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Handoff {
    pub verify_until: Option<Ms>,
}

/// A start the Host runs itself, waiting for the unit's previous process
/// first (B2b–B2d, §17.9; the B2b rows of §4.7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostStart {
    /// A trigger on a unit in standby (today's `lazy.rs` `wake` with
    /// `Parked.waiting`): `Start{Trigger}`. Removed in B2c.
    Wake(Priority),
    /// `packages.setEnabled(true)` (today's enable-after-park entry in
    /// `packages.reloads`; now after any stop, the D13 fix):
    /// `Start{Enable}`, `JobMode::Fail`. Removed in B2d.
    Enable,
    /// `packages.reload` (today's `packages.reloads` + `reload_tick`):
    /// `Restart{fresh}`, `JobMode::Fail`. Removed in B2d.
    Reload { verify_until: Option<Ms> },
    /// A package operation's start: install, upgrade, reinstall, or the
    /// old version back after a failed or cancelled one (today's
    /// `Operation.waiting`): `Restart{fresh, image}`. The operation is
    /// the package RPC that is busy already, so it merges with what is
    /// installed (`JobMode::Replace`), as today's install goes on beside a
    /// waiting reload. Removed in B2d.
    Install { image: String },
}

/// What `host_start` answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostStarted {
    /// Nothing to wait for: the Host starts now.
    Now,
    /// The job waits; `JobFinished` of `job` says when (`Done`: start now)
    /// or that it failed. `new`: no job was installed before (a request
    /// merged into an installed one is not new).
    Waits { job: JobId, new: bool },
}
impl Job {
    /// Runs as a start now (Start, or Restart past its stop half).
    pub fn starts(&self) -> bool {
        match self.kind {
            JobKind::Start { .. } => true,
            JobKind::Restart { .. } => self.restarting,
            _ => false,
        }
    }
    /// Start or Restart: a trigger can be held for it.
    pub fn will_start(&self) -> bool {
        matches!(self.kind, JobKind::Start { .. } | JobKind::Restart { .. })
    }
    pub fn cause(&self) -> Option<Cause> {
        match self.kind {
            JobKind::Start { cause } => Some(cause),
            _ => None,
        }
    }
}

/// The Restart stop reason for a request (§7.3: reload → `Reload`,
/// image → `Upgrade`).
fn restart_reason(_fresh: bool, image: &Option<String>) -> StopReason {
    if image.is_some() {
        StopReason::Upgrade
    } else {
        StopReason::Reload
    }
}

fn answer_for(then: Then, reason: StopReason) -> AnswerReason {
    match (then, reason) {
        (Then::Removed, _) | (_, StopReason::Uninstall) => AnswerReason::Removed,
        (_, StopReason::Disable) => AnswerReason::Disabled,
        _ => AnswerReason::Stopped,
    }
}

impl<P: Clone> Supervisor<P> {
    /// §5 `enqueue_job`: the merge/supersede table (§4.2) and job modes.
    pub fn enqueue_job(
        &mut self,
        u: UnitId,
        kind: JobKind,
        mode: JobMode,
        priority: Priority,
        origin: Origin,
    ) -> Result<JobId, Busy> {
        let id = JobId(self.next_job);
        self.next_job += 1;
        let Some(unit) = self.units.get(&u) else {
            return Ok(id);
        };
        if mode == JobMode::Fail
            && unit
                .job
                .as_ref()
                .is_some_and(|job| job.origin == Origin::PackageRpc)
        {
            return Err(Busy);
        }
        let job = Job {
            id,
            unit: u,
            reason: match &kind {
                JobKind::Restart { fresh, image } => restart_reason(*fresh, image),
                JobKind::Stop { reason, .. } => *reason,
                _ => StopReason::Idle,
            },
            kind,
            mode,
            priority,
            origin,
            state: JobState::Waiting,
            deadline: None,
            not_before: None,
            created: self.now,
            restarting: false,
            queued: false,
            handoff: None,
        };
        if self.shutting_down {
            self.record(u, id, job.kind, JobResult::Dropped);
            return Ok(id);
        }
        let result = match job.kind.clone() {
            JobKind::Start { cause } => self.merge_start(u, job, cause),
            JobKind::Standby => self.merge_standby(u, job),
            JobKind::Stop { reason, then } => self.merge_stop(u, job, reason, then),
            JobKind::Restart { fresh, image } => self.merge_restart(u, job, fresh, image),
        };
        self.run.push_back(u);
        self.sync(u);
        Ok(result)
    }

    /// Records a job that never got installed (done at once or refused).
    fn settle(&mut self, u: UnitId, job: Job, result: JobResult) -> JobId {
        self.record(u, job.id, job.kind, result);
        job.id
    }

    fn merge_start(&mut self, u: UnitId, mut job: Job, cause: Cause) -> JobId {
        let unit = &self.units[&u];
        if !unit.enabled() || unit.removed {
            return self.settle(u, job, JobResult::Dropped);
        }
        // A trigger has no entry point left on a failed unit (§4.3).
        if matches!(unit.state, UnitState::Failed { .. }) && matches!(cause, Cause::Trigger(_)) {
            return self.settle(u, job, JobResult::Dropped);
        }
        let active = matches!(unit.state, UnitState::Active { .. });
        let deactivating = unit.state.is_deactivating();
        // Today an enable that waits for the previous process is an entry
        // in `packages.reloads`: `package_busy` until it starts; also when
        // that stop already landed stuck (`parked_process`, §17.7).
        if cause == Cause::Enable && unit.stopping.is_some() && self.tunables.enable_wait_busy {
            job.origin = Origin::PackageRpc;
        }
        let Some(installed) = unit.job.as_ref() else {
            if active {
                return self.settle(u, job, JobResult::Done);
            }
            return self.install(u, job);
        };
        let installed_id = installed.id;
        match installed.kind.clone() {
            JobKind::Start { .. } | JobKind::Restart { .. } => {
                self.raise(u, job.priority);
                installed_id
            }
            JobKind::Standby => {
                if deactivating {
                    // Waits for reap, then starts from standby (§4.2).
                    self.finish(u, JobResult::Done);
                    self.install(u, job)
                } else {
                    // Still asking: the unit stays active, the trigger goes now.
                    self.cancel_standby(u);
                    self.settle(u, job, JobResult::Done)
                }
            }
            JobKind::Stop { reason, then } => {
                if then == Then::Removed || reason == StopReason::Uninstall {
                    return self.settle(u, job, JobResult::Dropped);
                }
                if deactivating {
                    // stop, then start → restart: the start waits for reap.
                    let installed = self.units.get_mut(&u).and_then(|unit| unit.job.as_mut());
                    if let Some(installed) = installed {
                        installed.kind = JobKind::Restart {
                            fresh: false,
                            image: None,
                        };
                        installed.reason = reason;
                        installed.priority = installed.priority.max(job.priority);
                        if job.origin != Origin::Internal {
                            installed.origin = job.origin;
                        }
                        // It now starts a process: a wait for a stuck stop
                        // counts from this request (today's `reloads(now)`,
                        // §17.7).
                        installed.created = job.created;
                    }
                    installed_id
                } else {
                    self.finish(u, JobResult::Canceled);
                    if active {
                        self.settle(u, job, JobResult::Done)
                    } else {
                        self.install(u, job)
                    }
                }
            }
        }
    }

    fn merge_standby(&mut self, u: UnitId, job: Job) -> JobId {
        let unit = &self.units[&u];
        let Some(installed) = unit.job.as_ref() else {
            return match unit.state {
                UnitState::Active { .. } => self.install(u, job),
                UnitState::Standby { .. } => self.settle(u, job, JobResult::Done),
                _ => self.settle(u, job, JobResult::Dropped),
            };
        };
        match installed.kind {
            JobKind::Standby => installed.id,
            _ => self.settle(u, job, JobResult::Dropped),
        }
    }

    fn merge_stop(&mut self, u: UnitId, job: Job, reason: StopReason, then: Then) -> JobId {
        let unit = &self.units[&u];
        let deactivating = unit.state.is_deactivating();
        let Some(installed) = unit.job.as_ref() else {
            return self.install(u, job);
        };
        match installed.kind.clone() {
            JobKind::Stop {
                reason: old_reason,
                then: old_then,
            } => {
                let merged = old_then.stronger(then);
                let reason = if merged == old_then {
                    old_reason
                } else {
                    reason
                };
                let id = installed.id;
                if let Some(installed) = self.units.get_mut(&u).and_then(|unit| unit.job.as_mut()) {
                    installed.kind = JobKind::Stop {
                        reason,
                        then: merged,
                    };
                    if job.origin == Origin::PackageRpc {
                        installed.origin = Origin::PackageRpc;
                    }
                }
                self.set_then(u, merged);
                id
            }
            JobKind::Start { .. } | JobKind::Restart { .. } => {
                let answer = answer_for(then, reason);
                self.answer_backlog(u, answer);
                self.finish(u, JobResult::Canceled);
                let id = self.install(u, job);
                if deactivating {
                    self.set_then(u, then);
                }
                id
            }
            JobKind::Standby => {
                self.finish(u, JobResult::Canceled);
                let id = self.install(u, job);
                if deactivating {
                    // Registrations kept for standby are revoked now.
                    self.set_then(u, then);
                } else if let Some(unit) = self.units.get_mut(&u) {
                    unit.asked_at = None;
                }
                id
            }
        }
    }

    fn merge_restart(&mut self, u: UnitId, job: Job, fresh: bool, image: Option<String>) -> JobId {
        let unit = &self.units[&u];
        if unit.removed {
            return self.settle(u, job, JobResult::Dropped);
        }
        let deactivating = unit.state.is_deactivating();
        let Some(installed) = unit.job.as_ref() else {
            // A disabled unit is upgraded/reloaded without starting
            // (`step_restart` ends the job after the stop half, §17.2 Q15).
            let id = self.install(u, job);
            if deactivating {
                // Stopping for standby (B4 crash hold): revoked now (§17.7).
                self.set_then(u, Then::Inactive);
            }
            return id;
        };
        if !unit.enabled() {
            return self.settle(u, job, JobResult::Dropped);
        }
        let installed_id = installed.id;
        let old = installed.kind.clone();
        if let JobKind::Stop { then, reason } = old
            && (then == Then::Removed || reason == StopReason::Uninstall)
        {
            return self.settle(u, job, JobResult::Dropped);
        }
        let was_standby_asking = matches!(old, JobKind::Standby) && !deactivating;
        if was_standby_asking && let Some(unit) = self.units.get_mut(&u) {
            unit.asked_at = None;
        }
        let Some(installed) = self.units.get_mut(&u).and_then(|unit| unit.job.as_mut()) else {
            return installed_id;
        };
        let (fresh, image, reason) = match old {
            JobKind::Restart {
                fresh: old_fresh,
                image: old_image,
            } => {
                let image = image.or(old_image);
                let fresh = fresh || old_fresh;
                (fresh, image.clone(), restart_reason(fresh, &image))
            }
            JobKind::Stop { reason, .. } if deactivating => (fresh, image, reason),
            _ => {
                let reason = restart_reason(fresh, &image);
                (fresh, image, reason)
            }
        };
        installed.kind = JobKind::Restart { fresh, image };
        installed.reason = reason;
        installed.restarting = false;
        installed.queued = false;
        installed.state = if deactivating {
            JobState::Running
        } else {
            JobState::Waiting
        };
        installed.priority = installed.priority.max(job.priority);
        if job.origin == Origin::PackageRpc || installed.origin == Origin::Internal {
            installed.origin = job.origin;
        }
        installed.mode = job.mode;
        // A wait for a stuck stop counts from this request (today's
        // `packages.reloads` entry is written with `now`, §17.7).
        installed.created = job.created;
        // A unit stopping for standby does not land there any more: its
        // kept registrations go now (§17.7).
        if deactivating {
            self.set_then(u, Then::Inactive);
        }
        installed_id
    }

    /// Installs `job` on a unit that has none.
    pub(super) fn install(&mut self, u: UnitId, job: Job) -> JobId {
        let id = job.id;
        if let Some(unit) = self.units.get_mut(&u) {
            debug_assert!(unit.job.is_none());
            unit.job = Some(job);
        }
        self.run.push_back(u);
        id
    }

    /// Raises the installed start's priority (merged trigger); a queued
    /// job moves to the higher lane (the old entry goes stale).
    pub(super) fn raise(&mut self, u: UnitId, priority: Priority) {
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        let Some(job) = unit.job.as_mut() else {
            return;
        };
        if priority > job.priority {
            job.priority = priority;
            if job.queued {
                job.queued = false;
                self.run.push_back(u);
            }
        }
    }

    /// Ends the installed job with `result`.
    pub(super) fn finish(&mut self, u: UnitId, result: JobResult) {
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        let Some(job) = unit.job.take() else {
            return;
        };
        self.record(u, job.id, job.kind, result);
    }

    pub(super) fn record(&mut self, u: UnitId, id: JobId, kind: JobKind, result: JobResult) {
        if let Some(unit) = self.units.get_mut(&u) {
            unit.results.push_back((id, result.clone()));
            while unit.results.len() > 8 {
                unit.results.pop_front();
            }
        }
        self.pending.push(Action::JobFinished {
            unit: u,
            job: id,
            kind,
            result,
        });
    }

    /// A Standby job still asking (unit active) is canceled; idle re-armed.
    pub(super) fn cancel_standby(&mut self, u: UnitId) {
        if let Some(unit) = self.units.get_mut(&u) {
            unit.asked_at = None;
        }
        self.finish(u, JobResult::Canceled);
        self.arm_idle(u);
    }

    // ---- B2b–B2d: starts the Host runs itself (§17.9) -------------------

    /// A start the Host runs itself (`HostStart`), once the unit's
    /// previous process is gone (§4.3 "stop, then start", "wake while
    /// deactivating"; the D13 fix: every start waits, whatever stopped that
    /// process). `Now` when no process of the unit may still run and no
    /// job is installed; else a hand-off job waits `STOP_WAIT` from this
    /// request (`JobFinished` `Done`, or `Failed` with today's text,
    /// `fail_stuck`). `Busy`: `JobMode::Fail` against an installed
    /// package-RPC job (today's `package_busy` reply).
    pub fn host_start(&mut self, u: UnitId, start: HostStart) -> Result<HostStarted, Busy> {
        let Some(unit) = self.units.get(&u) else {
            return Ok(HostStarted::Now);
        };
        let going = unit.stopping.is_some_and(|s| !s.done());
        let installed = unit.job.as_ref().map(|job| job.id);
        if !going && installed.is_none() {
            return Ok(HostStarted::Now);
        }
        let (kind, mode, priority, origin, verify_until) = match start {
            HostStart::Wake(priority) => (
                JobKind::Start {
                    cause: Cause::Trigger(priority),
                },
                JobMode::Replace,
                priority,
                Origin::Trigger,
                None,
            ),
            HostStart::Enable => (
                JobKind::Start {
                    cause: Cause::Enable,
                },
                JobMode::Fail,
                Priority::Interactive,
                Origin::User,
                None,
            ),
            HostStart::Reload { verify_until } => (
                JobKind::Restart {
                    fresh: true,
                    image: None,
                },
                JobMode::Fail,
                Priority::Interactive,
                Origin::PackageRpc,
                verify_until,
            ),
            HostStart::Install { image } => (
                JobKind::Restart {
                    fresh: true,
                    image: Some(image),
                },
                JobMode::Replace,
                Priority::Interactive,
                Origin::PackageRpc,
                None,
            ),
        };
        let job = self.enqueue_job(u, kind, mode, priority, origin)?;
        if let Some(installed) = self
            .units
            .get_mut(&u)
            .and_then(|unit| unit.job.as_mut())
            .filter(|installed| installed.id == job)
        {
            installed.handoff = Some(Handoff { verify_until });
        }
        Ok(HostStarted::Waits {
            job,
            new: installed != Some(job),
        })
    }

    /// The digest check a hand-off reload waits for is over (§17.9).
    pub fn verify_finished(&mut self, u: UnitId) {
        let Some(job) = self.units.get_mut(&u).and_then(|unit| unit.job.as_mut()) else {
            return;
        };
        if let Some(handoff) = job.handoff.as_mut()
            && handoff.verify_until.take().is_some()
        {
            self.run.push_back(u);
        }
    }

    /// A deactivating unit lands somewhere else (stronger stop merged).
    /// Leaving `then: Standby` (disable, uninstall, reload, upgrade of a
    /// unit stopping for standby) revokes the kept registrations at once,
    /// not at reap, as today's `forget_parked` does (coordinator ruling,
    /// §17.7): a trigger to them would be refused anyway.
    pub(super) fn set_then(&mut self, u: UnitId, new: Then) {
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        let UnitState::Deactivating {
            pid, then, reason, ..
        } = &mut unit.state
        else {
            return;
        };
        let merged = then.stronger(new);
        if merged == *then {
            return;
        }
        let was_standby = *then == Then::Standby;
        let pid = *pid;
        *then = merged;
        if merged == Then::Removed {
            *reason = StopReason::Uninstall;
            unit.remove_data = true;
        }
        if was_standby && let Some(stopping) = unit.stopping.as_mut().filter(|s| s.kept) {
            stopping.kept = false;
            self.pending.push(Action::Revoke { unit: u, pid });
        }
        // An uninstall after the reap: the data goes now (§17.7).
        self.flush_data_removal(u);
    }
}
