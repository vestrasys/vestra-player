//! Start-limit spec tests (queue limits), written from the design tables,
//! not from the core: lanes, priorities, `max_active`, the Interactive
//! reserve, built-ins first, bounds, deadlines and answers (supervisor.md
//! §5 "Start order and limits", §4), supervisor-loop.md §4 B3 ("LIMIT 64
//! removed; wake failures keep registrations; 200-wake storm"), and the
//! coordinator's ruling on the request timeout's scope (blackboard
//! `doc/process/rounds/2026-11-supervisor.md`, Open items).
//!
//! Everything runs under `Profile::other_timers()`: `max_active` =
//! min(4, max(2, 8 / 2)) = 4, Interactive cap 4 + 2 = 6, backlog 64, lane
//! 256, start timeout 15 s, no request timeout. The request-timeout scope
//! tests use `Profile::target()`, which has the 5 min value.
//!
//! Names: `c_` concurrency and reserve, `o_` order (priority, FIFO, lanes),
//! `bf_` built-ins first and outside the limit, `q_` backlog bound and
//! answers, `d_` deadlines, `k_` wake failures keep registrations, `s_`
//! storms, `w_` work (request accounting), `x_` `LIMIT 64`.
use std::collections::{BTreeMap, VecDeque};

use super::*;

fn b3() -> Sim {
    Sim::new(&Profile::other_timers())
}

const MAX: usize = 4;
/// Interactive cap: `max_active` + reserve 1 + overrun 1.
const ICAP: usize = MAX + 2;
const START: Ms = 15_000;

/// `n` Normal units in standby (start-up settled), not woken.
fn standbys(sim: &mut Sim, prefix: &str, n: usize) -> Vec<(UnitId, Pid)> {
    let held = (0..n)
        .map(|i| sim.in_standby(normal(&format!("{prefix}{i}"))))
        .collect::<Vec<_>>();
    sim.settle();
    held
}

fn spawned_units(sim: &Sim, acts: &[Act]) -> Vec<UnitId> {
    spawns(acts)
        .into_iter()
        .map(|p| sim.unit_of(p).expect("spawned pid is indexed"))
        .collect()
}

/// Third-party units of `us` holding a running Start (`Activating{Spawn |
/// Ready}`: a slot from `run_jobs` until `process_ready`).
fn starting(sim: &Sim, us: &[UnitId]) -> usize {
    us.iter()
        .filter(|u| matches!(sim.st(**u), S::Spawn(_) | S::Ready(_)))
        .count()
}

/// Of those, the ones whose Start is not Interactive.
fn starting_not_interactive(sim: &Sim, us: &[UnitId]) -> usize {
    us.iter()
        .filter(|u| matches!(sim.st(**u), S::Spawn(_) | S::Ready(_)))
        .filter(|u| {
            sim.running_start_cause(**u)
                .is_some_and(|c| !c.contains("interactive"))
        })
        .count()
}

/// `MAX` units woken by an event and spawned (each holds a slot).
fn fill_with_events(sim: &mut Sim, prefix: &str) -> Vec<(UnitId, Pid)> {
    let held = standbys(sim, prefix, MAX);
    for (_, h) in &held {
        assert!(matches!(
            sim.wake(*h, event("playback.state", 0)),
            Delivery::Started
        ));
    }
    let acts = sim.run();
    assert_eq!(spawns(&acts).len(), MAX);
    held.iter()
        .map(|(u, _)| {
            let pid = sim.pid(*u);
            sim.spawned(pid, true);
            (*u, pid)
        })
        .collect()
}

fn is_waiting(sim: &Sim, u: UnitId) -> bool {
    matches!(sim.job(u), Some(J::Start { running: false, .. }))
}

// ---------------------------------------------------------------------------
// c_: start-job concurrency and the Interactive reserve
// ---------------------------------------------------------------------------

#[test]
fn c_max_active_is_four_at_eight_cores() {
    let t = Tunables::new(8, 16_384);
    assert_eq!(t.max_active, 4);
    assert_eq!(max_active_for(1), 2, "max(2, …)");
    assert_eq!(max_active_for(4), 2);
    assert_eq!(max_active_for(6), 3);
    assert_eq!(max_active_for(8), 4);
    assert_eq!(max_active_for(32), 4, "min(4, …)");
}

