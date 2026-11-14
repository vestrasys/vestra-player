//! The trigger rules T1–T15 (events-and-triggers.md §4.1) on the registry;
//! T2 and T11 with playback in motion are also in `triggers_sim.rs`, T8 and
//! T9 are the SDK's (its tests).
use super::*;
use serde_json::json;

const P: &str = "zysilm.test";
const NOW: f64 = 1_800_000_000_000.;

fn ready(paused: bool, fullscreen: bool) -> Facts {
    Facts::new(
        &json!({"phase":"ready","paused":paused,"stamp":{"generation":1}}),
        &json!({"fullscreen":fullscreen}),
        "vestra",
        "system",
    )
}
fn named(name: &str) -> TriggerId {
    TriggerId::Named(name.into())
}
fn edges(out: &[Notification]) -> Vec<(&'static str, Option<u64>)> {
    out.iter()
        .map(|n| {
            (
                n.edge.as_str(),
                n.range.as_ref().map(|r| r["index"].as_u64().unwrap()),
            )
        })
        .collect()
}
fn playing_fullscreen() -> Value {
    json!({"all":[{"phase":["ready"]},{"paused":false},{"fullscreen":true}]})
}
/// A registry at a ready, paused, windowed source 1 at position 0.
fn registry() -> Triggers {
    let mut triggers = Triggers::new(None);
    triggers.set_facts(ready(true, false), 1, Some(0.), NOW);
    triggers
}

#[test]
fn t1_enter_once_when_it_turns_true_and_leave_once_when_false() {
    let mut t = registry();
    let (_, out) = t
        .set(P, 5, TriggerId::Unnamed(0), &playing_fullscreen(), NOW)
        .unwrap();
    assert!(out.is_empty());
    assert!(
        t.set_facts(ready(false, false), 1, Some(0.), NOW)
            .is_empty()
    );
    assert_eq!(
        edges(&t.set_facts(ready(false, true), 1, Some(0.), NOW)),
        [("enter", None)]
    );
    // It stays true: nothing repeats.
    assert!(t.set_facts(ready(false, true), 1, Some(1.), NOW).is_empty());
    let mut muted = ready(false, true);
    muted.muted = true;
    assert!(t.set_facts(muted, 1, Some(1.), NOW).is_empty());
    assert_eq!(
        edges(&t.set_facts(ready(true, true), 1, Some(1.), NOW)),
        [("leave", None)]
    );
    assert!(t.set_facts(ready(true, false), 1, Some(1.), NOW).is_empty());
}

#[test]
fn t2_a_range_passed_between_two_checks_is_entered_and_left_once() {
    let mut t = registry();
    t.set_facts(ready(false, false), 1, Some(9.), NOW);
    let when = json!({"position":{"ranges":[[10, 10.004],[10.004, 10.01],[12, 13]]}});
    let (_, out) = t.set(P, 5, TriggerId::Unnamed(0), &when, NOW).unwrap();
    assert!(out.is_empty());
    assert_eq!(t.watch(), (1, vec![10.]));
    // One continuous report at 10.02: both short ranges were passed.
    let out = t.position(1, 10.02, true, NOW);
    assert_eq!(
        edges(&out),
        [
            ("enter", Some(0)),
            ("leave", Some(0)),
            ("enter", Some(1)),
            ("leave", Some(1))
        ]
    );
    // Each at the place it was passed.
    assert_eq!(out[0].position, Some(10.));
    assert_eq!(out[3].position, Some(10.01));
    assert_eq!(t.watch(), (1, vec![12.]));
    // A jump over the third (not continuous): nothing.
    assert!(t.position(1, 14., false, NOW).is_empty());
}

#[test]
fn t3_a_condition_already_true_when_given_enters_at_once() {
    let mut t = registry();
    t.set_facts(ready(false, true), 1, Some(0.), NOW);
    let (seq, out) = t
        .set(P, 5, TriggerId::Unnamed(0), &playing_fullscreen(), NOW)
        .unwrap();
    assert_eq!(edges(&out), [("enter", None)]);
    assert_eq!(out[0].seq, seq);
    // A time condition already due, too (the sleep timer the next morning).
    let (_, out) = t
        .set(P, 5, named("sleep"), &json!({"time":{"min":NOW - 1.}}), NOW)
        .unwrap();
    assert_eq!(edges(&out), [("enter", None)]);
}

