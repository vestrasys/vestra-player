//! Plugin process supervisor, pure core (doc/architecture/supervisor.md):
//! decides when a plugin process should exist — start, stop, restart,
//! standby, limits. No `Host`, no
//! I/O, no environment, no clock: time comes in as `Ms`, randomness from a
//! seeded xorshift, and every decision goes out as an `Action` for the Host
//! adapter (`host/supervisor/`) to execute. Payloads are generic (`serde_json::Value` in
//! production, small numbers in simulations).
//!
//! Inputs without a time argument use the latest time the supervisor was
//! given (`run_jobs`, `touch`, `begin_work`, `shutdown`, `set_now`).
#![forbid(unsafe_code)]

pub mod activation;
mod exit;
mod identity;
mod job;
mod kill;
mod limits;
mod pressure;
mod queue;
mod reap;
mod restart;
mod timers;
mod trigger;
mod unit;
mod work;

#[cfg(test)]
mod sim;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

pub use exit::ExitCause;
pub use job::{Busy, Cause, Job, JobId, JobKind, JobMode, JobResult, JobState, Origin, Priority};
#[cfg(test)]
pub use kill::grace;
#[cfg(test)]
pub use limits::{Policy, max_active_for, process_budget_for};
pub use limits::{RestartMode, StartLimit, Tunables};
pub use pressure::Pressure;
// The 20 s wait texts: the waits fail with them in their job results. The
// adapter tells a reload's or enable's wait that ran out (today's
// "reload: …" failure) from a failed spawn by the text.
pub use reap::STUCK_RELOAD;
#[cfg(test)]
pub use reap::STUCK_WAKE;
pub use trigger::{AnswerReason, Delivery, Trigger, TriggerKind};
pub use unit::{
    ActivatingSub, Class, DeactivatingSub, Enablement, FailResult, ListState, Ms, Pid, Placement,
    ProcessState, RunnerId, StopReason, Stopping, Then, Unit, UnitId, UnitSpec, UnitState,
};
pub use work::WorkKind;

use queue::{LANES, Lane, Lanes};
use restart::{Rng, RunnerPolicy, retry_delay};
use timers::{Target, TimerKind, Timers};

/// The failure text of a unit that hit its start limit: the cause, the limit,
/// and what starts it again (a reload clears it).
fn start_limit_text(cause: &str, limit: StartLimit) -> String {
    format!(
        "{cause}; stopped after {} failures within {} minutes (reload it to start it again)",
        limit.burst,
        limit.interval / 60_000
    )
}

/// A user-visible notice (OSD; i18n texts in the adapter).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notice {
    /// `"Starting <name>…"` (a wake slower than 400 ms).
    Starting,
    /// `"<name> could not start"`.
    CouldNotStart,
    /// `"<name> stopped"` / `"…ran out of memory"`.
    Crashed(ExitCause),
    /// Start limit hit.
    StartLimitHit,
    /// "Built-in plugins restarted after a fault".
    RunnerRestarted,
    /// "Built-in plugins run separately for this session after repeated
    /// faults".
    RunnerFailedOpen,
}

/// What the Host adapter executes, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action<P> {
    /// Create the process (`process::Plugin::load_manifest`); answer with
    /// `process_spawned`.
    Spawn {
        unit: UnitId,
        pid: Pid,
        placement: Placement,
        image: Option<String>,
        /// No checkpoint: classify at ready (`classified`).
        first_run: bool,
    },
    /// Built-in digest check; answer with `verify_done`.
    Verify {
        unit: UnitId,
    },
    /// Replay the checkpoint's registrations under `holder`.
    Replay {
        unit: UnitId,
        holder: Pid,
    },
    /// Move registrations `from` → `to` (`capabilities::rekey`).
    Rekey {
        unit: UnitId,
        from: Pid,
        to: Pid,
    },
    /// At ready after a start from standby: drop recorded entries the live
    /// process did not register again.
    Prune {
        unit: UnitId,
        pid: Pid,
    },
    /// Send a held trigger (`Plugin::send`).
    Deliver {
        unit: UnitId,
        pid: Pid,
        payload: P,
    },
    /// Reply failed to a held trigger.
    Answer {
        unit: UnitId,
        reason: AnswerReason,
        payload: P,
    },
    /// Ask `owner.resources`; answer with `resources_answered`.
    QueryResources {
        unit: UnitId,
        pid: Pid,
    },
    /// Write the checkpoint (recording) from the registrations under `pid`.
    Checkpoint {
        unit: UnitId,
        pid: Pid,
        resident: bool,
    },
    DropCheckpoint {
        unit: UnitId,
    },
    /// `lifecycle.rs::stop`'s cleanup, then the stop message; `Kill` after
    /// `grace`. Answer with `process_reaped` and `backend_cleaned`.
    Terminate {
        unit: UnitId,
        pid: Pid,
        reason: StopReason,
        grace: Ms,
        keep_registrations: bool,
    },
    Kill {
        unit: UnitId,
        pid: Pid,
    },
    /// `retire(false)` of that runner (all its members).
    KillRunner {
        runner: RunnerId,
    },
    /// `capabilities::revoke` of the registrations under `pid`.
    Revoke {
        unit: UnitId,
        pid: Pid,
    },
    /// Remove the unit's data (uninstall), then `remove_unit` while the unit is
    /// still removed and tracks no process. What it deletes is what the adapter
    /// resolved at each uninstall request: never resolved from the unit when
    /// the action comes, since a reinstall may own it by then; one action may
    /// cover two uninstalls across a reinstall.
    RemoveData {
        unit: UnitId,
    },
    Notify {
        unit: Option<UnitId>,
        notice: Notice,
    },
    /// Its `packages.list` entry changed (`process` excluded).
    ListChanged {
        unit: UnitId,
    },
    /// A job ended (package transactions read the result). `kind`: what it was
    /// when it ended (a request merged into it may have changed it), so the
    /// adapter knows whose wait ended.
    JobFinished {
        unit: UnitId,
        job: JobId,
        kind: JobKind,
        result: JobResult,
    },
}

/// The supervisor: units, jobs, timers and limits.
pub struct Supervisor<P> {
    tunables: Tunables,
    units: BTreeMap<UnitId, Unit<P>>,
    by_id: HashMap<String, UnitId>,
    by_pid: HashMap<Pid, UnitId>,
    next_unit: u64,
    next_pid: u64,
    next_job: u64,
    now: Ms,
    timers: Timers,
    shutdown_generation: u64,
    lanes: Lanes,
    /// Units whose installed job may progress.
    run: VecDeque<UnitId>,
    pending: Vec<Action<P>>,
    /// Third-party starts holding a slot (`max_active`).
    slots: BTreeSet<UnitId>,
    /// Built-ins with a start installed and not yet past `Verify`.
    builtin_pending: BTreeSet<UnitId>,
    /// Third-party start jobs waiting in a lane.
    queued_third: BTreeSet<UnitId>,
    /// Third-party processes counted in the budget.
    procs_third: BTreeSet<UnitId>,
    runner: RunnerId,
    runner_policy: RunnerPolicy,
    runner_restart: restart::RestartState,
    rng: Rng,
    muted: bool,
    quiet_until: Ms,
    pressure: Pressure,
    shutting_down: bool,
    /// Start-up starts still waiting (today's `StartQueue` entries:
    /// `boot_pending`).
    boot_waiting: BTreeSet<UnitId>,
    /// First runs waiting for a place (today's `FirstQueue`:
    /// `first_runs_queued`).
    first_waiting: BTreeSet<UnitId>,
    /// Built-ins the adapter has still to load (`set_builtins_to_load`):
    /// third-party starts wait for them as for a built-in with a start
    /// installed.
    builtins_to_load: usize,
}

/// A start-up start that has not started yet (today's `StartQueue`
/// entries): a boot start (or a runner member's restart) waiting for its
/// turn or its digest check.
fn waits_at_boot<P>(unit: &Unit<P>) -> bool {
    let Some(job) = unit.job.as_ref() else {
        return false;
    };
    let verifying = matches!(
        unit.state,
        UnitState::Activating {
            sub: ActivatingSub::Verify,
            ..
        }
    );
    let boot = match job.kind {
        JobKind::Start { cause } => cause == Cause::Boot,
        JobKind::Restart { .. } => job.origin == Origin::Crash,
        _ => false,
    };
    boot && (verifying
        || matches!(
            unit.state,
            UnitState::Inactive | UnitState::Deactivating { .. }
        ))
}

/// The unit's job is a Standby job that asked for its resources (or is
/// asking: `step_standby` set it running).
fn standby_running<P>(unit: &Unit<P>) -> bool {
    matches!(
        unit.job.as_ref(),
        Some(Job {
            kind: JobKind::Standby,
            state: JobState::Running,
            ..
        })
    )
}

/// The idle time after which the unit's run goes to standby: the idle
/// timeout, or a lazy first run's quiet period (`Unit::quiet_park`).
fn idle_time<P>(tunables: &Tunables, unit: &Unit<P>) -> Ms {
    if unit.quiet_park {
        tunables.first_run_quiet
    } else {
        tunables.idle_timeout
    }
}

/// A first run waiting for a place (today's `FirstQueue` entry).
fn first_queued<P>(unit: &Unit<P>) -> bool {
    matches!(unit.state, UnitState::Inactive)
        && unit.job.as_ref().is_some_and(|job| {
            job.cause() == Some(Cause::FirstRun) && job.state == JobState::Waiting
        })
}

/// What `packages.list` reads of the unit table, one entry per unit in
/// unit order (`Supervisor::list_key`): its list state, its standby holder
/// (`holder_of`), whether its run was woken and is not ready yet
/// (`waking`), whether a package RPC's job is on it (`package_busy`).
/// `by_id` changes only with `units` (`load_unit`, `add_unit`,
/// `remove_unit`), and unit ids are never reused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListKey(Vec<(UnitId, ListState, Option<Pid>, bool, bool)>);

/// `packages.list` of one unit at `now`. A run's activation (`activation.rs`:
/// `runtime.ready` and its initial UI requests) says whether it is still
/// starting, as `package_list` reads the activation of a running plugin.
fn list_state<P>(unit: &Unit<P>, now: Ms) -> ListState {
    if unit.removed {
        return ListState::Stopped;
    }
    // Today's `package_list` puts `disabled` before a failure
    // (`set_enabled(false)` clears it too).
    if !unit.enabled() {
        return ListState::Disabled;
    }
    let will_start = unit.job.as_ref().is_some_and(Job::will_start);
    let woken = matches!(
        unit.job.as_ref().and_then(Job::cause),
        Some(Cause::Trigger(_))
    );
    let restart = matches!(
        unit.job.as_ref().map(|job| &job.kind),
        Some(JobKind::Restart { .. })
    );
    // A spawned run: its activation says (today's `Status` of a plugin in
    // `plugins`; one woken from standby was running all along until its
    // ready, `lazy.waking`).
    if let Some(pid) = match unit.state {
        UnitState::Activating {
            pid: Some(pid),
            sub: ActivatingSub::Ready,
            ..
        }
        | UnitState::Active { pid, .. } => Some(pid),
        _ => None,
    } && let Some(run) = unit.activation.as_ref().filter(|run| run.pid == pid)
    {
        let woken = unit.from_standby && matches!(unit.state, UnitState::Activating { .. });
        let running = ListState::Running {
            process: ProcessState::Running,
        };
        return match run.activation.status_at(now) {
            activation::Status::Pending { .. } if woken => running,
            activation::Status::Pending { .. } => ListState::Starting,
            activation::Status::Failed { message } => ListState::Failed(message),
            activation::Status::TimedOut => {
                ListState::Failed(ExitCause::StartTimeout.text().to_owned())
            }
            _ => running,
        };
    }
    match &unit.state {
        UnitState::Failed { text, .. } => return ListState::Failed(text.clone()),
        UnitState::Deactivating {
            then: Then::Failed, ..
        } => {
            let text = unit
                .failing
                .as_ref()
                .map_or_else(|| ExitCause::Exited.text().to_owned(), |(_, t)| t.clone());
            return ListState::Failed(text);
        }
        _ => {}
    }
    let idle = ListState::Running {
        process: ProcessState::Idle,
    };
    match &unit.state {
        UnitState::Inactive if will_start => ListState::Starting,
        UnitState::Inactive => ListState::Stopped,
        UnitState::Activating {
            sub: ActivatingSub::AutoRestart { .. },
            ..
        } => idle,
        // Woken from standby: it was running all along (today's
        // `package_list` for `lazy.waking`), so a wake announces nothing.
        UnitState::Activating { .. } if woken => ListState::Running {
            process: ProcessState::Running,
        },
        UnitState::Activating { .. } => ListState::Starting,
        UnitState::Active { .. } => ListState::Running {
            process: ProcessState::Running,
        },
        UnitState::Standby { .. } => idle,
        UnitState::Deactivating {
            then: Then::Standby,
            ..
        } if !restart => idle,
        UnitState::Deactivating { .. } if will_start => ListState::Starting,
        UnitState::Deactivating { .. } => ListState::Stopped,
        UnitState::Failed { text, .. } => ListState::Failed(text.clone()),
    }
}

