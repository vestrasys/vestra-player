//! The unit table's identity and what the adapter reads of it
//! (doc/architecture/supervisor.md §13 B2a–B2d, §17.8, §17.11): owner
//! generations (`alloc_pid`, today's `next_generation += 1`), the indexes
//! by id and by pid, the process and standby holder of an id, and each
//! run's activation (`activation.rs`, MOVE of `host/package_activation.rs`).
//!
//! Since B2d every start and stop of a unit is a job (`add_unit`, `start`,
//! `stop`, `restart`, `wake`, the idle Standby job): the process of a unit
//! is its state's pid, not a transitional field the Host sets. What is left
//! here besides identity are the adapter's answers to actions the core
//! cannot check itself (`replay_failed`, `checkpoint_failed`) and the
//! Host's own idle check (`standby_refused`, until B3's `work.rs`).
#![forbid(unsafe_code)]

use super::Supervisor;
use super::activation::{self, Error, LIMIT, MAX_SAFE, Run, Status};
use super::job::{Cause, Job, JobKind, JobMode, JobResult, Origin, Priority};
use super::timers::{TimerKind, Timers};
use super::unit::{ActivatingSub, Class, Ms, Pid, Then, UnitId, UnitSpec, UnitState};

impl<P: Clone> Supervisor<P> {
    /// The unit of manifest `id`; a new one is loaded (no job, `Inactive`,
    /// enabled, class `Normal`) when there is none. `add_unit` gives a
    /// loaded unit its spec.
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
    /// start past the wire's safe integers (a development load).
    pub fn next_pid(&self) -> Pid {
        Pid(self.next_pid)
    }

