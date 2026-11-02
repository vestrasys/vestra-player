//! Backlog answers and bounds, design §4.6 and §4.4 "Bounds" (A9): only
//! wake events merge; commands, callbacks, drops and resolves are kept one
//! by one and answered on cancel, failure or overflow; wake events are
//! dropped silently.
use super::*;

fn woken_unit(sim: &mut Sim) -> (UnitId, Pid) {
    let (u, holder) = sim.in_standby(normal("a"));
    sim.settle();
    (u, holder)
}

fn deliver_all(sim: &mut Sim, u: UnitId) -> Vec<u32> {
    sim.run();
    let pid = sim.pid(u);
    sim.spawned(pid, true);
    deliveries(&sim.ready(pid))
}

#[test]
fn b_wake_events_same_name_merge_newest_wins() {
    let mut sim = Sim::new(&Profile::b1());
    let (u, h) = woken_unit(&mut sim);
    sim.wake(h, event("playback.state", 1));
    sim.wake(h, event("playback.state", 2));
    sim.wake(h, event("player.fullscreen", 3));
    assert_eq!(sim.backlog_len(u), 2);
    let mut got = deliver_all(&mut sim, u);
    got.sort();
    assert_eq!(got, vec![2, 3]);
}

#[test]
fn b_commands_are_kept_one_by_one() {
    let mut sim = Sim::new(&Profile::b1());
    let (u, h) = woken_unit(&mut sim);
    sim.wake(h, cmd(1));
    sim.wake(h, cmd(1));
    assert_eq!(sim.backlog_len(u), 2);
    assert_eq!(deliver_all(&mut sim, u), vec![1, 1]);
}

#[test]
fn b_resolve_drop_callback_are_kept_in_order() {
    let mut sim = Sim::new(&Profile::b1());
    let (u, h) = woken_unit(&mut sim);
    sim.wake(h, resolve(1));
    sim.wake(h, drop_(2));
    sim.wake(h, callback(3));
    sim.wake(h, resolve(1));
    assert_eq!(deliver_all(&mut sim, u), vec![1, 2, 3, 1]);
}

// A9 answers per held trigger arrive with B3 (`answer_held`, §17.7 item 5):
// run under `Profile::b3()`; the `…_legacy` twin pins today's silence.
#[test]
fn b_cancel_answers_each_non_event_trigger_events_silent() {
    let mut sim = Sim::new(&Profile::b3());
    let (u, h) = woken_unit(&mut sim);
    sim.wake(h, cmd(1));
    sim.wake(h, resolve(2));
    sim.wake(h, drop_(3));
    sim.wake(h, callback(4));
    sim.wake(h, event("playback.state", 5));
    sim.set_enabled(u, false);
    sim.stop(u, StopReason::Disable, Then::Inactive).unwrap();
    let acts = sim.run();
    let mut got = answers(&acts);
    got.sort_by_key(|a| a.0);
    assert_eq!(
        got.iter().map(|a| a.0).collect::<Vec<_>>(),
        vec![1, 2, 3, 4],
        "event 5 dropped silently"
    );
    // commands get "<name> is disabled"; the others a rejected/dropped reply.
    assert_eq!(got[0].1, Why::Disabled);
    assert_eq!(sim.backlog_len(u), 0);
}

// Today a canceled start drops its held triggers silently (`lazy.waking`
// is dropped; §17.7 item 5).
#[test]
fn b_cancel_legacy_drops_held_triggers_silently() {
    let mut sim = Sim::new(&Profile::b1());
    let (u, h) = woken_unit(&mut sim);
    sim.wake(h, cmd(1));
    sim.wake(h, resolve(2));
    sim.wake(h, event("playback.state", 3));
    sim.set_enabled(u, false);
    sim.stop(u, StopReason::Disable, Then::Inactive).unwrap();
    let acts = sim.run();
    assert!(answers(&acts).is_empty(), "{:?}", answers(&acts));
    assert!(deliveries(&acts).is_empty());
    assert!(spawns(&acts).is_empty());
    assert_eq!(sim.backlog_len(u), 0);
}