impl<P: Clone> Supervisor<P> {
    pub fn new(tunables: Tunables, seed: u64) -> Self {
        Self {
            tunables,
            units: BTreeMap::new(),
            by_id: HashMap::new(),
            by_pid: HashMap::new(),
            next_unit: 1,
            next_pid: 1,
            next_job: 1,
            now: 0,
            timers: Timers::default(),
            shutdown_generation: 0,
            lanes: Lanes::default(),
            run: VecDeque::new(),
            pending: Vec::new(),
            slots: BTreeSet::new(),
            builtin_pending: BTreeSet::new(),
            queued_third: BTreeSet::new(),
            procs_third: BTreeSet::new(),
            runner: RunnerId(1),
            runner_policy: RunnerPolicy::default(),
            runner_restart: restart::RestartState::default(),
            rng: Rng::new(seed),
            muted: false,
            quiet_until: 0,
            pressure: Pressure::None,
            shutting_down: false,
            boot_waiting: BTreeSet::new(),
            first_waiting: BTreeSet::new(),
            builtins_to_load: 0,
        }
    }

    pub fn tunables(&self) -> &Tunables {
        &self.tunables
    }
    /// The latest time given.
    pub fn now(&self) -> Ms {
        self.now
    }
    /// Advances the supervisor's time without running jobs (for inputs that
    /// carry no time).
    pub fn set_now(&mut self, now: Ms) {
        self.now = self.now.max(now);
    }
    /// The current shared runner.
    pub fn runner(&self) -> RunnerId {
        self.runner
    }
    /// Members run in their own processes for the session (fail-open).
    #[cfg(test)]
    pub fn failed_open(&self) -> bool {
        self.runner_policy.failed_open
    }

    // ---- units ---------------------------------------------------------

    /// `add_unit`: load a unit. The id of a unit being removed is a
    /// reinstall; the id of a loaded unit with no process, no job and no
    /// process still going (a runner member queued again, a development load or
    /// a unit loaded for its identity only) takes the new spec and starts as at
    /// boot (reloaded, then started). Any other loaded unit
    /// is left as it is.
    pub fn add_unit(&mut self, spec: UnitSpec) -> UnitId {
        let mut unit = Unit::new(&spec);
        if unit.class == Class::Normal && unit.resident {
            unit.class = Class::Resident;
        }
        let class = unit.class;
        let (recordable, recorded, resident) = (unit.recordable, unit.recorded, unit.resident);
        let replay =
            spec.enabled && recorded && !resident && self.tunables.policy(class).idle_standby;
        let u = if let Some(u) = self.by_id.get(&spec.id).copied() {
            // Loaded already; a reinstall of a unit being removed comes back,
            // an idle unit takes the new spec.
            if !self.reinstall(u, unit) {
                return u;
            }
            u
        } else {
            let u = UnitId(self.next_unit);
            self.next_unit += 1;
            self.by_id.insert(spec.id.clone(), u);
            self.units.insert(u, unit);
            u
        };
        if !spec.enabled {
            self.sync(u);
            return u;
        }
        // A reinstall that still waits for the uninstalled process starts (once
        // it is gone), never replays meanwhile: the replay would drop the pid
        // that process is tracked under (a reinstall carries no checkpoint
        // anyway).
        let going = self
            .units
            .get(&u)
            .is_some_and(|unit| unit.stopping.is_some());
        if replay && !going {
            self.replay(u);
        } else {
            // No recording key (a development load, a built-in without a
            // digest): started at boot, never a first run.
            let first = recordable && !recorded;
            let cause = if first { Cause::FirstRun } else { Cause::Boot };
            let _ = self.enqueue_job(
                u,
                JobKind::Start { cause },
                JobMode::Replace,
                Priority::Background,
                Origin::Internal,
            );
        }
        self.sync(u);
        u
    }

    /// `add_unit` of the id of a loaded unit: a unit without a process, a job
    /// or a process still going takes the new spec (`true`), keeping its run
    /// history (results, timers, runner quarantine, last pid). Otherwise the id
    /// of a unit being removed (uninstalled, its `remove_unit` not called yet:
    /// its stopped process may still run, or its uninstall is still stopping
    /// it): a reinstall. The unit takes the new spec under the same `UnitId`
    /// and keeps what the old install left: the tracked process (a start waits
    /// `STOP_WAIT` from its request for it, as after any stuck stop), its
    /// timers and job results, and an owed `RemoveData` (emitted at that
    /// process's reap, so before the new spawn). An uninstall still stopping
    /// now lands `Inactive` and its Stop job ends `Done`. The adapter resolves
    /// which directory `RemoveData` deletes at the uninstall request, never
    /// from the reinstalled unit. Not a removed unit: `false`.
    fn reinstall(&mut self, u: UnitId, mut fresh: Unit<P>) -> bool {
        let Some(unit) = self.units.get_mut(&u) else {
            return false;
        };
        let idle = !unit.removed
            && unit.job.is_none()
            && unit.stopping.is_none()
            && unit.activation.is_none()
            && matches!(unit.state, UnitState::Inactive | UnitState::Failed { .. });
        if idle {
            fresh.placement = unit.placement;
            fresh.quarantined = unit.quarantined;
            fresh.last_pid = unit.last_pid;
            fresh.restart = std::mem::take(&mut unit.restart);
            fresh.gens = std::mem::take(&mut unit.gens);
            fresh.results = std::mem::take(&mut unit.results);
            fresh.listed = unit.listed.take();
            *unit = fresh;
            return true;
        }
        let uninstalling = matches!(
            unit.state,
            UnitState::Deactivating {
                then: Then::Removed,
                ..
            }
        );
        if !unit.removed && !uninstalling {
            return false;
        }
        fresh.state = unit.state.clone();
        if let UnitState::Deactivating { then, .. } = &mut fresh.state {
            *then = Then::Inactive;
        }
        fresh.placement = unit.placement;
        fresh.stopping = unit.stopping;
        fresh.remove_data = unit.remove_data;
        fresh.last_pid = unit.last_pid;
        fresh.job = unit.job.take();
        fresh.activation = unit.activation.take();
        fresh.gens = std::mem::take(&mut unit.gens);
        fresh.results = std::mem::take(&mut unit.results);
        *unit = fresh;
        let uninstall = unit.job.as_ref().is_some_and(|job| {
            matches!(
                job.kind,
                JobKind::Stop {
                    then: Then::Removed,
                    ..
                } | JobKind::Stop {
                    reason: StopReason::Uninstall,
                    ..
                }
            )
        });
        if uninstall {
            self.finish(u, JobResult::Done);
        }
        true
    }

    /// `remove_unit` (after `RemoveData` / development unload). The adapter
    /// calls it only while the unit is still removed and tracks no process.
    pub fn remove_unit(&mut self, u: UnitId) {
        let Some(unit) = self.units.remove(&u) else {
            return;
        };
        self.by_id.remove(&unit.id);
        let run = unit.activation.as_ref().map(|run| run.pid);
        for pid in [unit.pid(), unit.alias, unit.last_pid, unit.holder, run]
            .into_iter()
            .flatten()
        {
            if self.by_pid.get(&pid) == Some(&u) {
                self.by_pid.remove(&pid);
            }
        }
        if let UnitState::Standby { holder, .. } = unit.state
            && self.by_pid.get(&holder) == Some(&u)
        {
            self.by_pid.remove(&holder);
        }
        self.sync(u);
    }

    /// Whether `remove_unit` may go now: the unit is still removed (a reinstall
    /// did not take it back), has no job, tracks no process and owes no
    /// `RemoveData`.
    pub fn removable(&self, u: UnitId) -> bool {
        self.units.get(&u).is_some_and(|unit| {
            unit.removed && unit.job.is_none() && unit.stopping.is_none() && !unit.remove_data
        })
    }

    /// `set_enabled`: unit-file state only; does not enqueue.
    pub fn set_enabled(&mut self, u: UnitId, enabled: bool) {
        if let Some(unit) = self.units.get_mut(&u) {
            unit.enablement = if enabled {
                Enablement::Enabled
            } else {
                Enablement::Disabled
            };
        }
        self.sync(u);
    }

    /// A package RPC is in flight on `u` (today's `package_busy`): the next
    /// `JobMode::Fail` request is `Busy`. Checked before changing anything.
    pub fn package_busy(&self, u: UnitId) -> bool {
        self.units
            .get(&u)
            .and_then(|unit| unit.job.as_ref())
            .is_some_and(|job| job.origin == Origin::PackageRpc)
    }

    /// `packages.setEnabled(false)`: `Busy` before anything changes (as
    /// today), else the unit-file state and the `Stop{Disable}` together.
    pub fn disable(&mut self, u: UnitId) -> Result<JobId, Busy> {
        if self.package_busy(u) {
            return Err(Busy);
        }
        self.set_enabled(u, false);
        self.stop(u, StopReason::Disable, Then::Inactive)
    }

    /// `packages.setEnabled(true)`: `Busy` before anything changes, else the
    /// unit-file state and the `Start{Enable}` together.
    pub fn enable(&mut self, u: UnitId) -> Result<JobId, Busy> {
        if self.package_busy(u) {
            return Err(Busy);
        }
        self.set_enabled(u, true);
        self.start(u, Cause::Enable)
    }

    // ---- requests ------------------------------------------------------

    /// `start`. `Enable` is the user's (`JobMode::Fail` against a package RPC,
    /// as `set_enabled` is today); other causes are internal.
    pub fn start(&mut self, u: UnitId, cause: Cause) -> Result<JobId, Busy> {
        let (mode, origin) = match cause {
            Cause::Enable => (JobMode::Fail, Origin::User),
            Cause::Trigger(_) => (JobMode::Replace, Origin::Trigger),
            Cause::AutoRestart => (JobMode::Replace, Origin::Crash),
            Cause::Boot | Cause::FirstRun => (JobMode::Replace, Origin::Internal),
        };
        self.enqueue_job(u, JobKind::Start { cause }, mode, cause.priority(), origin)
    }

