//! Host adapter of the plugin process supervisor
//! (doc/architecture/supervisor.md): executes the core's decisions and
//! feeds it the Host's events. Safe code only.
//!
//! Identity: the core's unit table says which process runs a
//! plugin (`process_of`) and which pid holds one in standby (`holder_of`);
//! owner generations are allocated here (`alloc_owner`) and each
//! activation is kept in its unit.
//!
//! Stops: every wait for a stopped process is the core's: a unit is
//! deactivating until the reaper's `Event::Reaped` and the playback
//! worker's `BackendCleaned` are in (`unit_reaped`, `unit_cleaned`).
//!
//! On-demand start (lazy activation) is the core's: when a run is
//! idle, its class, when a lazy first run goes to standby, and the wake of
//! a unit in standby. The adapter feeds it
//! the Host's half of each decision (the playback worker's answers, the
//! Host's own busy checks, what is on disk); recordings are
//! `checkpoint.rs`, the test switches `probe.rs`.
//!
//! Jobs: every start and stop is a job. Start-up adds the installed
//! units in the start order under the per-tick start budget (`load_queued`,
//! package_manager.rs); the core replays, verifies and starts them in its
//! lanes (built-ins first). Package RPCs
//! enqueue `start` / `stop` / `restart` jobs, crashes and failed
//! activations are the core's inputs, and the adapter executes what the
//! core decides (`actions.rs`: `Spawn`, `Verify`, `Replay`, `Terminate`,
//! `RemoveData`...) with today's bodies. `packages.list` reads the core's
//! `list_state`.
//!
//! Limits: third-party starts a few at a time, clicks first, a
//! bounded backlog whose refused triggers are answered (`actions.rs`
//! `held_unanswered`), start failures that keep their entry points. The
//! Host's busy checks are declared work in the core (`sync_work`), a
//! `source.resolveRequest` in flight a capped request (`request_sent`).
//!
//! Budget: the process budget for this machine's physical memory
//! and the memory pressure source (`memory.rs`): over the budget or under
//! pressure, idle third-party plugins go to standby through the idle
//! timer's Standby job.
//!
//! The player runs the design's values (`Tunables::new`: idle 30 s,
//! the start timeout 60 s, the stop grace 5 s, a 5 min cap on a request
//! awaiting the plugin's reply, a resident plugin never in standby)
//! with the spawn thread and the memory pressure source; the shared runner
//! waits at least 30 s without a member before it exits.
use super::revised::Revised;
use super::*;
use crate::binding_manager::KEYS_EVENT;
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
/// (`Policy::standby_grace`), which `Action::Terminate` carries:
/// the stop grace 5 s, as a disable, reload, uninstall or upgrade.
#[cfg(test)]
const PARK_WAIT: Duration = Duration::from_secs(5);
/// `VESTRA_IDLE_MS` when unset (the core's `Tunables::idle_timeout`): 30 s
/// (supervisor.md §6).
const IDLE_MS: u64 = 30_000;
/// How often the Host's busy checks and start-up are polled (`CHECK_MS`
/// of lazy activation).
const CHECK_MS: u64 = 250;
/// The shortest time the shared runner stays without a member before it exits
/// ("≥ 30 s"; `runner_idle_exit`).
const RUNNER_IDLE_EXIT_MIN: u64 = 30_000;
/// What a unit in standby is woken by (the recorded entries' triggers); held
/// for its ready while it starts, answered when it cannot.
pub(crate) const TRIGGERS: &[&str] = &[
    "commands.execute",
    "source.resolveRequest",
    "drops.drop",
    "ui.callback",
    // The command palette's first row activated (capabilities/palette_lead.rs).
    "palette.lead",
];

/// What of a run's resources in the playback worker (its answer to
/// `owner.resources`) its next activation takes up again from its own
/// settings (shaders, a processing chain, render options, a subtitle style,
/// the video view, upscaling it turned on): the worker's `held`, or `None`
/// when there is none. The rest of `live` is work in flight (a source
/// resolution, a snapshot or job, a producer stream or the audio device, an
/// upscaling request waiting, a subtitle track of the current media): it
/// keeps a run up while it lasts, never makes it resident.
fn held_in_worker(answer: &Value) -> Option<Value> {
    answer["held"]
        .as_array()
        .filter(|held| !held.is_empty())
        .map(|held| Value::Array(held.clone()))
}

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

/// What the Host itself knows keeps a run busy (today's `lazy_idle` without its
/// idle time, which is the core's timer): each is declared work in the core
/// (`WorkKind::Declared`, never capped). Being woken, not recordable or not
/// running are the core's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Busy {
    /// Its class is still asked (`Ask::Classify`).
    Classify = 1,
    /// Its activation is not over (`runtime.ready`, a UI request it waits
    /// on).
    Activation,
    /// A host dialog is open, or a confirmation waits on the sheet.
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
    /// Its sidebar tab is the current one (sidebar-tabs.md): a sidebar
    /// panel is never live, so its plugin stays Normal, and this keeps it
    /// running only while the tab is shown.
    ShownTab,
    /// What it holds in the playback worker is asked again after a request
    /// of its there (`worker_requested`), not answered yet: a first run
    /// whose class came back lazy is not parked before the answer says
    /// whether it took up something its next start must restore.
    WorkerAsk,
}

