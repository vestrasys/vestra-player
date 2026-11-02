//! Named cases, design §4.3: one end-to-end test per row.
use super::*;

// start + start: one Start; higher priority kept; triggers accumulate
#[test]
fn nc_start_plus_start_one_job_triggers_accumulate() {
    let mut sim = Sim::new(&Profile::b1());
    let (u, holder) = sim.in_standby(normal("a"));
    sim.settle();
    sim.wake(holder, event("playback.state", 1));
    sim.wake(holder, cmd(2));
    sim.wake(holder, cmd(3));
    assert_eq!(sim.backlog_len(u), 3);
    let acts = sim.run();
    assert_eq!(spawns(&acts).len(), 1, "one Start job, one process");
    let pid = sim.pid(u);
    sim.spawned(pid, true);
    let mut got = deliveries(&sim.ready(pid));
    got.sort();
    assert_eq!(got, vec![1, 2, 3]);
}

// start, then stop: Stop replaces; held triggers answered
#[test]
fn nc_start_then_stop_answers_held_triggers() {
    let mut sim = Sim::new(&Profile::b1());
    let (u, holder) = sim.in_standby(normal("a"));
    sim.settle();
    sim.wake(holder, cmd(1));
    sim.wake(holder, resolve(2));
    sim.stop(u, StopReason::Disable, Then::Inactive).unwrap();
    let acts = sim.run();
    let mut got = answers(&acts)
        .into_iter()
        .map(|(p, _)| p)
        .collect::<Vec<_>>();
    got.sort();
    assert_eq!(got, vec![1, 2]);
    assert!(spawns(&acts).is_empty());
}

// stop, then start: Restart; start waits for reap + backend cleaned (≤ 20 s)
both! {
    fn nc_stop_then_start_waits_for_old_process(p) {
        let mut sim = Sim::new(p);
        let (u, pid) = sim.in_active(normal("a"));
        sim.stop(u, StopReason::Failed, Then::Failed).unwrap();
        sim.run();
        sim.start_interactive(u).unwrap();
        assert!(spawns(&sim.after(p.stop_grace)).is_empty());
        assert!(spawns(&sim.reaped(pid)).is_empty());
        assert_eq!(spawns(&sim.cleaned(pid)).len(), 1);
    }
}

// restart + stop: Stop wins
#[test]
fn nc_restart_plus_stop_stop_wins() {
    let mut sim = Sim::new(&Profile::b1());
    let (u, pid) = sim.in_active(normal("a"));
    sim.restart(u, true, None).unwrap();
    sim.stop(u, StopReason::Disable, Then::Inactive).unwrap();
    sim.run();
    let acts = sim.gone(pid);
    assert!(spawns(&acts).is_empty());
    assert_eq!(sim.st(u), S::Inactive);
}

// reload while activating: Restart{fresh}: Terminate{Reload} the activating
// process, wait for reap, drop checkpoint, re-verify built-in, start; held
// triggers carried to the new run
#[test]
fn nc_reload_while_activating_carries_triggers() {
    let mut sim = Sim::new(&Profile::b1());
    let (u, holder) = sim.in_standby(normal("a"));
    sim.settle();
    sim.wake(holder, cmd(1));
    sim.run();
    let old = sim.pid(u);
    sim.spawned(old, true);
    sim.rpc_restart(u, true, None).unwrap();
    let acts = sim.run();
    let t = terminates(&acts);
    assert_eq!(t.len(), 1);
    assert_eq!((t[0].2.as_str(), t[0].3), ("reload", old));
    assert!(answers(&acts).is_empty(), "held triggers kept");
    let acts = sim.gone(old);
    assert!(has(&acts, "DropCheckpoint"));
    let new = spawns(&acts);
    assert_eq!(new.len(), 1);
    sim.spawned(new[0], true);
    assert_eq!(deliveries(&sim.ready(new[0])), vec![1]);
}

#[test]
fn nc_reload_while_activating_reverifies_builtin() {
    let mut sim = Sim::new(&Profile::b1());
    let u = sim.add(builtin("b").shared(1).verify());
    sim.run();
    sim.verified(u, true);
    let old = sim.pid(u);
    sim.spawned(old, true);
    sim.restart(u, true, None).unwrap();
    sim.run();
    let acts = sim.gone(old);
    assert!(
        has(&acts, "Verify"),
        "fresh re-verifies a built-in: {:?}",
        names(&acts)
    );
    assert_eq!(sim.st(u), S::Verify);
}

