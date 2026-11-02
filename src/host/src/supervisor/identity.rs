//! B2a: the unit table owns identity (doc/architecture/supervisor.md §13
//! B2a, §17.8). The Host still decides every start and stop until B2b–B2d
//! move those decisions into jobs; at its choke points it tells the core
//! which process runs a unit (`adopt_process`, today's `Host.plugin_ids`)
//! and which owner holds a unit's registrations in standby (`adopt_holder`,
//! today's `Lazy.parked_ids`), and it allocates every owner generation here
//! (`alloc_pid`, today's `next_generation += 1`). Activations live in their
//! unit (`activation.rs`, MOVE of `host/package_activation.rs`).
//!
//! None of this installs a job, arms a timer or queues an action: a unit
//! loaded here stays `Inactive` with no job until B2d's `add_unit`.
#![forbid(unsafe_code)]

use super::Supervisor;
use super::activation::{self, Error, LIMIT, MAX_SAFE, Run, Status};
use super::unit::{Class, Pid, Unit, UnitId, UnitSpec};

impl<P: Clone> Supervisor<P> {
    /// The unit of manifest `id`; a new one is loaded (no job, `Inactive`,
    /// enabled) when there is none. Its class is not read before B2d, whose
    /// `add_unit` gives the real spec.
    pub fn load_unit(&mut self, id: &str) -> UnitId {
        if let Some(u) = self.by_id.get(id) {
            return *u;
        }
        let u = UnitId(self.next_unit);
        self.next_unit += 1;
        self.by_id.insert(id.to_owned(), u);
        self.units
            .insert(u, Unit::new(&UnitSpec::new(id, Class::Normal)));
        u
    }

    /// A new owner generation (§3: the pid of a run, the identity on the
    /// wire and in every registry). Today's `next_generation += 1`: the
    /// first is 1, then +1 each, never reused.
    pub fn alloc_pid(&mut self) -> Pid {
        let pid = Pid(self.next_pid);
        self.next_pid += 1;
        pid
    }

    /// The pid the next `alloc_pid` returns, for a caller that refuses to
    /// allocate past the wire's safe integers (a development load).
    pub fn next_pid(&self) -> Pid {
        Pid(self.next_pid)
    }

    /// The Host runs `u` as process `pid` (`Host::add_plugin`).
    pub fn adopt_process(&mut self, u: UnitId, pid: Pid) {
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        let old = unit.process.replace(pid);
        self.map_pid(u, pid);
        if let Some(old) = old.filter(|old| *old != pid) {
            self.unmap_pid(u, old);
        }
    }

    /// Process `pid` of `u` stopped (`Host::stop`). Another pid of the unit
    /// (a newer run) stays.
    pub fn release_process(&mut self, u: UnitId, pid: Pid) {
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        if unit.process == Some(pid) {
            unit.process = None;
            self.unmap_pid(u, pid);
        }
    }

    /// `u`'s registrations are held in standby under `pid` (lazy park or
    /// replay).
    pub fn adopt_holder(&mut self, u: UnitId, pid: Pid) {
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        let old = unit.parked.replace(pid);
        self.map_pid(u, pid);
        if let Some(old) = old.filter(|old| *old != pid) {
            self.unmap_pid(u, old);
        }
    }

    /// `pid` no longer holds `u`'s registrations in standby (woken,
    /// forgotten). Another holder of the unit stays.
    pub fn release_holder(&mut self, u: UnitId, pid: Pid) {
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        if unit.parked == Some(pid) {
            unit.parked = None;
            self.unmap_pid(u, pid);
        }
    }

    /// The process the Host runs for plugin `id` (today's `Host::running`).
    pub fn process_of(&self, id: &str) -> Option<Pid> {
        self.units.get(self.by_id.get(id)?)?.process
    }

    /// The owner holding plugin `id`'s registrations in standby (today's
    /// `Lazy::parked_id`).
    pub fn holder_of(&self, id: &str) -> Option<Pid> {
        self.units.get(self.by_id.get(id)?)?.parked
    }

    // ---- activation (MOVE of `host/package_activation.rs`) -------------

