//! Plugin process supervisor, pure core (doc/architecture/supervisor.md):
//! decides when a plugin process should exist — start, stop, restart,
//! standby, limits — modelled on systemd's service manager. No `Host`, no
//! I/O, no environment, no clock: time comes in as `Ms`, randomness from a
//! seeded xorshift, and every decision goes out as an `Action` for the Host
//! adapter (B2) to execute. Payloads are generic (`serde_json::Value` in
//! production, small numbers in simulations).
//!
//! Inputs without a time argument use the latest time the supervisor was
//! given (`run_jobs`, `touch`, `begin_work`, `shutdown`, `set_now`).
#![forbid(unsafe_code)]
// B1: the core is not wired yet (B2a wires it and removes both allows);
// its tests use it.
#![allow(dead_code, unused_imports)]

mod exit;
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
pub use kill::grace;
pub use limits::{
    Backoff, BuiltinsFirst, Policy, RestartMode, Retry, StartLimit, Tunables, max_active_for,
    process_budget_for,
};
pub use pressure::Pressure;
pub use reap::{STUCK_RELOAD, STUCK_WAKE};
pub use trigger::{AnswerReason, Delivery, Trigger, TriggerKind};
pub use unit::{
    ActivatingSub, Checkpoint, Class, DeactivatingSub, Enablement, FailResult, ListState, Ms, Pid,
    Placement, ProcessState, RunnerId, StopReason, Stopping, Then, Unit, UnitId, UnitSpec,
    UnitState,
};
pub use work::WorkKind;

use queue::{LANES, Lane, Lanes};
use restart::{Rng, RunnerPolicy, retry_delay};
use timers::{Target, TimerKind, Timers};

/// A user-visible notice (OSD; i18n texts in the adapter).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notice {
    /// "Starting <name>…" (a wake slower than 400 ms).
    Starting,
    /// "<name> could not start".
    CouldNotStart,
    /// "<name> stopped" / "…ran out of memory" (T25).
    Crashed(ExitCause),
    /// Start limit hit (T27).
    StartLimitHit,
    /// "Built-in plugins restarted after a fault".
    RunnerRestarted,
    /// "Built-in plugins run separately for this session after repeated
    /// faults".
    RunnerFailedOpen,
}

/// What the Host adapter executes (§5), in order.
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
    /// Reply failed to a held trigger (§4.6).
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
    /// Remove the unit's data (uninstall), then `remove_unit`.
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
    /// A job ended (package transactions read the result).
    JobFinished {
        unit: UnitId,
        job: JobId,
        result: JobResult,
    },
}

/// Start-slot holding of one unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Slot {
    first_run: bool,
    third: bool,
}

/// systemd `Manager`.
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
    slots: BTreeMap<UnitId, Slot>,
    first_runs: usize,
    third_slots: usize,
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
}

