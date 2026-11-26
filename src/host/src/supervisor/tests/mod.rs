//! Spec tests for the pure supervisor core, written from the design
//! without reading the implementation. Each test is named after the row of
//! the original design tables it checks (`t19_…` = transition row,
//! `m_<row>_<col>` = a merge table cell, `nc_…` = a named case, …; that
//! text is in git history).
//!
//! Every guess about the concrete Rust shape (type and field names,
//! argument types, how a finished job's result is read) is confined to the
//! **adapter** section of this file, so that only this file needs mapping
//! if the core spells something differently. The test files themselves
//! talk only to `Sim`, `Spec`, `Profile`, `S`, `J` and the action helpers.
//!
//! Two tunable profiles: `Profile::target()` is the player's (idle 30 s,
//! start 60 s, stop 5 s, `max_active` 4 + 1 Interactive reserve, backlog
//! 64, lane 256, start retry 3, per-class restart policy, request timeout
//! 5 min, a process budget); `Profile::other_timers()` keeps those rules
//! with the timer values the earlier tests count with. `spec!` runs a test
//! under `target()`.
//!
//! Lines marked `// ANSWERED` were design questions, since answered;
//! lines marked `// DESIGN QUESTION:` are places where the design was
//! ambiguous; the test encodes the author's best reading.
#![forbid(unsafe_code)]
#![allow(dead_code)]

use std::fmt::Debug;

use super::*;

pub(super) type Ms = u64;

/// The time every simulation starts at (not 0, so "armed at 0" bugs show).
pub(super) const T0: Ms = 1_000;
/// `STOP_WAIT`: one constant for every "wait for the old process" deadline.
pub(super) const STOP_WAIT: Ms = 20_000;
/// `VERIFY_WAIT`.
pub(super) const VERIFY_WAIT: Ms = 3_000;
/// `STARTING_OSD_MS`.
pub(super) const STARTING_OSD: Ms = 400;
/// `QUIET_AFTER_START_MS` ("Event wakes").
pub(super) const QUIET_AFTER_START: Ms = 1_000;
/// A lazy first run's idle time before standby.
pub(super) const FIRST_RUN_QUIET: Ms = 1_000;
/// Text of a Start that waited on a stop job (reload).
pub(super) const TEXT_RELOAD_STUCK: &str = "the previous process did not stop within 20 s";
/// Text of a wake that waited on a standby stop.
pub(super) const TEXT_WAKE_STUCK: &str = "its stopped process did not go within 20 s";

// ---------------------------------------------------------------------------
// Profiles (Tunables)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub(super) struct Profile {
    pub name: &'static str,
    /// Idle timeout.
    pub idle: Ms,
    /// The start timeout of installed plugins.
    pub start_timeout: Ms,
    /// The stop grace for disable/reload/uninstall/upgrade.
    pub stop_grace: Ms,
    /// The stop grace for standby.
    pub standby_grace: Ms,
    /// The stop grace after a failure.
    pub failure_grace: Ms,
    /// Third-party running Start jobs limit.
    pub max_active: usize,
    /// Interactive reserve slots.
    pub reserve: usize,
    /// `backlog_max`.
    pub backlog_max: usize,
    /// Third-party lane waiting Start jobs.
    pub lane_max: usize,
    /// Start retries within one Start job.
    pub start_retries: u32,
    /// Request timeout.
    pub request_timeout: Option<Ms>,
    /// Third-party process budget.
    pub budget: usize,
}

impl Profile {
    /// The player's values (`Tunables::new` for 8 cores) with a budget of
    /// 3 third-party processes.
    pub fn target() -> Self {
        Profile {
            name: "target",
            idle: 30_000,
            start_timeout: 60_000,
            stop_grace: 5_000,
            standby_grace: 5_000,
            failure_grace: 300,
            // min(4, max(2, cores / 2)) with cores = 8.
            max_active: 4,
            reserve: 1,
            backlog_max: 64,
            lane_max: 256,
            start_retries: 3,
            request_timeout: Some(300_000),
            budget: 3,
        }
    }
    /// The player's rules with other timer values: idle 60 s, start 15 s,
    /// stop 300 ms, standby 2 s, no request timeout and no process budget
    /// (the numbers the tests written for the earlier batches count with).
    pub fn other_timers() -> Self {
        Profile {
            name: "other_timers",
            idle: 60_000,
            start_timeout: 15_000,
            stop_grace: 300,
            standby_grace: 2_000,
            request_timeout: None,
            budget: usize::MAX,
            ..Profile::target()
        }
    }
}