    /// `stop`. Disable is the user's, uninstall/reload/upgrade are package RPCs
    /// (both `JobMode::Fail`); idle/failed/quit are internal.
    pub fn stop(&mut self, u: UnitId, reason: StopReason, then: Then) -> Result<JobId, Busy> {
        let (mode, origin) = match reason {
            StopReason::Disable => (JobMode::Fail, Origin::User),
            StopReason::Uninstall | StopReason::Reload | StopReason::Upgrade => {
                (JobMode::Fail, Origin::PackageRpc)
            }
            StopReason::Quit | StopReason::Idle | StopReason::Failed => {
                (JobMode::Replace, Origin::Internal)
            }
        };
        self.enqueue_job(
            u,
            JobKind::Stop { reason, then },
            mode,
            Priority::Background,
            origin,
        )
    }

    /// `restart` (a package RPC: reload, install, upgrade, cancel).
    pub fn restart(
        &mut self,
        u: UnitId,
        fresh: bool,
        image: Option<String>,
    ) -> Result<JobId, Busy> {
        self.enqueue_job(
            u,
            JobKind::Restart { fresh, image },
            JobMode::Fail,
            Priority::Interactive,
            Origin::PackageRpc,
        )
    }

    /// A Standby job from outside. The budget, memory pressure and the idle
    /// timer enqueue theirs inside the core; only tests ask for one.
    #[cfg(test)]
    pub fn standby(&mut self, u: UnitId) -> JobId {
        self.enqueue_job(
            u,
            JobKind::Standby,
            JobMode::Replace,
            Priority::Background,
            Origin::Internal,
        )
        .unwrap_or(JobId(0))
    }

    /// `wake`: a trigger on an entry point of `pid` (held in the unit's
    /// backlog until its process is ready). The
    /// trigger is borrowed: on `Now`/`Forward` the caller sends the message
    /// it still owns; a held or answered trigger is copied into the core.
    ///
    /// `pid` must be the unit's current process, its holder (standby), or
    /// the previous pid of a run that is still going (forwarded, today's
    /// `lazy.moved`). Any other pid of the unit is dead: dropped, as a
    /// message to a stopped owner is today.
    pub fn wake(&mut self, pid: Pid, trigger: &Trigger<P>) -> Delivery {
        if self.shutting_down {
            return Delivery::Dropped;
        }
        let Some(&u) = self.by_pid.get(&pid) else {
            return Delivery::Dropped;
        };
        let muted = matches!(trigger.kind, TriggerKind::Event(_)) && self.events_muted();
        let now = self.now;
        let backlog_max = self.tunables.backlog_max;
        let Some(unit) = self.units.get_mut(&u) else {
            return Delivery::Dropped;
        };
        // A muted wake event starts nothing; a unit whose start already
        // spawned holds it for its ready all the same (today's broadcast
        // holds wake events for `lazy.waking` whatever `lazy_event` mutes).
        if muted && !matches!(unit.state, UnitState::Activating { pid: Some(_), .. }) {
            return Delivery::Dropped;
        }
        if !unit.enabled() || unit.removed {
            return Delivery::Dropped;
        }
        let holder = match unit.state {
            UnitState::Standby { holder, .. } => Some(holder),
            UnitState::Activating { pid: None, .. } => unit.holder,
            _ => None,
        };
        let live = unit.pid() == Some(pid) || holder == Some(pid) || unit.alias == Some(pid);
        if !live {
            return Delivery::Dropped;
        }
        match &mut unit.state {
            UnitState::Active {
                pid: current,
                last_active,
                ..
            } => {
                let current = *current;
                // Sent to the current process (an old pid is forwarded to it),
                // after the job rules: an old pid cannot bypass a restart's
                // absorption.
                let now_or_forward = if current == pid {
                    Delivery::Now
                } else {
                    Delivery::Forward(current)
                };
                let kind = unit.job.as_ref().map(|j| j.kind.clone());
                match kind {
                    None | Some(JobKind::Start { .. }) => {
                        *last_active = (*last_active).max(now);
                        return now_or_forward;
                    }
                    Some(JobKind::Standby) => {
                        *last_active = (*last_active).max(now);
                        self.cancel_standby(u);
                        self.sync(u);
                        return now_or_forward;
                    }
                    // Restart[any] × Start: the start is absorbed, the trigger
                    // held for the new run.
                    Some(JobKind::Restart { .. }) => {
                        if trigger.kind == TriggerKind::Follow {
                            unit.backlog.push_back(trigger.clone());
                            return Delivery::Held;
                        }
                        if trigger::would_overflow(&unit.backlog, trigger, backlog_max) {
                            self.answer(u, AnswerReason::Busy, trigger.clone());
                            return Delivery::Refused;
                        }
                        let priority = trigger.priority();
                        trigger::hold(&mut unit.backlog, trigger.clone());
                        self.raise(u, priority);
                        self.sync(u);
                        return Delivery::Held;
                    }
                    // Stop[waiting] × Start: the start replaces the stop
                    // (nothing to do on an active unit), except an uninstall (a
                    // trigger never re-enables).
                    Some(JobKind::Stop { .. }) => {
                        let priority = trigger.priority();
                        let accepted = self.enqueue_job(
                            u,
                            JobKind::Start {
                                cause: Cause::Trigger(priority),
                            },
                            JobMode::Replace,
                            priority,
                            Origin::Trigger,
                        );
                        let stopped = self.units.get(&u).is_none_or(|unit| unit.job.is_some());
                        if accepted.is_err() || stopped {
                            return Delivery::Dropped;
                        }
                        self.touch(current, now);
                        return now_or_forward;
                    }
                }
            }
            UnitState::Failed { .. } => return Delivery::Dropped,
            _ => {}
        }
        let will_start = unit.job.as_ref().is_some_and(Job::will_start);
        let holding = will_start
            || matches!(
                unit.state,
                UnitState::Standby { .. }
                    | UnitState::Activating { .. }
                    | UnitState::Deactivating {
                        then: Then::Standby,
                        ..
                    }
            );
        if !holding {
            return Delivery::Dropped;
        }
        if trigger.kind == TriggerKind::Follow {
            if will_start || !unit.backlog.is_empty() {
                unit.backlog.push_back(trigger.clone());
                return Delivery::Held;
            }
            return Delivery::Dropped;
        }
        if let UnitState::Standby {
            hold_until: Some(until),
            ..
        } = unit.state
            && until > now
            && trigger.answered()
        {
            self.answer(u, AnswerReason::Held { until }, trigger.clone());
            return Delivery::Refused;
        }
        if trigger::would_overflow(&unit.backlog, trigger, backlog_max) {
            if trigger.answered() {
                self.answer(u, AnswerReason::Busy, trigger.clone());
            }
            return Delivery::Refused;
        }
        let priority = trigger.priority();
        if will_start {
            trigger::hold(&mut unit.backlog, trigger.clone());
            self.raise(u, priority);
            self.sync(u);
            return Delivery::Held;
        }
        let builtin = unit.class == Class::Builtin;
        if !builtin && self.queued_third.len() >= self.tunables.lane_max {
            if trigger.answered() {
                self.answer(u, AnswerReason::Busy, trigger.clone());
            }
            return Delivery::Refused;
        }
        let Ok(job) = self.enqueue_job(
            u,
            JobKind::Start {
                cause: Cause::Trigger(priority),
            },
            JobMode::Replace,
            priority,
            Origin::Trigger,
        ) else {
            return Delivery::Dropped;
        };
        let Some(unit) = self.units.get_mut(&u) else {
            return Delivery::Dropped;
        };
        if unit.job.as_ref().is_some_and(|j| j.id == job) {
            trigger::hold(&mut unit.backlog, trigger.clone());
            self.sync(u);
            Delivery::Started
        } else {
            Delivery::Dropped
        }
    }

    /// `kill`: escalate the stopping process now (not a job).
    pub fn kill(&mut self, u: UnitId) {
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        if let UnitState::Deactivating { pid, sub, .. } = &mut unit.state
            && unit.stopping.is_some_and(|s| !s.reaped)
            && *sub == DeactivatingSub::Term
        {
            *sub = DeactivatingSub::Kill;
            let pid = *pid;
            Timers::cancel(&mut unit.gens, TimerKind::StopGrace);
            self.pending.push(Action::Kill { unit: u, pid });
        }
    }

    /// `reset_failed`. The adapter's enable, reload and install
    /// clear a failure by their start (`begin_start`); only tests call it.
    #[cfg(test)]
    pub fn reset_failed(&mut self, u: UnitId) {
        if let Some(unit) = self.units.get_mut(&u) {
            if matches!(unit.state, UnitState::Failed { .. }) {
                unit.state = UnitState::Inactive;
            }
            unit.failure = None;
            unit.stuck = false;
            unit.restart.crashes.clear();
            unit.restart.streak = 0;
        }
        self.sync(u);
    }

    /// `job`: the installed job.
    pub fn job(&self, u: UnitId) -> Option<&Job> {
        self.units.get(&u).and_then(|unit| unit.job.as_ref())
    }
    /// The result of a job of `u` that ended (the last 8 are kept).
    pub fn job_result(&self, u: UnitId, job: JobId) -> Option<&JobResult> {
        self.units.get(&u).and_then(|unit| {
            unit.results
                .iter()
                .rev()
                .find(|(id, _)| *id == job)
                .map(|(_, r)| r)
        })
    }

    // ---- decision step -------------------------------------------------

    /// `run_jobs`: due timers, runnable jobs, start slots. O(due + started +
    /// inputs since the last call); never a scan over all units.
    pub fn run_jobs(&mut self, now: Ms) -> Vec<Action<P>> {
        self.now = self.now.max(now);
        loop {
            let mut progressed = false;
            while let Some(entry) = self.timers.pop_due(self.now) {
                self.fire(entry);
                progressed = true;
            }
            while let Some(u) = self.run.pop_front() {
                self.step(u);
                progressed = true;
            }
            if self.start_from_lanes() {
                progressed = true;
            }
            if !progressed {
                break;
            }
        }
        // Stale entries at the top go, so `next_deadline` is exact.
        while let Some(entry) = self.timers.peek() {
            if !self.stale(&entry) {
                break;
            }
            self.timers.pop();
        }
        std::mem::take(&mut self.pending)
    }

    fn stale(&self, entry: &timers::Entry) -> bool {
        match entry.target {
            Target::Shutdown => entry.generation != self.shutdown_generation,
            Target::Unit(u, kind) => self
                .units
                .get(&u)
                .is_none_or(|unit| unit.gens.get(kind) != entry.generation),
        }
    }

    /// When `run_jobs` has something to do (`now` while actions or runnable
    /// jobs wait). The adapter runs the jobs every host tick; only tests
    /// ask.
    #[cfg(test)]
    pub fn next_deadline(&self) -> Option<Ms> {
        if !self.pending.is_empty() || !self.run.is_empty() {
            return Some(self.now);
        }
        self.timers.next()
    }

    /// `shutdown` (quit / device loss): checkpoints from the recorded state,
    /// every process terminated in parallel with one deadline `budget`, then
    /// killed; the process of a stop that landed stuck is killed at once. Wakes
    /// are dropped from now on.
    pub fn shutdown(&mut self, now: Ms, budget: Ms) -> Vec<Action<P>> {
        self.now = self.now.max(now);
        self.shutting_down = true;
        let units = self.units.keys().copied().collect::<Vec<_>>();
        // Every checkpoint before the first Terminate ("checkpoints first",
        // coordinator ruling): if the forced exit comes mid-way, each one is
        // already written.
        for (unit, pid, resident) in self.quit_checkpoints() {
            self.pending.push(Action::Checkpoint {
                unit,
                pid,
                resident,
            });
        }
        for u in units {
            let Some(unit) = self.units.get_mut(&u) else {
                continue;
            };
            unit.backlog.clear();
            let job = unit.job.take();
            let state = unit.state.clone();
            let stopping = unit.stopping;
            if let Some(job) = job {
                self.record(u, job.id, job.kind, JobResult::Canceled);
            }
            // The process of a stop that landed stuck: still tracked, past its
            // grace; killed now, so quit leaves no plugin process behind.
            if !state.is_deactivating()
                && let Some(stopping) = stopping.filter(|s| !s.reaped)
            {
                self.pending.push(Action::Kill {
                    unit: u,
                    pid: stopping.pid,
                });
            }
            match state {
                UnitState::Active { .. } | UnitState::Activating { pid: Some(_), .. } => {
                    self.terminate_with(u, StopReason::Quit, Then::Inactive, false, budget);
                }
                UnitState::Activating { pid: None, .. } => {
                    if let Some(unit) = self.units.get_mut(&u) {
                        unit.state = UnitState::Inactive;
                    }
                }
                _ => {}
            }
            self.sync(u);
        }
        self.shutdown_generation += 1;
        self.timers.push(
            self.now + budget,
            Target::Shutdown,
            self.shutdown_generation,
        );
        self.run_jobs(now)
    }

