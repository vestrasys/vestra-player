//! Merge / supersede table, design §4.2: one test per cell
//! (`m_<installed row>_<new request>`), plus job mode `Fail` ("busy").
//! The merge rules do not depend on tunables, so these run with B1 values.
use super::*;

fn sim() -> Sim {
    sim_with(&Profile::b1())
}

/// The cells whose answers per held trigger (A9) or carried backlog (§4.3)
/// arrive with B3 (§17.7 item 5) run under `Profile::b3()`, with a
/// `…_today` twin under `b1()`.
fn sim_with(p: &Profile) -> Sim {
    let mut s = Sim::new(p);
    s.settle();
    s
}

/// Row `Start [waiting]`: unit in Standby, a command wake installed a Start
/// job that has not run yet. The backlog holds command 1.
fn start_waiting(sim: &mut Sim) -> (UnitId, Pid) {
    let (u, holder) = sim.in_standby(normal("a"));
    sim.settle();
    assert!(matches!(sim.wake(holder, cmd(1)), Delivery::Started));
    assert!(matches!(sim.job(u), Some(J::Start { running: false, .. })));
    (u, holder)
}

/// Row `Start [running: Activating]`: as above, after `run_jobs`.
fn start_running(sim: &mut Sim) -> (UnitId, Pid, Pid) {
    let (u, holder) = start_waiting(sim);
    sim.run();
    let S::Spawn(pid) = sim.st(u) else {
        panic!("{:?}", sim.st(u))
    };
    (u, holder, pid)
}

/// Row `Standby [waiting for resources answer]`.
fn standby_waiting(sim: &mut Sim) -> (UnitId, Pid) {
    let (u, pid) = sim.in_active(normal("a"));
    let q = sim.after(sim.p.idle);
    assert!(q.iter().any(|a| query_pid(a) == Some(pid)));
    assert!(matches!(sim.job(u), Some(J::Standby { .. })));
    (u, pid)
}

/// Row `Standby [running: Deactivating{then: Standby}]`.
fn standby_running(sim: &mut Sim) -> (UnitId, Pid) {
    sim.deactivating_to_standby(normal("a"))
}

/// Row `Stop [waiting]` (enqueued, `run_jobs` not called yet).
fn stop_waiting(sim: &mut Sim, reason: StopReason, then: Then) -> (UnitId, Pid) {
    let (u, pid) = sim.in_active(normal("a"));
    sim.stop(u, reason, then).unwrap();
    assert!(matches!(sim.job(u), Some(J::Stop { running: false, .. })));
    (u, pid)
}

/// Row `Stop [running: Deactivating]`.
fn stop_running(sim: &mut Sim, reason: StopReason, then: Then) -> (UnitId, Pid) {
    let (u, pid) = stop_waiting(sim, reason, then);
    sim.run();
    assert!(sim.st(u).is_deactivating());
    (u, pid)
}

/// Row `Restart [any]` (waiting on an Active unit).
fn restart_installed(sim: &mut Sim) -> (UnitId, Pid) {
    let (u, pid) = sim.in_active(normal("a"));
    sim.restart(u, false, None).unwrap();
    (u, pid)
}

// --- (none) ---------------------------------------------------------------

#[test]
fn m_none_start_on_active_is_done_and_delivers_now() {
    let mut sim = sim();
    let (u, pid) = sim.in_active(normal("a"));
    assert!(matches!(sim.wake(pid, cmd(5)), Delivery::Now));
    let id = sim.start_interactive(u).unwrap();
    assert_eq!(sim.job(u), None, "done at once");
    assert_eq!(sim.result(id), Some(R::Done));
}

#[test]
fn m_none_standby_installs_standby() {
    let mut sim = sim();
    let (u, _) = sim.in_active(normal("a"));
    sim.standby(u);
    assert!(matches!(sim.job(u), Some(J::Standby { .. })));
}

#[test]
fn m_none_stop_installs_stop() {
    let mut sim = sim();
    let (u, _) = sim.in_active(normal("a"));
    sim.stop(u, StopReason::Disable, Then::Inactive).unwrap();
    assert!(matches!(
        sim.job(u),
        Some(J::Stop {
            then: Then::Inactive,
            ..
        })
    ));
}

