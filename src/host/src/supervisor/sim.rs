//! Deterministic simulations of the core (the maintainer's own; the spec
//! tests from the design tables live in `tests/`). A small fake adapter
//! answers actions the way the Host does: spawn → spawned (+ ready),
//! terminate → reaped + backend cleaned, resources → not busy.
#![forbid(unsafe_code)]

use super::limits::Retry;
use super::*;

type S = Supervisor<u32>;

/// The preset (8 cores, 16 GiB) with the timer values these simulations
/// count with (idle 60 s, start 15 s, stop 300 ms, standby 2 s).
fn preset() -> Tunables {
    Tunables::new(8, 16_384).with_other_timers()
}

struct World {
    s: S,
    now: Ms,
    auto_spawn: bool,
    auto_ready: bool,
    auto_reap: bool,
    /// Answer `QueryResources` at once (not busy).
    auto_resources: bool,
    /// Answer `classified` at ready of a first run: `Some(resident)`.
    auto_classify: Option<bool>,
    log: Vec<Action<u32>>,
    delivered: Vec<(Pid, u32)>,
    answered: Vec<(AnswerReason, u32)>,
}

impl World {
    fn new(tunables: Tunables) -> Self {
        Self {
            s: Supervisor::new(tunables, 42),
            now: 0,
            auto_spawn: true,
            auto_ready: true,
            auto_reap: true,
            auto_resources: true,
            auto_classify: None,
            log: Vec::new(),
            delivered: Vec::new(),
            answered: Vec::new(),
        }
    }

    /// Runs jobs at `now` and lets the fake adapter answer until quiet.
    /// Returns every action of this tick.
    fn tick(&mut self, now: Ms) -> Vec<Action<u32>> {
        self.now = now;
        let mut all = Vec::new();
        loop {
            let actions = self.s.run_jobs(now);
            if actions.is_empty() {
                break;
            }
            for action in &actions {
                self.answer(action);
            }
            all.extend(actions);
        }
        self.log.extend(all.iter().cloned());
        all
    }
    fn answer(&mut self, action: &Action<u32>) {
        match *action {
            Action::Spawn { pid, first_run, .. } => {
                if !self.auto_spawn {
                    return;
                }
                self.s.process_spawned(pid, Ok(()));
                if self.auto_ready {
                    self.s.process_ready(pid);
                    if first_run && let Some(resident) = self.auto_classify {
                        let why = resident.then(|| "live".to_owned());
                        self.s.classified(pid, why);
                    }
                }
            }
            Action::Terminate { pid, .. } if self.auto_reap => {
                self.s.process_reaped(pid, Some(0));
                self.s.backend_cleaned(pid);
            }
            Action::QueryResources { pid, .. } if self.auto_resources => {
                self.s.resources_answered(pid, false);
            }
            Action::Deliver { pid, payload, .. } => self.delivered.push((pid, payload)),
            Action::Answer {
                reason, payload, ..
            } => self.answered.push((reason, payload)),
            _ => {}
        }
    }
    /// `process_exited`; a process that ended by itself gets no
    /// `Terminate`: the fake adapter cleans up
    /// after it on the exit and its reap follows (with V8's heap abort
    /// code for an `Oom`).
    fn exited(&mut self, pid: Pid, cause: ExitCause, text: Option<String>) {
        self.s.process_exited(pid, cause, text);
        let gone = matches!(cause, ExitCause::Exited | ExitCause::Oom);
        if self.auto_reap && gone {
            if cause == ExitCause::Oom {
                self.s.process_out_of_memory(pid);
            }
            let code = (cause == ExitCause::Oom).then_some(super::exit::HEAP_ABORT_STATUS);
            self.s.process_reaped(pid, code.or(Some(1)));
            self.s.backend_cleaned(pid);
        }
    }
    fn pid(&self, u: UnitId) -> Pid {
        self.s.unit(u).and_then(Unit::pid).expect("a process")
    }
    fn holder(&self, u: UnitId) -> Pid {
        match self.s.unit(u).map(|unit| &unit.state) {
            Some(UnitState::Standby { holder, .. }) => *holder,
            other => panic!("not in standby: {other:?}"),
        }
    }
    fn state(&self, u: UnitId) -> &UnitState {
        &self.s.unit(u).expect("unit").state
    }
}

fn spawns(actions: &[Action<u32>]) -> Vec<(UnitId, Pid, Placement)> {
    actions
        .iter()
        .filter_map(|a| match *a {
            Action::Spawn {
                unit,
                pid,
                placement,
                ..
            } => Some((unit, pid, placement)),
            _ => None,
        })
        .collect()
}
fn count(actions: &[Action<u32>], f: impl Fn(&Action<u32>) -> bool) -> usize {
    actions.iter().filter(|a| f(a)).count()
}
fn is_terminate(a: &Action<u32>) -> bool {
    matches!(a, Action::Terminate { .. })
}
fn active(world: &World, u: UnitId) -> bool {
    matches!(world.state(u), UnitState::Active { .. })
}

/// A unit in standby (recorded, not resident).
fn standby_unit(world: &mut World, id: &str) -> UnitId {
    world
        .s
        .add_unit(UnitSpec::new(id, Class::Normal).checkpoint(false))
}

// ---- start-up ----------------------------------------------------------

#[test]
fn verifying_builtin_holds_back_third_party_first_runs() {
    let mut w = World::new(preset());
    let b =
        w.s.add_unit(UnitSpec::new("b", Class::Builtin).shared().verify());
    let t = w.s.add_unit(UnitSpec::new("t", Class::Normal));
    let actions = w.tick(0);
    assert_eq!(actions.first(), Some(&Action::Verify { unit: b }));
    assert!(spawns(&actions).is_empty(), "third-party first run waits");
    w.s.verify_done(b, true);
    let started = spawns(&w.tick(5));
    assert_eq!(started[0].0, b);
    assert_eq!(started[0].2, Placement::Shared(w.s.runner()));
    assert_eq!(started[1].0, t);
}

#[test]
fn verify_timeout_runs_in_own_process() {
    let mut w = World::new(preset());
    let b =
        w.s.add_unit(UnitSpec::new("b", Class::Builtin).shared().verify());
    w.tick(0);
    assert!(spawns(&w.tick(2_999)).is_empty());
    let started = spawns(&w.tick(3_000));
    assert_eq!(started, [(b, started[0].1, Placement::Own)]);
}

// ---- standby and wake ---------------------------------------------------

#[test]
fn standby_wake_round_trip() {
    let mut w = World::new(preset());
    let u = standby_unit(&mut w, "n");
    let holder = w.holder(u);
    assert_eq!(w.tick(0), [Action::Replay { unit: u, holder }]);
    assert_eq!(
        w.s.wake(holder, &Trigger::interactive(7)),
        Delivery::Started
    );
    assert_eq!(w.s.wake(holder, &Trigger::follow(8)), Delivery::Held);
    let actions = w.tick(10);
    let pid = w.pid(u);
    assert_eq!(
        actions[..2],
        [
            Action::Rekey {
                unit: u,
                from: holder,
                to: pid
            },
            Action::Spawn {
                unit: u,
                pid,
                placement: Placement::Own,
                image: None,
                first_run: false
            },
        ]
    );
    assert!(actions.contains(&Action::Prune { unit: u, pid }));
    assert_eq!(w.delivered, [(pid, 7), (pid, 8)]);
    assert!(active(&w, u));
    // The old pid forwards; the new one delivers now.
    assert_eq!(
        w.s.wake(holder, &Trigger::interactive(9)),
        Delivery::Forward(pid)
    );
    assert_eq!(w.s.wake(pid, &Trigger::interactive(9)), Delivery::Now);
    // Idle 60 s after the last use.
    w.s.touch(pid, 1_000);
    assert!(!w.tick(60_999).iter().any(is_terminate));
    let actions = w.tick(61_000);
    assert!(actions.contains(&Action::QueryResources { unit: u, pid }));
    assert!(actions.contains(&Action::Terminate {
        unit: u,
        pid,
        reason: StopReason::Idle,
        grace: 2_000,
        keep_registrations: true
    }));
    assert_eq!(w.holder(u), pid);
    assert_eq!(
        w.s.list_state(u),
        Some(ListState::Running {
            process: ProcessState::Idle
        })
    );
}

#[test]
fn busy_answer_keeps_it_running_and_asks_again_later() {
    let mut w = World::new(preset());
    w.s = Supervisor::new(preset(), 1);
    let u = standby_unit(&mut w, "n");
    let holder = w.holder(u);
    w.s.wake(holder, &Trigger::interactive(1));
    w.tick(0);
    let pid = w.pid(u);
    w.s.begin_work(pid, WorkKind::Request(5), 100);
    assert!(!w.tick(70_000).iter().any(is_terminate), "work in flight");
    w.s.end_work(pid, WorkKind::Request(5));
    let actions = w.tick(140_000);
    assert!(actions.iter().any(is_terminate));
}

#[test]
fn wake_while_deactivating_waits_for_reap() {
    let mut w = World::new(preset());
    w.auto_reap = false;
    let u = standby_unit(&mut w, "n");
    let holder = w.holder(u);
    w.s.wake(holder, &Trigger::interactive(1));
    w.tick(0);
    let pid = w.pid(u);
    w.tick(60_000);
    assert!(matches!(
        w.state(u),
        UnitState::Deactivating {
            then: Then::Standby,
            ..
        }
    ));
    assert_eq!(w.s.wake(pid, &Trigger::interactive(2)), Delivery::Started);
    assert!(spawns(&w.tick(60_100)).is_empty());
    w.s.process_reaped(pid, Some(0));
    assert!(
        spawns(&w.tick(60_200)).is_empty(),
        "backend not cleaned yet"
    );
    w.s.backend_cleaned(pid);
    let actions = w.tick(60_300);
    let next = w.pid(u);
    assert!(actions.contains(&Action::Rekey {
        unit: u,
        from: pid,
        to: next
    }));
    assert_eq!(w.delivered.last(), Some(&(next, 2)));
}

#[test]
fn wake_whose_stopped_process_never_goes_fails_after_20_s() {
    let mut w = World::new(preset());
    w.auto_reap = false;
    let u = standby_unit(&mut w, "n");
    let holder = w.holder(u);
    w.s.wake(holder, &Trigger::interactive(1));
    w.tick(0);
    let pid = w.pid(u);
    w.tick(60_000);
    w.s.wake(pid, &Trigger::interactive(2));
    w.tick(60_001);
    // Kill at the standby grace.
    assert!(w.tick(62_000).contains(&Action::Kill { unit: u, pid }));
    assert!(w.tick(79_999).is_empty());
    let actions = w.tick(80_000);
    assert!(actions.contains(&Action::Revoke { unit: u, pid }));
    // One notice; the held click is answered.
    assert!(actions.contains(&Action::Notify {
        unit: Some(u),
        notice: Notice::CouldNotStart
    }));
    assert_eq!(w.answered.iter().map(|(_, p)| *p).collect::<Vec<_>>(), [2]);
    assert_eq!(
        w.s.list_state(u),
        Some(ListState::Failed(STUCK_WAKE.into()))
    );
    assert!(w.s.unit(u).unwrap().stuck);
}

// ---- 200 simultaneous wakes ---------------------------------------------

fn storm(tunables: Tunables) -> (World, Vec<UnitId>, usize) {
    let mut w = World::new(tunables);
    w.auto_ready = false;
    let units = (0..200)
        .map(|i| standby_unit(&mut w, &format!("n{i}")))
        .collect::<Vec<_>>();
    w.tick(0);
    w.s.settled(0);
    for (i, u) in units.iter().enumerate() {
        let holder = w.holder(*u);
        assert_eq!(
            w.s.wake(holder, &Trigger::interactive(i as u32)),
            Delivery::Started
        );
    }
    let mut max_activating = 0;
    let mut now = 1_000;
    let mut first = true;
    loop {
        let started = spawns(&w.tick(now));
        if first {
            max_activating = started.len();
            first = false;
        }
        let activating =
            w.s.units()
                .filter(|(_, unit)| matches!(unit.state, UnitState::Activating { .. }))
                .map(|(_, unit)| unit.pid().unwrap())
                .collect::<Vec<_>>();
        max_activating = max_activating.max(activating.len());
        if activating.is_empty() {
            break;
        }
        // One becomes ready per step.
        w.s.process_ready(activating[0]);
        now += 10;
    }
    (w, units, max_activating)
}