#[test]
fn t4_the_condition_stays_until_off_or_another() {
    let mut t = registry();
    t.set(P, 5, TriggerId::Unnamed(0), &playing_fullscreen(), NOW)
        .unwrap();
    for _ in 0..3 {
        assert_eq!(
            edges(&t.set_facts(ready(false, true), 1, Some(0.), NOW)),
            [("enter", None)]
        );
        assert_eq!(
            edges(&t.set_facts(ready(true, true), 1, Some(0.), NOW)),
            [("leave", None)]
        );
    }
    assert_eq!(t.kept(P).len(), 1);
}

#[test]
fn t5_the_same_condition_again_changes_nothing() {
    let mut t = registry();
    t.set_facts(ready(false, true), 1, Some(0.), NOW);
    let (seq, out) = t
        .set(P, 5, TriggerId::Unnamed(0), &playing_fullscreen(), NOW)
        .unwrap();
    assert_eq!(out.len(), 1);
    // `activate()` again (a wake): the same seq, still entered, no enter.
    let (again, out) = t
        .set(P, 6, TriggerId::Unnamed(0), &playing_fullscreen(), NOW)
        .unwrap();
    assert_eq!((again, out.len()), (seq, 0));
    assert!(t.kept(P)[0].3);
}

#[test]
fn t6_another_condition_replaces_it_without_leave_and_with_a_new_seq() {
    let mut t = registry();
    t.set_facts(ready(false, true), 1, Some(0.), NOW);
    let (first, _) = t
        .set(P, 5, TriggerId::Unnamed(0), &playing_fullscreen(), NOW)
        .unwrap();
    // Replaced by one that is false: no leave; true again: enter (T3).
    let (second, out) = t
        .set(P, 5, TriggerId::Unnamed(0), &json!({"paused":true}), NOW)
        .unwrap();
    assert!(second > first && out.is_empty());
    let (third, out) = t
        .set(P, 5, TriggerId::Unnamed(0), &json!({"paused":false}), NOW)
        .unwrap();
    assert!(third > second);
    assert_eq!(edges(&out), [("enter", None)]);
    assert_eq!(out[0].seq, third);
    // `off` gets one too.
    assert!(t.off(P, &TriggerId::Unnamed(0)) > third);
}

#[test]
fn t7_off_removes_it_without_leave() {
    let mut t = registry();
    t.set_facts(ready(false, true), 1, Some(0.), NOW);
    t.set(P, 5, TriggerId::Unnamed(0), &playing_fullscreen(), NOW)
        .unwrap();
    t.off(P, &TriggerId::Unnamed(0));
    assert!(t.kept(P).is_empty());
    assert!(t.set_facts(ready(true, false), 1, Some(0.), NOW).is_empty());
}

#[test]
fn t10_from_one_range_straight_into_another_and_the_earliest_start() {
    let mut t = registry();
    t.set_facts(ready(false, false), 1, Some(5.), NOW);
    let when = json!({"position":{"ranges":[[20, 30],[10, 20],[15, 25]]}});
    t.set(P, 5, TriggerId::Unnamed(0), &when, NOW).unwrap();
    assert_eq!(edges(&t.position(1, 10., true, NOW)), [("enter", Some(1))]);
    // [15, 25) starts later than [10, 20): still the first.
    assert!(t.position(1, 15., true, NOW).is_empty());
    assert_eq!(
        edges(&t.position(1, 20., true, NOW)),
        [("leave", Some(1)), ("enter", Some(2))]
    );
    let out = t.position(1, 30., true, NOW);
    assert_eq!(
        edges(&out),
        [("leave", Some(2)), ("enter", Some(0)), ("leave", Some(0))]
    );
    assert_eq!(out[1].range, Some(json!({"start":20.,"end":30.,"index":0})));
}