/// A spec test run under `Profile::target()`, bound to `$p`.
macro_rules! spec {
    ($(#[$m:meta])* fn $name:ident($p:ident) $body:block) => {
        $(#[$m])*
        #[test]
        #[allow(unused_variables)]
        fn $name() {
            let $p = &Profile::target();
            $body
        }
    };
}

// Test files come after `spec!` (macro_rules is textually scoped).
mod b3;
mod b4;
mod backlog;
mod budget;
mod deadlines;
mod lanes;
mod merge;
mod named_cases;
mod policy;
mod stop;
mod transitions;

// ---------------------------------------------------------------------------
// Test-side views (stable vocabulary the test files use)
// ---------------------------------------------------------------------------

/// Unit state flattened with its sub-state.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum S {
    Inactive,
    Verify,
    Spawn(Pid),
    Ready(Pid),
    AutoRestart(Ms),
    Active(Pid),
    Term(Then),
    Kill(Then),
    Cleanup(Then),
    /// Normalised `Result=` (e.g. "startlimithit", "spawn").
    Failed(String),
    Standby {
        holder: Pid,
        hold_until: Option<Ms>,
    },
}

impl S {
    pub fn is_deactivating(&self) -> bool {
        matches!(self, S::Term(_) | S::Kill(_) | S::Cleanup(_))
    }
    pub fn then(&self) -> Option<Then> {
        match self {
            S::Term(t) | S::Kill(t) | S::Cleanup(t) => Some(*t),
            _ => None,
        }
    }
    pub fn is_failed(&self) -> bool {
        matches!(self, S::Failed(_))
    }
}

/// Installed job, flattened.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum J {
    Start {
        cause: String,
        running: bool,
    },
    Stop {
        then: Then,
        running: bool,
    },
    Restart {
        fresh: bool,
        image: Option<String>,
        running: bool,
    },
    Standby {
        running: bool,
    },
}

impl J {
    pub fn kind(&self) -> &'static str {
        match self {
            J::Start { .. } => "Start",
            J::Stop { .. } => "Stop",
            J::Restart { .. } => "Restart",
            J::Standby { .. } => "Standby",
        }
    }
}

/// Finished job result (`done`, `failed`, `canceled`, `timeout`).
#[derive(Clone, Debug, PartialEq)]
pub(super) enum R {
    Done,
    Failed(String),
    Canceled,
    Timeout,
    Dropped,
}

/// Why a held trigger was answered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Why {
    Busy,
    CouldNotStart,
    Disabled,
    Removed,
    Stopped,
    /// In standby with a restart hold.
    Held,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Ck {
    /// No checkpoint: first run.
    None,
    /// Checkpoint says "can go to standby".
    Standby,
    /// Checkpoint says resident.
    Resident,
}

/// Unit description given to `add_unit`.
#[derive(Clone, Debug)]
pub(super) struct Spec {
    pub id: String,
    pub class: Class,
    pub runner: Option<u32>,
    pub enabled: bool,
    pub ck: Ck,
    /// Built-in needing a digest check.
    pub verify: bool,
    /// `--plugin` start timeout (1–120 s).
    pub dev_timeout: Option<Ms>,
}

impl Spec {
    fn new(id: &str, class: Class, ck: Ck) -> Self {
        Spec {
            id: id.into(),
            class,
            runner: None,
            enabled: true,
            ck,
            verify: false,
            dev_timeout: None,
        }
    }
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }
    pub fn shared(mut self, runner: u32) -> Self {
        self.runner = Some(runner);
        self
    }
    pub fn verify(mut self) -> Self {
        self.verify = true;
        self
    }
    pub fn ck(mut self, ck: Ck) -> Self {
        self.ck = ck;
        self
    }
}

/// Normal third-party plugin with a "can go to standby" checkpoint.
pub(super) fn normal(id: &str) -> Spec {
    Spec::new(id, Class::Normal, Ck::Standby)
}
/// Normal third-party plugin without a checkpoint (first run).
pub(super) fn first_run(id: &str) -> Spec {
    Spec::new(id, Class::Normal, Ck::None)
}
/// Third-party plugin recorded resident.
pub(super) fn resident(id: &str) -> Spec {
    // Resident is observed, never declared: a Normal plugin whose checkpoint
    // says resident.
    Spec::new(id, Class::Normal, Ck::Resident)
}
/// Built-in in its own process that does not go to standby.
pub(super) fn builtin(id: &str) -> Spec {
    Spec::new(id, Class::Builtin, Ck::Resident)
}
/// Built-in with a "can go to standby" checkpoint.
pub(super) fn builtin_standby(id: &str) -> Spec {
    Spec::new(id, Class::Builtin, Ck::Standby)
}
/// `--plugin` development load.
pub(super) fn dev(id: &str, timeout: Ms) -> Spec {
    let mut s = Spec::new(id, Class::Development, Ck::None);
    s.dev_timeout = Some(timeout);
    s
}