#[test]
fn storm_of_200_wakes_with_b3_limits_is_bounded_and_complete() {
    let (w, units, max) = storm(preset());
    // max_active 4 + 1 interactive reserve + 1 overrun.
    assert_eq!(max, 6);
    assert!(units.iter().all(|u| active(&w, *u)));
    assert_eq!(
        w.delivered.len(),
        200,
        "each trigger delivered exactly once"
    );
    assert_eq!(w.s.slots_in_use(), 0);
    // Within a priority, FIFO.
    let order = w
        .log
        .iter()
        .filter_map(|a| match a {
            Action::Spawn { unit, .. } => Some(*unit),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(order, units);
}

#[test]
fn event_wakes_use_the_plain_limit_and_are_muted_at_start_up() {
    let mut w = World::new(preset());
    w.auto_ready = false;
    let units = (0..10)
        .map(|i| standby_unit(&mut w, &format!("n{i}")))
        .collect::<Vec<_>>();
    w.tick(0);
    w.s.set_muted(true);
    let h0 = w.holder(units[0]);
    assert_eq!(w.s.wake(h0, &Trigger::event("e", 0)), Delivery::Dropped);
    w.s.settled(100);
    assert_eq!(w.s.wake(h0, &Trigger::event("e", 0)), Delivery::Dropped);
    w.tick(1_100);
    for u in &units {
        let holder = w.holder(*u);
        assert_eq!(w.s.wake(holder, &Trigger::event("e", 1)), Delivery::Started);
    }
    assert_eq!(spawns(&w.tick(1_200)).len(), 4);
}

// ---- disable, enable, reload ---------------------------------------------

#[test]
fn disable_then_enable_waits_for_reap_d13() {
    let mut w = World::new(preset());
    w.auto_reap = false;
    let u = w.s.add_unit(UnitSpec::new("n", Class::Normal));
    w.tick(0);
    let first = w.pid(u);
    w.s.set_enabled(u, false);
    w.s.stop(u, StopReason::Disable, Then::Inactive).unwrap();
    let actions = w.tick(10);
    assert!(actions.contains(&Action::Terminate {
        unit: u,
        pid: first,
        reason: StopReason::Disable,
        grace: 300,
        keep_registrations: false
    }));
    assert_eq!(w.s.list_state(u), Some(ListState::Disabled));
    w.s.set_enabled(u, true);
    let job = w.s.start(u, Cause::Enable).expect("enable is not busy");
    assert!(matches!(
        w.s.job(u).map(|j| &j.kind),
        Some(JobKind::Restart { .. })
    ));
    assert_eq!(w.s.list_state(u), Some(ListState::Starting));
    assert!(spawns(&w.tick(20)).is_empty(), "first process still going");
    w.s.process_reaped(first, Some(0));
    assert!(spawns(&w.tick(30)).is_empty(), "backend not cleaned");
    w.s.backend_cleaned(first);
    let started = spawns(&w.tick(40));
    assert_eq!(started.len(), 1);
    assert_ne!(started[0].1, first);
    assert!(active(&w, u));
    assert_eq!(w.s.job_result(u, job), Some(&JobResult::Done));
}

#[test]
fn package_rpcs_are_busy_while_one_is_installed() {
    let mut w = World::new(preset());
    w.auto_reap = false;
    let u = w.s.add_unit(UnitSpec::new("n", Class::Normal));
    w.tick(0);
    w.s.restart(u, true, None).unwrap();
    assert_eq!(w.s.restart(u, true, None), Err(Busy));
    assert_eq!(w.s.start(u, Cause::Enable), Err(Busy));
    assert_eq!(w.s.stop(u, StopReason::Uninstall, Then::Removed), Err(Busy));
    // Internal requests still merge.
    let standby = w.s.standby(u);
    assert_eq!(w.s.job_result(u, standby), Some(&JobResult::Dropped));
    // A disable is not a package RPC: a later reload is not busy (today).
    let v = w.s.add_unit(UnitSpec::new("v", Class::Normal));
    w.tick(1);
    w.s.set_enabled(v, false);
    w.s.stop(v, StopReason::Disable, Then::Inactive).unwrap();
    assert!(w.s.stop(v, StopReason::Uninstall, Then::Removed).is_ok());
}

#[test]
fn reload_while_activating_carries_held_triggers() {
    let mut w = World::new(preset());
    w.auto_ready = false;
    let u = standby_unit(&mut w, "n");
    let holder = w.holder(u);
    w.s.wake(holder, &Trigger::interactive(1));
    w.tick(0);
    let first = w.pid(u);
    let reload = w.s.restart(u, true, None).unwrap();
    let actions = w.tick(10);
    assert!(actions.contains(&Action::Terminate {
        unit: u,
        pid: first,
        reason: StopReason::Reload,
        grace: 300,
        keep_registrations: false
    }));
    assert!(actions.contains(&Action::DropCheckpoint { unit: u }));
    let started = spawns(&actions);
    assert_eq!(started.len(), 1);
    let second = started[0].1;
    assert!(actions.contains(&Action::Spawn {
        unit: u,
        pid: second,
        placement: Placement::Own,
        image: None,
        first_run: true
    }));
    w.s.process_ready(second);
    w.tick(20);
    // The click is carried to the new run.
    assert_eq!(w.delivered, [(second, 1)]);
    assert_eq!(w.s.job_result(u, reload), Some(&JobResult::Done));
}

#[test]
fn uninstall_from_standby_removes_data_and_refuses_triggers() {
    let mut w = World::new(preset());
    let u = standby_unit(&mut w, "n");
    let holder = w.holder(u);
    w.tick(0);
    w.s.stop(u, StopReason::Uninstall, Then::Removed).unwrap();
    let actions = w.tick(1);
    assert_eq!(
        actions[..3],
        [
            Action::Revoke {
                unit: u,
                pid: holder
            },
            Action::DropCheckpoint { unit: u },
            Action::RemoveData { unit: u },
        ]
    );
    assert_eq!(
        w.s.wake(holder, &Trigger::interactive(1)),
        Delivery::Dropped
    );
    w.s.remove_unit(u);
    assert_eq!(w.s.by_id("n"), None);
    assert_eq!(w.s.by_pid(holder), None);
}

// ---- failures -----------------------------------------------------------

#[test]
fn spawn_failure_keeps_entry_points() {
    let mut w = World::new(Tunables {
        normal: Policy {
            start_retry: None,
            ..preset().normal
        },
        ..preset()
    });
    w.auto_spawn = false;
    w.auto_ready = false;
    let u = standby_unit(&mut w, "n");
    let holder = w.holder(u);
    w.s.wake(holder, &Trigger::interactive(5));
    w.tick(0);
    let pid = w.pid(u);
    w.s.process_spawned(pid, Err("activation tracking limit reached".into()));
    let actions = w.tick(1);
    // The click is answered.
    assert_eq!(w.answered, [(AnswerReason::CouldNotStart, 5)]);
    assert!(actions.contains(&Action::Notify {
        unit: Some(u),
        notice: Notice::CouldNotStart
    }));
    assert!(!actions.contains(&Action::Revoke { unit: u, pid }));
    assert_eq!(w.holder(u), pid);
    // After its restart hold (the restart delay), the next click starts it.
    if let UnitState::Standby {
        hold_until: Some(until),
        ..
    } = *w.state(u)
    {
        w.tick(until);
    }
    assert_eq!(w.s.wake(pid, &Trigger::interactive(6)), Delivery::Started);
}

#[test]
fn b4_normal_crash_holds_in_standby_and_builtin_restarts() {
    let mut w = World::new(preset());
    let n = standby_unit(&mut w, "n");
    let b = w.s.add_unit(UnitSpec::new("b", Class::Builtin));
    w.auto_classify = Some(true);
    w.tick(0);
    let holder = w.holder(n);
    w.s.wake(holder, &Trigger::interactive(1));
    w.tick(10);
    let npid = w.pid(n);
    w.s.set_now(20);
    w.exited(npid, ExitCause::Fault, None);
    let actions = w.tick(20);
    assert!(actions.contains(&Action::Terminate {
        unit: n,
        pid: npid,
        reason: StopReason::Failed,
        grace: 300,
        keep_registrations: true
    }));
    assert!(actions.contains(&Action::Notify {
        unit: Some(n),
        notice: Notice::Crashed(ExitCause::Fault)
    }));
    assert!(matches!(
        w.state(n),
        UnitState::Standby {
            hold_until: Some(1_020),
            ..
        }
    ));
    assert_eq!(w.s.wake(npid, &Trigger::interactive(2)), Delivery::Refused);
    w.tick(30);
    assert_eq!(w.answered, [(AnswerReason::Held { until: 1_020 }, 2)]);
    assert_eq!(w.s.wake(npid, &Trigger::event("e", 3)), Delivery::Started);
    assert!(spawns(&w.tick(1_019)).is_empty());
    assert_eq!(spawns(&w.tick(1_020)).len(), 1);
    // Built-in: restart on failure after 3 s.
    let bpid = w.pid(b);
    w.s.set_now(2_000);
    w.exited(bpid, ExitCause::Oom, None);
    w.tick(2_000);
    assert!(matches!(
        w.state(b),
        UnitState::Activating {
            sub: ActivatingSub::AutoRestart { until: 5_000 },
            ..
        }
    ));
    assert_eq!(
        w.s.list_state(b),
        Some(ListState::Running {
            process: ProcessState::Idle
        })
    );
    assert_eq!(spawns(&w.tick(5_000)).len(), 1);
    assert!(active(&w, b));
}

#[test]
fn b4_start_limit_fails_after_five_crashes() {
    let mut w = World::new(preset());
    let b = w.s.add_unit(UnitSpec::new("b", Class::Builtin));
    w.auto_classify = Some(true);
    let mut now = 0;
    w.tick(now);
    for _ in 0..4 {
        let pid = w.pid(b);
        w.exited(pid, ExitCause::Exited, None);
        // Reaped at once; restarted after the backoff.
        w.tick(now);
        assert!(!active(&w, b));
        now += 100_000;
        w.tick(now);
        assert!(active(&w, b), "{:?} {:?}", w.state(b), w.s.job(b));
    }
    let pid = w.pid(b);
    w.exited(pid, ExitCause::Exited, None);
    let actions = w.tick(now + 1);
    assert!(actions.contains(&Action::Notify {
        unit: Some(b),
        notice: Notice::StartLimitHit
    }));
    assert!(matches!(
        w.state(b),
        UnitState::Failed {
            result: FailResult::StartLimitHit,
            ..
        }
    ));
}

#[test]
fn runtime_failed_is_never_retried() {
    let mut w = World::new(preset());
    w.auto_ready = false;
    let b =
        w.s.add_unit(UnitSpec::new("b", Class::Builtin).checkpoint(true));
    w.tick(0);
    let pid = w.pid(b);
    w.s.process_failed(pid, "no device".into());
    w.tick(10);
    assert_eq!(
        w.s.list_state(b),
        Some(ListState::Failed("no device".into()))
    );
    assert!(spawns(&w.tick(100_000)).is_empty());
}

// ---- runner group restart -------------------------------------------------

#[test]
fn runner_group_restart_then_fail_open() {
    // The members wait for the playback worker's cleanup.
    let mut w = World::new(preset());
    w.auto_reap = false;
    let members = (0..3)
        .map(|i| {
            w.s.add_unit(
                UnitSpec::new(format!("b{i}"), Class::Builtin)
                    .shared()
                    .verify(),
            )
        })
        .collect::<Vec<_>>();
    let own = w.s.add_unit(UnitSpec::new("own", Class::Builtin));
    w.tick(0);
    for u in &members {
        w.s.verify_done(*u, true);
    }
    let mut now = 10;
    w.tick(now);
    for round in 1..=3 {
        let runner = w.s.runner();
        assert!(
            members
                .iter()
                .all(|u| w.s.unit(*u).unwrap().placement == Placement::Shared(runner))
        );
        let pids = members.iter().map(|u| w.pid(*u)).collect::<Vec<_>>();
        w.s.runner_failed(runner, ExitCause::Fault);
        now += 1_000;
        let actions = w.tick(now);
        assert_eq!(
            count(&actions, |a| matches!(a, Action::KillRunner { .. })),
            1
        );
        assert_eq!(count(&actions, is_terminate), 3);
        assert!(
            actions
                .iter()
                .filter(|a| is_terminate(a))
                .all(|a| matches!(a, Action::Terminate { grace: 0, .. }))
        );
        let notice = if round == 3 {
            Notice::RunnerFailedOpen
        } else {
            Notice::RunnerRestarted
        };
        assert!(actions.contains(&Action::Notify { unit: None, notice }));
        assert!(active(&w, own), "own-process built-in untouched");
        assert!(spawns(&actions).is_empty(), "waits for member cleanup");
        assert_eq!(count(&actions, |a| matches!(a, Action::Verify { .. })), 0);
        for pid in pids {
            w.s.backend_cleaned(pid);
        }
        // The first runner restart is at once; later ones after the
        // runner's backoff (3 s × 2ⁿ).
        now += 10;
        let mut actions = w.tick(now);
        let delay = if round == 1 { 0 } else { 3_000 << (round - 2) };
        if delay > 0 {
            assert!(
                actions
                    .iter()
                    .all(|a| !matches!(a, Action::Verify { .. } | Action::Spawn { .. }))
            );
            now += delay;
            actions = w.tick(now);
        }
        if round < 3 {
            assert_eq!(count(&actions, |a| matches!(a, Action::Verify { .. })), 3);
            for u in &members {
                w.s.verify_done(*u, true);
            }
            now += 10;
            let started = spawns(&w.tick(now));
            assert_eq!(started.len(), 3);
            assert!(
                started
                    .iter()
                    .all(|(.., p)| *p == Placement::Shared(w.s.runner()))
            );
        } else {
            let started = spawns(&actions);
            assert_eq!(started.len(), 3, "fail-open: no verify, own processes");
            assert!(started.iter().all(|(.., p)| *p == Placement::Own));
            assert!(w.s.failed_open());
        }
    }
}

#[test]
fn member_transport_failure_restarts_the_group_and_fault_quarantines() {
    let mut w = World::new(preset());
    let a = w.s.add_unit(UnitSpec::new("a", Class::Builtin).shared());
    let b = w.s.add_unit(UnitSpec::new("b", Class::Builtin).shared());
    w.tick(0);
    let runner = w.s.runner();
    let apid = w.pid(a);
    w.exited(apid, ExitCause::Transport, None);
    let actions = w.tick(10);
    assert!(actions.contains(&Action::KillRunner { runner }));
    assert_eq!(count(&actions, is_terminate), 2);
    // Attributed fault: that member alone, its own process next.
    let bpid = w.pid(b);
    w.exited(bpid, ExitCause::Fault, None);
    let actions = w.tick(20);
    assert_eq!(
        count(&actions, |a| matches!(a, Action::KillRunner { .. })),
        0
    );
    assert!(w.s.unit(b).unwrap().quarantined);
}

// ---- backlog bounds and answers -------------------------------------------

#[test]
fn backlog_overflow_and_stop_answer_every_held_trigger() {
    let mut tunables = preset();
    tunables.backlog_max = 3;
    let mut w = World::new(tunables);
    w.auto_ready = false;
    let u = standby_unit(&mut w, "n");
    let holder = w.holder(u);
    w.tick(0);
    assert_eq!(
        w.s.wake(holder, &Trigger::interactive(1)),
        Delivery::Started
    );
    assert_eq!(w.s.wake(holder, &Trigger::event("e", 2)), Delivery::Held);
    assert_eq!(w.s.wake(holder, &Trigger::event("e", 3)), Delivery::Held);
    assert_eq!(w.s.wake(holder, &Trigger::interactive(4)), Delivery::Held);
    assert_eq!(
        w.s.wake(holder, &Trigger::interactive(5)),
        Delivery::Refused
    );
    assert_eq!(w.s.wake(holder, &Trigger::event("f", 6)), Delivery::Refused);
    w.tick(1);
    assert_eq!(w.answered, [(AnswerReason::Busy, 5)]);
    let held =
        w.s.unit(u)
            .unwrap()
            .backlog
            .iter()
            .map(|t| t.payload)
            .collect::<Vec<_>>();
    assert_eq!(held, [1, 3, 4], "only wake events merge, newest wins");
    w.s.set_enabled(u, false);
    w.s.stop(u, StopReason::Disable, Then::Inactive).unwrap();
    w.tick(2);
    assert_eq!(
        w.answered[1..],
        [(AnswerReason::Disabled, 1), (AnswerReason::Disabled, 4)]
    );
}

#[test]
fn lane_bound_refuses_new_starts() {
    let mut tunables = preset();
    tunables.lane_max = 2;
    tunables.max_active = 0;
    tunables.interactive_reserve = 0;
    tunables.interactive_overrun = 0;
    let mut w = World::new(tunables);
    let units = (0..3)
        .map(|i| standby_unit(&mut w, &format!("n{i}")))
        .collect::<Vec<_>>();
    w.tick(0);
    let mut deliveries = Vec::new();
    for (i, u) in units.iter().enumerate() {
        let holder = w.holder(*u);
        deliveries.push(w.s.wake(holder, &Trigger::interactive(i as u32)));
        w.tick(1);
    }
    assert_eq!(
        deliveries,
        [Delivery::Started, Delivery::Started, Delivery::Refused]
    );
    assert_eq!(w.answered, [(AnswerReason::Busy, 2)]);
}

// ---- budget / LRU ---------------------------------------------------------

fn budget_world() -> (World, [UnitId; 6]) {
    let mut tunables = preset();
    tunables.idle_timeout = 3_600_000;
    tunables.process_budget = 4;
    let mut w = World::new(tunables);
    let builtin =
        w.s.add_unit(UnitSpec::new("b", Class::Builtin).checkpoint(true));
    let resident =
        w.s.add_unit(UnitSpec::new("r", Class::Normal).checkpoint(true));
    let busy = standby_unit(&mut w, "busy");
    let old = standby_unit(&mut w, "old");
    let recent = standby_unit(&mut w, "recent");
    let new = standby_unit(&mut w, "new");
    w.tick(0);
    for (u, at) in [(busy, 0), (old, 0), (recent, 50_000)] {
        let holder = w.holder(u);
        w.s.set_now(at);
        w.s.wake(holder, &Trigger::interactive(0));
        w.tick(at);
    }
    let pid = w.pid(busy);
    w.s.begin_work(pid, WorkKind::Request(1), 60_000);
    assert_eq!(w.s.budget_count(), 4);
    (w, [builtin, resident, busy, old, recent, new])
}

#[test]
fn lru_picks_the_least_important_idle_unit_only() {
    let (mut w, [builtin, resident, busy, old, recent, new]) = budget_world();
    let holder = w.holder(new);
    w.s.set_now(100_000);
    w.s.wake(holder, &Trigger::interactive(1));
    let actions = w.tick(100_000);
    let asked = actions
        .iter()
        .filter_map(|a| match a {
            Action::QueryResources { unit, .. } => Some(*unit),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(asked, [old]);
    assert!(matches!(w.state(old), UnitState::Standby { .. }));
    for u in [builtin, resident, busy, recent, new] {
        assert!(active(&w, u), "{u:?}");
    }
}

#[test]
fn lru_never_picks_busy_and_overruns_when_none_qualifies() {
    let (mut w, [.., old, recent, new]) = budget_world();
    for u in [old, recent] {
        let pid = w.pid(u);
        w.s.begin_work(pid, WorkKind::Request(2), 90_000);
    }
    let holder = w.holder(new);
    w.s.wake(holder, &Trigger::interactive(1));
    let actions = w.tick(100_000);
    assert_eq!(
        count(&actions, |a| matches!(a, Action::QueryResources { .. })),
        0
    );
    assert_eq!(w.s.budget_count(), 5, "soft budget: overrun");
}

#[test]
fn event_wakes_never_cause_lru_standby() {
    let (mut w, [.., new]) = budget_world();
    let holder = w.holder(new);
    w.s.wake(holder, &Trigger::event("e", 1));
    let actions = w.tick(100_000);
    assert_eq!(
        count(&actions, |a| matches!(a, Action::QueryResources { .. })),
        0
    );
}

#[test]
fn memory_pressure_standby_skips_busy_and_resident() {
    let (mut w, [_, resident, busy, old, recent, _]) = budget_world();
    w.s.memory_pressure(Pressure::Low);
    w.tick(100_000);
    assert!(matches!(w.state(old), UnitState::Standby { .. }));
    assert!(matches!(w.state(recent), UnitState::Standby { .. }));
    assert!(active(&w, busy));
    assert!(active(&w, resident));
}

// ---- merge table cells ------------------------------------------------------

#[test]
fn merge_start_start_raises_priority() {
    let mut w = World::new(preset());
    let u = standby_unit(&mut w, "n");
    let holder = w.holder(u);
    w.tick(0);
    assert_eq!(w.s.wake(holder, &Trigger::event("e", 1)), Delivery::Started);
    let job = w.s.job(u).unwrap().id;
    assert_eq!(w.s.wake(holder, &Trigger::interactive(2)), Delivery::Held);
    let installed = w.s.job(u).unwrap();
    assert_eq!(installed.id, job);
    assert_eq!(installed.priority, Priority::Interactive);
}

#[test]
fn merge_standby_asking_then_trigger_cancels_standby() {
    let mut w = World::new(preset());
    // Classified resident at its first run: no standby of its own.
    w.auto_classify = Some(true);
    let u = w.s.add_unit(UnitSpec::new("n", Class::Normal));
    w.tick(0);
    let pid = w.pid(u);
    let standby = w.s.standby(u);
    let actions = w.s.run_jobs(10);
    assert!(actions.contains(&Action::QueryResources { unit: u, pid }));
    assert_eq!(w.s.wake(pid, &Trigger::interactive(1)), Delivery::Now);
    assert_eq!(w.s.job_result(u, standby), Some(&JobResult::Canceled));
    // The late answer is ignored.
    w.s.resources_answered(pid, false);
    assert!(!w.tick(20).iter().any(is_terminate));
}

#[test]
fn merge_restart_then_stop_stop_wins() {
    let mut w = World::new(preset());
    w.auto_reap = false;
    let u = w.s.add_unit(UnitSpec::new("n", Class::Normal));
    w.tick(0);
    let pid = w.pid(u);
    let restart = w.s.restart(u, false, None).unwrap();
    w.tick(10);
    let stop =
        w.s.enqueue_job(
            u,
            JobKind::Stop {
                reason: StopReason::Uninstall,
                then: Then::Removed,
            },
            JobMode::Replace,
            Priority::Background,
            Origin::Internal,
        )
        .unwrap();
    assert_eq!(w.s.job_result(u, restart), Some(&JobResult::Canceled));
    w.s.process_reaped(pid, None);
    w.s.backend_cleaned(pid);
    let actions = w.tick(20);
    assert!(actions.contains(&Action::RemoveData { unit: u }));
    assert!(spawns(&actions).is_empty());
    assert_eq!(w.s.job_result(u, stop), Some(&JobResult::Done));
}

#[test]
fn merge_stop_then_trigger_never_re_enables() {
    let mut w = World::new(preset());
    let u = standby_unit(&mut w, "n");
    let holder = w.holder(u);
    w.tick(0);
    w.s.set_enabled(u, false);
    w.s.stop(u, StopReason::Disable, Then::Inactive).unwrap();
    assert_eq!(
        w.s.wake(holder, &Trigger::interactive(1)),
        Delivery::Dropped
    );
    let actions = w.tick(1);
    assert!(actions.contains(&Action::Revoke {
        unit: u,
        pid: holder
    }));
    assert!(
        !actions.contains(&Action::DropCheckpoint { unit: u }),
        "checkpoint kept"
    );
    assert_eq!(w.s.list_state(u), Some(ListState::Disabled));
}

#[test]
fn stronger_then_wins_while_deactivating() {
    let mut w = World::new(preset());
    w.auto_reap = false;
    let u = standby_unit(&mut w, "n");
    let holder = w.holder(u);
    w.s.wake(holder, &Trigger::interactive(1));
    w.tick(0);
    let pid = w.pid(u);
    w.tick(60_000);
    // Deactivating for standby, registrations kept; an uninstall arrives: they
    // are revoked at once (today's `forget_parked`), the data goes at reap; its
    // checkpoint at the request.
    w.s.stop(u, StopReason::Uninstall, Then::Removed).unwrap();
    let actions = w.tick(60_001);
    assert!(actions.contains(&Action::Revoke { unit: u, pid }));
    assert!(actions.contains(&Action::DropCheckpoint { unit: u }));
    w.s.process_reaped(pid, None);
    w.s.backend_cleaned(pid);
    let actions = w.tick(60_002);
    assert_eq!(actions[..1], [Action::RemoveData { unit: u }]);
    assert!(!actions.contains(&Action::DropCheckpoint { unit: u }));
    assert!(!actions.iter().any(|a| matches!(a, Action::Revoke { .. })));
}

// ---- shutdown, timers -------------------------------------------------------

#[test]
fn shutdown_one_deadline_then_kill() {
    let mut w = World::new(preset());
    w.auto_reap = false;
    let units = (0..3)
        .map(|i| w.s.add_unit(UnitSpec::new(format!("n{i}"), Class::Normal)))
        .collect::<Vec<_>>();
    w.tick(0);
    let pids = units.iter().map(|u| w.pid(*u)).collect::<Vec<_>>();
    let actions = w.s.shutdown(100, 1_000);
    assert_eq!(count(&actions, is_terminate), 3);
    assert!(actions.iter().filter(|a| is_terminate(a)).all(|a| matches!(
        a,
        Action::Terminate {
            grace: 1_000,
            reason: StopReason::Quit,
            ..
        }
    )));
    w.s.process_reaped(pids[0], Some(0));
    let actions = w.s.run_jobs(1_100);
    assert_eq!(count(&actions, |a| matches!(a, Action::Kill { .. })), 2);
    assert_eq!(
        w.s.wake(pids[1], &Trigger::interactive(1)),
        Delivery::Dropped
    );
}

#[test]
fn timers_cost_nothing_until_due() {
    let mut w = World::new(preset());
    w.s = Supervisor::new(preset(), 3);
    let units = (0..1_000)
        .map(|i| standby_unit(&mut w, &format!("n{i}")))
        .collect::<Vec<_>>();
    w.tick(0);
    for u in &units {
        let holder = w.holder(*u);
        w.s.wake(holder, &Trigger::interactive(0));
    }
    w.tick(5);
    assert_eq!(w.s.next_deadline(), Some(60_005));
    assert!(w.s.run_jobs(60_004).is_empty());
    let actions = w.tick(60_005);
    assert_eq!(count(&actions, is_terminate), 1_000);
}

#[test]
fn list_changed_ignores_process_and_reports_state() {
    let mut w = World::new(preset());
    let u = standby_unit(&mut w, "n");
    w.tick(0);
    let holder = w.holder(u);
    w.s.wake(holder, &Trigger::interactive(1));
    w.auto_ready = false;
    let actions = w.tick(10);
    // A wake is `running` all along, as today (`lazy.waking`): nothing to
    // announce while it activates or once it is ready.
    assert_eq!(
        w.s.list_state(u),
        Some(ListState::Running {
            process: ProcessState::Running
        })
    );
    let pid = w.pid(u);
    w.s.process_ready(pid);
    let more = w.tick(20);
    assert!(active(&w, u));
    assert_eq!(
        count(&actions, |a| matches!(a, Action::ListChanged { .. }))
            + count(&more, |a| matches!(a, Action::ListChanged { .. })),
        0
    );
    // A first run waiting or activating is `starting` (today `first_queue`
    // and a pending activation): announced.
    let mut w2 = World::new(preset());
    w2.auto_ready = false;
    let f = w2.s.add_unit(UnitSpec::new("f", Class::Normal));
    assert_eq!(w2.s.list_state(f), Some(ListState::Starting));
    w2.tick(0);
    let fpid = w2.pid(f);
    w2.s.process_ready(fpid);
    let actions = w2.tick(10);
    assert_eq!(
        count(&actions, |a| *a == Action::ListChanged { unit: f }),
        1
    );
    // running → standby (idle): no change for `packages.changed`.
    let actions = w.tick(60_010);
    assert!(matches!(w.state(u), UnitState::Standby { .. }));
    assert_eq!(
        count(&actions, |a| matches!(a, Action::ListChanged { .. })),
        0
    );
}

#[test]
fn seeded_runs_are_deterministic() {
    let run = || {
        let mut tunables = preset();
        tunables.normal.start_retry = Some(Retry {
            count: 3,
            base: 250,
            jitter_pct: 50,
        });
        let mut w = World::new(tunables);
        w.auto_spawn = false;
        w.auto_ready = false;
        let u = standby_unit(&mut w, "n");
        let holder = w.holder(u);
        w.s.wake(holder, &Trigger::interactive(1));
        let mut log = w.tick(0);
        for t in 1..4 {
            let pid = w.pid(u);
            w.s.process_spawned(pid, Err("x".into()));
            log.extend(w.tick(t));
            let next = w.s.next_deadline().unwrap();
            log.extend(w.tick(next));
        }
        log
    };
    let a = run();
    assert_eq!(a, run());
    assert_eq!(spawns(&a).len(), 4, "1 start + 3 retries");
}

// ---- answers to the spec author's design questions -----------------

#[test]
fn q15_upgrade_of_a_disabled_unit_applies_without_starting() {
    let mut w = World::new(preset());
    let u = w.s.add_unit(
        UnitSpec::new("n", Class::Normal)
            .checkpoint(false)
            .disabled(),
    );
    w.tick(0);
    let job = w.s.restart(u, true, Some("v2".into())).unwrap();
    let actions = w.tick(10);
    assert!(actions.contains(&Action::DropCheckpoint { unit: u }));
    assert!(spawns(&actions).is_empty());
    assert_eq!(w.s.job_result(u, job), Some(&JobResult::Done));
    assert_eq!(w.s.list_state(u), Some(ListState::Disabled));
    // Enabled later, the new image starts as a first run.
    w.s.set_enabled(u, true);
    w.s.start(u, Cause::Enable).unwrap();
    let started = w.tick(20);
    assert!(started.contains(&Action::Spawn {
        unit: u,
        pid: w.pid(u),
        placement: Placement::Own,
        image: Some("v2".into()),
        first_run: true
    }));
    // A disabled unit with a job installed still refuses.
    let v = w.s.add_unit(UnitSpec::new("v", Class::Normal).disabled());
    w.s.stop(v, StopReason::Disable, Then::Inactive).unwrap();
    let refused = w.s.restart(v, true, None).unwrap();
    assert_eq!(w.s.job_result(v, refused), Some(&JobResult::Dropped));
}

#[test]
fn q24_backoff_and_hold_count_from_the_reap() {
    let mut w = World::new(preset());
    w.auto_reap = false;
    let n = standby_unit(&mut w, "n");
    let b =
        w.s.add_unit(UnitSpec::new("b", Class::Builtin).checkpoint(true));
    w.tick(0);
    let holder = w.holder(n);
    w.s.wake(holder, &Trigger::interactive(1));
    w.tick(10);
    let (npid, bpid) = (w.pid(n), w.pid(b));
    w.exited(npid, ExitCause::Exited, None);
    w.exited(bpid, ExitCause::Exited, None);
    w.tick(100);
    // Reaped 5 s after the exit: the hold and the 3 s backoff start then.
    w.s.set_now(5_100);
    for pid in [npid, bpid] {
        w.s.process_reaped(pid, Some(1));
        w.s.backend_cleaned(pid);
    }
    w.tick(5_100);
    assert!(matches!(
        w.state(n),
        UnitState::Standby {
            hold_until: Some(6_100),
            ..
        }
    ));
    assert!(matches!(
        w.state(b),
        UnitState::Activating {
            sub: ActivatingSub::AutoRestart { until: 8_100 },
            ..
        }
    ));
    assert!(spawns(&w.tick(8_099)).is_empty());
    assert_eq!(spawns(&w.tick(8_100)).len(), 1);
}

// ---- coordinator rulings after the spec tests -----------------------

#[test]
fn builtins_first() {
    let mut w = World::new(preset());
    w.auto_ready = false;
    let n = w.s.add_unit(UnitSpec::new("n", Class::Normal));
    let b = w.s.add_unit(UnitSpec::new("b", Class::Builtin));
    let r =
        w.s.add_unit(UnitSpec::new("r", Class::Normal).checkpoint(true));
    let order = spawns(&w.tick(0))
        .into_iter()
        .map(|(u, ..)| u)
        .collect::<Vec<_>>();
    // Third-party background starts wait until the built-in is active.
    assert_eq!(order, [b]);
    let pid = w.pid(b);
    w.s.process_ready(pid);
    let next = spawns(&w.tick(10))
        .into_iter()
        .map(|(u, ..)| u)
        .collect::<Vec<_>>();
    assert_eq!(next, [r, n]);
}

#[test]
fn runner_restart_replays_members_with_a_standby_checkpoint() {
    let mut w = World::new(preset());
    let a = standby_unit(&mut w, "x");
    let m = w.s.add_unit(
        UnitSpec::new("m", Class::Builtin)
            .shared()
            .checkpoint(false),
    );
    let k =
        w.s.add_unit(UnitSpec::new("k", Class::Builtin).shared().checkpoint(true));
    w.tick(0);
    let holder = w.holder(m);
    w.s.wake(holder, &Trigger::interactive(1));
    w.tick(10);
    assert!(active(&w, m) && active(&w, k));
    let mpid = w.pid(m);
    w.s.runner_failed(w.s.runner(), ExitCause::Fault);
    let actions = w.tick(20);
    let started = spawns(&actions);
    assert_eq!(started.len(), 1, "only the resident member is spawned");
    assert_eq!(started[0].0, k);
    let replayed = w.holder(m);
    assert!(actions.contains(&Action::Replay {
        unit: m,
        holder: replayed
    }));
    // The old pid is dead (today's `forget_running`): dropped; the new
    // holder wakes it.
    assert_eq!(w.s.wake(mpid, &Trigger::interactive(2)), Delivery::Dropped);
    assert_eq!(
        w.s.wake(replayed, &Trigger::interactive(3)),
        Delivery::Started
    );
    assert!(matches!(w.state(a), UnitState::Standby { .. }));
}

#[test]
fn crash_during_wake_keeps_a_start_only_for_held_events() {
    for with_event in [false, true] {
        let mut w = World::new(preset());
        w.auto_ready = false;
        let u = standby_unit(&mut w, "n");
        let holder = w.holder(u);
        w.tick(0);
        w.s.wake(holder, &Trigger::interactive(1));
        if with_event {
            w.s.wake(holder, &Trigger::event("e", 2));
        }
        w.tick(10);
        let pid = w.pid(u);
        w.s.process_spawned(pid, Ok(()));
        w.exited(pid, ExitCause::Fault, None);
        w.tick(20);
        assert_eq!(w.answered, [(AnswerReason::CouldNotStart, 1)]);
        let until = match w.state(u) {
            UnitState::Standby {
                hold_until: Some(until),
                ..
            } => *until,
            other => panic!("{other:?}"),
        };
        assert_eq!(w.s.job(u).is_some(), with_event);
        assert_eq!(spawns(&w.tick(until)).len(), usize::from(with_event));
    }
}

/// A Resident whose start times out restarts by its class's restart rule (on
/// failure, 1 s × 2ⁿ from the reap), never in standby. A start timeout is not
/// retried (coordinator ruling, supervisor.md §7); it used to be retried
/// 250 ms after the reap.
#[test]
fn resident_start_timeout_restarts_by_its_class_and_never_lands_in_standby() {
    let mut w = World::new(preset());
    w.auto_ready = false;
    let r =
        w.s.add_unit(UnitSpec::new("r", Class::Normal).checkpoint(true));
    w.tick(0);
    let pid = w.pid(r);
    let actions = w.tick(15_000);
    assert!(actions.contains(&Action::Terminate {
        unit: r,
        pid,
        reason: StopReason::Failed,
        grace: 300,
        keep_registrations: false
    }));
    assert!(matches!(
        w.state(r),
        UnitState::Activating {
            sub: ActivatingSub::AutoRestart { .. },
            ..
        }
    ));
    let next = w.s.next_deadline().unwrap();
    assert!((16_000..=16_300).contains(&next), "{next}");
    assert!(spawns(&w.tick(next - 1)).is_empty());
    assert_eq!(spawns(&w.tick(next)).len(), 1);
}

// ---- review probe cases ----------------------------------------------------
//
// The independent review's probe cases (one per deviation) and one test per
// fix. "Today" cites the host code before the supervisor.

fn is_checkpoint(a: &Action<u32>) -> bool {
    matches!(a, Action::Checkpoint { .. })
}

/// Item 1 (probe `reload_then_classified_lazy_parks_at_once`): only a
/// first run goes to standby at its classification (today's
/// `lazy_classified` checks `lazy.first_runs`); a reload runs until idle.
#[test]
fn only_a_first_run_goes_to_standby_when_classified() {
    {
        let tunables = preset();
        let idle = tunables.idle_timeout;
        let mut w = World::new(tunables);
        let u = standby_unit(&mut w, "n");
        w.tick(0);
        w.s.restart(u, true, None).unwrap();
        w.tick(10);
        let pid = w.pid(u);
        assert!(active(&w, u));
        w.s.classified(pid, None);
        let actions = w.tick(20);
        assert!(active(&w, u), "{actions:?}");
        assert_eq!(count(&actions, is_terminate), 0);
        w.tick(10 + idle);
        assert!(matches!(w.state(u), UnitState::Standby { .. }));
    }
    // A first run classified lazy goes after its quiet period.
    let tunables = preset();
    let quiet = tunables.first_run_quiet;
    let mut w = World::new(tunables);
    let u = w.s.add_unit(UnitSpec::new("f", Class::Normal));
    w.tick(0);
    let pid = w.pid(u);
    w.s.classified(pid, None);
    let actions = w.tick(10 + quiet);
    assert!(actions.contains(&Action::Checkpoint {
        unit: u,
        pid,
        resident: false
    }));
    assert!(matches!(w.state(u), UnitState::Standby { .. }));
    // Without a quiet period: at its classification.
    let mut tunables = preset();
    tunables.first_run_quiet = 0;
    let mut w = World::new(tunables);
    let u = w.s.add_unit(UnitSpec::new("f", Class::Normal));
    w.tick(0);
    let pid = w.pid(u);
    w.s.classified(pid, None);
    let actions = w.tick(10);
    assert!(actions.contains(&Action::Checkpoint {
        unit: u,
        pid,
        resident: false
    }));
    assert!(matches!(w.state(u), UnitState::Standby { .. }));
}

/// A first run classified lazy is not stopped at its classification: a
/// saved effect it restores just after its ready (a timer, a round trip)
/// would reach the host while it stops, be dropped, and its lazy
/// checkpoint would not restore it at the next start. It goes to standby
/// after `first_run_quiet` of idle time, through the idle timer's Standby
/// job (the adapter asks the playback worker again then); a use meanwhile
/// starts the period again, and a held resource taken up meanwhile
/// (`holds_live`) makes it resident: no standby, a resident checkpoint.
#[test]
fn a_lazy_first_run_goes_to_standby_after_its_quiet_period() {
    let tunables = preset();
    let quiet = tunables.first_run_quiet;
    assert!(quiet > 0 && quiet < tunables.idle_timeout);
    {
        let mut w = World::new(tunables.clone());
        let u = w.s.add_unit(UnitSpec::new("f", Class::Normal));
        w.tick(0);
        let pid = w.pid(u);
        w.s.classified(pid, None);
        let mut actions = w.tick(quiet - 1);
        assert!(active(&w, u), "{actions:?}");
        // Used just before the period ends: it starts again.
        w.s.touch(pid, quiet - 1);
        actions.extend(w.tick(quiet));
        actions.extend(w.tick(2 * quiet - 2));
        assert!(active(&w, u), "{actions:?}");
        assert_eq!(count(&actions, is_terminate), 0, "{actions:?}");
        assert_eq!(count(&actions, is_checkpoint), 0, "{actions:?}");
        // Quiet: the Standby job asks for its resources, then it goes.
        let actions = w.tick(2 * quiet);
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, Action::QueryResources { pid: p, .. } if *p == pid)),
            "{actions:?}"
        );
        assert!(actions.contains(&Action::Checkpoint {
            unit: u,
            pid,
            resident: false
        }));
        assert!(matches!(w.state(u), UnitState::Standby { .. }));
    }
    // Takes up something held within the period: resident, never parked.
    let mut w = World::new(tunables);
    let u = w.s.add_unit(UnitSpec::new("f", Class::Normal));
    w.tick(0);
    let pid = w.pid(u);
    w.s.classified(pid, None);
    w.tick(quiet / 2);
    assert!(w.s.holds_live(pid));
    let mut actions = w.tick(quiet / 2 + 1);
    actions.extend(w.tick(10 * quiet));
    assert!(actions.contains(&Action::Checkpoint {
        unit: u,
        pid,
        resident: true
    }));
    assert_eq!(count(&actions, is_terminate), 0, "{actions:?}");
    assert!(active(&w, u));
}

/// A lazy first run busy at its classification (the Host's busy checks,
/// synced as declared work before it: a command, a dialog, a settings
/// page) gets no quiet period: it waits the idle timeout after the work
/// ends, as before.
#[test]
fn a_first_run_busy_at_its_classification_waits_the_idle_timeout() {
    let tunables = preset();
    let (quiet, idle) = (tunables.first_run_quiet, tunables.idle_timeout);
    let mut w = World::new(tunables);
    let u = w.s.add_unit(UnitSpec::new("f", Class::Normal));
    w.tick(0);
    let pid = w.pid(u);
    w.s.begin_work(pid, WorkKind::Declared(1), 0);
    w.s.classified(pid, None);
    let mut actions = w.tick(quiet);
    w.s.set_now(2 * quiet);
    w.s.end_work(pid, WorkKind::Declared(1));
    actions.extend(w.tick(4 * quiet));
    actions.extend(w.tick(2 * quiet + idle - 1));
    assert!(active(&w, u), "{actions:?}");
    assert!(
        !actions
            .iter()
            .any(|a| matches!(a, Action::QueryResources { .. })),
        "no quiet period: {actions:?}"
    );
    let actions = w.tick(2 * quiet + idle);
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, Action::QueryResources { pid: p, .. } if *p == pid)),
        "{actions:?}"
    );
    assert!(matches!(w.state(u), UnitState::Standby { .. }));
}