#[test]
fn t11_a_jump_into_a_range_is_an_enter_and_out_a_leave() {
    let mut t = registry();
    t.set(
        P,
        5,
        TriggerId::Unnamed(0),
        &json!({"position":{"ranges":[[10, 20]],"lead":2}}),
        NOW,
    )
    .unwrap();
    assert_eq!(edges(&t.position(1, 8.5, false, NOW)), [("enter", Some(0))]);
    assert_eq!(edges(&t.position(1, 25., false, NOW)), [("leave", Some(0))]);
    assert_eq!(edges(&t.position(1, 15., false, NOW)), [("enter", Some(0))]);
    assert_eq!(edges(&t.position(1, 1., false, NOW)), [("leave", Some(0))]);
}

#[test]
fn t12_a_range_list_belongs_to_the_source_it_was_given_for() {
    let mut t = registry();
    t.set_facts(ready(false, false), 1, Some(15.), NOW);
    let when = json!({"position":{"ranges":[[10, 20]]}});
    let (_, out) = t.set(P, 5, TriggerId::Unnamed(0), &when, NOW).unwrap();
    assert_eq!(edges(&out), [("enter", Some(0))]);
    // Another source: false there, whatever the position.
    assert_eq!(
        edges(&t.set_facts(ready(false, false), 2, Some(15.), NOW)),
        [("leave", Some(0))]
    );
    assert!(t.position(2, 15., true, NOW).is_empty());
    assert_eq!(t.watch(), (0, vec![]));
    // The same list given again for this source binds it here: a new seq.
    let (_, out) = t.set(P, 5, TriggerId::Unnamed(0), &when, NOW).unwrap();
    assert_eq!(edges(&out), [("enter", Some(0))]);
}

#[test]
fn t13_a_fault_forgets_entered_a_standby_does_not() {
    let mut t = registry();
    t.set_facts(ready(false, true), 1, Some(0.), NOW);
    t.set(P, 5, TriggerId::Unnamed(0), &playing_fullscreen(), NOW)
        .unwrap();
    // Standby is nothing here: entered stays; a fault enters again.
    assert_eq!(edges(&t.fault(P, NOW)), [("enter", None)]);
    assert!(t.fault("other.plugin", NOW).is_empty());
}

#[test]
fn t13_a_player_restart_keeps_the_condition_not_entered_nor_a_range_list() {
    let dir = tempfile::tempdir().unwrap();
    let mut t = Triggers::new(Some(dir.path().into()));
    t.load(P, Some("guid-1".into()));
    t.set_facts(ready(false, true), 1, Some(15.), NOW);
    t.set(P, 5, TriggerId::Unnamed(0), &playing_fullscreen(), NOW)
        .unwrap();
    t.set(
        P,
        5,
        named("skip"),
        &json!({"position":{"ranges":[[10, 20]]}}),
        NOW,
    )
    .unwrap();
    t.declare(P, 5, &["skip".into()], 1);
    let written = std::fs::read_to_string(dir.path().join("guid-1.json")).unwrap();
    assert!(!written.contains("ranges"), "{written}");
    assert!(!written.contains("entered"), "{written}");
    // The next start: read back; nothing entered; evaluated when the mute
    // ends: enter for what is true.
    let mut next = Triggers::new(Some(dir.path().into()));
    next.load(P, Some("guid-1".into()));
    let kept = next.kept(P);
    assert_eq!(kept.len(), 1);
    assert_eq!((&kept[0].0, kept[0].3), (&TriggerId::Unnamed(0), false));
    assert!(next.evaluate_all(NOW).is_empty());
    assert_eq!(
        edges(&next.set_facts(ready(false, true), 1, Some(0.), NOW)),
        [("enter", None)]
    );
    assert!(next.evaluate_all(NOW).is_empty());
    // Sequence numbers go on after the file's.
    let (seq, _) = next
        .set(P, 6, named("x"), &json!({"paused":true}), NOW)
        .unwrap();
    assert!(seq > kept[0].2);
}

