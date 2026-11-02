//! Deterministic simulations of the core (the maintainer's own; the spec
//! tests from the design tables live in `tests/`). A small fake adapter
//! answers actions the way the Host does: spawn → spawned (+ ready),
//! terminate → reaped + backend cleaned, resources → not busy.
#![forbid(unsafe_code)]

use super::*;

type S = Supervisor<u32>;

struct World {
    s: S,
    now: Ms,
    auto_spawn: bool,
    auto_ready: bool,
    auto_reap: bool,
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
            auto_classify: None,
            log: Vec::new(),
            delivered: Vec::new(),
            answered: Vec::new(),
        }
    }
    fn off_demand() -> Tunables {
        Tunables {
            on_demand: false,
            ..Tunables::today()
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
            Action::QueryResources { pid, .. } => self.s.resources_answered(pid, false),
            Action::Deliver { pid, payload, .. } => self.delivered.push((pid, payload)),
            Action::Answer {
                reason, payload, ..
            } => self.answered.push((reason, payload)),
            _ => {}
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
fn boot_builtins_first_and_eight_first_runs() {
    let mut w = World::new(Tunables::today());
    w.auto_ready = false;
    let third = (0..12)
        .map(|i| w.s.add_unit(UnitSpec::new(format!("t{i}"), Class::Normal)))
        .collect::<Vec<_>>();
    let builtins = (0..3)
        .map(|i| w.s.add_unit(UnitSpec::new(format!("b{i}"), Class::Builtin)))
        .collect::<Vec<_>>();
    let started = spawns(&w.tick(0));
    assert_eq!(started.len(), 8, "FIRST_RUNS = 8 counts built-ins too");
    let order = started.iter().map(|(u, ..)| *u).collect::<Vec<_>>();
    assert_eq!(&order[..3], &builtins[..], "built-ins first");
    assert_eq!(&order[3..], &third[..5]);
    assert_eq!(w.s.slots_in_use(), (0, 8));
    // A first run frees its place once classified lazy and its process is
    // gone (as `lazy_first_runs_tick`).
    let (u, pid, _) = started[3];
    w.s.process_spawned(pid, Ok(()));
    w.s.process_ready(pid);
    assert!(spawns(&w.tick(10)).is_empty(), "still classifying");
    w.s.classified(pid, None);
    let actions = w.tick(20);
    assert!(actions.contains(&Action::Checkpoint {
        unit: u,
        pid,
        resident: false
    }));
    assert!(actions.contains(&Action::Terminate {
        unit: u,
        pid,
        reason: StopReason::Idle,
        grace: 2_000,
        keep_registrations: true,
    }));
    let next = spawns(&actions);
    assert_eq!(
        next.len(),
        1,
        "process gone at once in the fake: next first run"
    );
    assert_eq!(next[0].0, third[5]);
    assert!(matches!(w.state(u), UnitState::Standby { .. }));
    // A resident first run frees its place at classification.
    let (b, bpid, _) = started[0];
    w.s.process_spawned(bpid, Ok(()));
    w.s.process_ready(bpid);
    w.s.classified(bpid, Some("a panel".into()));
    assert_eq!(spawns(&w.tick(30)).len(), 1);
    assert!(active(&w, b));
    assert!(w.s.unit(b).unwrap().resident);
}

#[test]
fn verifying_builtin_holds_back_third_party_first_runs() {
    let mut w = World::new(Tunables::today());
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
    let mut w = World::new(World::off_demand());
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
    let mut w = World::new(Tunables::today());
    let u = standby_unit(&mut w, "n");
    let holder = w.holder(u);
    assert_eq!(w.tick(0), [Action::Replay { unit: u, holder }]);
    assert_eq!(w.s.wake(holder, Trigger::interactive(7)), Delivery::Started);
    assert_eq!(w.s.wake(holder, Trigger::follow(8)), Delivery::Held);
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
        w.s.wake(holder, Trigger::interactive(9)),
        Delivery::Forward(pid)
    );
    assert_eq!(w.s.wake(pid, Trigger::interactive(9)), Delivery::Now);
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
    let mut w = World::new(World::off_demand());
    let mut t = World::off_demand();
    t.on_demand = true;
    w.s = Supervisor::new(t, 1);
    let u = standby_unit(&mut w, "n");
    let holder = w.holder(u);
    w.s.wake(holder, Trigger::interactive(1));
    w.tick(0);
    let pid = w.pid(u);
    w.s.begin_work(pid, 5, 100);
    assert!(!w.tick(70_000).iter().any(is_terminate), "work in flight");
    w.s.end_work(pid, 5);
    let actions = w.tick(140_000);
    assert!(actions.iter().any(is_terminate));
}

#[test]
fn wake_while_deactivating_waits_for_reap() {
    let mut w = World::new(Tunables::today());
    w.auto_reap = false;
    let u = standby_unit(&mut w, "n");
    let holder = w.holder(u);
    w.s.wake(holder, Trigger::interactive(1));
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
    assert_eq!(w.s.wake(pid, Trigger::interactive(2)), Delivery::Started);
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
    let mut w = World::new(Tunables::today());
    w.auto_reap = false;
    let u = standby_unit(&mut w, "n");
    let holder = w.holder(u);
    w.s.wake(holder, Trigger::interactive(1));
    w.tick(0);
    let pid = w.pid(u);
    w.tick(60_000);
    w.s.wake(pid, Trigger::interactive(2));
    w.tick(60_001);
    // Kill at the standby grace.
    assert!(w.tick(62_000).contains(&Action::Kill { unit: u, pid }));
    assert!(w.tick(79_999).is_empty());
    let actions = w.tick(80_000);
    assert!(actions.contains(&Action::Revoke { unit: u, pid }));
    assert_eq!(w.answered, [(AnswerReason::CouldNotStart, 2)]);
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
            w.s.wake(holder, Trigger::interactive(i as u32)),
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
fn storm_of_200_wakes_today_starts_all_at_once() {
    let (w, units, max) = storm(Tunables::today());
    assert_eq!(max, 200, "today: wakes are immediate and unbounded (D4)");
    assert!(units.iter().all(|u| active(&w, *u)));
    let mut payloads = w.delivered.iter().map(|(_, p)| *p).collect::<Vec<_>>();
    payloads.sort_unstable();
    assert_eq!(payloads, (0..200).collect::<Vec<_>>());
}

#[test]
fn storm_of_200_wakes_with_b3_limits_is_bounded_and_complete() {
    let (w, units, max) = storm(Tunables::today().with_b3(8));
    // max_active 4 + 1 interactive reserve + 1 overrun.
    assert_eq!(max, 6);
    assert!(units.iter().all(|u| active(&w, *u)));
    assert_eq!(
        w.delivered.len(),
        200,
        "each trigger delivered exactly once"
    );
    assert_eq!(w.s.slots_in_use(), (0, 0));
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
    let mut w = World::new(Tunables::today().with_b3(8));
    w.auto_ready = false;
    let units = (0..10)
        .map(|i| standby_unit(&mut w, &format!("n{i}")))
        .collect::<Vec<_>>();
    w.tick(0);
    w.s.set_muted(true);
    let h0 = w.holder(units[0]);
    assert_eq!(w.s.wake(h0, Trigger::event("e", 0)), Delivery::Dropped);
    w.s.settled(100);
    assert_eq!(w.s.wake(h0, Trigger::event("e", 0)), Delivery::Dropped);
    w.tick(1_100);
    for u in &units {
        let holder = w.holder(*u);
        assert_eq!(w.s.wake(holder, Trigger::event("e", 1)), Delivery::Started);
    }
    assert_eq!(spawns(&w.tick(1_200)).len(), 4);
}

// ---- disable, enable, reload ---------------------------------------------

#[test]
fn disable_then_enable_waits_for_reap_d13() {
    let mut w = World::new(World::off_demand());
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
    let mut w = World::new(World::off_demand());
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
    let mut w = World::new(Tunables::today());
    w.auto_ready = false;
    let u = standby_unit(&mut w, "n");
    let holder = w.holder(u);
    w.s.wake(holder, Trigger::interactive(1));
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
    assert_eq!(w.delivered, [(second, 1)]);
    assert_eq!(w.s.job_result(u, reload), Some(&JobResult::Done));
}

#[test]
fn uninstall_from_standby_removes_data_and_refuses_triggers() {
    let mut w = World::new(Tunables::today());
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
    assert_eq!(w.s.wake(holder, Trigger::interactive(1)), Delivery::Dropped);
    w.s.remove_unit(u);
    assert_eq!(w.s.by_id("n"), None);
    assert_eq!(w.s.by_pid(holder), None);
}

// ---- failures -----------------------------------------------------------

#[test]
fn start_timeout_fails_without_retry_today() {
    let mut w = World::new(World::off_demand());
    w.auto_ready = false;
    w.auto_reap = false;
    let u = w.s.add_unit(UnitSpec::new("n", Class::Normal));
    w.tick(0);
    let pid = w.pid(u);
    assert!(w.tick(14_999).is_empty());
    let actions = w.tick(15_000);
    assert!(actions.contains(&Action::Terminate {
        unit: u,
        pid,
        reason: StopReason::Failed,
        grace: 300,
        keep_registrations: false
    }));
    assert!(actions.iter().any(|a| matches!(
        a,
        Action::JobFinished { result: JobResult::Failed(t), .. } if t == "activation timed out"
    )));
    assert!(w.tick(15_300).contains(&Action::Kill { unit: u, pid }));
    w.s.process_reaped(pid, None);
    w.s.backend_cleaned(pid);
    w.tick(15_400);
    assert_eq!(
        w.s.list_state(u),
        Some(ListState::Failed("activation timed out".into()))
    );
}

#[test]
fn spawn_failure_revokes_today_and_keeps_entry_points_from_b3() {
    for b3 in [false, true] {
        let tunables = if b3 {
            Tunables::today().with_b3(8)
        } else {
            Tunables::today()
        };
        let mut w = World::new(tunables);
        w.auto_spawn = false;
        w.auto_ready = false;
        let u = standby_unit(&mut w, "n");
        let holder = w.holder(u);
        w.s.wake(holder, Trigger::interactive(5));
        w.tick(0);
        let pid = w.pid(u);
        w.s.process_spawned(pid, Err("activation tracking limit reached".into()));
        let actions = w.tick(1);
        assert_eq!(w.answered, [(AnswerReason::CouldNotStart, 5)]);
        assert!(actions.contains(&Action::Notify {
            unit: Some(u),
            notice: Notice::CouldNotStart
        }));
        let revoked = actions.contains(&Action::Revoke { unit: u, pid });
        assert_eq!(revoked, !b3);
        if b3 {
            assert_eq!(w.holder(u), pid);
            assert_eq!(w.s.wake(pid, Trigger::interactive(6)), Delivery::Started);
        } else {
            assert!(matches!(w.state(u), UnitState::Failed { .. }));
            assert_eq!(w.s.wake(pid, Trigger::interactive(6)), Delivery::Dropped);
        }
    }
}

#[test]
fn crash_today_fails_every_class() {
    for class in [Class::Builtin, Class::Normal, Class::Development] {
        let mut w = World::new(World::off_demand());
        let u = w.s.add_unit(UnitSpec::new("x", class).checkpoint(false));
        w.tick(0);
        let pid = w.pid(u);
        w.s.process_exited(pid, ExitCause::Exited);
        let actions = w.tick(10);
        assert!(actions.contains(&Action::Terminate {
            unit: u,
            pid,
            reason: StopReason::Failed,
            grace: 300,
            keep_registrations: false
        }));
        assert!(spawns(&actions).is_empty(), "Restart=no until B4");
        assert_eq!(
            w.s.list_state(u),
            Some(ListState::Failed("plugin process exited".into()))
        );
        // Reload clears it (T28).
        w.s.restart(u, true, None).unwrap();
        assert_eq!(spawns(&w.tick(20)).len(), 1);
    }
}

#[test]
fn b4_normal_crash_holds_in_standby_and_builtin_restarts() {
    let mut w = World::new(Tunables::today().with_b4());
    let n = standby_unit(&mut w, "n");
    let b = w.s.add_unit(UnitSpec::new("b", Class::Builtin));
    w.auto_classify = Some(true);
    w.tick(0);
    let holder = w.holder(n);
    w.s.wake(holder, Trigger::interactive(1));
    w.tick(10);
    let npid = w.pid(n);
    w.s.set_now(20);
    w.s.process_exited(npid, ExitCause::Fault);
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
    assert_eq!(w.s.wake(npid, Trigger::interactive(2)), Delivery::Refused);
    w.tick(30);
    assert_eq!(w.answered, [(AnswerReason::Held { until: 1_020 }, 2)]);
    assert_eq!(w.s.wake(npid, Trigger::event("e", 3)), Delivery::Started);
    assert!(spawns(&w.tick(1_019)).is_empty());
    assert_eq!(spawns(&w.tick(1_020)).len(), 1);
    // Built-in: Restart=on-failure after 3 s.
    let bpid = w.pid(b);
    w.s.set_now(2_000);
    w.s.process_exited(bpid, ExitCause::Oom);
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
    let mut w = World::new(Tunables::today().with_b4());
    let b = w.s.add_unit(UnitSpec::new("b", Class::Builtin));
    w.auto_classify = Some(true);
    let mut now = 0;
    w.tick(now);
    for _ in 0..4 {
        let pid = w.pid(b);
        w.s.process_exited(pid, ExitCause::Exited);
        // Reaped at once; restarted after the backoff.
        w.tick(now);
        assert!(!active(&w, b));
        now += 100_000;
        w.tick(now);
        assert!(active(&w, b), "{:?} {:?}", w.state(b), w.s.job(b));
    }
    let pid = w.pid(b);
    w.s.process_exited(pid, ExitCause::Exited);
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
    let mut w = World::new(Tunables::today().with_b4());
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
    let mut w = World::new(World::off_demand());
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
        assert!(spawns(&actions).is_empty(), "waits for member cleanup");
        assert!(active(&w, own), "own-process built-in untouched");
        for pid in pids {
            w.s.backend_cleaned(pid);
        }
        now += 10;
        let actions = w.tick(now);
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
    let mut w = World::new(World::off_demand());
    let a = w.s.add_unit(UnitSpec::new("a", Class::Builtin).shared());
    let b = w.s.add_unit(UnitSpec::new("b", Class::Builtin).shared());
    w.tick(0);
    let runner = w.s.runner();
    let apid = w.pid(a);
    w.s.process_exited(apid, ExitCause::Transport);
    let actions = w.tick(10);
    assert!(actions.contains(&Action::KillRunner { runner }));
    assert_eq!(count(&actions, is_terminate), 2);
    // Attributed fault: that member alone, its own process next.
    let bpid = w.pid(b);
    w.s.process_exited(bpid, ExitCause::Fault);
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
    let mut tunables = Tunables::today().with_b3(8);
    tunables.backlog_max = Some(3);
    let mut w = World::new(tunables);
    w.auto_ready = false;
    let u = standby_unit(&mut w, "n");
    let holder = w.holder(u);
    w.tick(0);
    assert_eq!(w.s.wake(holder, Trigger::interactive(1)), Delivery::Started);
    assert_eq!(w.s.wake(holder, Trigger::event("e", 2)), Delivery::Held);
    assert_eq!(w.s.wake(holder, Trigger::event("e", 3)), Delivery::Held);
    assert_eq!(w.s.wake(holder, Trigger::interactive(4)), Delivery::Held);
    assert_eq!(w.s.wake(holder, Trigger::interactive(5)), Delivery::Refused);
    assert_eq!(w.s.wake(holder, Trigger::event("f", 6)), Delivery::Refused);
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
    let mut tunables = Tunables::today().with_b3(8);
    tunables.lane_max = Some(2);
    tunables.max_active = Some(0);
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
        deliveries.push(w.s.wake(holder, Trigger::interactive(i as u32)));
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
    let mut tunables = Tunables::today();
    tunables.idle_timeout = 3_600_000;
    tunables.process_budget = Some(4);
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
        w.s.wake(holder, Trigger::interactive(0));
        w.tick(at);
    }
    let pid = w.pid(busy);
    w.s.begin_work(pid, 1, 60_000);
    assert_eq!(w.s.budget_count(), 4);
    (w, [builtin, resident, busy, old, recent, new])
}

#[test]
fn lru_picks_the_least_important_idle_unit_only() {
    let (mut w, [builtin, resident, busy, old, recent, new]) = budget_world();
    let holder = w.holder(new);
    w.s.set_now(100_000);
    w.s.wake(holder, Trigger::interactive(1));
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
        w.s.begin_work(pid, 2, 90_000);
    }
    let holder = w.holder(new);
    w.s.wake(holder, Trigger::interactive(1));
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
    w.s.wake(holder, Trigger::event("e", 1));
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
    let mut w = World::new(Tunables::today());
    let u = standby_unit(&mut w, "n");
    let holder = w.holder(u);
    w.tick(0);
    assert_eq!(w.s.wake(holder, Trigger::event("e", 1)), Delivery::Started);
    let job = w.s.job(u).unwrap().id;
    assert_eq!(w.s.wake(holder, Trigger::interactive(2)), Delivery::Held);
    let installed = w.s.job(u).unwrap();
    assert_eq!(installed.id, job);
    assert_eq!(installed.priority, Priority::Interactive);
}