#[test]
fn m_none_restart_installs_restart() {
    let mut sim = sim();
    let (u, _) = sim.in_active(normal("a"));
    sim.restart(u, true, None).unwrap();
    assert!(matches!(sim.job(u), Some(J::Restart { fresh: true, .. })));
}

// --- Start [waiting] ------------------------------------------------------

#[test]
fn m_start_waiting_start_merges_max_priority_and_appends() {
    let mut sim = sim();
    let (u, holder) = sim.in_standby(normal("a"));
    sim.settle();
    sim.wake(holder, event("playback.state", 1)); // Event priority
    assert!(matches!(sim.wake(holder, cmd(2)), Delivery::Held)); // Interactive
    assert_eq!(sim.backlog_len(u), 2);
    assert!(
        matches!(sim.job(u), Some(J::Start { ref cause, .. }) if cause.contains("interactive")),
        "priority = max: {:?}",
        sim.job(u)
    );
}

#[test]
fn m_start_waiting_standby_is_dropped() {
    let mut sim = sim();
    let (u, _) = start_waiting(&mut sim);
    sim.standby(u);
    assert_eq!(sim.job(u).map(|j| j.kind()), Some("Start"));
}

#[test]
fn m_start_waiting_stop_replaces_and_answers_backlog() {
    let mut sim = sim_with(&Profile::b3());
    let (u, _) = start_waiting(&mut sim);
    sim.stop(u, StopReason::Disable, Then::Inactive).unwrap();
    assert_eq!(sim.job(u).map(|j| j.kind()), Some("Stop"));
    let acts = sim.run();
    assert_eq!(answers(&acts).len(), 1, "backlog answered (§4.6)");
    assert!(spawns(&acts).is_empty());
}

#[test]
fn m_start_waiting_stop_replaces_and_drops_backlog_today() {
    let mut sim = sim();
    let (u, _) = start_waiting(&mut sim);
    sim.stop(u, StopReason::Disable, Then::Inactive).unwrap();
    assert_eq!(sim.job(u).map(|j| j.kind()), Some("Stop"));
    let acts = sim.run();
    assert!(answers(&acts).is_empty(), "today: dropped silently");
    assert!(spawns(&acts).is_empty());
    assert_eq!(sim.backlog_len(u), 0);
}

#[test]
fn m_start_waiting_restart_keeps_backlog() {
    let mut sim = sim();
    let (u, _) = start_waiting(&mut sim);
    sim.restart(u, true, None).unwrap();
    assert_eq!(sim.job(u).map(|j| j.kind()), Some("Restart"));
    assert_eq!(sim.backlog_len(u), 1);
}

// --- Start [running: Activating] -----------------------------------------

#[test]
fn m_start_running_start_merges() {
    let mut sim = sim();
    let (u, holder, _) = start_running(&mut sim);
    assert!(matches!(sim.wake(holder, cmd(2)), Delivery::Held));
    assert!(matches!(sim.job(u), Some(J::Start { running: true, .. })));
    assert_eq!(sim.backlog_len(u), 2);
}

#[test]
fn m_start_running_standby_is_dropped() {
    let mut sim = sim();
    let (u, _, _) = start_running(&mut sim);
    sim.standby(u);
    assert!(matches!(sim.job(u), Some(J::Start { running: true, .. })));
}

#[test]
fn m_start_running_stop_terminates_activating_and_answers() {
    let mut sim = sim_with(&Profile::b3());
    let (u, _, pid) = start_running(&mut sim);
    sim.spawned(pid, true);
    sim.stop(u, StopReason::Disable, Then::Inactive).unwrap();
    let acts = sim.run();
    assert_eq!(sim.job(u).map(|j| j.kind()), Some("Stop"));
    assert_eq!(
        terminates(&acts).iter().map(|t| t.3).collect::<Vec<_>>(),
        vec![pid]
    );
    assert_eq!(answers(&acts), vec![(1, Why::Disabled)]);
}

