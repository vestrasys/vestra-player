//! Host adapter of the plugin process supervisor
//! (doc/architecture/supervisor.md §2.2): executes the core's decisions and
//! feeds it the Host's events. Safe code only.
//!
//! B2a (§13): identity. At its choke points the Host tells the core's unit
//! table which process runs a plugin (`add_plugin` / `stop`:
//! `unit_started` / `unit_stopped`), allocates owner generations here
//! (`alloc_owner`) and keeps each activation in its unit. `Host.plugin_ids`
//! and `OnDemand.parked_ids` stay as `debug_assert` cross-checks of the
//! answers (B7 removes them).
//!
//! B2b (§17.9): every wait for a stopped process is the core's: a unit is
//! deactivating until the reaper's `Event::Reaped` and the playback
//! worker's `BackendCleaned` are in (`unit_reaped`, `unit_cleaned`).
//!
//! B2c (§17.10): on-demand start (lazy activation, `VESTRA_LAZY=1` until
//! B6) is the core's: when a run is idle, its class, when a lazy first run
//! goes to standby, and the wake of a unit in standby. The adapter feeds it
//! the Host's half of each decision (the playback worker's answers, the
//! Host's own busy checks, what is on disk); recordings are
//! `checkpoint.rs`, the test switches `probe.rs`.
//!
//! B2d (§17.11): every start and stop is a job. Start-up adds the installed
//! units in the start order under the per-tick start budget (`load_queued`,
//! package_manager.rs); the core replays, verifies and starts them in its
//! lanes (built-ins first, first runs at most 8 at a time). Package RPCs
//! enqueue `start` / `stop` / `restart` jobs, crashes and failed
//! activations are the core's inputs, and the adapter executes what the
//! core decides (`actions.rs`: `Spawn`, `Verify`, `Replay`, `Terminate`,
//! `RemoveData`...) with today's bodies. `packages.list` reads the core's
//! `list_state`.
//!
//! B3 (§17.12): with on-demand start on, the core runs the B3 values
//! (`with_b3`): third-party starts a few at a time, clicks first, a bounded
//! backlog whose refused triggers are answered (`actions.rs`
//! `held_unanswered`), start failures that keep their entry points. The
//! Host's busy checks are declared work in the core (`sync_work`), a
//! `source.resolveRequest` in flight a capped request (`request_sent`).
//!
//! B5 (§17.14): with on-demand start on, the process budget for this
//! machine's physical memory and the memory pressure source
//! (`memory.rs`): over the budget or under pressure, idle third-party
//! plugins go to standby through the idle timer's Standby job.
//!
//! B6 (§17.15): on-demand start is everyone's. `VESTRA_LAZY` is gone: the
//! player always runs the design's values (`Tunables::target`: idle 30 s,
//! `TimeoutStartSec` 60 s, `TimeoutStopSec` 5 s, a 5 min cap on a request
//! awaiting the plugin's reply, a resident plugin never in standby) with
//! the B3–B5 limits, the spawn thread and the memory pressure source; the
//! shared runner waits at least 30 s without a member before it exits. The
//! off path (`OnDemand::enabled` false, `Tunables::legacy()`) is no longer
//! reachable from the player; B7 removes it.
use super::revised::Revised;
use super::*;
use crate::capabilities;
use crate::supervisor::{Action, Delivery, Trigger, Tunables, UnitId, WorkKind};
use std::collections::BTreeSet;

pub(crate) mod actions;
pub(crate) mod checkpoint;
mod memory;
pub(crate) mod probe;
pub(crate) mod shutdown;
mod standby;
pub(crate) mod worker;

pub(crate) use standby::{Held, Origin};

/// How long a park waits for its process: the core's standby grace
/// (`Policy::standby_grace`), which `Action::Terminate` carries. 2 s until
/// B6 (lazy activation record §5.4 item 6), `TimeoutStopSec` 5 s since, as
/// a disable, reload, uninstall or upgrade (§7.2, A15d).
#[cfg(test)]
const PARK_WAIT: Duration = Duration::from_secs(5);
/// `VESTRA_IDLE_MS` when unset (the core's `Tunables::idle_timeout`): 60 s
/// until B6, Chrome MV3's 30 s since (§11).
const IDLE_MS: u64 = 30_000;
/// The shortest time the shared runner stays without a member before it
/// exits (§11 "≥ 30 s"; `runner_idle_exit`).
const RUNNER_IDLE_EXIT_MIN: u64 = 30_000;
/// What a unit in standby is woken by (the recorded entries' triggers);
/// held for its ready while it starts, answered when it cannot (§4.6).
pub(crate) const TRIGGERS: &[&str] = &[
    "commands.execute",
    "source.resolveRequest",
    "drops.drop",
    "ui.callback",
];

/// What the playback worker was asked for a run (`owner.resources`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ask {
    /// Its class at its ready, asked then (the adapter times the answer:
    /// `classify_wait`).
    Classify(u64),
    /// Whether it holds a live resource now, for the core's Standby job,
    /// asked then.
    Standby(u64),
}

/// What the Host itself knows keeps a run busy (§2.3, today's `lazy_idle`
/// without its idle time, which is the core's timer): each is declared work
/// in the core (`WorkKind::Declared`, never capped; §17.12). Being woken,
/// not recordable or not running are the core's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Busy {
    /// Its class is still asked (`Ask::Classify`).
    Classify = 1,
    /// Its activation is not over (`runtime.ready`, a UI request it waits
    /// on).
    Activation,
    /// A host dialog is open.
    Dialog,
    /// A package RPC of it is in flight.
    PackageRpc,
    /// One of its commands is busy (`commands.update { busy }`).
    Command,
    /// One of its settings pages is compiling or shown.
    Settings,
    /// A source state request of it waits for the playback worker.
    SourceState,
    /// Its standby recording could not be written ahead (today's `park`
    /// does not park then): busy until the next sync.
    Recording,
}

