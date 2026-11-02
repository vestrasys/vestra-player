//! Running jobs, design §4.4: lanes, priorities, concurrency, Interactive
//! reserve, bounds. A Start job occupies a slot from `run_jobs` until
//! `process_ready` (T9 completes it).
use super::*;

/// `n` third-party units started by event wakes and left in
/// `Activating{Ready}` (each holds a running Start job).
fn fill_slots(sim: &mut Sim, n: usize) -> Vec<(UnitId, Pid)> {
    let mut held = Vec::new();
    for i in 0..n {
        held.push(sim.in_standby(normal(&format!("w{i}"))));
    }
    sim.settle();
    for (_, holder) in &held {
        sim.wake(*holder, event("playback.state", 0));
    }
    let acts = sim.run();
    assert_eq!(spawns(&acts).len(), n, "filling {n} slots");
    held.iter()
        .map(|(u, _)| {
            let pid = sim.pid(*u);
            sim.spawned(pid, true);
            (*u, pid)
        })
        .collect()
}

/// A unit in standby, woken (not run yet) with `t`.
fn woken(sim: &mut Sim, id: &str, t: Trigger<u32>) -> UnitId {
    let (u, holder) = sim.in_standby(normal(id));
    sim.wake(holder, t);
    u
}

/// Units in standby, all woken by a command before any `run_jobs` (adding a
/// unit runs jobs, so `woken` one by one would start the earlier ones).
fn woken_together<const N: usize>(sim: &mut Sim, ids: [&str; N]) -> [UnitId; N] {
    let held = ids.map(|id| sim.in_standby(normal(id)));
    for (i, (_, h)) in held.iter().enumerate() {
        sim.wake(*h, cmd(i as u32));
    }
    held.map(|(u, _)| u)
}

fn spawned_units(sim: &Sim, acts: &[Act]) -> Vec<UnitId> {
    spawns(acts)
        .into_iter()
        .map(|p| sim.unit_of(p).expect("spawned pid is indexed"))
        .collect()
}

// Lanes: Builtin lane (FIFO in add order) before the third-party lane.
// RULED (coordinator 2026-11-02, `Tunables::builtins_first`): today's
// tunables reproduce today — lane order within a tick, so built-in boot
// starts and then third-party first runs spawn in the same tick; target
// tunables follow the design row — third-party Background starts wait while
// any built-in has a start job installed (until active) or is verifying.
fn builtins_and_first_runs(sim: &mut Sim) -> [UnitId; 4] {
    let n1 = sim.add(first_run("n1"));
    let b1 = sim.add(builtin("b1"));
    let n2 = sim.add(first_run("n2"));
    let b2 = sim.add(builtin("b2"));
    [n1, b1, n2, b2]
}

#[test]
fn l_builtin_lane_runs_first_in_add_order_b1() {
    let mut sim = Sim::new(&Profile::b1());
    let [n1, b1, n2, b2] = builtins_and_first_runs(&mut sim);
    let acts = sim.run();
    assert_eq!(
        spawned_units(&sim, &acts),
        vec![b1, b2, n1, n2],
        "today: built-ins first in the tick, then third-party first runs"
    );
}

#[test]
fn l_builtin_lane_runs_first_in_add_order_target() {
    let mut sim = Sim::new(&Profile::target());
    let [n1, b1, n2, b2] = builtins_and_first_runs(&mut sim);
    let acts = sim.run();
    assert_eq!(
        spawned_units(&sim, &acts),
        vec![b1, b2],
        "built-ins first; third-party first runs wait"
    );
    assert!(matches!(sim.job(n1), Some(J::Start { running: false, .. })));
    sim.bring_up(b1);
    assert!(
        matches!(sim.job(n1), Some(J::Start { running: false, .. })),
        "still a Builtin-lane job installed (b2)"
    );
    sim.bring_up(b2);
    sim.run();
    let order = spawned_units(&sim, &sim.log.clone());
    assert_eq!(
        order,
        vec![b1, b2, n1, n2],
        "FIFO within the third-party lane"
    );
}

// Lanes: the Builtin lane never counts toward the start-job limit (A7)
both! {
    fn l_builtin_lane_outside_max_active(p) {
        let mut sim = Sim::new(p);
        for i in 0..(p.max_active + 6) {
            sim.add(builtin(&format!("b{i}")));
        }
        let acts = sim.run();
        assert_eq!(spawns(&acts).len(), p.max_active + 6);
    }
}