#[test]
fn c_event_wakes_take_exactly_max_active_in_fifo_order() {
    let mut sim = b3();
    let held = standbys(&mut sim, "e", MAX + 2);
    for (_, h) in &held {
        assert!(matches!(
            sim.wake(*h, event("playback.state", 0)),
            Delivery::Started
        ));
    }
    let acts = sim.run();
    let us = held.iter().map(|(u, _)| *u).collect::<Vec<_>>();
    assert_eq!(spawned_units(&sim, &acts), us[..MAX].to_vec());
    assert_eq!(sim.slots(), MAX);
    assert!(is_waiting(&sim, us[MAX]) && is_waiting(&sim, us[MAX + 1]));
}

#[test]
fn c_interactive_reserve_and_overrun_exactly_two_over_max() {
    let mut sim = b3();
    fill_with_events(&mut sim, "f");
    let held = standbys(&mut sim, "i", 3);
    for (i, (_, h)) in held.iter().enumerate() {
        sim.wake(*h, cmd(i as u32));
    }
    let acts = sim.run();
    assert_eq!(
        spawned_units(&sim, &acts),
        vec![held[0].0, held[1].0],
        "reserve + overrun"
    );
    assert_eq!(sim.slots(), ICAP);
    assert!(is_waiting(&sim, held[2].0));
}

#[test]
fn c_interactive_alone_is_capped_at_max_plus_two() {
    let mut sim = b3();
    let held = standbys(&mut sim, "i", ICAP + 1);
    for (i, (_, h)) in held.iter().enumerate() {
        sim.wake(*h, cmd(i as u32));
    }
    let acts = sim.run();
    assert_eq!(spawns(&acts).len(), ICAP);
    assert!(is_waiting(&sim, held[ICAP].0));
}

// An event may start only while fewer than `max_active` third-party starts
// run, whatever their priority: the reserve is for Interactive only.
#[test]
fn c_event_waits_until_total_starts_drop_below_max() {
    let mut sim = b3();
    let held = standbys(&mut sim, "i", ICAP);
    for (i, (_, h)) in held.iter().enumerate() {
        sim.wake(*h, cmd(i as u32));
    }
    sim.run();
    let pids = held
        .iter()
        .map(|(u, _)| {
            let pid = sim.pid(*u);
            sim.spawned(pid, true);
            pid
        })
        .collect::<Vec<_>>();
    let (e, eh) = sim.in_standby(normal("e"));
    sim.wake(eh, event("playback.state", 9));
    assert!(spawns(&sim.run()).is_empty());
    // 6 → 5 → 4 running: still no room for an event.
    let a = sim.ready(pids[0]);
    assert!(spawned_units(&sim, &a).is_empty());
    let a = sim.ready(pids[1]);
    assert!(spawned_units(&sim, &a).is_empty());
    assert!(is_waiting(&sim, e));
    // 4 → 3: the event starts.
    let a = sim.ready(pids[2]);
    assert_eq!(spawned_units(&sim, &a), vec![e]);
}

// A slot is held from `run_jobs` until `process_ready`, and freed by a failed
// start as well.
#[test]
fn c_failed_spawn_frees_its_slot() {
    let mut sim = b3();
    let held = standbys(&mut sim, "g", MAX + 1);
    for (_, h) in &held {
        sim.wake(*h, event("playback.state", 0));
    }
    sim.run();
    assert!(is_waiting(&sim, held[MAX].0));
    let first = sim.pid(held[0].0);
    let acts = sim.spawned(first, false);
    assert_eq!(spawned_units(&sim, &acts), vec![held[MAX].0]);
    assert_eq!(sim.slots(), MAX);
}

// ---------------------------------------------------------------------------
// o_: priority order and FIFO within a lane
// ---------------------------------------------------------------------------

/// Spawn order while one running start at a time becomes ready (oldest
/// first): Interactive > Event > Background > first runs; FIFO within each.
#[test]
fn o_drain_order_interactive_event_background_first_run_fifo() {
    let mut sim = b3();
    let fill = fill_with_events(&mut sim, "E");
    let mut running: VecDeque<Pid> = fill.iter().map(|(_, p)| *p).collect();
    let is = standbys(&mut sim, "i", 3);
    for (k, (_, h)) in is.iter().enumerate() {
        sim.wake(*h, cmd(k as u32));
    }
    for p in spawns(&sim.run()) {
        sim.spawned(p, true);
        running.push_back(p);
    }
    assert_eq!(running.len(), ICAP);
    // Queued later than Interactive, in this order: first runs, then a
    // Resident boot start (Background), then events.
    let f1 = sim.add(first_run("f1"));
    let f2 = sim.add(first_run("f2"));
    let r1 = sim.add(resident("r1"));
    let (e1, h1) = sim.in_standby(normal("e1"));
    let (e2, h2) = sim.in_standby(normal("e2"));
    sim.wake(h1, event("playback.state", 11));
    sim.wake(h2, event("player.fullscreen", 12));
    assert!(spawns(&sim.run()).is_empty());
    let third = [f1, f2, r1, e1, e2]
        .into_iter()
        .chain(fill.iter().map(|(u, _)| *u))
        .chain(is.iter().map(|(u, _)| *u))
        .collect::<Vec<_>>();
    let mut order = Vec::new();
    while let Some(pid) = running.pop_front() {
        let acts = sim.ready(pid);
        for p in spawns(&acts) {
            order.push(sim.unit_of(p).unwrap());
            sim.spawned(p, true);
            running.push_back(p);
        }
        assert!(starting(&sim, &third) <= ICAP);
        assert!(starting_not_interactive(&sim, &third) <= MAX);
    }
    assert_eq!(order, vec![is[2].0, e1, e2, r1, f1, f2]);
}

