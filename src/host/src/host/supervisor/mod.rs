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
//! B2b (§17.9): every wait for a stopped process is the core's. A stop the
//! Host runs makes its unit `Deactivating` (`unit_stopping`) until the
//! reaper's `Event::Reaped` and the playback worker's `BackendCleaned` are
//! in (`unit_reaped`, `unit_cleaned`); a start that must wait for it (an
//! enable, a reload, a package operation) is a hand-off job (`host_start`),
//! and the Host runs the start once the job says so (`supervisor_tick` →
//! `JobFinished`). An uninstall's data goes at the reap
//! (`remove_data_after` → `RemoveData`).
//!
//! B2c (§17.10): on-demand start (lazy activation, `VESTRA_LAZY=1` until
//! B6) is the core's. It decides when a run is idle (its idle timer, the
//! Standby job → `QueryResources` → `resources_answered`), what a run's
//! class is (`classified`) and when a lazy first run goes to standby, and
//! it wakes a unit in standby (`wake` → `Rekey` + `Spawn`, the held
//! triggers → `Prune` + `Deliver` at its ready). The adapter feeds it the
//! Host's half of each decision (the playback worker's answers, the Host's
//! own busy checks, what is on disk) and executes its actions
//! (`actions.rs`); recordings are `checkpoint.rs`, the test switches
//! `probe.rs`. Still the Host's until B2d: start-up replay and first runs
//! (`first_runs.rs`), and the starts and stops of package RPCs.
use super::*;
use crate::capabilities;
use crate::supervisor::{
    Action, Busy, Delivery, HostStart, HostStarted, JobId, JobResult, Then, Trigger, Tunables,
    UnitId,
};
use std::collections::BTreeSet;

pub(crate) mod actions;
pub(crate) mod checkpoint;
mod first_runs;
pub(crate) mod probe;
mod standby;

pub(crate) use first_runs::FirstRuns;
pub(crate) use standby::{Held, Origin};

/// How long a park waits for its process (lazy activation record §5.4
/// item 6; disable and quit keep 300 ms): the core's standby grace
/// (`Policy::standby_grace`, `Action::Terminate::grace`), which the Host's
/// `stop` gives the reaper until B2d's `Terminate` executor.
pub(crate) const PARK_WAIT: Duration = Duration::from_secs(2);
/// `VESTRA_IDLE_MS` when unset (the core's `Tunables::idle_timeout`).
const IDLE_MS: u64 = 60_000;
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

/// The Host's side of on-demand start (B2c; today's `Lazy`): what the
/// core's decisions need of the Host and the core does not keep.
#[derive(Default)]
pub(crate) struct OnDemand {
    /// `VESTRA_LAZY=1` (the core's `Tunables::on_demand`).
    pub enabled: bool,
    /// Units in standby by holder: what their kept registrations need.
    held: BTreeMap<u64, Held>,
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
    /// A standby's recording written before the core decided it (the run,
    /// its calls): the core's `Checkpoint` for it is done already.
    written: Option<(u64, usize)>,
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
    /// First runs (B2d: Start jobs of `cause: FirstRun`).
    pub first_runs: FirstRuns,
    /// Test switches for the registry diff (tests/lazy_activation.rs):
    /// see [`probe::Probe`].
    pub probe: probe::Probe,
}
impl OnDemand {
    pub fn from_env() -> Self {
        let enabled = std::env::var("VESTRA_LAZY").as_deref() == Ok("1");
        let tests = probe::test_executes(&std::env::var("VESTRA_TEST_EXECUTE").unwrap_or_default());
        Self {
            enabled,
            tests,
            probe: probe::Probe::from_env(),
            ..Self::default()
        }
    }
    /// A held unit's data, by holder.
    pub fn held(&self, owner: u64) -> Option<&Held> {
        self.held.get(&owner)
    }
}

