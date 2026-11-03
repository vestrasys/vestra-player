//! Units (supervisor.md §2): one plugin each, its identities,
//! its state (doc/architecture/supervisor.md) and the static facts `add_unit`
//! is given.
#![forbid(unsafe_code)]

use std::collections::VecDeque;

use super::activation::Run;
use super::exit::ExitCause;
use super::job::{Job, JobId, JobResult};
use super::restart::RestartState;
use super::timers::Gens;
use super::trigger::Trigger;
use super::work::Work;

/// Monotonic milliseconds, passed in by the caller (the core has no clock).
pub type Ms = u64;

/// Stable for the unit's life (`add_unit` → `remove_unit`); never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnitId(pub u64);

/// One process run: today's owner generation, the identity on the wire and in
/// every registry. New on every start.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pid(pub u64);

/// One shared-runner process; a new id after every runner restart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RunnerId(pub u64);

/// A unit's class (supervisor.md §2). `Resident` is observed at a ready
/// (`classified` at every ready of a recordable unit), never declared.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Class {
    Builtin,
    Resident,
    Normal,
    Development,
}

/// Where the next process of a unit runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    Own,
    Shared(RunnerId),
}

/// Whether the unit is enabled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Enablement {
    Enabled,
    Disabled,
}

/// The 7 wire stop reasons (`process::StopReason`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    Quit,
    Disable,
    Uninstall,
    Reload,
    Upgrade,
    Idle,
    Failed,
}

/// Where a `Deactivating` unit lands when its process is gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Then {
    Inactive,
    Standby,
    Failed,
    Removed,
}
impl Then {
    /// Merge order of two stops: `Removed` > `Failed` > `Inactive` (>
    /// `Standby`, which keeps the most).
    pub fn strength(self) -> u8 {
        match self {
            Then::Standby => 0,
            Then::Inactive => 1,
            Then::Failed => 2,
            Then::Removed => 3,
        }
    }
    pub fn stronger(self, other: Then) -> Then {
        if other.strength() > self.strength() {
            other
        } else {
            self
        }
    }
}

/// `Activating` sub-states.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivatingSub {
    /// Built-in digest check, no process yet.
    Verify,
    /// Process creation in flight.
    Spawn,
    /// Waiting for `runtime.ready` + UI activation.
    Ready,
    /// No process; waiting for the restart delay.
    AutoRestart { until: Ms },
}

/// `Deactivating` sub-states.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeactivatingSub {
    /// Grace running.
    Term,
    /// Kill sent.
    Kill,
    /// Process gone, waiting for `backend_cleaned`.
    Cleanup,
}

/// Why a unit is `Failed`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailResult {
    /// The process could not be created (`process_spawned(Err)`).
    Spawn,
    /// The process ended or was ended.
    Exit(ExitCause),
    /// Start limit hit.
    StartLimitHit,
    /// The previous process did not go within `STOP_WAIT`.
    Stuck,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnitState {
    Inactive,
    /// `pid` is `None` in `Verify` and `AutoRestart` (no process).
    Activating {
        pid: Option<Pid>,
        since: Ms,
        sub: ActivatingSub,
    },
    Active {
        pid: Pid,
        since: Ms,
        last_active: Ms,
    },
    Deactivating {
        pid: Pid,
        reason: StopReason,
        then: Then,
        sub: DeactivatingSub,
    },
    Failed {
        result: FailResult,
        text: String,
    },
    /// Registrations kept under `holder` (the last run's pid); no process.
    Standby {
        holder: Pid,
        recorded: bool,
        hold_until: Option<Ms>,
    },
}
impl UnitState {
    /// The pid of the process this state has (none in `Inactive`, `Failed`,
    /// `Standby`, `Activating{Verify|AutoRestart}`).
    pub fn pid(&self) -> Option<Pid> {
        match self {
            UnitState::Activating { pid, .. } => *pid,
            UnitState::Active { pid, .. } | UnitState::Deactivating { pid, .. } => Some(*pid),
            _ => None,
        }
    }
    pub fn is_deactivating(&self) -> bool {
        matches!(self, UnitState::Deactivating { .. })
    }
}

/// `packages.list` per unit (output kept, `process` excluded from the
/// `packages.changed` comparison).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListState {
    Starting,
    Running { process: ProcessState },
    Failed(String),
    Disabled,
    Stopped,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessState {
    Running,
    /// Standby (wire value `idle`, "Naming").
    Idle,
}
impl ListState {
    /// What `packages.changed` compares: everything but `process`.
    pub fn same_for_changed(&self, other: &ListState) -> bool {
        match (self, other) {
            (ListState::Running { .. }, ListState::Running { .. }) => true,
            _ => self == other,
        }
    }
}