/// Lower-case alphanumerics of a value's `Debug` text, for comparing
/// enum spellings without depending on their exact Rust shape.
pub(super) fn norm(x: impl Debug) -> String {
    format!("{x:?}")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// ADAPTER: the only place that names the core's concrete Rust shapes
// (mapped to the core API).
// ---------------------------------------------------------------------------

pub(super) type Act = Action<u32>;

/// Memory for `Tunables::new` (the budget is overridden from the profile).
const PHYS_MIB: u64 = 16_384;

/// Tunables from the core's preset (`Tunables::new`, 8 cores) with the
/// profile's timers and budget; `t00_presets_match_design_values` checks
/// that `Profile::target()` is the preset.
fn tunables(p: &Profile) -> Tunables {
    let mut t = Tunables::new(8, PHYS_MIB);
    t.idle_timeout = p.idle;
    for policy in [&mut t.builtin, &mut t.resident, &mut t.normal] {
        policy.start_timeout = p.start_timeout;
        policy.stop_grace = p.stop_grace;
        policy.standby_grace = p.standby_grace;
        policy.failure_grace = p.failure_grace;
        policy.request_timeout = p.request_timeout;
    }
    t.process_budget = p.budget;
    t
}

fn unit_spec(s: &Spec) -> UnitSpec {
    let mut u = UnitSpec::new(s.id.clone(), s.class);
    if !s.enabled {
        u = u.disabled();
    }
    if s.runner.is_some() {
        u = u.shared();
    }
    if s.verify {
        u = u.verify();
    }
    match s.ck {
        Ck::None => {}
        Ck::Standby => u = u.checkpoint(false),
        Ck::Resident => u = u.checkpoint(true),
    }
    if let Some(ms) = s.dev_timeout {
        u = u.start_timeout(ms);
    }
    u
}

fn state_view(s: &UnitState) -> S {
    match s {
        UnitState::Inactive => S::Inactive,
        UnitState::Activating { pid, sub, .. } => match sub {
            ActivatingSub::Verify => S::Verify,
            ActivatingSub::Spawn => S::Spawn(pid.expect("Spawn has a pid")),
            ActivatingSub::Ready => S::Ready(pid.expect("Ready has a pid")),
            ActivatingSub::AutoRestart { until } => S::AutoRestart(*until),
        },
        UnitState::Active { pid, .. } => S::Active(*pid),
        UnitState::Deactivating { then, sub, .. } => match sub {
            DeactivatingSub::Term => S::Term(*then),
            DeactivatingSub::Kill => S::Kill(*then),
            DeactivatingSub::Cleanup => S::Cleanup(*then),
        },
        UnitState::Failed { result, .. } => S::Failed(norm(result)),
        UnitState::Standby {
            holder, hold_until, ..
        } => S::Standby {
            holder: *holder,
            hold_until: *hold_until,
        },
    }
}

fn job_view(j: &Job) -> J {
    let running = j.state == JobState::Running;
    match &j.kind {
        JobKind::Start { cause } => J::Start {
            // A merged trigger raises the job priority, not the cause.
            cause: match cause {
                Cause::Trigger(_) => format!("trigger{}", norm(j.priority)),
                other => norm(other),
            },
            running,
        },
        JobKind::Stop { then, .. } => J::Stop {
            then: *then,
            running,
        },
        JobKind::Restart { fresh, image } => J::Restart {
            fresh: *fresh,
            image: image.clone(),
            running,
        },
        JobKind::Standby => J::Standby { running },
    }
}

/// Command (`commands.execute`) trigger with payload `p`: Interactive.
pub(super) fn cmd(p: u32) -> Trigger<u32> {
    Trigger::interactive(p)
}
/// `source.resolveRequest` trigger: Interactive.
pub(super) fn resolve(p: u32) -> Trigger<u32> {
    Trigger::interactive(p)
}
/// `drops.drop` trigger: Interactive.
pub(super) fn drop_(p: u32) -> Trigger<u32> {
    Trigger::interactive(p)
}
/// `ui.callback` trigger: Interactive.
pub(super) fn callback(p: u32) -> Trigger<u32> {
    Trigger::interactive(p)
}
/// Wake event broadcast (`WAKE_EVENTS`): Event priority.
pub(super) fn event(name: &str, p: u32) -> Trigger<u32> {
    Trigger::event(name, p)
}

pub(super) fn act_name(a: &Act) -> &'static str {
    match a {
        Action::Spawn { .. } => "Spawn",
        Action::Verify { .. } => "Verify",
        Action::Replay { .. } => "Replay",
        Action::Rekey { .. } => "Rekey",
        Action::Prune { .. } => "Prune",
        Action::Deliver { .. } => "Deliver",
        Action::Answer { .. } => "Answer",
        Action::QueryResources { .. } => "QueryResources",
        Action::Checkpoint { .. } => "Checkpoint",
        Action::DropCheckpoint { .. } => "DropCheckpoint",
        Action::Terminate { .. } => "Terminate",
        Action::Kill { .. } => "Kill",
        Action::KillRunner { .. } => "KillRunner",
        Action::Revoke { .. } => "Revoke",
        Action::RemoveData { .. } => "RemoveData",
        Action::Notify { .. } => "Notify",
        Action::ListChanged { .. } => "ListChanged",
        Action::JobFinished { .. } => "JobFinished",
    }
}

pub(super) fn spawn_pid(a: &Act) -> Option<Pid> {
    match a {
        Action::Spawn { pid, .. } => Some(*pid),
        _ => None,
    }
}
pub(super) fn kill_pid(a: &Act) -> Option<Pid> {
    match a {
        Action::Kill { pid, .. } => Some(*pid),
        _ => None,
    }
}
pub(super) fn query_pid(a: &Act) -> Option<Pid> {
    match a {
        Action::QueryResources { pid, .. } => Some(*pid),
        _ => None,
    }
}
pub(super) fn rekey(a: &Act) -> Option<(Pid, Pid)> {
    match a {
        Action::Rekey { from, to, .. } => Some((*from, *to)),
        _ => None,
    }
}
/// `(pid, payload)` of a `Deliver`.
pub(super) fn delivered(a: &Act) -> Option<(Pid, u32)> {
    match a {
        Action::Deliver { pid, payload, .. } => Some((*pid, *payload)),
        _ => None,
    }
}
/// `(payload, why)` of an `Answer`.
pub(super) fn answered(a: &Act) -> Option<(u32, Why)> {
    match a {
        Action::Answer {
            reason, payload, ..
        } => Some((
            *payload,
            match reason {
                AnswerReason::Busy => Why::Busy,
                AnswerReason::CouldNotStart => Why::CouldNotStart,
                AnswerReason::Disabled => Why::Disabled,
                AnswerReason::Removed => Why::Removed,
                AnswerReason::Stopped => Why::Stopped,
                AnswerReason::Held { .. } => Why::Held,
            },
        )),
        _ => None,
    }
}
/// `Terminate{grace, keep_registrations, reason}` → (grace ms, keep, norm(reason), pid).
pub(super) fn terminate(a: &Act) -> Option<(Ms, bool, String, Pid)> {
    match a {
        Action::Terminate {
            pid,
            grace,
            keep_registrations,
            reason,
            ..
        } => Some((*grace, *keep_registrations, norm(reason), *pid)),
        _ => None,
    }
}
/// A "<name> could not start" notice (today's `wake_failed`).
pub(super) fn is_could_not_start_notice(a: &Act) -> bool {
    matches!(
        a,
        Action::Notify {
            notice: Notice::CouldNotStart,
            ..
        }
    )
}
/// A "…ran out of memory" notice.
pub(super) fn is_oom_notice(a: &Act) -> bool {
    matches!(
        a,
        Action::Notify {
            notice: Notice::Crashed(ExitCause::Oom),
            ..
        }
    )
}