/// A lazy first run whose quiet period ended while it was busy (the
/// adapter answered its Standby job's question busy: a command, a dialog
/// opened after its classification) falls back to the idle timeout: it is
/// not asked again every quiet period while it stays busy.
#[test]
fn a_first_run_found_busy_after_its_quiet_period_waits_the_idle_timeout() {
    let tunables = preset();
    let (quiet, idle) = (tunables.first_run_quiet, tunables.idle_timeout);
    let mut w = World::new(tunables);
    w.auto_resources = false;
    let u = w.s.add_unit(UnitSpec::new("f", Class::Normal));
    w.tick(0);
    let pid = w.pid(u);
    w.s.classified(pid, None);
    let asks = |actions: &[Action<u32>]| {
        actions
            .iter()
            .filter(|a| matches!(a, Action::QueryResources { pid: p, .. } if *p == pid))
            .count()
    };
    assert_eq!(asks(&w.tick(quiet)), 1);
    w.s.resources_answered(pid, true);
    let mut actions = w.tick(quiet);
    for t in (2..30).map(|n| n * quiet) {
        actions.extend(w.tick(t.min(quiet + idle - 1)));
    }
    assert_eq!(asks(&actions), 0, "not every quiet period: {actions:?}");
    assert!(active(&w, u));
    let actions = w.tick(quiet + idle);
    assert_eq!(asks(&actions), 1, "after the idle timeout: {actions:?}");
    w.s.resources_answered(pid, false);
    w.tick(quiet + idle);
    w.tick(quiet + idle + 1);
    assert!(matches!(w.state(u), UnitState::Standby { .. }));
}

