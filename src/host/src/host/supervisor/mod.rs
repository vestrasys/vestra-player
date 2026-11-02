//! Host adapter of the plugin process supervisor
//! (doc/architecture/supervisor.md §2.2): executes the core's decisions and
//! feeds it the Host's events. Safe code only.
//!
//! B2a (§13): only identity. The Host still decides every start and stop;
//! at its choke points it tells the core's unit table which process runs a
//! plugin (`add_plugin` / `stop`: `unit_started` / `unit_stopped`) and which
//! owner holds a parked plugin's registrations (`Lazy::park` / `unpark`:
//! `unit_parked` / `unit_unparked`), allocates owner generations here
//! (`alloc_owner`) and keeps each activation in its unit. `Host.plugin_ids`
//! and `Lazy.parked_ids` stay one release, maintained at the same choke
//! points, as `debug_assert` cross-checks of the answers (B7 removes them).
use super::*;
use crate::supervisor::Tunables;

/// The Host's supervisor: `Tunables::legacy()` (today's values). Nothing
/// runs its jobs before B2b; the seed only feeds jitter, which `legacy()`
/// does not use.
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