/// `packages.list` of one unit (§9). `boot_stopped`:
/// `Tunables::boot_wait_lists_stopped`.
fn list_state<P>(unit: &Unit<P>, boot_stopped: bool) -> ListState {
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
    // Today a boot start waiting in the start queue (or verifying) and a
    // runner member queued again are not running yet: `stopped`.
    let boot_wait = boot_stopped
        && unit.job.as_ref().is_some_and(|job| match job.kind {
            JobKind::Start { cause } => cause == Cause::Boot,
            JobKind::Restart { .. } => job.origin == Origin::Crash,
            _ => false,
        })
        && matches!(
            unit.state,
            UnitState::Inactive
                | UnitState::Deactivating { .. }
                | UnitState::Activating {
                    sub: ActivatingSub::Verify,
                    ..
                }
        );
    if boot_wait {
        return ListState::Stopped;
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
            slots: BTreeMap::new(),
            first_runs: 0,
            third_slots: 0,
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
    pub fn failed_open(&self) -> bool {
        self.runner_policy.failed_open
    }

    // ---- units ---------------------------------------------------------

    /// §5 `add_unit` (systemd: load unit): T1 / T2 / T3.
    pub fn add_unit(&mut self, spec: UnitSpec) -> UnitId {
        if let Some(&u) = self.by_id.get(&spec.id) {
            return u;
        }
        let u = UnitId(self.next_unit);
        self.next_unit += 1;
        let mut unit = Unit::new(&spec);
        if unit.class == Class::Normal && unit.resident {
            unit.class = Class::Resident;
        }
        let class = unit.class;
        let (recordable, recorded, resident) = (unit.recordable, unit.recorded, unit.resident);
        let replay = self.tunables.on_demand
            && spec.enabled
            && recorded
            && !resident
            && self.tunables.policy(class).idle_standby;
        self.by_id.insert(spec.id.clone(), u);
        self.units.insert(u, unit);
        if !spec.enabled {
            self.sync(u);
            return u;
        }
        if replay {
            self.replay(u);
        } else {
            // No recording key (a development load, a built-in without a
            // digest): started at boot, never a first run.
            let first = self.tunables.on_demand && recordable && !recorded;
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

    /// §5 `remove_unit` (after `RemoveData` / development unload).
    pub fn remove_unit(&mut self, u: UnitId) {
        let Some(unit) = self.units.remove(&u) else {
            return;
        };
        self.by_id.remove(&unit.id);
        for pid in [unit.pid(), unit.alias, unit.last_pid, unit.holder]
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

    /// §5 `set_enabled`: unit-file state only; does not enqueue.
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
    /// unit-file state and the `Start{Enable}` together (T32).
    pub fn enable(&mut self, u: UnitId) -> Result<JobId, Busy> {
        if self.package_busy(u) {
            return Err(Busy);
        }
        self.set_enabled(u, true);
        self.start(u, Cause::Enable)
    }

    // ---- requests ------------------------------------------------------

    /// §5 `start`. `Enable` is the user's (`JobMode::Fail` against a package
    /// RPC, as `set_enabled` is today); other causes are internal.
    pub fn start(&mut self, u: UnitId, cause: Cause) -> Result<JobId, Busy> {
        let (mode, origin) = match cause {
            Cause::Enable => (JobMode::Fail, Origin::User),
            Cause::Trigger(_) => (JobMode::Replace, Origin::Trigger),
            Cause::AutoRestart => (JobMode::Replace, Origin::Crash),
            Cause::Boot | Cause::FirstRun => (JobMode::Replace, Origin::Internal),
        };
        self.enqueue_job(u, JobKind::Start { cause }, mode, cause.priority(), origin)
    }

    /// §5 `stop`. Disable is the user's, uninstall/reload/upgrade are
    /// package RPCs (both `JobMode::Fail`); idle/failed/quit are internal.
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

    /// §5 `restart` (a package RPC: reload, install, upgrade, cancel).
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

    /// §5 `standby`.
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

    /// §5 `wake`: a trigger on an entry point of `pid` (systemd socket
    /// activation: the connection is queued on the listening socket). The
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
        if matches!(trigger.kind, TriggerKind::Event(_)) && self.events_muted() {
            return Delivery::Dropped;
        }
        let now = self.now;
        let backlog_max = self.tunables.backlog_max;
        let Some(unit) = self.units.get_mut(&u) else {
            return Delivery::Dropped;
        };
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
                // Sent to the current process (an old pid is forwarded to
                // it), after the job rules: an old pid cannot bypass a
                // restart's absorption (F3).
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
                    // Restart[any] × Start: the start is absorbed, the
                    // trigger held for the new run (§4.2).
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
                    // (nothing to do on an active unit), except an
                    // uninstall (a trigger never re-enables, §4.2).
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
        if !builtin
            && self
                .tunables
                .lane_max
                .is_some_and(|max| self.queued_third.len() >= max)
        {
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

    /// §5 `kill`: escalate the stopping process now (not a job).
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

    /// §5 `reset_failed` (T28).
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

    /// §5 `job`: the installed job.
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

    /// §5 `run_jobs`: due timers, runnable jobs, start slots. O(due +
    /// started + inputs since the last call); never a scan over all units.
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

    /// §5 `next_deadline`: when `run_jobs` has something to do (`now` while
    /// actions or runnable jobs wait).
    pub fn next_deadline(&self) -> Option<Ms> {
        if !self.pending.is_empty() || !self.run.is_empty() {
            return Some(self.now);
        }
        self.timers.next()
    }

    /// §5 `shutdown` (quit / device loss, T31): checkpoints from the
    /// recorded state, every process terminated in parallel with one
    /// deadline `budget`, then killed.
    pub fn shutdown(&mut self, now: Ms, budget: Ms) -> Vec<Action<P>> {
        self.now = self.now.max(now);
        self.shutting_down = true;
        let units = self.units.keys().copied().collect::<Vec<_>>();
        // Every checkpoint before the first Terminate (§7.2 "checkpoints
        // first", coordinator ruling): if the forced exit comes mid-way,
        // each one is already written.
        // As today's `lazy_quit`: every run classified this run, unless its
        // checkpoint already says resident and it still is.
        if self.tunables.on_demand {
            for (u, unit) in &self.units {
                if let UnitState::Active { pid, .. } = unit.state
                    && unit.classified_run
                    && !(unit.resident && unit.recorded && unit.recorded_resident)
                {
                    self.pending.push(Action::Checkpoint {
                        unit: *u,
                        pid,
                        resident: unit.resident,
                    });
                }
            }
        }
        for u in units {
            let Some(unit) = self.units.get_mut(&u) else {
                continue;
            };
            unit.backlog.clear();
            let job = unit.job.take();
            let state = unit.state.clone();
            if let Some(job) = job {
                self.record(u, job.id, JobResult::Canceled);
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

    // ---- event inputs --------------------------------------------------

    /// §5 `process_spawned` (T7 / T8).
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

    /// §5 `verify_done` (T5): not verified → its own process this run.
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

    /// §5 `process_ready` (`runtime.ready`; T9 once the UI activation is
    /// done too).
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

    /// §5 `process_failed` (`runtime.failed`; T10: no retry).
    pub fn process_failed(&mut self, pid: Pid, text: String) {
        let Some(u) = self.current(pid) else {
            return;
        };
        self.crash(u, ExitCause::Refused, Some(text));
        self.sync(u);
    }

    /// §5 `ui_begin`: an initial UI registration of an activating process.
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

    /// §5 `ui_finish` (a failed one is reported with `process_failed`).
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

    /// §5 `process_exited` (T25; expected exits of a deactivating process
    /// are ignored: its reap follows). `text`: the fault's error text (today
    /// `RuntimeFault` writes it into `failures`, shown by `packages.list`);
    /// `None`: the cause's own text.
    pub fn process_exited(&mut self, pid: Pid, cause: ExitCause, text: Option<String>) {
        let Some(u) = self.current(pid) else {
            return;
        };
        self.crash(u, cause, text);
        self.sync(u);
    }

    /// §5 `resources_answered` (`owner.resources` for a Standby job; T14 /
    /// T15).
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
        let asking = matches!(
            unit.job.as_ref(),
            Some(Job {
                kind: JobKind::Standby,
                state: JobState::Running,
                ..
            })
        );
        let Some(asked) = unit.asked_at.filter(|_| asking) else {
            return;
        };
        Timers::cancel(&mut unit.gens, TimerKind::Resources);
        unit.asked_at = None;
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
            unit.park_when_idle = false;
            self.pending.push(Action::Checkpoint {
                unit: u,
                pid,
                resident,
            });
            self.terminate(u, StopReason::Idle, Then::Standby, true);
        }
        self.sync(u);
    }

    /// §5 `classified` (MOVE `lazy_classified`; T12 / T13). `resident`: why
    /// it holds a live resource. A run without a checkpoint is classified
    /// (the 5 s timer runs); with `Tunables::reclassify` (today) any ready
    /// run of a recordable unit may be, once per run, as today's
    /// `lazy_ready` classifies on every ready.
    pub fn classified(&mut self, pid: Pid, resident: Option<String>) {
        let Some(u) = self.current(pid) else {
            return;
        };
        let reclassify = self.tunables.reclassify;
        let parks_when_idle = self.tunables.first_run_parks_when_idle;
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
            unit.first_run = false;
            unit.park_when_idle = false;
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
        } else {
            unit.resident = false;
            if unit.class == Class::Resident {
                unit.class = Class::Normal;
            }
            let idle_standby = self.tunables.policy(unit.class).idle_standby;
            let at_once = unit.run_is_first && idle_standby;
            let busy = unit.job.is_some() || unit.work.busy();
            if !(at_once && !busy) && !reclassify {
                // Classified once (T12): the checkpoint is written now.
                // Today (`reclassify`) a lazy class is written only when it
                // goes to standby, or at quit.
                unit.recorded = true;
                unit.recorded_resident = false;
                self.pending.push(Action::Checkpoint {
                    unit: u,
                    pid,
                    resident: false,
                });
            }
            let Some(unit) = self.units.get_mut(&u) else {
                return;
            };
            // Only a first run goes to standby at once (T12, today's
            // `lazy_classified`); any other run waits for its idle timeout.
            if !at_once {
                self.arm_idle(u);
            } else if busy {
                if parks_when_idle {
                    // Today: it keeps its first-run place and goes to
                    // standby as soon as it is idle (`lazy_first_lazy`).
                    unit.park_when_idle = true;
                } else {
                    unit.first_run = false;
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

    /// §5 `touch`: the unit was used (MOVE `Lazy::touch`).
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

    /// §5 `begin_work`: work in flight keeps the unit out of standby.
    pub fn begin_work(&mut self, pid: Pid, kind: WorkKind, now: Ms) {
        self.touch(pid, now);
        let Some(u) = self.current(pid) else {
            return;
        };
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        let first = !unit.work.busy();
        unit.work.begin(kind, now);
        if first && let Some(timeout) = self.tunables.policy(unit.class).request_timeout {
            self.timers
                .arm(&mut unit.gens, u, TimerKind::Request, now + timeout);
        }
    }

    /// §5 `end_work`.
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

    /// §5 `runner_failed` (T29, §7.4): the runner group stops at once; every
    /// member gets a Restart job (immediate today; `runner_backoff` from
    /// B4); fail-open after `runner_restart_limit` in the window.
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
            .next_delay(now, self.tunables.runner_backoff);
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
        // Today `restart_shared` queues the members again at once; from B4
        // they wait for the playback worker's cleanup (§7.4).
        let cleaned = !self.tunables.runner_restart_waits_cleanup;
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
                backend_cleaned: cleaned,
                deadline: now + stop_wait,
                kept: false,
            });
            if !failed_open && unit.verifiable {
                unit.verify = true;
            }
            // Re-placement (§7.4, A4): a member with a standby checkpoint
            // and no start in flight goes back to standby by replay (no
            // spawn), as today's `start_installed` replays a recording.
            let replay = self.tunables.on_demand
                && unit.recorded
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

    /// §5 `memory_pressure` (B5): Low/Critical put every idle reclaimable
    /// unit in standby through the same Standby job (A13).
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
    pub fn list_state(&self, u: UnitId) -> Option<ListState> {
        let boot_stopped = self.tunables.boot_wait_lists_stopped;
        self.units
            .get(&u)
            .map(|unit| list_state(unit, boot_stopped))
    }
    pub fn units(&self) -> impl Iterator<Item = (UnitId, &Unit<P>)> {
        self.units.iter().map(|(u, unit)| (*u, unit))
    }
    /// Third-party processes counted in the budget.
    pub fn budget_count(&self) -> usize {
        self.procs_third.len()
    }
    /// Third-party start slots and first-run places in use.
    pub fn slots_in_use(&self) -> (usize, usize) {
        (self.third_slots, self.first_runs)
    }

    // ---- internals -----------------------------------------------------

    fn alloc_pid(&mut self) -> Pid {
        let pid = Pid(self.next_pid);
        self.next_pid += 1;
        pid
    }

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

    fn events_muted(&self) -> bool {
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

    /// Every held trigger is answered (§4.6); events and follows go
    /// silently. Today (`answer_held` off) they all go silently.
    pub(crate) fn answer_backlog(&mut self, u: UnitId, reason: AnswerReason) {
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        let backlog = std::mem::take(&mut unit.backlog);
        if !self.tunables.answer_held {
            return;
        }
        for trigger in backlog {
            self.answer(u, reason, trigger);
        }
    }

    /// T1 (and runner re-placement, A4): the checkpoint's registrations are
    /// replayed under a new holder pid; the unit is in standby.
    fn replay(&mut self, u: UnitId) {
        let holder = self.alloc_pid();
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        // Earlier pids are dead (a runner member replayed after its
        // process went): messages to them are dropped, as today.
        for old in [unit.alias.take(), unit.last_pid.take()]
            .into_iter()
            .flatten()
        {
            if self.by_pid.get(&old) == Some(&u) {
                self.by_pid.remove(&old);
            }
        }
        unit.last_pid = Some(holder);
        unit.state = UnitState::Standby {
            holder,
            recorded: true,
            hold_until: None,
        };
        self.by_pid.insert(holder, u);
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
        if !self.tunables.answer_held {
            return;
        }
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
            TimerKind::Resources,
            TimerKind::Request,
            TimerKind::Verify,
        ] {
            Timers::cancel(&mut unit.gens, kind);
        }
        unit.classifying = false;
        unit.classified_run = false;
        unit.may_reclassify = false;
        unit.park_when_idle = false;
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
        if !self.tunables.on_demand
            || !self.tunables.policy(unit.class).idle_standby
            || !unit.recordable
            || unit.resident
            || unit.classifying
        {
            return;
        }
        let due = if unit.park_when_idle {
            self.now + self.tunables.idle_check
        } else {
            last_active + self.tunables.idle_timeout
        };
        self.timers.arm(&mut unit.gens, u, TimerKind::Idle, due);
    }

    /// Stops the unit's process (T24 / T12 / T14 / T25...).
    fn terminate(&mut self, u: UnitId, reason: StopReason, then: Then, keep: bool) {
        let Some(unit) = self.units.get(&u) else {
            return;
        };
        let grace = kill::grace(self.tunables.policy(unit.class), reason, then);
        self.terminate_with(u, reason, then, keep, grace);
    }

    fn terminate_with(&mut self, u: UnitId, reason: StopReason, then: Then, keep: bool, grace: Ms) {
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
        self.timers
            .arm(&mut unit.gens, u, TimerKind::StopGrace, now + grace);
        self.timers
            .arm(&mut unit.gens, u, TimerKind::StopWait, now + stop_wait);
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

    /// T6: a new pid, registrations rekeyed from the holder, `Spawn`.
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
        unit.park_when_idle = false;
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
        self.timers
            .arm(&mut unit.gens, u, TimerKind::StartTimeout, now + timeout);
        let mut priority = Priority::Background;
        if let Some(job) = unit.job.as_mut() {
            job.deadline = Some(now + timeout);
            priority = job.priority;
            if matches!(job.cause(), Some(Cause::Trigger(_))) {
                let due = (job.created + starting_notice).max(now);
                self.timers.arm(&mut unit.gens, u, TimerKind::Notice, due);
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

    /// T8: the process could not be created.
    fn spawn_failed(&mut self, u: UnitId, pid: Pid, text: String) {
        let now = self.now;
        let keeps = self.tunables.start_failure_keeps_registrations;
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
        // Today only a wake's failure shows "could not start"
        // (`wake_failed`); a boot, first run, enable or reload only records
        // it (`start_installed`, `start_stopped`).
        let woken = matches!(
            unit.job.as_ref().and_then(Job::cause),
            Some(Cause::Trigger(_))
        );
        let notice = woken || self.tunables.start_failure_notice;
        self.drop_alias(u);
        self.answer_backlog(u, AnswerReason::CouldNotStart);
        self.finish(u, JobResult::Failed(text.clone()));
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        if keeps && unit.recorded && !unit.resident && policy.idle_standby {
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
        if notice {
            self.pending.push(Action::Notify {
                unit: Some(u),
                notice: Notice::CouldNotStart,
            });
        }
    }

    /// T9: ready and every initial UI registration done.
    fn try_ready(&mut self, u: UnitId) {
        let now = self.now;
        let on_demand = self.tunables.on_demand;
        let reclassify = self.tunables.reclassify;
        let classify_wait = self.tunables.classify_wait;
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
        // A run without a checkpoint is classified (T12/T13); never a
        // development load or a unit without a recording key (today's
        // `lazy_development`, `lazy_key_of` is `None`). With `reclassify`
        // (today) a recorded run may be classified again (`classified`),
        // without the timer.
        if on_demand && unit.recordable && !unit.recorded {
            unit.classifying = true;
            self.timers
                .arm(&mut unit.gens, u, TimerKind::Classify, now + classify_wait);
        } else if on_demand && unit.recordable {
            // Its class is the checkpoint's until classified again.
            unit.classified_run = true;
            unit.may_reclassify = reclassify;
        }
        if unit.job.as_ref().is_some_and(Job::starts) {
            self.finish(u, JobResult::Done);
        }
        self.arm_idle(u);
    }

    /// The process failed or ended unasked (T10 / T11 / T25), per the
    /// class's `Restart=`.
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
        // where the class's `Restart=` says).
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
        // Its entry points can wait in standby (never a Resident's: it is
        // started, not woken).
        let standby_capable = recorded && !unit.resident && policy.idle_standby;
        let retry_left = activating
            && cause == ExitCause::StartTimeout
            && policy
                .start_retry
                .is_some_and(|r| unit.restart.retries < r.count);
        let mut then = Then::Failed;
        let mut keep = false;
        let mut failing = (FailResult::Exit(cause), text.clone());
        let mut auto_restart = None;
        if retry_left {
            let retry = policy.start_retry.unwrap_or(Retry {
                count: 0,
                base: 0,
                jitter_pct: 0,
            });
            let delay = retry_delay(retry, unit.restart.retries, &mut self.rng);
            unit.restart.retries += 1;
            then = if standby_capable {
                Then::Standby
            } else {
                Then::Inactive
            };
            keep = standby_capable;
            unit.after_reap = Some(delay);
            if let Some(job) = unit.job.as_mut() {
                job.state = JobState::Waiting;
                job.not_before = None;
            }
        } else if cause.restartable() && !development {
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
        if then == Then::Failed {
            unit.failing = Some(failing);
            let starts = unit.job.as_ref().is_some_and(Job::starts);
            self.answer_backlog(u, AnswerReason::CouldNotStart);
            if starts {
                self.finish(u, JobResult::Failed(text.clone()));
            }
        } else if !retry_left && auto_restart.is_none() {
            // Standby with a hold (T20 ruling): interactive triggers are
            // answered now; the start job stays (waiting for the hold) only
            // while event triggers remain in the backlog — no demand, no
            // start of a crashing plugin.
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
        // Today a crash only records the failure; the notice is B4's.
        if cause.notifies() && self.tunables.crash_notice {
            self.pending.push(Action::Notify {
                unit: Some(u),
                notice: Notice::Crashed(cause),
            });
        }
        if limit_hit {
            self.pending.push(Action::Notify {
                unit: Some(u),
                notice: Notice::StartLimitHit,
            });
        }
        let failure_grace = self.tunables.policy(self.units[&u].class).failure_grace;
        self.terminate_with(u, cause.stop_reason(), then, keep, failure_grace);
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
                        self.pending.push(Action::RemoveData { unit: u });
                    }
                    _ => {
                        if reason == StopReason::Disable
                            || !matches!(state, UnitState::Failed { .. })
                        {
                            unit.state = UnitState::Inactive;
                        }
                    }
                }
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
        // Today the triggers held for the old run are lost with it
        // (`lazy.waking`, `Parked.triggers`); from B3 they carry over.
        if !self.tunables.restart_keeps_backlog {
            self.answer_backlog(u, AnswerReason::Stopped);
        }
        if !enabled {
            self.finish(u, JobResult::Done);
            return;
        }
        self.step_start(u);
    }

    fn step_standby(&mut self, u: UnitId) {
        let now = self.now;
        let resources_wait = self.tunables.resources_wait;
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
                if let Some(wait) = resources_wait {
                    self.timers
                        .arm(&mut unit.gens, u, TimerKind::Resources, now + wait);
                }
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
                let first_run = lane.first_run() && self.tunables.first_run_limit.is_some();
                let third = !lane.builtin() && self.tunables.max_active.is_some();
                if first_run || third {
                    self.slots.insert(u, Slot { first_run, third });
                    self.first_runs += usize::from(first_run);
                    self.third_slots += usize::from(third);
                    if let Some(unit) = self.units.get_mut(&u) {
                        unit.first_run = first_run;
                    }
                }
                self.begin_start(u);
                self.sync(u);
                started = true;
            }
        }
        started
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
        if lane.first_run()
            && self
                .tunables
                .first_run_limit
                .is_some_and(|limit| self.first_runs >= limit)
        {
            return false;
        }
        if lane.builtin() {
            return true;
        }
        if let Some(max) = self.tunables.max_active {
            let mut limit = max;
            if lane == Lane::Interactive {
                limit += self.tunables.interactive_reserve;
                if self.pressure != Pressure::Critical {
                    limit += self.tunables.interactive_overrun;
                }
            }
            if self.third_slots >= limit {
                return false;
            }
        }
        let waits = match self.tunables.builtins_first {
            BuiltinsFirst::Queued => lane == Lane::FirstRun,
            BuiltinsFirst::Installed => matches!(lane, Lane::Background | Lane::FirstRun),
        };
        if waits && !self.builtin_pending.is_empty() {
            return false;
        }
        true
    }

    /// Over the process budget after starting `starting`: standby for the
    /// least important idle units (event wakes never cause it).
    fn enforce_budget(&mut self, starting: UnitId, priority: Priority) {
        let Some(budget) = self.tunables.process_budget else {
            return;
        };
        if !pressure::may_reclaim(priority) || !self.procs_third.contains(&starting) {
            return;
        }
        let count = self.procs_third.len();
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
                Origin::Pressure,
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
                // A first run classified lazy while busy: no idle time,
                // checked every `idle_check` (today's `lazy_first_lazy`).
                let (due, again) = if unit.park_when_idle {
                    (now, now + self.tunables.idle_check)
                } else {
                    (
                        last_active + self.tunables.idle_timeout,
                        now + self.tunables.idle_timeout,
                    )
                };
                if now < due {
                    self.timers.arm(&mut unit.gens, u, TimerKind::Idle, due);
                } else if unit.work.busy() {
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
            TimerKind::StopWait => self.land(u, true),
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
            TimerKind::Resources => {
                if let Some(pid) = unit.pid() {
                    self.resources_answered(pid, true);
                }
            }
            TimerKind::Request => {
                let Some(oldest) = unit.work.oldest() else {
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
            if let Some(slot) = self.slots.remove(&u) {
                self.first_runs -= usize::from(slot.first_run);
                self.third_slots -= usize::from(slot.third);
            }
            self.builtin_pending.remove(&u);
            self.queued_third.remove(&u);
            self.procs_third.remove(&u);
            return;
        };
        if let Some(slot) = self.slots.get(&u).copied() {
            let activating = matches!(
                unit.state,
                UnitState::Activating {
                    sub: ActivatingSub::Verify | ActivatingSub::Spawn | ActivatingSub::Ready,
                    ..
                }
            );
            let holds = if slot.first_run {
                unit.first_run
                    && (activating
                        || matches!(
                            unit.state,
                            UnitState::Active { .. } | UnitState::Deactivating { .. }
                        ))
            } else {
                activating
            };
            if !holds {
                self.slots.remove(&u);
                self.first_runs -= usize::from(slot.first_run);
                self.third_slots -= usize::from(slot.third);
                unit.first_run = false;
            }
        }
        let installed = self.tunables.builtins_first == BuiltinsFirst::Installed;
        let pending = unit.class == Class::Builtin
            && unit.job.as_ref().is_some_and(Job::will_start)
            && (installed
                || matches!(
                    unit.state,
                    UnitState::Inactive
                        | UnitState::Standby { .. }
                        | UnitState::Failed { .. }
                        | UnitState::Activating {
                            sub: ActivatingSub::Verify,
                            ..
                        }
                ))
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
        let list = list_state(unit, self.tunables.boot_wait_lists_stopped);
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