// Built-ins first: Background waits while a built-in is Activating{Verify}
#[test]
fn l_verifying_builtin_holds_background_first_runs() {
    let mut sim = Sim::new(&Profile::b1());
    let b = sim.add(builtin("b").shared(1).verify());
    let n = sim.add(first_run("n"));
    let acts = sim.run();
    assert!(spawns(&acts).is_empty());
    assert_eq!(sim.st(b), S::Verify);
    assert!(matches!(sim.job(n), Some(J::Start { running: false, .. })));
    sim.bring_up(b);
    sim.run();
    assert!(matches!(sim.st(n), S::Spawn(_)), "{:?}", sim.st(n));
}

// Built-ins first applies to Background only: Interactive starts go
#[test]
fn l_interactive_start_not_held_by_builtins() {
    let mut sim = Sim::new(&Profile::target());
    let (u, holder) = sim.in_standby(normal("n"));
    sim.add(builtin("b"));
    sim.wake(holder, cmd(1));
    let acts = sim.run();
    assert!(spawned_units(&sim, &acts).contains(&u));
}

// Third-party priority: Interactive > Event > Background; FIFO within one
#[test]
fn l_priority_interactive_then_event_then_background() {
    let p = Profile::target();
    let mut sim = Sim::new(&p);
    let full = fill_slots(&mut sim, p.max_active);
    let bg = sim.add(first_run("bg"));
    let ev = woken(&mut sim, "ev", event("playback.state", 1));
    let ia = woken(&mut sim, "ia", cmd(2));
    let acts = sim.run();
    assert_eq!(
        spawned_units(&sim, &acts),
        vec![ia],
        "Interactive takes the reserved slot at once"
    );
    sim.bring_up(ia); // reserve free again
    let acts = sim.ready(full[0].1);
    assert_eq!(
        spawned_units(&sim, &acts),
        vec![ev],
        "Event before Background when one slot frees"
    );
    let acts = sim.ready(full[1].1);
    assert_eq!(spawned_units(&sim, &acts), vec![bg]);
}

#[test]
fn l_fifo_within_a_priority() {
    let p = Profile::target();
    let mut sim = Sim::new(&p);
    let full = fill_slots(&mut sim, p.max_active);
    let e1 = woken(&mut sim, "e1", event("playback.state", 1));
    let e2 = woken(&mut sim, "e2", event("playback.state", 2));
    assert!(spawns(&sim.run()).is_empty());
    let acts = sim.ready(full[0].1);
    assert_eq!(spawned_units(&sim, &acts), vec![e1]);
    let acts = sim.ready(full[1].1);
    assert_eq!(spawned_units(&sim, &acts), vec![e2]);
}

// Start-job concurrency: third-party running Starts ≤ max_active (B3)
#[test]
fn l_max_active_limits_wakes_target() {
    let p = Profile::target();
    let mut sim = Sim::new(&p);
    let mut held = Vec::new();
    for i in 0..(p.max_active + 3) {
        held.push(sim.in_standby(normal(&format!("u{i}"))));
    }
    sim.settle();
    for (_, h) in &held {
        sim.wake(*h, event("playback.state", 0));
    }
    assert_eq!(spawns(&sim.run()).len(), p.max_active);
}

// B1–B2: 8 for first runs only, wakes unlimited (today)
#[test]
fn l_b1_wakes_unlimited() {
    let mut sim = Sim::new(&Profile::b1());
    let mut held = Vec::new();
    for i in 0..20 {
        held.push(sim.in_standby(normal(&format!("u{i}"))));
    }
    sim.settle();
    for (_, h) in &held {
        sim.wake(*h, event("playback.state", 0));
    }
    assert_eq!(spawns(&sim.run()).len(), 20);
}

#[test]
fn l_b1_first_runs_limited_to_eight() {
    let mut sim = Sim::new(&Profile::b1());
    let us = (0..20)
        .map(|i| sim.add(first_run(&format!("f{i}"))))
        .collect::<Vec<_>>();
    let acts = sim.run();
    assert_eq!(spawned_units(&sim, &acts), us[..8].to_vec());
    let pid = sim.pid(us[0]);
    sim.spawned(pid, true);
    let acts = sim.ready(pid);
    assert!(
        spawned_units(&sim, &acts).is_empty(),
        "ready keeps the place"
    );
    // §17.3 (today's `lazy_first_runs_tick`): the place frees when the
    // run is classified resident (or its process is gone after standby).
    let acts = sim.classified(pid, true);
    assert_eq!(spawned_units(&sim, &acts), vec![us[8]]);
}