#[test]
fn m_start_running_stop_terminates_activating_today_answers_nothing() {
    let mut sim = sim();
    let (u, _, pid) = start_running(&mut sim);
    sim.spawned(pid, true);
    sim.stop(u, StopReason::Disable, Then::Inactive).unwrap();
    let acts = sim.run();
    assert_eq!(sim.job(u).map(|j| j.kind()), Some("Stop"));
    assert_eq!(
        terminates(&acts).iter().map(|t| t.3).collect::<Vec<_>>(),
        vec![pid]
    );
    assert!(answers(&acts).is_empty(), "today: dropped silently");
}

#[test]
fn m_start_running_restart_terminates_then_starts_again_keeping_backlog() {
    let mut sim = sim_with(&Profile::b3());
    let (u, _, pid) = start_running(&mut sim);
    sim.spawned(pid, true);
    sim.restart(u, true, None).unwrap();
    let acts = sim.run();
    assert_eq!(terminates(&acts).len(), 1);
    assert!(answers(&acts).is_empty(), "backlog kept");
    let acts = sim.gone(pid);
    let new = spawns(&acts);
    assert_eq!(new.len(), 1);
    sim.spawned(new[0], true);
    assert_eq!(
        deliveries(&sim.ready(new[0])),
        vec![1],
        "held trigger carried to the new run"
    );
}

#[test]
fn m_start_running_restart_today_drops_backlog_at_the_new_start() {
    let mut sim = sim();
    let (u, _, pid) = start_running(&mut sim);
    sim.spawned(pid, true);
    sim.restart(u, true, None).unwrap();
    let acts = sim.run();
    assert_eq!(terminates(&acts).len(), 1);
    assert!(answers(&acts).is_empty());
    let acts = sim.gone(pid);
    let new = spawns(&acts);
    assert_eq!(new.len(), 1);
    assert!(answers(&acts).is_empty(), "today: dropped silently");
    sim.spawned(new[0], true);
    assert!(
        deliveries(&sim.ready(new[0])).is_empty(),
        "today the held trigger is lost with lazy.waking"
    );
}

// --- Standby [waiting for resources answer] ------------------------------

#[test]
fn m_standby_waiting_start_replaces_and_delivers_now() {
    let mut sim = sim();
    let (u, pid) = standby_waiting(&mut sim);
    assert!(matches!(sim.wake(pid, cmd(2)), Delivery::Now));
    assert_eq!(sim.job(u), None, "Standby canceled");
    // The late "not busy" answer must not stop it any more.
    let acts = sim.resources(pid, false);
    assert!(terminates(&acts).is_empty());
    assert_eq!(sim.st(u), S::Active(pid));
}

#[test]
fn m_standby_waiting_standby_stays() {
    let mut sim = sim();
    let (u, _) = standby_waiting(&mut sim);
    sim.standby(u);
    assert!(matches!(sim.job(u), Some(J::Standby { .. })));
}

#[test]
fn m_standby_waiting_stop_replaces() {
    let mut sim = sim();
    let (u, _) = standby_waiting(&mut sim);
    sim.stop(u, StopReason::Disable, Then::Inactive).unwrap();
    assert!(matches!(sim.job(u), Some(J::Stop { .. })));
}

#[test]
fn m_standby_waiting_restart_replaces() {
    let mut sim = sim();
    let (u, _) = standby_waiting(&mut sim);
    sim.restart(u, true, None).unwrap();
    assert!(matches!(sim.job(u), Some(J::Restart { .. })));
}

// --- Standby [running: Deactivating{then: Standby}] ----------------------

#[test]
fn m_standby_running_start_waits_for_reap_then_starts_from_standby() {
    let mut sim = sim();
    let (u, pid) = standby_running(&mut sim);
    sim.wake(pid, cmd(2));
    assert!(matches!(sim.job(u), Some(J::Start { running: false, .. })));
    assert!(spawns(&sim.reaped(pid)).is_empty());
    let acts = sim.cleaned(pid);
    let new = spawns(&acts);
    assert_eq!(
        new.len(),
        1,
        "runs from Standby once the old process is gone"
    );
    assert!(acts.iter().any(|a| rekey(a) == Some((pid, new[0]))));
}