/// The Host's supervisor under `Tunables::legacy()` (today's values), with
/// on-demand start as `VESTRA_LAZY` and `VESTRA_IDLE_MS` say. Wake events
/// wake nothing until start-up settled (`OnDemand::settled`). The seed only
/// feeds jitter, which `legacy()` does not use.
pub(crate) fn new(on_demand: &OnDemand) -> Supervisor<Value> {
    let idle_timeout = std::env::var("VESTRA_IDLE_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
        .unwrap_or(IDLE_MS);
    if on_demand.enabled {
        diag!("lazy: on, idle after {idle_timeout} ms");
    }
    let mut supervisor = Supervisor::new(
        Tunables {
            on_demand: on_demand.enabled,
            idle_timeout,
            // The Host's activation deadline times every start until B2d.
            start_timeout_by_host: true,
            ..Tunables::legacy()
        },
        1,
    );
    supervisor.set_muted(true);
    supervisor
}

/// The JavaScript-safe integer bound of owner identities on the wire.
const MAX_OWNER: u64 = 9_007_199_254_740_991;

fn event(message: &Value) -> Option<&str> {
    message["event"].as_str()
}

impl Host {
    /// A new owner generation (today's `next_generation += 1`).
    pub(crate) fn alloc_owner(&mut self) -> u64 {
        self.supervisor.alloc_pid().0
    }
    /// `alloc_owner`, refused past the wire's safe integers without using
    /// one up (a development load).
    pub(crate) fn alloc_owner_checked(&mut self) -> Result<u64> {
        if self.supervisor.next_pid().0 > MAX_OWNER {
            bail!("plugin owner identities exhausted");
        }
        Ok(self.alloc_owner())
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
    /// Plugin `id` runs as `owner` (`add_plugin`).
    pub(crate) fn unit_started(&mut self, id: &str, owner: u64) {
        self.plugin_ids.insert(id.to_owned(), owner);
        let now = self.now_ms();
        self.supervisor.set_now(now);
        let unit = self.supervisor.load_unit(id);
        if let Some(old) = self.supervisor.adopt_process(unit, Pid(owner)) {
            // A Host bug (unreachable, §17.8): the core's `debug_assert!`
            // stops a debug build; a release build reports it here (the
            // core does no I/O), and the new process has replaced the old.
            diag!("supervisor: unit {id} hosted twice: {} then {owner}", old.0);
        }
    }
    /// Plugin `id`'s process `owner` stopped (`stop`).
    pub(crate) fn unit_stopped(&mut self, id: &str, owner: u64) {
        if self.plugin_ids.get(id) == Some(&owner) {
            self.plugin_ids.remove(id);
        }
        if let Some(unit) = self.supervisor.by_id(id) {
            self.supervisor.release_process(unit, Pid(owner));
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
    // ---- B2b: stops and the waits for them (§17.9) ---------------------

    /// The Host stops process `owner` of plugin `id` for `reason`
    /// (`lifecycle.rs` `stop`, before the reaper takes it): its unit is
    /// deactivating until the process is reaped and the playback worker
    /// cleaned up after it (today's `Host.cleanup` entry). A park keeps the
    /// registrations and lands in standby; a fault lands failed; any other
    /// stop lands inactive (an uninstalled unit stays loaded until B2d).
    pub(crate) fn unit_stopping(&mut self, id: &str, owner: u64, reason: StopReason) {
        use crate::supervisor::StopReason as Reason;
        let (reason, then) = match reason {
            StopReason::Idle => (Reason::Idle, Then::Standby),
            StopReason::Failed => (Reason::Failed, Then::Failed),
            StopReason::Quit => (Reason::Quit, Then::Inactive),
            StopReason::Disable => (Reason::Disable, Then::Inactive),
            StopReason::Uninstall => (Reason::Uninstall, Then::Inactive),
            StopReason::Reload => (Reason::Reload, Then::Inactive),
            StopReason::Upgrade => (Reason::Upgrade, Then::Inactive),
        };
        let now = self.now_ms();
        self.supervisor.set_now(now);
        let unit = self.supervisor.load_unit(id);
        if let Some(old) = self
            .supervisor
            .process_stopping(unit, Pid(owner), reason, then)
        {
            // A start that did not wait for the previous process (the
            // precondition of `process_stopping`; B2d makes every start a
            // job): the older process is no longer tracked.
            diag!(
                "supervisor: unit {id} stops {owner} while {} still goes",
                old.0
            );
        }
    }
    /// The members of a runner the Host killed (`restart_shared`): gone
    /// with it, so their restart does not wait (today's behaviour, §17.7
    /// item 9c). B4's `runner_failed` takes this over.
    pub(crate) fn units_killed_with_runner(&mut self, owners: &[u64]) {
        for owner in owners {
            self.supervisor.member_killed(Pid(*owner));
        }
    }
    /// `Event::Reaped(owner)`: the reaper saw the process go. An
    /// uninstall's data owed at this reap goes now, as at today's
    /// `data_removals` (the starts waiting for it run in the next
    /// `supervisor_tick`, where today's ticks ran them).
    pub(crate) fn unit_reaped(&mut self, owner: u64) {
        let now = self.now_ms();
        self.supervisor.set_now(now);
        self.supervisor.process_reaped(Pid(owner), None);
        let actions = self.supervisor.take_actions();
        self.supervisor_execute(actions);
    }
    /// `Event::BackendCleaned(owner)`: the playback worker let go of what
    /// it held for that owner.
    pub(crate) fn unit_cleaned(&mut self, owner: u64) {
        let now = self.now_ms();
        self.supervisor.set_now(now);
        self.supervisor.backend_cleaned(Pid(owner));
        let actions = self.supervisor.take_actions();
        self.supervisor_execute(actions);
    }
    /// The process a start of plugin `id` must wait for: stopped, not yet
    /// reaped and cleaned up after (today's `!process_gone`,
    /// `parked_process`, `!cleanup_done`).
    pub(crate) fn previous_process(&self, id: &str) -> Option<u64> {
        let unit = self.supervisor.by_id(id)?;
        self.supervisor.previous_process(unit).map(|pid| pid.0)
    }
    /// Whether process `owner` was stopped and is not gone yet (today's
    /// `!process_gone(owner)`).
    pub(crate) fn process_going(&self, owner: u64) -> bool {
        self.supervisor.process_going(Pid(owner))
    }
    /// A start of plugin `id` the Host runs itself (§17.9): `Now`, or a job
    /// that waits for its previous process (`JobFinished`). `Busy`: a
    /// package RPC's job is installed (`JobMode::Fail`).
    pub(crate) fn host_start(
        &mut self,
        id: &str,
        start: HostStart,
    ) -> std::result::Result<HostStarted, Busy> {
        let Some(unit) = self.supervisor.by_id(id) else {
            return Ok(HostStarted::Now);
        };
        let now = self.now_ms();
        self.supervisor.set_now(now);
        self.supervisor.host_start(unit, start)
    }
    /// A package-RPC job of plugin `id` is installed (a reload, an enable
    /// or a package operation waiting for the previous process): the
    /// core's half of today's `package_busy`.
    pub(crate) fn package_job_busy(&self, id: &str) -> bool {
        self.supervisor
            .by_id(id)
            .is_some_and(|unit| self.supervisor.package_busy(unit))
    }
    /// The result of a package operation's wait `job` for plugin `id`:
    /// `None` while it waits (today's `Operation.waiting`).
    pub(crate) fn package_wait(&self, id: &str, job: JobId) -> Option<JobResult> {
        let Some(unit) = self.supervisor.by_id(id) else {
            return Some(JobResult::Failed("no unit".into()));
        };
        if self
            .supervisor
            .job(unit)
            .is_some_and(|installed| installed.id == job)
        {
            return None;
        }
        // Only a unit's last 8 results are kept: a lost one is not taken
        // for a process that went.
        Some(
            self.supervisor
                .job_result(unit, job)
                .cloned()
                .unwrap_or_else(|| {
                    JobResult::Failed("the wait's result is no longer known".into())
                }),
        )
    }
    /// An uninstall's data `directory` goes once no process of plugin `id`
    /// may write it (today's `data_removals`: at once without one, else at
    /// its reap).
    pub(crate) fn remove_data_after(&mut self, id: &str, directory: PathBuf) {
        let data = Value::String(directory.to_string_lossy().into_owned());
        let now_or_later = match self.supervisor.by_id(id) {
            Some(unit) => self.supervisor.remove_data_after(unit, data),
            None => Some(data),
        };
        if let Some(data) = now_or_later {
            remove_data(&data);
        }
    }
    /// A built-in's digest check is over: a reload that waits for it goes
    /// on (today's `reload_tick` `!verifying(id)`).
    pub(crate) fn unit_verified(&mut self, id: &str) {
        if let Some(unit) = self.supervisor.by_id(id) {
            self.supervisor.verify_finished(unit);
        }
    }
    /// Due timers and waits end (`run_jobs`): once per host tick (today's
    /// `lazy_tick` idle checks, wake loop and "Starting…" notices, and
    /// `reload_tick`). A classification the playback worker did not answer
    /// in time is resident first (`classify_timeouts`).
    pub(crate) fn supervisor_tick(&mut self) {
        let now = self.now_ms();
        self.classify_timeouts(now);
        if self.supervisor.next_deadline().is_none_or(|due| due > now) {
            return;
        }
        let actions = self.supervisor.run_jobs(now);
        self.supervisor_execute(actions);
    }
    /// Runs what the core queued since the last batch (an input's actions,
    /// in the order today's code did them inside that input).
    fn supervisor_actions(&mut self) {
        let actions = self.supervisor.take_actions();
        self.supervisor_execute(actions);
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
            Delivery::Now => Some(owner),
            Delivery::Forward(to) => {
                self.send(to.0, message.clone());
                None
            }
            Delivery::Started => {
                self.wake_started(owner);
                None
            }
            Delivery::Held | Delivery::Refused => {
                // Held only while the unit starts: a running plugin's trigger
                // is never kept from it (`legacy()` has no restart absorbing
                // one before B2d).
                debug_assert!(
                    !self.plugins.contains_key(&owner)
                        || self.supervisor.waking(Pid(owner)).is_some(),
                    "a trigger for running plugin {owner} was held"
                );
                self.supervisor_actions();
                None
            }
            // No unit runs it any more (a stopped run's late message), or
            // its unit failed: a process still there gets it, as before.
            Delivery::Dropped => self.plugins.contains_key(&owner).then_some(owner),
        }
    }
    /// A wake of `owner`'s unit was enqueued: it starts at once (today's
    /// `wake` started it inside the trigger), unless the process it was
    /// parked from is still going (one instance per data directory; the
    /// core's job starts it once that is gone, or fails after 20 s).
    fn wake_started(&mut self, owner: u64) {
        let Some(unit) = self.supervisor.by_pid(Pid(owner)) else {
            return;
        };
        self.supervisor.advance(unit);
        let actions = self.supervisor.take_actions();
        if !actions.iter().any(|a| matches!(a, Action::Spawn { .. })) {
            let id = self.unit_id(unit);
            diag!("lazy: {id} waits for its stopped process");
        }
        self.supervisor_execute(actions);
    }
    /// The manifest id of `unit`.
    fn unit_id(&self, unit: UnitId) -> String {
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
            _ => self.supervisor_actions(),
        }
    }
    /// Start-up is over: every installed plugin started, replayed or
    /// queued has run its first activation (as files opened at launch wait,
    /// `open_files_tick`).
    fn startup_settled(&mut self) -> bool {
        if !self.packages_started()
            || self.builtin_first_run_queued()
            || !self.on_demand.first_runs.queue.is_empty()
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
        self.on_demand.held.insert(owner, held);
    }
    /// `owner` is no longer held: its catalog goes.
    fn unhold(&mut self, owner: u64) -> Option<Held> {
        let held = self.on_demand.held.remove(&owner)?;
        if self.on_demand.parked_ids.get(&held.id) == Some(&owner) {
            self.on_demand.parked_ids.remove(&held.id);
        }
        held.release();
        Some(held)
    }
    /// A held owner's registrations go (it is disabled, uninstalled,
    /// reloaded or could not start).
    fn forget_entries(&mut self, owner: u64) {
        capabilities::revoke(self, owner, false);
        self.plugin_shaders_gone(owner);
        let _ = self.backend.commands.send(Command {
            reply: Reply { owner, id: 0 },
            method: "owner.disabled".into(),
            params: Value::Null,
        });
    }
    /// Plugin `id`, if in standby, leaves it: its entries go. With
    /// `drop_recording` its recording goes too, in standby or not and
    /// whether on-demand start is on or not (an uninstall, reload, install
    /// or upgrade: what it recorded is of no use any more). Whether it was
    /// in standby.
    pub(crate) fn forget_parked(&mut self, id: &str, drop_recording: bool) -> bool {
        if drop_recording {
            self.drop_recording(id);
        }
        let Some(owner) = self.parked_id(id) else {
            return false;
        };
        let guid = self
            .on_demand
            .held
            .get(&owner)
            .map(|held| held.guid.clone());
        if let Some(unit) = self.supervisor.by_id(id) {
            self.supervisor.standby_forgotten(unit);
            self.supervisor_actions();
        }
        // The core revoked its entries (`Revoke`); a holder it did not
        // name is let go here all the same.
        if self.unhold(owner).is_some() {
            diag!("supervisor: unit {id}: standby forgotten without a revoke");
            self.forget_entries(owner);
        }
        if drop_recording && let Some(guid) = guid {
            checkpoint::remove(&self.activation_dir(), &guid);
        }
        true
    }
    /// What the core knows of plugin `id`'s recording now: the file was
    /// removed (`None`) or read at a start (`Some(resident)`).
    pub(crate) fn checkpoint_known(&mut self, id: &str, checkpoint: Option<bool>) {
        let unit = self.supervisor.load_unit(id);
        self.supervisor.checkpoint_known(unit, checkpoint);
    }
    /// At start-up: register `id`'s recorded calls under a new owner with no
    /// process; the unit is in standby (§2.2; B2d: `add_unit`'s `Replay`).
    pub(crate) fn standby_replay(
        &mut self,
        id: &str,
        directory: &Path,
        manifest: &process::Manifest,
        key: &checkpoint::Key,
        recording: checkpoint::Recording,
    ) -> Result<()> {
        let owner = self.alloc_owner();
        let catalog = i18n::load(directory);
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
        let name = crate::packages::package_manager::display_name(manifest, catalog.as_deref());
        diag!(
            "lazy: {id} replayed ({} calls) as owner {owner}, not started",
            recording.calls.len()
        );
        let unit = self.supervisor.load_unit(id);
        self.supervisor.standby_replayed(unit, Pid(owner));
        self.hold(owner, Held::new(id.into(), key.guid.clone(), origin, name));
        self.probe_registry(owner, "replayed");
        Ok(())
    }
    /// `runtime.ready` of `owner` (today's `lazy_ready`). A run the Host
    /// started: its class is asked (`host_ready`, `classify`). A woken run:
    /// the core prunes the recorded entries it did not register again and
    /// delivers its held triggers, each checked against its live state;
    /// its class is asked before they run (what they open is not what it
    /// holds at its ready, §17.7 item 14).
    pub(crate) fn unit_ready(&mut self, owner: u64) {
        let now = self.now_ms();
        self.supervisor.set_now(now);
        self.supervisor.touch(Pid(owner), now);
        let Some(since) = self.supervisor.waking(Pid(owner)) else {
            self.probe_registry(owner, "ready");
            let recordable = self.checkpoint_key_of(owner).is_some();
            let first_run = self.on_demand.first_runs.running.contains(&owner);
            self.supervisor
                .host_ready(Pid(owner), recordable, first_run);
            self.supervisor_actions();
            self.classify(owner);
            return;
        };
        self.supervisor.process_ready(Pid(owner));
        let actions = self.supervisor.take_actions();
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
                        self.woken_ready(owner, now.saturating_sub(since), triggers);
                    }
                    self.deliver_held(pid.0, payload, &titles);
                }
                other => self.supervisor_execute(vec![other]),
            }
        }
        if !classified {
            self.woken_ready(owner, now.saturating_sub(since), triggers);
        }
    }
    /// A woken run is ready: its registries are probed and its class asked.
    fn woken_ready(&mut self, owner: u64, after: u64, triggers: usize) {
        self.probe_registry(owner, "woken");
        diag!("lazy: owner {owner} ready after {after} ms, {triggers} trigger(s)");
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
            // The run's ready, as the core has it (`host_ready` /
            // `process_ready`): one deadline with the core's classify timer.
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
        let first_run = self.on_demand.first_runs.running.contains(&owner);
        if resident.is_some() {
            // Its place goes to the next first run (also when the core closed
            // the classification already: today's timeout path frees it).
            self.on_demand.first_runs.running.remove(&owner);
        }
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
        let busy = resident.is_none()
            && first_run
            && (!self.host_idle(owner, now) || !self.write_ahead(owner, &backend, false));
        self.on_demand.answers.insert(owner, backend);
        self.supervisor.set_now(now);
        self.supervisor.classified_with(Pid(owner), resident, busy);
        self.supervisor_actions();
        self.on_demand.written = None;
    }
    /// What the Host itself knows of `owner` being idle (§2.3, today's
    /// `lazy_idle` without its idle time, which is the core's timer):
    /// installed, ready, not waking, its class not still asked, nothing of
    /// its in flight.
    fn host_idle(&mut self, owner: u64, now: u64) -> bool {
        if !self.plugins.contains_key(&owner)
            || self.supervisor.waking(Pid(owner)).is_some()
            || matches!(self.on_demand.asked.get(&owner), Some(Ask::Classify(_)))
            || self.checkpoint_key_of(owner).is_none()
        {
            return false;
        }
        let id = self.plugins[&owner].manifest.id.clone();
        matches!(
            self.activation_status(owner, now),
            Ok(activation::Status::Ready)
        ) && !self.dialog_busy
            && !self.package_rpc_busy(&id)
            && !self.commands.busy(owner)
            && !self.settings.busy(owner)
            && !self.supervisor.activation_ui_waiting(Pid(owner))
            && !self.source_state_replies.iter().any(|(o, _)| *o == owner)
    }
    /// A standby's recording, written before the core decides the standby
    /// (today's `park` writes it first and does not park when it fails):
    /// whether it was written. The core's `Checkpoint` for it is then done.
    fn write_ahead(&mut self, owner: u64, backend: &Value, resident: bool) -> bool {
        let Ok(calls) = self.standby_record(owner, backend) else {
            return false;
        };
        self.write_recording(owner, calls, resident)
    }
    fn write_recording(
        &mut self,
        owner: u64,
        calls: Vec<capabilities::Call>,
        resident: bool,
    ) -> bool {
        let Some(key) = self.checkpoint_key_of(owner) else {
            return false;
        };
        let count = calls.len();
        if let Err(error) = checkpoint::write(&self.activation_dir(), &key, calls, resident) {
            diag!(
                "lazy: {} not parked: its recording: {error}",
                self.plugin_id(owner)
            );
            let now = self.now_ms();
            self.supervisor.touch(Pid(owner), now);
            return false;
        }
        self.on_demand.written = Some((owner, count));
        true
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
        if used || !self.host_idle(owner, now) {
            self.supervisor.standby_refused(Pid(owner));
            self.supervisor_actions();
            return;
        }
        let recorded = match answer["live"].as_array().and_then(|live| live.first()) {
            Some(_) if !self.on_demand.probe.park_all => Err("a resource in the playback worker"),
            _ => self.standby_record(owner, answer),
        };
        let parks = match recorded {
            // A plugin resident at its ready that let go of everything since
            // is parked, but the next start still launches it (§3.4).
            Ok(calls) => {
                let resident = self
                    .supervisor
                    .by_pid(Pid(owner))
                    .and_then(|unit| self.supervisor.unit(unit))
                    .is_some_and(|unit| unit.resident);
                self.write_recording(owner, calls, resident)
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
                false
            }
        };
        self.supervisor.resources_answered(Pid(owner), !parks);
        self.supervisor_actions();
        self.on_demand.written = None;
    }
    /// The core's Standby job asks whether `owner` may go to standby
    /// (`Action::QueryResources`): the Host's own idle check first (today
    /// `lazy_tick` asked only an idle plugin), then the playback worker.
    pub(super) fn query_resources(&mut self, owner: u64) {
        let now = self.now_ms();
        if self.host_idle(owner, now) {
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
        self.supervisor.standby_refused(Pid(owner));
        self.supervisor_actions();
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
        self.on_demand.told.remove(&owner);
        self.on_demand.answers.remove(&owner);
    }
    /// Every host tick: the test hooks, and every `idle_check` (250 ms)
    /// whether start-up settled and which first runs may start. The idle
    /// checks, "Starting…" notices and classification timers are the
    /// core's (`supervisor_tick`).
    pub(crate) fn on_demand_tick(&mut self) {
        let now = self.now_ms();
        self.test_execute(now);
        if !self.on_demand.enabled {
            return;
        }
        self.probe_tick(now);
        if now < self.on_demand.next_check {
            return;
        }
        self.on_demand.next_check = now + self.supervisor.tunables().idle_check;
        if !self.on_demand.settled && self.startup_settled() {
            diag!("lazy: start-up settled");
            self.on_demand.settled = true;
            self.supervisor.settled(now);
        }
        self.first_runs_tick();
    }
    /// At quit (§2.1): every running installed plugin whose class came back
    /// this run is recorded (the core's `quit_checkpoints`): a resident one
    /// as resident, a lazy one with its registrations as they are now (left
    /// as it was when it holds a live resource at this moment: an open
    /// window, say).
    pub(crate) fn on_demand_quit(&mut self) {
        self.probe_quit();
        if !self.on_demand.enabled {
            return;
        }
        let directory = self.activation_dir();
        let mut runs = self.supervisor.quit_checkpoints();
        runs.sort_by_key(|(_, pid, _)| *pid);
        for (_, pid, resident) in runs {
            let owner = pid.0;
            let Some(key) = self.checkpoint_key_of(owner) else {
                continue;
            };
            let id = self.plugin_id(owner);
            let (calls, resident) = if resident {
                if checkpoint::read(&directory, &key).is_some_and(|r| r.resident_at_ready) {
                    continue;
                }
                (Vec::new(), true)
            } else {
                let backend = self
                    .on_demand
                    .answers
                    .get(&owner)
                    .cloned()
                    .unwrap_or(Value::Null);
                match self.standby_record(owner, &backend) {
                    Ok(calls) => (calls, false),
                    Err(reason) => {
                        diag!("lazy: {id} not recorded at quit: {reason}");
                        continue;
                    }
                }
            };
            match checkpoint::write(&directory, &key, calls, resident) {
                Ok(()) => diag!("lazy: {id} recorded at quit"),
                Err(error) => diag!("lazy: {id}: its recording: {error}"),
            }
        }
    }
}

