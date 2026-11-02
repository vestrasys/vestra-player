//! Units (systemd `Unit` + service settings): one plugin each, its
//! identities, its state (doc/architecture/supervisor.md §3.1) and the
//! static facts `add_unit` is given.
#![forbid(unsafe_code)]

use std::collections::VecDeque;

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

/// One process run: today's owner generation, the identity on the wire and
/// in every registry (A10). New on every start.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pid(pub u64);

/// One shared-runner process; a new id after every runner restart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RunnerId(pub u64);

/// §3.3. `Resident` is observed at the first run (`classified`), never
/// declared.
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

/// systemd `UnitFileState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Enablement {
    Enabled,
    Disabled,
}

/// The 7 wire stop reasons (`process::StopReason`, A14).
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

/// Where a `Deactivating` unit lands when its process is gone (§3.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Then {
    Inactive,
    Standby,
    Failed,
    Removed,
}
impl Then {
    /// Merge order of two stops (§4.2): `Removed` > `Failed` > `Inactive`
    /// (> `Standby`, which keeps the most).
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

/// `Activating` sub-states (§3.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivatingSub {
    /// Built-in digest check, no process yet (systemd `start-pre`).
    Verify,
    /// Process creation in flight.
    Spawn,
    /// Waiting for `runtime.ready` + UI activation (systemd `Type=notify`).
    Ready,
    /// No process; waiting `RestartSec` (systemd `activating (auto-restart)`).
    AutoRestart { until: Ms },
}

/// `Deactivating` sub-states (§3.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeactivatingSub {
    /// Grace running (systemd `stop-sigterm`).
    Term,
    /// Kill sent (systemd `stop-sigkill`).
    Kill,
    /// Process gone, waiting for `backend_cleaned` (systemd `stop-post`).
    Cleanup,
}

/// Why a unit is `Failed` (systemd `Result=`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailResult {
    /// The process could not be created (`process_spawned(Err)`).
    Spawn,
    /// The process ended or was ended (§7.3).
    Exit(ExitCause),
    /// `StartLimitBurst` hit (T27).
    StartLimitHit,
    /// The previous process did not go within `STOP_WAIT` (T18).
    Stuck,
}

/// §3.1.
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
    /// systemd's `ActiveState` word.
    pub fn name(&self) -> &'static str {
        match self {
            UnitState::Inactive => "inactive",
            UnitState::Activating { .. } => "activating",
            UnitState::Active { .. } => "active",
            UnitState::Deactivating { .. } => "deactivating",
            UnitState::Failed { .. } => "failed",
            UnitState::Standby { .. } => "standby",
        }
    }
    pub fn is_deactivating(&self) -> bool {
        matches!(self, UnitState::Deactivating { .. })
    }
}

/// `packages.list` per unit (§9; output kept, `process` excluded from the
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
    /// Standby (wire value `idle`, §3.1 "Naming").
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
    /// Display name for answers and notices.
    pub name: String,
    pub class: Class,
    pub enabled: bool,
    /// Trusted for the shared runner (a built-in member).
    pub shared: bool,
    /// A built-in whose digest is checked before its first start (T4).
    pub verify: bool,
    pub checkpoint: Option<Checkpoint>,
    /// Development `--plugin` start timeout (1–120 s); `None`: the class's.
    pub start_timeout: Option<Ms>,
}
impl UnitSpec {
    pub fn new(id: impl Into<String>, class: Class) -> Self {
        let id = id.into();
        Self {
            name: id.clone(),
            id,
            class,
            enabled: true,
            shared: false,
            verify: false,
            checkpoint: None,
            start_timeout: None,
        }
    }
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
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
}

/// Reap bookkeeping of the process a `Deactivating` unit waits for
/// (systemd "control group empty" + `ExecStopPost=`; REPLACE `Host.cleanup`:
/// it exists only while the unit is deactivating, so it cannot grow).
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

/// One plugin (§3). Read through `Supervisor::unit`; only the supervisor
/// changes it.
#[derive(Debug)]
pub struct Unit<P> {
    pub id: String,
    pub name: String,
    pub class: Class,
    pub enablement: Enablement,
    pub state: UnitState,
    /// At most one installed job (§4).
    pub job: Option<Job>,
    /// Held triggers (the listening socket's backlog).
    pub backlog: VecDeque<Trigger<P>>,
    /// It has a valid checkpoint (its entry points are known).
    pub recorded: bool,
    /// It held a live resource at its first ready (never idle standby).
    pub resident: bool,
    /// Placement of the current/last run (`Shared` carries that runner).
    pub placement: Placement,
    /// `packages.list` error text of `Failed` (today's `failures`).
    pub failure: Option<String>,
    /// Landed after `STOP_WAIT` without reap/cleanup (T18).
    pub stuck: bool,
    /// Stopped with `then: Removed`; waiting for `remove_unit`.
    pub removed: bool,
    pub stopping: Option<Stopping>,
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
    /// Previous pid, forwarded to the current one (REPLACE `lazy.moved`).
    pub(super) alias: Option<Pid>,
    /// Last pid this unit had (kept in `by_pid` for late replies).
    pub(super) last_pid: Option<Pid>,
    pub(super) runtime_ready: bool,
    pub(super) pending_ui: u32,
    pub(super) classifying: bool,
    /// Holds a first-run place until classified resident or landed.
    pub(super) first_run: bool,
    /// The run came from `Standby` (`Prune` at ready).
    pub(super) from_standby: bool,
    pub(super) asked_at: Option<Ms>,
    pub(super) failing: Option<(FailResult, String)>,
    pub(super) hold_next: Option<Ms>,
    pub(super) work: Work,
    pub(super) restart: RestartState,
    pub(super) gens: Gens,
    pub(super) results: VecDeque<(JobId, JobResult)>,
    pub(super) listed: Option<ListState>,
}
impl<P> Unit<P> {
    pub(super) fn new(spec: &UnitSpec) -> Self {
        Self {
            id: spec.id.clone(),
            name: spec.name.clone(),
            class: spec.class,
            enablement: if spec.enabled {
                Enablement::Enabled
            } else {
                Enablement::Disabled
            },
            state: UnitState::Inactive,
            job: None,
            backlog: VecDeque::new(),
            recorded: spec.checkpoint.is_some(),
            resident: spec.checkpoint.is_some_and(|c| c.resident),
            placement: Placement::Own,
            failure: None,
            stuck: false,
            removed: false,
            stopping: None,
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
            first_run: false,
            from_standby: false,
            asked_at: None,
            failing: None,
            hold_next: None,
            work: Work::default(),
            restart: RestartState::default(),
            gens: Gens::default(),
            results: VecDeque::new(),
            listed: None,
        }
    }
    /// The pid of its process, if it has one.
    pub fn pid(&self) -> Option<Pid> {
        self.state.pid()
    }
    pub fn enabled(&self) -> bool {
        self.enablement == Enablement::Enabled
    }
    /// In-flight work (`begin_work` without `end_work`).
    pub fn busy(&self) -> bool {
        self.work.busy()
    }
    pub fn last_active(&self) -> Option<Ms> {
        match self.state {
            UnitState::Active { last_active, .. } => Some(last_active),
            _ => None,
        }
    }
}