#[test]
fn merge_standby_asking_then_trigger_cancels_standby() {
    let mut w = World::new(World::off_demand());
    let u = w.s.add_unit(UnitSpec::new("n", Class::Normal));
    w.tick(0);
    let pid = w.pid(u);
    let standby = w.s.standby(u);
    let actions = w.s.run_jobs(10);
    assert!(actions.contains(&Action::QueryResources { unit: u, pid }));
    assert_eq!(w.s.wake(pid, Trigger::interactive(1)), Delivery::Now);
    assert_eq!(w.s.job_result(u, standby), Some(&JobResult::Canceled));
    // The late answer is ignored.
    w.s.resources_answered(pid, false);
    assert!(!w.tick(20).iter().any(is_terminate));
}

#[test]
fn merge_restart_then_stop_stop_wins() {
    let mut w = World::new(World::off_demand());
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
    let mut w = World::new(Tunables::today());
    let u = standby_unit(&mut w, "n");
    let holder = w.holder(u);
    w.tick(0);
    w.s.set_enabled(u, false);
    w.s.stop(u, StopReason::Disable, Then::Inactive).unwrap();
    assert_eq!(w.s.wake(holder, Trigger::interactive(1)), Delivery::Dropped);
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
    let mut w = World::new(Tunables::today());
    w.auto_reap = false;
    let u = standby_unit(&mut w, "n");
    let holder = w.holder(u);
    w.s.wake(holder, Trigger::interactive(1));
    w.tick(0);
    let pid = w.pid(u);
    w.tick(60_000);
    // Deactivating for standby, registrations kept; an uninstall arrives.
    w.s.stop(u, StopReason::Uninstall, Then::Removed).unwrap();
    w.tick(60_001);
    w.s.process_reaped(pid, None);
    w.s.backend_cleaned(pid);
    let actions = w.tick(60_002);
    assert_eq!(
        actions[..3],
        [
            Action::Revoke { unit: u, pid },
            Action::DropCheckpoint { unit: u },
            Action::RemoveData { unit: u },
        ]
    );
}