pub(super) struct Sim {
    pub sup: Supervisor<u32>,
    pub now: Ms,
    pub p: Profile,
    /// Every action returned so far, in order.
    pub log: Vec<Act>,
}

impl Sim {
    pub fn new(p: &Profile) -> Sim {
        let mut sup = Supervisor::new(tunables(p), 1);
        sup.set_now(T0);
        Sim {
            sup,
            now: T0,
            p: p.clone(),
            log: Vec::new(),
        }
    }
    fn keep(&mut self, acts: Vec<Act>) -> Vec<Act> {
        self.log.extend(acts.iter().cloned());
        acts
    }
    /// `run_jobs(now)` at the current time.
    pub fn run(&mut self) -> Vec<Act> {
        let acts = self.sup.run_jobs(self.now);
        self.keep(acts)
    }
    /// Move the clock to `t` (absolute, monotonic) and `run_jobs(t)`.
    pub fn at(&mut self, t: Ms) -> Vec<Act> {
        assert!(t >= self.now, "time goes forward: {t} < {}", self.now);
        self.now = t;
        self.run()
    }
    /// Move the clock by `d` and `run_jobs`.
    pub fn after(&mut self, d: Ms) -> Vec<Act> {
        let t = self.now + d;
        self.at(t)
    }
    /// Start-up is over ("Event wakes"; the adapter reports it with `settled`),
    /// and the quiet second after it has passed.
    pub fn settle(&mut self) -> Vec<Act> {
        self.sup.settled(self.now);
        self.after(QUIET_AFTER_START)
    }
    /// Start-up in progress: wake events wake nothing (`set_muted`).
    pub fn mute(&mut self) {
        self.sup.set_muted(true)
    }

    pub fn add(&mut self, s: Spec) -> UnitId {
        self.sup.add_unit(unit_spec(&s))
    }
    fn u(&self, u: UnitId) -> &Unit<u32> {
        self.sup.unit(u).expect("unit exists")
    }
    pub fn st(&self, u: UnitId) -> S {
        state_view(&self.u(u).state)
    }
    pub fn job(&self, u: UnitId) -> Option<J> {
        self.sup.job(u).map(job_view)
    }
    /// The result of a job that ended, looked up on every unit
    /// (`job_result(u, id)`).
    pub fn result(&self, id: JobId) -> Option<R> {
        let r = self
            .sup
            .units()
            .find_map(|(u, _)| self.sup.job_result(u, id).cloned())?;
        Some(match r {
            JobResult::Done => R::Done,
            JobResult::Failed(text) => R::Failed(text),
            JobResult::Canceled => R::Canceled,
            JobResult::Dropped => R::Dropped,
        })
    }
    pub fn class(&self, u: UnitId) -> Class {
        self.u(u).class
    }
    pub fn own_process(&self, u: UnitId) -> bool {
        matches!(self.u(u).placement, Placement::Own)
    }
    pub fn enabled(&self, u: UnitId) -> bool {
        self.u(u).enabled()
    }
    pub fn backlog_len(&self, u: UnitId) -> usize {
        self.u(u).backlog.len()
    }
    pub fn stuck(&self, u: UnitId) -> bool {
        self.u(u).stuck
    }
    pub fn failure_text(&self, u: UnitId) -> Option<String> {
        match &self.u(u).state {
            UnitState::Failed { text, .. } => Some(text.clone()),
            _ => None,
        }
    }
    pub fn unit_of(&self, pid: Pid) -> Option<UnitId> {
        self.sup.by_pid(pid)
    }
    pub fn by_id(&self, id: &str) -> Option<UnitId> {
        self.sup.by_id(id)
    }
    /// The unit's current (or holder) pid; panics if it has none.
    pub fn pid(&self, u: UnitId) -> Pid {
        match &self.u(u).state {
            UnitState::Standby { holder, .. } => *holder,
            other => other.pid().unwrap_or_else(|| panic!("no pid in {other:?}")),
        }
    }