// ---------------------------------------------------------------------------
// bf_: built-ins first (`Installed`) and outside `max_active`
// ---------------------------------------------------------------------------

#[test]
fn bf_background_waits_until_every_builtin_is_active() {
    let mut sim = b3();
    let b1 = sim.add(builtin("b1"));
    let n = sim.add(first_run("n"));
    let r = sim.add(resident("r"));
    let b2 = sim.add(builtin("b2"));
    let acts = sim.run();
    assert_eq!(spawned_units(&sim, &acts), vec![b1, b2]);
    let p1 = sim.pid(b1);
    let p2 = sim.pid(b2);
    sim.spawned(p1, true);
    sim.spawned(p2, true);
    assert!(
        is_waiting(&sim, n) && is_waiting(&sim, r),
        "spawned is not active"
    );
    assert!(spawns(&sim.ready(p1)).is_empty(), "b2 still starting");
    let acts = sim.ready(p2);
    assert_eq!(spawned_units(&sim, &acts), vec![r, n]);
}

#[test]
fn bf_verifying_builtin_holds_background() {
    let mut sim = b3();
    let b = sim.add(builtin("b").shared(1).verify());
    let n = sim.add(first_run("n"));
    sim.run();
    assert_eq!(sim.st(b), S::Verify);
    assert!(is_waiting(&sim, n));
    sim.at(T0 + VERIFY_WAIT); // own process, still not active
    assert!(is_waiting(&sim, n));
}

// "until it is active": a built-in whose start fails has no Start job installed
// any more, so it holds nobody back.
#[test]
fn bf_failed_builtin_start_releases_background() {
    let mut sim = b3();
    let b = sim.add(builtin("b"));
    let n = sim.add(first_run("n"));
    sim.run();
    // Its start retries hold the background start back; the last failure
    // releases it.
    let acts = sim.spawn_fails(b);
    assert!(sim.st(b).is_failed(), "{:?}", sim.st(b));
    let last = spawns(&acts).last().copied().expect("a spawn");
    assert_eq!(sim.unit_of(last), Some(n));
}

#[test]
fn bf_wakes_are_not_held_by_starting_builtins() {
    let mut sim = b3();
    let (e, eh) = sim.in_standby(normal("e"));
    let (i, ih) = sim.in_standby(normal("i"));
    sim.settle();
    let b = sim.add(builtin("b"));
    sim.wake(eh, event("playback.state", 1));
    sim.wake(ih, cmd(2));
    let acts = sim.run();
    let got = spawned_units(&sim, &acts);
    assert!(
        got.contains(&b) && got.contains(&e) && got.contains(&i),
        "{got:?}"
    );
}

#[test]
fn bf_builtins_and_shared_members_never_take_a_slot() {
    let mut sim = b3();
    let fill = fill_with_events(&mut sim, "f");
    let mut bs = (0..8)
        .map(|k| sim.add(builtin(&format!("b{k}"))))
        .collect::<Vec<_>>();
    bs.extend((0..4).map(|k| sim.add(builtin(&format!("m{k}")).shared(1))));
    sim.run();
    for b in &bs {
        assert!(
            matches!(sim.st(*b), S::Spawn(_) | S::Ready(_) | S::Active(_)),
            "{:?}",
            sim.st(*b)
        );
    }
    assert_eq!(sim.slots(), MAX, "only the third-party starts count");
    // The Interactive reserve and overrun are still free.
    let is = standbys(&mut sim, "i", 3);
    for (k, (_, h)) in is.iter().enumerate() {
        sim.wake(*h, cmd(k as u32));
    }
    assert_eq!(spawns(&sim.run()).len(), 2);
    let _ = fill;
}