// upgrade while in standby: Restart{fresh, image: new}: Revoke + DropCheckpoint
// at once, then Start of the new image if enabled (first run)
#[test]
fn nc_upgrade_while_in_standby_revokes_and_starts_new_image() {
    let mut sim = Sim::new(&Profile::b1());
    let (u, _) = sim.in_standby(normal("a"));
    sim.rpc_restart(u, true, Some("2.0.0")).unwrap();
    let acts = sim.run();
    assert!(
        has(&acts, "Revoke") && has(&acts, "DropCheckpoint"),
        "{:?}",
        names(&acts)
    );
    assert_eq!(spawns(&acts).len(), 1, "first run of the new image");
}

#[test]
fn nc_upgrade_while_standby_stop_still_deactivating_waits() {
    let mut sim = Sim::new(&Profile::b1());
    let (u, pid) = sim.deactivating_to_standby(normal("a"));
    sim.rpc_restart(u, true, Some("2.0.0")).unwrap();
    let acts = sim.run();
    assert!(spawns(&acts).is_empty());
    assert_eq!(spawns(&sim.gone(pid)).len(), 1);
}

#[test]
fn nc_upgrade_of_disabled_unit_in_standby_does_not_start() {
    let mut sim = Sim::new(&Profile::b1());
    let (u, _) = sim.in_standby(normal("a"));
    sim.set_enabled(u, false);
    // ANSWERED (supervisor.md §17.2 Q15), was a design question: §4.2 says Restart on a disabled unit is "refused"
    // (Stop row) while this case says the new image starts only "if
    // enabled". Best reading: the upgrade job is accepted, drops the
    // checkpoint, and does not start.
    let _ = sim.rpc_restart(u, true, Some("2.0.0"));
    let acts = sim.run();
    assert!(spawns(&acts).is_empty());
}

// wake while deactivating: Start waits for reap; if not gone in 20 s, fails
// with "its stopped process did not go within 20 s"; interactive answered
both! {
    fn nc_wake_while_deactivating_waits_then_runs(p) {
        let mut sim = Sim::new(p);
        let (_, pid) = sim.deactivating_to_standby(normal("a"));
        sim.wake(pid, cmd(1));
        assert!(spawns(&sim.run()).is_empty());
        let acts = sim.gone(pid);
        let new = spawns(&acts);
        assert_eq!(new.len(), 1);
        sim.spawned(new[0], true);
        assert_eq!(deliveries(&sim.ready(new[0])), vec![1]);
    }
}

#[test]
fn nc_wake_while_deactivating_fails_after_20s() {
    let mut sim = Sim::new(&Profile::b1());
    let (u, pid) = sim.deactivating_to_standby(normal("a"));
    let t = sim.now;
    sim.wake(pid, cmd(1));
    assert!(answers(&sim.at(t + STOP_WAIT - 1)).is_empty());
    let acts = sim.at(t + STOP_WAIT);
    assert_eq!(answers(&acts), vec![(1, Why::CouldNotStart)]);
    assert!(spawns(&acts).is_empty());
    assert_eq!(sim.job(u), None);
}

// disable while waking: Stop{Disable, then: Inactive} replaces the Start;
// Terminate{Disable} the activating process; backlog answered "is disabled"
#[test]
fn nc_disable_while_waking() {
    let mut sim = Sim::new(&Profile::b1());
    let (u, holder) = sim.in_standby(normal("a"));
    sim.settle();
    sim.wake(holder, cmd(1));
    sim.run();
    let pid = sim.pid(u);
    sim.set_enabled(u, false);
    sim.rpc_stop(u, StopReason::Disable, Then::Inactive)
        .unwrap();
    let acts = sim.run();
    assert_eq!(
        terminates(&acts)
            .iter()
            .map(|t| (t.2.clone(), t.3))
            .collect::<Vec<_>>(),
        vec![("disable".into(), pid)]
    );
    assert_eq!(answers(&acts), vec![(1, Why::Disabled)]);
    sim.gone(pid);
    assert_eq!(sim.st(u), S::Inactive);
}

// enable while disabling: Start waits for reap (no second instance)
both! {
    fn nc_enable_while_disabling_waits_for_reap(p) {
        let mut sim = Sim::new(p);
        let (u, pid) = sim.in_active(normal("a"));
        // The user's disable (origin User, §17.1) — not a package RPC, so a
        // following enable is not "busy" (as today, §17.2 Q16).
        sim.user_disable(u).unwrap();
        sim.run();
        // The enable must wait for the old process (D13), not start a second.
        sim.set_enabled(u, true);
        sim.start(u, Cause::Enable).unwrap();
        assert!(spawns(&sim.after(p.stop_grace)).is_empty());
        assert!(spawns(&sim.reaped(pid)).is_empty());
        let acts = sim.cleaned(pid);
        assert!(has(&acts, "Spawn") || has(&acts, "Replay"));
    }
}