/// The Host's side of on-demand start (formerly `lazy.rs`'s `Lazy`): what the
/// core's decisions need of the Host and the core does not keep.
#[derive(Default)]
pub(crate) struct OnDemand {
    /// Units in standby by holder: what their kept registrations need.
    held: Revised<BTreeMap<u64, Held>>,
    /// The playback worker's answers waited for, by run.
    asked: BTreeMap<u64, Ask>,
    /// Each run's answer at its classification (`{ live, resolvers }`): its
    /// resolvers, for a checkpoint at quit (today's `Class.backend`).
    answers: BTreeMap<u64, Value>,
    /// Runs whose `owner.resources` was asked again after a request of theirs
    /// to the playback worker (`worker_requested`), not answered yet. Only
    /// asked when nothing else is, so its answer comes before any later ask's.
    refreshing: BTreeSet<u64>,
    /// Runs with a request to the playback worker since their last ask that
    /// could not be asked yet (another ask out): asked once that is answered.
    stale: BTreeSet<u64>,
    /// Plugins already logged as staying up, with the reason.
    told: BTreeMap<u64, &'static str>,
    /// The Host's busy checks of each run as the core has them (declared work,
    /// `Busy`): synced when the core asks (classification, standby) and every
    /// `CHECK_MS` while any is in.
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
    /// (`capabilities::events::WAKE_EVENTS`): such an event wakes the unit,
    /// which receives it at its ready.
    pub wakes: BTreeMap<u64, BTreeSet<String>>,
    /// Set around an announcement that wakes no unit in standby: the first
    /// `packages.changed` and `bindings.changed` of a run (nothing changed
    /// for a plugin that last ran in an earlier one).
    pub mute: bool,
    /// Each wake event's state as last announced in this run (`broadcast`):
    /// its digest (`state_digest`) and the event's data.
    /// `commands.keys` goes to one plugin at a time and is not here
    /// (`keys_sent` is what each plugin has).
    pub announced: BTreeMap<String, (u64, Value)>,
    /// Plugins parked in this run while wake events are muted (start-up):
    /// for each wake event they listen to, the digest of the state they
    /// last had (`None`: the event was not announced to them, nor its keys
    /// sent). Read once the mute ends (`wake_missed`), then empty.
    pub seen: BTreeMap<u64, BTreeMap<String, Option<u64>>>,
    /// The mute of start-up ended and `wake_missed` ran (once per run).
    pub caught_up: bool,
    /// Source events (`capabilities::SOURCE_EVENTS`) the start-up mute kept
    /// from plugins in standby that listen to them, in order, one per event
    /// name and source (`wake_trigger`'s key): delivered when the mute ends
    /// (`wake_missed`). A file opened at launch opens during the mute; a
    /// change is not a state the plugin can read again later.
    pub muted_sources: BTreeMap<u64, Vec<(String, Value)>>,
    /// Start-up settled (latched once per run; the core's `settled`).
    settled: bool,
    next_check: u64,
    /// The `CHECK_MS` checks are due at this time (`on_demand_tick`), run
    /// as the next step (`on_demand_check`), after what the step before
    /// asked of the interface was carried out (plan step S2f(3)).
    check_due: Option<u64>,
    tests: Vec<probe::TestExecute>,
    /// Test switches for the registry diff (tests/lazy_activation.rs):
    /// see [`probe::Probe`].
    pub probe: probe::Probe,
    /// Units held in standby after a fault, by holder: (unit, hold end). Their
    /// menu items show disabled with the reason meanwhile (`note_hold`).
    holds: BTreeMap<u64, (UnitId, u64)>,
    /// Owners whose run became ready (`runtime.ready`) while an initial UI
    /// request of theirs is still answered: ready once it is (`ui_finished`):
    /// when it was woken, if it was.
    ui_pending: BTreeMap<u64, Option<u64>>,
    /// The quit's stop has begun (`shutdown_plugins`): a `Terminate` is a
    /// quit's (no UI cleanup, one deadline).
    pub quitting: bool,
    /// The spawn thread (`worker.rs`), started at the first spawn it takes.
    spawner: Option<worker::Spawner>,
    /// Starts whose process the spawn thread creates, by pid.
    spawning: BTreeMap<u64, worker::Spawning>,
    /// The memory pressure source and the level the core has (supervisor.md §8).
    memory: memory::Memory,
}
impl OnDemand {
    /// A plugin's shown name by manifest id, as held (the command palette
    /// names the commands of a plugin in standby with it).
    pub fn name_of(&self, id: &str) -> Option<&str> {
        self.names.get(id).map(String::as_str)
    }
    pub fn from_env() -> Self {
        let tests = probe::test_executes(&std::env::var("VESTRA_TEST_EXECUTE").unwrap_or_default());
        Self {
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
    /// Every held unit, by owner (the interface's copy of their packages,
    /// plan step S3a(1)).
    pub fn held_all(&self) -> &BTreeMap<u64, Held> {
        &self.held
    }
    /// The runs whose declared work has `Busy::ShownTab` (their tab is
    /// current): `Core::tab_work` does nothing for them (plan step S3a(2)).
    pub fn shown_tab_owners(&self) -> BTreeSet<u64> {
        self.work
            .iter()
            .filter(|(_, busy)| busy.contains(&Busy::ShownTab))
            .map(|(owner, _)| *owner)
            .collect()
    }
    /// The shown names of the units ever held, by manifest id (`name_of`).
    pub fn names(&self) -> &BTreeMap<String, String> {
        &self.names
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

/// The Host's supervisor: the design's values for this machine
/// (`Tunables::new`: start-job limits for its cores, the process budget
/// for its physical memory), with the idle time from `VESTRA_IDLE_MS` (30 s
/// when unset; tests shorten it). Memory pressure is `memory.rs`'s, polled
/// in `on_demand_tick`. Wake events wake nothing until start-up settled
/// (`OnDemand::settled`). The seed feeds the start retries' jitter.
pub(crate) fn new() -> Supervisor<Value> {
    let idle_timeout = std::env::var("VESTRA_IDLE_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
        .unwrap_or(IDLE_MS);
    let cores = std::thread::available_parallelism().map_or(2, std::num::NonZero::get);
    let physical = memory::physical_mib();
    let tunables = Tunables::new(cores, physical);
    diag!(
        "supervisor: idle after {idle_timeout} ms, {} starts at once ({cores} cores), {} third-party processes ({physical} MiB)",
        tunables.max_active,
        tunables.process_budget
    );
    let mut supervisor = Supervisor::new(
        Tunables {
            idle_timeout,
            // Never longer than the idle time itself.
            first_run_quiet: tunables.first_run_quiet.min(idle_timeout),
            ..tunables
        },
        1,
    );
    supervisor.set_muted(true);
    supervisor
}

/// How long the shared runner stays without a member before it exits: at
/// least the idle time and at least 30 s, so built-ins going to standby one
/// after another do not stop and start it again (it was 1 s before built-ins
/// went to standby).
pub(crate) fn runner_idle_exit(supervisor: &Supervisor<Value>) -> u64 {
    supervisor.tunables().idle_timeout.max(RUNNER_IDLE_EXIT_MIN)
}

/// The JavaScript-safe integer bound of owner identities on the wire.
const MAX_OWNER: u64 = 9_007_199_254_740_991;

fn event(message: &Value) -> Option<&str> {
    message["event"].as_str()
}

impl Core {
    /// The running owner of plugin `id` (by `by_id`).
    pub(crate) fn running(&self, id: &str) -> Option<u64> {
        let owner = self.supervisor.process_of(id).map(|pid| pid.0);
        // Tests only: a scan per lookup made a debug start-up quadratic.
        #[cfg(test)]
        assert_eq!(
            owner,
            self.plugins
                .iter()
                .find(|(_, plugin)| plugin.manifest.id == id)
                .map(|(owner, _)| *owner),
            "the unit table follows plugins"
        );
        owner
    }
}
impl Core {
    /// The owner holding plugin `id`'s registrations in standby (the
    /// core's `holder_of`; today's `Lazy::parked_id`).
    pub(crate) fn parked_id(&self, id: &str) -> Option<u64> {
        let owner = self.supervisor.holder_of(id).map(|pid| pid.0);
        // Tests only: a scan per lookup made a debug start-up quadratic.
        #[cfg(test)]
        assert_eq!(
            owner,
            self.on_demand
                .held
                .iter()
                .find(|(_, held)| held.id == id)
                .map(|(owner, _)| *owner),
            "the unit table follows held"
        );
        owner
    }
}
impl Host {
    // ---- stops and the waits for them --------------------------
}
impl Core {
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
    /// A unit that landed in standby with a restart hold (after a crash, or a
    /// start that failed): until the hold is over its menu items show disabled
    /// with the reason, and a key or click shows it (a coordinator ruling).
    /// `on_demand_tick` lets go at its end.
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
}
impl Host {
    // ---- On-demand start ---------------------------------------
}
impl Core {
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
                // Held only while a unit starts (until its ready), or for the
                // next run of a restart: a running plugin's trigger is never
                // kept from it otherwise.
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
    /// A wake of `owner`'s unit was enqueued: it starts in this turn if a start
    /// slot is free (`max_active` third-party starts at once, an
    /// interactive trigger first), else when one frees; and only once the
    /// process it was parked from is gone (one instance per data directory; the
    /// core's job starts it then, or fails after 20 s).
    pub(crate) fn wake_started(&mut self, owner: u64) {
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
    /// `Host::broadcast` of `event`: each unit in standby that listened to it
    /// (a wake event) is woken and receives it at its ready; a newer one of the
    /// same event replaces one still held. Not while the player starts nor
    /// `quiet_after_start` after it settled (the core mutes them), nor for the
    /// first announcement of a run (`OnDemand::mute`): the plugins registering
    /// then are not a change, and a woken plugin reads the current state when
    /// it starts anyway.
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
        for owner in owners {
            self.wake_listener(owner, event, data);
        }
    }
    /// A plugin in standby that listens to the wake event `event` is woken
    /// to receive it (`data`) at its ready; nothing for one not in standby.
    /// `commands.keys` (player 0.7.6) wakes only the plugin whose keys
    /// changed, through this.
    pub(crate) fn wake_listener(&mut self, owner: u64, event: &str, data: &Value) {
        if !self.on_demand.held.contains_key(&owner) {
            return;
        }
        let now = self.now_ms();
        self.supervisor.set_now(now);
        let trigger = capabilities::wake_trigger(event, data);
        let delivery = self.supervisor.wake(Pid(owner), &trigger);
        if delivery == Delivery::Started {
            self.wake_started(owner);
        }
        if delivery == Delivery::Dropped
            && !self.on_demand.caught_up
            && self.supervisor.events_muted()
            && capabilities::SOURCE_EVENTS.contains(&event)
        {
            self.mute_source_event(owner, event, data);
        }
    }
    /// The start-up mute dropped source event `event` for `owner` in
    /// standby: kept for the mute's end (`OnDemand::muted_sources`), the
    /// newest of each event name and source.
    fn mute_source_event(&mut self, owner: u64, event: &str, data: &Value) {
        let id = crate::source_events::source_id(data);
        let max = self.supervisor.tunables().backlog_max;
        let held = self.on_demand.muted_sources.entry(owner).or_default();
        held.retain(|(name, old)| name != event || crate::source_events::source_id(old) != id);
        held.push((event.to_owned(), data.clone()));
        // The backlog's bound: the oldest go first.
        if held.len() > max {
            held.drain(..held.len() - max);
        }
    }
    /// `event`, a wake event, was announced with `data`: its state now
    /// (`OnDemand::announced`). `bindings.changed` carries none (a listener
    /// reads `bindings.list`): its state is the list it announces
    /// (`bindings_list_sent`).
    pub(crate) fn note_announced(&mut self, event: &str, data: &Value) {
        let state = if event == "bindings.changed" {
            &self.bindings_list_sent
        } else {
            data
        };
        let digest = state_digest(state);
        self.on_demand
            .announced
            .insert(event.to_owned(), (digest, data.clone()));
    }
    /// `owner`, running, is parked (or kept in standby) listening to the
    /// wake events `events`: while they are muted (start-up), the state of
    /// each it last had is noted (`OnDemand::seen`), for `wake_missed`.
    /// Nothing once that ran.
    pub(crate) fn note_parked_listener(&mut self, owner: u64, events: &BTreeSet<String>) {
        if self.on_demand.caught_up {
            return;
        }
        let manifest = self.plugins.get(&owner).map(|p| p.manifest.id.clone());
        let seen = events
            .iter()
            .map(|event| {
                let digest = if event == KEYS_EVENT {
                    manifest
                        .as_ref()
                        .and_then(|manifest| self.keys_sent.get(manifest))
                        .map(state_digest)
                } else {
                    self.on_demand
                        .announced
                        .get(event)
                        .map(|(digest, _)| *digest)
                };
                (event.clone(), digest)
            })
            .collect();
        self.on_demand.seen.insert(owner, seen);
    }
    /// The mute of start-up ended (`on_demand_tick`): wake events muted
    /// meanwhile were dropped, so a plugin parked before them would keep
    /// what it had at its park. Each plugin parked in this run and still in
    /// standby is woken once, with every wake event it listens to whose
    /// state is not the one it last had (`missed_wakes`): the event's last
    /// announcement, or its own current key labels (`commands.keys`). One
    /// whose events did not change, or which never ran in this run
    /// (replayed: it has what its recording has), is not woken. Whatever
    /// the order plugins parked and registered in.
    fn wake_missed(&mut self) {
        let seen = std::mem::take(&mut self.on_demand.seen);
        let mut current = BTreeMap::new();
        for (owner, events) in &seen {
            for event in events.keys() {
                let state = if event == KEYS_EVENT {
                    let keys = self.keys_state(*owner);
                    Some((state_digest(&keys), keys))
                } else {
                    self.on_demand.announced.get(event).cloned()
                };
                if let Some(state) = state {
                    current.insert((*owner, event.clone()), state);
                }
            }
        }
        let missed = missed_wakes(
            &seen,
            |owner, event| {
                self.on_demand.held.contains_key(&owner)
                    && self
                        .on_demand
                        .wakes
                        .get(&owner)
                        .is_some_and(|events| events.contains(event))
            },
            |owner, event| {
                current
                    .get(&(owner, event.to_owned()))
                    .map(|(digest, _)| *digest)
            },
        );
        for (owner, event) in missed {
            let Some((_, state)) = current.remove(&(owner, event.clone())) else {
                continue;
            };
            let id = self.held_id(owner).unwrap_or_default().to_owned();
            diag!("lazy: {id} parked before {event} changed at start-up, waking it");
            let data = if event == KEYS_EVENT {
                self.keys_woken(owner, state)
            } else {
                state
            };
            self.wake_listener(owner, &event, &data);
        }
        // The source events the mute kept, in their order, to each plugin
        // still in standby and still listening (one started meanwhile
        // started after them and reads the state, as every start does).
        let muted = std::mem::take(&mut self.on_demand.muted_sources);
        for (owner, events) in muted {
            for (event, data) in events {
                let listens = self.on_demand.held.contains_key(&owner)
                    && self
                        .on_demand
                        .wakes
                        .get(&owner)
                        .is_some_and(|events| events.contains(&event));
                if listens {
                    self.wake_listener(owner, &event, &data);
                }
            }
        }
        // Trigger conditions: evaluated once, and what the mute kept
        // delivered (events-and-triggers.md §4.9, T13).
        self.triggers_caught_up();
    }
    /// The manifest id of a plugin in standby.
    pub(crate) fn held_id(&self, owner: u64) -> Option<&str> {
        self.on_demand.held.get(&owner).map(|held| held.id.as_str())
    }
    /// A settings page of `owner` is shown: a unit in standby starts, so the
    /// page shows its current values and its callbacks have a receiver (nothing
    /// is delivered at its ready).
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
    /// The sidebar tab of `owner` became the current one while its plugin
    /// stands by (sidebar-tabs.md): an interactive trigger, as a shown
    /// settings page is; its activation mounts the panel again, which takes
    /// the tab over.
    pub(crate) fn panel_shown(&mut self, owner: u64) {
        self.page_shown(owner);
    }
    /// The current sidebar tab's running plugin (called every window tick,
    /// `tab_demand`): its declared work (`Busy::ShownTab`) is registered with
    /// the core at once, not at its next idle check, so neither the budget
    /// nor memory pressure (`pressure::reclaimable`) stops it while the tab is
    /// shown, also after it woke under a new owner. Already registered: a
    /// map lookup. The previous tab's work ends at the next `CHECK_MS` poll.
    pub(crate) fn tab_work(&mut self, owner: u64) {
        if self
            .on_demand
            .work
            .get(&owner)
            .is_some_and(|busy| busy.contains(&Busy::ShownTab))
        {
            return;
        }
        if self.plugins.contains_key(&owner) && self.checkpoint_key_of(owner).is_some() {
            self.sync_work(owner, false);
        }
    }
    /// Start-up settled (now, or at an earlier check): the plugins started
    /// at launch have run their first activation, and with it asked for
    /// the panels they mount at once (`open_files_tick`).
    pub(crate) fn startup_has_settled(&mut self) -> bool {
        self.on_demand.settled || self.startup_settled()
    }
    /// `owner` is held in standby now (its registrations kept under it).
    pub(super) fn hold(&mut self, owner: u64, held: Held) {
        self.on_demand
            .names
            .insert(held.id.clone(), held.name.clone());
        self.on_demand.held.edit().insert(owner, held);
    }
    /// `owner` is no longer held: its catalog goes.
    pub(super) fn unhold(&mut self, owner: u64) -> Option<Held> {
        let held = self.on_demand.held.edit().remove(&owner)?;
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
    /// `Action::Replay` (today's `lazy_replay` from `start_installed`): `id`'s
    /// recorded calls are registered under `holder`, a pid with no process; the
    /// unit is in standby. One that cannot be replayed starts instead
    /// (`replay_failed`).
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
            if let Err(error) = capabilities::replay(self, owner, &origin, call) {
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
    /// otherwise than from standby (boot, first run, enable, reload, install)
    /// that holds no trigger: the core's `process_ready`, then its class is
    /// asked (`classify`). A woken run, or any start that held triggers for its
    /// ready: the core prunes the recorded entries a woken run did not register
    /// again and delivers the held triggers, each checked against its live
    /// state; its class is asked before they run (what they open is not what it
    /// holds at its ready).
    pub(crate) fn unit_ready(&mut self, owner: u64) {
        let now = self.now_ms();
        self.supervisor.set_now(now);
        self.supervisor.touch(Pid(owner), now);
        let waking = self.supervisor.waking(Pid(owner));
        self.supervisor.process_ready(Pid(owner));
        // A start is ready once its initial UI requests are answered too: until
        // then it holds its triggers and its start slot (`ui_finished`).
        if self.supervisor.activating(Pid(owner)) {
            diag!("supervisor: owner {owner} ready, its UI not yet");
            self.on_demand.ui_pending.insert(owner, waking);
            return;
        }
        self.became_ready(owner, waking);
    }
    /// An initial UI request of `owner`'s activation was answered: a run whose
    /// `runtime.ready` came already is ready once its last one is.
    pub(crate) fn ui_finished(&mut self, owner: u64) {
        let now = self.now_ms();
        self.supervisor.set_now(now);
        self.supervisor.ui_finish(Pid(owner));
        if !self.supervisor.activating(Pid(owner))
            && let Some(waking) = self.on_demand.ui_pending.remove(&owner)
        {
            self.became_ready(owner, waking);
        }
    }
    /// `owner`'s run is ready: what was held for it runs, after its class was
    /// asked.
    fn became_ready(&mut self, owner: u64, waking: Option<u64>) {
        let now = self.now_ms();
        let actions = self.supervisor.run_jobs(now);
        let delivers = actions
            .iter()
            .any(|a| matches!(a, Action::Deliver { pid, .. } if pid.0 == owner));
        // Any other start (boot, first run, enable, reload, install): its class
        // is asked after its ready's actions ran. Such a start holds its
        // triggers for its ready too: those are delivered as a woken run's are,
        // after its class was asked.
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
    /// Classification at `runtime.ready` (supervisor.md §4): whether
    /// installed plugin `owner` holds a live resource. The host's half is
    /// read now (before any trigger it was woken by runs); the playback
    /// worker's half is asked, and its answer comes before anything the
    /// plugin asks it after this (`resources_answer`).
    fn classify(&mut self, owner: u64) {
        if self.checkpoint_key_of(owner).is_none() {
            self.development_ready(owner);
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
    /// A development load (`--plugin`) is never put in standby; at its ready
    /// the log says whether it could be, and if not why, for its author. Only
    /// the host's half: what the playback worker holds for it (a filter chain,
    /// a stream) is not asked.
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
    /// lazy first run goes to standby once it has been idle for
    /// `first_run_quiet` (with that 0: at once if the Host's own idle check
    /// passes and its recording is written, else as soon as it is idle).
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
        // A lazy first run goes to standby after a short quiet period
        // (`first_run_quiet`) unless busy, through the idle timer's Standby
        // job, which writes its recording then (`standby_answer`); busy, it
        // waits its idle timeout. The Host's checks are declared work
        // (`sync_work`), synced now so that the core's classification knows
        // them. Only without a quiet period does an idle one go at once: its
        // recording is written ahead (today's `park` does not park when that
        // fails), on the recorder thread, and the classification waits for it.
        let busy = resident.is_none() && first_run && self.sync_work(owner, false);
        if resident.is_none()
            && first_run
            && !busy
            && self.supervisor.tunables().first_run_quiet == 0
        {
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
        if self.on_demand.refreshing.contains(&owner) || self.on_demand.stale.contains(&owner) {
            busy.insert(Busy::WorkerAsk);
        }
        if !matches!(
            self.activation_status(owner, now),
            Ok(activation::Status::Ready)
        ) || self.supervisor.activation_ui_waiting(Pid(owner))
        {
            busy.insert(Busy::Activation);
        }
        // The interface's facts from the core's copy (plan step S2f(1)).
        #[cfg(debug_assertions)]
        {
            use crate::host::outbox::Fact;
            // Plan step S3a(3): a stale read is recorded, not asserted, so
            // the supervisor batch needs no allowance (`batch_columns`).
            self.check_settled(&[Fact::Sheet, Fact::Options, Fact::Columns]);
        }
        // A confirmation on the sheet counts as a dialog open (H5).
        if self.dialog_busy || self.confirm_waiting() {
            busy.insert(Busy::Dialog);
        }
        if self.package_rpc_busy(&id) {
            busy.insert(Busy::PackageRpc);
        }
        if self.commands.busy(owner) {
            busy.insert(Busy::Command);
        }
        // The options window opening (issued, not yet applied) counts as open
        // (S3a(3) review B3).
        let options_open =
            self.ui_facts.options_open || self.in_flight(crate::host::outbox::Mark::Options);
        if self.settings.busy(owner, options_open) {
            busy.insert(Busy::Settings);
        }
        if self.source_state_replies.iter().any(|(o, _)| *o == owner) {
            busy.insert(Busy::SourceState);
        }
        // The adjust column's current tab counts only while the column is
        // wanted shown (its panel is not needed while it is closed).
        // Any of its panels (one per slot) being a current tab.
        if self.mounts.values().filter(|m| m.plugin == owner).any(|m| {
            crate::host::policy_slot(m.spec.slot).is_some_and(|column| {
                let open = column != crate::ui::window_state::Slot::Adjust
                    || self.ui_facts.columns[crate::ui::sidebar_tabs::Column::Adjust.index()]
                        .wanted;
                // None while the column waits for its remembered tab.
                open && crate::ui::sidebar_tabs::Column::of(m.spec.slot).is_some_and(|tabs| {
                    crate::ui::sidebar_tabs::shown_tab(
                        m.spec.slot,
                        m.spec.owner,
                        self.current_tab(tabs),
                    )
                })
            })
        }) {
            busy.insert(Busy::ShownTab);
        }
        busy
    }
    /// The core's declared work of `owner` follows the Host's busy checks
    /// now (`recording`: its standby recording could not be written ahead):
    /// whether it is busy. Work that ended counts as use (the idle time
    /// starts again: an event in flight extends the lifetime).
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
    /// (today's `park` writes it first and does not park when it fails), on the
    /// recorder thread (supervisor.md): `after` waits for it (`recorded`). The
    /// core's `Checkpoint` for it is then done. Without the thread it is
    /// written here, as before.
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
        // when it was written in the same turn). Only while the core still
        // waits for it (its Standby job asking, its class not given yet):
        // after a reload replaced that job, its `drop_recording` stays done
        // (#154 review).
        let waited = match &recording.after {
            AfterRecording::Standby(_) => self.supervisor.standby_asking(Pid(owner)),
            AfterRecording::Classified(_) => self.supervisor.classifying(Pid(owner)),
        };
        if result.is_ok() && waited && self.plugins.contains_key(&owner) {
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
        if !self.on_demand.refreshing.contains(&owner)
            && matches!(self.on_demand.asked.get(&owner), Some(Ask::Classify(_)))
        {
            let now = self.now_ms();
            if self.probe_hold_answer(owner, answer, now) {
                return;
            }
        }
        self.resources_answer_now(owner, answer);
    }
    fn resources_answer_now(&mut self, owner: u64, answer: &Value) {
        // Asked only when nothing else was: this answer is the refresh's.
        if self.on_demand.refreshing.remove(&owner) {
            self.refresh_answer(owner, answer);
        } else {
            self.asked_answer(owner, answer);
        }
        // A request to the worker came while this ask was out.
        if self.on_demand.stale.remove(&owner) {
            self.worker_requested(owner);
        }
    }
    fn asked_answer(&mut self, owner: u64, answer: &Value) {
        match self.on_demand.asked.remove(&owner) {
            Some(Ask::Classify(ready)) => {
                if !self.plugins.contains_key(&owner) {
                    return;
                }
                // A request to the worker came after this ask (a first run
                // that sets its saved shaders just after its ready): the
                // answer may not count it. Asked again before a lazy class
                // parks it, within the same deadline (`classify_timeouts`).
                if held_in_worker(answer).is_none()
                    && self.on_demand.stale.remove(&owner)
                    && self
                        .backend
                        .commands
                        .send(Command {
                            reply: Reply { owner, id: 0 },
                            method: "owner.resources".into(),
                            params: Value::Null,
                        })
                        .is_ok()
                {
                    self.on_demand.asked.insert(owner, Ask::Classify(ready));
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
        // Busy now (declared work): the core keeps it up and asks
        // again after another idle time.
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
            // A plugin resident at its ready that let go of everything since is
            // parked, but the next start still launches it. The core is
            // answered once its recording is written (`recorded`).
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
                // Classified lazy, it holds something in the worker its next
                // activation takes up again: resident from now on
                // (`holds_live`). Work in flight there only keeps it up.
                self.acquired_live(owner, answer);
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
    /// A request of `owner` was handed to the playback worker (`forward`).
    /// Its class came from what it held at its ready; a run that may still
    /// go to standby can take up something live there after it (a plugin
    /// whose first run held no shader sets some later), so the worker is
    /// asked again. The ask follows the request on the worker's queue: its
    /// answer counts the request. One ask at a time per run; a request
    /// while another ask is out is asked after its answer.
    pub(crate) fn worker_requested(&mut self, owner: u64) {
        if self.checkpoint_key_of(owner).is_none()
            || self.on_demand.probe.park_all
            || !self.supervisor.may_stand_by(Pid(owner))
        {
            return;
        }
        if self.on_demand.refreshing.contains(&owner) || self.on_demand.asked.contains_key(&owner) {
            self.on_demand.stale.insert(owner);
            return;
        }
        let asked = self.backend.commands.send(Command {
            reply: Reply { owner, id: 0 },
            method: "owner.resources".into(),
            params: Value::Null,
        });
        if asked.is_ok() {
            self.on_demand.refreshing.insert(owner);
        }
    }
    /// The worker's answer to `worker_requested`'s ask: the run's resolvers
    /// as they are now (for a checkpoint at quit), and a held resource
    /// makes it resident (`acquired_live`).
    fn refresh_answer(&mut self, owner: u64, answer: &Value) {
        if !self.plugins.contains_key(&owner) {
            return;
        }
        if let Some(known) = self.on_demand.answers.get_mut(&owner) {
            *known = answer.clone();
        }
        self.acquired_live(owner, answer);
    }
    /// `answer` (the worker's `{ live, held, resolvers }` for `owner`) names a
    /// held resource (`held_in_worker`): a run that may still go to standby
    /// is resident from now on, and its checkpoint says so
    /// (`Supervisor::holds_live`). Work in flight (a resolution, a job, a
    /// stream, a request waiting) never does: it keeps the run up only, as
    /// before, and it is asked again after another idle period.
    fn acquired_live(&mut self, owner: u64, answer: &Value) {
        let Some(live) = held_in_worker(answer) else {
            return;
        };
        if self.on_demand.probe.park_all {
            return;
        }
        let now = self.now_ms();
        self.supervisor.set_now(now);
        if self.supervisor.holds_live(Pid(owner)) {
            diag!(
                "lazy: {} resident from now on: a resource in the playback worker {live}",
                self.plugin_id(owner)
            );
            self.supervisor_run();
        }
    }
    /// The core's Standby job asks whether `owner` may go to standby
    /// (`Action::QueryResources`): the Host's own busy checks first, synced
    /// as declared work (today `lazy_tick` asked only an idle plugin), then
    /// the playback worker. Busy (or the worker gone): answered busy.
    pub(super) fn query_resources(&mut self, owner: u64) {
        self.log_reclaim(owner);
        let now = self.now_ms();
        // This busy check reads the column facts as of the last drain (the
        // start of the event or step that runs the batch): not after a stop
        // earlier in this batch asked the interface to drop its panels, nor
        // after panel or layout commands queued earlier in the same turn
        // (plan step S2f(3), a named exception: core code cannot drain in
        // the middle of a turn).
        let busy = self.plugins.contains_key(&owner) && self.sync_work(owner, false);
        if self.plugins.contains_key(&owner) && !busy {
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
    /// checkpoint from the same ready (`Ask::Classify` holds the core's time),
    /// and this runs before `run_jobs` in the same tick, so it comes first and
    /// the Host logs and frees the place either way; a recorded run has only
    /// this one. An answer after it is dropped.
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
        self.on_demand.refreshing.remove(&owner);
        self.on_demand.stale.remove(&owner);
        // The core's work of the run went with it.
        self.on_demand.work.remove(&owner);
    }
    /// Every host tick: the test hooks, and every `CHECK_MS` (250 ms)
    /// whether start-up settled. The idle checks, first runs, "Starting…"
    /// notices and classification timers are the core's (`supervisor_tick`).
    pub(crate) fn on_demand_tick(&mut self) {
        let now = self.now_ms();
        self.test_execute(now);
        if !self.on_demand.holds.is_empty() {
            self.holds_tick(now);
        }
        self.probe_tick(now);
        self.probe_late_tick(now);
        self.memory_tick(now);
        // Trigger conditions on the clock (crate::triggers).
        self.triggers_tick();
        if now < self.on_demand.next_check {
            return;
        }
        self.on_demand.next_check = now + CHECK_MS;
        self.on_demand.check_due = Some(now);
    }
    /// The `CHECK_MS` checks `on_demand_tick` found due, as its own step
    /// (`Core::STEPS`): the driver carries out what that step asked of the
    /// interface (a park, a column toggle of the test switch) first, so the
    /// busy checks read the interface's facts as they are now (plan step
    /// S2f(3)).
    pub(crate) fn on_demand_check(&mut self) {
        let Some(now) = self.on_demand.check_due.take() else {
            return;
        };
        // Busy runs: their declared work ends when the Host's checks pass
        // (O(busy runs) every `CHECK_MS`).
        let busy = self.on_demand.work.keys().copied().collect::<Vec<_>>();
        for owner in busy {
            self.sync_work(owner, false);
        }
        if !self.on_demand.settled && self.startup_settled() {
            diag!("lazy: start-up settled");
            self.on_demand.settled = true;
            self.supervisor.settled(now);
        }
        // The mute of start-up ended (`quiet_after_start` after it settled):
        // what it dropped wakes now, where it changed something.
        if self.on_demand.settled && !self.on_demand.caught_up {
            self.supervisor.set_now(now);
            if !self.supervisor.events_muted() {
                self.on_demand.caught_up = true;
                self.wake_missed();
            }
        }
    }
}
impl Core {
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
    /// Plugin `owner` runs (`add_plugin`): the core's spawn of it ran.
    pub(crate) fn unit_started(&mut self, owner: u64) {
        let now = self.now_ms();
        self.supervisor.set_now(now);
        self.supervisor.process_spawned(Pid(owner), Ok(()));
    }
    /// A package-RPC job of plugin `id` is installed (a reload, an enable
    /// or a package operation waiting for the previous process): the
    /// core's half of today's `package_busy`.
    pub(crate) fn package_job_busy(&self, id: &str) -> bool {
        self.supervisor
            .by_id(id)
            .is_some_and(|unit| self.supervisor.package_busy(unit))
    }
    /// A trigger went to `owner`'s process: a `source.resolveRequest` is a
    /// host→plugin request awaiting its reply (`WorkKind::Request`, busy until
    /// answered, capped by the request timeout); its
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
    /// The manifest id of `unit`.
    pub(crate) fn unit_id(&self, unit: UnitId) -> String {
        self.supervisor
            .unit(unit)
            .map(|unit| unit.id.clone())
            .unwrap_or_default()
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
    /// An initial UI request of `owner`'s activation is waited on.
    pub(crate) fn ui_begun(&mut self, owner: u64) {
        self.supervisor.ui_begin(Pid(owner));
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
}

/// The digest of a wake event's state (`OnDemand::announced`, `seen`):
/// equal states, equal digests.
fn state_digest(state: &Value) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    state.to_string().hash(&mut hasher);
    hasher.finish()
}

/// The wakes the mute of start-up dropped (`Host::wake_missed`), in order:
/// for each plugin parked meanwhile (`seen`: the state of each wake event
/// it last had), each event it still listens to in standby (`listens`)
/// whose state now (`current`; `None`: never announced in this run) is
/// another. An event it never had (`None` in `seen`) differs from any
/// announced state: what it read itself at its start is not known.
fn missed_wakes(
    seen: &BTreeMap<u64, BTreeMap<String, Option<u64>>>,
    listens: impl Fn(u64, &str) -> bool,
    current: impl Fn(u64, &str) -> Option<u64>,
) -> Vec<(u64, String)> {
    let mut missed = Vec::new();
    for (owner, events) in seen {
        for (event, had) in events {
            if !listens(*owner, event) {
                continue;
            }
            if let Some(now) = current(*owner, event)
                && *had != Some(now)
            {
                missed.push((*owner, event.clone()));
            }
        }
    }
    missed
}

/// Deletes an uninstalled plugin's data directory (`Action::RemoveData`).
fn remove_data(directory: &Path) {
    let _ = std::fs::remove_dir_all(directory);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lazy activation's tunables and triggers (supervisor.md): the core's
    /// preset and the adapter's constants.
    #[test]
    fn lazy_tunables_and_triggers() {
        let t = Tunables::new(16, 16_384);
        // `CLASSIFY_WAIT_MS`, `IDLE_MS`, `CHECK_MS`, `STARTING_OSD_MS`,
        // `QUIET_AFTER_START_MS`, `STOP_WAIT_MS`.
        assert_eq!(t.classify_wait, 5_000);
        assert_eq!(IDLE_MS, 30_000);
        assert_eq!(t.idle_timeout, IDLE_MS);
        assert_eq!(CHECK_MS, 250);
        assert_eq!(t.starting_notice, 400);
        assert_eq!(t.quiet_after_start, 1_000);
        assert_eq!(t.first_run_quiet, 1_000);
        assert_eq!(t.stop_wait, 20_000);
        // `PARK_WAIT`, the grace the core gives a stop for standby.
        assert_eq!(PARK_WAIT, Duration::from_secs(5));
        assert_eq!(
            crate::supervisor::grace(
                t.policy(crate::supervisor::Class::Normal),
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
                "ui.callback",
                "palette.lead"
            ]
        );
    }

    /// Only what a plugin's next activation takes up again makes a lazy run
    /// resident (`acquired_live`, the standby path, the quit checkpoint):
    /// work in flight in the worker (a source resolution, a snapshot or a job
    /// such as Clip Export's, a producer stream or the audio device such as
    /// VestraNES's, an upscaling state read waiting) keeps it up only; held
    /// shaders (Shader Upscale), a filter chain or the view do.
    #[test]
    fn only_held_worker_resources_make_a_lazy_run_resident() {
        for live in [
            json!(["a source resolution in flight"]),
            json!(["a snapshot or a job"]),
            json!(["a producer stream", "the audio device"]),
            json!(["video upscaling"]),
            json!(["a subtitle track or style"]),
        ] {
            let answer = json!({"live": live, "held": [], "resolvers": []});
            assert_eq!(held_in_worker(&answer), None, "{answer}");
        }
        // An answer without `held` (none counts) never promotes.
        assert_eq!(held_in_worker(&json!({"live": ["video shaders"]})), None);
        assert_eq!(held_in_worker(&Value::Null), None);
        for held in [
            "video shaders",
            "a processing chain",
            "the video view",
            "the video render options",
            "a subtitle style",
            "video upscaling",
        ] {
            let answer = json!({"live": [held, "a snapshot or a job"], "held": [held]});
            assert_eq!(held_in_worker(&answer), Some(json!([held])), "{answer}");
        }
    }

    /// The shared runner waits at least 30 s without a member (was 1 s), and at
    /// least the idle time.
    #[test]
    fn b6_runner_idle_exit_is_at_least_30_s_and_the_idle_time() {
        let supervisor = new();
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

    /// The player's supervisor runs the design's values (`Tunables::new` for
    /// this machine) whatever the environment says.
    #[test]
    fn b6_the_player_runs_the_target_values() {
        let supervisor = new();
        let t = supervisor.tunables();
        let cores = std::thread::available_parallelism().map_or(2, std::num::NonZero::get);
        let preset = Tunables::new(cores, memory::physical_mib());
        assert_eq!(t.normal, preset.normal);
        assert_eq!(t.resident, preset.resident);
        assert_eq!(t.builtin, preset.builtin);
        assert_eq!(t.normal.start_timeout, 60_000);
        assert_eq!(t.normal.stop_grace, 5_000);
        assert_eq!(t.normal.request_timeout, Some(300_000));
        assert!(!t.resident.idle_standby);
        assert_eq!(t.max_active, preset.max_active);
    }

    /// The core's standby holders follow its standby, by id and by pid: a
    /// replay at `add_unit` holds a unit, a disable lets it go (`Revoke`).
    #[test]
    fn standby_holders_follow_replay_and_disable() {
        let mut supervisor = new();
        for id in ["a.one", "b.two"] {
            let spec = crate::supervisor::UnitSpec::new(id, crate::supervisor::Class::Normal)
                .checkpoint(false);
            let unit = supervisor.add_unit(spec);
            let actions = supervisor.run_jobs(0);
            assert!(
                actions
                    .iter()
                    .any(|a| matches!(a, Action::Replay { unit: u, .. } if *u == unit)),
                "replayed"
            );
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
    }

    /// The wakes start-up's mute dropped (`wake_missed`): a plugin parked
    /// meanwhile is woken for each event whose state is not the one it last
    /// had, and only then; the order plugins parked and registered in does
    /// not matter, only the states.
    #[test]
    fn a_plugin_parked_at_start_up_is_woken_only_for_what_changed() {
        let list = |n: usize| json!({"commands": (0..n).collect::<Vec<_>>()});
        let (a, b) = (state_digest(&list(337)), state_digest(&list(372)));
        assert_ne!(a, b);
        assert_eq!(a, state_digest(&list(337)));
        let had = |events: &[(&str, Option<u64>)]| {
            events
                .iter()
                .map(|(event, digest)| ((*event).to_owned(), *digest))
                .collect::<BTreeMap<_, _>>()
        };
        let seen = BTreeMap::from([
            // Parked before the third-party plugins registered their commands.
            (1, had(&[("bindings.changed", Some(a))])),
            // Parked after: it has the current list.
            (2, had(&[("bindings.changed", Some(b))])),
            // Its palette never changed in this run (never announced).
            (3, had(&[("appearance.changed", None)])),
            // Its keys changed, its packages did not.
            (
                4,
                had(&[("commands.keys", Some(7)), ("packages.changed", Some(9))]),
            ),
            // Parked before any announcement: what it read itself is not known.
            (5, had(&[("packages.changed", None)])),
            // Woken since (no longer in standby).
            (6, had(&[("bindings.changed", Some(a))])),
        ]);
        let current = |owner: u64, event: &str| match (owner, event) {
            (_, "bindings.changed") => Some(b),
            (_, "appearance.changed") => None,
            (4, "commands.keys") => Some(8),
            (_, "packages.changed") => Some(9),
            _ => None,
        };
        let missed = missed_wakes(&seen, |owner, _| owner != 6, current);
        assert_eq!(
            missed,
            [
                (1, "bindings.changed".to_owned()),
                (4, "commands.keys".to_owned()),
                (5, "packages.changed".to_owned()),
            ]
        );
        // A normal start: every state as each had it, nothing is woken.
        let same = |owner: u64, event: &str| seen[&owner][event];
        assert!(missed_wakes(&seen, |_, _| true, same).is_empty());
        // No longer listening to the event: not woken for it.
        assert!(missed_wakes(&seen, |_, _| false, current).is_empty());
    }
}