/// A valid activation checkpoint read at `add_unit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Checkpoint {
    /// It held a live resource at its last ready (`residentAtReady`).
    pub resident: bool,
}

/// What `add_unit` is told (from `Store::record` or a development load).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitSpec {
    /// Manifest id.
    pub id: String,
    pub class: Class,
    pub enabled: bool,
    /// Trusted for the shared runner (a built-in member).
    pub shared: bool,
    /// A built-in whose digest is checked before its first start.
    pub verify: bool,
    pub checkpoint: Option<Checkpoint>,
    /// Development `--plugin` start timeout (1–120 s); `None`: the class's.
    pub start_timeout: Option<Ms>,
    /// It can have a checkpoint (today: `lazy_key` is `Some`: an installed
    /// package, or a built-in listed with a digest). An unrecordable unit is
    /// never a first run, never classified, never put in standby and never
    /// checkpointed. A development load never is.
    pub recordable: bool,
}
impl UnitSpec {
    pub fn new(id: impl Into<String>, class: Class) -> Self {
        Self {
            id: id.into(),
            class,
            enabled: true,
            shared: false,
            verify: false,
            checkpoint: None,
            start_timeout: None,
            recordable: class != Class::Development,
        }
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }
    pub fn shared(mut self) -> Self {
        self.shared = true;
        self
    }
    pub fn verify(mut self) -> Self {
        self.verify = true;
        self
    }
    pub fn checkpoint(mut self, resident: bool) -> Self {
        self.checkpoint = Some(Checkpoint { resident });
        self
    }
    pub fn start_timeout(mut self, ms: Ms) -> Self {
        self.start_timeout = Some(ms);
        self
    }
    /// No checkpoint possible (a built-in listed without a digest).
    pub fn unrecordable(mut self) -> Self {
        self.recordable = false;
        self
    }
}

/// Reap bookkeeping of the process a `Deactivating` unit waits for
/// (the process is gone, then its cleanup ran; replaces `Host.cleanup`:
/// it exists while the unit is deactivating, and after a stuck landing
/// until that process goes: one per unit, so it cannot grow).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stopping {
    pub pid: Pid,
    pub reaped: bool,
    pub backend_cleaned: bool,
    /// `STOP_WAIT` from the stop.
    pub deadline: Ms,
    /// `Terminate` kept the registrations (standby): revoked at reap if the
    /// unit does not land in `Standby` after all.
    pub kept: bool,
}
impl Stopping {
    pub fn done(&self) -> bool {
        self.reaped && self.backend_cleaned
    }
}

/// One plugin. Read through `Supervisor::unit`; only the supervisor changes it.
#[derive(Debug)]
pub struct Unit<P> {
    pub id: String,

    pub class: Class,
    pub enablement: Enablement,
    pub state: UnitState,
    /// At most one installed job.
    pub job: Option<Job>,
    /// Held triggers (the listening socket's backlog).
    pub backlog: VecDeque<Trigger<P>>,
    /// It has a valid checkpoint (its entry points are known).
    pub recorded: bool,
    /// Classified resident (at its ready, or by its checkpoint): no idle
    /// standby.
    pub resident: bool,
    /// It can have a checkpoint (`UnitSpec::recordable`).
    pub recordable: bool,
    /// Placement of the current/last run (`Shared` carries that runner).
    pub placement: Placement,
    /// `packages.list` error text of `Failed` (today's `failures`).
    pub failure: Option<String>,
    /// Landed after `STOP_WAIT` without reap/cleanup; `stopping` keeps tracking
    /// that process until it goes.
    pub stuck: bool,
    /// Stopped with `then: Removed`; waiting for `remove_unit` (or brought
    /// back by `add_unit`, a reinstall).
    pub removed: bool,
    pub stopping: Option<Stopping>,
    /// An uninstall's `RemoveData` is owed: emitted once the process that may
    /// still write the data is reaped and cleaned up after, at once without
    /// one.
    pub(super) remove_data: bool,
    pub(super) shared: bool,
    pub(super) verify: bool,
    pub(super) verifiable: bool,