#[test]
fn t14_a_name_not_declared_and_another_count_of_unnamed_go() {
    let mut t = registry();
    t.set(P, 5, named("a"), &json!({"paused":true}), NOW)
        .unwrap();
    t.set(P, 5, named("b"), &json!({"paused":true}), NOW)
        .unwrap();
    t.set(P, 5, TriggerId::Unnamed(0), &json!({"paused":true}), NOW)
        .unwrap();
    t.set(P, 5, TriggerId::Unnamed(1), &json!({"paused":true}), NOW)
        .unwrap();
    t.declare(P, 5, &["a".into(), "b".into()], 2);
    assert_eq!(t.kept(P).len(), 4);
    // The next run (same version) declares "a" only, and one unnamed
    // trigger, which it gives again.
    t.set(P, 6, TriggerId::Unnamed(0), &json!({"paused":true}), NOW)
        .unwrap();
    t.declare(P, 6, &["a".into()], 1);
    let ids: Vec<_> = t.kept(P).into_iter().map(|k| k.0).collect();
    assert_eq!(ids, [TriggerId::Unnamed(0), named("a")]);
    let told = t.diagnostics().join("\n");
    assert!(told.contains("no longer declares trigger \"b\""), "{told}");
    assert!(told.contains("#1"), "{told}");
    // Nobody to handle a notification (the SDK says so): removed, logged.
    t.unhandled(P, &named("a"));
    assert_eq!(t.kept(P).len(), 1);
    assert!(t.diagnostics()[0].contains("has no enter or leave"));
}

#[test]
fn t15_nothing_limits_how_often_a_condition_flips() {
    let mut t = registry();
    // Always true: accepted, entered at once.
    let (_, out) = t
        .set(
            P,
            5,
            TriggerId::Unnamed(0),
            &json!({"not":{"phase":["error"]}}),
            NOW,
        )
        .unwrap();
    assert_eq!(out.len(), 1);
    t.set(P, 5, TriggerId::Unnamed(1), &json!({"paused":true}), NOW)
        .unwrap();
    let mut flips = 0;
    for i in 1..=1000 {
        flips += t
            .set_facts(ready(i % 2 == 0, false), 1, Some(0.), NOW)
            .len();
    }
    assert_eq!(flips, 1000);
}

#[test]
fn the_caps_64_triggers_and_1000_ranges() {
    let mut t = registry();
    for place in 0..64 {
        t.set(
            P,
            5,
            TriggerId::Unnamed(place),
            &json!({"paused":true}),
            NOW,
        )
        .unwrap();
    }
    let refusal = t
        .set(P, 5, named("one-more"), &json!({"paused":true}), NOW)
        .unwrap_err();
    assert_eq!(refusal.code, "limit");
    // Replacing a kept one is no new trigger.
    assert!(
        t.set(P, 5, TriggerId::Unnamed(3), &json!({"paused":false}), NOW)
            .is_ok()
    );
    assert_eq!(TriggerId::parse(&json!(64)).unwrap_err().code, "limit");
    assert!(TriggerId::parse(&json!("")).is_err());
    assert!(TriggerId::parse(&json!("x".repeat(65))).is_err());
    let many: Vec<_> = (0..1001).map(|i| json!([i, i as f64 + 0.5])).collect();
    let refusal = t
        .set(
            P,
            5,
            TriggerId::Unnamed(0),
            &json!({"position":{"ranges":many}}),
            NOW,
        )
        .unwrap_err();
    assert_eq!(refusal.code, "limit");
}

#[test]
fn an_upgrade_clears_unnamed_and_keeps_named() {
    let dir = tempfile::tempdir().unwrap();
    let v1 = Package {
        digest: "d1".into(),
        version: Some("1.0.0".into()),
    };
    let v2 = Package {
        digest: "d2".into(),
        version: Some("1.1.0".into()),
    };
    let mut t = Triggers::new(Some(dir.path().into()));
    t.load(P, Some("guid-1".into()));
    t.spawned(P, Some(v1.clone()));
    t.set(
        P,
        5,
        named("sleep"),
        &json!({"time":{"min":NOW + 60_000.}}),
        NOW,
    )
    .unwrap();
    t.set(
        P,
        5,
        TriggerId::Unnamed(0),
        &json!({"time":{"min":NOW + 60_000.}}),
        NOW,
    )
    .unwrap();
    t.declare(P, 5, &["sleep".into()], 1);
    // A standby and a wake of the same version keep both.
    t.spawned(P, Some(v1.clone()));
    assert_eq!(t.kept(P).len(), 2);
    // The user updates the plugin: unnamed cleared, named kept.
    t.spawned(P, Some(v2.clone()));
    assert_eq!(
        t.kept(P).into_iter().map(|k| k.0).collect::<Vec<_>>(),
        [named("sleep")]
    );
    assert!(t.diagnostics()[0].contains("another version"));
    // Kept through a player restart too.
    let mut next = Triggers::new(Some(dir.path().into()));
    next.load(P, Some("guid-1".into()));
    next.spawned(P, Some(v2));
    assert_eq!(next.kept(P).len(), 1);
    // A development reload is another version.
    next.set(P, 7, TriggerId::Unnamed(0), &json!({"paused":true}), NOW)
        .unwrap();
    next.reloaded(P);
    assert_eq!(next.kept(P).len(), 1);
    // Disable or uninstall: everything, and the file.
    next.clear(P);
    assert!(next.kept(P).is_empty());
    assert!(!dir.path().join("guid-1.json").exists());
}