/// Item 2 (probe `development_gets_a_checkpoint_after_classify_wait`): a
/// development load is never classified, checkpointed or put in standby
/// (today's `lazy_development` only logs).
#[test]
fn development_is_never_classified_or_checkpointed() {
    let mut w = World::new(preset());
    let u =
        w.s.add_unit(UnitSpec::new("d", Class::Development).start_timeout(120_000));
    let mut actions = w.tick(0);
    let pid = w.pid(u);
    assert!(actions.contains(&Action::Spawn {
        unit: u,
        pid,
        placement: Placement::Own,
        image: None,
        first_run: false
    }));
    actions.extend(w.tick(6_000));
    w.s.classified(pid, None);
    actions.extend(w.tick(6_001));
    actions.extend(w.tick(200_000));
    assert!(active(&w, u));
    actions.extend(w.s.shutdown(200_001, 1_000));
    assert_eq!(count(&actions, is_checkpoint), 0, "{actions:?}");
}

/// `UnitSpec::unrecordable` (a built-in without a digest, today's
/// `lazy_key` is `None`): never a first run, classified or in standby.
#[test]
fn unrecordable_builtin_starts_at_boot_and_stays() {
    let mut w = World::new(preset());
    let u =
        w.s.add_unit(UnitSpec::new("b", Class::Builtin).unrecordable());
    let mut actions = w.tick(0);
    let pid = w.pid(u);
    assert!(actions.contains(&Action::Spawn {
        unit: u,
        pid,
        placement: Placement::Own,
        image: None,
        first_run: false
    }));
    assert_eq!(w.s.slots_in_use(), 0, "no start slot");
    w.s.classified(pid, None);
    actions.extend(w.tick(200_000));
    assert!(active(&w, u));
    actions.extend(w.s.shutdown(200_001, 1_000));
    assert_eq!(count(&actions, is_checkpoint), 0, "{actions:?}");
}

