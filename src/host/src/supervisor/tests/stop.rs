//! Stop, exit, reap, design §7 (and T16–T18, T31): both halves (reaped +
//! backend cleaned) are required; 20 s stuck; `Kill` after
//! `TimeoutStopSec`; `kill(u)` escalates at once and is not a job;
//! shutdown is parallel with one deadline.
use super::*;

fn stopping(p: &Profile) -> (Sim, UnitId, Pid) {
    let mut sim = Sim::new(p);
    let (u, pid) = sim.in_active(normal("a"));
    sim.stop(u, StopReason::Disable, Then::Inactive).unwrap();
    sim.run();
    (sim, u, pid)
}

#[test]
fn s_reaped_alone_is_not_enough() {
    let (mut sim, u, pid) = stopping(&Profile::b1());
    sim.reaped(pid);
    assert!(sim.st(u).is_deactivating(), "{:?}", sim.st(u));
}

#[test]
fn s_backend_cleaned_alone_is_not_enough() {
    let (mut sim, u, pid) = stopping(&Profile::b1());
    sim.cleaned(pid);
    assert!(sim.st(u).is_deactivating(), "{:?}", sim.st(u));
}

#[test]
fn s_both_halves_in_either_order() {
    let (mut sim, u, pid) = stopping(&Profile::b1());
    sim.cleaned(pid);
    sim.reaped(pid);
    assert_eq!(sim.st(u), S::Inactive);
}

#[test]
fn s_reaped_moves_to_cleanup_substate() {
    let (mut sim, u, pid) = stopping(&Profile::b1());
    sim.reaped(pid);
    assert_eq!(sim.st(u), S::Cleanup(Then::Inactive));
}

both! {
    fn s_stuck_after_20s_without_either_half(p) {
        let (mut sim, u, _) = stopping(p);
        let t = sim.now;
        sim.at(t + STOP_WAIT - 1);
        assert!(!sim.stuck(u));
        sim.at(t + STOP_WAIT);
        assert!(sim.stuck(u));
        assert_eq!(sim.st(u), S::Inactive, "lands in `then`, flagged stuck");
    }
}

both! {
    fn s_kill_after_timeout_stop_sec(p) {
        let (mut sim, u, pid) = stopping(p);
        let t = sim.now;
        assert!(!has(&sim.at(t + p.stop_grace - 1), "Kill"));
        assert_eq!(sim.at(t + p.stop_grace).iter().filter_map(kill_pid).collect::<Vec<_>>(), vec![pid]);
        assert_eq!(sim.st(u), S::Kill(Then::Inactive));
        assert!(!has(&sim.after(10 * p.stop_grace), "Kill"), "Kill sent once");
    }
}

both! {
    fn s_no_kill_when_reaped_within_grace(p) {
        let (mut sim, _, pid) = stopping(p);
        sim.reaped(pid);
        assert!(!has(&sim.after(p.stop_grace), "Kill"));
    }
}

// kill(u) is not a job: escalates the current Deactivating process at once
#[test]
fn s_kill_escalates_now_and_is_not_a_job() {
    let (mut sim, u, pid) = stopping(&Profile::target());
    let job_before = sim.job(u);
    let acts = sim.kill(u);
    assert_eq!(
        acts.iter().filter_map(kill_pid).collect::<Vec<_>>(),
        vec![pid]
    );
    assert_eq!(sim.st(u), S::Kill(Then::Inactive));
    assert_eq!(
        sim.job(u),
        job_before,
        "kill does not install or change a job"
    );
}

#[test]
fn s_kill_on_active_unit_does_nothing() {
    let mut sim = Sim::new(&Profile::b1());
    let (u, pid) = sim.in_active(normal("a"));
    let acts = sim.kill(u);
    assert!(!has(&acts, "Kill"));
    assert_eq!(sim.st(u), S::Active(pid));
}

// Shutdown: checkpoints first (from the recorded state, no round-trip),
// parallel Terminate{Quit}, one deadline, then Kill
#[test]
fn s_shutdown_checkpoints_before_any_terminate() {
    let mut sim = Sim::new(&Profile::b1());
    sim.in_active(normal("a"));
    sim.in_active(normal("b"));
    let acts = sim.shutdown(1_000);
    let last_ck = acts
        .iter()
        .rposition(|a| act_name(a) == "Checkpoint")
        .expect("Checkpoint");
    let first_term = pos(&acts, "Terminate").expect("Terminate");
    assert!(last_ck < first_term, "{:?}", names(&acts));
    assert!(!has(&acts, "QueryResources"), "no round-trip to the plugin");
}

#[test]
fn s_shutdown_kills_only_unreaped_at_deadline() {
    let mut sim = Sim::new(&Profile::b1());
    let (_, a) = sim.in_active(normal("a"));
    let (_, b) = sim.in_active(normal("b"));
    let t = sim.now;
    sim.shutdown(1_000);
    sim.at(t + 200);
    sim.reaped(a);
    let killed = sim
        .at(t + 1_000)
        .iter()
        .filter_map(kill_pid)
        .collect::<Vec<_>>();
    assert_eq!(killed, vec![b]);
}

#[test]
fn s_shutdown_grace_is_the_single_budget() {
    for budget in [1_000, 300] {
        let mut sim = Sim::new(&Profile::target());
        sim.in_active(normal("a"));
        sim.in_active(builtin("b"));
        let acts = sim.shutdown(budget);
        assert!(
            terminates(&acts)
                .iter()
                .all(|t| t.0 <= budget && t.2 == "quit"),
            "{:?}",
            terminates(&acts)
        );
    }
}

#[test]
fn s_shutdown_does_not_terminate_units_without_a_process() {
    let mut sim = Sim::new(&Profile::b1());
    sim.in_standby(normal("s"));
    let (_, a) = sim.in_active(normal("a"));
    let acts = sim.shutdown(1_000);
    assert_eq!(
        terminates(&acts).iter().map(|t| t.3).collect::<Vec<_>>(),
        vec![a]
    );
}

#[test]
fn s_shutdown_also_kills_units_already_stopping() {
    let (mut sim, _, pid) = stopping(&Profile::target()); // 5 s grace
    let t = sim.now;
    sim.shutdown(300);
    assert_eq!(
        sim.at(t + 300)
            .iter()
            .filter_map(kill_pid)
            .collect::<Vec<_>>(),
        vec![pid],
        "one deadline for all"
    );
}
