//! Process budget and memory pressure, design over budget or under pressure,
//! idle Normal units get the same `Standby` job as idle standby, in LRU order;
//! busy, built-in and Resident units are never selected; recently used units go
//! later. Budget = 3 in the target profile; `other_timers` has none.
use super::*;

fn queried(acts: &[Act]) -> Vec<Pid> {
    acts.iter().filter_map(query_pid).collect()
}

/// Active normal units `names`, touched in order 1 s apart (first = LRU).
fn actives(sim: &mut Sim, ids: &[&str]) -> Vec<(UnitId, Pid)> {
    let v = ids
        .iter()
        .map(|id| sim.in_active(normal(id)))
        .collect::<Vec<_>>();
    for (_, pid) in &v {
        sim.after(1_000);
        sim.touch(*pid);
    }
    v
}

/// Starts one more normal unit with an interactive trigger and brings it
/// up; returns every action from the wake to `Active`.
fn one_more(sim: &mut Sim, id: &str) -> Vec<Act> {
    let from = sim.log.len();
    let (u, h) = sim.in_standby(normal(id));
    sim.wake(h, cmd(1));
    sim.run();
    sim.bring_up(u);
    sim.log[from..].to_vec()
}

#[test]
fn g_over_budget_selects_lru_normal() {
    let mut sim = Sim::new(&Profile::target());
    let v = actives(&mut sim, &["a", "b", "c"]);
    let acts = one_more(&mut sim, "d");
    assert_eq!(queried(&acts), vec![v[0].1], "least recently used first");
    sim.resources(v[0].1, false);
    assert_eq!(
        sim.st(v[0].0).then(),
        Some(Then::Standby),
        "same path as idle standby"
    );
}

#[test]
fn g_over_budget_never_selects_busy() {
    let mut sim = Sim::new(&Profile::target());
    let v = actives(&mut sim, &["a", "b", "c"]);
    sim.begin_work(v[0].1);
    let acts = one_more(&mut sim, "d");
    assert_eq!(queried(&acts), vec![v[1].1]);
}

#[test]
fn g_busy_answer_keeps_unit_and_budget_overruns() {
    let mut sim = Sim::new(&Profile::target());
    let v = actives(&mut sim, &["a", "b", "c"]);
    one_more(&mut sim, "d");
    let acts = sim.resources(v[0].1, true);
    assert!(
        terminates(&acts).is_empty(),
        "busy (live resource) is never put in standby"
    );
    assert_eq!(sim.st(v[0].0), S::Active(v[0].1));
}

#[test]
fn g_never_selects_resident() {
    let mut sim = Sim::new(&Profile::target());
    let (_, r) = sim.in_active(resident("r")); // counted, oldest
    sim.after(1_000);
    let v = actives(&mut sim, &["a", "b"]);
    let acts = one_more(&mut sim, "d");
    let q = queried(&acts);
    assert!(!q.contains(&r));
    assert_eq!(q, vec![v[0].1]);
}

#[test]
fn g_builtins_are_outside_the_budget() {
    let mut sim = Sim::new(&Profile::target());
    for i in 0..5 {
        sim.in_active(builtin(&format!("b{i}")));
    }
    actives(&mut sim, &["a", "b"]);
    let acts = one_more(&mut sim, "c");
    assert!(
        queried(&acts).is_empty(),
        "3 third-party processes fit a budget of 3"
    );
}

#[test]
fn g_overrun_when_none_qualifies() {
    let mut sim = Sim::new(&Profile::target());
    for id in ["r1", "r2", "r3"] {
        sim.in_active(resident(id));
    }
    let acts = one_more(&mut sim, "d");
    assert!(queried(&acts).iter().all(|p| {
        sim.unit_of(*p)
            .is_some_and(|u| sim.class(u) == Class::Normal)
    }));
    assert!(
        spawns(&acts).len() == 1,
        "the start still happens (soft budget)"
    );
}

#[test]
fn g_event_wake_never_causes_lru_standby() {
    let mut sim = Sim::new(&Profile::target());
    actives(&mut sim, &["a", "b", "c"]);
    let from = sim.log.len();
    let (u, h) = sim.in_standby(normal("d"));
    sim.wake(h, event("playback.state", 1));
    sim.run();
    if matches!(sim.st(u), S::Spawn(_)) {
        sim.bring_up(u);
    }
    assert!(queried(&sim.log[from..]).is_empty());
}

#[test]
fn g_low_pressure_standby_for_all_idle_normal_units() {
    let mut p = Profile::target();
    p.budget = usize::MAX; // pressure alone, not the budget
    let mut sim = Sim::new(&p);
    let v = actives(&mut sim, &["a", "b"]);
    let (_, busy) = sim.in_active(normal("w"));
    sim.begin_work(busy);
    let (_, r) = sim.in_active(resident("r"));
    let (_, b) = sim.in_active(builtin_standby("bi"));
    let (_, d) = sim.in_active(dev("d", 120_000));
    let q = queried(&sim.pressure_low());
    assert_eq!(q.len(), 2, "{q:?}");
    for pid in [v[0].1, v[1].1] {
        assert!(q.contains(&pid));
    }
    for pid in [busy, r, b, d] {
        assert!(!q.contains(&pid));
    }
}

// Importance: recently used −100 → go to standby later ANSWERED (supervisor.md), was a design question:
// "recently used" has no window in the design. This only checks the order (older first), which holds under any
// window.
#[test]
fn g_recently_used_go_later() {
    let mut sim = Sim::new(&Profile::target());
    let (_, a) = sim.in_active(normal("a"));
    let (_, b) = sim.in_active(normal("b"));
    sim.after(5_000);
    sim.touch(a); // a used just now, b earlier
    assert_eq!(queried(&sim.pressure_low()), vec![b, a]);
}