#[test]
fn bf_builtin_wake_is_outside_the_limit() {
    let mut sim = b3();
    let held = standbys(&mut sim, "i", ICAP);
    for (k, (_, h)) in held.iter().enumerate() {
        sim.wake(*h, cmd(k as u32));
    }
    assert_eq!(spawns(&sim.run()).len(), ICAP);
    let (b, bh) = sim.in_standby(builtin_standby("b"));
    sim.wake(bh, event("playback.state", 7));
    let acts = sim.run();
    assert_eq!(spawned_units(&sim, &acts), vec![b]);
}

// ---------------------------------------------------------------------------
// q_: backlog ≤ 64, the 65th trigger, answers (Bounds)
// ---------------------------------------------------------------------------

/// The new (65th) trigger is the one refused; the 64 held ones are kept and
/// delivered in order; the refused command is answered "busy" once.
#[test]
fn q_sixty_fifth_command_is_refused_and_answered_busy() {
    let mut sim = b3();
    let (u, h) = sim.in_standby(normal("a"));
    sim.settle();
    for k in 0..64u32 {
        assert!(
            matches!(sim.wake(h, cmd(k)), Delivery::Started | Delivery::Held),
            "#{k}"
        );
    }
    assert_eq!(sim.backlog_len(u), 64);
    assert!(matches!(sim.wake(h, cmd(64)), Delivery::Refused));
    assert_eq!(sim.backlog_len(u), 64);
    sim.run();
    let pid = sim.pid(u);
    sim.spawned(pid, true);
    let acts = sim.ready(pid);
    assert_eq!(deliveries(&acts), (0..64).collect::<Vec<_>>());
    assert_eq!(answers(&sim.log), vec![(64, Why::Busy)]);
}

#[test]
fn q_sixty_fifth_trigger_of_each_kind() {
    for (name, t, answered) in [
        ("resolve", resolve(64), true),
        ("drop", drop_(64), true),
        ("callback", callback(64), true),
        ("event", event("player.fullscreen", 64), false),
    ] {
        let mut sim = b3();
        let (u, h) = sim.in_standby(normal("a"));
        sim.settle();
        for k in 0..64u32 {
            sim.wake(h, cmd(k));
        }
        assert!(matches!(sim.wake(h, t), Delivery::Refused), "{name}");
        sim.run();
        assert_eq!(sim.backlog_len(u), 64, "{name}");
        let want = if answered {
            vec![(64, Why::Busy)]
        } else {
            vec![]
        };
        assert_eq!(answers(&sim.log), want, "{name}: answers");
    }
}

/// Only wake events merge (same name, newest wins), also with a full backlog.
#[test]
fn q_full_backlog_still_merges_a_same_name_event() {
    let mut sim = b3();
    let (u, h) = sim.in_standby(normal("a"));
    sim.settle();
    for k in 0..63u32 {
        sim.wake(h, cmd(k));
    }
    sim.wake(h, event("playback.state", 100));
    assert_eq!(sim.backlog_len(u), 64);
    assert!(!matches!(
        sim.wake(h, event("playback.state", 101)),
        Delivery::Refused
    ));
    assert!(matches!(
        sim.wake(h, event("player.fullscreen", 102)),
        Delivery::Refused
    ));
    sim.run();
    let pid = sim.pid(u);
    sim.spawned(pid, true);
    let mut got = deliveries(&sim.ready(pid));
    got.sort();
    let mut want = (0..63).collect::<Vec<_>>();
    want.push(101);
    assert_eq!(got, want);
    assert!(answers(&sim.log).is_empty(), "events refused silently");
}

/// The lane bound (256 waiting Starts) answers an Interactive trigger it
/// cannot take.
#[test]
fn q_full_lane_answers_an_interactive_trigger_busy() {
    let mut sim = b3();
    fill_with_events(&mut sim, "f");
    for k in 0..256 {
        let (_, h) = sim.in_standby(normal(&format!("q{k}")));
        assert!(
            matches!(sim.wake(h, event("playback.state", 0)), Delivery::Started),
            "#{k}"
        );
    }
    let (u, h) = sim.in_standby(normal("over"));
    assert!(matches!(sim.wake(h, cmd(77)), Delivery::Refused));
    assert_eq!(sim.job(u), None);
    sim.run();
    assert_eq!(answers(&sim.log), vec![(77, Why::Busy)]);
}

// ---------------------------------------------------------------------------
// d_: deadlines
// ---------------------------------------------------------------------------