    /// `Book::begin`: the activation of `u`'s run `pid`, due by `deadline`.
    /// One per unit: a unit's earlier activation that was never ended (a
    /// run that never started) is replaced and frees its place (A18 leak
    /// 1). At most `LIMIT` units activate at once (`Full`, until B3).
    pub fn activation_begin(&mut self, u: UnitId, pid: Pid, deadline: u64) -> Result<(), Error> {
        activation::identity(pid.0)?;
        activation::identity(deadline)?;
        if self.run_of(pid).is_some() {
            return Err(Error::Conflict);
        }
        let Some(unit) = self.units.get(&u) else {
            return Err(Error::Unknown);
        };
        let stale = unit.activation.as_ref().map(|run| run.pid);
        if stale.is_none() && self.activations == LIMIT {
            return Err(Error::Full);
        }
        let Some(unit) = self.units.get_mut(&u) else {
            return Err(Error::Unknown);
        };
        unit.activation = Some(Run::new(pid, deadline));
        self.map_pid(u, pid);
        match stale {
            Some(old) => self.unmap_pid(u, old),
            None => self.activations += 1,
        }
        Ok(())
    }

    /// `Book::status`: `Unknown` for a pid without an activation.
    pub fn activation_status(&mut self, pid: Pid, now: u64) -> Result<Status, Error> {
        activation::identity(pid.0)?;
        if now > MAX_SAFE {
            return Err(Error::Invalid);
        }
        match self.run_of(pid) {
            Some(run) => {
                run.activation.observe(now)?;
                Ok(run.activation.status())
            }
            None => Ok(Status::Unknown),
        }
    }

    /// `Book::ui_begin`.
    pub fn activation_ui_begin(
        &mut self,
        pid: Pid,
        registration: u64,
        now: u64,
    ) -> Result<(), Error> {
        activation::identity(registration)?;
        self.observed(pid, now)?.ui_begin(registration)
    }

    /// `Book::ui_finish`.
    pub fn activation_ui_finish(
        &mut self,
        pid: Pid,
        registration: u64,
        result: Result<(), String>,
        now: u64,
    ) -> Result<(), Error> {
        activation::identity(registration)?;
        if let Err(failure) = &result {
            activation::message(failure)?;
        }
        self.observed(pid, now)?.ui_finish(registration, result)
    }

    /// `Book::runtime_ready` (`runtime.ready`).
    pub fn activation_ready(&mut self, pid: Pid, now: u64) -> Result<(), Error> {
        self.observed(pid, now)?.runtime_ready()
    }

    /// `Book::runtime_failed` (`runtime.failed`, a fault, a stop).
    pub fn activation_failed(&mut self, pid: Pid, failure: String, now: u64) -> Result<(), Error> {
        activation::message(&failure)?;
        self.observed(pid, now)?.runtime_failed(failure)
    }

    /// `Book::remove`: the run's activation is over (stopped, or its spawn
    /// failed), with the UI requests waited on for it.
    pub fn activation_end(&mut self, pid: Pid) {
        let Some(u) = self.by_pid.get(&pid).copied() else {
            return;
        };
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        if unit.activation.as_ref().is_some_and(|run| run.pid == pid) {
            unit.activation = None;
            self.activations -= 1;
            self.unmap_pid(u, pid);
        }
    }

    /// The Host waits on UI request `reply` of `pid` for its activation
    /// (old `Host.activation_ui.insert`, after a successful `ui_begin`).
    pub fn activation_ui_wait(&mut self, pid: Pid, reply: u64) {
        if let Some(run) = self.run_of(pid) {
            run.ui_replies.insert(reply);
        }
    }

    /// The answer to `reply` of `pid` went out: whether the activation
    /// waited on it (old `Host.activation_ui.remove`).
    pub fn activation_ui_take(&mut self, pid: Pid, reply: u64) -> bool {
        self.run_of(pid)
            .is_some_and(|run| run.ui_replies.remove(&reply))
    }

    /// Whether `pid`'s activation waits on a UI request (old
    /// `Host.activation_ui` has an entry of that owner).
    pub fn activation_ui_waiting(&self, pid: Pid) -> bool {
        self.by_pid
            .get(&pid)
            .and_then(|u| self.units.get(u))
            .and_then(|unit| unit.activation.as_ref())
            .is_some_and(|run| run.pid == pid && !run.ui_replies.is_empty())
    }

    // ---- internals -------------------------------------------------------

    fn run_of(&mut self, pid: Pid) -> Option<&mut Run> {
        let u = self.by_pid.get(&pid)?;
        self.units
            .get_mut(u)?
            .activation
            .as_mut()
            .filter(|run| run.pid == pid)
    }

    /// `Book::entry`: the activation of `pid`, its deadline observed at
    /// `now`.
    fn observed(&mut self, pid: Pid, now: u64) -> Result<&mut activation::Activation, Error> {
        activation::identity(pid.0)?;
        if now > MAX_SAFE {
            return Err(Error::Invalid);
        }
        let run = self.run_of(pid).ok_or(Error::Unknown)?;
        run.activation.observe(now)?;
        Ok(&mut run.activation)
    }