#[test]
fn time_conditions_are_due_at_their_bound_and_when_the_clock_goes_back() {
    let mut t = registry();
    let at = NOW + 45. * 60_000.;
    t.set(P, 5, named("sleep"), &json!({"time":{"min":at}}), NOW)
        .unwrap();
    assert_eq!(t.next_time(NOW), Some(at));
    assert!(t.time(at - 1.).is_empty());
    assert_eq!(edges(&t.time(at + 10.)), [("enter", None)]);
    assert_eq!(t.next_time(at + 10.), None);
    // The system clock set back an hour: evaluated again, false: leave.
    assert_eq!(edges(&t.time(at - 3_600_000.)), [("leave", None)]);
    // A source change evaluates other conditions with the clock too, but
    // never skips a time bound passed meanwhile.
    t.set(
        P,
        5,
        named("sleep"),
        &json!({"time":{"min":NOW + 1000.}}),
        NOW,
    )
    .unwrap();
    t.time(NOW);
    t.set_facts(ready(false, false), 2, Some(0.), NOW + 2000.);
    assert_eq!(edges(&t.time(NOW + 2000.)), [("enter", None)]);
}

#[test]
fn notifications_carry_the_trigger_seq_edge_state_and_range() {
    let mut t = registry();
    t.set_facts(ready(false, false), 1, Some(3.), NOW);
    t.set(
        P,
        5,
        named("skip"),
        &json!({"position":{"ranges":[[6, 9]],"lead":2}}),
        NOW,
    )
    .unwrap();
    let out = t.position(1, 4., true, NOW);
    // The state carries the position the flip was seen at.
    let data = out[0].data(&json!({"phase":"ready","position":3.9}));
    assert_eq!(
        data,
        json!({"trigger":"skip","seq":out[0].seq,"edge":"enter",
            "state":{"phase":"ready","position":4.},
            "range":{"start":6.,"end":9.,"index":0}})
    );
    assert_eq!(out[0].merge_key(), "triggers.fired \"skip\" enter");
    assert_eq!(
        TriggerId::Unnamed(3).wire(),
        json!(3),
        "an unnamed trigger is its place"
    );
}

/// A6 at 2× (Silence Skipper, the first silence): the model reported the
/// lead boundary 38.15 (enter); the next `source.state`, taken a push
/// earlier, says 38.13 and changes another source fact. That stale position
/// is no step back: no `leave`, and the model's next report enters nothing
/// again (T1: one enter per real edge).
#[test]
fn a_stale_state_position_behind_a_report_is_no_leave() {
    let mut t = registry();
    let mut facts = ready(false, false);
    facts.rate = 2.;
    t.set_facts(facts.clone(), 1, Some(37.9), NOW);
    let when = json!({"all":[{"phase":["ready"]},{"paused":false},
        {"position":{"ranges":[[40.15, 43.87325]],"lead":2}}]});
    t.set(P, 5, TriggerId::Unnamed(0), &when, NOW).unwrap();
    assert_eq!(
        edges(&t.position(1, 38.15, true, NOW)),
        [("enter", Some(0))]
    );
    // Another source fact changes in a push read just before the report.
    let mut changed = facts.clone();
    changed.volume = 50.;
    assert!(t.set_facts(changed.clone(), 1, Some(38.13), NOW).is_empty());
    assert!(t.position(1, 38.52, true, NOW).is_empty());
    // A real step back (a seek) is still a leave: far behind, or reported
    // by the model as a jump.
    changed.volume = 60.;
    assert_eq!(
        edges(&t.set_facts(changed.clone(), 1, Some(30.), NOW)),
        [("leave", Some(0))]
    );
    assert_eq!(
        edges(&t.position(1, 38.5, false, NOW)),
        [("enter", Some(0))]
    );
    assert_eq!(
        edges(&t.position(1, 38.0, false, NOW)),
        [("leave", Some(0))]
    );
    // A new source forgets the report.
    assert!(t.set_facts(changed, 2, Some(38.4), NOW).is_empty());
}