    /// The process that runs plugin `id` (today's `Host::running`): the
    /// state's pid once its spawn ran (`Activating{Ready}` or `Active`);
    /// none while the spawn is still to be executed, in standby, or while
    /// its process stops.
    pub fn process_of(&self, id: &str) -> Option<Pid> {
        match self.units.get(self.by_id.get(id)?)?.state {
            UnitState::Activating {
                pid: Some(pid),
                sub: ActivatingSub::Ready,
                ..
            }
            | UnitState::Active { pid, .. } => Some(pid),
            _ => None,
        }
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

    /// When a unit woken from standby started `pid`, while it is not ready
    /// yet (today's `lazy.waking`): triggers and wake events for it are
    /// held for its ready. Any other start delivers to its process at once
    /// under `Tunables::deliver_while_starting` (today's boot, first run,
    /// enable, reload, install and development starts).
    pub fn waking(&self, pid: Pid) -> Option<Ms> {
        let u = self.current(pid)?;
        let unit = self.units.get(&u)?;
        match unit.state {
            UnitState::Activating {
                pid: Some(_),
                since,
                ..
            } if unit.from_standby => Some(since),
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

    /// Run `pid` was started as a first run (`Start{FirstRun}`, today's
    /// `lazy.first_runs`): only such a run goes to standby at once when
    /// classified lazy (T12).
    pub fn run_is_first(&self, pid: Pid) -> bool {
        self.current(pid)
            .and_then(|u| self.units.get(&u))
            .is_some_and(|unit| unit.run_is_first)
    }

    /// A start-up start still waits (today's `StartQueue` is not empty): a
    /// `Start{Boot}` (or a runner member's restart) not yet past its digest
    /// check, or a first run still being checked (`packages_started`).
    pub fn boot_pending(&self) -> bool {
        !self.boot_waiting.is_empty()
    }

    /// First runs wait for a place (today's `FirstQueue`): any, and whether
    /// a built-in is among them (`builtin_first_run_queued`).
    pub fn first_runs_queued(&self) -> (bool, bool) {
        (
            !self.first_waiting.is_empty(),
            self.first_waiting.iter().any(|u| {
                self.units
                    .get(u)
                    .is_some_and(|unit| unit.class == Class::Builtin)
            }),
        )
    }

    /// The adapter could not replay `u`'s checkpoint (`Action::Replay`;
    /// today's `lazy_replay` failing in `start_installed`): it revoked what
    /// it registered under the holder, and the unit starts instead, as
    /// today's "replay failed, starting it" (a start at boot, its checkpoint
    /// still on disk: not a first run).
    pub fn replay_failed(&mut self, u: UnitId) {
        let Some(unit) = self.units.get_mut(&u) else {
            return;
        };
        let UnitState::Standby { holder, .. } = unit.state else {
            return;
        };
        unit.state = UnitState::Inactive;
        if unit.last_pid == Some(holder) {
            unit.last_pid = None;
        }
        self.unmap_pid(u, holder);
        let _ = self.enqueue_job(
            u,
            JobKind::Start { cause: Cause::Boot },
            JobMode::Replace,
            Priority::Background,
            Origin::Internal,
        );
        self.sync(u);
    }

    /// Whether `u` can have a checkpoint changed with its package: a package
    /// operation's target cannot while it activates (its package is not
    /// committed: today's `lazy_key_of` is `None` for it, so its ready is
    /// not classified), and can from its commit on. Read at the next ready
    /// (a development load never can).
    pub fn set_recordable(&mut self, u: UnitId, recordable: bool) {
        if let Some(unit) = self.units.get_mut(&u) {
            unit.recordable = recordable && unit.class != Class::Development;
        }
    }

    /// The `Checkpoint` executor could not write `u`'s recording; what is
    /// on disk now (`Some(resident)`, or none): `classified` and
    /// `quit_checkpoints` follow the file, as today's checks of the file do.
    pub fn checkpoint_failed(&mut self, u: UnitId, on_disk: Option<bool>) {
        if let Some(unit) = self.units.get_mut(&u) {
            unit.recorded = unit.recordable && on_disk.is_some();
            unit.recorded_resident = unit.recorded && on_disk == Some(true);
            unit.resident = unit.recorded_resident;
        }
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
        let held = unit.stopping.is_some_and(|s| s.pid == pid)
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

    #[test]
    fn pids_count_from_one_like_the_owner_generation() {
        let mut sup = new();
        assert_eq!(sup.next_pid(), Pid(1));
        assert_eq!(sup.alloc_pid(), Pid(1));
        assert_eq!(sup.alloc_pid(), Pid(2));
        assert_eq!(sup.next_pid(), Pid(3));
    }

    /// The spawn of `actions`.
    fn spawned(actions: &[Action<u32>]) -> Option<Pid> {
        actions.iter().find_map(|a| match a {
            Action::Spawn { pid, .. } => Some(*pid),
            _ => None,
        })
    }

    /// A run started as the adapter starts it (B2d: every start a job),
    /// spawned as `pid` and ready at `at`: a unit added without a
    /// checkpoint is a first run (on-demand start on); with one it is
    /// loaded disabled, then enabled and started (`Start{Enable}`), so its
    /// checkpoint is kept and it does not replay.
    fn start_with(s: &mut S, spec: UnitSpec, pid: u64, at: Ms) -> UnitId {
        s.set_now(at);
        let replays = spec.checkpoint.is_some();
        let spec = if replays { spec.disabled() } else { spec };
        let u = s.add_unit(spec);
        if replays {
            s.set_enabled(u, true);
            s.start(u, Cause::Enable).unwrap();
        }
        let actions = s.run_jobs(at);
        assert_eq!(spawned(&actions), Some(Pid(pid)), "{actions:?}");
        s.process_spawned(Pid(pid), Ok(()));
        s.process_ready(Pid(pid));
        s.run_jobs(at);
        u
    }

    /// `start_with` for a third-party unit: a first run, or a run whose
    /// checkpoint says lazy.
    fn ready(s: &mut S, id: &str, pid: u64, at: Ms, first_run: bool) -> UnitId {
        let spec = UnitSpec::new(id, Class::Normal);
        let spec = if first_run {
            spec
        } else {
            spec.checkpoint(false)
        };
        start_with(s, spec, pid, at)
    }

    /// `Host.plugin_ids` semantics, now the state's: a unit's process is the
    /// run its start spawned, until its stop; another id has its own.
    #[test]
    fn process_index_follows_the_runs() {
        let mut sup = new();
        let a = ready(&mut sup, "a.one", 1, 0, true);
        assert_eq!(sup.load_unit("a.one"), a);
        let b = ready(&mut sup, "b.two", 2, 0, true);
        assert_ne!(a, b);
        assert_eq!(sup.process_of("a.one"), Some(Pid(1)));
        assert_eq!(sup.holder_of("a.one"), None);
        assert_eq!(sup.by_pid(Pid(1)), Some(a));
        sup.disable(a).unwrap();
        sup.run_jobs(1);
        assert_eq!(sup.process_of("a.one"), None);
        // Still tracked until it is reaped and cleaned up after.
        assert_eq!(sup.by_pid(Pid(1)), Some(a));
        assert_eq!(sup.process_of("b.two"), Some(Pid(2)));
        assert_eq!(sup.process_of("c.none"), None);
    }

    /// `Lazy.parked_ids` semantics: a replayed unit is held under its
    /// holder; a disable revokes and clears the index; its last pid stays
    /// mapped for late replies until its next run.
    #[test]
    fn holder_index_follows_replay_and_disable() {
        let mut sup = new();
        let a = sup.add_unit(UnitSpec::new("a.one", Class::Normal).checkpoint(false));
        let actions = sup.run_jobs(0);
        assert!(actions.contains(&Action::Replay {
            unit: a,
            holder: Pid(1)
        }));
        assert_eq!(sup.process_of("a.one"), None);
        assert_eq!(sup.holder_of("a.one"), Some(Pid(1)));
        assert_eq!(sup.by_pid(Pid(1)), Some(a));
        sup.disable(a).unwrap();
        assert!(sup.run_jobs(1).contains(&Action::Revoke {
            unit: a,
            pid: Pid(1)
        }));
        assert_eq!(sup.holder_of("a.one"), None);
        assert_eq!(sup.by_pid(Pid(1)), Some(a));
        sup.enable(a).unwrap();
        let actions = sup.run_jobs(2);
        assert_eq!(spawned(&actions), Some(Pid(2)));
        assert_eq!(sup.by_pid(Pid(1)), None);
        assert_eq!(sup.holder_of("a.one"), None);
    }

    /// One process per unit by construction (§17.8's "hosted twice" is the
    /// adapter's `debug_assert!` now): a start of a running unit spawns
    /// nothing.
    #[test]
    fn a_start_of_a_running_unit_spawns_nothing() {
        let mut sup = new();
        let a = ready(&mut sup, "a.one", 1, 0, true);
        let job = sup.start(a, Cause::Enable).unwrap();
        assert_eq!(sup.job_result(a, job), Some(&JobResult::Done));
        assert_eq!(spawned(&sup.run_jobs(1)), None);
        assert_eq!(sup.process_of("a.one"), Some(Pid(1)));
    }

    /// Loading a unit for its identity (an activation of an id `add_unit`
    /// did not load) installs no job and queues nothing.
    #[test]
    fn identity_inputs_queue_no_action() {
        let mut sup = new();
        let a = sup.load_unit("a.one");
        let pid = sup.alloc_pid();
        sup.activation_begin(a, pid, 1_000).unwrap();
        assert!(sup.job(a).is_none());
        assert!(sup.run_jobs(10).is_empty());
        assert_eq!(sup.unit(a).unwrap().state, UnitState::Inactive);
        sup.activation_end(pid);
        assert!(sup.run_jobs(20).is_empty());
    }

    // ---- B2c: standby, wake and classification (§17.10) -----------------
    //
    // The ported `lazy.rs` paths, driven as the adapter drives them: a run
    // starts (B2d: a start job), reports its ready (`process_ready`) and its
    // class (`classified_with`); the core decides the standby (idle timer →
    // `QueryResources` → `Checkpoint` + `Terminate`) and the wake (`Rekey` +
    // `Spawn` → `Prune` + `Deliver`).

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

    /// The core parks an idle run as today's `park` (`Terminate` keeping
    /// its registrations), and it lands in standby once reaped and cleaned
    /// up after.
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
        let actions = s.run_jobs(at);
        assert_eq!(kinds(&actions), ["Checkpoint", "Terminate"], "{actions:?}");
        assert_eq!(s.holder_of(&s.unit(u).unwrap().id.clone()), Some(Pid(pid)));
        s.process_reaped(Pid(pid), None);
        s.backend_cleaned(Pid(pid));
        s.run_jobs(at + 1);
    }

    /// A wake of `u` in standby (held as `holder`) at `at`: its start runs at
    /// once (`advance`); the new pid.
    fn wake(s: &mut S, u: UnitId, holder: u64, at: Ms) -> Pid {
        s.set_now(at);
        assert_eq!(
            s.wake(Pid(holder), &Trigger::interactive(7)),
            Delivery::Started
        );
        s.advance(u);
        spawned(&s.run_jobs(at)).expect("spawned")
    }

    /// Today's idle park (`lazy_tick` → `owner.resources` → `lazy_answer` →
    /// `park`): 60 s after the last use, not before; a use in between moves
    /// it; the unit lands in standby under its last pid.
    #[test]
    fn b2c_idle_standby_after_the_idle_time() {
        let mut s = new();
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
        let mut s = new();
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
        let mut s = new();
        let u = ready(&mut s, "a.one", 1, 0, true);
        s.classified_with(Pid(1), None, false);
        let actions = s.run_jobs(0);
        assert_eq!(kinds(&actions), ["Checkpoint", "Terminate"], "{actions:?}");
        assert!(actions.contains(&Action::Checkpoint {
            unit: u,
            pid: Pid(1),
            resident: false
        }));
        let v = ready(&mut s, "b.two", 2, 0, true);
        s.classified_with(Pid(2), None, true);
        assert!(kinds(&s.run_jobs(0)).is_empty());
        assert_eq!(kinds(&s.run_jobs(250)), ["QueryResources"]);
        s.standby_refused(Pid(2));
        assert_eq!(kinds(&s.run_jobs(500)), ["QueryResources"]);
        s.resources_answered(Pid(2), false);
        assert_eq!(kinds(&s.run_jobs(500)), ["Checkpoint", "Terminate"]);
        assert!(s.unit(v).unwrap().state.is_deactivating());
        let w = ready(&mut s, "c.three", 3, 500, true);
        s.classified_with(Pid(3), Some("a listener".into()), false);
        assert_eq!(
            s.run_jobs(500)
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
        assert!(kinds(&s.run_jobs(500)).is_empty());
    }

    /// A run that is not a first run is classified at every ready (today's
    /// `lazy_ready` classifies every ready) and parks only after its idle
    /// time; a resident class is written once while the checkpoint says so,
    /// and again after a reload dropped it.
    #[test]
    fn b2c_later_runs_are_classified_and_checkpointed_as_the_file_says() {
        let mut s = new();
        let u = start_with(
            &mut s,
            UnitSpec::new("a.one", Class::Normal).checkpoint(true),
            1,
            0,
        );
        s.classified_with(Pid(1), Some("a panel".into()), false);
        assert!(kinds(&s.run_jobs(0)).is_empty(), "the file says resident");
        s.classified_with(Pid(1), None, false);
        assert!(kinds(&s.run_jobs(0)).is_empty(), "once per run");
        // A reload drops the checkpoint: the next run writes it again.
        s.restart(u, true, None).unwrap();
        assert_eq!(kinds(&s.run_jobs(1)), ["Terminate"]);
        s.process_reaped(Pid(1), None);
        s.backend_cleaned(Pid(1));
        assert_eq!(spawned(&s.run_jobs(2)), Some(Pid(2)));
        s.process_spawned(Pid(2), Ok(()));
        s.process_ready(Pid(2));
        s.run_jobs(2);
        s.classified_with(Pid(2), Some("a panel".into()), false);
        assert_eq!(kinds(&s.run_jobs(2)), ["Checkpoint"]);
        // A lazy later run waits its idle time (60 s from its ready).
        let v = ready(&mut s, "b.two", 3, 2, false);
        s.classified_with(Pid(3), None, false);
        assert!(kinds(&s.run_jobs(2)).is_empty());
        assert!(!s.unit(v).unwrap().state.is_deactivating());
    }

    /// Quit (today's `lazy_quit`): every run whose class came back this run
    /// is checkpointed, a resident one only when the file does not say so;
    /// a run whose answer never came is not.
    #[test]
    fn b2c_quit_checkpoints_the_classified_runs() {
        let mut s = new();
        let a = ready(&mut s, "a.one", 1, 0, false);
        start_with(
            &mut s,
            UnitSpec::new("b.two", Class::Normal).checkpoint(true),
            2,
            0,
        );
        ready(&mut s, "c.three", 3, 0, false);
        let d = ready(&mut s, "d.four", 4, 0, false);
        s.classified_with(Pid(1), None, false);
        s.classified_with(Pid(2), Some("x".into()), false);
        s.classified_with(Pid(4), Some("y".into()), false);
        s.run_jobs(0);
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
        let mut s = new();
        let u = ready(&mut s, "a.one", 1, 0, false);
        park(&mut s, u, 1, 60_000);
        s.set_now(61_000);
        assert_eq!(s.wake(Pid(1), &Trigger::interactive(7)), Delivery::Started);
        s.advance(u);
        let actions = s.run_jobs(61_000);
        assert_eq!(kinds(&actions), ["Rekey", "Spawn"], "{actions:?}");
        let pid = spawned(&actions).unwrap();
        assert_eq!(pid, Pid(2));
        assert!(actions.contains(&Action::Rekey {
            unit: u,
            from: Pid(1),
            to: pid
        }));
        s.process_spawned(pid, Ok(()));
        assert_eq!(s.waking(pid), Some(61_000));
        assert_eq!(s.wake(Pid(1), &Trigger::interactive(8)), Delivery::Held);
        assert_eq!(s.wake(pid, &Trigger::interactive(9)), Delivery::Held);
        s.process_ready(pid);
        assert_eq!(s.waking(pid), None);
        let actions = s.run_jobs(61_000);
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
        assert!(kinds(&s.run_jobs(61_000)).is_empty());
        // The old pid goes with the run (today's `forget_running`).
        s.disable(u).unwrap();
        s.run_jobs(61_000);
        assert_eq!(s.wake(Pid(1), &Trigger::interactive(12)), Delivery::Dropped);
    }

    /// B2d: a trigger to a run started otherwise (boot, first run, enable,
    /// reload, install) goes to its process at once while it starts
    /// (`deliver_while_starting`, today: only `lazy.waking` holds them).
    #[test]
    fn b2d_a_starting_run_not_woken_gets_its_triggers_at_once() {
        let mut s = new();
        let u = s.add_unit(UnitSpec::new("a.one", Class::Normal));
        assert_eq!(spawned(&s.run_jobs(0)), Some(Pid(1)));
        s.process_spawned(Pid(1), Ok(()));
        assert!(matches!(
            s.unit(u).unwrap().state,
            UnitState::Activating {
                sub: ActivatingSub::Ready,
                ..
            }
        ));
        assert_eq!(s.waking(Pid(1)), None);
        assert_eq!(s.wake(Pid(1), &Trigger::interactive(7)), Delivery::Now);
        let mut target = Supervisor::<u32>::new(Tunables::legacy().with_b3(8), 1);
        let v = target.add_unit(UnitSpec::new("a.one", Class::Normal));
        target.run_jobs(0);
        target.process_spawned(Pid(1), Ok(()));
        assert_eq!(
            target.wake(Pid(1), &Trigger::interactive(7)),
            Delivery::Held,
            "from B3 held for its ready (T9)"
        );
        target.process_ready(Pid(1));
        assert!(target.run_jobs(0).contains(&Action::Deliver {
            unit: v,
            pid: Pid(1),
            payload: 7
        }));
    }

    /// "Starting <name>…" 400 ms after a wake's spawn while it is not ready
    /// (today's `lazy_tick` over `lazy.waking`).
    #[test]
    fn b2c_a_slow_wake_shows_starting() {
        let mut s = new();
        let u = ready(&mut s, "a.one", 1, 0, false);
        park(&mut s, u, 1, 60_000);
        let pid = wake(&mut s, u, 1, 61_000);
        s.process_spawned(pid, Ok(()));
        assert!(kinds(&s.run_jobs(61_399)).is_empty());
        assert_eq!(kinds(&s.run_jobs(61_400)), ["Notify"]);
    }

    /// A wake whose spawn fails (today's `wake_failed`): the job fails with
    /// the error, the entries are revoked, "could not start" shows.
    #[test]
    fn b2c_a_wake_that_cannot_start_revokes_and_tells() {
        let mut s = new();
        let u = ready(&mut s, "a.one", 1, 0, false);
        park(&mut s, u, 1, 60_000);
        let pid = wake(&mut s, u, 1, 61_000);
        s.process_spawned(pid, Err("start Node".into()));
        let actions = s.run_jobs(61_000);
        assert_eq!(kinds(&actions), ["Revoke", "Notify"], "{actions:?}");
        assert!(actions.iter().any(|a| matches!(
            a,
            Action::JobFinished { result: JobResult::Failed(text), .. } if text == "start Node"
        )));
        assert!(matches!(s.unit(u).unwrap().state, UnitState::Failed { .. }));
    }

    /// A disable of a waking run (or a failed activation): the core's
    /// start goes with it, and so does the old pid's forwarding.
    #[test]
    fn b2c_a_stop_of_a_waking_run_ends_its_start() {
        let mut s = new();
        let u = ready(&mut s, "a.one", 1, 0, false);
        park(&mut s, u, 1, 60_000);
        let pid = wake(&mut s, u, 1, 61_000);
        s.process_spawned(pid, Ok(()));
        assert!(matches!(
            s.unit(u).unwrap().state,
            UnitState::Activating {
                sub: ActivatingSub::Ready,
                ..
            }
        ));
        s.disable(u).unwrap();
        assert!(kinds(&s.run_jobs(61_000)).contains(&"Terminate"));
        assert!(!s.job(u).is_some_and(|job| job.starts()));
        assert_eq!(s.wake(Pid(1), &Trigger::interactive(8)), Delivery::Dropped);
        s.process_reaped(pid, None);
        s.backend_cleaned(pid);
        s.run_jobs(70_000);
        assert_eq!(s.unit(u).unwrap().state, UnitState::Inactive);
    }

    /// Wake events (today's `lazy_event`): muted during start-up and 1 s
    /// after it settled for a unit in standby; a unit already waking holds
    /// them anyway (today's broadcast holds them in `lazy.waking`), only the
    /// newest of each.
    #[test]
    fn b2c_muted_wake_events_still_reach_a_waking_unit() {
        let mut s = new();
        s.set_muted(true);
        let u = ready(&mut s, "a.one", 1, 0, false);
        park(&mut s, u, 1, 60_000);
        let event = |n| Trigger::event("packages.changed", n);
        assert_eq!(s.wake(Pid(1), &event(1)), Delivery::Dropped);
        assert_eq!(s.wake(Pid(1), &Trigger::interactive(2)), Delivery::Started);
        s.advance(u);
        let pid = spawned(&s.run_jobs(60_001)).unwrap();
        s.process_spawned(pid, Ok(()));
        assert_eq!(s.wake(pid, &event(3)), Delivery::Held);
        assert_eq!(s.wake(pid, &event(4)), Delivery::Held);
        s.process_ready(pid);
        let delivered = s
            .run_jobs(60_001)
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

    /// A wake while the parked process still goes waits for it, and a
    /// disable meanwhile takes the wake with it (today's `Parked.waiting`
    /// went with the parked entry).
    #[test]
    fn b2c_a_wake_waits_for_the_parked_process_and_goes_with_its_standby() {
        let mut s = new();
        let u = ready(&mut s, "a.one", 1, 0, false);
        assert_eq!(kinds(&s.run_jobs(60_000)), ["QueryResources"]);
        s.resources_answered(Pid(1), false);
        s.run_jobs(60_000);
        s.set_now(60_100);
        assert_eq!(s.wake(Pid(1), &Trigger::interactive(7)), Delivery::Started);
        s.advance(u);
        assert!(kinds(&s.run_jobs(60_100)).is_empty(), "waits for the reap");
        s.process_reaped(Pid(1), None);
        s.backend_cleaned(Pid(1));
        assert_eq!(kinds(&s.run_jobs(60_200)), ["Rekey", "Spawn"]);
        // Again, disabled before the process went.
        let v = ready(&mut s, "b.two", 3, 60_200, false);
        s.run_jobs(120_200);
        s.resources_answered(Pid(3), false);
        s.run_jobs(120_200);
        assert_eq!(s.wake(Pid(3), &Trigger::interactive(7)), Delivery::Started);
        s.disable(v).unwrap();
        let actions = s.run_jobs(120_200);
        assert_eq!(
            kinds(&actions).iter().filter(|k| **k == "Revoke").count(),
            1,
            "{actions:?}"
        );
        assert!(!s.job(v).is_some_and(|job| job.will_start()));
        s.process_reaped(Pid(3), None);
        s.backend_cleaned(Pid(3));
        assert!(kinds(&s.run_jobs(120_300)).is_empty());
        assert_eq!(s.unit(v).unwrap().state, UnitState::Inactive);
        assert!(s.job(v).is_none());
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
    /// digest, a package operation's target before its commit: today's
    /// `lazy_key_of` is `None`) is never classified nor put in standby.
    #[test]
    fn b2c_an_unrecordable_run_is_never_classified() {
        let mut s = new();
        start_with(
            &mut s,
            UnitSpec::new("a.one", Class::Normal).unrecordable(),
            1,
            0,
        );
        s.classified_with(Pid(1), None, false);
        assert!(kinds(&s.run_jobs(0)).is_empty());
        assert!(s.run_jobs(600_000).is_empty());
        assert!(s.quit_checkpoints().is_empty());
        // `set_recordable(false)`: an install's target is not classified.
        let u = s.add_unit(UnitSpec::new("b.two", Class::Normal).disabled());
        s.set_recordable(u, false);
        s.set_enabled(u, true);
        s.restart(u, true, Some("digest".into())).unwrap();
        let pid = spawned(&s.run_jobs(600_000)).unwrap();
        s.process_spawned(pid, Ok(()));
        s.process_ready(pid);
        s.run_jobs(600_000);
        assert!(!s.classifying(pid));
        assert!(s.run_jobs(1_200_000).is_empty());
    }

    /// B2d: a woken run has one start deadline, the core's (the adapter
    /// gives the activation the same one): no ready within it, the run is
    /// stopped as failed ("activation timed out").
    #[test]
    fn b2d_a_wakes_start_times_out_once() {
        let mut s = new();
        let u = ready(&mut s, "a.one", 1, 0, false);
        park(&mut s, u, 1, 60_000);
        let pid = wake(&mut s, u, 1, 61_000);
        s.process_spawned(pid, Ok(()));
        assert!(!kinds(&s.run_jobs(75_999)).contains(&"Terminate"));
        assert!(kinds(&s.run_jobs(76_000)).contains(&"Terminate"));
        assert!(matches!(
            s.unit(u).unwrap().state,
            UnitState::Deactivating {
                then: Then::Failed,
                ..
            }
        ));
    }

    /// Review fix: a run's classification closes with its answer or its
    /// timeout; a later answer finds it closed (the adapter drops it).
    #[test]
    fn b2c_classification_closes_once() {
        let mut s = new();
        ready(&mut s, "a.one", 1, 0, true);
        assert!(s.classifying(Pid(1)));
        s.run_jobs(5_000);
        assert!(!s.classifying(Pid(1)), "timed out: resident");
        ready(&mut s, "b.two", 2, 5_000, false);
        assert!(s.classifying(Pid(2)));
        s.classified_with(Pid(2), None, false);
        assert!(!s.classifying(Pid(2)));
    }

    /// Review note B2-1: a reinstall that still waits for the uninstalled
    /// process does not replay (its pid stays tracked: the reap reaches the
    /// unit, the owed data removal goes, and the start runs then).
    #[test]
    fn b2d_a_reinstall_with_a_checkpoint_waits_without_replaying() {
        let mut s = new();
        let u = ready(&mut s, "a.one", 1, 0, true);
        s.stop(u, StopReason::Uninstall, Then::Removed).unwrap();
        assert!(kinds(&s.run_jobs(1)).contains(&"Terminate"));
        // A reinstall whose spec carries a checkpoint (the adapter's never
        // does: the uninstall drops it).
        s.add_unit(UnitSpec::new("a.one", Class::Normal).checkpoint(false));
        let actions = s.run_jobs(2);
        assert!(
            !actions.iter().any(|a| matches!(a, Action::Replay { .. })),
            "{actions:?}"
        );
        assert_eq!(s.by_pid(Pid(1)), Some(u));
        s.process_reaped(Pid(1), None);
        let actions = s.run_jobs(3);
        assert!(
            actions.contains(&Action::RemoveData { unit: u }),
            "{actions:?}"
        );
        s.backend_cleaned(Pid(1));
        let actions = s.run_jobs(4);
        assert_eq!(spawned(&actions), Some(Pid(2)), "{actions:?}");
    }
}