    // requests
    pub fn start(&mut self, u: UnitId, cause: Cause) -> Result<JobId, Busy> {
        self.sup.start(u, cause)
    }
    pub fn start_interactive(&mut self, u: UnitId) -> Result<JobId, Busy> {
        self.sup.start(u, Cause::Trigger(Priority::Interactive))
    }
    /// A stop request in `JobMode::Replace` (the merge table). The core's
    /// `stop` gives disable/uninstall/reload `JobMode::Fail`; package-RPC
    /// conflicts are tested with `rpc_*`.
    pub fn stop(&mut self, u: UnitId, reason: StopReason, then: Then) -> Result<JobId, Busy> {
        let kind = JobKind::Stop { reason, then };
        self.sup.enqueue_job(
            u,
            kind,
            JobMode::Replace,
            Priority::Background,
            Origin::Internal,
        )
    }
    /// A restart request in `JobMode::Replace` (the core's `restart` is a
    /// package RPC; see `rpc_restart`).
    pub fn restart(&mut self, u: UnitId, fresh: bool, image: Option<&str>) -> Result<JobId, Busy> {
        let kind = JobKind::Restart {
            fresh,
            image: image.map(|i| i.to_string()),
        };
        self.sup.enqueue_job(
            u,
            kind,
            JobMode::Replace,
            Priority::Interactive,
            Origin::Internal,
        )
    }
    /// The user's disable as the package RPC does it: unit-file state, then the
    /// core's `stop(Disable)` (origin User, `JobMode::Fail` only against a
    /// package-RPC job).
    pub fn user_disable(&mut self, u: UnitId) -> Result<JobId, Busy> {
        self.sup.set_enabled(u, false);
        self.sup.stop(u, StopReason::Disable, Then::Inactive)
    }
    pub fn failed_open(&self) -> bool {
        self.sup.failed_open()
    }
    pub fn standby(&mut self, u: UnitId) -> JobId {
        self.sup.standby(u)
    }
    /// A package RPC (`JobMode::Fail`, `origin: PackageRpc`).
    pub fn rpc(&mut self, u: UnitId, kind: JobKind) -> Result<JobId, Busy> {
        self.sup.enqueue_job(
            u,
            kind,
            JobMode::Fail,
            Priority::Interactive,
            Origin::PackageRpc,
        )
    }
    pub fn rpc_stop(&mut self, u: UnitId, reason: StopReason, then: Then) -> Result<JobId, Busy> {
        self.rpc(u, JobKind::Stop { reason, then })
    }
    pub fn rpc_restart(
        &mut self,
        u: UnitId,
        fresh: bool,
        image: Option<&str>,
    ) -> Result<JobId, Busy> {
        self.rpc(
            u,
            JobKind::Restart {
                fresh,
                image: image.map(|i| i.to_string()),
            },
        )
    }
    pub fn wake(&mut self, pid: Pid, t: Trigger<u32>) -> Delivery {
        self.sup.set_now(self.now);
        self.sup.wake(pid, &t)
    }
    /// A message sent to running `pid` outside `wake` (`delivering`).
    pub fn delivering(&mut self, pid: Pid) {
        self.sup.set_now(self.now);
        self.sup.delivering(pid)
    }
    pub fn kill(&mut self, u: UnitId) -> Vec<Act> {
        self.sup.kill(u);
        self.run()
    }
    pub fn reset_failed(&mut self, u: UnitId) {
        self.sup.reset_failed(u)
    }
    pub fn set_enabled(&mut self, u: UnitId, on: bool) {
        self.sup.set_enabled(u, on)
    }
    pub fn remove(&mut self, u: UnitId) {
        self.sup.remove_unit(u)
    }
    pub fn shutdown(&mut self, budget: Ms) -> Vec<Act> {
        let acts = self.sup.shutdown(self.now, budget);
        self.keep(acts)
    }

    // events (each followed by `run_jobs(now)`)
    pub fn spawned(&mut self, pid: Pid, ok: bool) -> Vec<Act> {
        self.sup.process_spawned(
            pid,
            if ok {
                Ok(())
            } else {
                Err("spawn failed".into())
            },
        );
        self.run()
    }
    /// The spawn of `u` fails, and so does every start retry
    /// (`Policy::start_retry`) after it; every action of them.
    pub fn spawn_fails(&mut self, u: UnitId) -> Vec<Act> {
        let mut acts = Vec::new();
        loop {
            let pid = self.pid(u);
            acts.extend(self.spawned(pid, false));
            match self.st(u) {
                S::AutoRestart(until) => acts.extend(self.at(until)),
                _ => return acts,
            }
        }
    }
    /// Past the restart hold of `u` in standby, if it has one.
    pub fn past_hold(&mut self, u: UnitId) -> Vec<Act> {
        match self.st(u) {
            S::Standby {
                hold_until: Some(until),
                ..
            } if until > self.now => self.at(until),
            _ => Vec::new(),
        }
    }
    pub fn verified(&mut self, u: UnitId, ok: bool) -> Vec<Act> {
        self.sup.verify_done(u, ok);
        self.run()
    }
    pub fn ready(&mut self, pid: Pid) -> Vec<Act> {
        self.sup.process_ready(pid);
        self.run()
    }
    pub fn ui_begin(&mut self, pid: Pid) {
        self.sup.ui_begin(pid)
    }
    pub fn ui_finish(&mut self, pid: Pid) -> Vec<Act> {
        self.sup.ui_finish(pid);
        self.run()
    }
    pub fn failed(&mut self, pid: Pid) -> Vec<Act> {
        self.sup.process_failed(pid, "cannot start".into());
        self.run()
    }
    pub fn exited(&mut self, pid: Pid, cause: ExitCause) -> Vec<Act> {
        self.sup.process_exited(pid, cause, None);
        self.run()
    }
    pub fn reaped(&mut self, pid: Pid) -> Vec<Act> {
        self.sup.process_reaped(pid, None);
        self.run()
    }
    pub fn cleaned(&mut self, pid: Pid) -> Vec<Act> {
        self.sup.backend_cleaned(pid);
        self.run()
    }
    /// Both halves of reaped and backend cleaned.
    pub fn gone(&mut self, pid: Pid) -> Vec<Act> {
        let mut a = self.reaped(pid);
        a.extend(self.cleaned(pid));
        a
    }
    pub fn resources(&mut self, pid: Pid, busy: bool) -> Vec<Act> {
        self.sup.resources_answered(pid, busy);
        self.run()
    }
    pub fn classified(&mut self, pid: Pid, resident: bool) -> Vec<Act> {
        self.sup
            .classified(pid, resident.then(|| "live resource".to_string()));
        self.run()
    }
    pub fn touch(&mut self, pid: Pid) {
        self.sup.touch(pid, self.now)
    }
    pub fn begin_work(&mut self, pid: Pid) {
        self.sup.begin_work(pid, WorkKind::Request(1), self.now)
    }
    pub fn end_work(&mut self, pid: Pid) {
        self.sup.end_work(pid, WorkKind::Request(1))
    }
    /// The shared runner hangs (the core allocates runner ids; the test's
    /// runner number only says "shared").
    pub fn runner_hung(&mut self, _runner: u32) -> Vec<Act> {
        let r = self.sup.runner();
        self.sup.runner_failed(r, ExitCause::Fault);
        self.run()
    }
    pub fn pressure_low(&mut self) -> Vec<Act> {
        self.sup.memory_pressure(Pressure::Low);
        self.run()
    }
    pub fn pressure_critical(&mut self) -> Vec<Act> {
        self.sup.memory_pressure(Pressure::Critical);
        self.run()
    }
    pub fn next_deadline(&self) -> Option<Ms> {
        self.sup.next_deadline()
    }
}