#[test]
fn m_standby_running_standby_stays() {
    let mut sim = sim();
    let (u, _) = standby_running(&mut sim);
    sim.standby(u);
    assert_eq!(sim.st(u).then(), Some(Then::Standby));
    assert!(!matches!(
        sim.job(u),
        Some(J::Start { .. } | J::Stop { .. } | J::Restart { .. })
    ));
}

// §4.2 cell as amended by the coordinator ruling (§17.7 item 6): the kept
// registrations are revoked at the stop, not again at reap, in both presets.
both! {
    fn m_standby_running_stop_changes_then_and_revokes_at_once(p) {
        let mut sim = sim_with(p);
        let (u, pid) = standby_running(&mut sim);
        sim.stop(u, StopReason::Disable, Then::Inactive).unwrap();
        let acts = sim.run();
        assert_eq!(sim.st(u).then(), Some(Then::Inactive));
        assert_eq!(count(&acts, "Revoke"), 1, "revoked at the stop: {:?}", names(&acts));
        let acts = sim.gone(pid);
        assert!(!has(&acts, "Revoke"), "not again at reap: {:?}", names(&acts));
        assert_eq!(sim.st(u), S::Inactive);
    }
}

#[test]
fn m_standby_running_restart_waits_then_applies_fresh() {
    let mut sim = sim();
    let (u, pid) = standby_running(&mut sim);
    sim.restart(u, true, None).unwrap();
    assert!(spawns(&sim.run()).is_empty());
    let acts = sim.gone(pid);
    assert!(has(&acts, "DropCheckpoint"));
    assert_eq!(spawns(&acts).len(), 1);
}

// --- Stop [waiting] -------------------------------------------------------

// ANSWERED (supervisor.md §17.2 Q12), was a design question: which Stop reasons are *not* "for disable/uninstall" in
// this cell (Start replaces the Stop)? Best reading: any other reason, e.g.
// a `Stop{Failed}` enqueued internally, or a cancel.
#[test]
fn m_stop_waiting_start_replaces_on_active_unit() {
    let mut sim = sim();
    let (u, pid) = stop_waiting(&mut sim, StopReason::Failed, Then::Failed);
    sim.start_interactive(u).unwrap();
    let acts = sim.run();
    assert!(
        terminates(&acts).is_empty(),
        "nothing to do: the unit is Active"
    );
    assert_eq!(sim.st(u), S::Active(pid));
    assert_eq!(sim.job(u), None);
}

#[test]
fn m_stop_waiting_disable_refuses_trigger_start() {
    let mut sim = sim();
    let (u, pid) = sim.in_active(normal("a"));
    // §17.2: the disable RPC sets the unit-file state first.
    sim.set_enabled(u, false);
    sim.stop(u, StopReason::Disable, Then::Inactive).unwrap();
    assert!(
        matches!(sim.wake(pid, cmd(2)), Delivery::Dropped),
        "a trigger never re-enables"
    );
    assert!(matches!(sim.job(u), Some(J::Stop { .. })));
}

#[test]
fn m_stop_waiting_uninstall_refuses_trigger_start() {
    let mut sim = sim();
    let (u, pid) = stop_waiting(&mut sim, StopReason::Uninstall, Then::Removed);
    let d = sim.wake(pid, cmd(2));
    assert!(matches!(d, Delivery::Dropped), "{d:?}");
    assert!(matches!(
        sim.job(u),
        Some(J::Stop {
            then: Then::Removed,
            ..
        })
    ));
}

#[test]
fn m_stop_waiting_standby_keeps_stop() {
    let mut sim = sim();
    let (u, _) = stop_waiting(&mut sim, StopReason::Disable, Then::Inactive);
    sim.standby(u);
    assert!(matches!(sim.job(u), Some(J::Stop { .. })));
}