    /// The checkpoints a quit writes (`shutdown`'s first pass), unit by unit:
    /// as today's `lazy_quit`, every run whose class came back this run, unless
    /// its checkpoint already says resident and it still is. The adapter writes
    /// them at quit (`shutdown_plugins`) from the registrations as they are
    /// then.
    pub fn quit_checkpoints(&self) -> Vec<(UnitId, Pid, bool)> {
        self.units
            .iter()
            .filter_map(|(u, unit)| match unit.state {
                UnitState::Active { pid, .. }
                    if unit.classified_run
                        && !(unit.resident && unit.recorded && unit.recorded_resident) =>
                {
                    Some((*u, pid, unit.resident))
                }
                _ => None,
            })
            .collect()
    }

    // ---- event inputs --------------------------------------------------

    /// `process_spawned`.
    pub fn process_spawned(&mut self, pid: Pid, result: Result<(), String>) {
        let Some(u) = self.current(pid) else {
            return;
        };
        let spawning = matches!(
            self.units[&u].state,
            UnitState::Activating {
                sub: ActivatingSub::Spawn,
                ..
            }
        );
        if !spawning {
            return;
        }
        match result {
            Ok(()) => {
                if let Some(unit) = self.units.get_mut(&u)
                    && let UnitState::Activating { sub, .. } = &mut unit.state
                {
                    *sub = ActivatingSub::Ready;
                }
                self.try_ready(u);
            }
            Err(text) => self.spawn_failed(u, pid, text),
        }
        self.sync(u);
    }

    /// `verify_done`: not verified → its own process this run.
    pub fn verify_done(&mut self, u: UnitId, ok: bool) {
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        if !matches!(
            unit.state,
            UnitState::Activating {
                sub: ActivatingSub::Verify,
                ..
            }
        ) {
            return;
        }
        unit.verify = false;
        unit.own_this_run = !ok;
        Timers::cancel(&mut unit.gens, TimerKind::Verify);
        self.spawn(u);
        self.sync(u);
    }

    /// `process_ready` (`runtime.ready`; once the UI activation is done too).
    pub fn process_ready(&mut self, pid: Pid) {
        let Some(u) = self.current(pid) else {
            return;
        };
        if let Some(unit) = self.units.get_mut(&u)
            && let UnitState::Activating { sub, .. } = &mut unit.state
            && matches!(sub, ActivatingSub::Spawn | ActivatingSub::Ready)
        {
            *sub = ActivatingSub::Ready;
            unit.runtime_ready = true;
        }
        self.try_ready(u);
        self.sync(u);
    }

    /// The run `pid` the core placed in the shared runner runs in its own
    /// process after all (the Host's trust check said so: a built-in not
    /// from the installation's own directory, say): a runner failure does
    /// not reach it.
    pub fn placed_own(&mut self, pid: Pid) {
        if let Some(u) = self.current(pid)
            && let Some(unit) = self.units.get_mut(&u)
        {
            unit.placement = Placement::Own;
        }
    }

    /// The failure text of `u`: its `Failed` state's, or the one it lands with
    /// (a start limit hit: what `packages.list` shows).
    pub fn failure_of(&self, u: UnitId) -> Option<String> {
        let unit = self.units.get(&u)?;
        unit.failure
            .clone()
            .or_else(|| unit.failing.as_ref().map(|(_, text)| text.clone()))
    }

    /// `process_failed` (`runtime.failed`, or a failed initial UI registration,
    /// as the activation says; no retry).
    pub fn process_failed(&mut self, pid: Pid, text: String) {
        let Some(u) = self.current(pid) else {
            return;
        };
        self.crash(u, ExitCause::Refused, Some(text));
        self.sync(u);
    }

    /// `ui_begin`: an initial UI registration of an activating process (ready
    /// once its UI is).
    pub fn ui_begin(&mut self, pid: Pid) {
        let Some(u) = self.current(pid) else {
            return;
        };
        if let Some(unit) = self.units.get_mut(&u)
            && matches!(unit.state, UnitState::Activating { .. })
        {
            unit.pending_ui += 1;
        }
    }

    /// `ui_finish` (a failed one is reported with `process_failed`).
    pub fn ui_finish(&mut self, pid: Pid) {
        let Some(u) = self.current(pid) else {
            return;
        };
        if let Some(unit) = self.units.get_mut(&u) {
            unit.pending_ui = unit.pending_ui.saturating_sub(1);
        }
        self.try_ready(u);
        self.sync(u);
    }

    /// `process_exited` (expected exits of a deactivating process are ignored:
    /// its reap follows). `text`: the fault's error text (today `RuntimeFault`
    /// writes it into `failures`, shown by `packages.list`); `None`: the
    /// cause's own text. The class's the restart rule says where the unit lands.
    pub fn process_exited(&mut self, pid: Pid, cause: ExitCause, text: Option<String>) {
        let Some(u) = self.current(pid) else {
            return;
        };
        self.crash(u, cause, text);
        self.sync(u);
    }

    /// Whether run `pid`'s Standby job asked for its resources
    /// (`QueryResources`) and still waits for `resources_answered`: the
    /// adapter's standby recording is still wanted. A job that replaced it
    /// (a reload's restart, a stop) or the answer ends that.
    pub fn standby_asking(&self, pid: Pid) -> bool {
        self.current(pid)
            .and_then(|u| self.units.get(&u))
            .is_some_and(|unit| {
                matches!(unit.state, UnitState::Active { .. })
                    && unit.asked_at.is_some()
                    && standby_running(unit)
            })
    }

    /// `resources_answered` (`owner.resources` for a Standby job).
    pub fn resources_answered(&mut self, pid: Pid, busy: bool) {
        let Some(u) = self.current(pid) else {
            return;
        };
        let now = self.now;
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        let UnitState::Active { last_active, .. } = unit.state else {
            return;
        };
        let Some(asked) = unit.asked_at.filter(|_| standby_running(unit)) else {
            return;
        };

        unit.asked_at = None;
        if busy || unit.work.busy() {
            // A first run found busy waits the idle timeout from now on, as
            // one busy at its classification does: not asked again every
            // quiet period while it stays busy.
            unit.quiet_park = false;
        }
        if busy || unit.work.busy() || last_active >= asked {
            // Asked again after another idle period.
            if let UnitState::Active { last_active, .. } = &mut unit.state {
                *last_active = now;
            }
            self.cancel_standby(u);
        } else {
            let resident = unit.resident;
            unit.recorded = true;
            unit.recorded_resident = resident;
            self.pending.push(Action::Checkpoint {
                unit: u,
                pid,
                resident,
            });
            self.terminate(u, StopReason::Idle, Then::Standby, true);
        }
        self.sync(u);
    }

    /// `classified` (formerly `lazy_classified`). `resident`: why it holds a live
    /// resource. A run without a checkpoint is classified (the 5 s timer runs);
    /// any other ready run of a recordable unit may be, once per run, as
    /// today's `lazy_ready` classifies on every ready (supervisor.md). A lazy
    /// first run goes to standby after `first_run_quiet` of idle time (at
    /// once with that 0) unless it is busy: a job, or work in flight (the
    /// Host's own idle check is declared work, `work.rs`; today's
    /// `lazy_classified` parks only when `lazy_idle` says idle and its
    /// recording was written); busy, it waits its idle timeout.
    pub fn classified(&mut self, pid: Pid, resident: Option<String>) {
        let Some(u) = self.current(pid) else {
            return;
        };
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        let open = unit.classifying || unit.may_reclassify;
        if !open || !matches!(unit.state, UnitState::Active { .. }) {
            return;
        }
        unit.classifying = false;
        unit.may_reclassify = false;
        unit.classified_run = true;
        Timers::cancel(&mut unit.gens, TimerKind::Classify);
        if resident.is_some() {
            unit.resident = true;
            if unit.class == Class::Normal {
                unit.class = Class::Resident;
            }
            Timers::cancel(&mut unit.gens, TimerKind::Idle);
            // Written only when the checkpoint does not say so already.
            if !(unit.recorded && unit.recorded_resident) {
                unit.recorded = true;
                unit.recorded_resident = true;
                self.pending.push(Action::Checkpoint {
                    unit: u,
                    pid,
                    resident: true,
                });
            }
            // A resident run never stands by when idle: no idle timer.
        } else {
            unit.resident = false;
            if unit.class == Class::Resident {
                unit.class = Class::Normal;
            }
            let idle_standby = self.tunables.policy(unit.class).idle_standby;
            let at_once = unit.run_is_first && idle_standby;
            let busy = unit.job.is_some() || unit.work.busy();
            // A lazy class is written only when it goes to standby, or at quit
            // (today's `lazy_classified` / `park` / `lazy_quit`).
            let Some(unit) = self.units.get_mut(&u) else {
                return;
            };
            // Only a first run goes to standby soon (today's
            // `lazy_classified`); any other run waits for its idle timeout.
            if !at_once {
                self.arm_idle(u);
            } else if busy {
                // It waits its idle timeout.
                self.arm_idle(u);
            } else if self.tunables.first_run_quiet > 0 {
                // After a short quiet period, counted from the end of its
                // classification (work: supervisor.md §4), through the idle
                // timer's Standby job: what the run takes up meanwhile (a
                // saved effect restored after a timer or a round trip)
                // reaches the playback worker while it runs, is asked about
                // again and makes it resident (`holds_live`); any use or work
                // starts the period again. Not parked at once: a request that
                // came while it stopped would be dropped, and its lazy
                // recording would not restore that effect at the next start.
                unit.quiet_park = true;
                let now = self.now;
                if let UnitState::Active { last_active, .. } = &mut unit.state {
                    *last_active = (*last_active).max(now);
                }
                self.arm_idle(u);
            } else {
                unit.recorded = true;
                unit.recorded_resident = false;
                self.pending.push(Action::Checkpoint {
                    unit: u,
                    pid,
                    resident: false,
                });
                let job = self.new_job(u, JobKind::Standby, Origin::Internal);
                if let Some(unit) = self.units.get_mut(&u) {
                    unit.job = Some(Job {
                        state: JobState::Running,
                        ..job
                    });
                }
                self.terminate(u, StopReason::Idle, Then::Standby, true);
            }
        }
        self.sync(u);
    }

    /// Run `pid` may still go to standby: active, recordable and not
    /// resident. What it holds in the playback worker is worth asking again
    /// after its requests there (`holds_live`).
    pub fn may_stand_by(&self, pid: Pid) -> bool {
        self.current(pid)
            .and_then(|u| self.units.get(&u))
            .is_some_and(|unit| {
                matches!(unit.state, UnitState::Active { .. }) && unit.recordable && !unit.resident
            })
    }