/// Item 3 (probe `disabled_failed_unit_lists_failed`), with the fault text
/// `process_exited` carries: disabling a failed unit clears the failure
/// and lists `disabled` (today's `set_enabled(false)` + `package_list`).
#[test]
fn disabling_a_failed_unit_lists_disabled_and_clears_the_failure() {
    {
        let mut w = World::new(preset());
        let u =
            w.s.add_unit(UnitSpec::new("n", Class::Normal).checkpoint(true));
        w.tick(0);
        let pid = w.pid(u);
        let text = "uncaught exception: boom";
        w.exited(pid, ExitCause::Fault, Some(text.into()));
        w.tick(10);
        if w.s.tunables().resident.restart == RestartMode::No {
            assert_eq!(w.s.list_state(u), Some(ListState::Failed(text.into())));
        }
        w.s.disable(u).unwrap();
        w.tick(20);
        assert_eq!(w.s.list_state(u), Some(ListState::Disabled));
        assert!(matches!(w.state(u), UnitState::Inactive));
        assert_eq!(w.s.unit(u).unwrap().failure, None);
        w.s.enable(u).unwrap();
        w.tick(30);
        assert!(active(&w, u));
    }
}

/// A disable of a unit still stopping after its failure lands `Inactive`.
#[test]
fn disable_while_a_failure_stops_lands_inactive() {
    let mut w = World::new(preset());
    w.auto_reap = false;
    let u =
        w.s.add_unit(UnitSpec::new("n", Class::Normal).checkpoint(true));
    w.tick(0);
    let pid = w.pid(u);
    w.exited(pid, ExitCause::Exited, None);
    w.tick(10);
    w.s.disable(u).unwrap();
    w.tick(20);
    assert_eq!(w.s.list_state(u), Some(ListState::Disabled));
    w.s.process_reaped(pid, None);
    w.s.backend_cleaned(pid);
    w.tick(30);
    assert!(matches!(w.state(u), UnitState::Inactive));
    assert_eq!(w.s.unit(u).unwrap().failure, None);
}

/// Item 4 (probe `second_enable_while_waiting_is_busy`; the characterization test
/// `standby_holds_wake_enable_and_uninstall_until_the_parked_process_is_gone`):
/// an enable that waits for the process stopped for standby keeps the
/// merge table: an enable is never busy for an enable or a reload.
#[test]
fn second_enable_while_the_standby_process_goes() {
    {
        let (tunables, busy) = (preset(), false);
        let idle = tunables.idle_timeout;
        let mut w = World::new(tunables);
        w.auto_reap = false;
        let u = standby_unit(&mut w, "n");
        let holder = w.holder(u);
        w.s.wake(holder, &Trigger::interactive(9));
        w.tick(0);
        let pid = w.pid(u);
        let actions = w.tick(idle);
        assert!(actions.iter().any(is_terminate), "{actions:?}");
        w.s.disable(u).unwrap();
        w.s.enable(u).unwrap();
        w.tick(idle + 1);
        assert_eq!(w.s.enable(u).is_err(), busy);
        assert_eq!(w.s.package_busy(u), busy);
        assert_eq!(w.s.restart(u, false, None).is_err(), busy);
        assert_eq!(w.s.list_state(u), Some(ListState::Starting));
        w.s.process_reaped(pid, None);
        w.s.backend_cleaned(pid);
        w.auto_reap = true;
        let started = spawns(&w.tick(idle + 2));
        assert_eq!(started.len(), 1);
    }
}

/// `disable`/`enable` check `busy` before changing anything (today's
/// `set_enabled` checks `package_busy` first).
#[test]
fn disable_during_a_package_rpc_changes_nothing() {
    let mut w = World::new(preset());
    w.auto_reap = false;
    let u =
        w.s.add_unit(UnitSpec::new("n", Class::Normal).checkpoint(true));
    w.tick(0);
    w.s.restart(u, false, Some("2.0.0".into())).unwrap();
    w.tick(10);
    assert!(w.s.package_busy(u));
    assert_eq!(w.s.disable(u), Err(Busy));
    assert!(w.s.unit(u).unwrap().enabled());
}

/// Item 5 (probe `exit_today_notifies`): a crash shows a notice.
#[test]
fn crash_notice() {
    {
        let notice = true;
        let mut w = World::new(preset());
        let u =
            w.s.add_unit(UnitSpec::new("n", Class::Normal).checkpoint(true));
        w.tick(0);
        let pid = w.pid(u);
        w.exited(pid, ExitCause::Exited, None);
        let actions = w.tick(10);
        let crashed = actions.contains(&Action::Notify {
            unit: Some(u),
            notice: Notice::Crashed(ExitCause::Exited),
        });
        assert_eq!(crashed, notice, "{actions:?}");
    }
}

/// Item 5 (probe `boot_spawn_failure_notifies`): "could not start" for a
/// failed boot start too, not only a wake's.
#[test]
fn boot_start_failure_notice() {
    {
        let notice = true;
        // Resident by its checkpoint; no start retry.
        let mut w = World::new(Tunables {
            resident: Policy {
                start_retry: None,
                ..preset().resident
            },
            ..preset()
        });
        w.auto_spawn = false;
        let u =
            w.s.add_unit(UnitSpec::new("n", Class::Normal).checkpoint(true));
        w.tick(0);
        let pid = w.pid(u);
        w.s.process_spawned(pid, Err("boom".into()));
        let actions = w.tick(1);
        let shown = actions.contains(&Action::Notify {
            unit: Some(u),
            notice: Notice::CouldNotStart,
        });
        assert_eq!(shown, notice, "{actions:?}");
        assert_eq!(w.s.list_state(u), Some(ListState::Failed("boom".into())));
    }
}

/// Item 5: a start canceled by a disable answers its held triggers.
#[test]
fn canceled_start_answers_held_triggers() {
    let mut w = World::new(preset());
    w.auto_ready = false;
    let u = standby_unit(&mut w, "n");
    let holder = w.holder(u);
    w.s.wake(holder, &Trigger::interactive(1));
    w.tick(0);
    w.s.disable(u).unwrap();
    w.tick(10);
    assert_eq!(w.answered, [(AnswerReason::Disabled, 1)]);
}

/// Item 6 (probe `disable_during_standby_stop_revokes_only_at_reap`;
/// coordinator ruling): a disable, uninstall or reload of a unit stopping for
/// standby revokes its kept registrations at once, and not again at reap.
#[test]
fn leaving_a_standby_stop_revokes_at_once() {
    {
        let tunables = preset();
        for request in ["disable", "uninstall", "reload"] {
            let idle = tunables.idle_timeout;
            let mut w = World::new(tunables.clone());
            w.auto_reap = false;
            let u = standby_unit(&mut w, "n");
            let holder = w.holder(u);
            w.s.wake(holder, &Trigger::interactive(9));
            w.tick(0);
            let pid = w.pid(u);
            let actions = w.tick(idle);
            assert!(actions.contains(&Action::Terminate {
                unit: u,
                pid,
                reason: StopReason::Idle,
                grace: tunables.normal.standby_grace,
                keep_registrations: true
            }));
            let accepted = match request {
                "disable" => w.s.disable(u),
                "uninstall" => w.s.stop(u, StopReason::Uninstall, Then::Removed),
                _ => w.s.restart(u, true, None),
            };
            assert!(accepted.is_ok());
            let actions = w.tick(idle + 1);
            assert!(
                actions.contains(&Action::Revoke { unit: u, pid }),
                "{request}: {actions:?}"
            );
            w.auto_reap = true;
            w.s.process_reaped(pid, None);
            w.s.backend_cleaned(pid);
            let actions = w.tick(idle + 2);
            assert!(
                !actions.contains(&Action::Revoke { unit: u, pid }),
                "{request}: revoked once: {actions:?}"
            );
            assert!(!matches!(w.state(u), UnitState::Standby { .. }));
        }
    }
}

/// Item 7 (probe `stale_alias_wakes_standby_unit`): the previous pid
/// forwards while the run it moved to goes on (today's `lazy.moved`), and
/// is dropped once that run stops (`forget_running`).
#[test]
fn previous_pid_forwards_only_while_its_run_goes_on() {
    let mut w = World::new(preset());
    let u = standby_unit(&mut w, "n");
    let p1 = w.holder(u);
    w.tick(0);
    assert_eq!(w.s.wake(p1, &Trigger::interactive(1)), Delivery::Started);
    w.tick(10);
    let p2 = w.pid(u);
    assert_eq!(
        w.s.wake(p1, &Trigger::interactive(2)),
        Delivery::Forward(p2)
    );
    w.tick(70_000);
    assert_eq!(w.holder(u), p2);
    assert_eq!(w.s.wake(p1, &Trigger::interactive(3)), Delivery::Dropped);
    assert_eq!(w.s.by_pid(p1), None);
    assert_eq!(w.s.wake(p2, &Trigger::interactive(4)), Delivery::Started);
}

/// Item 7: a trigger to the previous pid cannot bypass a restart's absorption:
/// it is held for the new run, not forwarded.
#[test]
fn previous_pid_trigger_is_absorbed_by_a_restart() {
    let mut w = World::new(preset());
    let u = standby_unit(&mut w, "n");
    let p1 = w.holder(u);
    w.s.wake(p1, &Trigger::interactive(1));
    w.tick(0);
    let p2 = w.pid(u);
    w.s.restart(u, false, None).unwrap();
    assert_eq!(w.s.wake(p1, &Trigger::interactive(2)), Delivery::Held);
    assert_eq!(w.s.wake(p2, &Trigger::interactive(3)), Delivery::Held);
    w.tick(10);
    let p3 = w.pid(u);
    assert_eq!(w.delivered, [(p2, 1), (p3, 2), (p3, 3)]);
}