// Interactive reserve: +1 slot for Interactive; if taken, may overrun by 1 more
// ANSWERED (supervisor.md §17.2 Q17), was a design question: read as "Interactive may run up to max_active + 2
// Starts in total (reserve + overrun)". The alternative reading — the
// overrun *is* the reserve — would allow only max_active + 1.
#[test]
fn l_interactive_reserve_then_overrun_by_one() {
    let p = Profile::target();
    let mut sim = Sim::new(&p);
    fill_slots(&mut sim, p.max_active);
    let [i1, i2, i3] = woken_together(&mut sim, ["i1", "i2", "i3"]);
    let acts = sim.run();
    assert_eq!(spawned_units(&sim, &acts), vec![i1, i2]);
    assert!(matches!(sim.job(i3), Some(J::Start { running: false, .. })));
}

// §8: critical pressure → no Interactive overrun (reserve still usable)
#[test]
fn l_critical_pressure_disables_interactive_overrun() {
    let p = Profile::target();
    let mut sim = Sim::new(&p);
    fill_slots(&mut sim, p.max_active);
    sim.pressure_critical();
    let [i1, i2] = woken_together(&mut sim, ["i1", "i2"]);
    let acts = sim.run();
    assert_eq!(spawned_units(&sim, &acts), vec![i1]);
    assert!(matches!(sim.job(i2), Some(J::Start { running: false, .. })));
}

// Event wakes wait for a slot (no reserve)
#[test]
fn l_event_wake_waits_for_a_slot() {
    let p = Profile::target();
    let mut sim = Sim::new(&p);
    let full = fill_slots(&mut sim, p.max_active);
    let e = woken(&mut sim, "e", event("playback.state", 1));
    assert!(spawns(&sim.run()).is_empty());
    let acts = sim.ready(full[0].1);
    assert_eq!(spawned_units(&sim, &acts), vec![e]);
}

// Event wakes muted during start-up and QUIET_AFTER_START_MS after settle
// (§17.1: the adapter reports start-up with `set_muted` / `settled`.)
#[test]
fn l_event_wakes_muted_right_after_start_up() {
    let mut sim = Sim::new(&Profile::b1());
    sim.mute();
    let (u, holder) = sim.in_standby(normal("a"));
    assert!(matches!(
        sim.wake(holder, event("playback.state", 1)),
        Delivery::Dropped
    ));
    assert_eq!(sim.job(u), None);
    sim.settle();
    assert!(matches!(
        sim.wake(holder, event("playback.state", 2)),
        Delivery::Started
    ));
}

// Development: `--plugin` Start runs immediately (bypasses lanes and limit)
#[test]
fn l_development_start_bypasses_limit() {
    let p = Profile::target();
    let mut sim = Sim::new(&p);
    fill_slots(&mut sim, p.max_active);
    woken(&mut sim, "i1", cmd(1));
    woken(&mut sim, "i2", cmd(2));
    sim.run(); // reserve and overrun taken too
    let d = sim.add(dev("d", 30_000));
    let acts = sim.run();
    assert_eq!(spawned_units(&sim, &acts), vec![d]);
}

// Bounds: third-party lane ≤ 256 waiting Start jobs (B3)
#[test]
fn l_third_party_lane_bounded_at_256() {
    let p = Profile::target();
    let mut sim = Sim::new(&p);
    fill_slots(&mut sim, p.max_active);
    for i in 0..p.lane_max.unwrap() {
        let (_, h) = sim.in_standby(normal(&format!("q{i}")));
        assert!(
            matches!(sim.wake(h, event("playback.state", 0)), Delivery::Started),
            "#{i}"
        );
    }
    let (u, h) = sim.in_standby(normal("over"));
    assert!(matches!(
        sim.wake(h, event("playback.state", 0)),
        Delivery::Refused
    ));
    assert_eq!(sim.job(u), None);
}

#[test]
fn l_b1_third_party_lane_unbounded() {
    let mut sim = Sim::new(&Profile::b1());
    let us = (0..300)
        .map(|i| sim.add(first_run(&format!("f{i}"))))
        .collect::<Vec<_>>();
    assert!(us.iter().all(|u| sim.job(*u).is_some()));
}

// Stop and Standby jobs are never concurrency-limited
both! {
    fn l_stop_jobs_not_concurrency_limited(p) {
        let mut sim = Sim::new(p);
        let units = (0..(p.max_active + 4)).map(|i| sim.in_active(normal(&format!("s{i}")))).collect::<Vec<_>>();
        for (u, _) in &units {
            sim.stop(*u, StopReason::Disable, Then::Inactive).unwrap();
        }
        assert_eq!(terminates(&sim.run()).len(), units.len());
    }
}