/// Waiting for a slot has no deadline and no answer (no cap for interactive
/// triggers either), and no "Starting <name>…" while it waits; once it has a
/// slot the notice is due 400 ms after its `Spawn`.
#[test]
fn d_interactive_waiting_for_a_slot_is_neither_answered_nor_announced() {
    let mut sim = b3();
    let held = standbys(&mut sim, "i", ICAP + 1);
    for (k, (_, h)) in held.iter().enumerate() {
        sim.wake(*h, cmd(k as u32));
    }
    let t = sim.now;
    sim.run();
    let pids = held[..ICAP]
        .iter()
        .map(|(u, _)| {
            let pid = sim.pid(*u);
            sim.spawned(pid, true);
            pid
        })
        .collect::<Vec<_>>();
    let c = held[ICAP].0;
    sim.at(t + START - 1);
    assert!(is_waiting(&sim, c));
    assert!(answers(&sim.log).is_empty());
    assert!(
        !sim.log.iter().any(|a| starting_notice_unit(a) == Some(c)),
        "no notice while waiting for a slot"
    );
    // A slot frees just before the fillers' own start timeouts.
    sim.ready(pids[0]);
    let t1 = sim.now;
    assert!(matches!(sim.st(c), S::Spawn(_)), "{:?}", sim.st(c));
    let pid = sim.pid(c);
    sim.spawned(pid, true);
    let early = sim.at(t1 + STARTING_OSD - 1);
    assert!(!early.iter().any(|a| starting_notice_unit(a) == Some(c)));
    let due = sim.at(t1 + STARTING_OSD);
    assert!(
        due.iter().any(|a| starting_notice_unit(a) == Some(c)),
        "{:?}",
        names(&due)
    );
}

/// `TimeoutStartSec` counts from the `Spawn`, not from the request: a start
/// that waited 10 s for a slot still gets its full 15 s.
#[test]
fn d_start_timeout_counts_from_the_spawn_after_a_slot_wait() {
    let mut sim = b3();
    let fill = fill_with_events(&mut sim, "f");
    let (e, eh) = sim.in_standby(normal("e"));
    sim.wake(eh, event("playback.state", 1));
    sim.run();
    sim.after(10_000);
    sim.ready(fill[0].1);
    let t1 = sim.now;
    let pid = sim.pid(e);
    sim.spawned(pid, true);
    let before = sim.at(t1 + START - 1);
    assert!(!terminates(&before).iter().any(|t| t.3 == pid));
    let at = sim.at(t1 + START);
    assert!(
        terminates(&at).iter().any(|t| t.3 == pid),
        "{:?}",
        names(&at)
    );
}

/// `TimeoutStartSec` per class under B3 (15 s for every class until B6; a
/// development load keeps its own 1–120 s).
#[test]
fn d_start_timeout_per_class() {
    let cases: [(&str, Spec, Ms); 3] = [
        ("builtin", builtin("b"), START),
        ("resident", resident("r"), START),
        ("development", dev("d", 30_000), 30_000),
    ];
    for (name, spec, limit) in cases {
        let mut sim = b3();
        let u = sim.add(spec);
        sim.run();
        let pid = sim.pid(u);
        sim.spawned(pid, true);
        assert!(
            terminates(&sim.at(T0 + limit - 1)).is_empty(),
            "{name}: not before"
        );
        let acts = sim.at(T0 + limit);
        assert_eq!(
            terminates(&acts).iter().map(|t| t.3).collect::<Vec<_>>(),
            vec![pid],
            "{name}"
        );
    }
    // Normal: a wake (the start a click waits for).
    let mut sim = b3();
    let (u, h) = sim.in_standby(normal("n"));
    sim.settle();
    sim.wake(h, cmd(1));
    let t = sim.now;
    sim.run();
    let pid = sim.pid(u);
    sim.spawned(pid, true);
    assert!(terminates(&sim.at(t + START - 1)).is_empty());
    assert_eq!(terminates(&sim.at(t + START)).len(), 1);
}

// ---------------------------------------------------------------------------
// k_: wake failures keep registrations
// ---------------------------------------------------------------------------

#[test]
fn k_start_timeout_of_a_wake_keeps_registrations_and_answers() {
    let mut sim = b3();
    let (u, h) = sim.in_standby(normal("a"));
    sim.settle();
    sim.wake(h, cmd(1));
    sim.wake(h, event("playback.state", 2));
    let t = sim.now;
    sim.run();
    let pid = sim.pid(u);
    sim.spawned(pid, true);
    let from = sim.log.len();
    sim.at(t + START);
    sim.gone(pid);
    let acts = sim.log[from..].to_vec();
    assert!(matches!(sim.st(u), S::Standby { .. }), "{:?}", sim.st(u));
    assert!(!has(&acts, "Revoke"), "{:?}", names(&acts));
    assert_eq!(answers(&acts), vec![(1, Why::CouldNotStart)]);
    // After the restart hold the held wake event starts it again.
    let a = sim.past_hold(u);
    assert!(spawned_units(&sim, &a).contains(&u));
}