/// A dead pid (a run that stopped, not in standby) is dropped even while a
/// start is installed: today a message to a stopped owner has no receiver.
#[test]
fn dead_pid_is_dropped_while_a_restart_waits() {
    let mut w = World::new(preset());
    w.auto_reap = false;
    let u =
        w.s.add_unit(UnitSpec::new("n", Class::Normal).checkpoint(true));
    w.tick(0);
    let pid = w.pid(u);
    w.s.restart(u, false, None).unwrap();
    w.tick(10);
    // The stopping process is still the unit's current pid.
    assert_eq!(w.s.wake(pid, &Trigger::interactive(1)), Delivery::Held);
    w.s.process_reaped(pid, None);
    w.s.backend_cleaned(pid);
    w.auto_reap = true;
    w.tick(20);
    assert_ne!(w.pid(u), pid);
    assert_eq!(w.s.wake(pid, &Trigger::interactive(2)), Delivery::Dropped);
}

/// Item 8 (ruling every preset): every ready of a recordable unit may be
/// classified again (`lazy_ready` → `lazy_classify`): a plugin that holds a
/// live resource now is re-recorded resident; once per run. It keeps running.
#[test]
fn reclassified_resident_at_a_later_ready() {
    {
        let (tunables, idles) = (preset(), false);
        let idle = tunables.idle_timeout;
        let mut w = World::new(tunables);
        let u = standby_unit(&mut w, "n");
        let holder = w.holder(u);
        w.s.wake(holder, &Trigger::interactive(1));
        w.tick(0);
        let pid = w.pid(u);
        w.s.classified(pid, Some("live".into()));
        let actions = w.tick(10);
        assert!(
            actions.contains(&Action::Checkpoint {
                unit: u,
                pid,
                resident: true,
            }),
            "{actions:?}"
        );
        // Once per run.
        w.s.classified(pid, Some("live".into()));
        assert_eq!(count(&w.tick(11), is_checkpoint), 0);
        assert!(!idles);
        w.tick(5 * idle);
        assert!(active(&w, u), "resident: no idle standby");
        // At quit: already recorded resident, not written again.
        assert_eq!(count(&w.s.shutdown(5 * idle + 1, 1_000), is_checkpoint), 0);
    }
}

/// A unit classified resident (by its checkpoint, at a first run, a
/// resident built-in) never stands by when idle.
#[test]
fn resident_never_goes_to_standby_when_idle() {
    {
        let (tunables, idles) = (preset(), false);
        let idle = tunables.idle_timeout;
        let mut w = World::new(tunables.clone());
        // Resident by its checkpoint, a first run classified resident, and
        // a resident built-in.
        let r =
            w.s.add_unit(UnitSpec::new("r", Class::Normal).checkpoint(true));
        let b =
            w.s.add_unit(UnitSpec::new("b", Class::Builtin).checkpoint(true));
        w.auto_classify = Some(true);
        let f = w.s.add_unit(UnitSpec::new("f", Class::Normal));
        w.tick(0);
        let pids = [r, b, f].map(|u| w.pid(u));
        for u in [r, b, f] {
            assert!(active(&w, u));
        }
        let actions = w.tick(idle + 1);
        for (u, pid) in [r, b, f].into_iter().zip(pids) {
            let standby = matches!(w.state(u), UnitState::Standby { .. });
            assert_eq!(standby, idles, "{u:?} {actions:?}");
            // Its checkpoint still says resident.
            let written = actions.contains(&Action::Checkpoint {
                unit: u,
                pid,
                resident: true,
            });
            assert_eq!(written, idles, "{u:?} {actions:?}");
        }
        // The next session starts it at boot (not replayed).
        let mut next = World::new(tunables);
        let r = next
            .s
            .add_unit(UnitSpec::new("r", Class::Normal).checkpoint(true));
        let started = spawns(&next.tick(0));
        assert_eq!(started.len(), 1);
        assert!(active(&next, r));
    }
}

/// Item 8 (refined by a re-review): a run started from its checkpoint
/// is recorded at quit once its classification answer came back (today's
/// `lazy_quit` writes the classes in `lazy.classes`, filled by that
/// answer). A run that quits before the answer (within the 5 s after its
/// ready) keeps the checkpoint on disk as it is.
#[test]
fn shutdown_checkpoints_runs_whose_class_is_known() {
    let mut w = World::new(preset());
    let u = standby_unit(&mut w, "n");
    let q = standby_unit(&mut w, "q");
    for unit in [u, q] {
        let holder = w.holder(unit);
        w.s.wake(holder, &Trigger::interactive(1));
    }
    w.tick(0);
    let pid = w.pid(u);
    let r =
        w.s.add_unit(UnitSpec::new("r", Class::Normal).checkpoint(true));
    w.tick(10);
    w.s.classified(pid, None);
    let rpid = w.pid(r);
    w.s.classified(rpid, Some("live".into()));
    w.tick(15);
    let actions = w.s.shutdown(20, 1_000);
    assert!(actions.contains(&Action::Checkpoint {
        unit: u,
        pid,
        resident: false
    }));
    let written = |x: UnitId| {
        actions
            .iter()
            .any(|a| matches!(a, Action::Checkpoint { unit, .. } if *unit == x))
    };
    // Recorded resident and still resident: not written again.
    assert!(!written(r));
    // No answer yet: the old checkpoint stays.
    assert!(!written(q), "{actions:?}");
}

// ---- A stop into standby that lands stuck --------------------------------
//
// Probe `review-b1b`: after a stuck landing nothing kept the old process
// in view, so a later wake, enable or reload spawned beside it. Today
// (`lazy.rs` wake / `lazy_tick`, `package_manager.rs` `parked_process` +
// `packages.reloads`) each such request waits 20 s from itself for the old
// process, then fails; it starts at once if the process goes meanwhile.

/// A unit woken once and stopped for standby by idle at `idle`; its
/// process never goes. Returns (world, unit, old pid, stop time).
fn stuck_standby_stop(tunables: Tunables) -> (World, UnitId, Pid, Ms) {
    let idle = tunables.idle_timeout;
    let mut w = World::new(tunables);
    let u = standby_unit(&mut w, "n");
    let holder = w.holder(u);
    w.s.wake(holder, &Trigger::interactive(1));
    w.tick(0);
    let pid = w.pid(u);
    w.auto_reap = false;
    let actions = w.tick(idle);
    assert!(actions.iter().any(is_terminate), "{actions:?}");
    (w, u, pid, idle)
}

fn is_could_not_start(a: &Action<u32>) -> bool {
    matches!(
        a,
        Action::Notify {
            notice: Notice::CouldNotStart,
            ..
        }
    )
}

#[test]
fn wake_after_a_stuck_standby_stop_waits_20_s_from_the_wake() {
    {
        let tunables = preset();
        let wait = tunables.stop_wait;
        let (mut w, u, pid, stop) = stuck_standby_stop(tunables);
        let landed = stop + wait;
        assert!(spawns(&w.tick(landed)).is_empty());
        assert!(matches!(w.state(u), UnitState::Standby { .. }));
        assert!(w.s.unit(u).unwrap().stuck);
        let at = landed + 10_000;
        w.tick(at);
        assert_eq!(w.s.wake(pid, &Trigger::interactive(2)), Delivery::Started);
        assert!(spawns(&w.tick(at)).is_empty(), "never beside the old one");
        assert!(spawns(&w.tick(at + wait - 1)).is_empty());
        assert!(matches!(w.state(u), UnitState::Standby { .. }));
        let actions = w.tick(at + wait);
        assert!(spawns(&actions).is_empty());
        assert!(actions.contains(&Action::Revoke { unit: u, pid }));
        assert!(actions.iter().any(is_could_not_start), "{actions:?}");
        assert_eq!(
            w.s.list_state(u),
            Some(ListState::Failed(STUCK_WAKE.into()))
        );
        // Each later request waits again (here: the user's reload).
        let again = at + wait + 1;
        w.s.set_now(again);
        w.s.restart(u, false, None).unwrap();
        assert!(spawns(&w.tick(again)).is_empty());
        assert!(spawns(&w.tick(again + wait - 1)).is_empty());
        w.tick(again + wait);
        assert_eq!(
            w.s.list_state(u),
            Some(ListState::Failed(STUCK_RELOAD.into()))
        );
    }
}

#[test]
fn wake_after_a_stuck_standby_stop_starts_once_the_process_goes() {
    {
        let tunables = preset();
        let wait = tunables.stop_wait;
        let (mut w, u, pid, stop) = stuck_standby_stop(tunables);
        let landed = stop + wait;
        w.tick(landed);
        w.s.wake(pid, &Trigger::interactive(2));
        assert!(spawns(&w.tick(landed + 1_000)).is_empty());
        w.s.process_reaped(pid, Some(0));
        assert!(spawns(&w.tick(landed + 2_000)).is_empty(), "not cleaned");
        w.s.backend_cleaned(pid);
        let actions = w.tick(landed + 3_000);
        let started = spawns(&actions);
        assert_eq!(started.len(), 1, "{actions:?}");
        let next = started[0].1;
        assert!(actions.contains(&Action::Rekey {
            unit: u,
            from: pid,
            to: next
        }));
        assert!(active(&w, u));
        assert_eq!(w.delivered.last(), Some(&(next, 2)));
    }
}

#[test]
fn enable_after_a_stuck_standby_stop_waits_and_fails() {
    {
        let (tunables, busy) = (preset(), false);
        let wait = tunables.stop_wait;
        let (mut w, u, _pid, stop) = stuck_standby_stop(tunables);
        let landed = stop + wait;
        w.tick(landed);
        w.s.disable(u).unwrap();
        w.tick(landed + 1_000);
        assert_eq!(w.s.list_state(u), Some(ListState::Disabled));
        let at = landed + 2_000;
        w.s.set_now(at);
        w.s.enable(u).unwrap();
        // Today the enable is an entry in `packages.reloads`: busy.
        assert_eq!(w.s.package_busy(u), busy);
        assert!(spawns(&w.tick(at)).is_empty(), "never beside the old one");
        assert!(spawns(&w.tick(at + wait - 1)).is_empty());
        let actions = w.tick(at + wait);
        assert!(spawns(&actions).is_empty());
        assert!(!actions.iter().any(is_could_not_start), "{actions:?}");
        assert_eq!(
            w.s.list_state(u),
            Some(ListState::Failed(STUCK_RELOAD.into()))
        );
    }
}

#[test]
fn reload_after_a_stuck_standby_stop_waits_and_fails() {
    {
        let tunables = preset();
        let wait = tunables.stop_wait;
        let (mut w, u, pid, stop) = stuck_standby_stop(tunables);
        let landed = stop + wait;
        w.tick(landed);
        let at = landed + 1;
        w.s.set_now(at);
        let job = w.s.restart(u, true, None).unwrap();
        let actions = w.tick(at);
        assert!(spawns(&actions).is_empty(), "never beside the old one");
        // As today's reload: its recording goes and its entry points too.
        assert!(actions.contains(&Action::DropCheckpoint { unit: u }));
        assert!(actions.contains(&Action::Revoke { unit: u, pid }));
        assert!(spawns(&w.tick(at + wait - 1)).is_empty());
        w.tick(at + wait);
        assert_eq!(
            w.s.job_result(u, job),
            Some(&JobResult::Failed(STUCK_RELOAD.into()))
        );
        assert_eq!(
            w.s.list_state(u),
            Some(ListState::Failed(STUCK_RELOAD.into()))
        );
    }
}

/// A wake 15 s into the stop waits 20 s from the wake (today), not the 5 s
/// left of the stop's own wait; it starts if the process goes meanwhile.
#[test]
fn wake_late_in_a_standby_stop_waits_20_s_from_the_wake() {
    {
        let tunables = preset();
        let wait = tunables.stop_wait;
        let (mut w, u, pid, stop) = stuck_standby_stop(tunables.clone());
        let at = stop + 15_000;
        w.tick(at);
        w.s.wake(pid, &Trigger::interactive(2));
        let actions = w.tick(stop + wait);
        assert!(spawns(&actions).is_empty());
        assert!(!actions.iter().any(is_could_not_start), "{actions:?}");
        assert!(matches!(w.state(u), UnitState::Standby { .. }));
        assert!(spawns(&w.tick(at + wait - 1)).is_empty());
        let actions = w.tick(at + wait);
        assert!(actions.iter().any(is_could_not_start), "{actions:?}");
        assert_eq!(
            w.s.list_state(u),
            Some(ListState::Failed(STUCK_WAKE.into()))
        );

        // The same wake, and the process goes 2 s after the stop's 20 s.
        let (mut w, u, pid, stop) = stuck_standby_stop(tunables);
        let at = stop + 15_000;
        w.tick(at);
        w.s.wake(pid, &Trigger::interactive(2));
        w.tick(stop + wait);
        w.s.process_reaped(pid, Some(0));
        w.s.backend_cleaned(pid);
        let started = spawns(&w.tick(stop + wait + 2_000));
        assert_eq!(started.len(), 1);
        assert!(active(&w, u));
    }
}

/// Uninstalled while the process of a stuck stop may still write its data:
/// the data goes once that process goes (today's `data_removals`).
#[test]
fn uninstall_after_a_stuck_stop_removes_data_once_the_process_goes() {
    let wait = preset().stop_wait;
    let (mut w, u, pid, stop) = stuck_standby_stop(preset());
    w.tick(stop + wait);
    w.s.stop(u, StopReason::Uninstall, Then::Removed).unwrap();
    let actions = w.tick(stop + wait + 1);
    assert!(!actions.contains(&Action::RemoveData { unit: u }));
    assert_eq!(w.s.list_state(u), Some(ListState::Stopped));
    w.s.process_reaped(pid, Some(0));
    w.s.backend_cleaned(pid);
    let actions = w.tick(stop + wait + 2);
    assert!(actions.contains(&Action::RemoveData { unit: u }));
}