    /// Run `pid`, classified lazy, holds a live resource it acquired after
    /// its classification (a shader list, a filter chain, a stream: the
    /// playback worker's answer to a later ask). It is resident from now on,
    /// as if classified so at its ready: no idle standby, and its checkpoint
    /// says resident, so the next start launches it (and it takes up what it
    /// holds again) instead of replaying its entry points. As a resident
    /// classification at ready, the next ready classifies it again: one that
    /// let go by then goes back to standby (supervisor.md §4). `false`:
    /// nothing changed (not active, its class still asked, resident already).
    pub fn holds_live(&mut self, pid: Pid) -> bool {
        let Some(u) = self.current(pid) else {
            return false;
        };
        let Some(unit) = self.units.get_mut(&u) else {
            return false;
        };
        if !matches!(unit.state, UnitState::Active { .. })
            || !unit.recordable
            || unit.resident
            || unit.classifying
            || unit.may_reclassify
        {
            return false;
        }
        unit.resident = true;
        unit.quiet_park = false;
        if unit.class == Class::Normal {
            unit.class = Class::Resident;
        }
        Timers::cancel(&mut unit.gens, TimerKind::Idle);
        if !(unit.recorded && unit.recorded_resident) {
            unit.recorded = true;
            unit.recorded_resident = true;
            self.pending.push(Action::Checkpoint {
                unit: u,
                pid,
                resident: true,
            });
        }
        self.sync(u);
        true
    }

    /// `touch`: the unit was used (formerly `Lazy::touch`).
    pub fn touch(&mut self, pid: Pid, now: Ms) {
        self.set_now(now);
        let Some(u) = self.current(pid) else {
            return;
        };
        if let Some(unit) = self.units.get_mut(&u)
            && let UnitState::Active { last_active, .. } = &mut unit.state
        {
            *last_active = (*last_active).max(now);
        }
    }

    /// `begin_work`: work in flight keeps the unit out of standby. A
    /// host→plugin request awaiting a reply (`WorkKind::Request`) is capped by
    /// the class's `request_timeout`; declared work is not.
    pub fn begin_work(&mut self, pid: Pid, kind: WorkKind, now: Ms) {
        self.touch(pid, now);
        let Some(u) = self.current(pid) else {
            return;
        };
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        let first = unit.work.oldest_request().is_none();
        unit.work.begin(kind, now);
        if first
            && kind.capped()
            && let Some(timeout) = self.tunables.policy(unit.class).request_timeout
        {
            self.timers
                .arm(&mut unit.gens, u, TimerKind::Request, now + timeout);
        }
    }

    /// `end_work`: the unit was used (its idle time counts from now).
    pub fn end_work(&mut self, pid: Pid, kind: WorkKind) {
        let Some(u) = self.current(pid) else {
            return;
        };
        let now = self.now;
        if let Some(unit) = self.units.get_mut(&u) {
            unit.work.end(kind);
            if let UnitState::Active { last_active, .. } = &mut unit.state {
                *last_active = (*last_active).max(now);
            }
        }
    }

    /// `runner_failed`: the runner group stops at once; every member gets a
    /// Restart job (the first at once, then after `runner_backoff`); fail-open
    /// after `runner_restart_limit` in the window.
    pub fn runner_failed(&mut self, runner: RunnerId, _cause: ExitCause) {
        if runner != self.runner || self.shutting_down {
            return;
        }
        let now = self.now;
        let failed_open = self.runner_policy.restart(
            now,
            self.tunables.runner_restart_limit,
            self.tunables.runner_restart_window,
        );
        let delay = self
            .runner_restart
            .next_delay(now, Some(self.tunables.runner_backoff));
        self.runner = RunnerId(runner.0 + 1);
        self.pending.push(Action::KillRunner { runner });
        let members = self
            .units
            .iter()
            .filter(|(_, unit)| {
                unit.placement == Placement::Shared(runner)
                    && (unit.pid().is_some() || unit.stopping.is_some())
            })
            .map(|(u, _)| *u)
            .collect::<Vec<_>>();
        let stop_wait = self.tunables.stop_wait;
        // The members start again only once the playback worker cleaned up
        // after them.
        for u in members {
            self.drop_alias(u);
            let Some(unit) = self.units.get_mut(&u) else {
                continue;
            };
            if let Some(stopping) = unit.stopping.as_mut() {
                stopping.reaped = true;
                if let UnitState::Deactivating { sub, .. } = &mut unit.state {
                    *sub = DeactivatingSub::Cleanup;
                }
                Timers::cancel(&mut unit.gens, TimerKind::StopGrace);
                // Cleaned up first (a shared member: `owner.parked` at once,
                // reap at `MEMBER_GONE`): it is gone now, landed or tracked
                // after a stuck landing.
                self.flush_data_removal(u);
                self.land_if_done(u);
                self.sync(u);
                continue;
            }
            let Some(pid) = unit.pid() else {
                continue;
            };
            Self::cancel_run_timers(unit);
            unit.state = UnitState::Deactivating {
                pid,
                reason: StopReason::Failed,
                then: Then::Inactive,
                sub: DeactivatingSub::Cleanup,
            };
            unit.stopping = Some(Stopping {
                pid,
                reaped: true,
                backend_cleaned: false,
                deadline: now + stop_wait,
                kept: false,
            });
            if !failed_open && unit.verifiable {
                unit.verify = true;
            }
            // Re-placement: a member with a standby checkpoint and no start in
            // flight goes back to standby by replay (no spawn), as today's
            // `start_installed` replays a recording.
            let replay = unit.recorded
                && !unit.recorded_resident
                && self.tunables.policy(unit.class).idle_standby
                && !unit.job.as_ref().is_some_and(Job::will_start);
            unit.replay_after = replay;
            self.timers
                .arm(&mut unit.gens, u, TimerKind::StopWait, now + stop_wait);
            self.pending.push(Action::Terminate {
                unit: u,
                pid,
                reason: StopReason::Failed,
                grace: 0,
                keep_registrations: false,
            });
            if replay {
                self.land_if_done(u);
                self.sync(u);
                continue;
            }
            let _ = self.enqueue_job(
                u,
                JobKind::Restart {
                    fresh: false,
                    image: None,
                },
                JobMode::Replace,
                Priority::Background,
                Origin::Crash,
            );
            if let Some(job) = self.units.get_mut(&u).and_then(|unit| unit.job.as_mut())
                && delay > 0
            {
                job.not_before = Some(now + delay);
            }
            self.land_if_done(u);
            self.sync(u);
        }
        let notice = if failed_open {
            Notice::RunnerFailedOpen
        } else {
            Notice::RunnerRestarted
        };
        self.pending.push(Action::Notify { unit: None, notice });
    }

    /// `memory_pressure` (supervisor.md §8): Low/Critical put every idle reclaimable unit in
    /// standby through the same Standby job, most important last; Critical also
    /// takes the Interactive overrun away (`admit`). The adapter calls it when
    /// the level changes.
    pub fn memory_pressure(&mut self, level: Pressure) {
        self.pressure = level;
        if level == Pressure::None {
            return;
        }
        let candidates = pressure::candidates(
            self.procs_third.iter().map(|u| (*u, &self.units[u])),
            &self.tunables,
            self.now,
        );
        for u in candidates {
            let _ = self.enqueue_job(
                u,
                JobKind::Standby,
                JobMode::Replace,
                Priority::Background,
                Origin::Pressure,
            );
        }
    }

    /// How many built-ins the adapter has still to load at start-up (its
    /// start order keeps a built-in whose digest is checked in place, as
    /// today's `StartQueue`): third-party `Background` starts and first
    /// runs wait for them as for a built-in with a start installed.
    pub fn set_builtins_to_load(&mut self, count: usize) {
        self.builtins_to_load = count;
    }