#[test]
fn k_spawn_failure_keeps_registrations_and_the_next_click_starts_again() {
    let mut sim = b3();
    let (u, h) = sim.in_standby(normal("a"));
    sim.settle();
    sim.wake(h, cmd(1));
    sim.run();
    let acts = sim.spawn_fails(u);
    assert!(matches!(sim.st(u), S::Standby { .. }), "{:?}", sim.st(u));
    assert!(acts.iter().filter_map(revoked).next().is_none());
    sim.past_hold(u);
    let holder = sim.pid(u);
    assert!(matches!(sim.wake(holder, cmd(2)), Delivery::Started));
    let acts = sim.run();
    assert_eq!(spawned_units(&sim, &acts), vec![u]);
    let pid = sim.pid(u);
    sim.spawned(pid, true);
    assert_eq!(deliveries(&sim.ready(pid)), vec![2]);
}

/// never kept in standby for a unit without a standby checkpoint.
#[test]
fn k_resident_spawn_failure_is_not_kept_in_standby() {
    let mut sim = b3();
    let u = sim.add(resident("r"));
    sim.run();
    sim.spawn_fails(u);
    assert!(sim.st(u).is_failed(), "{:?}", sim.st(u));
}

// ---------------------------------------------------------------------------
// s_: 200-wake storms (supervisor-loop.md §4 B3)
// ---------------------------------------------------------------------------

struct Storm {
    units: Vec<UnitId>,
    max_seen: usize,
    max_not_interactive: usize,
}

/// Runs every start to the end, one at a time (oldest spawn first); the
/// units for which `fail` is true fail their spawn. Checks the limits after
/// every input.
fn drain(sim: &mut Sim, units: Vec<UnitId>, fail: impl Fn(UnitId) -> bool) -> Storm {
    let mut st = Storm {
        units,
        max_seen: 0,
        max_not_interactive: 0,
    };
    let mut pending: VecDeque<Pid> = VecDeque::new();
    let acts = sim.run();
    pending.extend(spawns(&acts));
    let mut guard = 0;
    while let Some(pid) = pending.pop_front() {
        guard += 1;
        assert!(guard < 10_000, "storm does not settle");
        st.max_seen = st.max_seen.max(starting(sim, &st.units));
        st.max_not_interactive = st
            .max_not_interactive
            .max(starting_not_interactive(sim, &st.units));
        assert!(sim.slots() <= ICAP, "slots {}", sim.slots());
        let u = sim.unit_of(pid).unwrap();
        let acts = if fail(u) {
            sim.spawned(pid, false)
        } else {
            let mut a = sim.spawned(pid, true);
            a.extend(sim.ready(pid));
            a
        };
        pending.extend(spawns(&acts));
        // Start retries of failed spawns are due a moment later.
        while pending.is_empty()
            && let Some(next) = sim
                .next_deadline()
                .filter(|t| *t > sim.now && *t < sim.now + 5_000)
        {
            let acts = sim.at(next);
            pending.extend(spawns(&acts));
        }
    }
    assert!(spawns(&sim.run()).is_empty());
    st
}

fn tally(sim: &Sim) -> (BTreeMap<u32, usize>, BTreeMap<u32, Vec<Why>>) {
    let mut del = BTreeMap::new();
    for p in deliveries(&sim.log) {
        *del.entry(p).or_insert(0) += 1;
    }
    let mut ans: BTreeMap<u32, Vec<Why>> = BTreeMap::new();
    for (p, w) in answers(&sim.log) {
        ans.entry(p).or_default().push(w);
    }
    (del, ans)
}

#[test]
fn s_two_hundred_event_wakes_never_exceed_max_active() {
    let mut sim = b3();
    let held = standbys(&mut sim, "s", 200);
    for (k, (_, h)) in held.iter().enumerate() {
        assert!(matches!(
            sim.wake(*h, event("playback.state", k as u32)),
            Delivery::Started
        ));
    }
    let st = drain(&mut sim, held.iter().map(|(u, _)| *u).collect(), |_| false);
    assert_eq!(st.max_seen, MAX, "reaches the limit, never more");
    let (del, ans) = tally(&sim);
    assert_eq!(del.len(), 200);
    assert!(del.values().all(|n| *n == 1));
    assert!(ans.is_empty());
    assert!(held.iter().all(|(u, _)| matches!(sim.st(*u), S::Active(_))));
}

