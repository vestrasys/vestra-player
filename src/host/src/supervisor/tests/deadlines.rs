//! Deadlines per job, design §4.5 and timer kinds §6.2, with explicit `now`
//! values: each timer fires at its due time, never one millisecond before,
//! and `next_deadline()` reports the earliest one.
use super::*;

#[test]
fn d_next_deadline_none_when_nothing_is_pending() {
    let mut sim = Sim::new(&Profile::b1());
    assert_eq!(sim.next_deadline(), None);
    sim.in_standby(normal("a"));
    assert_eq!(sim.next_deadline(), None, "a unit in standby has no timer");
}

both! {
    fn d_next_deadline_is_the_earliest(p) {
        let mut sim = Sim::new(p);
        sim.in_active(normal("a")); // idle due at t0 + idle
        let t0 = sim.now;
        sim.after(1_000);
        let u = sim.add(resident("r"));
        sim.run(); // start timeout due at t0 + 1 s + start
        sim.spawned(sim.pid(u), true);
        sim.after(STARTING_OSD); // past any "Starting…" OSD timer
        // B1: 16 s start < 60 s idle; target: 30 s idle < 61 s start.
        let want = (t0 + p.idle).min(t0 + 1_000 + p.start_timeout);
        assert_eq!(sim.next_deadline(), Some(want));
    }
}

// Start, waiting for the previous process: 20 s from the stop
both! {
    fn d_start_waiting_for_previous_process_fails_at_20s(p) {
        let mut sim = Sim::new(p);
        let (u, _) = sim.in_active(normal("a"));
        sim.stop(u, StopReason::Failed, Then::Failed).unwrap();
        sim.run();
        let t = sim.now;
        let job = sim.restart(u, false, None).unwrap();
        sim.at(t + STOP_WAIT - 1);
        assert_eq!(sim.result(job), None, "still waiting");
        sim.at(t + STOP_WAIT);
        assert_eq!(sim.result(job), Some(R::Failed(TEXT_RELOAD_STUCK.into())));
    }
}

// Start, Activating{Verify}: 3 s → own process (also T5)
#[test]
fn d_verify_deadline_is_three_seconds() {
    let mut sim = Sim::new(&Profile::b1());
    let u = sim.add(builtin("b").shared(1).verify());
    sim.run();
    assert_eq!(sim.next_deadline(), Some(T0 + VERIFY_WAIT));
    sim.at(T0 + VERIFY_WAIT - 1);
    assert_eq!(sim.st(u), S::Verify);
    sim.at(T0 + VERIFY_WAIT);
    assert!(matches!(sim.st(u), S::Spawn(_)));
}

// Start, Activating{Spawn|Ready}: TimeoutStartSec (15 s → 60 s)
both! {
    fn d_start_timeout_from_spawn(p) {
        let mut sim = Sim::new(p);
        let u = sim.add(resident("r"));
        sim.run(); // Spawn in flight, no process_spawned yet
        assert!(terminates(&sim.at(T0 + p.start_timeout - 1)).is_empty());
        assert!(matches!(sim.st(u), S::Spawn(_)));
        sim.at(T0 + p.start_timeout);
        assert!(sim.st(u).is_deactivating() || matches!(sim.st(u), S::AutoRestart(_) | S::Failed(_)), "{:?}", sim.st(u));
    }
}

// Development: `--plugin` start timeout 1–120 s kept (A16)
both! {
    fn d_development_start_timeout_120s(p) {
        let mut sim = Sim::new(p);
        let u = sim.add(dev("d", 120_000));
        sim.run();
        let pid = sim.pid(u);
        sim.spawned(pid, true);
        assert!(terminates(&sim.at(T0 + 120_000 - 1)).is_empty());
        assert_eq!(terminates(&sim.at(T0 + 120_000)).len(), 1);
    }
}

#[test]
fn d_development_start_timeout_is_per_load() {
    let mut sim = Sim::new(&Profile::target());
    let u = sim.add(dev("d", 1_000));
    sim.run();
    sim.spawned(sim.pid(u), true);
    assert_eq!(terminates(&sim.at(T0 + 1_000)).len(), 1);
}

// Start, waiting for a slot: no deadline (systemd JobTimeoutSec=infinity)
#[test]
fn d_waiting_for_a_slot_has_no_deadline() {
    let p = Profile::target();
    let mut sim = Sim::new(&p);
    let mut held = Vec::new();
    for i in 0..(p.max_active + 1) {
        held.push(sim.in_standby(normal(&format!("u{i}"))));
    }
    sim.settle();
    for (_, h) in &held {
        sim.wake(*h, event("playback.state", 0));
    }
    let t = sim.now;
    sim.run();
    for (u, _) in &held[..p.max_active] {
        sim.spawned(sim.pid(*u), true);
    }
    // The slot holders time out at `t + start_timeout`; until then the
    // last Start just waits: no shorter deadline, no answer.
    let last = held[p.max_active].0;
    let acts = sim.at(t + p.start_timeout - 1);
    assert!(
        matches!(sim.job(last), Some(J::Start { running: false, .. })),
        "{:?}",
        sim.job(last)
    );
    assert!(answers(&acts).is_empty());
}