// ---- shutdown, timers -------------------------------------------------------

#[test]
fn shutdown_one_deadline_then_kill() {
    let mut w = World::new(World::off_demand());
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
        w.s.wake(pids[1], Trigger::interactive(1)),
        Delivery::Dropped
    );
}

#[test]
fn timers_cost_nothing_until_due() {
    let mut w = World::new(World::off_demand());
    let mut tunables = World::off_demand();
    tunables.on_demand = true;
    w.s = Supervisor::new(tunables, 3);
    let units = (0..1_000)
        .map(|i| standby_unit(&mut w, &format!("n{i}")))
        .collect::<Vec<_>>();
    w.tick(0);
    for u in &units {
        let holder = w.holder(*u);
        w.s.wake(holder, Trigger::interactive(0));
    }
    w.tick(5);
    assert_eq!(w.s.next_deadline(), Some(60_005));
    assert!(w.s.run_jobs(60_004).is_empty());
    let actions = w.tick(60_005);
    assert_eq!(count(&actions, is_terminate), 1_000);
}

#[test]
fn list_changed_ignores_process_and_reports_state() {
    let mut w = World::new(Tunables::today());
    let u = standby_unit(&mut w, "n");
    w.tick(0);
    let holder = w.holder(u);
    w.s.wake(holder, Trigger::interactive(1));
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
    let mut w2 = World::new(Tunables::today());
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
        let mut tunables = Tunables::today().with_b3(8).with_b4();
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
        w.s.wake(holder, Trigger::interactive(1));
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

// ---- answers to the spec author's design questions (§17.2) -----------------

#[test]
fn q15_upgrade_of_a_disabled_unit_applies_without_starting() {
    let mut w = World::new(Tunables::today());
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
    // A disabled unit with a job installed still refuses (§4.2).
    let v = w.s.add_unit(UnitSpec::new("v", Class::Normal).disabled());
    w.s.stop(v, StopReason::Disable, Then::Inactive).unwrap();
    let refused = w.s.restart(v, true, None).unwrap();
    assert_eq!(w.s.job_result(v, refused), Some(&JobResult::Dropped));
}

#[test]
fn q24_backoff_and_hold_count_from_the_reap() {
    let mut w = World::new(Tunables::today().with_b4());
    w.auto_reap = false;
    let n = standby_unit(&mut w, "n");
    let b =
        w.s.add_unit(UnitSpec::new("b", Class::Builtin).checkpoint(true));
    w.tick(0);
    let holder = w.holder(n);
    w.s.wake(holder, Trigger::interactive(1));
    w.tick(10);
    let (npid, bpid) = (w.pid(n), w.pid(b));
    w.s.process_exited(npid, ExitCause::Exited);
    w.s.process_exited(bpid, ExitCause::Exited);
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

// ---- coordinator rulings after the spec tests (§17.2) -----------------------

#[test]
fn builtins_first_today_vs_design_row() {
    for mode in [BuiltinsFirst::Queued, BuiltinsFirst::Installed] {
        let tunables = Tunables {
            builtins_first: mode,
            ..Tunables::today()
        };
        let mut w = World::new(tunables);
        w.auto_ready = false;
        let n = w.s.add_unit(UnitSpec::new("n", Class::Normal));
        let b = w.s.add_unit(UnitSpec::new("b", Class::Builtin));
        let r =
            w.s.add_unit(UnitSpec::new("r", Class::Normal).checkpoint(true));
        let order = spawns(&w.tick(0))
            .into_iter()
            .map(|(u, ..)| u)
            .collect::<Vec<_>>();
        match mode {
            // Today: built-ins start first in the tick; a started built-in
            // holds nobody back.
            BuiltinsFirst::Queued => assert_eq!(order, [b, r, n]),
            // Design: third-party background starts wait until it is active.
            BuiltinsFirst::Installed => {
                assert_eq!(order, [b]);
                let pid = w.pid(b);
                w.s.process_ready(pid);
                let next = spawns(&w.tick(10))
                    .into_iter()
                    .map(|(u, ..)| u)
                    .collect::<Vec<_>>();
                assert_eq!(next, [r, n]);
            }
        }
    }
}

#[test]
fn runner_restart_replays_members_with_a_standby_checkpoint() {
    let mut w = World::new(Tunables::today());
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
    w.s.wake(holder, Trigger::interactive(1));
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
    // The old pid still finds the unit and wakes it.
    assert_eq!(w.s.wake(mpid, Trigger::interactive(2)), Delivery::Started);
    assert!(matches!(w.state(a), UnitState::Standby { .. }));
}

#[test]
fn crash_during_wake_keeps_a_start_only_for_held_events() {
    for with_event in [false, true] {
        let mut w = World::new(Tunables::today().with_b4());
        w.auto_ready = false;
        let u = standby_unit(&mut w, "n");
        let holder = w.holder(u);
        w.tick(0);
        w.s.wake(holder, Trigger::interactive(1));
        if with_event {
            w.s.wake(holder, Trigger::event("e", 2));
        }
        w.tick(10);
        let pid = w.pid(u);
        w.s.process_spawned(pid, Ok(()));
        w.s.process_exited(pid, ExitCause::Fault);
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

#[test]
fn resident_start_timeout_retries_and_never_lands_in_standby() {
    let mut w = World::new(Tunables::today().with_b4());
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
    assert!((15_250..=15_275).contains(&next), "{next}");
    assert_eq!(spawns(&w.tick(next)).len(), 1);
}
