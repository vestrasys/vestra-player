//! Spec tests for the pure supervisor core, written from the approved
//! design (`doc/architecture/supervisor.md`) without reading the
//! implementation. Each test is named after the design row it checks
//! (`t19_…` = transition table §3.2 row T19, `m_<row>_<col>` = merge table
//! §4.2 cell, `nc_…` = named case §4.3, …).
//!
//! The design's API (§5) is written as text, not Rust signatures. Every
//! guess about the concrete Rust shape (type and field names, argument
//! types, how a finished job's result is read) is confined to the
//! **adapter** section of this file, so that only this file needs mapping
//! if the core spells something differently. The test files themselves
//! talk only to `Sim`, `Spec`, `Profile`, `S`, `J` and the action helpers.
//!
//! Two tunable profiles (§11): `Profile::b1()` reproduces today's values
//! (B1–B2: idle 60 s, start 15 s, stop 300 ms / standby 2 s, `max_active`
//! 8 for first runs only, unlimited wakes/backlog/budget, no retry, no
//! restart policy); `Profile::target()` is the end state (B3–B6 values:
//! idle 30 s, start 60 s, stop 5 s, `max_active` 4 + 1 Interactive
//! reserve, backlog 64, lane 256, start retry 3, per-class restart policy,
//! request timeout 5 min, a process budget). Tests whose expected values
//! differ between the two are generated for both with `both!`.
//!
//! Lines marked `// DESIGN QUESTION:` are places where the design is
//! ambiguous or contradictory; the test encodes the author's best reading.
#![forbid(unsafe_code)]
#![allow(dead_code)]

use std::fmt::Debug;

use super::*;

pub(super) type Ms = u64;

/// The time every simulation starts at (not 0, so "armed at 0" bugs show).
pub(super) const T0: Ms = 1_000;
/// `STOP_WAIT` (§4.5, §6.2): one constant for every "wait for the old
/// process" deadline.
pub(super) const STOP_WAIT: Ms = 20_000;
/// `VERIFY_WAIT` (T4/T5, A6).
pub(super) const VERIFY_WAIT: Ms = 3_000;
/// `STARTING_OSD_MS` (T7, §4.5).
pub(super) const STARTING_OSD: Ms = 400;
/// `QUIET_AFTER_START_MS` (§4.4 "Event wakes").
pub(super) const QUIET_AFTER_START: Ms = 1_000;
/// Text of a Start that waited on a stop job (reload, T18).
pub(super) const TEXT_RELOAD_STUCK: &str = "the previous process did not stop within 20 s";
/// Text of a wake that waited on a standby stop (T18, §4.3).
pub(super) const TEXT_WAKE_STUCK: &str = "its stopped process did not go within 20 s";

// ---------------------------------------------------------------------------
// Profiles (§11 Tunables)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub(super) struct Profile {
    pub name: &'static str,
    /// Idle timeout (§6.2): 60 s → 30 s.
    pub idle: Ms,
    /// `TimeoutStartSec` installed plugins: 15 s → 60 s.
    pub start_timeout: Ms,
    /// `TimeoutStopSec` for disable/reload/uninstall/upgrade: 300 ms → 5 s.
    pub stop_grace: Ms,
    /// `TimeoutStopSec` for standby: 2 s (`PARK_WAIT`) → 5 s.
    pub standby_grace: Ms,
    /// `TimeoutStopSec` after a failure: 300 ms in both (KEEP).
    pub failure_grace: Ms,
    /// Third-party running Start jobs limit.
    pub max_active: usize,
    /// Whether `max_active` limits wakes too (B3) or only first runs (B1).
    pub limit_wakes: bool,
    /// Interactive reserve slots (+1 from B3, 0 before).
    pub reserve: usize,
    /// `backlog_max`: None = ∞ (B1), 64 (B3).
    pub backlog_max: Option<usize>,
    /// Third-party lane waiting Start jobs: None = ∞ (B1), 256 (B3).
    pub lane_max: Option<usize>,
    /// Start retries within one Start job: 0 (B1), 3 (B4).
    pub start_retries: u32,
    /// Per-class `Restart=`, backoff and start limits (B4). B1: `Restart=no`
    /// and revoke for every class.
    pub restart: bool,
    /// Request timeout: None (off) until B6, 5 min at B6.
    pub request_timeout: Option<Ms>,
    /// Third-party process budget: None = ∞ until B5.
    pub budget: Option<usize>,
}

impl Profile {
    pub fn b1() -> Self {
        Profile {
            name: "b1",
            idle: 60_000,
            start_timeout: 15_000,
            stop_grace: 300,
            standby_grace: 2_000,
            failure_grace: 300,
            max_active: 8,
            limit_wakes: false,
            reserve: 0,
            backlog_max: None,
            lane_max: None,
            start_retries: 0,
            restart: false,
            request_timeout: None,
            budget: None,
        }
    }
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
            limit_wakes: true,
            reserve: 1,
            backlog_max: Some(64),
            lane_max: Some(256),
            start_retries: 3,
            restart: true,
            request_timeout: Some(300_000),
            budget: Some(3),
        }
    }
}