// "Starting <name>…" OSD 400 ms after a triggered start
#[test]
fn d_starting_osd_after_400ms() {
    let mut sim = Sim::new(&Profile::b1());
    let (u, h) = sim.in_standby(normal("a"));
    sim.settle();
    sim.wake(h, cmd(1));
    let t = sim.now;
    sim.run();
    sim.spawned(sim.pid(u), true);
    assert!(!has(&sim.at(t + STARTING_OSD - 1), "Notify"));
    assert!(has(&sim.at(t + STARTING_OSD), "Notify"));
}

#[test]
fn d_no_starting_osd_when_ready_within_400ms() {
    let mut sim = Sim::new(&Profile::b1());
    let (u, h) = sim.in_standby(normal("a"));
    sim.settle();
    sim.wake(h, cmd(1));
    let t = sim.now;
    sim.run();
    let pid = sim.pid(u);
    sim.spawned(pid, true);
    sim.at(t + STARTING_OSD - 1);
    sim.ready(pid);
    assert!(!has(&sim.at(t + STARTING_OSD), "Notify"));
}

// Stop, Deactivating{Term}: TimeoutStopSec → Kill (T17); standby 2 s → 5 s
both! {
    fn d_standby_stop_grace(p) {
        let mut sim = Sim::new(p);
        let (_, pid) = sim.in_active(normal("a"));
        sim.idle_out(pid);
        let t = sim.now;
        assert!(!has(&sim.at(t + p.standby_grace - 1), "Kill"));
        assert_eq!(sim.at(t + p.standby_grace).iter().filter_map(kill_pid).collect::<Vec<_>>(), vec![pid]);
    }
}

// Stop, reap + backend cleanup: 20 s → T18
both! {
    fn d_reap_and_cleanup_deadline_20s(p) {
        let mut sim = Sim::new(p);
        let (u, _) = sim.in_active(normal("a"));
        sim.stop(u, StopReason::Disable, Then::Inactive).unwrap();
        sim.run();
        let t = sim.now;
        sim.at(t + STOP_WAIT - 1);
        assert!(sim.st(u).is_deactivating());
        assert!(!sim.stuck(u));
        sim.at(t + STOP_WAIT);
        assert_eq!(sim.st(u), S::Inactive);
        assert!(sim.stuck(u));
    }
}

// Standby, resources answer: "today's lazy.asking wait (value MOVE)" →
// treat as busy, re-arm idle.
// DESIGN QUESTION: today's `lazy.asking` has no timeout (an unanswered
// `owner.resources` keeps the owner out of idle checks until an answer
// arrives), so there is no value to MOVE. This test only checks that an
// unanswered query never stops the unit.
both! {
    fn d_unanswered_resources_query_never_stops(p) {
        let mut sim = Sim::new(p);
        let (u, pid) = sim.in_active(normal("a"));
        sim.after(p.idle);
        let acts = sim.after(10 * p.idle);
        assert!(terminates(&acts).is_empty());
        assert_eq!(sim.st(u), S::Active(pid));
    }
}

// Request (in-flight work): 5 min from B6, off before
both! {
    fn d_request_timeout(p) {
        let mut sim = Sim::new(p);
        let (u, pid) = sim.in_active(normal("a"));
        sim.begin_work(pid);
        let t = sim.now;
        match p.request_timeout {
            Some(limit) => {
                assert!(terminates(&sim.at(t + limit - 1)).is_empty());
                assert_eq!(terminates(&sim.at(t + limit)).len(), 1);
            }
            None => {
                assert!(terminates(&sim.at(t + 3_600_000)).is_empty());
                assert_eq!(sim.st(u), S::Active(pid), "busy keeps it active, no request limit");
            }
        }
    }
}

#[test]
fn d_request_timeout_cleared_by_end_work() {
    let p = Profile::target();
    let mut sim = Sim::new(&p);
    let (u, pid) = sim.in_active(normal("a"));
    sim.begin_work(pid);
    sim.after(1_000);
    sim.end_work(pid);
    sim.touch(pid);
    let acts = sim.at(T0 + p.request_timeout.unwrap() + 1_000);
    assert!(
        !acts
            .iter()
            .any(|a| matches!(terminate(a), Some(t) if t.2 == "failed"))
    );
    assert!(!sim.st(u).is_failed());
}

// Idle timer re-armed lazily by `touch`
both! {
    fn d_touch_pushes_idle_deadline(p) {
        let mut sim = Sim::new(p);
        let (_, pid) = sim.in_active(normal("a"));
        let t = sim.now;
        sim.at(t + p.idle - 10);
        sim.touch(pid);
        let acts = sim.at(t + p.idle);
        assert!(!has(&acts, "QueryResources"), "touched 10 ms before: not idle yet");
        let acts = sim.at(t + p.idle - 10 + p.idle);
        assert!(has(&acts, "QueryResources"));
    }
}

// Shutdown: one deadline (1 s quit / 300 ms device loss) → Kill all
#[test]
fn d_shutdown_single_deadline() {
    for budget in [1_000, 300] {
        let mut sim = Sim::new(&Profile::b1());
        let (_, a) = sim.in_active(normal("a"));
        sim.after(100);
        let (_, b) = sim.in_active(builtin("b"));
        let t = sim.now;
        sim.shutdown(budget);
        assert_eq!(sim.next_deadline(), Some(t + budget));
        assert!(!has(&sim.at(t + budget - 1), "Kill"));
        let killed = sim
            .at(t + budget)
            .iter()
            .filter_map(kill_pid)
            .collect::<Vec<_>>();
        assert!(same_set(&killed, &[a, b]), "budget {budget}: {killed:?}");
    }
}