/// The core's presets carry the design's numbers.
#[test]
fn t00_presets_match_design_values() {
    let p = Profile::target();
    let t = Tunables::new(8, PHYS_MIB);
    for c in [Class::Builtin, Class::Resident, Class::Normal] {
        let n = t.policy(c);
        assert_eq!(n.start_timeout, p.start_timeout, "{c:?}");
        assert_eq!(n.stop_grace, p.stop_grace, "{c:?}");
        assert_eq!(n.standby_grace, p.standby_grace, "{c:?}");
        assert_eq!(n.failure_grace, p.failure_grace, "{c:?}");
        assert_eq!(n.request_timeout, p.request_timeout, "{c:?}");
        let retries = n.start_retry.map_or(0, |r| r.count);
        assert_eq!(retries, p.start_retries, "{c:?}");
        assert_ne!(n.restart, RestartMode::No, "{c:?}");
    }
    assert_eq!(t.idle_timeout, p.idle);
    assert_eq!(t.stop_wait, STOP_WAIT);
    assert_eq!(t.verify_wait, VERIFY_WAIT);
    assert_eq!(t.starting_notice, STARTING_OSD);
    assert_eq!(t.quiet_after_start, QUIET_AFTER_START);
    assert_eq!(t.first_run_quiet, FIRST_RUN_QUIET);
    assert_eq!(t.backlog_max, p.backlog_max);
    assert_eq!(t.lane_max, p.lane_max);
    assert_eq!(t.interactive_reserve, p.reserve);
    assert_eq!(t.max_active, p.max_active);
    assert_eq!(t.policy(Class::Development).restart, RestartMode::No);
    assert_eq!(max_active_for(8), 4);
    assert_eq!(max_active_for(2), 2);
    assert_eq!(max_active_for(64), 4);
    // clamp(physMiB / 2 × 0.30 / 85, 3, 16).
    assert_eq!(process_budget_for(4_096), 7);
    assert_eq!(process_budget_for(1_024), 3);
    assert_eq!(process_budget_for(65_536), 16);
}

// ---------------------------------------------------------------------------
// Scenario helpers (built only on the Sim API above)
// ---------------------------------------------------------------------------

impl Sim {
    /// Adds the unit and runs; returns it in `Standby` (needs a standby
    /// checkpoint) with its holder pid.
    pub fn in_standby(&mut self, s: Spec) -> (UnitId, Pid) {
        let u = self.add(s);
        self.run();
        match self.st(u) {
            S::Standby { holder, .. } => (u, holder),
            other => panic!("expected Standby after add_unit, got {other:?}"),
        }
    }

    /// Brings a new unit to `Active` and returns it with its pid. A unit
    /// with a standby checkpoint is started from `Standby` by an
    /// Interactive start (not a first run, so no classification pending);
    /// any other unit by its boot Start job.
    pub fn in_active(&mut self, s: Spec) -> (UnitId, Pid) {
        let u = self.add(s);
        self.run();
        if let S::Standby { .. } = self.st(u) {
            self.start_interactive(u).expect("start from standby");
            self.run();
        }
        self.bring_up(u)
    }

    /// From `Activating{Spawn}` (or `Verify`) to `Active`.
    pub fn bring_up(&mut self, u: UnitId) -> (UnitId, Pid) {
        if self.st(u) == S::Verify {
            self.verified(u, true);
        }
        let pid = match self.st(u) {
            S::Spawn(pid) => pid,
            other => panic!("expected Activating{{Spawn}}, got {other:?}"),
        };
        self.spawned(pid, true);
        self.ready(pid);
        assert_eq!(
            self.st(u),
            S::Active(pid),
            "unit should be Active after ready"
        );
        (u, pid)
    }