#[test]
fn s_two_hundred_mixed_wakes_respect_limit_and_reserve() {
    let mut sim = b3();
    let held = standbys(&mut sim, "s", 200);
    for (k, (_, h)) in held.iter().enumerate() {
        let t = if k % 4 == 0 {
            cmd(k as u32)
        } else {
            event("playback.state", k as u32)
        };
        sim.wake(*h, t);
    }
    let st = drain(&mut sim, held.iter().map(|(u, _)| *u).collect(), |_| false);
    assert!(st.max_seen <= ICAP, "{}", st.max_seen);
    assert!(st.max_not_interactive <= MAX, "{}", st.max_not_interactive);
    let (del, ans) = tally(&sim);
    assert_eq!(del.len(), 200);
    assert!(del.values().all(|n| *n == 1));
    assert!(ans.is_empty());
}

/// Every trigger is either delivered once or answered once (a command "could
/// not start"; a wake event dropped silently); a failed wake keeps its entry
/// points (no `Revoke`).
#[test]
fn s_storm_with_failures_delivers_or_answers_every_trigger() {
    let mut sim = b3();
    let held = standbys(&mut sim, "s", 200);
    let mut is_cmd = BTreeMap::new();
    for (k, (_, h)) in held.iter().enumerate() {
        let c = k % 3 == 0;
        is_cmd.insert(k as u32, c);
        let t = if c {
            cmd(k as u32)
        } else {
            event("playback.state", k as u32)
        };
        sim.wake(*h, t);
    }
    let failing = held
        .iter()
        .enumerate()
        .filter(|(k, _)| k % 7 == 0)
        .map(|(_, (u, _))| *u)
        .collect::<Vec<_>>();
    let st = drain(&mut sim, held.iter().map(|(u, _)| *u).collect(), |u| {
        failing.contains(&u)
    });
    assert!(st.max_seen <= ICAP);
    assert!(st.max_not_interactive <= MAX);
    let (del, ans) = tally(&sim);
    for (k, (u, _)) in held.iter().enumerate() {
        let k = k as u32;
        let d = del.get(&k).copied().unwrap_or(0);
        let a = ans.get(&k).cloned().unwrap_or_default();
        if failing.contains(u) {
            assert_eq!(d, 0, "#{k}");
            let want = if is_cmd[&k] {
                vec![Why::CouldNotStart]
            } else {
                vec![]
            };
            assert_eq!(a, want, "#{k}");
            assert!(matches!(sim.st(*u), S::Standby { .. }), "#{k}");
        } else {
            assert_eq!((d, a.len()), (1, 0), "#{k}");
        }
    }
    assert!(!has(&sim.log, "Revoke"), "wake failures keep registrations");
}

/// 200 wakes to one unit: 64 held (delivered in order), 136 refused and
/// answered "busy", each exactly once.
#[test]
fn s_two_hundred_wakes_to_one_unit() {
    let mut sim = b3();
    let (u, h) = sim.in_standby(normal("a"));
    sim.settle();
    let refused = (0..200u32)
        .filter(|k| matches!(sim.wake(h, cmd(*k)), Delivery::Refused))
        .count();
    assert_eq!(refused, 136);
    let st = drain(&mut sim, vec![u], |_| false);
    assert_eq!(st.max_seen, 1);
    let (del, ans) = tally(&sim);
    assert_eq!(
        del.keys().copied().collect::<Vec<_>>(),
        (0..64).collect::<Vec<_>>()
    );
    assert!(del.values().all(|n| *n == 1));
    assert_eq!(
        ans.keys().copied().collect::<Vec<_>>(),
        (64..200).collect::<Vec<_>>()
    );
    assert!(ans.values().all(|w| *w == vec![Why::Busy]));
}

// ---------------------------------------------------------------------------
// w_: request accounting (Request row, `work.rs`) and the
// timeout scope ruling
// ---------------------------------------------------------------------------

/// Answers an idle query "not busy" if one came; returns the terminates of
/// the answer.
fn answer_idle(sim: &mut Sim, pid: Pid, acts: &[Act]) -> Vec<(Ms, bool, String, Pid)> {
    if acts.iter().any(|a| query_pid(a) == Some(pid)) {
        terminates(&sim.resources(pid, false))
    } else {
        terminates(acts)
    }
}