/// `"Starting <name>…"` 400 ms after the spawn (today `lazy.waking.since`),
/// also for a wake that first waited for the old process.
#[test]
fn starting_notice_counts_from_the_spawn() {
    let (mut w, u, pid, stop) = stuck_standby_stop(preset());
    w.auto_ready = false;
    w.s.wake(pid, &Trigger::interactive(2));
    w.tick(stop + 1);
    w.s.process_reaped(pid, Some(0));
    w.s.backend_cleaned(pid);
    let spawned_at = stop + 3_000;
    assert_eq!(spawns(&w.tick(spawned_at)).len(), 1);
    let notice = |a: &Action<u32>| {
        matches!(
            a,
            Action::Notify {
                notice: Notice::Starting,
                ..
            }
        )
    };
    assert!(!w.tick(spawned_at + 399).iter().any(notice));
    assert!(w.tick(spawned_at + 400).iter().any(notice));
    assert!(matches!(w.state(u), UnitState::Activating { .. }));
}

/// Item 9b: a boot start not yet spawned (verifying) lists `starting`.
#[test]
fn boot_start_waiting_lists_starting() {
    {
        let (tunables, listed) = (preset(), ListState::Starting);
        let mut w = World::new(tunables);
        let u =
            w.s.add_unit(UnitSpec::new("b", Class::Builtin).verify().checkpoint(true));
        w.tick(0);
        assert!(matches!(
            w.state(u),
            UnitState::Activating {
                sub: ActivatingSub::Verify,
                ..
            }
        ));
        assert_eq!(w.s.list_state(u), Some(listed));
        w.s.verify_done(u, true);
        w.tick(1);
        assert!(active(&w, u));
    }
    // A first run waiting for a place is `starting` today too (`first_queue`).
    let mut w = World::new(preset());
    w.auto_ready = false;
    let units = (0..9)
        .map(|i| w.s.add_unit(UnitSpec::new(format!("f{i}"), Class::Normal)))
        .collect::<Vec<_>>();
    w.tick(0);
    assert!(matches!(w.state(units[8]), UnitState::Inactive));
    assert_eq!(w.s.list_state(units[8]), Some(ListState::Starting));
}

/// Item 9d: a crash cancels an installed Standby job (it did not go to
/// standby); the crash's own restart is installed.
#[test]
fn crash_cancels_an_installed_standby_job() {
    let mut w = World::new(preset());
    let u =
        w.s.add_unit(UnitSpec::new("n", Class::Normal).checkpoint(true));
    w.tick(0);
    let pid = w.pid(u);
    // Before its idle timeout; ask for standby directly.
    let job = w.s.standby(u);
    w.exited(pid, ExitCause::Exited, None);
    w.tick(10);
    assert_eq!(w.s.job_result(u, job), Some(&JobResult::Canceled));
    // Resident by its checkpoint: its restart rule starts it again.
    assert!(matches!(
        w.s.job(u).map(|j| &j.kind),
        Some(JobKind::Start {
            cause: Cause::AutoRestart
        })
    ));

    let mut w = World::new(preset());
    let b =
        w.s.add_unit(UnitSpec::new("b", Class::Builtin).checkpoint(true));
    w.auto_reap = false;
    w.tick(0);
    let pid = w.pid(b);
    let job = w.s.standby(b);
    w.exited(pid, ExitCause::Exited, None);
    w.tick(10);
    assert_eq!(w.s.job_result(b, job), Some(&JobResult::Canceled));
    assert!(matches!(
        w.s.job(b).map(|j| &j.kind),
        Some(JobKind::Start {
            cause: Cause::AutoRestart
        })
    ));
}

// ---- A shared member stopped for standby whose reap never came ------

/// A shared member woken once and stopped for standby by idle. The playback
/// worker cleaned up after it at once (`owner.parked`); its reap
/// (`MEMBER_GONE`) never came. Returns (world, unit, old pid, stop time).
fn shared_standby_stop(tunables: Tunables) -> (World, UnitId, Pid, Ms) {
    let idle = tunables.idle_timeout;
    let mut w = World::new(tunables);
    let u = w.s.add_unit(
        UnitSpec::new("m", Class::Builtin)
            .shared()
            .checkpoint(false),
    );
    w.tick(0);
    let holder = w.holder(u);
    w.s.wake(holder, &Trigger::interactive(1));
    w.tick(0);
    assert_eq!(
        w.s.unit(u).unwrap().placement,
        Placement::Shared(w.s.runner())
    );
    let pid = w.pid(u);
    w.auto_reap = false;
    assert!(w.tick(idle).iter().any(is_terminate));
    w.s.backend_cleaned(pid);
    (w, u, pid, idle)
}

fn is_remove_data(a: &Action<u32>) -> bool {
    matches!(a, Action::RemoveData { .. })
}

/// Probe i10 / i9: a runner restart reaps a member whose cleanup came
/// first. It is gone: a member still stopping lands (not stuck), one whose
/// stop landed stuck is no longer tracked, and the next wake starts it.
/// Before the fix neither was cleared and every later start failed stuck.
#[test]
fn runner_restart_clears_a_member_cleaned_up_before_its_reap() {
    {
        let tunables = preset();
        let wait = tunables.stop_wait;
        // i10: still deactivating.
        let (mut w, u, _pid, stop) = shared_standby_stop(tunables.clone());
        w.tick(stop + 1_000);
        assert!(w.state(u).is_deactivating());
        w.s.runner_failed(w.s.runner(), ExitCause::Exited);
        w.tick(stop + 1_001);
        assert!(
            matches!(w.state(u), UnitState::Standby { .. }),
            "{:?}",
            w.state(u)
        );
        let unit = w.s.unit(u).unwrap();
        assert!(unit.stopping.is_none() && !unit.stuck);
        let holder = w.holder(u);
        w.s.wake(holder, &Trigger::interactive(2));
        assert_eq!(spawns(&w.tick(stop + 2_000)).len(), 1);
        w.tick(stop + wait + 2_000);
        assert!(active(&w, u), "{:?}", w.state(u));

        // i9: the stop landed stuck first, then the runner failed.
        let (mut w, u, _pid, stop) = shared_standby_stop(tunables.clone());
        w.tick(stop + wait);
        assert!(w.s.unit(u).unwrap().stuck);
        w.s.runner_failed(w.s.runner(), ExitCause::Exited);
        w.tick(stop + wait + 1);
        assert!(w.s.unit(u).unwrap().stopping.is_none());
        let holder = w.holder(u);
        w.s.wake(holder, &Trigger::interactive(2));
        assert_eq!(spawns(&w.tick(stop + wait + 1_000)).len(), 1);
        w.tick(stop + 2 * wait + 2_000);
        assert!(active(&w, u), "{:?}", w.state(u));
    }
}

/// Probe i12: a reload 5 s into a standby stop that lands stuck waits 20 s
/// from the reload (the merged job counts from the request: today's
/// `packages.reloads` entry is written with the request's time).
#[test]
fn reload_late_in_a_standby_stop_waits_20_s_from_the_reload() {
    {
        let tunables = preset();
        let wait = tunables.stop_wait;
        let (mut w, u, _pid, stop) = stuck_standby_stop(tunables.clone());
        let at = stop + 5_000;
        w.tick(at);
        let job = w.s.restart(u, true, None).unwrap();
        assert!(spawns(&w.tick(stop + wait)).is_empty());
        assert_eq!(w.s.job_result(u, job), None, "its own 20 s are not over");
        assert!(spawns(&w.tick(at + wait - 1)).is_empty());
        assert_eq!(w.s.job_result(u, job), None);
        w.tick(at + wait);
        assert_eq!(
            w.s.job_result(u, job),
            Some(&JobResult::Failed(STUCK_RELOAD.into()))
        );
        assert_eq!(
            w.s.list_state(u),
            Some(ListState::Failed(STUCK_RELOAD.into()))
        );

        // The same reload, and the process goes 2 s after the stop's 20 s.
        let (mut w, u, pid, stop) = stuck_standby_stop(tunables);
        let at = stop + 5_000;
        w.tick(at);
        let job = w.s.restart(u, true, None).unwrap();
        w.tick(stop + wait);
        w.s.process_reaped(pid, Some(0));
        w.s.backend_cleaned(pid);
        assert_eq!(spawns(&w.tick(stop + wait + 2_000)).len(), 1);
        assert_eq!(w.s.job_result(u, job), Some(&JobResult::Done));
        assert!(active(&w, u));
    }
}

/// Disable then enable during a standby stop that lands stuck: the enable
/// waits 20 s from the enable (today `set_enabled` → `parked_process` →
/// `reloads(now)`), not from the stop.
#[test]
fn enable_late_in_a_standby_stop_waits_20_s_from_the_enable() {
    let failed = |w: &World, u| matches!(w.s.list_state(u), Some(ListState::Failed(_)));
    {
        let (tunables, busy) = (preset(), false);
        let wait = tunables.stop_wait;
        let (mut w, u, _pid, stop) = stuck_standby_stop(tunables.clone());
        w.tick(stop + 2_000);
        w.s.disable(u).unwrap();
        let at = stop + 10_000;
        w.tick(at);
        w.s.enable(u).unwrap();
        assert_eq!(w.s.package_busy(u), busy);
        assert!(spawns(&w.tick(stop + wait)).is_empty());
        assert!(!failed(&w, u), "{:?}", w.s.list_state(u));
        assert!(spawns(&w.tick(at + wait - 1)).is_empty());
        assert!(!failed(&w, u), "{:?}", w.s.list_state(u));
        w.tick(at + wait);
        assert_eq!(
            w.s.list_state(u),
            Some(ListState::Failed(STUCK_RELOAD.into()))
        );

        // The process goes 5 s after the stop's 20 s: it starts.
        let (mut w, u, pid, stop) = stuck_standby_stop(tunables);
        w.tick(stop + 2_000);
        w.s.disable(u).unwrap();
        let at = stop + 10_000;
        w.tick(at);
        w.s.enable(u).unwrap();
        w.tick(stop + wait);
        w.s.process_reaped(pid, Some(0));
        w.s.backend_cleaned(pid);
        assert_eq!(spawns(&w.tick(stop + wait + 5_000)).len(), 1);
        assert!(active(&w, u));
    }
}

/// Probe i3: uninstall of a unit whose stuck process was reaped but not yet
/// cleaned up after: the data goes only once the cleanup is in too. Exactly
/// one `RemoveData`.
#[test]
fn uninstall_after_the_reap_of_a_stuck_process_removes_data_once_cleaned() {
    {
        let (tunables, waits) = (preset(), true);
        let wait = tunables.stop_wait;
        let (mut w, u, pid, stop) = stuck_standby_stop(tunables);
        let landed = stop + wait;
        w.tick(landed);
        w.s.process_reaped(pid, Some(0));
        w.tick(landed + 1);
        w.s.stop(u, StopReason::Uninstall, Then::Removed).unwrap();
        let actions = w.tick(landed + 2);
        assert_eq!(actions.iter().any(is_remove_data), !waits, "{actions:?}");
        w.s.backend_cleaned(pid);
        let actions = w.tick(landed + 3);
        assert_eq!(actions.iter().any(is_remove_data), waits, "{actions:?}");
        assert_eq!(count(&w.log, is_remove_data), 1);
    }
}

/// Probe i4: an uninstall whose stop lands stuck. The unit is removed (its
/// checkpoint goes); its data goes when the late reap arrives (today's
/// `data_removals` at `Event::Reaped`; now once cleaned up too).
#[test]
fn stuck_uninstall_removes_data_at_the_late_reap() {
    {
        let (tunables, waits) = (preset(), true);
        let wait = tunables.stop_wait;
        let mut w = World::new(tunables);
        let u = standby_unit(&mut w, "n");
        let holder = w.holder(u);
        w.s.wake(holder, &Trigger::interactive(1));
        w.tick(0);
        let pid = w.pid(u);
        w.auto_reap = false;
        w.tick(100);
        let job = w.s.stop(u, StopReason::Uninstall, Then::Removed).unwrap();
        // Its checkpoint goes at the request.
        assert!(w.tick(100).contains(&Action::DropCheckpoint { unit: u }));
        let actions = w.tick(100 + wait);
        assert!(!actions.iter().any(is_remove_data), "{actions:?}");
        assert_eq!(
            w.s.job_result(u, job),
            Some(&JobResult::Failed(STUCK_RELOAD.into()))
        );
        assert!(w.s.unit(u).unwrap().removed);
        w.s.process_reaped(pid, Some(0));
        let actions = w.tick(100 + wait + 1_000);
        assert_eq!(actions.iter().any(is_remove_data), !waits, "{actions:?}");
        w.s.backend_cleaned(pid);
        let actions = w.tick(100 + wait + 2_000);
        assert_eq!(actions.iter().any(is_remove_data), waits, "{actions:?}");
        assert_eq!(count(&w.log, is_remove_data), 1);
        assert!(w.s.unit(u).unwrap().stopping.is_none());
    }
}