/// A notification the Host could not deliver (its backlog full, its plugin
/// without a process) is not lost behind "entered": `undelivered`
/// un-latches it with a diagnostic line, so the next evaluation fires it
/// again, and the next `triggers.declare` of the plugin evaluates it
/// (`redeliver`). A lost `leave` latches it again the same way.
#[test]
fn an_undelivered_notification_fires_again_at_the_next_evaluation() {
    let mut t = registry();
    t.set(P, 5, TriggerId::Unnamed(0), &playing_fullscreen(), NOW)
        .unwrap();
    let out = t.set_facts(ready(false, true), 1, Some(0.), NOW);
    assert_eq!(edges(&out), [("enter", None)]);
    let data = out[0].data(&json!({}));
    t.undelivered(P, &data, "its backlog or the start lane is full");
    let told = t.diagnostics().join("\n");
    assert!(told.contains("#0 enter not delivered"), "{told}");
    assert!(!t.kept(P)[0].3, "un-latched");
    // The next evaluation (another fact of the group changed) enters again.
    let mut muted = ready(false, true);
    muted.muted = true;
    assert_eq!(
        edges(&t.set_facts(muted.clone(), 1, Some(1.), NOW)),
        [("enter", None)]
    );
    assert!(
        t.redeliver(P, NOW).is_empty(),
        "delivered: nothing to retry"
    );
    // Lost again: the next declaration evaluates it with nothing changed.
    let out = t.set_facts(ready(true, true), 1, Some(1.), NOW);
    assert_eq!(edges(&out), [("leave", None)]);
    let out = t.set_facts(ready(false, true), 1, Some(1.), NOW);
    assert_eq!(edges(&out), [("enter", None)]);
    t.undelivered(P, &out[0].data(&json!({})), "it has no process to take it");
    t.declare(P, 6, &[], 1);
    assert_eq!(edges(&t.redeliver(P, NOW)), [("enter", None)]);
    assert!(t.redeliver(P, NOW).is_empty());
    // A lost leave: latched again, so the next evaluation leaves again.
    let out = t.set_facts(ready(true, true), 1, Some(1.), NOW);
    assert_eq!(edges(&out), [("leave", None)]);
    t.undelivered(
        P,
        &out[0].data(&json!({})),
        "its plugin is disabled, failed or stopping",
    );
    assert!(t.kept(P)[0].3, "latched again");
    let mut paused_muted = ready(true, true);
    paused_muted.muted = true;
    assert_eq!(
        edges(&t.set_facts(paused_muted, 1, Some(1.), NOW)),
        [("leave", None)]
    );
}

/// An undelivered notification of a condition replaced since (another
/// `seq`) changes nothing; a range list's lost `enter` un-latches that
/// range only.
#[test]
fn an_undelivered_notification_of_a_replaced_condition_changes_nothing() {
    let mut t = registry();
    t.set_facts(ready(false, true), 1, Some(0.), NOW);
    let (_, out) = t
        .set(P, 5, TriggerId::Unnamed(0), &playing_fullscreen(), NOW)
        .unwrap();
    let stale = out[0].data(&json!({}));
    t.set(
        P,
        5,
        TriggerId::Unnamed(0),
        &json!({"fullscreen":true}),
        NOW,
    )
    .unwrap();
    assert!(t.kept(P)[0].3);
    t.undelivered(P, &stale, "its backlog or the start lane is full");
    assert!(t.kept(P)[0].3, "the new condition's latch stays");
    // A range list.
    let when = json!({"position":{"ranges":[[0, 5],[10, 20]]}});
    t.set_facts(ready(false, false), 1, Some(12.), NOW);
    let (_, out) = t.set(P, 5, named("r"), &when, NOW).unwrap();
    assert_eq!(edges(&out), [("enter", Some(1))]);
    t.undelivered(
        P,
        &out[0].data(&json!({})),
        "its backlog or the start lane is full",
    );
    assert_eq!(edges(&t.position(1, 13., false, NOW)), [("enter", Some(1))]);
}