    fn map_pid(&mut self, u: UnitId, pid: Pid) {
        let previous = self.by_pid.insert(pid, u);
        debug_assert!(
            previous.is_none_or(|previous| previous == u),
            "pid {pid:?} belongs to one unit"
        );
    }

    /// `pid` leaves `by_pid` once nothing of `u` refers to it: the process,
    /// the standby holder, the activation, and what the job machinery
    /// keeps (state, alias, last pid, holder).
    fn unmap_pid(&mut self, u: UnitId, pid: Pid) {
        let Some(unit) = self.units.get(&u) else {
            return;
        };
        let held = unit.process == Some(pid)
            || unit.parked == Some(pid)
            || unit.activation.as_ref().is_some_and(|run| run.pid == pid)
            || unit.pid() == Some(pid)
            || unit.alias == Some(pid)
            || unit.last_pid == Some(pid)
            || unit.holder == Some(pid)
            || matches!(unit.state, super::UnitState::Standby { holder, .. } if holder == pid);
        if !held && self.by_pid.get(&pid) == Some(&u) {
            self.by_pid.remove(&pid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Supervisor, Tunables};
    use super::*;

    #[test]
    fn pids_count_from_one_like_the_owner_generation() {
        let mut sup = Supervisor::<u32>::new(Tunables::legacy(), 1);
        assert_eq!(sup.next_pid(), Pid(1));
        assert_eq!(sup.alloc_pid(), Pid(1));
        assert_eq!(sup.alloc_pid(), Pid(2));
        assert_eq!(sup.next_pid(), Pid(3));
    }

    /// `Host.plugin_ids` semantics: the last adopted process wins, a stop
    /// of an older one leaves it.
    #[test]
    fn process_index_follows_adopt_and_release() {
        let mut sup = Supervisor::<u32>::new(Tunables::legacy(), 1);
        let a = sup.load_unit("a.one");
        assert_eq!(sup.load_unit("a.one"), a);
        let b = sup.load_unit("b.two");
        assert_ne!(a, b);
        sup.adopt_process(a, Pid(7));
        sup.adopt_process(b, Pid(8));
        assert_eq!(sup.process_of("a.one"), Some(Pid(7)));
        assert_eq!(sup.by_pid(Pid(7)), Some(a));
        sup.adopt_process(a, Pid(9));
        assert_eq!(sup.by_pid(Pid(7)), None);
        sup.release_process(a, Pid(7));
        assert_eq!(sup.process_of("a.one"), Some(Pid(9)));
        sup.release_process(a, Pid(9));
        assert_eq!(sup.process_of("a.one"), None);
        assert_eq!(sup.by_pid(Pid(9)), None);
        assert_eq!(sup.process_of("b.two"), Some(Pid(8)));
        assert_eq!(sup.process_of("c.none"), None);
    }

    /// `Lazy.parked_ids` semantics, and a pid both stopped and holding
    /// (a park keeps the stopped owner as the holder) stays in `by_pid`
    /// until both let go.
    #[test]
    fn holder_index_follows_adopt_and_release() {
        let mut sup = Supervisor::<u32>::new(Tunables::legacy(), 1);
        let a = sup.load_unit("a.one");
        sup.adopt_process(a, Pid(3));
        sup.adopt_holder(a, Pid(3));
        sup.release_process(a, Pid(3));
        assert_eq!(sup.process_of("a.one"), None);
        assert_eq!(sup.holder_of("a.one"), Some(Pid(3)));
        assert_eq!(sup.by_pid(Pid(3)), Some(a));
        sup.release_holder(a, Pid(4));
        assert_eq!(sup.holder_of("a.one"), Some(Pid(3)));
        sup.release_holder(a, Pid(3));
        assert_eq!(sup.holder_of("a.one"), None);
        assert_eq!(sup.by_pid(Pid(3)), None);
    }

    /// Loading units this way installs no job and queues nothing.
    #[test]
    fn identity_inputs_queue_no_action() {
        let mut sup = Supervisor::<u32>::new(Tunables::legacy(), 1);
        let a = sup.load_unit("a.one");
        let pid = sup.alloc_pid();
        sup.activation_begin(a, pid, 1_000).unwrap();
        sup.adopt_process(a, pid);
        assert!(sup.job(a).is_none());
        assert!(sup.run_jobs(10).is_empty());
        assert_eq!(
            sup.unit(a).unwrap().state,
            super::super::UnitState::Inactive
        );
    }
}
