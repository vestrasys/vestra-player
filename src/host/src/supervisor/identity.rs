//! B2a: the unit table owns identity (doc/architecture/supervisor.md §13
//! B2a, §17.8). The Host still decides the starts and stops B2d moves into
//! jobs; at its choke points it tells the core which process runs a unit
//! (`adopt_process`, today's `Host.plugin_ids`): one process per unit
//! (`Unit::hosted`), and it allocates every owner generation here
//! (`alloc_pid`, today's `next_generation += 1`). Activations live in their
//! unit (`activation.rs`, MOVE of `host/package_activation.rs`).
//!
//! None of this installs a job or queues an action; from B2b the unit's
//! state follows the process (`Active`, unless it is still deactivating)
//! for the waits (§17.9). Since B2c standby is the core's own state
//! (`UnitState::Standby`, reached through the Standby job and left through
//! `wake`, §17.10); the Host's starts that are not jobs yet report their
//! readiness and what they found on disk here (`host_ready`,
//! `checkpoint_known`, `standby_replayed`, `standby_forgotten`; removed in
//! B2d). A unit loaded here gets its real spec from B2d's `add_unit`.
#![forbid(unsafe_code)]

use super::activation::{self, Error, LIMIT, MAX_SAFE, Run, Status};
use super::job::{Cause, Job, JobKind, JobResult};
use super::timers::{TimerKind, Timers};
use super::trigger::AnswerReason;
use super::unit::{Class, Ms, Pid, Then, UnitId, UnitSpec, UnitState};
use super::{Action, Supervisor};

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
            .insert(u, super::Unit::new(&UnitSpec::new(id, Class::Normal)));
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

    /// The Host runs `u` as process `pid` (`Host::add_plugin`); the process
    /// this displaced (a bug, `adopt`). B2b: the unit is `Active` under it
    /// (a start the Host ran, `host_start`), unless a stop of it is still
    /// deactivating (unreachable: every start waits for it, §17.9). B2c: a
    /// process the core spawned itself (a wake's `Spawn`) stays
    /// `Activating` until `process_ready`.
    pub fn adopt_process(&mut self, u: UnitId, pid: Pid) -> Option<Pid> {
        let now = self.now;
        if let Some(unit) = self.units.get_mut(&u)
            && !unit.state.is_deactivating()
            && !matches!(unit.state, UnitState::Activating { pid: Some(spawned), .. } if spawned == pid)
        {
            unit.state = UnitState::Active {
                pid,
                since: now,
                last_active: now,
            };
        }
        self.adopt(u, pid)
    }

    /// Process `pid` of `u` stopped (`Host::stop`, after
    /// `process_stopping`), or never ran.
    pub fn release_process(&mut self, u: UnitId, pid: Pid) {
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        if matches!(unit.state, UnitState::Active { pid: active, .. } if active == pid) {
            unit.state = UnitState::Inactive;
        }
        if unit.hosted == Some(pid) {
            unit.hosted = None;
            self.unmap_pid(u, pid);
        }
    }

    /// The process the Host runs for plugin `id` (today's `Host::running`).
    pub fn process_of(&self, id: &str) -> Option<Pid> {
        self.units.get(self.by_id.get(id)?)?.hosted
    }

    /// The owner holding plugin `id`'s registrations in standby (today's
    /// `Lazy::parked_id`): the holder of a unit in `Standby`, or the
    /// process of one stopping for standby (its registrations are kept
    /// under it from the `Terminate` on, as today's park holds them at
    /// once).
    pub fn holder_of(&self, id: &str) -> Option<Pid> {
        match self.units.get(self.by_id.get(id)?)?.state {
            UnitState::Standby { holder, .. } => Some(holder),
            UnitState::Deactivating {
                pid,
                then: Then::Standby,
                ..
            } => Some(pid),
            _ => None,
        }
    }

    /// When the core's own start of `pid` spawned it, while it is not ready
    /// yet (today's `lazy.waking`): triggers and wake events for it are
    /// held for its ready. The Host's own starts are `Active` at once and
    /// never waking.
    pub fn waking(&self, pid: Pid) -> Option<Ms> {
        let u = self.current(pid)?;
        match self.units.get(&u)?.state {
            UnitState::Activating {
                pid: Some(_),
                since,
                ..
            } => Some(since),
            _ => None,
        }
    }

    /// Whether run `pid`'s class may still be given (`classified`): asked at
    /// its ready, not answered yet, not timed out. The adapter drops a
    /// playback-worker answer that comes later, as today's `lazy_answer`
    /// drops one after `lazy_classify_tick` gave up on it.
    pub fn classifying(&self, pid: Pid) -> bool {
        self.current(pid)
            .and_then(|u| self.units.get(&u))
            .is_some_and(|unit| unit.classifying || unit.may_reclassify)
    }

    // ---- B2c–B2d: the Host's own starts (§17.10) ------------------------

    /// `runtime.ready` of run `pid`, which the Host started itself (the
    /// start-up queue, a first run, an enable, a reload, an install, a
    /// development load): as `try_ready`'s end for a run the core starts,
    /// the run's class is asked for (the 5 s timer for a run without a
    /// checkpoint) and its idle timer armed. `recordable`: it can have a
    /// checkpoint (today's `lazy_key_of` is `Some`); `first_run`: it was
    /// started as a first run (today's `lazy.first_runs`). Removed in B2d
    /// (every start a job; `add_unit` gives the spec).
    pub fn host_ready(&mut self, pid: Pid, recordable: bool, first_run: bool) {
        let now = self.now;
        let Some(u) = self.current(pid) else {
            return;
        };
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        let UnitState::Active { last_active, .. } = &mut unit.state else {
            return;
        };
        *last_active = now;
        unit.recordable = recordable && unit.class != Class::Development;
        unit.run_is_first = first_run;
        unit.from_standby = false;
        Timers::cancel(&mut unit.gens, TimerKind::Idle);
        self.arm_classification(u);
        self.arm_idle(u);
    }

    /// What the Host found of `u`'s checkpoint on disk (`Some(resident)`) at
    /// a start it runs itself (today's `start_installed` reads the
    /// recording), or that it is gone (`None`: uninstall, reload, install,
    /// upgrade, a write that failed and left none): the core's `recorded` /
    /// `recorded_resident` follow the file, so `classified` and
    /// `quit_checkpoints` write what today's checks of the file write.
    /// Removed in B2d (`add_unit`'s spec, the `DropCheckpoint` executor).
    pub fn checkpoint_known(&mut self, u: UnitId, checkpoint: Option<bool>) {
        if let Some(unit) = self.units.get_mut(&u) {
            unit.recorded = unit.recordable && checkpoint.is_some();
            unit.recorded_resident = unit.recorded && checkpoint == Some(true);
            unit.resident = unit.recorded_resident;
        }
    }

    /// Start-up replayed `u`'s checkpoint under `holder` itself (today's
    /// `start_installed` → `lazy_replay`; T1 without `add_unit`): `u` is in
    /// standby, recorded lazy. Removed in B2d (`add_unit`'s `Replay`).
    pub fn standby_replayed(&mut self, u: UnitId, holder: Pid) {
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        if unit.state.is_deactivating() {
            // Unreachable: the Host replays only a plugin without a process.
            debug_assert!(false, "unit {} replayed while it stops", unit.id);
            return;
        }
        unit.recorded = unit.recordable;
        unit.recorded_resident = false;
        unit.resident = false;
        unit.state = UnitState::Standby {
            holder,
            recorded: true,
            hold_until: None,
        };
        let old = [unit.alias.take(), unit.last_pid.replace(holder)];
        self.map_pid(u, holder);
        for old in old.into_iter().flatten() {
            if old != holder {
                self.unmap_pid(u, old);
            }
        }
        self.sync(u);
    }

    /// The Host forgot `u`'s standby (today's `forget_parked`: a disable,
    /// uninstall, reload, install or upgrade of a plugin in standby): its
    /// kept registrations are revoked now (T21–T23's `Revoke`, without the
    /// stop job the Host does not enqueue before B2d). A unit still stopping
    /// for standby lands `Inactive` (§17.7 item 6), and a wake waiting for
    /// that process goes with it, as today's `Parked.waiting` went with the
    /// parked entry. Removed in B2d (stop and restart jobs).
    pub fn standby_forgotten(&mut self, u: UnitId) {
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        let wake = unit.job.as_ref().is_some_and(|job| {
            matches!(
                job.kind,
                JobKind::Start {
                    cause: Cause::Trigger(_)
                }
            )
        });
        match unit.state {
            UnitState::Standby { holder, .. } => {
                unit.state = UnitState::Inactive;
                self.pending.push(Action::Revoke {
                    unit: u,
                    pid: holder,
                });
            }
            UnitState::Deactivating {
                then: Then::Standby,
                ..
            } => self.set_then(u, Then::Inactive),
            _ => {}
        }
        if wake {
            self.answer_backlog(u, AnswerReason::Stopped);
            self.finish(u, JobResult::Canceled);
        }
        self.sync(u);
    }

    /// The Host's own idle check of `pid` says busy (today's `lazy_idle`: a
    /// dialog, a package RPC, a busy command or settings page, a UI request
    /// its activation waits on, a pending source state reply, its class
    /// still asked, its activation not ready): its Standby job is canceled
    /// without counting as use, and the idle timer checks again after
    /// `idle_check` (today's 250 ms poll). Nothing without a Standby job
    /// still asking. Removed in B3 (`work.rs`: these become in-flight work).
    pub fn standby_refused(&mut self, pid: Pid) {
        let due = self.now + self.tunables.idle_check;
        let Some(u) = self.current(pid) else {
            return;
        };
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        let asking = matches!(unit.state, UnitState::Active { .. })
            && matches!(
                unit.job,
                Some(Job {
                    kind: JobKind::Standby,
                    ..
                })
            );
        if !asking {
            return;
        }
        unit.asked_at = None;
        Timers::cancel(&mut unit.gens, TimerKind::Resources);
        self.timers.arm(&mut unit.gens, u, TimerKind::Idle, due);
        self.finish(u, JobResult::Canceled);
        self.sync(u);
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

    /// `u`'s process is now `pid`; the process it displaced, if any. The
    /// Host never gives a unit a second process while it has one (§17.8: a
    /// start of a running plugin returns; a reload, install or uninstall
    /// stops it first): unreachable. If it happens it is a bug: a
    /// `debug_assert!` here, reported by the adapter in release, and the new
    /// process replaces the old. Adopting the process it already has
    /// displaces nothing.
    fn adopt(&mut self, u: UnitId, pid: Pid) -> Option<Pid> {
        let unit = self.units.get_mut(&u)?;
        let displaced = unit.hosted.replace(pid).filter(|old| *old != pid);
        debug_assert!(
            displaced.is_none(),
            "unit {} hosted twice: {displaced:?} then {pid:?}",
            unit.id
        );
        self.map_pid(u, pid);
        if let Some(old) = displaced {
            self.unmap_pid(u, old);
        }
        displaced
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
    /// keeps (state, alias, last pid, holder, the stopping process it still
    /// waits for).
    pub(super) fn unmap_pid(&mut self, u: UnitId, pid: Pid) {
        let Some(unit) = self.units.get(&u) else {
            return;
        };
        let held = unit.hosted == Some(pid)
            || unit.stopping.is_some_and(|s| s.pid == pid)
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
    use super::super::{
        Action, ActivatingSub, Delivery, JobKind, JobResult, StopReason, Supervisor, Trigger,
        Tunables,
    };
    use super::*;

    type S = Supervisor<u32>;

    fn new() -> S {
        Supervisor::new(Tunables::legacy(), 1)
    }

    /// For the B2c tests: the pids a test gives are below 100, the ones the
    /// core allocates for a wake from 101.
    fn b2c() -> S {
        let mut s = new();
        while s.next_pid().0 <= 100 {
            s.alloc_pid();
        }
        s
    }

    #[test]
    fn pids_count_from_one_like_the_owner_generation() {
        let mut sup = new();
        assert_eq!(sup.next_pid(), Pid(1));
        assert_eq!(sup.alloc_pid(), Pid(1));
        assert_eq!(sup.alloc_pid(), Pid(2));
        assert_eq!(sup.next_pid(), Pid(3));
    }

    /// `Host.plugin_ids` semantics: a stop of another pid leaves the
    /// process, its own stop clears it.
    #[test]
    fn process_index_follows_adopt_and_release() {
        let mut sup = new();
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
        assert_eq!(sup.process_of("a.one"), Some(Pid(7)));
        sup.release_process(a, Pid(7));
        assert_eq!(sup.process_of("a.one"), None);
        assert_eq!(sup.by_pid(Pid(7)), None);
        sup.adopt_process(a, Pid(9));
        assert_eq!(sup.process_of("a.one"), Some(Pid(9)));
        assert_eq!(sup.process_of("b.two"), Some(Pid(8)));
        assert_eq!(sup.process_of("c.none"), None);
    }

    /// `Lazy.parked_ids` semantics (B2a's `holder_index_follows_adopt_and_
    /// release`, ported to B2c's inputs): a replayed unit is held under its
    /// holder; forgetting it revokes and clears the index; a later run
    /// starts with no holder.
    #[test]
    fn holder_index_follows_replay_and_forget() {
        let mut sup = new();
        let a = sup.load_unit("a.one");
        sup.standby_replayed(a, Pid(3));
        assert_eq!(sup.process_of("a.one"), None);
        assert_eq!(sup.holder_of("a.one"), Some(Pid(3)));
        assert_eq!(sup.by_pid(Pid(3)), Some(a));
        sup.standby_forgotten(a);
        assert!(sup.take_actions().contains(&Action::Revoke {
            unit: a,
            pid: Pid(3)
        }));
        assert_eq!(sup.holder_of("a.one"), None);
        // Its last pid stays mapped for late replies until the next run.
        sup.adopt_process(a, Pid(4));
        assert_eq!(sup.by_pid(Pid(3)), Some(a));
        assert_eq!(sup.process_of("a.one"), Some(Pid(4)));
        assert_eq!(sup.holder_of("a.one"), None);
    }

    /// A second process while the unit has one is a Host bug (§17.8).
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "hosted twice")]
    fn a_second_owner_is_a_bug() {
        let mut sup = new();
        let a = sup.load_unit("a.one");
        sup.adopt_process(a, Pid(3));
        sup.adopt_process(a, Pid(4));
    }

    /// Adopting the process the unit already has displaces nothing.
    #[test]
    fn adopting_the_same_owner_displaces_nothing() {
        let mut sup = new();
        let a = sup.load_unit("a.one");
        assert_eq!(sup.adopt_process(a, Pid(3)), None);
        assert_eq!(sup.adopt_process(a, Pid(3)), None);
        assert_eq!(sup.process_of("a.one"), Some(Pid(3)));
    }

    /// Loading units this way installs no job and queues nothing.
    #[test]
    fn identity_inputs_queue_no_action() {
        let mut sup = new();
        let a = sup.load_unit("a.one");
        let pid = sup.alloc_pid();
        sup.activation_begin(a, pid, 1_000).unwrap();
        sup.adopt_process(a, pid);
        assert!(sup.job(a).is_none());
        assert!(sup.run_jobs(10).is_empty());
        // B2b: the state follows the process (the waits read it), and still
        // no job, timer or action.
        assert!(matches!(
            sup.unit(a).unwrap().state,
            UnitState::Active { pid: p, .. } if p == pid
        ));
        sup.release_process(a, pid);
        assert!(sup.job(a).is_none());
        assert!(sup.run_jobs(20).is_empty());
        assert_eq!(sup.unit(a).unwrap().state, UnitState::Inactive);
    }

    // ---- B2c: standby, wake and classification (§17.10) -----------------
    //
    // The ported `lazy.rs` paths, driven as the adapter drives them: the
    // Host starts a run (`adopt_process`), reports its ready (`host_ready`)
    // and its class (`classified_with`); the core decides the standby
    // (idle timer → `QueryResources` → `Checkpoint` + `Terminate`) and the
    // wake (`Rekey` + `Spawn` → `Prune` + `Deliver`).

    /// A run the Host started, ready at `at` (`first_run`: no checkpoint).
    fn ready(s: &mut S, id: &str, pid: u64, at: Ms, first_run: bool) -> UnitId {
        s.set_now(at);
        let u = s.load_unit(id);
        if !first_run {
            s.checkpoint_known(u, Some(false));
        }
        s.adopt_process(u, Pid(pid));
        s.host_ready(Pid(pid), true, first_run);
        u
    }

    fn kinds(actions: &[Action<u32>]) -> Vec<&'static str> {
        actions
            .iter()
            .map(|a| match a {
                Action::Spawn { .. } => "Spawn",
                Action::Rekey { .. } => "Rekey",
                Action::Prune { .. } => "Prune",
                Action::Deliver { .. } => "Deliver",
                Action::QueryResources { .. } => "QueryResources",
                Action::Checkpoint { .. } => "Checkpoint",
                Action::Terminate { .. } => "Terminate",
                Action::Revoke { .. } => "Revoke",
                Action::Notify { .. } => "Notify",
                Action::JobFinished { .. } => "JobFinished",
                Action::ListChanged { .. } => "ListChanged",
                _ => "other",
            })
            .filter(|k| *k != "ListChanged" && *k != "JobFinished")
            .collect()
    }

    /// The core parks a stopped run as today's `park`: the Host's stop of it
    /// is the core's `Terminate` (`process_stopping` changes nothing), and it
    /// lands in standby once reaped and cleaned up after.
    fn park(s: &mut S, u: UnitId, pid: u64, at: Ms) {
        let actions = s.run_jobs(at);
        assert!(
            actions.contains(&Action::QueryResources {
                unit: u,
                pid: Pid(pid)
            }),
            "{actions:?}"
        );
        s.resources_answered(Pid(pid), false);
        let actions = s.take_actions();
        assert_eq!(kinds(&actions), ["Checkpoint", "Terminate"], "{actions:?}");
        assert_eq!(
            s.process_stopping(u, Pid(pid), StopReason::Idle, Then::Standby),
            None
        );
        s.release_process(u, Pid(pid));
        assert_eq!(s.holder_of(&s.unit(u).unwrap().id.clone()), Some(Pid(pid)));
        s.process_reaped(Pid(pid), None);
        s.backend_cleaned(Pid(pid));
        s.run_jobs(at + 1);
    }

    /// Today's idle park (`lazy_tick` → `owner.resources` → `lazy_answer` →
    /// `park`): 60 s after the last use, not before; a use in between moves
    /// it; the unit lands in standby under its last pid.
    #[test]
    fn b2c_idle_standby_after_the_idle_time() {
        let mut s = b2c();
        let u = ready(&mut s, "a.one", 1, 0, false);
        assert!(s.run_jobs(59_999).is_empty());
        s.touch(Pid(1), 30_000);
        assert!(s.run_jobs(60_000).is_empty(), "used at 30 s");
        park(&mut s, u, 1, 90_000);
        assert_eq!(
            s.unit(u).unwrap().state,
            UnitState::Standby {
                holder: Pid(1),
                recorded: true,
                hold_until: None
            }
        );
    }

    /// Today's `lazy_idle` busy check (a busy command, a dialog...): the
    /// Host refuses the standby without counting it as use, and is asked
    /// again 250 ms later (`CHECK_MS`), not after another idle time.
    #[test]
    fn b2c_a_refused_standby_is_asked_again_after_250_ms() {
        let mut s = b2c();
        let u = ready(&mut s, "a.one", 1, 0, false);
        assert_eq!(kinds(&s.run_jobs(60_000)), ["QueryResources"]);
        s.standby_refused(Pid(1));
        assert!(s.job(u).is_none());
        assert!(kinds(&s.run_jobs(60_249)).is_empty());
        assert_eq!(kinds(&s.run_jobs(60_250)), ["QueryResources"]);
        // A live resource is use: asked again after another idle time.
        s.resources_answered(Pid(1), true);
        assert!(kinds(&s.run_jobs(120_249)).is_empty());
        assert_eq!(kinds(&s.run_jobs(120_250)), ["QueryResources"]);
        // Without a Standby job asking, a refusal changes nothing.
        s.standby_refused(Pid(1));
        s.standby_refused(Pid(1));
        assert!(s.job(u).is_none());
        assert!(kinds(&s.run_jobs(120_499)).is_empty());
        assert_eq!(kinds(&s.run_jobs(120_500)), ["QueryResources"]);
        assert!(matches!(s.job(u).map(|j| &j.kind), Some(JobKind::Standby)));
    }

    /// Today's lazy first run (`lazy_classified`): classified lazy and idle,
    /// it parks at once; busy (or its recording failed), it parks as soon
    /// as it is idle, checked every 250 ms; resident, it stays and is
    /// checkpointed resident once.
    #[test]
    fn b2c_first_runs_park_at_their_classification() {
        let mut s = b2c();
        let u = ready(&mut s, "a.one", 1, 0, true);
        s.classified_with(Pid(1), None, false);
        let actions = s.take_actions();
        assert_eq!(kinds(&actions), ["Checkpoint", "Terminate"], "{actions:?}");
        assert!(actions.contains(&Action::Checkpoint {
            unit: u,
            pid: Pid(1),
            resident: false
        }));
        let v = ready(&mut s, "b.two", 2, 0, true);
        s.classified_with(Pid(2), None, true);
        assert!(kinds(&s.take_actions()).is_empty());
        assert_eq!(kinds(&s.run_jobs(250)), ["QueryResources"]);
        s.standby_refused(Pid(2));
        assert_eq!(kinds(&s.run_jobs(500)), ["QueryResources"]);
        s.resources_answered(Pid(2), false);
        assert_eq!(kinds(&s.take_actions()), ["Checkpoint", "Terminate"]);
        assert!(s.unit(v).unwrap().state.is_deactivating());
        let w = ready(&mut s, "c.three", 3, 0, true);
        s.classified_with(Pid(3), Some("a listener".into()), false);
        assert_eq!(
            s.take_actions()
                .into_iter()
                .filter(|a| matches!(a, Action::Checkpoint { .. }))
                .collect::<Vec<_>>(),
            [Action::Checkpoint {
                unit: w,
                pid: Pid(3),
                resident: true
            }]
        );
        // Classified once per run: a second answer changes nothing.
        s.classified_with(Pid(3), None, false);
        assert!(kinds(&s.take_actions()).is_empty());
    }

    /// A run that is not a first run is classified at every ready (today's
    /// `lazy_ready` classifies every ready) and parks only after its idle
    /// time; a resident class is written once while the file says so
    /// (`checkpoint_known`), and again after the file was removed.
    #[test]
    fn b2c_later_runs_are_classified_and_checkpointed_as_the_file_says() {
        let mut s = b2c();
        let u = s.load_unit("a.one");
        s.checkpoint_known(u, Some(true));
        s.adopt_process(u, Pid(1));
        s.host_ready(Pid(1), true, false);
        s.classified_with(Pid(1), Some("a panel".into()), false);
        assert!(
            kinds(&s.take_actions()).is_empty(),
            "the file says resident"
        );
        s.classified_with(Pid(1), None, false);
        assert!(kinds(&s.take_actions()).is_empty(), "once per run");
        // A reload removes the file: the next run writes it again.
        s.checkpoint_known(u, None);
        s.process_stopping(u, Pid(1), StopReason::Reload, Then::Inactive);
        s.release_process(u, Pid(1));
        s.process_reaped(Pid(1), None);
        s.backend_cleaned(Pid(1));
        s.adopt_process(u, Pid(101));
        s.host_ready(Pid(101), true, false);
        s.classified_with(Pid(101), Some("a panel".into()), false);
        assert_eq!(kinds(&s.take_actions()), ["Checkpoint"]);
        // A lazy later run waits its idle time (60 s from its ready).
        let v = ready(&mut s, "b.two", 3, 0, false);
        s.classified_with(Pid(3), None, false);
        assert!(kinds(&s.take_actions()).is_empty());
        assert!(!s.unit(v).unwrap().state.is_deactivating());
    }

    /// Quit (today's `lazy_quit`): every run whose class came back this run
    /// is checkpointed, a resident one only when the file does not say so;
    /// a run whose answer never came is not.
    #[test]
    fn b2c_quit_checkpoints_the_classified_runs() {
        let mut s = b2c();
        let a = ready(&mut s, "a.one", 1, 0, false);
        let b = s.load_unit("b.two");
        s.checkpoint_known(b, Some(true));
        s.adopt_process(b, Pid(101));
        s.host_ready(Pid(101), true, false);
        ready(&mut s, "c.three", 3, 0, false);
        let d = ready(&mut s, "d.four", 4, 0, false);
        s.classified_with(Pid(1), None, false);
        s.classified_with(Pid(101), Some("x".into()), false);
        s.classified_with(Pid(4), Some("y".into()), false);
        s.take_actions();
        // `d`'s resident class was written at its classification.
        assert!(s.unit(d).unwrap().recorded_resident);
        assert_eq!(s.quit_checkpoints(), [(a, Pid(1), false)]);
    }

    /// Today's wake (`lazy_deliver` → `wake` → `wake_now` → `lazy_ready`): a
    /// trigger on a unit in standby starts it at once (`advance`): the
    /// entries move to the new pid, the trigger is held for its ready and
    /// delivered after `Prune`; a trigger to the old pid is held while it
    /// starts and forwarded once it runs (today's `lazy.moved`).
    #[test]
    fn b2c_a_trigger_wakes_a_unit_in_standby() {
        let mut s = b2c();
        let u = ready(&mut s, "a.one", 1, 0, false);
        park(&mut s, u, 1, 60_000);
        s.set_now(61_000);
        assert_eq!(s.wake(Pid(1), &Trigger::interactive(7)), Delivery::Started);
        s.advance(u);
        let actions = s.take_actions();
        assert_eq!(kinds(&actions), ["Rekey", "Spawn"], "{actions:?}");
        let Some(Action::Spawn { pid, .. }) = actions.last().cloned() else {
            unreachable!()
        };
        assert_eq!(pid, Pid(101));
        assert!(actions.contains(&Action::Rekey {
            unit: u,
            from: Pid(1),
            to: pid
        }));
        s.adopt_process(u, pid);
        assert_eq!(s.waking(pid), Some(61_000));
        s.process_spawned(pid, Ok(()));
        assert_eq!(s.wake(Pid(1), &Trigger::interactive(8)), Delivery::Held);
        assert_eq!(s.wake(pid, &Trigger::interactive(9)), Delivery::Held);
        s.process_ready(pid);
        assert_eq!(s.waking(pid), None);
        let actions = s.take_actions();
        assert_eq!(kinds(&actions), ["Prune", "Deliver", "Deliver", "Deliver"]);
        let delivered = actions
            .iter()
            .filter_map(|a| match a {
                Action::Deliver { payload, .. } => Some(*payload),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(delivered, [7, 8, 9]);
        assert_eq!(
            s.wake(Pid(1), &Trigger::interactive(10)),
            Delivery::Forward(pid)
        );
        assert_eq!(s.wake(pid, &Trigger::interactive(11)), Delivery::Now);
        // The run's class is asked again (it came from a checkpoint).
        s.classified_with(pid, None, false);
        assert!(kinds(&s.take_actions()).is_empty());
        // The old pid goes with the run (today's `forget_running`).
        s.process_stopping(u, pid, StopReason::Disable, Then::Inactive);
        assert_eq!(s.wake(Pid(1), &Trigger::interactive(12)), Delivery::Dropped);
    }

    /// "Starting <name>…" 400 ms after a wake's spawn while it is not ready
    /// (today's `lazy_tick` over `lazy.waking`).
    #[test]
    fn b2c_a_slow_wake_shows_starting() {
        let mut s = b2c();
        let u = ready(&mut s, "a.one", 1, 0, false);
        park(&mut s, u, 1, 60_000);
        s.set_now(61_000);
        s.wake(Pid(1), &Trigger::interactive(7));
        s.advance(u);
        s.take_actions();
        s.adopt_process(u, Pid(101));
        s.process_spawned(Pid(101), Ok(()));
        assert!(kinds(&s.run_jobs(61_399)).is_empty());
        assert_eq!(kinds(&s.run_jobs(61_400)), ["Notify"]);
    }

    /// A wake whose spawn fails (today's `wake_failed`): the job fails with
    /// the error, the entries are revoked, "could not start" shows.
    #[test]
    fn b2c_a_wake_that_cannot_start_revokes_and_tells() {
        let mut s = b2c();
        let u = ready(&mut s, "a.one", 1, 0, false);
        park(&mut s, u, 1, 60_000);
        s.wake(Pid(1), &Trigger::interactive(7));
        s.advance(u);
        s.take_actions();
        s.process_spawned(Pid(101), Err("start Node".into()));
        let actions = s.take_actions();
        assert_eq!(kinds(&actions), ["Revoke", "Notify"], "{actions:?}");
        assert!(actions.iter().any(|a| matches!(
            a,
            Action::JobFinished { result: JobResult::Failed(text), .. } if text == "start Node"
        )));
        assert!(matches!(s.unit(u).unwrap().state, UnitState::Failed { .. }));
    }

    /// The Host stops a waking run (a disable, a failed activation): the
    /// core's start goes with it, and so does the old pid's forwarding.
    #[test]
    fn b2c_a_host_stop_of_a_waking_run_ends_its_start() {
        let mut s = b2c();
        let u = ready(&mut s, "a.one", 1, 0, false);
        park(&mut s, u, 1, 60_000);
        s.wake(Pid(1), &Trigger::interactive(7));
        s.advance(u);
        s.take_actions();
        s.adopt_process(u, Pid(101));
        s.process_spawned(Pid(101), Ok(()));
        assert!(matches!(
            s.unit(u).unwrap().state,
            UnitState::Activating {
                sub: ActivatingSub::Ready,
                ..
            }
        ));
        s.process_stopping(u, Pid(101), StopReason::Disable, Then::Inactive);
        assert!(s.job(u).is_none());
        assert_eq!(s.wake(Pid(1), &Trigger::interactive(8)), Delivery::Dropped);
        s.release_process(u, Pid(101));
        s.process_reaped(Pid(101), None);
        s.backend_cleaned(Pid(101));
        s.run_jobs(70_000);
        assert_eq!(s.unit(u).unwrap().state, UnitState::Inactive);
    }

    /// Wake events (today's `lazy_event`): muted during start-up and 1 s
    /// after it settled for a unit in standby; a unit already waking holds
    /// them anyway (today's broadcast holds them in `lazy.waking`), only the
    /// newest of each.
    #[test]
    fn b2c_muted_wake_events_still_reach_a_waking_unit() {
        let mut s = b2c();
        s.set_muted(true);
        let u = ready(&mut s, "a.one", 1, 0, false);
        park(&mut s, u, 1, 60_000);
        let event = |n| Trigger::event("packages.changed", n);
        assert_eq!(s.wake(Pid(1), &event(1)), Delivery::Dropped);
        assert_eq!(s.wake(Pid(1), &Trigger::interactive(2)), Delivery::Started);
        s.advance(u);
        s.take_actions();
        s.adopt_process(u, Pid(101));
        s.process_spawned(Pid(101), Ok(()));
        assert_eq!(s.wake(Pid(101), &event(3)), Delivery::Held);
        assert_eq!(s.wake(Pid(101), &event(4)), Delivery::Held);
        s.process_ready(Pid(101));
        let delivered = s
            .take_actions()
            .into_iter()
            .filter_map(|a| match a {
                Action::Deliver { payload, .. } => Some(payload),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(delivered, [2, 4]);
        s.settled(61_000);
        let v = ready(&mut s, "b.two", 3, 61_000, false);
        park(&mut s, v, 3, 121_000);
        s.set_now(121_500);
        s.settled(121_000);
        assert_eq!(s.wake(Pid(3), &event(5)), Delivery::Dropped);
        s.set_now(122_000);
        assert_eq!(s.wake(Pid(3), &event(6)), Delivery::Started);
    }

    /// A wake while the parked process still goes waits for it (today's
    /// `host_start(Wake)` of B2b, now the core's own start), and a standby
    /// forgotten meanwhile takes the wake with it.
    #[test]
    fn b2c_a_wake_waits_for_the_parked_process_and_goes_with_its_standby() {
        let mut s = b2c();
        let u = ready(&mut s, "a.one", 1, 0, false);
        assert_eq!(kinds(&s.run_jobs(60_000)), ["QueryResources"]);
        s.resources_answered(Pid(1), false);
        s.take_actions();
        s.process_stopping(u, Pid(1), StopReason::Idle, Then::Standby);
        s.release_process(u, Pid(1));
        s.set_now(60_100);
        assert_eq!(s.wake(Pid(1), &Trigger::interactive(7)), Delivery::Started);
        s.advance(u);
        assert!(kinds(&s.take_actions()).is_empty(), "waits for the reap");
        s.process_reaped(Pid(1), None);
        s.backend_cleaned(Pid(1));
        assert_eq!(kinds(&s.run_jobs(60_200)), ["Rekey", "Spawn"]);
        // Again, forgotten before the process went.
        let v = ready(&mut s, "b.two", 10, 0, false);
        s.run_jobs(70_000);
        s.resources_answered(Pid(10), false);
        s.take_actions();
        s.process_stopping(v, Pid(10), StopReason::Idle, Then::Standby);
        s.release_process(v, Pid(10));
        assert_eq!(s.wake(Pid(10), &Trigger::interactive(7)), Delivery::Started);
        s.standby_forgotten(v);
        let actions = s.take_actions();
        assert_eq!(kinds(&actions), ["Revoke"], "{actions:?}");
        assert!(s.job(v).is_none());
        s.process_reaped(Pid(10), None);
        s.backend_cleaned(Pid(10));
        assert!(kinds(&s.run_jobs(70_100)).is_empty());
        assert_eq!(s.unit(v).unwrap().state, UnitState::Inactive);
    }

    /// Lazy activation off (`on_demand` false): nothing is classified or
    /// put in standby; a trigger goes to the process.
    #[test]
    fn b2c_off_demand_never_stands_by() {
        let mut s = Supervisor::<u32>::new(
            Tunables {
                on_demand: false,
                ..Tunables::legacy()
            },
            1,
        );
        ready(&mut s, "a.one", 1, 0, true);
        assert!(s.run_jobs(600_000).is_empty());
        assert_eq!(s.wake(Pid(1), &Trigger::interactive(7)), Delivery::Now);
        assert!(s.quit_checkpoints().is_empty());
    }

    /// An unrecordable run (a development load, a built-in without a
    /// digest: today's `lazy_key_of` is `None`) is never classified nor put
    /// in standby.
    #[test]
    fn b2c_an_unrecordable_run_is_never_classified() {
        let mut s = b2c();
        let u = s.load_unit("a.one");
        s.adopt_process(u, Pid(1));
        s.host_ready(Pid(1), false, false);
        s.classified_with(Pid(1), None, false);
        assert!(kinds(&s.take_actions()).is_empty());
        assert!(s.run_jobs(600_000).is_empty());
        assert!(s.quit_checkpoints().is_empty());
    }
    /// Review fix: with `start_timeout_by_host` (the adapter's, until B2d)
    /// a woken run has only the Host's activation deadline: the core never
    /// fails it on its own, however late its ready.
    #[test]
    fn b2c_the_host_times_a_wakes_start() {
        let mut s = b2c();
        s.tunables.start_timeout_by_host = true;
        let u = ready(&mut s, "a.one", 1, 0, false);
        park(&mut s, u, 1, 60_000);
        s.wake(Pid(1), &Trigger::interactive(7));
        s.advance(u);
        s.take_actions();
        s.adopt_process(u, Pid(101));
        s.process_spawned(Pid(101), Ok(()));
        assert!(!kinds(&s.run_jobs(200_000)).contains(&"Terminate"));
        s.process_ready(Pid(101));
        assert!(matches!(s.unit(u).unwrap().state, UnitState::Active { .. }));
    }

    /// Review fix: a run's classification closes with its answer or its
    /// timeout; a later answer finds it closed (the adapter drops it).
    #[test]
    fn b2c_classification_closes_once() {
        let mut s = b2c();
        ready(&mut s, "a.one", 1, 0, true);
        assert!(s.classifying(Pid(1)));
        s.run_jobs(5_000);
        assert!(!s.classifying(Pid(1)), "timed out: resident");
        ready(&mut s, "b.two", 2, 0, false);
        assert!(s.classifying(Pid(2)));
        s.classified_with(Pid(2), None, false);
        assert!(!s.classifying(Pid(2)));
    }
}