    /// Wake events wake nothing (start-up; `Lazy::mute`).
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }
    /// Start-up settled at `now`: wake events wake again after
    /// `quiet_after_start`.
    pub fn settled(&mut self, now: Ms) {
        self.set_now(now);
        self.muted = false;
        self.quiet_until = now + self.tunables.quiet_after_start;
    }

    // ---- queries -------------------------------------------------------

    pub fn unit(&self, u: UnitId) -> Option<&Unit<P>> {
        self.units.get(&u)
    }
    pub fn by_pid(&self, pid: Pid) -> Option<UnitId> {
        self.by_pid.get(&pid).copied()
    }
    pub fn by_id(&self, id: &str) -> Option<UnitId> {
        self.by_id.get(id).copied()
    }
    /// `packages.list`'s state of `u` now.
    pub fn list_state(&self, u: UnitId) -> Option<ListState> {
        self.units.get(&u).map(|unit| list_state(unit, self.now))
    }
    /// Everything `packages.list` reads of the unit table now
    /// (`list_state`, `holder_of`, `waking`, `package_busy`), to compare
    /// with an earlier one: the same key, the same answers. The clock is in
    /// it through each run's activation (a start deadline passing turns
    /// `Starting` into `Failed`). O(units), no I/O.
    pub fn list_key(&self) -> ListKey {
        ListKey(
            self.units
                .iter()
                .map(|(u, unit)| {
                    let holder = match unit.state {
                        UnitState::Standby { holder, .. } => Some(holder),
                        UnitState::Deactivating {
                            pid,
                            then: Then::Standby,
                            ..
                        } => Some(pid),
                        _ => None,
                    };
                    let waking = matches!(
                        unit.state,
                        UnitState::Activating { pid: Some(pid), .. }
                            if unit.from_standby && self.current(pid) == Some(*u)
                    );
                    let package_busy = unit
                        .job
                        .as_ref()
                        .is_some_and(|job| job.origin == Origin::PackageRpc);
                    (*u, list_state(unit, self.now), holder, waking, package_busy)
                })
                .collect(),
        )
    }
    #[cfg(test)]
    pub fn units(&self) -> impl Iterator<Item = (UnitId, &Unit<P>)> {
        self.units.iter().map(|(u, unit)| (*u, unit))
    }
    /// Third-party processes counted in the budget.
    pub fn budget_count(&self) -> usize {
        self.procs_third.len()
    }
    /// The memory pressure level last given (`memory_pressure`).
    pub fn pressure(&self) -> Pressure {
        self.pressure
    }
    /// Third-party start slots in use.
    #[cfg(test)]
    pub fn slots_in_use(&self) -> usize {
        self.slots.len()
    }

    // ---- internals -----------------------------------------------------

    fn new_job(&mut self, u: UnitId, kind: JobKind, origin: Origin) -> Job {
        let id = JobId(self.next_job);
        self.next_job += 1;
        Job {
            id,
            unit: u,
            kind,
            mode: JobMode::Replace,
            priority: Priority::Background,
            origin,
            state: JobState::Waiting,
            deadline: None,
            not_before: None,
            created: self.now,
            restarting: false,
            reason: StopReason::Idle,
            queued: false,
        }
    }

    /// The unit whose *current* process is `pid`.
    fn current(&self, pid: Pid) -> Option<UnitId> {
        let u = *self.by_pid.get(&pid)?;
        (self.units.get(&u)?.pid() == Some(pid)).then_some(u)
    }

    /// Wake events wake nothing now: start-up has not settled
    /// (`set_muted`), or it settled less than `quiet_after_start` ago
    /// (`settled`). The adapter wakes what a muted event would have woken
    /// once this ends (the Host's `wake_missed`).
    pub fn events_muted(&self) -> bool {
        self.muted || self.now < self.quiet_until
    }

    fn answer(&mut self, u: UnitId, reason: AnswerReason, trigger: Trigger<P>) {
        if trigger.answered() {
            self.pending.push(Action::Answer {
                unit: u,
                reason,
                payload: trigger.payload,
            });
        }
    }

    /// Every held trigger is answered; events and follows go silently.
    pub(crate) fn answer_backlog(&mut self, u: UnitId, reason: AnswerReason) {
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        let backlog = std::mem::take(&mut unit.backlog);
        for trigger in backlog {
            self.answer(u, reason, trigger);
        }
    }

    /// A start from a recording (and a runner member's re-placement): the
    /// checkpoint's registrations are replayed under a new holder pid; the
    /// unit is in standby.
    fn replay(&mut self, u: UnitId) {
        let holder = self.alloc_pid();
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        // Earlier pids are dead (a runner member replayed after its
        // process went): messages to them are dropped, as today. A pid a
        // stop still tracks stays mapped, so its reap and cleanup reach the
        // unit.
        let old = [unit.alias.take(), unit.last_pid.take()];
        unit.last_pid = Some(holder);
        unit.state = UnitState::Standby {
            holder,
            recorded: true,
            hold_until: None,
        };
        self.by_pid.insert(holder, u);
        for old in old.into_iter().flatten() {
            self.unmap_pid(u, old);
        }
        self.pending.push(Action::Replay { unit: u, holder });
    }

    /// Answers the interactive triggers (follows go with them); wake events
    /// stay held.
    fn answer_interactive(&mut self, u: UnitId, reason: AnswerReason) {
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        let backlog = std::mem::take(&mut unit.backlog);
        let (events, others): (VecDeque<_>, VecDeque<_>) = backlog
            .into_iter()
            .partition(|t| matches!(t.kind, TriggerKind::Event(_)));
        unit.backlog = events;
        for trigger in others {
            self.answer(u, reason, trigger);
        }
    }

    fn cancel_run_timers(unit: &mut Unit<P>) {
        for kind in [
            TimerKind::Idle,
            TimerKind::StartTimeout,
            TimerKind::Notice,
            TimerKind::Classify,
            TimerKind::Request,
            TimerKind::Verify,
        ] {
            Timers::cancel(&mut unit.gens, kind);
        }
        unit.classifying = false;
        unit.classified_run = false;
        unit.may_reclassify = false;
        unit.quiet_park = false;
        unit.asked_at = None;
        unit.runtime_ready = false;
        unit.pending_ui = 0;
        unit.work.clear();
    }

    /// The previous pid that forwarded to the run now ending goes (today's
    /// `forget_running` drops `lazy.moved` entries to a stopped owner):
    /// later messages to it are dropped.
    fn drop_alias(&mut self, u: UnitId) {
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        if let Some(alias) = unit.alias.take()
            && unit.pid() != Some(alias)
            && self.by_pid.get(&alias) == Some(&u)
        {
            self.by_pid.remove(&alias);
        }
    }

    fn arm_idle(&mut self, u: UnitId) {
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        let UnitState::Active { last_active, .. } = unit.state else {
            return;
        };
        // A unit classified resident never stands by when idle.
        let idles = !unit.resident && self.tunables.policy(unit.class).idle_standby;
        if !idles || !unit.recordable || unit.classifying {
            return;
        }
        let due = last_active + idle_time(&self.tunables, unit);
        self.timers.arm(&mut unit.gens, u, TimerKind::Idle, due);
    }

    /// Stops the unit's process (...).
    fn terminate(&mut self, u: UnitId, reason: StopReason, then: Then, keep: bool) {
        let Some(unit) = self.units.get(&u) else {
            return;
        };
        let grace = kill::grace(self.tunables.policy(unit.class), reason, then);
        self.terminate_with(u, reason, then, keep, grace);
    }

    fn terminate_with(&mut self, u: UnitId, reason: StopReason, then: Then, keep: bool, grace: Ms) {
        self.deactivate(u, reason, then, keep, Some(grace));
    }

    /// The unit leaves its run for `then`: with `grace`, its process gets a
    /// `Terminate` (then `Kill` after the grace); without, the process is gone
    /// already (an exit or a heap abort; coordinator ruling 2026-11-03) and the
    /// adapter cleans up after it on the exit itself, without a stop message.
    /// Either way the unit waits for the reap and the backend cleanup.
    fn deactivate(
        &mut self,
        u: UnitId,
        reason: StopReason,
        then: Then,
        keep: bool,
        grace: Option<Ms>,
    ) {
        self.drop_alias(u);
        let now = self.now;
        let stop_wait = self.tunables.stop_wait;
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        let Some(pid) = unit.pid() else {
            return;
        };
        if unit.state.is_deactivating() {
            return;
        }
        Self::cancel_run_timers(unit);
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
            kept: keep,
        });
        if then == Then::Removed {
            unit.remove_data = true;
        }
        self.timers
            .arm(&mut unit.gens, u, TimerKind::StopWait, now + stop_wait);
        let Some(grace) = grace else {
            // Gone already: its entries go now unless kept for standby, as a
            // `Terminate`'s cleanup would revoke them.
            if !keep {
                self.pending.push(Action::Revoke { unit: u, pid });
            }
            return;
        };
        self.timers
            .arm(&mut unit.gens, u, TimerKind::StopGrace, now + grace);
        self.pending.push(Action::Terminate {
            unit: u,
            pid,
            reason,
            grace,
            keep_registrations: keep,
        });
    }

    /// Advances the installed job of `u`.
    fn step(&mut self, u: UnitId) {
        let Some(job) = self.units.get(&u).and_then(|unit| unit.job.as_ref()) else {
            return;
        };
        match job.kind.clone() {
            JobKind::Start { .. } => self.step_start(u),
            JobKind::Restart { fresh, image } => {
                if job.restarting {
                    self.step_start(u);
                } else {
                    self.step_restart(u, fresh, image);
                }
            }
            JobKind::Stop { reason, then } => self.step_stop(u, reason, then),
            JobKind::Standby => self.step_standby(u),
        }
        self.sync(u);
    }

    fn step_start(&mut self, u: UnitId) {
        let now = self.now;
        let stop_wait = self.tunables.stop_wait;
        // A tracked process that is reaped and cleaned up after is gone,
        // whatever path set the flags.
        if self.units.get(&u).is_some_and(|unit| {
            !unit.state.is_deactivating() && unit.stopping.is_some_and(|s| s.done())
        }) {
            self.land_if_done(u);
        }
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        let Some(job) = unit.job.as_ref() else {
            return;
        };
        if job.state == JobState::Running || job.queued {
            return;
        }
        match unit.state {
            UnitState::Deactivating { .. } => return,
            UnitState::Active { .. } => {
                self.finish(u, JobResult::Done);
                return;
            }
            UnitState::Activating {
                sub: ActivatingSub::AutoRestart { until },
                ..
            } if until > now => {
                self.timers
                    .arm(&mut unit.gens, u, TimerKind::NotBefore, until);
                return;
            }
            UnitState::Activating {
                sub: ActivatingSub::AutoRestart { .. },
                ..
            } => {}
            UnitState::Activating { .. } => return,
            _ => {}
        }
        if !unit.enabled() || unit.removed {
            self.answer_backlog(u, AnswerReason::Disabled);
            self.finish(u, JobResult::Dropped);
            return;
        }
        // The previous stop landed stuck and its process may still run: never a
        // second instance on the same data. The start waits `STOP_WAIT` from
        // its own request for that process to go, then fails (today's `lazy.rs`
        // wake / `packages.reloads`).
        if unit.stopping.is_some() {
            let due = job.created + stop_wait;
            if now < due {
                self.timers.arm(&mut unit.gens, u, TimerKind::StopWait, due);
            } else {
                self.fail_stuck(u);
            }
            return;
        }
        let hold = match unit.state {
            UnitState::Standby {
                hold_until: Some(until),
                ..
            } => Some(until),
            _ => None,
        };
        let not_before = job.not_before.max(hold).filter(|t| *t > now);
        if let Some(until) = not_before {
            if matches!(unit.state, UnitState::Inactive) {
                unit.state = UnitState::Activating {
                    pid: None,
                    since: now,
                    sub: ActivatingSub::AutoRestart { until },
                };
            }
            self.timers
                .arm(&mut unit.gens, u, TimerKind::NotBefore, until);
            return;
        }
        if unit.class == Class::Development {
            self.begin_start(u);
            return;
        }
        let lane = Lane::of(unit.class, job.cause(), job.priority);
        let id = job.id;
        if let Some(job) = unit.job.as_mut() {
            job.queued = true;
        }
        self.lanes.push(lane, u, id);
    }

    /// The job leaves its lane (or skips it) and starts the unit.
    fn begin_start(&mut self, u: UnitId) {
        let now = self.now;
        let verify_wait = self.tunables.verify_wait;
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        if let Some(job) = unit.job.as_mut() {
            job.queued = false;
            job.state = JobState::Running;
        }
        if matches!(unit.state, UnitState::Failed { .. }) {
            unit.state = UnitState::Inactive;
            unit.failure = None;
            unit.stuck = false;
            unit.restart.crashes.clear();
        }
        if let UnitState::Activating {
            sub: ActivatingSub::AutoRestart { .. },
            ..
        } = unit.state
        {
            unit.state = UnitState::Inactive;
        }
        if unit.verify {
            if let UnitState::Standby { holder, .. } = unit.state {
                unit.holder = Some(holder);
            }
            unit.state = UnitState::Activating {
                pid: None,
                since: now,
                sub: ActivatingSub::Verify,
            };
            self.timers
                .arm(&mut unit.gens, u, TimerKind::Verify, now + verify_wait);
            self.pending.push(Action::Verify { unit: u });
        } else {
            self.spawn(u);
        }
    }

    /// Spawn: a new pid, registrations rekeyed from the holder, `Spawn`.
    fn spawn(&mut self, u: UnitId) {
        let now = self.now;
        let pid = self.alloc_pid();
        let runner = self.runner;
        let failed_open = self.runner_policy.failed_open;
        let starting_notice = self.tunables.starting_notice;
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        let holder = match unit.state {
            UnitState::Standby { holder, .. } => Some(holder),
            _ => unit.holder.take(),
        };
        unit.holder = None;
        for old in [unit.alias.take(), unit.last_pid] {
            if let Some(old) = old
                && Some(old) != holder
                && self.by_pid.get(&old) == Some(&u)
            {
                self.by_pid.remove(&old);
            }
        }
        unit.alias = holder;
        unit.last_pid = Some(pid);
        self.by_pid.insert(pid, u);
        let placement = if unit.shared && !failed_open && !unit.quarantined && !unit.own_this_run {
            Placement::Shared(runner)
        } else {
            Placement::Own
        };
        unit.own_this_run = false;
        unit.placement = placement;
        unit.runtime_ready = false;
        unit.pending_ui = 0;
        unit.classified_run = false;
        unit.may_reclassify = false;
        unit.quiet_park = false;
        unit.run_is_first = unit
            .job
            .as_ref()
            .is_some_and(|job| job.cause() == Some(Cause::FirstRun));
        unit.from_standby = holder.is_some();
        unit.state = UnitState::Activating {
            pid: Some(pid),
            since: now,
            sub: ActivatingSub::Spawn,
        };
        let timeout = unit
            .start_timeout
            .unwrap_or(self.tunables.policy(unit.class).start_timeout);
        // The one start deadline: the adapter gives the activation the same
        // one (`Job::deadline`).
        self.timers
            .arm(&mut unit.gens, u, TimerKind::StartTimeout, now + timeout);
        let mut priority = Priority::Background;
        if let Some(job) = unit.job.as_mut() {
            job.deadline = Some(now + timeout);
            priority = job.priority;
            if matches!(job.cause(), Some(Cause::Trigger(_))) {
                // 400 ms after the spawn, as today's `lazy.waking.since`, not
                // after the request: a start that waited for an old process
                // still gets its 400 ms.
                self.timers
                    .arm(&mut unit.gens, u, TimerKind::Notice, now + starting_notice);
            }
        }
        if let Some(from) = holder {
            self.pending.push(Action::Rekey {
                unit: u,
                from,
                to: pid,
            });
        }
        self.pending.push(Action::Spawn {
            unit: u,
            pid,
            placement,
            image: unit.image.clone(),
            first_run: unit.recordable && !unit.recorded,
        });
        self.sync(u);
        self.enforce_budget(u, priority);
    }

    /// Spawn failed: the process could not be created.
    fn spawn_failed(&mut self, u: UnitId, pid: Pid, text: String) {
        let now = self.now;
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        let policy = self.tunables.policy(unit.class).clone();
        Self::cancel_run_timers(unit);
        if let Some(retry) = policy.start_retry
            && unit.restart.retries < retry.count
        {
            let delay = retry_delay(retry, unit.restart.retries, &mut self.rng);
            unit.restart.retries += 1;
            unit.holder = Some(pid);
            unit.state = UnitState::Activating {
                pid: None,
                since: now,
                sub: ActivatingSub::AutoRestart { until: now + delay },
            };
            if let Some(job) = unit.job.as_mut() {
                job.state = JobState::Waiting;
                job.not_before = Some(now + delay);
            }
            self.run.push_back(u);
            return;
        }
        unit.restart.retries = 0;
        self.drop_alias(u);
        self.answer_backlog(u, AnswerReason::CouldNotStart);
        self.finish(u, JobResult::Failed(text.clone()));
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        // A failed start keeps its entry points in standby.
        if unit.recorded && !unit.resident && policy.idle_standby {
            let hold_until = unit.restart.next_delay(now, policy.restart_sec);
            unit.state = UnitState::Standby {
                holder: pid,
                recorded: true,
                hold_until: (hold_until > 0).then_some(now + hold_until),
            };
        } else {
            unit.failure = Some(text.clone());
            unit.state = UnitState::Failed {
                result: FailResult::Spawn,
                text,
            };
            self.pending.push(Action::Revoke { unit: u, pid });
        }
        self.pending.push(Action::Notify {
            unit: Some(u),
            notice: Notice::CouldNotStart,
        });
    }

    /// Ready: `runtime.ready` and every initial UI registration done.
    fn try_ready(&mut self, u: UnitId) {
        let now = self.now;
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        let UnitState::Activating {
            pid: Some(pid),
            sub: ActivatingSub::Ready,
            ..
        } = unit.state
        else {
            return;
        };
        if !unit.runtime_ready || unit.pending_ui > 0 {
            return;
        }
        unit.state = UnitState::Active {
            pid,
            since: now,
            last_active: now,
        };
        Timers::cancel(&mut unit.gens, TimerKind::StartTimeout);
        Timers::cancel(&mut unit.gens, TimerKind::Notice);
        unit.restart.retries = 0;
        unit.restart.active_since = Some(now);
        if unit.from_standby {
            self.pending.push(Action::Prune { unit: u, pid });
        }
        let backlog = std::mem::take(&mut unit.backlog);
        for trigger in backlog {
            self.pending.push(Action::Deliver {
                unit: u,
                pid,
                payload: trigger.payload,
            });
        }
        if unit.job.as_ref().is_some_and(Job::starts) {
            self.finish(u, JobResult::Done);
        }
        self.arm_classification(u);
        self.arm_idle(u);
    }

    /// At a run's ready: a run without a checkpoint is classified (the 5 s
    /// timer); never a development load or a unit without a recording key
    /// (today's `lazy_development`, `lazy_key_of` is `None`). A recorded run
    /// may be classified again (`classified`, once per run), without the core's
    /// timer (the adapter times its answer).
    fn arm_classification(&mut self, u: UnitId) {
        let now = self.now;
        let classify_wait = self.tunables.classify_wait;
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        unit.classifying = false;
        unit.classified_run = false;
        unit.may_reclassify = false;
        Timers::cancel(&mut unit.gens, TimerKind::Classify);
        if unit.recordable && !unit.recorded {
            unit.classifying = true;
            self.timers
                .arm(&mut unit.gens, u, TimerKind::Classify, now + classify_wait);
        } else if unit.recordable {
            // Classified again once (`classified`). Only that answer makes the
            // run checkpointed at quit: today's `lazy_quit` writes the classes
            // in `lazy.classes`, filled when the answer comes back; until then
            // the checkpoint on disk stays as it is.
            unit.may_reclassify = true;
        }
    }

    /// The process failed or ended unasked, per the class's restart rule.
    fn crash(&mut self, u: UnitId, cause: ExitCause, text: Option<String>) {
        let now = self.now;
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        let activating = match unit.state {
            UnitState::Active { .. } => false,
            UnitState::Activating { pid: Some(_), .. } => true,
            _ => return,
        };
        if let Placement::Shared(runner) = unit.placement {
            if cause == ExitCause::Transport {
                self.runner_failed(runner, cause);
                return;
            }
            if cause == ExitCause::Fault {
                // Attributed fault: that member alone; its own process next.
                unit.quarantined = true;
            }
        }
        // A Standby job does not finish by a crash: canceled (the unit lands
        // where the class's restart rule says).
        if unit
            .job
            .as_ref()
            .is_some_and(|job| job.kind == JobKind::Standby)
        {
            unit.asked_at = None;
            self.finish(u, JobResult::Canceled);
        }
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        let text = text.unwrap_or_else(|| cause.text().to_owned());
        let policy = self.tunables.policy(unit.class).clone();
        let development = unit.class == Class::Development;
        let recorded = unit.recorded;
        // A start that timed out is never tried again within its start job:
        // start retries (`Policy::start_retry`) are for a process that could
        // not be created (`spawn_failed`). With the start timeout 60 s, four
        // tries kept a click waiting about 4 min, and a start that times out is
        // a stuck plugin, not a passing failure (coordinator ruling). The unit follows its class's restart rule below.
        let timed_out = activating && cause == ExitCause::StartTimeout;
        let mut then = Then::Failed;
        let mut keep = false;
        let mut failing = (FailResult::Exit(cause), text.clone());
        let mut auto_restart = None;
        if cause.restartable() && !development {
            match policy.restart {
                RestartMode::No => {}
                RestartMode::OnFailure => {
                    if unit.restart.crash(now, policy.start_limit) {
                        failing = (FailResult::StartLimitHit, text.clone());
                    } else {
                        then = Then::Inactive;
                        let delay = unit.restart.next_delay(now, policy.restart_sec);
                        unit.after_reap = Some(delay);
                        auto_restart = Some(delay);
                    }
                }
                RestartMode::OnNextTrigger if recorded => {
                    if unit.restart.crash(now, policy.start_limit) {
                        failing = (FailResult::StartLimitHit, text.clone());
                    } else {
                        then = Then::Standby;
                        keep = true;
                        let delay = unit.restart.next_delay(now, policy.restart_sec);
                        unit.hold_next = (delay > 0).then_some(delay);
                    }
                }
                RestartMode::OnNextTrigger => {}
            }
        }
        let limit_hit = failing.0 == FailResult::StartLimitHit;
        if limit_hit && let Some(limit) = policy.start_limit {
            // Settings > Plugins says why it stays stopped and what clears it
            // ("reload clears it").
            failing.1 = start_limit_text(&text, limit);
        }
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        if then == Then::Failed {
            unit.failing = Some(failing);
            let starts = unit.job.as_ref().is_some_and(Job::starts);
            self.answer_backlog(u, AnswerReason::CouldNotStart);
            if starts {
                self.finish(u, JobResult::Failed(text.clone()));
            }
        } else if auto_restart.is_some() {
            // A start that timed out restarts by its class's restart rule (a
            // built-in, a Resident) after its backoff; a click held for it does
            // not wait that long: answered "could not start" now (coordinator
            // ruling). Wake events stay for the restart.
            if timed_out {
                self.answer_interactive(u, AnswerReason::CouldNotStart);
            }
        } else {
            // Standby with a hold (ruling): interactive triggers are answered
            // now; the start job stays (waiting for the hold) only while event
            // triggers remain in the backlog — no demand, no start of a
            // crashing plugin.
            if self.units[&u].job.as_ref().is_some_and(Job::starts) {
                self.answer_interactive(u, AnswerReason::CouldNotStart);
                let events_left = !self.units[&u].backlog.is_empty();
                if events_left {
                    if let Some(job) = self.units.get_mut(&u).and_then(|unit| unit.job.as_mut()) {
                        job.state = JobState::Waiting;
                        job.queued = false;
                    }
                } else {
                    self.finish(u, JobResult::Failed(text.clone()));
                }
            }
        }
        // An own process that exited by itself is told at its reap, where its
        // exit code says whether it ran out of memory. A start limit hit says
        // it itself (one notice, not two).
        if cause.notifies() && !limit_hit {
            let exited = self.units[&u].pid().filter(|_| {
                cause == ExitCause::Exited && self.units[&u].placement == Placement::Own
            });
            match exited {
                Some(pid) => {
                    if let Some(unit) = self.units.get_mut(&u) {
                        unit.exit_notice = Some(pid);
                    }
                }
                None => self.pending.push(Action::Notify {
                    unit: Some(u),
                    notice: Notice::Crashed(cause),
                }),
            }
        }
        if limit_hit {
            self.pending.push(Action::Notify {
                unit: Some(u),
                notice: Notice::StartLimitHit,
            });
        }
        let failure_grace = self.tunables.policy(self.units[&u].class).failure_grace;
        // An exit or a heap abort: the process is gone, no stop is sent;
        // anything else is terminated first.
        let gone = matches!(cause, ExitCause::Exited | ExitCause::Oom);
        let grace = (!gone).then_some(failure_grace);
        self.deactivate(u, cause.stop_reason(), then, keep, grace);
        // The delay (`after_reap`) applies once the process is gone.
        if auto_restart.is_some() {
            let has_start = self.units[&u].job.as_ref().is_some_and(Job::will_start);
            if !has_start {
                let job = self.new_job(
                    u,
                    JobKind::Start {
                        cause: Cause::AutoRestart,
                    },
                    Origin::Crash,
                );
                if let Some(unit) = self.units.get_mut(&u)
                    && unit.job.is_none()
                {
                    unit.job = Some(job);
                }
            } else if let Some(job) = self.units.get_mut(&u).and_then(|unit| unit.job.as_mut()) {
                job.state = JobState::Waiting;
                job.queued = false;
                job.not_before = None;
            }
        }
        self.run.push_back(u);
    }

    fn step_stop(&mut self, u: UnitId, reason: StopReason, then: Then) {
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        if let Some(job) = unit.job.as_mut() {
            job.state = JobState::Running;
        }
        let answer = match (then, reason) {
            (Then::Removed, _) | (_, StopReason::Uninstall) => AnswerReason::Removed,
            (_, StopReason::Disable) => AnswerReason::Disabled,
            _ => AnswerReason::Stopped,
        };
        // The user's disable clears a failure (today's `set_enabled(false)`
        // does `failures.remove`).
        if reason == StopReason::Disable {
            unit.failure = None;
            unit.stuck = false;
            if let UnitState::Deactivating {
                then: landing @ Then::Failed,
                ..
            } = &mut unit.state
            {
                *landing = Then::Inactive;
                unit.failing = None;
            }
        }
        match unit.state.clone() {
            UnitState::Deactivating { .. } => self.set_then(u, then),
            UnitState::Active { .. } | UnitState::Activating { pid: Some(_), .. } => {
                // The checkpoint of a unit being uninstalled goes at the
                // request, not at the landing.
                if then == Then::Removed && unit.recorded {
                    unit.recorded = false;
                    self.pending.push(Action::DropCheckpoint { unit: u });
                }
                self.answer_backlog(u, answer);
                self.terminate(u, reason, then, false);
            }
            state => {
                self.answer_backlog(u, answer);
                let Some(unit) = self.units.get_mut(&u) else {
                    return;
                };
                let holder = match state {
                    UnitState::Standby { holder, .. } => Some(holder),
                    _ => unit.holder.take(),
                };
                Timers::cancel(&mut unit.gens, TimerKind::Verify);
                Timers::cancel(&mut unit.gens, TimerKind::NotBefore);
                if let Some(holder) = holder {
                    self.pending.push(Action::Revoke {
                        unit: u,
                        pid: holder,
                    });
                }
                match then {
                    Then::Removed => {
                        if unit.recorded {
                            self.pending.push(Action::DropCheckpoint { unit: u });
                        }
                        unit.recorded = false;
                        unit.removed = true;
                        unit.state = UnitState::Inactive;
                        // The process of a stop that landed stuck may still
                        // write its data: removed at its reap
                        // (`flush_data_removal`, today's `data_removals`; at
                        // once if it was reaped already).
                        unit.remove_data = true;
                    }
                    _ => {
                        if reason == StopReason::Disable
                            || !matches!(state, UnitState::Failed { .. })
                        {
                            unit.state = UnitState::Inactive;
                        }
                    }
                }
                self.flush_data_removal(u);
                self.finish(u, JobResult::Done);
            }
        }
    }

    fn step_restart(&mut self, u: UnitId, fresh: bool, image: Option<String>) {
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        let reason = unit.job.as_ref().map_or(StopReason::Reload, |j| j.reason);
        if let Some(job) = unit.job.as_mut() {
            job.state = JobState::Running;
        }
        match unit.state.clone() {
            UnitState::Deactivating { .. } => return,
            UnitState::Active { .. } | UnitState::Activating { pid: Some(_), .. } => {
                self.terminate(u, reason, Then::Inactive, false);
                return;
            }
            UnitState::Standby { holder, .. } => {
                self.pending.push(Action::Revoke {
                    unit: u,
                    pid: holder,
                });
            }
            UnitState::Activating { pid: None, .. } => {
                if let Some(holder) = unit.holder.take() {
                    self.pending.push(Action::Revoke {
                        unit: u,
                        pid: holder,
                    });
                }
            }
            UnitState::Inactive | UnitState::Failed { .. } => {}
        }
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        Timers::cancel(&mut unit.gens, TimerKind::Verify);
        unit.state = UnitState::Inactive;
        unit.failure = None;
        unit.stuck = false;
        unit.restart.crashes.clear();
        if fresh {
            if unit.recorded {
                self.pending.push(Action::DropCheckpoint { unit: u });
            }
            unit.recorded = false;
            unit.resident = false;
            unit.verify = unit.verifiable;
        }
        if image.is_some() {
            unit.image = image;
        }
        let enabled = unit.enabled();
        if let Some(job) = unit.job.as_mut() {
            job.restarting = true;
            job.state = JobState::Waiting;
        }
        // The triggers held for the old run carry over to the new one.
        if !enabled {
            self.finish(u, JobResult::Done);
            return;
        }
        self.step_start(u);
    }

    fn step_standby(&mut self, u: UnitId) {
        let now = self.now;
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        let waiting = unit
            .job
            .as_ref()
            .is_some_and(|j| j.state == JobState::Waiting);
        match unit.state {
            UnitState::Active { pid, .. } if waiting => {
                if unit.work.busy() || unit.classifying || !unit.backlog.is_empty() {
                    self.cancel_standby(u);
                    return;
                }
                if let Some(job) = unit.job.as_mut() {
                    job.state = JobState::Running;
                }
                unit.asked_at = Some(now);
                self.pending.push(Action::QueryResources { unit: u, pid });
            }
            UnitState::Active { .. } | UnitState::Deactivating { .. } => {}
            UnitState::Standby { .. } => self.finish(u, JobResult::Done),
            _ => self.finish(u, JobResult::Dropped),
        }
    }

    /// Starts runnable heads of the lanes while slots are free. Whether
    /// anything started.
    fn start_from_lanes(&mut self) -> bool {
        let mut started = false;
        for lane in LANES {
            while let Some((u, id)) = self.lanes.front(lane) {
                if !self.valid_entry(u, id, lane) {
                    self.lanes.pop(lane);
                    continue;
                }
                if !self.admit(lane) {
                    break;
                }
                self.lanes.pop(lane);
                self.take_slot(u, lane);
                self.begin_start(u);
                self.sync(u);
                started = true;
            }
        }
        started
    }

    /// `u` leaves `lane` for a start: a third-party start takes a slot.
    fn take_slot(&mut self, u: UnitId, lane: Lane) {
        if !lane.builtin() {
            self.slots.insert(u);
        }
    }

    fn valid_entry(&self, u: UnitId, id: JobId, lane: Lane) -> bool {
        let Some(unit) = self.units.get(&u) else {
            return false;
        };
        unit.job.as_ref().is_some_and(|job| {
            job.id == id && job.queued && Lane::of(unit.class, job.cause(), job.priority) == lane
        })
    }

    fn admit(&self, lane: Lane) -> bool {
        if lane.builtin() {
            return true;
        }
        let mut limit = self.tunables.max_active;
        if lane == Lane::Interactive {
            limit += self.tunables.interactive_reserve;
            if self.pressure != Pressure::Critical {
                limit += self.tunables.interactive_overrun;
            }
        }
        if self.slots.len() >= limit {
            return false;
        }
        // Built-ins first: third-party `Background` starts (first runs
        // included) wait while any built-in has a start job installed, until it
        // is active, or is verifying.
        let waits = matches!(lane, Lane::Background | Lane::FirstRun);
        if waits && (!self.builtin_pending.is_empty() || self.builtins_to_load > 0) {
            return false;
        }
        true
    }

    /// Over the process budget after starting `starting`: standby for the
    /// least important idle units (event wakes never cause it). A unit
    /// whose Standby job is already installed is on its way out and not
    /// counted, so starts in a row while those are still asked for their
    /// resources do not put more units in standby than the budget needs.
    fn enforce_budget(&mut self, starting: UnitId, priority: Priority) {
        let budget = self.tunables.process_budget;
        if !pressure::may_reclaim(priority) || !self.procs_third.contains(&starting) {
            return;
        }
        let leaving = self
            .procs_third
            .iter()
            .filter(|u| {
                self.units[u]
                    .job
                    .as_ref()
                    .is_some_and(|job| matches!(job.kind, JobKind::Standby))
            })
            .count();
        let count = self.procs_third.len() - leaving;
        if count <= budget {
            return;
        }
        let candidates = pressure::candidates(
            self.procs_third
                .iter()
                .filter(|u| **u != starting)
                .map(|u| (*u, &self.units[u])),
            &self.tunables,
            self.now,
        );
        for u in candidates.into_iter().take(count - budget) {
            let _ = self.enqueue_job(
                u,
                JobKind::Standby,
                JobMode::Replace,
                Priority::Background,
                Origin::Budget,
            );
        }
    }

    fn fire(&mut self, entry: timers::Entry) {
        let (u, kind) = match entry.target {
            Target::Shutdown => {
                if entry.generation == self.shutdown_generation {
                    let units = self.units.keys().copied().collect::<Vec<_>>();
                    for u in units {
                        self.kill(u);
                    }
                }
                return;
            }
            Target::Unit(u, kind) => (u, kind),
        };
        let now = self.now;
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        if unit.gens.get(kind) != entry.generation {
            return;
        }
        match kind {
            TimerKind::Idle => {
                let UnitState::Active { last_active, .. } = unit.state else {
                    return;
                };
                if unit.job.is_some() {
                    return;
                }
                let due = last_active + idle_time(&self.tunables, unit);
                if now < due {
                    self.timers.arm(&mut unit.gens, u, TimerKind::Idle, due);
                } else if unit.work.busy() {
                    // Busy: a first run's quiet period is over (as in
                    // `resources_answered`).
                    unit.quiet_park = false;
                    let again = now + self.tunables.idle_timeout;
                    self.timers.arm(&mut unit.gens, u, TimerKind::Idle, again);
                } else {
                    let _ = self.enqueue_job(
                        u,
                        JobKind::Standby,
                        JobMode::Replace,
                        Priority::Background,
                        Origin::Timer,
                    );
                }
            }
            TimerKind::StartTimeout => {
                if matches!(unit.state, UnitState::Activating { pid: Some(_), .. }) {
                    self.crash(u, ExitCause::StartTimeout, None);
                }
            }
            TimerKind::StopGrace => self.kill(u),
            TimerKind::StopWait => {
                if unit.state.is_deactivating() {
                    let stuck = !unit.stopping.is_some_and(|s| s.done());
                    self.land(u, stuck);
                } else {
                    // A start waiting for a stop that landed stuck; a process
                    // already gone clears it first.
                    self.land_if_done(u);
                    self.run.push_back(u);
                }
            }
            TimerKind::Verify => self.verify_done(u, false),
            TimerKind::NotBefore => self.run.push_back(u),
            TimerKind::Notice => {
                if matches!(
                    unit.state,
                    UnitState::Activating {
                        sub: ActivatingSub::Spawn | ActivatingSub::Ready,
                        ..
                    }
                ) {
                    self.pending.push(Action::Notify {
                        unit: Some(u),
                        notice: Notice::Starting,
                    });
                }
            }
            TimerKind::Classify => {
                if let Some(pid) = unit.pid() {
                    self.classified(pid, Some("no answer from the playback worker".into()));
                }
            }

            TimerKind::Request => {
                let Some(oldest) = unit.work.oldest_request() else {
                    return;
                };
                let Some(timeout) = self.tunables.policy(unit.class).request_timeout else {
                    return;
                };
                if now >= oldest + timeout {
                    self.crash(u, ExitCause::RequestTimeout, None);
                } else {
                    self.timers
                        .arm(&mut unit.gens, u, TimerKind::Request, oldest + timeout);
                }
            }
        }
        self.sync(u);
    }

    /// Keeps the derived sets of one unit current (slots, built-ins
    /// pending, waiting third-party starts, the budget count) and emits
    /// `ListChanged`. O(log n); called after every change of a unit.
    fn sync(&mut self, u: UnitId) {
        let Some(unit) = self.units.get_mut(&u) else {
            self.slots.remove(&u);
            self.builtin_pending.remove(&u);
            self.queued_third.remove(&u);
            self.procs_third.remove(&u);
            self.boot_waiting.remove(&u);
            self.first_waiting.remove(&u);
            return;
        };
        // A slot is held while the start activates.
        let activating = matches!(
            unit.state,
            UnitState::Activating {
                sub: ActivatingSub::Verify | ActivatingSub::Spawn | ActivatingSub::Ready,
                ..
            }
        );
        if !activating {
            self.slots.remove(&u);
        }
        let pending = unit.class == Class::Builtin
            && unit.job.as_ref().is_some_and(Job::will_start)
            && unit
                .job
                .as_ref()
                .is_some_and(|j| j.starts() || j.state == JobState::Waiting);
        if pending {
            self.builtin_pending.insert(u);
        } else {
            self.builtin_pending.remove(&u);
        }
        let queued = unit.class != Class::Builtin && unit.job.as_ref().is_some_and(|j| j.queued);
        if queued {
            self.queued_third.insert(u);
        } else {
            self.queued_third.remove(&u);
        }
        let counted = self.tunables.policy(unit.class).in_budget
            && unit.pid().is_some()
            && !unit.state.is_deactivating();
        if counted {
            self.procs_third.insert(u);
        } else {
            self.procs_third.remove(&u);
        }
        if waits_at_boot(unit) {
            self.boot_waiting.insert(u);
        } else {
            self.boot_waiting.remove(&u);
        }
        if first_queued(unit) {
            self.first_waiting.insert(u);
        } else {
            self.first_waiting.remove(&u);
        }
        let list = list_state(unit, self.now);
        let changed = unit
            .listed
            .as_ref()
            .is_some_and(|old| !old.same_for_changed(&list));
        unit.listed = Some(list);
        if changed {
            self.pending.push(Action::ListChanged { unit: u });
        }
    }
}