/// Probe i5: a reinstall of the same id while the uninstalled unit's process
/// may still run. `add_unit` brings the unit back under the same `UnitId` (its
/// spec replaced) and the start waits 20 s from the reinstall for that process,
/// as any start after a stuck stop (today the install operation waits for
/// `parked_process`). Never two processes on one data directory; the
/// uninstall's `RemoveData` comes before the new spawn and only once (the
/// adapter deletes the directory it resolved at the uninstall); if the process
/// never goes, no `RemoveData` and the start fails.
#[test]
fn reinstall_while_the_uninstalled_process_goes_waits_for_it() {
    let order = |actions: &[Action<u32>], u| {
        let removed = actions
            .iter()
            .position(|a| *a == Action::RemoveData { unit: u })
            .expect("RemoveData");
        let spawned = actions
            .iter()
            .position(|a| matches!(a, Action::Spawn { .. }))
            .expect("Spawn");
        assert!(removed < spawned, "{actions:?}");
    };
    {
        let tunables = preset();
        let wait = tunables.stop_wait;

        // The old process goes 5 s after the reinstall.
        let (mut w, u, pid, stop) = stuck_standby_stop(tunables.clone());
        let landed = stop + wait;
        w.tick(landed);
        w.s.stop(u, StopReason::Uninstall, Then::Removed).unwrap();
        assert!(!w.tick(landed + 1).iter().any(is_remove_data));
        let at = landed + 1_000;
        w.tick(at);
        assert_eq!(w.s.add_unit(UnitSpec::new("n", Class::Normal)), u);
        assert!(!w.s.unit(u).unwrap().removed);
        assert!(spawns(&w.tick(at)).is_empty(), "never beside the old one");
        assert!(spawns(&w.tick(at + 5_000)).is_empty());
        w.s.process_reaped(pid, Some(0));
        w.s.backend_cleaned(pid);
        let actions = w.tick(at + 5_001);
        order(&actions, u);
        assert!(active(&w, u));
        assert_eq!(count(&w.log, is_remove_data), 1);

        // It never goes: the reinstall fails 20 s after its request.
        let (mut w, u, _pid, stop) = stuck_standby_stop(tunables.clone());
        let landed = stop + wait;
        w.tick(landed);
        w.s.stop(u, StopReason::Uninstall, Then::Removed).unwrap();
        let at = landed + 1_000;
        w.tick(at);
        assert_eq!(w.s.add_unit(UnitSpec::new("n", Class::Normal)), u);
        assert!(spawns(&w.tick(at + wait - 1)).is_empty());
        assert!(spawns(&w.tick(at + wait)).is_empty());
        assert_eq!(
            w.s.list_state(u),
            Some(ListState::Failed(STUCK_RELOAD.into()))
        );
        assert_eq!(count(&w.log, is_remove_data), 0);

        // Reinstalled while the uninstall still stops the running process:
        // the uninstall's job ends, the unit lands `Inactive` and starts.
        let mut w = World::new(tunables);
        let u = standby_unit(&mut w, "n");
        let holder = w.holder(u);
        w.s.wake(holder, &Trigger::interactive(1));
        w.tick(0);
        let pid = w.pid(u);
        w.auto_reap = false;
        w.tick(100);
        let job = w.s.stop(u, StopReason::Uninstall, Then::Removed).unwrap();
        w.tick(100);
        assert!(w.state(u).is_deactivating());
        w.tick(200);
        assert_eq!(w.s.add_unit(UnitSpec::new("n", Class::Normal)), u);
        assert!(spawns(&w.tick(200)).is_empty());
        assert_eq!(w.s.job_result(u, job), Some(&JobResult::Done));
        w.s.process_reaped(pid, Some(0));
        w.s.backend_cleaned(pid);
        let actions = w.tick(300);
        order(&actions, u);
        assert!(active(&w, u));
        assert_eq!(count(&w.log, is_remove_data), 1);
    }
}

// ---- exit causes and the restart policy ------------------------------

/// An exit is `Oom` only with V8's heap line on stderr **and** the heap abort's
/// exit code; the notice waits for the reap that brings the code. Either alone
/// is a plain exit.
#[test]
fn b4_oom_needs_the_heap_line_and_the_abort_code() {
    for (heap_line, code, oom) in [
        (true, Some(exit::HEAP_ABORT_STATUS), true),
        (false, Some(exit::HEAP_ABORT_STATUS), false),
        (true, Some(1), false),
    ] {
        let mut w = World::new(preset());
        w.auto_reap = false;
        let u = w.s.add_unit(UnitSpec::new("d", Class::Development));
        w.tick(0);
        let pid = w.pid(u);
        if heap_line {
            w.s.process_out_of_memory(pid);
        }
        w.s.process_exited(pid, ExitCause::Exited, None);
        let actions = w.tick(1);
        // Nothing is sent to a process that is gone; its entries go.
        assert!(!actions.iter().any(is_terminate), "{actions:?}");
        assert!(actions.contains(&Action::Revoke { unit: u, pid }));
        assert!(
            !actions.iter().any(|a| matches!(
                a,
                Action::Notify {
                    notice: Notice::Crashed(_),
                    ..
                }
            )),
            "the notice waits for the reap: {actions:?}"
        );
        w.s.process_reaped(pid, code);
        w.s.backend_cleaned(pid);
        let actions = w.tick(2);
        let cause = if oom {
            ExitCause::Oom
        } else {
            ExitCause::Exited
        };
        assert!(
            actions.contains(&Action::Notify {
                unit: Some(u),
                notice: Notice::Crashed(cause)
            }),
            "{actions:?}"
        );
        assert_eq!(
            w.s.list_state(u),
            Some(ListState::Failed(cause.text().to_owned()))
        );
    }
}

/// (coordinator ruling 2026-11-03): an own process's transport failure fails it
/// for good, every class; a fault restarts a built-in.
#[test]
fn b4_own_transport_fails_and_a_fault_restarts() {
    let mut w = World::new(preset());
    let b = w.s.add_unit(UnitSpec::new("b", Class::Builtin));
    w.tick(0);
    let pid = w.pid(b);
    w.exited(pid, ExitCause::Transport, None);
    w.tick(1);
    assert!(matches!(w.state(b), UnitState::Failed { .. }));
    let mut w = World::new(preset());
    let b = w.s.add_unit(UnitSpec::new("b", Class::Builtin));
    w.tick(0);
    let pid = w.pid(b);
    w.exited(pid, ExitCause::Fault, None);
    w.tick(1);
    assert!(spawns(&w.tick(3_001)).len() == 1, "{:?}", w.state(b));
}

/// The Host may run a unit the core placed in the shared runner in its own
/// process (its trust check): a runner failure does not reach it.
#[test]
fn b4_a_unit_placed_own_is_not_in_the_runner_group() {
    let mut w = World::new(preset());
    let shared =
        w.s.add_unit(UnitSpec::new("vestra.a", Class::Builtin).shared());
    let own =
        w.s.add_unit(UnitSpec::new("vestra.b", Class::Builtin).shared());
    w.auto_spawn = false;
    w.tick(0);
    let (a, b) = (w.pid(shared), w.pid(own));
    w.s.process_spawned(a, Ok(()));
    w.s.placed_own(b);
    w.s.process_spawned(b, Ok(()));
    w.s.process_ready(a);
    w.s.process_ready(b);
    w.tick(1);
    let runner = w.s.runner();
    w.s.runner_failed(runner, ExitCause::Transport);
    let actions = w.tick(2);
    assert!(
        actions
            .iter()
            .any(|x| matches!(x, Action::Terminate { pid, .. } if *pid == a))
    );
    assert!(
        !actions
            .iter()
            .any(|x| matches!(x, Action::Terminate { pid, .. } if *pid == b))
    );
    assert!(active(&w, own));
}

// ---- Process budget and memory pressure -----------------------

/// The player's preset (`host/supervisor/mod.rs` `new`) for this machine,
/// with an idle time long enough that only the budget or pressure puts a
/// unit in standby.
fn b5(cores: usize, physical_mib: u64) -> Tunables {
    Tunables {
        idle_timeout: 3_600_000,
        ..Tunables::new(cores, physical_mib)
    }
}

/// `n` units in standby, each woken by a click 1 s apart and ready
/// (the first is the least recently used).
fn clicked(w: &mut World, ids: &[&str], from: Ms) -> Vec<UnitId> {
    let units = ids.iter().map(|id| standby_unit(w, id)).collect::<Vec<_>>();
    w.tick(from);
    for (i, u) in units.iter().enumerate() {
        let at = from + 1_000 * (i as Ms + 1);
        let holder = w.holder(*u);
        w.s.set_now(at);
        w.s.wake(holder, &Trigger::interactive(i as u32));
        w.tick(at);
        assert!(active(w, *u), "{}", ids[i]);
    }
    units
}

fn asked(actions: &[Action<u32>]) -> Vec<UnitId> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::QueryResources { unit, .. } => Some(*unit),
            _ => None,
        })
        .collect()
}

#[test]
fn b5_budget_formula_per_machine() {
    assert_eq!(b5(8, 2_048).process_budget, 3);
    assert_eq!(b5(8, 4_096).process_budget, 7);
    assert_eq!(b5(8, 8_192).process_budget, 14);
    assert_eq!(b5(8, 28_381).process_budget, 16);
    assert_eq!(Tunables::new(8, 8_192).process_budget, 14);
    // The memory changes nothing else of the preset.
    assert_eq!(
        Tunables {
            process_budget: 16,
            ..b5(8, 2_048)
        },
        b5(8, 16_384)
    );
}

/// Clicks on one plugin after another (`Interactive` starts): after each
/// start the third-party processes are back at the budget, and the one
/// put in standby is always the least recently used.
#[test]
fn b5_budget_holds_under_clicks_in_lru_order() {
    let mut w = World::new(b5(8, 2_048));
    let ids = (0..12).map(|i| format!("p{i:02}")).collect::<Vec<_>>();
    let ids = ids.iter().map(String::as_str).collect::<Vec<_>>();
    let units = ids
        .iter()
        .map(|id| standby_unit(&mut w, id))
        .collect::<Vec<_>>();
    w.tick(0);
    let mut order = Vec::new();
    for (i, u) in units.iter().enumerate() {
        let at = 120_000 * (i as Ms + 1);
        let holder = w.holder(*u);
        w.s.set_now(at);
        w.s.wake(holder, &Trigger::interactive(i as u32));
        let actions = w.tick(at);
        order.extend(asked(&actions));
        assert!(active(&w, *u));
        assert!(w.s.budget_count() <= 3, "after {}", ids[i]);
    }
    // p00 went when p03 started, p01 at p04, ...: oldest first.
    assert_eq!(order, units[..9]);
    for u in &units[9..] {
        assert!(active(&w, *u));
    }
}

/// Starts in a row while the first standby is still asked for its
/// resources put only as many units in standby as the budget needs: a
/// unit whose Standby job is installed is not counted again.
#[test]
fn b5_starts_in_a_row_reclaim_only_the_overrun() {
    let mut w = World::new(b5(8, 2_048));
    let v = clicked(&mut w, &["a", "b", "c"], 0);
    let (d, e) = (standby_unit(&mut w, "d"), standby_unit(&mut w, "e"));
    w.tick(10_000);
    w.auto_resources = false;
    for (i, u) in [d, e].into_iter().enumerate() {
        let holder = w.holder(u);
        w.s.set_now(10_000);
        w.s.wake(holder, &Trigger::interactive(10 + i as u32));
    }
    let actions = w.tick(10_000);
    assert_eq!(asked(&actions), [v[0], v[1]], "two over, two asked");
    for u in [v[0], v[1]] {
        assert_eq!(w.s.job(u).map(|job| job.origin), Some(Origin::Budget));
    }
    for pid in [w.pid(v[0]), w.pid(v[1])] {
        w.s.resources_answered(pid, false);
    }
    w.tick(10_001);
    assert!(matches!(w.state(v[0]), UnitState::Standby { .. }));
    assert!(matches!(w.state(v[1]), UnitState::Standby { .. }));
    for u in [v[2], d, e] {
        assert!(active(&w, u));
    }
    assert_eq!(w.s.budget_count(), 3);
}

/// Memory pressure (`memory_pressure`, called on a level change): Low puts
/// every idle reclaimable unit in standby, least important first; `None` puts
/// nothing; Critical also takes the Interactive overrun away (max + 1 instead
/// of max + 2).
#[test]
fn b5_pressure_levels() {
    let mut w = World::new(b5(4, 28_381));
    let v = clicked(&mut w, &["a", "b", "c"], 0);
    let pid = w.pid(v[1]);
    w.s.begin_work(pid, WorkKind::Declared(1), 4_000);
    w.s.set_now(70_000);
    w.s.memory_pressure(Pressure::None);
    assert!(asked(&w.tick(70_000)).is_empty());
    w.s.set_now(70_000);
    w.s.memory_pressure(Pressure::Low);
    assert_eq!(w.s.pressure(), Pressure::Low);
    assert_eq!(asked(&w.tick(70_000)), [v[0], v[2]], "the busy one stays");
    assert!(active(&w, v[1]));
    // Critical: max_active 2 at 4 cores; clicks may take 3, not 4.
    w.s.memory_pressure(Pressure::Critical);
    w.auto_ready = false;
    let more = (0..5)
        .map(|i| standby_unit(&mut w, &format!("x{i}")))
        .collect::<Vec<_>>();
    w.tick(80_000);
    for (i, u) in more.iter().enumerate() {
        let holder = w.holder(*u);
        w.s.set_now(80_000);
        w.s.wake(holder, &Trigger::interactive(100 + i as u32));
    }
    assert_eq!(spawns(&w.tick(80_000)).len(), 3);
    assert_eq!(w.s.slots_in_use(), 3);
    // Back to Low: the overrun is there again.
    w.s.memory_pressure(Pressure::Low);
    assert_eq!(spawns(&w.tick(80_001)).len(), 1);
}

/// Built-ins, Residents and development loads are outside pressure
/// standby; the budget counts Residents but never reclaims them.
#[test]
fn b5_pressure_and_budget_never_take_builtins_or_residents() {
    let mut w = World::new(b5(8, 2_048));
    let builtin =
        w.s.add_unit(UnitSpec::new("vestra.b", Class::Builtin).checkpoint(false));
    let resident =
        w.s.add_unit(UnitSpec::new("r", Class::Normal).checkpoint(true));
    w.tick(0);
    let holder = w.holder(builtin);
    w.s.wake(holder, &Trigger::interactive(0));
    w.tick(1);
    let v = clicked(&mut w, &["a", "b"], 100);
    assert_eq!(
        w.s.budget_count(),
        3,
        "the resident counts, the built-in not"
    );
    w.s.set_now(200_000);
    w.s.memory_pressure(Pressure::Critical);
    let actions = w.tick(200_000);
    assert_eq!(asked(&actions), v);
    assert!(active(&w, builtin));
    assert!(active(&w, resident));
}
