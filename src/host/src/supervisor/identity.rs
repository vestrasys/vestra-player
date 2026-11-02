//! B2a: the unit table owns identity (doc/architecture/supervisor.md §13
//! B2a, §17.8). The Host still decides every start and stop until B2b–B2d
//! move those decisions into jobs; at its choke points it tells the core
//! which process runs a unit (`adopt_process`, today's `Host.plugin_ids`)
//! or which owner holds a unit's registrations in standby (`adopt_holder`,
//! today's `Lazy.parked_ids`): one owner per unit (`Unit::hosted`, never
//! both), and it allocates every owner generation here
//! (`alloc_pid`, today's `next_generation += 1`). Activations live in their
//! unit (`activation.rs`, MOVE of `host/package_activation.rs`).
//!
//! None of this installs a job, arms a timer or queues an action; from B2b
//! the unit's state follows the owner (`Active` under a running process,
//! `Standby` under a holder, unless it is still deactivating) for the
//! waits (§17.9), and forgetting a holder cancels its waiting wake. A unit
//! loaded here gets its real spec from B2d's `add_unit`.
#![forbid(unsafe_code)]

use super::Supervisor;
use super::activation::{self, Error, LIMIT, MAX_SAFE, Run, Status};
use super::job::{Cause, JobKind, JobResult};
use super::unit::{Class, Hosted, Pid, Then, Unit, UnitId, UnitSpec, UnitState};

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

    /// The Host runs `u` as process `pid` (`Host::add_plugin`); the owner
    /// this displaced (a bug, `adopt`). B2b: the unit is `Active` under it
    /// (a start the Host ran, `host_start`), unless a stop of it is still
    /// deactivating (unreachable: every start waits for it, §17.9).
    pub fn adopt_process(&mut self, u: UnitId, pid: Pid) -> Option<(Pid, Hosted)> {
        let now = self.now;
        if let Some(unit) = self.units.get_mut(&u)
            && !unit.state.is_deactivating()
        {
            unit.state = UnitState::Active {
                pid,
                since: now,
                last_active: now,
            };
        }
        self.adopt(u, pid, Hosted::Running)
    }

    /// Process `pid` of `u` stopped (`Host::stop`, after
    /// `process_stopping`), or never ran.
    pub fn release_process(&mut self, u: UnitId, pid: Pid) {
        if let Some(unit) = self.units.get_mut(&u)
            && matches!(unit.state, UnitState::Active { pid: active, .. } if active == pid)
        {
            unit.state = UnitState::Inactive;
        }
        self.release(u, pid, Hosted::Running);
    }

    /// `u`'s registrations are held in standby under `pid` (lazy park, after
    /// its stop, or replay); the owner this displaced (a bug, `adopt`). B2b:
    /// a replayed unit is in `Standby` under it; a parked one lands there
    /// when its stop does (T16).
    pub fn adopt_holder(&mut self, u: UnitId, pid: Pid) -> Option<(Pid, Hosted)> {
        if let Some(unit) = self.units.get_mut(&u)
            && !unit.state.is_deactivating()
        {
            unit.state = UnitState::Standby {
                holder: pid,
                recorded: true,
                hold_until: None,
            };
        }
        self.adopt(u, pid, Hosted::Standby)
    }

    /// `pid` no longer holds `u`'s registrations in standby (woken,
    /// forgotten). B2b: a unit stopping for standby lands `Inactive`
    /// instead (its registrations went with the Host's `forget_parked`),
    /// and a wake still waiting for its stopped process is canceled, as
    /// today's `Parked.waiting` goes with the parked entry.
    pub fn release_holder(&mut self, u: UnitId, pid: Pid) {
        let mut deactivating = false;
        let mut wake = false;
        if let Some(unit) = self.units.get_mut(&u) {
            match unit.state {
                UnitState::Standby { holder, .. } if holder == pid => {
                    unit.state = UnitState::Inactive;
                }
                UnitState::Deactivating {
                    pid: stopping,
                    then: Then::Standby,
                    ..
                } if stopping == pid => deactivating = true,
                _ => {}
            }
            wake = unit.job.as_ref().is_some_and(|job| {
                job.handoff.is_some()
                    && matches!(
                        job.kind,
                        JobKind::Start {
                            cause: Cause::Trigger(_)
                        }
                    )
            });
        }
        if deactivating {
            self.set_then(u, Then::Inactive);
        }
        if wake {
            self.finish(u, JobResult::Canceled);
        }
        self.release(u, pid, Hosted::Standby);
    }

    /// The process the Host runs for plugin `id` (today's `Host::running`).
    pub fn process_of(&self, id: &str) -> Option<Pid> {
        self.hosted_as(id, Hosted::Running)
    }

    /// The owner holding plugin `id`'s registrations in standby (today's
    /// `Lazy::parked_id`).
    pub fn holder_of(&self, id: &str) -> Option<Pid> {
        self.hosted_as(id, Hosted::Standby)
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

    /// `u`'s one owner is now `pid`, as `how`; the owner it displaced, if
    /// any. The Host never gives a unit a second owner, nor changes the
    /// kind of the one it has, while it has one (§17.8: a wake unparks
    /// before its start, a park stops first, a start of a running or
    /// parked plugin returns, an install, reload or uninstall forgets the
    /// parked owner first, no park during a package operation, a
    /// development load is never parked): unreachable. If it happens it is
    /// a bug: a `debug_assert!` here, reported by the adapter in release,
    /// and the new owner replaces the old. Adopting the owner it already
    /// has displaces nothing.
    fn adopt(&mut self, u: UnitId, pid: Pid, how: Hosted) -> Option<(Pid, Hosted)> {
        let unit = self.units.get_mut(&u)?;
        let displaced = unit
            .hosted
            .replace((pid, how))
            .filter(|old| *old != (pid, how));
        debug_assert!(
            displaced.is_none(),
            "unit {} hosted twice: {displaced:?} then {:?}",
            unit.id,
            (pid, how)
        );
        self.map_pid(u, pid);
        if let Some((old, _)) = displaced.filter(|(old, _)| *old != pid) {
            self.unmap_pid(u, old);
        }
        displaced
    }

    /// `pid` is no longer `u`'s owner as `how`; another owner (a newer one,
    /// or `pid` as the other kind) stays.
    fn release(&mut self, u: UnitId, pid: Pid, how: Hosted) {
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        if unit.hosted == Some((pid, how)) {
            unit.hosted = None;
            self.unmap_pid(u, pid);
        }
    }

    fn hosted_as(&self, id: &str, how: Hosted) -> Option<Pid> {
        match self.units.get(self.by_id.get(id)?)?.hosted {
            Some((pid, hosted)) if hosted == how => Some(pid),
            _ => None,
        }
    }

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

    pub(super) fn map_pid(&mut self, u: UnitId, pid: Pid) {
        let previous = self.by_pid.insert(pid, u);
        debug_assert!(
            previous.is_none_or(|previous| previous == u),
            "pid {pid:?} belongs to one unit"
        );
    }

    /// `pid` leaves `by_pid` once nothing of `u` refers to it: the process,
    /// the standby holder, the activation, and what the job machinery
    /// keeps (state, alias, last pid, holder).
    pub(super) fn unmap_pid(&mut self, u: UnitId, pid: Pid) {
        let Some(unit) = self.units.get(&u) else {
            return;
        };
        let held = unit.hosted.is_some_and(|(hosted, _)| hosted == pid)
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

    /// `Host.plugin_ids` semantics: a stop of another pid leaves the
    /// process, its own stop clears it.
    #[test]
    fn process_index_follows_adopt_and_release() {
        let mut sup = Supervisor::<u32>::new(Tunables::legacy(), 1);
        let a = sup.load_unit("a.one");
        assert_eq!(sup.load_unit("a.one"), a);
        let b = sup.load_unit("b.two");
        assert_ne!(a, b);
        sup.adopt_process(a, Pid(7));
        sup.adopt_process(b, Pid(8));
        sup.adopt_process(a, Pid(7));
        assert_eq!(sup.process_of("a.one"), Some(Pid(7)));
        assert_eq!(sup.holder_of("a.one"), None);
        assert_eq!(sup.by_pid(Pid(7)), Some(a));
        sup.release_process(a, Pid(6));
        sup.release_holder(a, Pid(7));
        assert_eq!(sup.process_of("a.one"), Some(Pid(7)));
        sup.release_process(a, Pid(7));
        assert_eq!(sup.process_of("a.one"), None);
        assert_eq!(sup.by_pid(Pid(7)), None);
        sup.adopt_process(a, Pid(9));
        assert_eq!(sup.process_of("a.one"), Some(Pid(9)));
        assert_eq!(sup.process_of("b.two"), Some(Pid(8)));
        assert_eq!(sup.process_of("c.none"), None);
    }

    /// `Lazy.parked_ids` semantics, in a park's order (the process stops,
    /// then the stopped owner holds) and a wake's (the holder lets go, then
    /// a new process starts).
    #[test]
    fn holder_index_follows_adopt_and_release() {
        let mut sup = Supervisor::<u32>::new(Tunables::legacy(), 1);
        let a = sup.load_unit("a.one");
        sup.adopt_process(a, Pid(3));
        sup.release_process(a, Pid(3));
        sup.adopt_holder(a, Pid(3));
        assert_eq!(sup.process_of("a.one"), None);
        assert_eq!(sup.holder_of("a.one"), Some(Pid(3)));
        assert_eq!(sup.by_pid(Pid(3)), Some(a));
        sup.release_holder(a, Pid(4));
        sup.release_process(a, Pid(3));
        assert_eq!(sup.holder_of("a.one"), Some(Pid(3)));
        sup.release_holder(a, Pid(3));
        assert_eq!(sup.holder_of("a.one"), None);
        assert_eq!(sup.by_pid(Pid(3)), None);
        sup.adopt_process(a, Pid(4));
        assert_eq!(sup.process_of("a.one"), Some(Pid(4)));
        assert_eq!(sup.holder_of("a.one"), None);
    }

    /// A second owner while the unit has one is a Host bug (§17.8).
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "hosted twice")]
    fn a_second_owner_is_a_bug() {
        let mut sup = Supervisor::<u32>::new(Tunables::legacy(), 1);
        let a = sup.load_unit("a.one");
        sup.adopt_holder(a, Pid(3));
        sup.adopt_process(a, Pid(4));
    }

    /// So is the same owner changing kind (a running process taken as the
    /// standby holder without its stop).
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "hosted twice")]
    fn an_owner_changing_kind_is_a_bug() {
        let mut sup = Supervisor::<u32>::new(Tunables::legacy(), 1);
        let a = sup.load_unit("a.one");
        sup.adopt_process(a, Pid(3));
        sup.adopt_holder(a, Pid(3));
    }

    /// Adopting the owner the unit already has displaces nothing.
    #[test]
    fn adopting_the_same_owner_displaces_nothing() {
        let mut sup = Supervisor::<u32>::new(Tunables::legacy(), 1);
        let a = sup.load_unit("a.one");
        assert_eq!(sup.adopt_process(a, Pid(3)), None);
        assert_eq!(sup.adopt_process(a, Pid(3)), None);
        assert_eq!(sup.process_of("a.one"), Some(Pid(3)));
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
        // B2b: the state follows the owner (the waits read it), and still
        // no job, timer or action.
        assert!(matches!(
            sup.unit(a).unwrap().state,
            super::super::UnitState::Active { pid: p, .. } if p == pid
        ));
        sup.release_process(a, pid);
        sup.adopt_holder(a, pid);
        sup.release_holder(a, pid);
        assert!(sup.job(a).is_none());
        assert!(sup.run_jobs(20).is_empty());
        assert_eq!(
            sup.unit(a).unwrap().state,
            super::super::UnitState::Inactive
        );
    }
}