/// The Host's side of on-demand start (B2c; today's `Lazy`): what the
/// core's decisions need of the Host and the core does not keep.
#[derive(Default)]
pub(crate) struct OnDemand {
    /// On-demand start (the core's `Tunables::on_demand`): always on in the
    /// player since B6 (`from_env`; was `VESTRA_LAZY=1`). Off only in unit
    /// tests of the old path, which B7 removes with this field.
    pub enabled: bool,
    /// Units in standby by holder: what their kept registrations need.
    held: Revised<BTreeMap<u64, Held>>,
    /// `held` by manifest id: the B2a `debug_assert` cross-check of the
    /// core's `holder_of` (removed in B7).
    parked_ids: BTreeMap<String, u64>,
    /// The playback worker's answers waited for, by run.
    asked: BTreeMap<u64, Ask>,
    /// Each run's answer at its classification (`{ live, resolvers }`): its
    /// resolvers, for a checkpoint at quit (today's `Class.backend`).
    answers: BTreeMap<u64, Value>,
    /// Plugins already logged as staying up, with the reason.
    told: BTreeMap<u64, &'static str>,
    /// The Host's busy checks of each run as the core has them (declared
    /// work, `Busy`; §17.12): synced when the core asks (classification,
    /// standby) and every `idle_check` while any is in.
    work: BTreeMap<u64, BTreeSet<Busy>>,
    /// The last supervisor OSD (unit, text, when): one storm of answers or
    /// a notice and its answer show it once.
    last_osd: Option<(UnitId, String, u64)>,
    /// A standby's recording written before the core decided it (the run,
    /// its calls): the core's `Checkpoint` for it is done already.
    written: Option<(u64, usize)>,
    /// The recorder thread (`checkpoint::Writer`), started at the first
    /// recording it takes.
    recorder: Option<checkpoint::Writer>,
    /// Recordings being written ahead, by run: what waits for each
    /// (`Event::Recorded`).
    recording: BTreeMap<u64, Recording>,
    /// A wake's `Rekey` until its `Spawn` (from, to): the entries move once
    /// the process started, as today's `wake_now` moves them.
    rekey: Option<(u64, u64)>,
    /// Woken runs whose spawn failed before their entries moved: the new
    /// pid → the holder still holding them (the core's `Revoke` names the
    /// new pid).
    unrekeyed: BTreeMap<u64, u64>,
    /// Names for notices, by manifest id (as held).
    names: BTreeMap<String, String>,
    /// Holders' recorded listeners of wake events
    /// (`capabilities::events::WAKE_EVENTS`, §6.9): such an event wakes the
    /// unit, which receives it at its ready.
    pub wakes: BTreeMap<u64, BTreeSet<String>>,
    /// Set around an announcement that wakes no unit in standby: the first
    /// `packages.changed` and `bindings.changed` of a run (nothing changed
    /// for a plugin that last ran in an earlier one).
    pub mute: bool,
    /// Start-up settled (latched once per run; the core's `settled`).
    settled: bool,
    next_check: u64,
    tests: Vec<probe::TestExecute>,
    /// Test switches for the registry diff (tests/lazy_activation.rs):
    /// see [`probe::Probe`].
    pub probe: probe::Probe,
    /// Units held in standby after a fault, by holder: (unit, hold end).
    /// Their menu items show disabled with the reason meanwhile
    /// (`note_hold`, T20; B4).
    holds: BTreeMap<u64, (UnitId, u64)>,
    /// Owners whose run became ready (`runtime.ready`) while an initial UI
    /// request of theirs is still answered: ready once it is (T9, with
    /// on-demand start on; `ui_finished`): when it was woken, if it was.
    ui_pending: BTreeMap<u64, Option<u64>>,
    /// The quit's stop has begun (`shutdown_plugins`): a `Terminate` is a
    /// quit's (no UI cleanup, one deadline).
    pub quitting: bool,
    /// The spawn thread (`worker.rs`), started at the first spawn it takes.
    spawner: Option<worker::Spawner>,
    /// Starts whose process the spawn thread creates, by pid.
    spawning: BTreeMap<u64, worker::Spawning>,
    /// The memory pressure source and the level the core has (B5).
    memory: memory::Memory,
}
impl OnDemand {
    pub fn from_env() -> Self {
        // B6 (§15 B6): on-demand start for everyone; `VESTRA_LAZY` is no
        // longer read.
        let enabled = true;
        let tests = probe::test_executes(&std::env::var("VESTRA_TEST_EXECUTE").unwrap_or_default());
        Self {
            enabled,
            tests,
            probe: probe::Probe::from_env(),
            memory: memory::Memory::from_env(),
            ..Self::default()
        }
    }
    /// A held unit's data, by holder.
    pub fn held(&self, owner: u64) -> Option<&Held> {
        self.held.get(&owner)
    }
    /// Moves whenever a unit is held or let go (a source of
    /// `packages.list`: a held unit's `warning` and `warnings`).
    pub fn held_revision(&self) -> u64 {
        self.held.revision()
    }
}

/// A recording written ahead on the recorder thread (`record_ahead`).
struct Recording {
    /// The write's serial: a later write for the run replaces it.
    serial: u64,
    /// What it holds: compared with the registrations once it is written
    /// (a plugin still runs meanwhile and may change them).
    calls: Vec<capabilities::Call>,
    resident: bool,
    after: AfterRecording,
}

/// What waits for a recording written ahead: decided once it is written,
/// as it was right after the write when the Host wrote on its own thread.
enum AfterRecording {
    /// The standby the core asked about, with the playback worker's answer
    /// (`standby_answer`).
    Standby(Value),
    /// A lazy first run's classification, with the playback worker's
    /// answer (`class_known`).
    Classified(Value),
}

/// How long a read, write or removal of a recording waits at most for the
/// recorder thread's earlier writes (`recordings_written`).
const RECORDING_WAIT: Duration = Duration::from_secs(2);