    /// Active unit whose idle timer fires and that answers "not busy": returns
    /// the actions of the answer.
    pub fn idle_out(&mut self, pid: Pid) -> Vec<Act> {
        let due = self.now + self.p.idle;
        let q = self.at(due);
        assert!(
            q.iter().any(|a| query_pid(a) == Some(pid)),
            "idle timer should query resources: {q:?}"
        );
        self.resources(pid, false)
    }

    /// Brings an Active normal unit to `Deactivating{then: Standby}`.
    pub fn deactivating_to_standby(&mut self, s: Spec) -> (UnitId, Pid) {
        let (u, pid) = self.in_active(s);
        self.idle_out(pid);
        assert_eq!(self.st(u).then(), Some(Then::Standby));
        (u, pid)
    }
}

// ---------------------------------------------------------------------------
// Action list helpers
// ---------------------------------------------------------------------------

pub(super) fn names(acts: &[Act]) -> Vec<&'static str> {
    acts.iter().map(act_name).collect()
}
pub(super) fn has(acts: &[Act], name: &str) -> bool {
    acts.iter().any(|a| act_name(a) == name)
}
pub(super) fn count(acts: &[Act], name: &str) -> usize {
    acts.iter().filter(|a| act_name(a) == name).count()
}
/// Index of the first action named `name`.
pub(super) fn pos(acts: &[Act], name: &str) -> Option<usize> {
    acts.iter().position(|a| act_name(a) == name)
}
pub(super) fn spawns(acts: &[Act]) -> Vec<Pid> {
    acts.iter().filter_map(spawn_pid).collect()
}
pub(super) fn deliveries(acts: &[Act]) -> Vec<u32> {
    acts.iter().filter_map(delivered).map(|(_, p)| p).collect()
}
pub(super) fn answers(acts: &[Act]) -> Vec<(u32, Why)> {
    acts.iter().filter_map(answered).collect()
}
/// Same elements, any order (needs only `PartialEq` on `Pid`).
pub(super) fn same_set(a: &[Pid], b: &[Pid]) -> bool {
    a.len() == b.len() && a.iter().all(|x| b.contains(x)) && b.iter().all(|x| a.contains(x))
}
pub(super) fn terminates(acts: &[Act]) -> Vec<(Ms, bool, String, Pid)> {
    acts.iter().filter_map(terminate).collect()
}

// ---------------------------------------------------------------------------
// Adapter additions for the start-limit spec tests (`b3.rs`)
// ---------------------------------------------------------------------------

/// The unit of a "Starting <name>…" notice.
pub(super) fn starting_notice_unit(a: &Act) -> Option<UnitId> {
    match a {
        Action::Notify {
            unit,
            notice: Notice::Starting,
        } => *unit,
        _ => None,
    }
}

/// The unit of a "<name> could not start" notice.
pub(super) fn could_not_start_unit(a: &Act) -> Option<UnitId> {
    match a {
        Action::Notify {
            unit,
            notice: Notice::CouldNotStart,
        } => *unit,
        _ => None,
    }
}

/// `(unit, pid)` of a `Revoke`.
pub(super) fn revoked(a: &Act) -> Option<(UnitId, Pid)> {
    match a {
        Action::Revoke { unit, pid } => Some((*unit, *pid)),
        _ => None,
    }
}

/// A plugin-declared long work kind (`commands.busy`, a settings page busy):
/// `WorkKind::Declared`, never capped (coordinator ruling).
/// The number is the adapter's own; see
/// `b3::w_declared_busy_has_no_request_cap`.
pub(super) const DECLARED_BUSY: WorkKind = WorkKind::Declared(0);

