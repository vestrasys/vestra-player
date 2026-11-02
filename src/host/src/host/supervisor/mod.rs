//! Host adapter of the plugin process supervisor
//! (doc/architecture/supervisor.md §2.2): executes the core's decisions and
//! feeds it the Host's events. Safe code only.
//!
//! B2a (§13): identity. The Host still decides every start and stop; at its
//! choke points it tells the core's unit table which process runs a plugin
//! (`add_plugin` / `stop`: `unit_started` / `unit_stopped`) and which owner
//! holds a parked plugin's registrations (`Lazy::park` / `unpark`:
//! `unit_parked` / `unit_unparked`), allocates owner generations here
//! (`alloc_owner`) and keeps each activation in its unit. `Host.plugin_ids`
//! and `Lazy.parked_ids` stay as `debug_assert` cross-checks of the answers
//! (B7 removes them).
//!
//! B2b (§17.9): every wait for a stopped process is the core's. A stop the
//! Host runs makes its unit `Deactivating` (`unit_stopping`) until the
//! reaper's `Event::Reaped` and the playback worker's `BackendCleaned` are
//! in (`unit_reaped`, `unit_cleaned`); a start that must wait for it (a
//! wake, an enable, a reload, a package operation) is a hand-off job
//! (`host_start`), and the Host runs the start once the job says so
//! (`supervisor_tick` → `JobFinished`). An uninstall's data goes at the
//! reap (`remove_data_after` → `RemoveData`).
use super::*;
use crate::supervisor::{
    Action, Busy, HostStart, HostStarted, JobId, JobKind, JobResult, Then, Tunables,
};

/// The Host's supervisor: `Tunables::legacy()` (today's values). The seed
/// only feeds jitter, which `legacy()` does not use.
pub(crate) fn new() -> Supervisor<Value> {
    Supervisor::new(Tunables::legacy(), 1)
}

/// The JavaScript-safe integer bound of owner identities on the wire.
const MAX_OWNER: u64 = 9_007_199_254_740_991;

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
        let displaced = self.supervisor.adopt_process(unit, Pid(owner));
        report_hosted_twice(id, displaced, owner, "Running");
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
            // Unreachable (every start waits for the previous process); the
            // core's `debug_assert!` stops a debug build.
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
            return Some(JobResult::Done);
        };
        if self
            .supervisor
            .job(unit)
            .is_some_and(|installed| installed.id == job)
        {
            return None;
        }
        Some(
            self.supervisor
                .job_result(unit, job)
                .cloned()
                .unwrap_or(JobResult::Done),
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
    /// Due waits end (`run_jobs`) and their starts run: once per host tick
    /// (today's `lazy_tick` wake loop and `reload_tick`).
    pub(crate) fn supervisor_tick(&mut self) {
        let now = self.now_ms();
        if self.supervisor.next_deadline().is_none_or(|due| due > now) {
            return;
        }
        let actions = self.supervisor.run_jobs(now);
        self.supervisor_execute(actions);
    }
    /// Runs what the core decided. In B2b only the waits' ends and the data
    /// removals are the core's; everything else the Host still does itself
    /// at the same points, so those actions are not run here (B2c, B2d).
    fn supervisor_execute(&mut self, actions: Vec<Action<Value>>) {
        for action in actions {
            match action {
                Action::JobFinished {
                    unit, kind, result, ..
                } => {
                    let Some(id) = self.supervisor.unit(unit).map(|unit| unit.id.clone()) else {
                        continue;
                    };
                    match kind {
                        JobKind::Start {
                            cause: crate::supervisor::Cause::Trigger(_),
                        } => self.wake_finished(&id, result),
                        JobKind::Start {
                            cause: crate::supervisor::Cause::Enable,
                        }
                        | JobKind::Restart { image: None, .. } => {
                            self.restart_finished(&id, result);
                        }
                        // A package operation reads its wait's result
                        // (`advance_package`, `package_wait`).
                        _ => {}
                    }
                }
                Action::RemoveData {
                    data: Some(data), ..
                } => remove_data(&data),
                // The Host revokes, records and notifies at its own choke
                // points until B2c (`forget_parked`, `wake_failed`, `park`)
                // and B2d (`stop`, package operations); it spawns and stops
                // processes itself until B2d; nothing announces through the
                // core before B2d's `list_state`.
                Action::RemoveData { data: None, .. }
                | Action::Spawn { .. }
                | Action::Verify { .. }
                | Action::Replay { .. }
                | Action::Rekey { .. }
                | Action::Prune { .. }
                | Action::Deliver { .. }
                | Action::Answer { .. }
                | Action::QueryResources { .. }
                | Action::Checkpoint { .. }
                | Action::DropCheckpoint { .. }
                | Action::Terminate { .. }
                | Action::Kill { .. }
                | Action::KillRunner { .. }
                | Action::Revoke { .. }
                | Action::Notify { .. }
                | Action::ListChanged { .. } => {}
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

/// The parked-owner index's choke points (`Lazy::park`, `Lazy::unpark`,
/// `Lazy::parked_id`) take the supervisor: `Lazy` is not the `Host`.
pub(crate) fn unit_parked(supervisor: &mut Supervisor<Value>, id: &str, owner: u64) {
    let unit = supervisor.load_unit(id);
    let displaced = supervisor.adopt_holder(unit, Pid(owner));
    report_hosted_twice(id, displaced, owner, "Standby");
}
/// A unit given a second owner, or its owner taken as the other kind, is
/// a Host bug (unreachable, §17.8): the core's `debug_assert!` stops a debug
/// build; a release build reports it here (the core does no I/O), and the
/// new owner has replaced the old.
fn report_hosted_twice(
    id: &str,
    displaced: Option<(Pid, impl std::fmt::Debug)>,
    owner: u64,
    how: &str,
) {
    if let Some((old, was)) = displaced {
        diag!(
            "supervisor: unit {id} hosted twice: {} ({was:?}) then {owner} ({how})",
            old.0
        );
    }
}
pub(crate) fn unit_unparked(supervisor: &mut Supervisor<Value>, id: &str, owner: u64) {
    if let Some(unit) = supervisor.by_id(id) {
        supervisor.release_holder(unit, Pid(owner));
    }
}
pub(crate) fn parked_id(supervisor: &Supervisor<Value>, id: &str) -> Option<u64> {
    supervisor.holder_of(id).map(|pid| pid.0)
}