#[test]
fn b_start_failure_answers_could_not_start() {
    let mut sim = Sim::new(&Profile::b3());
    let (u, h) = woken_unit(&mut sim);
    sim.wake(h, cmd(1));
    sim.wake(h, event("playback.state", 2));
    sim.run();
    let acts = sim.spawned(sim.pid(u), false);
    assert_eq!(answers(&acts), vec![(1, Why::CouldNotStart)]);
}

// Today a wake's failure shows one "could not start" notice and answers
// no held trigger (`wake_failed`; §17.7 item 5).
#[test]
fn b_start_failure_legacy_notifies_once_and_answers_nothing() {
    let mut sim = Sim::new(&Profile::b1());
    let (u, h) = woken_unit(&mut sim);
    sim.wake(h, cmd(1));
    sim.wake(h, cmd(2));
    sim.wake(h, event("playback.state", 3));
    sim.run();
    let acts = sim.spawned(sim.pid(u), false);
    assert!(answers(&acts).is_empty(), "{:?}", answers(&acts));
    assert_eq!(
        acts.iter().filter(|a| is_could_not_start_notice(a)).count(),
        1,
        "{:?}",
        names(&acts)
    );
    assert_eq!(sim.backlog_len(u), 0);
}

#[test]
fn b_overflow_refuses_and_answers_busy() {
    let p = Profile::target();
    let mut sim = Sim::new(&p);
    let (u, h) = woken_unit(&mut sim);
    let max = p.backlog_max.unwrap();
    for i in 0..max {
        assert!(
            matches!(
                sim.wake(h, cmd(i as u32)),
                Delivery::Started | Delivery::Held
            ),
            "#{i}"
        );
    }
    assert_eq!(sim.backlog_len(u), max);
    assert!(matches!(sim.wake(h, cmd(999)), Delivery::Refused));
    assert_eq!(sim.backlog_len(u), max);
    let acts = sim.run();
    assert!(
        answers(&acts).contains(&(999, Why::Busy)),
        "{:?}",
        answers(&acts)
    );
}

#[test]
fn b_overflow_event_is_refused_silently() {
    let p = Profile::target();
    let mut sim = Sim::new(&p);
    let (_, h) = woken_unit(&mut sim);
    for i in 0..p.backlog_max.unwrap() {
        sim.wake(h, cmd(i as u32));
    }
    assert!(matches!(
        sim.wake(h, event("playback.state", 999)),
        Delivery::Refused
    ));
    assert!(!answers(&sim.run()).iter().any(|a| a.0 == 999));
}

#[test]
fn b_merged_event_does_not_count_twice_toward_bound() {
    let p = Profile::target();
    let mut sim = Sim::new(&p);
    let (u, h) = woken_unit(&mut sim);
    for i in 0..(p.backlog_max.unwrap() - 1) {
        sim.wake(h, cmd(i as u32));
    }
    sim.wake(h, event("playback.state", 1));
    assert!(
        !matches!(sim.wake(h, event("playback.state", 2)), Delivery::Refused),
        "replaces in place"
    );
    assert_eq!(sim.backlog_len(u), p.backlog_max.unwrap());
}

#[test]
fn b_b1_backlog_unbounded() {
    let mut sim = Sim::new(&Profile::b1());
    let (u, h) = woken_unit(&mut sim);
    for i in 0..500 {
        assert!(!matches!(sim.wake(h, cmd(i)), Delivery::Refused));
    }
    assert_eq!(sim.backlog_len(u), 500);
}

// Delivery::Dropped for a disabled unit (§5)
#[test]
fn b_wake_on_disabled_unit_is_dropped() {
    let mut sim = Sim::new(&Profile::b1());
    let (u, h) = woken_unit(&mut sim);
    sim.set_enabled(u, false);
    sim.stop(u, StopReason::Disable, Then::Inactive).unwrap();
    sim.run();
    assert!(matches!(sim.wake(h, cmd(1)), Delivery::Dropped));
}