#[test]
fn w_work_in_flight_keeps_the_unit_out_of_standby() {
    let p = Profile::other_timers();
    let mut sim = b3();
    let (u, pid) = sim.in_active(normal("a"));
    sim.begin_work_kind(pid, 1);
    let acts = sim.after(p.idle);
    assert!(answer_idle(&mut sim, pid, &acts).is_empty());
    assert_eq!(sim.st(u), S::Active(pid), "busy: no standby");
    sim.end_work_kind(pid, 1);
    let acts = sim.after(p.idle);
    let t = answer_idle(&mut sim, pid, &acts);
    assert_eq!(t.len(), 1, "idle and not busy: standby");
    assert!(t[0].1, "standby keeps registrations");
}

#[test]
fn w_work_is_counted_per_request() {
    let p = Profile::other_timers();
    let mut sim = b3();
    let (u, pid) = sim.in_active(normal("a"));
    sim.begin_work_kind(pid, 1);
    sim.begin_work_kind(pid, 2);
    sim.end_work_kind(pid, 1);
    let acts = sim.after(p.idle);
    assert!(answer_idle(&mut sim, pid, &acts).is_empty());
    assert_eq!(sim.st(u), S::Active(pid), "request 2 still in flight");
}

#[test]
fn w_no_request_timeout_before_b6() {
    let mut sim = b3();
    let (u, pid) = sim.in_active(normal("a"));
    sim.begin_work_kind(pid, 1);
    let t = sim.now;
    let acts = sim.at(t + 3_600_000);
    assert!(terminates(&acts).is_empty());
    assert!(!has(&acts, "Kill"));
    assert_eq!(sim.st(u), S::Active(pid));
}

/// Chrome's 5 min is per request: a request that ended does not count; the
/// deadline is the oldest request still in flight.
#[test]
fn w_request_timeout_is_per_request_in_flight() {
    let p = Profile::target();
    let limit = p.request_timeout.unwrap();
    let mut sim = Sim::new(&p);
    let (_, pid) = sim.in_active(normal("a"));
    let t = sim.now;
    sim.begin_work_kind(pid, 1);
    sim.at(t + 60_000);
    sim.begin_work_kind(pid, 2);
    sim.at(t + 120_000);
    sim.end_work_kind(pid, 1);
    let acts = sim.at(t + limit);
    assert!(
        terminates(&acts).is_empty() && !has(&acts, "Kill"),
        "request 1 ended: {:?}",
        names(&acts)
    );
    let acts = sim.at(t + 60_000 + limit - 1);
    assert!(terminates(&acts).is_empty() && !has(&acts, "Kill"));
    let acts = sim.at(t + 60_000 + limit);
    assert!(
        terminates(&acts).iter().any(|x| x.3 == pid)
            || acts.iter().any(|a| kill_pid(a) == Some(pid)),
        "request 2 timed out: {:?}",
        names(&acts)
    );
}

/// Coordinator ruling (blackboard, Open items): the 5 min applies only to
/// host→plugin requests awaiting a reply; plugin-declared long work
/// (`commands.busy`, settings busy) has no cap, though it keeps the unit
/// busy (out of standby).
#[test]
fn w_declared_busy_has_no_request_cap() {
    let p = Profile::target();
    let mut sim = Sim::new(&p);
    let (u, pid) = sim.in_active(normal("a"));
    sim.begin_declared_busy(pid);
    let t = sim.now;
    let acts = sim.at(t + 30 * 60_000);
    let acts = answer_idle(&mut sim, pid, &acts);
    assert!(acts.is_empty(), "{acts:?}");
    assert!(!sim.log.iter().any(|a| kill_pid(a) == Some(pid)));
    assert_eq!(sim.st(u), S::Active(pid), "busy for 30 min, never capped");
}

// ---------------------------------------------------------------------------
// x_: `LIMIT 64` removed (supervisor-loop.md §4)
// ---------------------------------------------------------------------------

#[test]
fn x_more_than_64_builtins_activate_at_once() {
    let mut sim = b3();
    let us = (0..70)
        .map(|k| sim.add(builtin(&format!("b{k}"))))
        .collect::<Vec<_>>();
    let acts = sim.run();
    assert_eq!(spawns(&acts).len(), 70);
    for u in &us {
        let pid = sim.pid(*u);
        sim.spawned(pid, true);
    }
    assert!(us.iter().all(|u| matches!(sim.st(*u), S::Ready(_))));
}

/// The activation book (B2a identity input) takes more than 64 activations
/// once B3 is on.
#[test]
fn x_activation_book_has_no_64_limit_under_b3() {
    let mut sim = b3();
    assert_eq!(sim.activations_accepted(100), 100);
}