impl Sim {
    /// Third-party start slots in use (`slots_in_use`).
    pub fn slots(&self) -> usize {
        self.sup.slots_in_use()
    }
    /// `begin_work` of a host→plugin request with the adapter's id
    /// (`WorkKind::Request`).
    pub fn begin_work_kind(&mut self, pid: Pid, kind: u64) {
        self.sup.set_now(self.now);
        self.sup.begin_work(pid, WorkKind::Request(kind), self.now)
    }
    pub fn end_work_kind(&mut self, pid: Pid, kind: u64) {
        self.sup.set_now(self.now);
        self.sup.end_work(pid, WorkKind::Request(kind))
    }
    /// Plugin-declared long work (`commands.busy`): see `DECLARED_BUSY`.
    pub fn begin_declared_busy(&mut self, pid: Pid) {
        self.sup.set_now(self.now);
        self.sup.begin_work(pid, DECLARED_BUSY, self.now)
    }
    pub fn end_declared_busy(&mut self, pid: Pid) {
        self.sup.set_now(self.now);
        self.sup.end_work(pid, DECLARED_BUSY)
    }
    /// The identity input `activation_begin` for `n` fresh units, each with
    /// a new pid: how many were accepted (no limit of 64).
    pub fn activations_accepted(&mut self, n: usize) -> usize {
        (0..n)
            .filter(|i| {
                let u = self.sup.load_unit(&format!("act.fixture-{i:03}"));
                let pid = self.sup.alloc_pid();
                self.sup.activation_begin(u, pid, self.now + 15_000).is_ok()
            })
            .count()
    }
    /// The cause of the unit's running Start job (`J::Start` view), if any.
    pub fn running_start_cause(&self, u: UnitId) -> Option<String> {
        match self.job(u) {
            Some(J::Start {
                cause,
                running: true,
            }) => Some(cause),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Adapter additions for the restart-policy spec tests (`b4.rs`)
// ---------------------------------------------------------------------------

impl Sim {
    pub fn with_tunables(p: &Profile, t: Tunables) -> Sim {
        let mut sup = Supervisor::new(t, 1);
        sup.set_now(T0);
        Sim {
            sup,
            now: T0,
            p: p.clone(),
            log: Vec::new(),
        }
    }
    /// `process_exited` with the fault's own text.
    pub fn exited_with(&mut self, pid: Pid, cause: ExitCause, text: &str) -> Vec<Act> {
        self.sup.process_exited(pid, cause, Some(text.into()));
        self.run()
    }
    /// `packages.list` state (`list_state`): `Some(text)` when failed.
    pub fn listed_failed(&self, u: UnitId) -> Option<String> {
        match self.sup.list_state(u) {
            Some(ListState::Failed(text)) => Some(text),
            _ => None,
        }
    }
    pub fn package_busy(&self, u: UnitId) -> bool {
        self.sup.package_busy(u)
    }
}

/// `until` of an `Answer{Held{until}}`.
pub(super) fn held_until(a: &Act) -> Option<Ms> {
    match a {
        Action::Answer {
            reason: AnswerReason::Held { until },
            ..
        } => Some(*until),
        _ => None,
    }
}

/// A notice, normalised (`crashedexited`, `crashedoom`, `startlimithit`,
/// `runnerrestarted`, `runnerfailedopen`, `couldnotstart`, `starting`).
pub(super) fn notice(a: &Act) -> Option<String> {
    match a {
        Action::Notify { notice, .. } => Some(norm(notice)),
        _ => None,
    }
}
pub(super) fn notices(acts: &[Act]) -> Vec<String> {
    acts.iter().filter_map(notice).collect()
}

/// `(pid, own process)` of a `Spawn`.
pub(super) fn spawn_own(a: &Act) -> Option<(Pid, bool)> {
    match a {
        Action::Spawn { pid, placement, .. } => Some((*pid, *placement == Placement::Own)),
        _ => None,
    }
}

/// `(unit, job, result)` of a `JobFinished`.
pub(super) fn finished(a: &Act) -> Option<(UnitId, JobId, R)> {
    match a {
        Action::JobFinished {
            unit, job, result, ..
        } => Some((
            *unit,
            *job,
            match result {
                JobResult::Done => R::Done,
                JobResult::Failed(t) => R::Failed(t.clone()),
                JobResult::Canceled => R::Canceled,
                JobResult::Dropped => R::Dropped,
            },
        )),
        _ => None,
    }
}

/// The preset's restart policy carries the design's numbers.
#[test]
fn t00_restart_policy_matches_design_values() {
    let t = Tunables::new(8, PHYS_MIB);
    for c in [Class::Builtin, Class::Resident, Class::Normal] {
        let p = t.policy(c);
        // Start retry 3 × (250 ms × 2ⁿ + 10 % jitter).
        let r = p.start_retry.expect("start retry");
        assert_eq!((r.count, r.base, r.jitter_pct), (3, 250, 10), "{c:?}");
        // 5 in 10 min.
        let l = p.start_limit.expect("start limit");
        assert_eq!((l.burst, l.interval), (5, 600_000), "{c:?}");
        // The heap cap: 512 MB for an own process.
        assert_eq!(p.memory_max_mib, Some(512), "{c:?}");
        // Reset after 60 s stable.
        let b = p.restart_sec.expect("restart delay");
        assert_eq!(b.reset_after, Some(60_000), "{c:?}");
        assert!(
            !b.first_immediate,
            "{c:?}: only the runner's first restart is immediate"
        );
    }
    let b = t.policy(Class::Builtin);
    assert_eq!(b.restart, RestartMode::OnFailure);
    assert_eq!(b.restart_sec.unwrap().base, 3_000);
    let r = t.policy(Class::Resident);
    assert_eq!(r.restart, RestartMode::OnFailure, "Resident");
    assert_eq!(r.restart_sec.unwrap().base, 1_000);
    assert_eq!(r.restart_sec.unwrap().cap, Some(60_000));
    let n = t.policy(Class::Normal);
    assert_ne!(
        n.restart,
        RestartMode::OnFailure,
        "Normal: on the next trigger"
    );
    assert_ne!(n.restart, RestartMode::No);
    assert_eq!(n.restart_sec.unwrap().base, 1_000);
    assert_eq!(n.restart_sec.unwrap().cap, Some(60_000));
    let d = t.policy(Class::Development);
    assert_eq!(d.restart, RestartMode::No);
    assert_eq!(d.start_retry.map_or(0, |r| r.count), 0, "no start retry");
    assert_eq!(
        d.start_limit.map(|l| l.burst),
        Some(1),
        "1 failure → Failed"
    );
    assert_eq!(d.memory_max_mib, None, "developers debug large heaps");
    // Runner: 3 restarts in 60 s → fail-open; first immediate, then 3 s.
    assert_eq!(
        (t.runner_restart_limit, t.runner_restart_window),
        (3, 60_000)
    );
    let rb = t.runner_backoff;
    assert!(rb.first_immediate);
    assert_eq!(rb.base, 3_000);
}