/// The Host's supervisor (B6, §17.15): the design's values for everyone,
/// `Tunables::target` for this machine (B3's start-job limits for its
/// cores, B4's restart policy, B5's process budget for its physical
/// memory, and B6's timings), with the idle time from `VESTRA_IDLE_MS`
/// (30 s when unset; tests shorten it). Memory pressure is `memory.rs`'s,
/// polled in `on_demand_tick`. Wake events wake nothing until start-up
/// settled (`OnDemand::settled`). The seed feeds the start retries'
/// jitter. Only a unit test's `OnDemand` with on-demand start off still
/// gets the old path, B4's values on `Tunables::legacy()` (removed in B7).
pub(crate) fn new(on_demand: &OnDemand) -> Supervisor<Value> {
    let idle_timeout = std::env::var("VESTRA_IDLE_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
        .unwrap_or(IDLE_MS);
    let base = if on_demand.enabled {
        let cores = std::thread::available_parallelism().map_or(2, std::num::NonZero::get);
        let physical = memory::physical_mib();
        let tunables = Tunables::target(cores, physical);
        diag!(
            "lazy: on, idle after {idle_timeout} ms, {} starts at once ({cores} cores), {} third-party processes ({physical} MiB)",
            tunables.max_active.unwrap_or_default(),
            tunables.process_budget.unwrap_or_default()
        );
        tunables
    } else {
        Tunables {
            start_job_ends_at_spawn: true,
            ..Tunables::legacy().with_b4()
        }
    };
    let mut supervisor = Supervisor::new(
        Tunables {
            on_demand: on_demand.enabled,
            idle_timeout,
            ..base
        },
        1,
    );
    supervisor.set_muted(true);
    supervisor
}

/// How long the shared runner stays without a member before it exits (A17,
/// §6.2, §7.4; B6): at least the idle time and at least 30 s, so built-ins
/// going to standby one after another do not stop and start it again (1 s
/// until B6).
pub(crate) fn runner_idle_exit(supervisor: &Supervisor<Value>) -> u64 {
    supervisor.tunables().idle_timeout.max(RUNNER_IDLE_EXIT_MIN)
}

/// The JavaScript-safe integer bound of owner identities on the wire.
const MAX_OWNER: u64 = 9_007_199_254_740_991;

fn event(message: &Value) -> Option<&str> {
    message["event"].as_str()
}

impl Host {
    /// Whether a development load would start past the wire's safe
    /// integers (the core allocates its pid with the `Spawn`).
    pub(crate) fn owners_exhausted(&self) -> bool {
        self.supervisor.next_pid().0 > MAX_OWNER
    }
    /// The activation of plugin `id`'s run `owner` begins, due at
    /// `deadline` (before its process is spawned).
    pub(crate) fn activation_begin(
        &mut self,
        id: &str,
        owner: u64,
        deadline: u64,
    ) -> std::result::Result<(), activation::Error> {
        let unit = self.supervisor.load_unit(id);
        self.supervisor.activation_begin(unit, Pid(owner), deadline)
    }
    /// `owner`'s activation status now.
    pub(crate) fn activation_status(
        &mut self,
        owner: u64,
        now: u64,
    ) -> std::result::Result<activation::Status, activation::Error> {
        self.supervisor.activation_status(Pid(owner), now)
    }
    /// Plugin `id` runs as `owner` (`add_plugin`): the core's spawn of it
    /// ran. Every start is a job and waits for the unit's previous process
    /// (§4.3), so a second process of one id is a bug: a `debug_assert!`,
    /// and a diagnostic line in a release build (the core does no I/O).
    pub(crate) fn unit_started(&mut self, id: &str, owner: u64) {
        let displaced = self.plugin_ids.insert(id.to_owned(), owner);
        debug_assert!(
            displaced.is_none_or(|old| old == owner),
            "unit {id} hosted twice: {displaced:?} then {owner}"
        );
        if let Some(old) = displaced.filter(|old| *old != owner) {
            diag!("supervisor: unit {id} hosted twice: {old} then {owner}");
        }
        let now = self.now_ms();
        self.supervisor.set_now(now);
        self.supervisor.process_spawned(Pid(owner), Ok(()));
    }
    /// Plugin `id`'s process `owner` stopped (the `Terminate` executor).
    pub(crate) fn unit_stopped(&mut self, id: &str, owner: u64) {
        if self.plugin_ids.get(id) == Some(&owner) {
            self.plugin_ids.remove(id);
        }
    }
    /// The running owner of plugin `id` (by `by_id`).
    pub(crate) fn running(&self, id: &str) -> Option<u64> {
        let owner = self.supervisor.process_of(id).map(|pid| pid.0);
        debug_assert_eq!(
            owner,
            self.plugin_ids.get(id).copied(),
            "the unit table follows plugin_ids"
        );
        // Tests only: a scan per lookup made a debug start-up quadratic.
        #[cfg(test)]
        assert_eq!(
            owner,
            self.plugins
                .iter()
                .find(|(_, plugin)| plugin.manifest.id == id)
                .map(|(owner, _)| *owner),
            "plugin_ids follows plugins"
        );
        owner
    }
    /// The owner holding plugin `id`'s registrations in standby (the
    /// core's `holder_of`; today's `Lazy::parked_id`).
    pub(crate) fn parked_id(&self, id: &str) -> Option<u64> {
        let owner = self.supervisor.holder_of(id).map(|pid| pid.0);
        debug_assert_eq!(
            owner,
            self.on_demand.parked_ids.get(id).copied(),
            "the unit table follows parked_ids"
        );
        // Tests only: a scan per lookup made a debug start-up quadratic.
        #[cfg(test)]
        assert_eq!(
            owner,
            self.on_demand
                .held
                .iter()
                .find(|(_, held)| held.id == id)
                .map(|(owner, _)| *owner),
            "parked_ids follows held"
        );
        owner
    }
    // ---- stops and the waits for them (§17.9) --------------------------

    /// `Event::Reaped(owner)`: the reaper saw the process go. An
    /// uninstall's data owed at this reap goes now, as at today's
    /// `data_removals` (the starts waiting for it run in the next
    /// `supervisor_tick`, where today's ticks ran them).
    pub(crate) fn unit_reaped(&mut self, owner: u64, code: Option<i32>) {
        let now = self.now_ms();
        self.supervisor.set_now(now);
        let unit = self.supervisor.by_pid(Pid(owner));
        self.supervisor.process_reaped(Pid(owner), code);
        self.supervisor_run();
        if let Some(unit) = unit {
            self.note_hold(unit);
            self.remove_if_gone(unit);
        }
    }
    /// `Event::BackendCleaned(owner)`: the playback worker let go of what
    /// it held for that owner.
    pub(crate) fn unit_cleaned(&mut self, owner: u64) {
        let now = self.now_ms();
        self.supervisor.set_now(now);
        let unit = self.supervisor.by_pid(Pid(owner));
        self.supervisor.backend_cleaned(Pid(owner));
        self.supervisor_run();
        if let Some(unit) = unit {
            self.note_hold(unit);
            self.remove_if_gone(unit);
        }
    }
    /// A unit that landed in standby with a restart hold (a crash, a start
    /// that failed: T20, B4): until the hold is over its menu items show
    /// disabled with the reason, and a key or click shows it (§12.1;
    /// coordinator ruling 4 of B3). `on_demand_tick` lets go at its end.
    pub(super) fn note_hold(&mut self, unit: UnitId) {
        let now = self.now_ms();
        let Some(crate::supervisor::UnitState::Standby {
            holder,
            hold_until: Some(until),
            ..
        }) = self.supervisor.unit(unit).map(|u| u.state.clone())
        else {
            return;
        };
        if until <= now || self.on_demand.holds.contains_key(&holder.0) {
            return;
        }
        let name = self.unit_name(unit);
        let reason = i18n::tr_args(
            "{} stopped after a fault; try again in a moment",
            &json!([name]),
        );
        diag!(
            "supervisor: {} held for {} ms after a fault",
            self.unit_id(unit),
            until - now
        );
        self.commands.set_held(holder.0, Some(reason));
        self.on_demand.holds.insert(holder.0, (unit, until));
    }
    /// Holds that are over, or whose unit left standby: the menu items
    /// show their own state again.
    fn holds_tick(&mut self, now: u64) {
        let over = self
            .on_demand
            .holds
            .iter()
            .filter(|(holder, (unit, until))| {
                *until <= now
                    || !matches!(
                        self.supervisor.unit(*unit).map(|u| &u.state),
                        Some(crate::supervisor::UnitState::Standby { holder: h, .. }) if h.0 == **holder
                    )
            })
            .map(|(holder, _)| *holder)
            .collect::<Vec<_>>();
        for holder in over {
            self.release_hold(holder);
        }
    }
    /// `holder`'s menu items show their own state again.
    pub(super) fn release_hold(&mut self, holder: u64) {
        if self.on_demand.holds.remove(&holder).is_some() {
            self.commands.set_held(holder, None);
        }
    }
    /// A package-RPC job of plugin `id` is installed (a reload, an enable
    /// or a package operation waiting for the previous process): the
    /// core's half of today's `package_busy`.
    pub(crate) fn package_job_busy(&self, id: &str) -> bool {
        self.supervisor
            .by_id(id)
            .is_some_and(|unit| self.supervisor.package_busy(unit))
    }
    /// Due timers and waits end (`run_jobs`): once per host tick (today's
    /// `lazy_tick` idle checks, wake loop and "Starting…" notices, and
    /// `reload_tick`). A classification the playback worker did not answer
    /// in time is resident first (`classify_timeouts`).
    pub(crate) fn supervisor_tick(&mut self) {
        let now = self.now_ms();
        self.classify_timeouts(now);
        self.supervisor_run();
    }
    /// Runs the core's jobs now and executes what it decided, until it has
    /// nothing more (an executor's input may start more: a spawn that
    /// failed, a check that is over). Every request and input of the Host
    /// ends with it, so a stop or start happens inside the call that asked
    /// for it, as today's direct calls did.
    pub(crate) fn supervisor_run(&mut self) {
        for _ in 0..64 {
            let now = self.now_ms();
            let actions = self.supervisor.run_jobs(now);
            if actions.is_empty() {
                return;
            }
            self.supervisor_execute(actions);
        }
        diag!("supervisor: actions still pending after 64 rounds");
    }

    // ---- B2c: on-demand start (§17.10) ----------------------------------

    /// Whether `message` for `owner` goes to its process now (`Host::send`):
    /// `Some(owner)`. A trigger (`TRIGGERS`; the `source.resolveCancel`
    /// that follows a held request) is the core's `wake`: sent now to a
    /// running unit (counts as use), forwarded from an old pid to the run
    /// it moved to, held while the unit starts, and it starts a unit in
    /// standby. Anything else goes to a process if there is one (a unit in
    /// standby has none).
    pub(crate) fn deliver_to(&mut self, owner: u64, message: &Value) -> Option<u64> {
        let trigger = match event(message) {
            Some(name) if TRIGGERS.contains(&name) => Trigger::interactive(message.clone()),
            Some("source.resolveCancel") => Trigger::follow(message.clone()),
            _ => return Some(owner),
        };
        let now = self.now_ms();
        self.supervisor.set_now(now);
        match self.supervisor.wake(Pid(owner), &trigger) {
            Delivery::Now => {
                self.request_sent(owner, message);
                Some(owner)
            }
            Delivery::Forward(to) => {
                self.send(to.0, message.clone());
                None
            }
            Delivery::Started => {
                self.wake_started(owner);
                None
            }
            Delivery::Held | Delivery::Refused => {
                // Held only while a unit starts (under `legacy()` only one
                // woken from standby: `deliver_while_starting`; from B3 every
                // start until its ready, T9), or for the next run of a
                // restart: a running plugin's trigger is never kept from it
                // otherwise.
                let restarting = self
                    .supervisor
                    .by_pid(Pid(owner))
                    .and_then(|unit| self.supervisor.job(unit))
                    .is_some_and(|job| {
                        matches!(job.kind, crate::supervisor::JobKind::Restart { .. })
                    });
                let held_from_running = self.plugins.contains_key(&owner)
                    && self.supervisor.waking(Pid(owner)).is_none()
                    && !self.supervisor.activating(Pid(owner))
                    && !restarting;
                debug_assert!(
                    !held_from_running,
                    "a trigger for running plugin {owner} was held"
                );
                if held_from_running {
                    diag!("supervisor: a trigger for running plugin {owner} was held");
                }
                self.supervisor_run();
                None
            }
            // No unit runs it any more (a stopped run's late message), or
            // its unit failed: a process still there gets it, as before.
            Delivery::Dropped => self.plugins.contains_key(&owner).then_some(owner),
        }
    }
    /// A trigger went to `owner`'s process: a `source.resolveRequest` is a
    /// host→plugin request awaiting its reply (`WorkKind::Request`, §17.12:
    /// busy until answered, capped by the request timeout from B6); its
    /// `source.resolveCancel` ends it.
    fn request_sent(&mut self, owner: u64, message: &Value) {
        let Some(id) = message["data"]["requestId"].as_u64() else {
            return;
        };
        let now = self.now_ms();
        match event(message) {
            Some("source.resolveRequest") => {
                self.supervisor
                    .begin_work(Pid(owner), WorkKind::Request(id), now);
            }
            Some("source.resolveCancel") => {
                self.supervisor.set_now(now);
                self.supervisor.end_work(Pid(owner), WorkKind::Request(id));
            }
            _ => {}
        }
    }
    /// `owner` answered resolution request `id` (`sourceResolvers.respond`).
    pub(crate) fn resolution_answered(&mut self, owner: u64, id: Option<u64>) {
        let Some(id) = id else {
            return;
        };
        let now = self.now_ms();
        self.supervisor.set_now(now);
        self.supervisor.end_work(Pid(owner), WorkKind::Request(id));
    }
    /// A wake of `owner`'s unit was enqueued: it starts in this turn if a
    /// start slot is free (B3: `max_active` third-party starts at once, an
    /// interactive trigger first, §4.4), else when one frees; and only once
    /// the process it was parked from is gone (one instance per data
    /// directory; the core's job starts it then, or fails after 20 s).
    fn wake_started(&mut self, owner: u64) {
        let Some(unit) = self.supervisor.by_pid(Pid(owner)) else {
            return;
        };
        let now = self.now_ms();
        let actions = self.supervisor.run_jobs(now);
        let started = actions.iter().any(|a| {
            matches!(a, Action::Spawn { unit: u, .. } | Action::Verify { unit: u } if *u == unit)
        });
        if !started {
            let id = self.unit_id(unit);
            if self
                .supervisor
                .unit(unit)
                .is_some_and(|unit| unit.stopping.is_some())
            {
                diag!("lazy: {id} waits for its stopped process");
            } else {
                diag!("lazy: {id} waits for a start slot");
            }
        }
        self.supervisor_execute(actions);
        self.supervisor_run();
    }
    /// The manifest id of `unit`.
    pub(crate) fn unit_id(&self, unit: UnitId) -> String {
        self.supervisor
            .unit(unit)
            .map(|unit| unit.id.clone())
            .unwrap_or_default()
    }
    /// `Host::broadcast` of `event`: each unit in standby that listened to
    /// it (a wake event, §6.9) is woken and receives it at its ready; a
    /// newer one of the same event replaces one still held. Not while the
    /// player starts nor `quiet_after_start` after it settled (the core
    /// mutes them), nor for the first announcement of a run
    /// (`OnDemand::mute`): the plugins registering then are not a change,
    /// and a woken plugin reads the current state when it starts anyway.
    pub(crate) fn wake_listeners(&mut self, event: &str, data: &Value) {
        if self.on_demand.mute {
            return;
        }
        // Every broadcast comes here (`source.state` ten times a second):
        // the listeners first.
        let owners = self
            .on_demand
            .wakes
            .iter()
            .filter(|(_, events)| events.contains(event))
            .map(|(owner, _)| *owner)
            .collect::<Vec<_>>();
        if owners.is_empty() {
            return;
        }
        let now = self.now_ms();
        self.supervisor.set_now(now);
        for owner in owners {
            if !self.on_demand.held.contains_key(&owner) {
                continue;
            }
            let trigger = Trigger::event(event, json!({"event":event,"data":data}));
            if self.supervisor.wake(Pid(owner), &trigger) == Delivery::Started {
                self.wake_started(owner);
            }
        }
    }
    /// A settings page of `owner` is shown (§2.1): a unit in standby
    /// starts, so the page shows its current values and its callbacks have
    /// a receiver (nothing is delivered at its ready).
    pub(crate) fn page_shown(&mut self, owner: u64) {
        if !self.on_demand.held.contains_key(&owner) {
            return;
        }
        let now = self.now_ms();
        self.supervisor.set_now(now);
        match self
            .supervisor
            .wake(Pid(owner), &Trigger::interactive(Value::Null))
        {
            Delivery::Started => self.wake_started(owner),
            _ => self.supervisor_run(),
        }
    }
    /// Start-up is over: every installed plugin started, replayed or
    /// queued has run its first activation (as files opened at launch wait,
    /// `open_files_tick`).
    fn startup_settled(&mut self) -> bool {
        if !self.packages_started()
            || self.builtin_first_run_queued()
            || self.supervisor.first_runs_queued().0
        {
            return false;
        }
        let now = self.now_ms();
        let owners = self
            .plugins
            .keys()
            .filter(|owner| self.supervisor.waking(Pid(**owner)).is_none())
            .copied()
            .collect::<Vec<_>>();
        !owners.into_iter().any(|owner| {
            matches!(
                self.activation_status(owner, now),
                Ok(activation::Status::Pending { .. })
            )
        })
    }
    /// `owner` is held in standby now (its registrations kept under it).
    fn hold(&mut self, owner: u64, held: Held) {
        self.on_demand.parked_ids.insert(held.id.clone(), owner);
        self.on_demand
            .names
            .insert(held.id.clone(), held.name.clone());
        self.on_demand.held.edit().insert(owner, held);
    }
    /// `owner` is no longer held: its catalog goes.
    pub(super) fn unhold(&mut self, owner: u64) -> Option<Held> {
        let held = self.on_demand.held.edit().remove(&owner)?;
        if self.on_demand.parked_ids.get(&held.id) == Some(&owner) {
            self.on_demand.parked_ids.remove(&held.id);
        }
        held.release();
        Some(held)
    }
    /// A held owner's registrations go (it is disabled, uninstalled,
    /// reloaded or could not start).
    pub(super) fn forget_entries(&mut self, owner: u64) {
        capabilities::revoke(self, owner, false);
        self.plugin_shaders_gone(owner);
        let _ = self.backend.commands.send(Command {
            reply: Reply { owner, id: 0 },
            method: "owner.disabled".into(),
            params: Value::Null,
        });
    }
    /// `Action::Replay` (T1; today's `lazy_replay` from `start_installed`):
    /// `id`'s recorded calls are registered under `holder`, a pid with no
    /// process; the unit is in standby. One that cannot be replayed starts
    /// instead (`replay_failed`).
    pub(super) fn replay_unit(&mut self, unit: UnitId, holder: u64) {
        let id = self.unit_id(unit);
        let replayed = self.standby_replay(&id, holder);
        if let Err(error) = replayed {
            diag!("lazy: {id} replay failed, starting it: {error}");
            self.supervisor.replay_failed(unit);
        }
    }
    fn standby_replay(&mut self, id: &str, owner: u64) -> Result<()> {
        let (recording, directory, manifest, key) = self
            .packages_replay(id)
            .ok_or_else(|| anyhow::anyhow!("its recording is gone"))?;
        let catalog = i18n::load(&directory);
        let origin = Origin {
            id: id.into(),
            directory: directory.to_owned(),
            catalog: catalog.clone(),
            data_directory: process::data_root()
                .map(|root| process::DataKey::Instance(key.guid.clone()).directory(&root)),
        };
        for call in &recording.calls {
            if let Err(error) =
                capabilities::replay(self, owner, &origin, &manifest.capabilities, call)
            {
                self.forget_entries(owner);
                bail!("{}: {}: {}", call.method, error.code, error.message);
            }
        }
        // Everything replayed is recorded (live registrations replace it).
        capabilities::revoke(self, owner, true);
        let name = crate::packages::package_manager::display_name(&manifest, catalog.as_deref());
        diag!(
            "lazy: {id} replayed ({} calls) as owner {owner}, not started",
            recording.calls.len()
        );
        self.hold(owner, Held::new(id.into(), origin, name));
        self.probe_registry(owner, "replayed");
        Ok(())
    }
    /// `runtime.ready` of `owner` (today's `lazy_ready`). A run started
    /// otherwise than from standby (boot, first run, enable, reload,
    /// install) that holds no trigger: the core's `process_ready`, then its
    /// class is asked (`classify`). A woken run, or from B3 any start that
    /// held triggers for its ready (T9): the core prunes the recorded
    /// entries a woken run did not register again and delivers the held
    /// triggers, each checked against its live state; its class is asked
    /// before they run (what they open is not what it holds at its ready,
    /// §17.7 item 14).
    pub(crate) fn unit_ready(&mut self, owner: u64) {
        let now = self.now_ms();
        self.supervisor.set_now(now);
        self.supervisor.touch(Pid(owner), now);
        let waking = self.supervisor.waking(Pid(owner));
        self.supervisor.process_ready(Pid(owner));
        // With on-demand start on, a start is ready once its initial UI
        // requests are answered too (T9; B4): until then it holds its
        // triggers and its start slot (`ui_finished`).
        if self.on_demand.enabled && self.supervisor.activating(Pid(owner)) {
            diag!("supervisor: owner {owner} ready, its UI not yet");
            self.on_demand.ui_pending.insert(owner, waking);
            return;
        }
        self.became_ready(owner, waking);
    }
    /// An initial UI request of `owner`'s activation was answered (T9,
    /// with on-demand start on): a run whose `runtime.ready` came already
    /// is ready once its last one is.
    pub(crate) fn ui_finished(&mut self, owner: u64) {
        if !self.on_demand.enabled {
            return;
        }
        let now = self.now_ms();
        self.supervisor.set_now(now);
        self.supervisor.ui_finish(Pid(owner));
        if !self.supervisor.activating(Pid(owner))
            && let Some(waking) = self.on_demand.ui_pending.remove(&owner)
        {
            self.became_ready(owner, waking);
        }
    }
    /// An initial UI request of `owner`'s activation is waited on (T9, with
    /// on-demand start on).
    pub(crate) fn ui_begun(&mut self, owner: u64) {
        if self.on_demand.enabled {
            self.supervisor.ui_begin(Pid(owner));
        }
    }
    /// `owner`'s run is ready (T9): what was held for it runs, after its
    /// class was asked (§17.7 item 14).
    fn became_ready(&mut self, owner: u64, waking: Option<u64>) {
        let now = self.now_ms();
        let actions = self.supervisor.run_jobs(now);
        let delivers = actions
            .iter()
            .any(|a| matches!(a, Action::Deliver { pid, .. } if pid.0 == owner));
        // Any other start (boot, first run, enable, reload, install): its
        // class is asked after its ready's actions ran. From B3 such a start
        // holds its triggers for its ready too (T9): those are delivered as
        // a woken run's are, after its class was asked.
        if waking.is_none() && !delivers {
            self.probe_registry(owner, "ready");
            self.supervisor_execute(actions);
            self.supervisor_run();
            self.classify(owner);
            return;
        }
        let triggers = actions
            .iter()
            .filter(|a| matches!(a, Action::Deliver { payload, .. } if !payload.is_null()))
            .count();
        // The titles of the commands triggered, before their entries may go.
        let titles = actions
            .iter()
            .filter_map(|a| match a {
                Action::Deliver { payload, .. } if event(payload) == Some("commands.execute") => {
                    payload["data"]["id"].as_str()
                }
                _ => None,
            })
            .filter_map(|id| {
                let title = self.commands.get(owner, id)?.declaration.shown_title();
                Some((id.to_owned(), title.to_owned()))
            })
            .collect::<BTreeMap<_, _>>();
        let mut classified = false;
        for action in actions {
            match action {
                Action::Prune { pid, .. } => capabilities::prune(self, pid.0),
                Action::Deliver { pid, payload, .. } => {
                    if !classified {
                        classified = true;
                        let after = waking.map(|since| now.saturating_sub(since));
                        self.woken_ready(owner, after, triggers);
                    }
                    self.deliver_held(pid.0, payload, &titles);
                }
                other => self.supervisor_execute(vec![other]),
            }
        }
        if !classified {
            let after = waking.map(|since| now.saturating_sub(since));
            self.woken_ready(owner, after, triggers);
        }
    }
    /// A run that held triggers is ready (woken from standby: `after` its
    /// spawn): its registries are probed and its class asked.
    fn woken_ready(&mut self, owner: u64, after: Option<u64>, triggers: usize) {
        match after {
            Some(after) => {
                self.probe_registry(owner, "woken");
                diag!("lazy: owner {owner} ready after {after} ms, {triggers} trigger(s)");
            }
            None => {
                self.probe_registry(owner, "ready");
                diag!("supervisor: owner {owner} ready, {triggers} held trigger(s)");
            }
        }
        // Before its triggers run: what they open is not what it holds at
        // its ready.
        self.classify(owner);
    }
    /// A trigger held for a woken run's ready (`Action::Deliver`).
    fn deliver_held(&mut self, owner: u64, message: Value, titles: &BTreeMap<String, String>) {
        // A shown settings page woke it: nothing to deliver.
        if message.is_null() {
            return;
        }
        if event(&message) != Some("commands.execute") {
            self.send(owner, message);
            return;
        }
        let id = message["data"]["id"].as_str().unwrap_or_default();
        match self.commands.resolve_execution(owner, id) {
            Ok(_) => {
                if let Err(error) = self.execute_command(owner, id) {
                    diag!("lazy: execute {id}: {error:?}");
                }
            }
            // As a key does: the command's reason shows.
            Err(commands::CommandError::Disabled {
                reason: Some(reason),
            }) => self.show_osd(&reason),
            Err(commands::CommandError::Missing(_)) => {
                let title = titles.get(id).cloned().unwrap_or_else(|| id.to_owned());
                self.show_osd(&i18n::tr_args("{} is no longer available", &json!([title])));
            }
            Err(error) => diag!("lazy: execute {id}: {error}"),
        }
    }
    /// §3.4: at `runtime.ready`, whether installed plugin `owner` holds a
    /// live resource. The host's half is read now (before any trigger it
    /// was woken by runs); the playback worker's half is asked, and its
    /// answer comes before anything the plugin asks it after this
    /// (`resources_answer`).
    fn classify(&mut self, owner: u64) {
        if self.checkpoint_key_of(owner).is_none() {
            self.development_ready(owner);
            return;
        }
        if !self.on_demand.enabled {
            return;
        }
        if self.on_demand.probe.park_all {
            self.class_known(owner, None, Value::Null);
            return;
        }
        if let Err(reason) = capabilities::record(self, owner, &Value::Null) {
            self.class_known(owner, Some(reason.to_owned()), Value::Null);
            return;
        }
        let asked = self.backend.commands.send(Command {
            reply: Reply { owner, id: 0 },
            method: "owner.resources".into(),
            params: Value::Null,
        });
        if asked.is_ok() {
            // The run's ready, as the core has it (`process_ready`): one
            // deadline with the core's classify timer.
            let ready = self.supervisor.now();
            self.on_demand.asked.insert(owner, Ask::Classify(ready));
        }
    }
    /// The listeners that keep `owner` running (the SDK's mirrors and wake
    /// events left out), each as its SDK described it (`events.subscribe
    /// { why }`, SDK 1.5.40: what listens and where the plugin added it):
    /// every reason of an event, ` | ` between them: `; listens to
    /// source.state: command "Save" enabled is a function that reads
    /// plugin.player.state (main.ts:361) — use plugin.when | a listener added
    /// at dialog.ts:183`. Empty without one.
    pub(crate) fn resident_listeners(&self, owner: u64) -> String {
        let Some(plugin) = self.plugins.get(&owner) else {
            return String::new();
        };
        let listeners = plugin
            .subscriptions
            .iter()
            .filter(|(event, active)| {
                **active && !capabilities::WAKE_EVENTS.contains(&event.as_str())
            })
            .map(|(event, _)| match plugin.listeners.get(event) {
                Some(whys) => format!("{event}: {}", whys.join(" | ")),
                None => event.clone(),
            })
            .collect::<Vec<_>>();
        if listeners.is_empty() {
            String::new()
        } else {
            format!("; listens to {}", listeners.join("; "))
        }
    }
    /// A development load (`--plugin`) is never put in standby; at its
    /// ready the log says whether it could be, and if not why, for its
    /// author (§6.9). Only the host's half: what the playback worker holds
    /// for it (a filter chain, a stream) is not asked.
    fn development_ready(&mut self, owner: u64) {
        let Some(plugin) = self.plugins.get(&owner) else {
            return;
        };
        if !matches!(plugin.data_key, process::DataKey::Development(_)) {
            return;
        }
        let id = plugin.manifest.id.clone();
        match capabilities::record(self, owner, &Value::Null) {
            Err(reason) => diag!(
                "plugin {id}: keeps running while idle: {reason}{}",
                self.resident_listeners(owner)
            ),
            Ok(_) => diag!(
                "plugin {id}: may be stopped while idle (it holds nothing only its process can serve; a filter chain or a stream would keep it)"
            ),
        }
    }
    /// `owner`'s class at its ready (today's `lazy_classified`), for the
    /// core: a resident one is checkpointed as such (the next start
    /// launches it, even after a crash) and frees its first-run place; a
    /// lazy first run goes to standby at once if the Host's own idle check
    /// passes and its recording is written, else as soon as it is idle.
    fn class_known(&mut self, owner: u64, resident: Option<String>, backend: Value) {
        let first_run = self.supervisor.run_is_first(Pid(owner));
        // The core closed this run's classification already (answered, or
        // timed out): a late answer changes nothing, and writes nothing
        // (today's `lazy_answer` drops it once `lazy_classify_tick` gave up).
        if !self.supervisor.classifying(Pid(owner)) {
            return;
        }
        let id = self.plugin_id(owner);
        match &resident {
            Some(reason) => diag!(
                "lazy: {id} classified resident: {reason}{}",
                self.resident_listeners(owner)
            ),
            None => diag!("lazy: {id} classified lazy"),
        }
        let now = self.now_ms();
        self.supervisor.set_now(now);
        // A lazy first run goes to standby at once unless busy: the Host's
        // checks are declared work (`sync_work`), and its recording is
        // written ahead (today's `park` does not park when that fails), on
        // the recorder thread: the classification waits for it.
        if resident.is_none() && first_run && !self.sync_work(owner, false) {
            match self.standby_record(owner, &backend) {
                Ok(calls) => {
                    self.on_demand.answers.insert(owner, backend.clone());
                    self.record_ahead(owner, calls, false, AfterRecording::Classified(backend));
                    return;
                }
                Err(_) => {
                    self.sync_work(owner, true);
                }
            }
        }
        self.classify_with(owner, resident, backend);
    }
    fn classify_with(&mut self, owner: u64, resident: Option<String>, backend: Value) {
        self.on_demand.answers.insert(owner, backend);
        self.supervisor.classified(Pid(owner), resident);
        self.supervisor_run();
        self.on_demand.written = None;
    }
    /// What the Host itself knows keeps `owner` busy now (`Busy`).
    fn host_busy(&mut self, owner: u64, now: u64) -> BTreeSet<Busy> {
        let mut busy = BTreeSet::new();
        let Some(plugin) = self.plugins.get(&owner) else {
            return busy;
        };
        let id = plugin.manifest.id.clone();
        if matches!(self.on_demand.asked.get(&owner), Some(Ask::Classify(_))) {
            busy.insert(Busy::Classify);
        }
        if !matches!(
            self.activation_status(owner, now),
            Ok(activation::Status::Ready)
        ) || self.supervisor.activation_ui_waiting(Pid(owner))
        {
            busy.insert(Busy::Activation);
        }
        if self.dialog_busy {
            busy.insert(Busy::Dialog);
        }
        if self.package_rpc_busy(&id) {
            busy.insert(Busy::PackageRpc);
        }
        if self.commands.busy(owner) {
            busy.insert(Busy::Command);
        }
        if self.settings.busy(owner) {
            busy.insert(Busy::Settings);
        }
        if self.source_state_replies.iter().any(|(o, _)| *o == owner) {
            busy.insert(Busy::SourceState);
        }
        busy
    }
    /// The core's declared work of `owner` follows the Host's busy checks
    /// now (`recording`: its standby recording could not be written ahead):
    /// whether it is busy. Work that ended counts as use (the idle time
    /// starts again, Chrome MV3: an event in flight extends the lifetime).
    fn sync_work(&mut self, owner: u64, recording: bool) -> bool {
        let now = self.now_ms();
        let mut busy = self.host_busy(owner, now);
        if recording {
            busy.insert(Busy::Recording);
        }
        let before = self.on_demand.work.remove(&owner).unwrap_or_default();
        self.supervisor.set_now(now);
        for kind in before.difference(&busy) {
            self.supervisor
                .end_work(Pid(owner), WorkKind::Declared(*kind as u64));
        }
        for kind in busy.difference(&before) {
            self.supervisor
                .begin_work(Pid(owner), WorkKind::Declared(*kind as u64), now);
        }
        if busy.is_empty() {
            return false;
        }
        self.on_demand.work.insert(owner, busy);
        true
    }
    /// A standby's recording, written before the core decides the standby
    /// (today's `park` writes it first and does not park when it fails),
    /// on the recorder thread (supervisor.md §6.1): `after` waits for it
    /// (`recorded`). The core's `Checkpoint` for it is then done. Without
    /// the thread it is written here, as before.
    fn record_ahead(
        &mut self,
        owner: u64,
        calls: Vec<capabilities::Call>,
        resident: bool,
        after: AfterRecording,
    ) {
        let count = calls.len();
        let Some(key) = self.checkpoint_key_of(owner) else {
            self.recorded_then(owner, count, Err(None), after);
            return;
        };
        let directory = self.activation_dir();
        if self.on_demand.recorder.is_none() {
            self.on_demand.recorder = checkpoint::Writer::start(self.events.clone());
        }
        let kept = calls.clone();
        let queued = match self.on_demand.recorder.as_mut() {
            Some(recorder) => {
                recorder.write(directory.clone(), key.clone(), calls, resident, owner)
            }
            None => Err(calls),
        };
        match queued {
            Ok(serial) => {
                self.on_demand.recording.insert(
                    owner,
                    Recording {
                        serial,
                        calls: kept,
                        resident,
                        after,
                    },
                );
            }
            Err(calls) => {
                let result = checkpoint::write(&directory, &key, calls, resident)
                    .map_err(|error| Some(format!("{error:#}")));
                self.recorded_then(owner, count, result, after);
            }
        }
    }
    /// `Event::Recorded`: the recorder thread wrote `owner`'s recording,
    /// or could not. A write a later one for the run replaced, and any
    /// once the player quits, decide nothing.
    pub(crate) fn recorded(
        &mut self,
        owner: u64,
        serial: u64,
        result: std::result::Result<(), String>,
    ) {
        if self
            .on_demand
            .recording
            .get(&owner)
            .is_none_or(|recording| recording.serial != serial)
        {
            return;
        }
        let Some(recording) = self.on_demand.recording.remove(&owner) else {
            return;
        };
        if self.on_demand.quitting {
            return;
        }
        // The plugin still ran while it was written: what it registered
        // since goes in too, written again before anything is decided (the
        // recording is the registrations as they are at the decision, as
        // when it was written in the same turn).
        if result.is_ok() && self.plugins.contains_key(&owner) {
            let backend = match &recording.after {
                AfterRecording::Standby(answer) | AfterRecording::Classified(answer) => answer,
            };
            match self.standby_record(owner, backend) {
                Ok(calls) if calls != recording.calls => {
                    self.record_ahead(owner, calls, recording.resident, recording.after);
                    return;
                }
                Ok(_) => {}
                // Nothing can be recorded now (a live resource): not parked.
                Err(_) => {
                    self.recorded_then(owner, recording.calls.len(), Err(None), recording.after);
                    return;
                }
            }
        }
        self.recorded_then(
            owner,
            recording.calls.len(),
            result.map_err(Some),
            recording.after,
        );
    }
    /// What waited for `owner`'s recording, decided as right after its
    /// write (`Err(None)`: there was nothing to write it for). The Host's
    /// own busy checks are synced again first: they may have changed while
    /// it was written.
    fn recorded_then(
        &mut self,
        owner: u64,
        count: usize,
        result: std::result::Result<(), Option<String>>,
        after: AfterRecording,
    ) {
        let written = match result {
            Ok(()) => {
                self.on_demand.written = Some((owner, count));
                true
            }
            Err(error) => {
                if let Some(error) = error {
                    diag!(
                        "lazy: {} not parked: its recording: {error}",
                        self.plugin_id(owner)
                    );
                    let now = self.now_ms();
                    self.supervisor.touch(Pid(owner), now);
                }
                false
            }
        };
        match after {
            AfterRecording::Standby(_) => {
                if self.plugins.contains_key(&owner) {
                    let busy = !written || self.sync_work(owner, false);
                    let now = self.now_ms();
                    self.supervisor.set_now(now);
                    self.supervisor.resources_answered(Pid(owner), busy);
                    self.supervisor_run();
                }
            }
            AfterRecording::Classified(backend) => {
                if self.supervisor.classifying(Pid(owner)) {
                    if !self.sync_work(owner, false) && !written {
                        self.sync_work(owner, true);
                    }
                    self.classify_with(owner, None, backend);
                }
            }
        }
        self.on_demand.written = None;
    }
    /// Waits (at most `RECORDING_WAIT`) until the recorder thread wrote
    /// every recording it was given: before a recording is read, written
    /// here or removed, so that an earlier write never lands after it.
    pub(crate) fn recordings_written(&self) {
        self.recordings_written_by(RECORDING_WAIT);
    }
    pub(crate) fn recordings_written_by(&self, budget: Duration) {
        if let Some(recorder) = &self.on_demand.recorder
            && !recorder.flush(budget)
        {
            diag!(
                "lazy: recordings still being written after {} ms",
                budget.as_millis()
            );
        }
    }
    /// The playback worker answered `owner.resources` (today's
    /// `lazy_answer`): the worker's half of a classification at ready, or
    /// of a standby the core asked for (`QueryResources`): it goes to
    /// standby if it is still idle and nothing it holds is live.
    pub(crate) fn resources_answer(&mut self, owner: u64, answer: &Value) {
        match self.on_demand.asked.remove(&owner) {
            Some(Ask::Classify(_)) => {
                if !self.plugins.contains_key(&owner) {
                    return;
                }
                let resident = answer["live"]
                    .as_array()
                    .filter(|live| !live.is_empty())
                    .map(|live| {
                        format!(
                            "a resource in the playback worker {}",
                            Value::Array(live.clone())
                        )
                    });
                self.class_known(owner, resident, answer.clone());
            }
            Some(Ask::Standby(asked)) => self.standby_answer(owner, asked, answer),
            None => {}
        }
    }
    fn standby_answer(&mut self, owner: u64, asked: u64, answer: &Value) {
        if !self.plugins.contains_key(&owner) {
            return;
        }
        let now = self.now_ms();
        self.supervisor.set_now(now);
        // Used since it was asked (a trigger, a request), or busy now: not
        // now, and nothing counts as use (today's early return).
        let used = self
            .supervisor
            .by_pid(Pid(owner))
            .and_then(|unit| self.supervisor.unit(unit))
            .and_then(|unit| unit.last_active())
            .is_some_and(|at| at >= asked);
        // Busy now (declared work since B3, §17.12): the core keeps it up
        // and asks again after another idle time.
        if used || self.sync_work(owner, false) {
            self.supervisor.resources_answered(Pid(owner), true);
            self.supervisor_run();
            return;
        }
        let recorded = match answer["live"].as_array().and_then(|live| live.first()) {
            Some(_) if !self.on_demand.probe.park_all => Err("a resource in the playback worker"),
            _ => self.standby_record(owner, answer),
        };
        match recorded {
            // A plugin resident at its ready that let go of everything since
            // is parked, but the next start still launches it (§3.4). The
            // core is answered once its recording is written (`recorded`).
            Ok(calls) => {
                let resident = self
                    .supervisor
                    .by_pid(Pid(owner))
                    .and_then(|unit| self.supervisor.unit(unit))
                    .is_some_and(|unit| unit.resident);
                self.record_ahead(
                    owner,
                    calls,
                    resident,
                    AfterRecording::Standby(answer.clone()),
                );
            }
            Err(reason) => {
                // Asked again after another idle period.
                if self.on_demand.told.insert(owner, reason) != Some(reason) {
                    diag!(
                        "lazy: {} stays running: {reason}{}{}",
                        self.plugin_id(owner),
                        answer["live"]
                            .as_array()
                            .filter(|live| !live.is_empty())
                            .map(|live| format!(" {}", Value::Array(live.clone())))
                            .unwrap_or_default(),
                        self.resident_listeners(owner)
                    );
                }
                self.supervisor.resources_answered(Pid(owner), true);
                self.supervisor_run();
            }
        }
    }
    /// The core's Standby job asks whether `owner` may go to standby
    /// (`Action::QueryResources`): the Host's own busy checks first, synced
    /// as declared work (today `lazy_tick` asked only an idle plugin), then
    /// the playback worker. Busy (or the worker gone): answered busy.
    pub(super) fn query_resources(&mut self, owner: u64) {
        self.log_reclaim(owner);
        let now = self.now_ms();
        if self.plugins.contains_key(&owner) && !self.sync_work(owner, false) {
            let asked = self.backend.commands.send(Command {
                reply: Reply { owner, id: 0 },
                method: "owner.resources".into(),
                params: Value::Null,
            });
            if asked.is_ok() {
                self.on_demand.asked.insert(owner, Ask::Standby(now));
                return;
            }
        }
        self.supervisor.set_now(now);
        self.supervisor.resources_answered(Pid(owner), true);
        self.supervisor_run();
    }
    /// The playback worker did not answer a classification within
    /// `classify_wait` (5 s): resident (the safe side; its first-run place
    /// frees), and said so. One deadline: the core times a run without a
    /// checkpoint from the same ready (`Ask::Classify` holds the core's
    /// time), and this runs before `run_jobs` in the same tick, so it comes
    /// first and the Host logs and frees the place either way; a recorded
    /// run has only this one (§17.7). An answer after it is dropped.
    fn classify_timeouts(&mut self, now: u64) {
        let wait = self.supervisor.tunables().classify_wait;
        let late = self
            .on_demand
            .asked
            .iter()
            .filter(|(_, ask)| matches!(ask, Ask::Classify(at) if now.saturating_sub(*at) >= wait))
            .map(|(owner, _)| *owner)
            .collect::<Vec<_>>();
        for owner in late {
            self.on_demand.asked.remove(&owner);
            if self.plugins.contains_key(&owner) {
                diag!(
                    "lazy: {}: the playback worker did not answer within {wait} ms",
                    self.plugin_id(owner)
                );
                let reason = "no answer from the playback worker".to_owned();
                self.class_known(owner, Some(reason), Value::Null);
            }
        }
    }
    /// A run stopped (`Host::stop`): what was asked or logged for it goes.
    pub(crate) fn on_demand_forget(&mut self, owner: u64) {
        self.on_demand.asked.remove(&owner);
        self.on_demand.ui_pending.remove(&owner);
        self.on_demand.told.remove(&owner);
        self.on_demand.answers.remove(&owner);
        // The core's work of the run went with it.
        self.on_demand.work.remove(&owner);
    }
    /// Every host tick: the test hooks, and every `idle_check` (250 ms)
    /// whether start-up settled. The idle checks, first runs, "Starting…"
    /// notices and classification timers are the core's (`supervisor_tick`).
    pub(crate) fn on_demand_tick(&mut self) {
        let now = self.now_ms();
        self.test_execute(now);
        if !self.on_demand.holds.is_empty() {
            self.holds_tick(now);
        }
        if !self.on_demand.enabled {
            return;
        }
        self.probe_tick(now);
        self.memory_tick(now);
        if now < self.on_demand.next_check {
            return;
        }
        self.on_demand.next_check = now + self.supervisor.tunables().idle_check;
        // Busy runs: their declared work ends when the Host's checks pass
        // (O(busy runs) every `idle_check`, today's 250 ms poll).
        let busy = self.on_demand.work.keys().copied().collect::<Vec<_>>();
        for owner in busy {
            self.sync_work(owner, false);
        }
        if !self.on_demand.settled && self.startup_settled() {
            diag!("lazy: start-up settled");
            self.on_demand.settled = true;
            self.supervisor.settled(now);
        }
    }
}

/// Deletes an uninstalled plugin's data directory (`Action::RemoveData`).
fn remove_data(directory: &Path) {
    let _ = std::fs::remove_dir_all(directory);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// B0 (supervisor.md §11; B1 reproduces these): lazy activation's
    /// tunables and triggers as they are today, now the core's tunables and
    /// the adapter's constants (`lazy.rs` until B2c).
    #[test]
    fn b0_lazy_tunables_and_triggers() {
        let legacy = Tunables::legacy();
        // `FIRST_RUNS`, `CLASSIFY_WAIT_MS`, `IDLE_MS`, `CHECK_MS`,
        // `STARTING_OSD_MS`, `QUIET_AFTER_START_MS`, `STOP_WAIT_MS`.
        assert_eq!(legacy.first_run_limit, Some(8));
        assert_eq!(legacy.classify_wait, 5_000);
        assert_eq!(legacy.idle_timeout, 60_000);
        assert_eq!(legacy.idle_check, 250);
        assert_eq!(legacy.starting_notice, 400);
        assert_eq!(legacy.quiet_after_start, 1_000);
        assert_eq!(legacy.stop_wait, 20_000);
        assert_eq!(
            crate::supervisor::grace(
                legacy.policy(crate::supervisor::Class::Normal),
                crate::supervisor::StopReason::Idle,
                crate::supervisor::Then::Standby
            ),
            2_000
        );
        // §15 B6: the player's values flipped to the design's. The idle time
        // is 30 s (`IDLE_MS`, was 60 s) and `PARK_WAIT`, the grace the core
        // gives a stop for standby, 5 s (was 2 s); the rest as above.
        let target = Tunables::target(16, 16_384);
        assert_eq!(IDLE_MS, 30_000);
        assert_eq!(target.idle_timeout, IDLE_MS);
        assert_eq!(target.classify_wait, legacy.classify_wait);
        assert_eq!(target.idle_check, legacy.idle_check);
        assert_eq!(target.starting_notice, legacy.starting_notice);
        assert_eq!(target.quiet_after_start, legacy.quiet_after_start);
        assert_eq!(target.stop_wait, legacy.stop_wait);
        assert_eq!(PARK_WAIT, Duration::from_secs(5));
        assert_eq!(
            crate::supervisor::grace(
                target.policy(crate::supervisor::Class::Normal),
                crate::supervisor::StopReason::Idle,
                crate::supervisor::Then::Standby
            ),
            5_000
        );
        assert_eq!(
            TRIGGERS,
            [
                "commands.execute",
                "source.resolveRequest",
                "drops.drop",
                "ui.callback"
            ]
        );
        // §15 B6: on-demand start is everyone's; only a unit test's default
        // `OnDemand` has it off (the old path, removed in B7).
        assert!(!OnDemand::default().enabled);
        assert!(OnDemand::from_env().enabled);
    }

    /// §15 B6 (A17): the shared runner waits at least 30 s without a member
    /// (was 1 s), and at least the idle time.
    #[test]
    fn b6_runner_idle_exit_is_at_least_30_s_and_the_idle_time() {
        let on_demand = OnDemand {
            enabled: true,
            ..OnDemand::default()
        };
        let supervisor = new(&on_demand);
        assert!(runner_idle_exit(&supervisor) >= RUNNER_IDLE_EXIT_MIN);
        assert!(runner_idle_exit(&supervisor) >= supervisor.tunables().idle_timeout);
        let mut shorter = supervisor.tunables().clone();
        shorter.idle_timeout = 500;
        assert_eq!(
            runner_idle_exit(&Supervisor::new(shorter, 1)),
            RUNNER_IDLE_EXIT_MIN
        );
        let mut longer = supervisor.tunables().clone();
        longer.idle_timeout = 600_000;
        assert_eq!(runner_idle_exit(&Supervisor::new(longer, 1)), 600_000);
    }

    /// §15 B6: the player's supervisor runs the design's values
    /// (`Tunables::target`) whatever the environment says.
    #[test]
    fn b6_the_player_runs_the_target_values() {
        let supervisor = new(&OnDemand::from_env());
        let t = supervisor.tunables();
        let cores = std::thread::available_parallelism().map_or(2, std::num::NonZero::get);
        let target = Tunables::target(cores, memory::physical_mib());
        assert!(t.on_demand);
        assert_eq!(t.normal, target.normal);
        assert_eq!(t.resident, target.resident);
        assert_eq!(t.builtin, target.builtin);
        assert_eq!(t.normal.start_timeout, 60_000);
        assert_eq!(t.normal.stop_grace, 5_000);
        assert_eq!(t.normal.request_timeout, Some(300_000));
        assert!(!t.resident_idle_standby);
        assert_eq!(t.max_active, target.max_active);
    }

    /// `Lazy.parked_ids` (B2a's `parked_ids_follow_park_and_unpark`, ported
    /// to B2c and B2d): the adapter's held units and their cross-check
    /// follow the core's standby, by id and by pid: a replay at `add_unit`
    /// holds a unit, a disable lets it go (`Revoke`).
    #[test]
    fn parked_ids_follow_hold_and_unhold() {
        let held = |id: &str| {
            let origin = Origin {
                id: id.into(),
                directory: PathBuf::from(id),
                catalog: None,
                data_directory: None,
            };
            Held::new(id.into(), origin, id.into())
        };
        let mut on_demand = OnDemand {
            enabled: true,
            ..OnDemand::default()
        };
        let mut supervisor = new(&on_demand);
        for id in ["a.one", "b.two"] {
            let spec = crate::supervisor::UnitSpec::new(id, crate::supervisor::Class::Normal)
                .checkpoint(false);
            let unit = supervisor.add_unit(spec);
            let actions = supervisor.run_jobs(0);
            let holder = actions
                .iter()
                .find_map(|a| match a {
                    Action::Replay { unit: u, holder } if *u == unit => Some(holder.0),
                    _ => None,
                })
                .expect("replayed");
            on_demand.parked_ids.insert(id.into(), holder);
            on_demand.held.edit().insert(holder, held(id));
        }
        assert_eq!(supervisor.holder_of("a.one"), Some(Pid(1)));
        let a = supervisor.by_id("a.one").unwrap();
        supervisor.disable(a).unwrap();
        let actions = supervisor.run_jobs(1);
        assert!(actions.contains(&Action::Revoke {
            unit: a,
            pid: Pid(1)
        }));
        assert_eq!(supervisor.holder_of("a.one"), None);
        assert_eq!(supervisor.holder_of("b.two"), Some(Pid(2)));
        // The unit table holds the standby holder by id and by pid.
        let unit = supervisor.by_id("b.two").unwrap();
        assert_eq!(supervisor.by_pid(Pid(2)), Some(unit));
        assert_eq!(on_demand.parked_ids.get("b.two"), Some(&2));
    }
}