#[test]
fn m_stop_waiting_stop_stronger_then_wins() {
    // Removed > Failed > Inactive, whichever order they arrive in.
    for (a, b, want) in [
        (Then::Inactive, Then::Removed, Then::Removed),
        (Then::Removed, Then::Inactive, Then::Removed),
        (Then::Inactive, Then::Failed, Then::Failed),
        (Then::Failed, Then::Removed, Then::Removed),
    ] {
        let mut sim = sim();
        let (u, _) = stop_waiting(&mut sim, StopReason::Disable, a);
        sim.stop(u, StopReason::Uninstall, b).unwrap();
        assert!(
            matches!(sim.job(u), Some(J::Stop { then, .. }) if then == want),
            "{a:?} + {b:?}: {:?}",
            sim.job(u)
        );
    }
}

#[test]
fn m_stop_waiting_restart_on_enabled_unit_becomes_restart() {
    let mut sim = sim();
    let (u, _) = stop_waiting(&mut sim, StopReason::Failed, Then::Failed);
    sim.restart(u, false, None).unwrap();
    assert!(matches!(sim.job(u), Some(J::Restart { .. })));
}

// "else refused": §17.2 — a refused request gets its own id, result `Dropped`.
#[test]
fn m_stop_waiting_restart_on_disabled_unit_is_refused() {
    let mut sim = sim();
    let (u, _) = sim.in_active(normal("a"));
    sim.set_enabled(u, false);
    sim.stop(u, StopReason::Disable, Then::Inactive).unwrap();
    let id = sim.restart(u, false, None).unwrap();
    assert_eq!(sim.result(id), Some(R::Dropped));
    assert!(matches!(sim.job(u), Some(J::Stop { .. })));
}

// --- Stop [running: Deactivating] -----------------------------------------

#[test]
fn m_stop_running_start_becomes_restart_waiting_for_reap() {
    let mut sim = sim();
    let (u, pid) = stop_running(&mut sim, StopReason::Failed, Then::Failed);
    sim.start_interactive(u).unwrap();
    assert!(
        matches!(
            sim.job(u),
            Some(J::Restart { .. }) | Some(J::Start { running: false, .. })
        ),
        "{:?}",
        sim.job(u)
    );
    assert!(spawns(&sim.reaped(pid)).is_empty());
    assert_eq!(spawns(&sim.cleaned(pid)).len(), 1);
}

// ANSWERED (supervisor.md §17.2 Q14), was a design question: this cell says a Start on a running *disable* Stop is
// refused, while named case "enable while disabling" says the Start waits
// for reap. Best reading: a trigger Start is refused (`Dropped`); a
// `Start{Enable}` after `set_enabled(true)` waits (see named_cases.rs).
#[test]
fn m_stop_running_disable_refuses_trigger_start() {
    let mut sim = sim();
    let (u, pid) = stop_running(&mut sim, StopReason::Disable, Then::Inactive);
    let d = sim.wake(pid, cmd(2));
    assert!(matches!(d, Delivery::Dropped), "{d:?}");
    sim.gone(pid);
    assert_eq!(sim.st(u), S::Inactive);
    assert!(spawns(&sim.run()).is_empty());
}

#[test]
fn m_stop_running_standby_keeps_stop() {
    let mut sim = sim();
    let (u, _) = stop_running(&mut sim, StopReason::Disable, Then::Inactive);
    sim.standby(u);
    assert!(matches!(sim.job(u), Some(J::Stop { .. })));
}

#[test]
fn m_stop_running_stop_merges_stronger_then() {
    let mut sim = sim();
    let (u, pid) = stop_running(&mut sim, StopReason::Disable, Then::Inactive);
    sim.stop(u, StopReason::Uninstall, Then::Removed).unwrap();
    sim.run();
    assert_eq!(sim.st(u).then(), Some(Then::Removed));
    let acts = sim.gone(pid);
    assert!(has(&acts, "RemoveData"));
}