/// Deletes an uninstalled plugin's data directory (`remove_data_after`).
fn remove_data(data: &Value) {
    if let Some(directory) = data.as_str() {
        let _ = std::fs::remove_dir_all(directory);
    }
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
        assert_eq!(first_runs::FIRST_RUNS, 8);
        assert_eq!(legacy.first_run_limit, Some(first_runs::FIRST_RUNS));
        assert_eq!(legacy.classify_wait, 5_000);
        assert_eq!(IDLE_MS, 60_000);
        assert_eq!(legacy.idle_timeout, IDLE_MS);
        assert_eq!(legacy.idle_check, 250);
        assert_eq!(legacy.starting_notice, 400);
        assert_eq!(legacy.quiet_after_start, 1_000);
        assert_eq!(legacy.stop_wait, 20_000);
        // `PARK_WAIT`: the grace the core gives a stop for standby.
        assert_eq!(PARK_WAIT, Duration::from_secs(2));
        assert_eq!(
            crate::supervisor::grace(
                legacy.policy(crate::supervisor::Class::Normal),
                crate::supervisor::StopReason::Idle,
                Then::Standby
            ),
            2_000
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
        let on_demand = OnDemand::default();
        assert!(!on_demand.enabled && on_demand.first_runs.free());
    }

    /// `Lazy.parked_ids` (B2a's `parked_ids_follow_park_and_unpark`, ported
    /// to B2c): the adapter's held units and their cross-check follow the
    /// core's standby, by id and by pid.
    #[test]
    fn parked_ids_follow_hold_and_unhold() {
        let held = |id: &str| {
            let origin = Origin {
                id: id.into(),
                directory: PathBuf::from(id),
                catalog: None,
                data_directory: None,
            };
            Held::new(id.into(), "guid".into(), origin, id.into())
        };
        let mut on_demand = OnDemand::default();
        let mut supervisor = new(&on_demand);
        for (id, owner) in [("a.one", 7), ("b.two", 9)] {
            let unit = supervisor.load_unit(id);
            supervisor.standby_replayed(unit, Pid(owner));
            on_demand.parked_ids.insert(id.into(), owner);
            on_demand.held.insert(owner, held(id));
        }
        assert_eq!(supervisor.holder_of("a.one"), Some(Pid(7)));
        let a = supervisor.by_id("a.one").unwrap();
        supervisor.standby_forgotten(a);
        assert_eq!(supervisor.holder_of("a.one"), None);
        assert_eq!(supervisor.holder_of("b.two"), Some(Pid(9)));
        // The unit table holds the standby holder by id and by pid.
        let unit = supervisor.by_id("b.two").unwrap();
        assert_eq!(supervisor.by_pid(Pid(9)), Some(unit));
        assert_eq!(on_demand.parked_ids.get("b.two"), Some(&9));
    }
}