/// A7 at 2× (Silence Skipper woken for its first silence): the run before
/// the wake had scanned two windows and gave a list of two ranges; the
/// enter came at the lead boundary 38.15. The woken run gives the list of
/// nine ranges it knows now (another condition, a new seq, T6) while the
/// position, 38.52, is still in the same first range: no second enter (T1),
/// and the seek out of it is one leave, of that range.
#[test]
fn a_replaced_list_still_in_the_same_range_is_not_entered_again() {
    let mut t = registry();
    let mut facts = ready(false, false);
    facts.rate = 2.;
    t.set_facts(facts.clone(), 1, Some(37.9), NOW);
    let list = |ranges: Value| {
        json!({"all":[{"all":[{"phase":["ready"]},{"paused":false}]},
            {"position":{"ranges":ranges,"lead":2}}]})
    };
    let two = list(json!([[40.15, 43.87325], [110.150104, 112.850021]]));
    let nine = list(json!([
        [40.15, 43.87325],
        [110.150104, 112.850021],
        [185.15, 188.850021],
        [215.15, 217.850042],
        [235.15, 237.384687],
        [255.15, 257.850021],
        [275.14991699999996, 276.85006200000004],
        [295.15, 298.850021],
        [370.15, 372.850021]
    ]));
    let (first, _) = t.set(P, 18, TriggerId::Unnamed(0), &two, NOW).unwrap();
    let out = t.position(1, 38.16, true, NOW);
    assert_eq!(edges(&out), [("enter", Some(0))]);
    // The woken run's source.state says 38.52; its list has seven more.
    t.set_facts(facts.clone(), 1, Some(38.52), NOW);
    let (second, out) = t.set(P, 20, TriggerId::Unnamed(0), &nine, NOW).unwrap();
    assert!(second > first);
    assert!(out.is_empty(), "{:?}", edges(&out));
    // The seek lands at the range's end: one leave of the range entered.
    assert_eq!(
        edges(&t.position(1, 43.87325, false, NOW)),
        [("leave", Some(0))]
    );
}

/// The carry-over is only for the same range of the same source: a list
/// whose range the position is in changed (another end), a range the
/// position is in only in the new list, and a new source enter at once
/// (T3, T6, T12).
#[test]
fn a_replaced_list_in_another_range_is_entered_at_once() {
    let mut t = registry();
    t.set_facts(ready(false, false), 1, Some(15.), NOW);
    let list = |ranges: Value| json!({"position":{"ranges":ranges}});
    t.set(P, 5, TriggerId::Unnamed(0), &list(json!([[10, 20]])), NOW)
        .unwrap();
    // Another end: another range.
    let (_, out) = t
        .set(P, 5, TriggerId::Unnamed(0), &list(json!([[10, 25]])), NOW)
        .unwrap();
    assert_eq!(edges(&out), [("enter", Some(0))]);
    // Not entered before (a list it was outside of): entered now.
    let mut u = registry();
    u.set_facts(ready(false, false), 1, Some(15.), NOW);
    u.set(P, 5, TriggerId::Unnamed(0), &list(json!([[30, 40]])), NOW)
        .unwrap();
    let (_, out) = u
        .set(
            P,
            5,
            TriggerId::Unnamed(0),
            &list(json!([[10, 20], [30, 40]])),
            NOW,
        )
        .unwrap();
    assert_eq!(edges(&out), [("enter", Some(0))]);
    // The same list given for a new source: bound there, entered there.
    t.set_facts(ready(false, false), 2, Some(15.), NOW);
    let (_, out) = t
        .set(P, 5, TriggerId::Unnamed(0), &list(json!([[10, 25]])), NOW)
        .unwrap();
    assert_eq!(edges(&out), [("enter", Some(0))]);
}