    pub(super) own_this_run: bool,
    pub(super) quarantined: bool,
    pub(super) start_timeout: Option<Ms>,
    pub(super) image: Option<String>,
    /// Registrations of a standby unit between leaving `Standby` for
    /// `Verify` and the spawn that rekeys them.
    pub(super) holder: Option<Pid>,
    /// Previous pid, forwarded to the current one (replaces `lazy.moved`).
    pub(super) alias: Option<Pid>,
    /// Last pid this unit had (kept in `by_pid` for late replies).
    pub(super) last_pid: Option<Pid>,
    pub(super) runtime_ready: bool,
    pub(super) pending_ui: u32,
    pub(super) classifying: bool,

    /// The current run was started as a first run (`Start{FirstRun}`): only
    /// such a run goes to standby at once when classified lazy.
    pub(super) run_is_first: bool,
    /// The current run was classified (or its class is known from the
    /// checkpoint): it is checkpointed at shutdown (today's `lazy.classes`).
    pub(super) classified_run: bool,
    /// A ready run with a checkpoint that `classified` may still classify (once
    /// per run).
    pub(super) may_reclassify: bool,
    /// The checkpoint on disk says resident (today's `resident_at_ready`):
    /// a resident classification writes it only when it does not.
    pub(super) recorded_resident: bool,

    /// The run came from `Standby` (`Prune` at ready).
    pub(super) from_standby: bool,
    pub(super) asked_at: Option<Ms>,
    pub(super) failing: Option<(FailResult, String)>,
    /// Restart hold (`hold_until`) after a crash, measured from the reap.
    pub(super) hold_next: Option<Ms>,
    /// Delay of the next automatic start (backoff, retry), from the reap
    /// (the restart delay starts once the stop finished).
    pub(super) after_reap: Option<Ms>,
    /// After a runner restart: back to standby by replay at reap.
    pub(super) replay_after: bool,
    /// The run that ended by itself (`ExitCause::Exited`) whose crash notice
    /// waits for its reap: its exit code tells a heap abort.
    pub(super) exit_notice: Option<Pid>,
    /// The run whose stderr carried V8's heap abort line
    /// (`process_out_of_memory`).
    pub(super) out_of_memory: Option<Pid>,
    pub(super) work: Work,
    pub(super) restart: RestartState,
    pub(super) gens: Gens,
    pub(super) results: VecDeque<(JobId, JobResult)>,
    pub(super) listed: Option<ListState>,
    /// The activation of the run being started (activation.rs, formerly
    /// the Host's `Book` entry).
    pub(super) activation: Option<Run>,
}
impl<P> Unit<P> {
    pub(super) fn new(spec: &UnitSpec) -> Self {
        let recordable = spec.recordable && spec.class != Class::Development;
        Self {
            id: spec.id.clone(),

            class: spec.class,
            enablement: if spec.enabled {
                Enablement::Enabled
            } else {
                Enablement::Disabled
            },
            state: UnitState::Inactive,
            job: None,
            backlog: VecDeque::new(),
            recorded: recordable && spec.checkpoint.is_some(),
            resident: recordable && spec.checkpoint.is_some_and(|c| c.resident),
            recordable,
            placement: Placement::Own,
            failure: None,
            stuck: false,
            removed: false,
            stopping: None,
            remove_data: false,
            shared: spec.shared,
            verify: spec.verify,
            verifiable: spec.verify,

            own_this_run: false,
            quarantined: false,
            start_timeout: spec.start_timeout,
            image: None,
            holder: None,
            alias: None,
            last_pid: None,
            runtime_ready: false,
            pending_ui: 0,
            classifying: false,

            run_is_first: false,
            classified_run: false,
            may_reclassify: false,
            recorded_resident: recordable && spec.checkpoint.is_some_and(|c| c.resident),

            from_standby: false,
            asked_at: None,
            failing: None,
            hold_next: None,
            after_reap: None,
            replay_after: false,
            exit_notice: None,
            out_of_memory: None,
            work: Work::default(),
            restart: RestartState::default(),
            gens: Gens::default(),
            results: VecDeque::new(),
            listed: None,
            activation: None,
        }
    }
    /// The pid of its process, if it has one.
    pub fn pid(&self) -> Option<Pid> {
        self.state.pid()
    }
    pub fn enabled(&self) -> bool {
        self.enablement == Enablement::Enabled
    }

    pub fn last_active(&self) -> Option<Ms> {
        match self.state {
            UnitState::Active { last_active, .. } => Some(last_active),
            _ => None,
        }
    }
}