/// Generates one test per profile: `name::b1` and `name::target`.
macro_rules! both {
    ($(#[$m:meta])* fn $name:ident($p:ident) $body:block) => {
        $(#[$m])*
        mod $name {
            #[allow(unused_imports)]
            use super::*;
            #[allow(unused_variables)]
            fn run($p: &Profile) $body
            #[test]
            fn b1() {
                run(&Profile::b1())
            }
            #[test]
            fn target() {
                run(&Profile::target())
            }
        }
    };
}

// Test files come after `both!` (macro_rules is textually scoped).
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

/// Unit state flattened with its sub-state (§3.1).
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

/// Installed job, flattened (§4.1).
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

/// Finished job result (§4: `done`, `failed`, `canceled`, `timeout`).
#[derive(Clone, Debug, PartialEq)]
pub(super) enum R {
    Done,
    Failed(String),
    Canceled,
    Timeout,
}

/// Why a held trigger was answered (§4.6).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Why {
    Busy,
    CouldNotStart,
    Disabled,
    Other,
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
    /// Built-in needing a digest check (T4).
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
    Spec::new(id, Class::Resident, Ck::Resident)
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
// ADAPTER: the only place that names the core's concrete Rust shapes.
// ---------------------------------------------------------------------------

pub(super) type Act = Action<u32>;

/// §11 Tunables from a profile.
// DESIGN QUESTION: §11 lists values but not field names, and does not say
// how B1's "`Restart=no` + revoke for every class" and B3's "limits live"
// are expressed (tunables? feature switches? per-class `Policy` built from
// tunables?). This maps each profile field to a guessed tunable.
fn tunables(p: &Profile) -> Tunables {
    Tunables {
        idle_timeout: p.idle,
        timeout_start: p.start_timeout,
        timeout_stop: p.stop_grace,
        timeout_stop_standby: p.standby_grace,
        timeout_stop_failed: p.failure_grace,
        stop_wait: STOP_WAIT,
        verify_wait: VERIFY_WAIT,
        max_active: p.max_active,
        max_active_wakes: p.limit_wakes,
        interactive_reserve: p.reserve,
        backlog_max: p.backlog_max,
        lane_max: p.lane_max,
        start_retries: p.start_retries,
        restart_policy: p.restart,
        request_timeout: p.request_timeout,
        process_budget: p.budget,
        ..Tunables::default()
    }
}

fn unit_spec(s: &Spec) -> UnitSpec {
    UnitSpec {
        id: s.id.clone(),
        class: s.class,
        placement: match s.runner {
            Some(r) => Placement::Shared(RunnerId(r)),
            None => Placement::Own,
        },
        enablement: if s.enabled {
            Enablement::Enabled
        } else {
            Enablement::Disabled
        },
        checkpoint: match s.ck {
            Ck::None => None,
            Ck::Standby => Some(false),
            Ck::Resident => Some(true),
        },
        verify: s.verify,
        dev_timeout: s.dev_timeout,
    }
}

fn state_view(s: &UnitState) -> S {
    match s {
        UnitState::Inactive => S::Inactive,
        UnitState::Activating { pid, sub, .. } => match sub {
            ActivatingSub::Verify => S::Verify,
            ActivatingSub::Spawn => S::Spawn(*pid),
            ActivatingSub::Ready => S::Ready(*pid),
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
    let running = norm(&j.state) == "running";
    match &j.kind {
        JobKind::Start { cause } => J::Start {
            cause: norm(cause),
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
    Trigger::Command(p)
}
/// `source.resolveRequest` trigger: Interactive.
pub(super) fn resolve(p: u32) -> Trigger<u32> {
    Trigger::Resolve(p)
}
/// `drops.drop` trigger: Interactive.
pub(super) fn drop_(p: u32) -> Trigger<u32> {
    Trigger::Drop(p)
}
/// `ui.callback` trigger: Interactive.
pub(super) fn callback(p: u32) -> Trigger<u32> {
    Trigger::Callback(p)
}
/// Wake event broadcast (`WAKE_EVENTS`): Event priority.
pub(super) fn event(name: &str, p: u32) -> Trigger<u32> {
    Trigger::Event(name.into(), p)
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
        Action::Deliver { pid, trigger, .. } => Some((*pid, *trigger.payload())),
        _ => None,
    }
}
/// `(payload, why)` of an `Answer`.
pub(super) fn answered(a: &Act) -> Option<(u32, Why)> {
    match a {
        Action::Answer { trigger, why, .. } => {
            let w = norm(why);
            let why = if w.contains("busy") {
                Why::Busy
            } else if w.contains("couldnotstart") || w.contains("start") {
                Why::CouldNotStart
            } else if w.contains("disabled") {
                Why::Disabled
            } else {
                Why::Other
            };
            Some((*trigger.payload(), why))
        }
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

pub(super) struct Sim {
    pub sup: Supervisor<u32>,
    pub now: Ms,
    pub p: Profile,
    /// Every action returned so far, in order.
    pub log: Vec<Act>,
}

impl Sim {
    pub fn new(p: &Profile) -> Sim {
        Sim {
            sup: Supervisor::new(tunables(p)),
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
    /// Start-up is over (§4.4 "Event wakes": muted during start-up and
    /// `QUIET_AFTER_START_MS` after settle).
    // DESIGN QUESTION: §5 has no input that tells the core start-up has
    // settled (`lazy_settled`/`Lazy::mute` today). Best reading: the core
    // settles itself once every Boot/FirstRun job is done; this waits the
    // quiet second after that.
    pub fn settle(&mut self) -> Vec<Act> {
        self.after(QUIET_AFTER_START)
    }

    pub fn add(&mut self, s: Spec) -> UnitId {
        self.sup.add_unit(unit_spec(&s))
    }
    pub fn st(&self, u: UnitId) -> S {
        state_view(&self.sup.unit(u).state)
    }
    pub fn job(&self, u: UnitId) -> Option<J> {
        self.sup.job(u).map(job_view)
    }
    // DESIGN QUESTION: §5 says `job(u)` returns "the installed job and its
    // result", but a finished job is no longer installed (one installed job
    // per unit). Best reading: results are kept by `JobId` until read.
    pub fn result(&self, id: JobId) -> Option<R> {
        self.sup.job_result(id).map(|r| match r {
            JobResult::Done => R::Done,
            JobResult::Failed(text) => R::Failed(text.to_string()),
            JobResult::Canceled => R::Canceled,
            JobResult::Timeout => R::Timeout,
        })
    }
    pub fn class(&self, u: UnitId) -> Class {
        self.sup.unit(u).class
    }
    pub fn own_process(&self, u: UnitId) -> bool {
        matches!(self.sup.unit(u).placement, Placement::Own)
    }
    pub fn enabled(&self, u: UnitId) -> bool {
        matches!(self.sup.unit(u).enablement, Enablement::Enabled)
    }
    pub fn backlog_len(&self, u: UnitId) -> usize {
        self.sup.unit(u).backlog.len()
    }
    pub fn stuck(&self, u: UnitId) -> bool {
        self.sup.unit(u).stuck
    }
    pub fn failure_text(&self, u: UnitId) -> Option<String> {
        match &self.sup.unit(u).state {
            UnitState::Failed { text, .. } => Some(text.to_string()),
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
        match &self.sup.unit(u).state {
            UnitState::Activating { pid, .. }
            | UnitState::Active { pid, .. }
            | UnitState::Deactivating { pid, .. } => *pid,
            UnitState::Standby { holder, .. } => *holder,
            other => panic!("no pid in {other:?}"),
        }
    }

    // requests
    pub fn start(&mut self, u: UnitId, cause: StartCause) -> Result<JobId, Busy> {
        self.sup.start(u, cause)
    }
    pub fn start_interactive(&mut self, u: UnitId) -> Result<JobId, Busy> {
        self.sup
            .start(u, StartCause::Trigger(Priority::Interactive))
    }
    pub fn stop(&mut self, u: UnitId, reason: StopReason, then: Then) -> Result<JobId, Busy> {
        self.sup.stop(u, reason, then)
    }
    pub fn restart(&mut self, u: UnitId, fresh: bool, image: Option<&str>) -> Result<JobId, Busy> {
        self.sup.restart(u, fresh, image.map(|i| i.to_string()))
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
        self.sup.wake(pid, t)
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
        self.sup.process_exited(pid, cause);
        self.run()
    }
    pub fn reaped(&mut self, pid: Pid) -> Vec<Act> {
        self.sup.process_reaped(pid, Default::default());
        self.run()
    }
    pub fn cleaned(&mut self, pid: Pid) -> Vec<Act> {
        self.sup.backend_cleaned(pid);
        self.run()
    }
    /// Both halves of T16: reaped and backend cleaned.
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
        self.sup.classified(
            pid,
            if resident {
                Some("live resource".into())
            } else {
                None
            },
        );
        self.run()
    }
    pub fn touch(&mut self, pid: Pid) {
        self.sup.touch(pid, self.now)
    }
    pub fn begin_work(&mut self, pid: Pid) {
        self.sup.begin_work(pid, Default::default(), self.now)
    }
    pub fn end_work(&mut self, pid: Pid) {
        self.sup.end_work(pid, Default::default())
    }
    pub fn runner_hung(&mut self, runner: u32) -> Vec<Act> {
        self.sup.runner_failed(RunnerId(runner), RunnerFault::Hung);
        self.run()
    }
    pub fn pressure_low(&mut self) -> Vec<Act> {
        self.sup.memory_pressure(PressureLevel::Low);
        self.run()
    }
    pub fn pressure_critical(&mut self) -> Vec<Act> {
        self.sup.memory_pressure(PressureLevel::Critical);
        self.run()
    }
    pub fn next_deadline(&self) -> Option<Ms> {
        self.sup.next_deadline()
    }
}

// ---------------------------------------------------------------------------
// Scenario helpers (built only on the Sim API above)
// ---------------------------------------------------------------------------

impl Sim {
    /// Adds the unit and runs; returns it in `Standby` (needs a standby
    /// checkpoint, T1) with its holder pid.
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

    /// Active unit whose idle timer fires and that answers "not busy":
    /// returns the actions of the answer (T14).
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