// uninstall while anything: Stop{then: Removed} supersedes all but another package RPC
#[test]
fn nc_uninstall_supersedes_internal_jobs() {
    let mut sim = Sim::new(&Profile::b1());
    let (u, holder) = sim.in_standby(normal("a"));
    sim.settle();
    sim.wake(holder, cmd(1));
    assert!(
        sim.rpc_stop(u, StopReason::Uninstall, Then::Removed)
            .is_ok()
    );
    let acts = sim.run();
    assert!(has(&acts, "RemoveData"));
    assert_eq!(answers(&acts).len(), 1);
}

#[test]
fn nc_uninstall_during_package_rpc_is_busy() {
    let mut sim = Sim::new(&Profile::b1());
    let (u, _) = sim.in_active(normal("a"));
    sim.rpc_restart(u, false, Some("2.0.0")).unwrap();
    assert!(
        sim.rpc_stop(u, StopReason::Uninstall, Then::Removed)
            .is_err()
    );
}

// trigger while Failed: Dropped
#[test]
fn nc_trigger_while_failed_is_dropped() {
    let mut sim = Sim::new(&Profile::b1());
    let (u, pid) = sim.in_active(normal("a"));
    sim.exited(pid, ExitCause::Exited);
    sim.gone(pid);
    assert!(sim.st(u).is_failed());
    assert!(matches!(sim.wake(pid, cmd(1)), Delivery::Dropped));
    assert_eq!(sim.job(u), None);
}

// trigger while Standby{hold_until}: Start waits for hold_until; interactive answered (T20)
// Coordinator ruling (2026-11-02): an Interactive trigger is answered at
// once with the reason and not held; a Start waiting for hold_until is
// installed only while event triggers remain in the backlog.
fn held_unit(sim: &mut Sim) -> (UnitId, Pid, Ms) {
    let (u, pid) = sim.in_active(normal("a"));
    sim.settle();
    sim.exited(pid, ExitCause::Exited);
    sim.gone(pid);
    let S::Standby {
        holder,
        hold_until: Some(until),
    } = sim.st(u)
    else {
        panic!("{:?}", sim.st(u))
    };
    (u, holder, until)
}

#[test]
fn nc_trigger_while_holding_interactive_is_answered_not_held() {
    let mut sim = Sim::new(&Profile::target());
    let (u, holder, until) = held_unit(&mut sim);
    sim.wake(holder, cmd(1));
    assert_eq!(answers(&sim.run()), vec![(1, Why::Held)]);
    assert_eq!(sim.backlog_len(u), 0, "not held");
    assert_eq!(sim.job(u), None, "no Start for an answered trigger");
    assert!(spawns(&sim.at(until)).is_empty());
}

#[test]
fn nc_trigger_while_holding_event_waits_for_hold_until() {
    let mut sim = Sim::new(&Profile::target());
    let (u, holder, until) = held_unit(&mut sim);
    sim.wake(holder, event("playback.state", 2));
    assert!(answers(&sim.run()).is_empty());
    assert!(matches!(sim.job(u), Some(J::Start { .. })));
    assert!(spawns(&sim.at(until - 1)).is_empty());
    assert_eq!(spawns(&sim.at(until)).len(), 1);
}

// runner restart: one Stop (runner group) for all members, then Restart jobs per member; first immediate
#[test]
fn nc_runner_restart_one_group_stop_then_per_member_restart() {
    let mut sim = Sim::new(&Profile::target());
    let (a, pa) = sim.in_active(builtin("a").shared(7));
    let (b, pb) = sim.in_active(builtin("b").shared(7));
    let (c, pc) = sim.in_active(builtin("c")); // own process, not in the runner
    let acts = sim.runner_hung(7);
    assert_eq!(count(&acts, "KillRunner"), 1);
    assert!(terminates(&acts).iter().all(|t| t.3 != pc));
    assert_eq!(
        sim.st(c),
        S::Active(pc),
        "an own-process built-in is unaffected"
    );
    sim.cleaned(pa);
    sim.cleaned(pb);
    assert!(matches!(sim.st(a), S::Spawn(_)));
    assert!(matches!(sim.st(b), S::Spawn(_)));
}

#[test]
fn nc_runner_restart_members_with_checkpoint_go_to_standby() {
    let mut sim = Sim::new(&Profile::target());
    let (a, pa) = sim.in_active(builtin_standby("a").shared(7));
    sim.runner_hung(7);
    let acts = sim.cleaned(pa);
    assert!(
        spawns(&acts).is_empty(),
        "no spawn for a member with a checkpoint (§7.4)"
    );
    assert!(has(&sim.log, "Replay"));
    assert!(matches!(sim.st(a), S::Standby { .. }));
}