#[test]
fn m_stop_running_restart_waits() {
    let mut sim = sim();
    let (u, pid) = stop_running(&mut sim, StopReason::Failed, Then::Failed);
    sim.restart(u, true, None).unwrap();
    assert!(matches!(sim.job(u), Some(J::Restart { .. })));
    assert!(spawns(&sim.reaped(pid)).is_empty());
    assert_eq!(spawns(&sim.cleaned(pid)).len(), 1);
}

// --- Restart [any] --------------------------------------------------------

#[test]
fn m_restart_start_is_absorbed_and_trigger_held() {
    let mut sim = sim();
    let (u, pid) = restart_installed(&mut sim);
    let d = sim.wake(pid, cmd(3));
    assert!(matches!(d, Delivery::Held), "{d:?}");
    assert!(matches!(sim.job(u), Some(J::Restart { .. })));
    let acts = sim.run();
    assert!(deliveries(&acts).is_empty(), "held for the new run");
}

#[test]
fn m_restart_standby_is_absorbed() {
    let mut sim = sim();
    let (u, _) = restart_installed(&mut sim);
    sim.standby(u);
    assert!(matches!(sim.job(u), Some(J::Restart { .. })));
}

#[test]
fn m_restart_stop_replaces_before_stop_half() {
    let mut sim = sim();
    let (u, _) = restart_installed(&mut sim);
    sim.stop(u, StopReason::Disable, Then::Inactive).unwrap();
    assert!(matches!(sim.job(u), Some(J::Stop { .. })));
}

#[test]
fn m_restart_stop_while_stop_half_runs_continues_as_stop() {
    let mut sim = sim();
    let (u, pid) = restart_installed(&mut sim);
    sim.run(); // stop half running
    assert!(sim.st(u).is_deactivating());
    sim.stop(u, StopReason::Disable, Then::Inactive).unwrap();
    let acts = sim.gone(pid);
    assert!(spawns(&acts).is_empty(), "Start half canceled");
    assert_eq!(sim.st(u), S::Inactive);
}

#[test]
fn m_restart_restart_ors_fresh_and_takes_newest_image() {
    let mut sim = sim();
    let (u, _) = sim.in_active(normal("a"));
    sim.restart(u, true, Some("1.0.0")).unwrap();
    sim.restart(u, false, Some("1.1.0")).unwrap();
    assert_eq!(
        sim.job(u).map(|j| match j {
            J::Restart { fresh, image, .. } => (fresh, image),
            other => panic!("{other:?}"),
        }),
        Some((true, Some("1.1.0".to_string())))
    );
}

// --- Job mode Fail ("busy") -----------------------------------------------

#[test]
fn m_mode_fail_rpc_on_rpc_job_is_busy() {
    let mut sim = sim();
    let (u, _) = sim.in_active(normal("a"));
    sim.rpc_restart(u, true, None).unwrap();
    assert!(
        sim.rpc_stop(u, StopReason::Disable, Then::Inactive)
            .is_err(),
        "busy"
    );
    assert!(sim.rpc_restart(u, true, None).is_err(), "busy");
    assert!(
        sim.rpc_stop(u, StopReason::Uninstall, Then::Removed)
            .is_err(),
        "uninstall too: 'all but another package RPC'"
    );
    assert!(
        matches!(sim.job(u), Some(J::Restart { .. })),
        "first RPC job untouched"
    );
}

#[test]
fn m_mode_fail_rpc_on_internal_job_follows_table() {
    let mut sim = sim();
    let (u, _) = start_waiting(&mut sim);
    assert!(
        sim.rpc_stop(u, StopReason::Disable, Then::Inactive).is_ok(),
        "a trigger job has no PackageRpc origin"
    );
    assert!(matches!(sim.job(u), Some(J::Stop { .. })));
}

#[test]
fn m_mode_fail_busy_ends_when_rpc_job_finishes() {
    let mut sim = sim();
    let (u, pid) = sim.in_active(normal("a"));
    sim.rpc_restart(u, true, None).unwrap();
    sim.run();
    sim.gone(pid);
    sim.bring_up(u);
    assert_eq!(sim.job(u), None);
    assert!(sim.rpc_restart(u, true, None).is_ok());
}
