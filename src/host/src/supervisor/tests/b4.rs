//! Restart-policy spec tests (exit causes, restart policy, backoff, start
//! limits, heap cap, OOM, shutdown ≤ 1 s), written from the design tables,
//! not from the core: the policy per class, a trigger while
//! `Standby{hold_until}`, the heap cap (supervisor.md §7, §8), and the
//! coordinator's ruling on holds (entries disabled with the reason while a
//! hold holds).
//!
//! Everything runs under `Profile::other_timers()`: start 15 s, stop
//! 300 ms, standby 2 s, idle 60 s, no request timeout, no process budget.
//! `policy.rs` and `transitions.rs` check the policy under
//! `Profile::target()`; these are the restart policy's own checks plus what
//! those files do not cover.
//!
//! Names: `r_` restart and backoff, `l_` start limits, `sr_` start retries,
//! `x_` exit causes and notices, `h_` the crash hold, `rn_` runner
//! restarts, `rd_` uninstall data removal, `q_` shutdown.
use super::*;

fn b4() -> Sim {
    Sim::new(&Profile::other_timers())
}

const START: Ms = 15_000;
const FAILURE_GRACE: Ms = 300;

/// Unexpected exit at `now`, reaped and cleaned at the same time.
fn crash(sim: &mut Sim, u: UnitId, cause: ExitCause) -> Vec<Act> {
    let pid = sim.pid(u);
    let mut acts = sim.exited(pid, cause);
    acts.extend(sim.gone(pid));
    acts
}

/// When the unit may start again after a crash: `AutoRestart{until}` or
/// `Standby{hold_until}`.
fn next_start(s: &S) -> Option<Ms> {
    match s {
        S::AutoRestart(t)
        | S::Standby {
            hold_until: Some(t),
            ..
        } => Some(*t),
        _ => None,
    }
}

/// Starts the unit again at the earliest allowed time and brings it up (a
/// Normal unit in its hold needs a start request; "on the next trigger").
fn back_up(sim: &mut Sim, u: UnitId) -> Pid {
    match sim.st(u) {
        S::Standby {
            hold_until: Some(t),
            ..
        } => {
            sim.start_interactive(u).unwrap();
            sim.at(t);
        }
        S::AutoRestart(t) => {
            sim.at(t);
        }
        other => panic!("not restartable: {other:?}"),
    }
    sim.bring_up(u).1
}

/// The delay of each of `n` crashes in a row, each brought back up at once.
fn delays(sim: &mut Sim, u: UnitId, cause: ExitCause, n: usize) -> Vec<Ms> {
    let mut got = Vec::new();
    for _ in 0..n {
        let now = sim.now;
        crash(sim, u, cause);
        let s = sim.st(u);
        got.push(next_start(&s).unwrap_or_else(|| panic!("no backoff in {s:?}")) - now);
        back_up(sim, u);
    }
    got
}

// ---------------------------------------------------------------------------
// Restart rule and restart delay per class
// ---------------------------------------------------------------------------

// by class: Builtin (own process, with or without a standby checkpoint) and
// Resident → Restart job (`AutoRestart`); Normal → `Standby{hold_until}`
// keeping its registrations; Development → `Failed`. First delay per class:
// Builtin 3 s (the runner's first restart is the only immediate one), Resident
// and Normal 1 s.
#[test]
fn r_crash_lands_per_class() {
    for cause in [
        ExitCause::Exited,
        ExitCause::Fault,
        ExitCause::Oom,
        ExitCause::RequestTimeout,
    ] {
        for (spec, delay, standby) in [
            (builtin("b"), 3_000, false),
            (builtin_standby("bs"), 3_000, false),
            (resident("r"), 1_000, false),
            (normal("n"), 1_000, true),
        ] {
            let mut sim = b4();
            let (u, pid) = sim.in_active(spec.clone());
            let t = sim.now;
            let acts = crash(&mut sim, u, cause);
            let want = if standby {
                S::Standby {
                    holder: pid,
                    hold_until: Some(t + delay),
                }
            } else {
                S::AutoRestart(t + delay)
            };
            assert_eq!(sim.st(u), want, "{cause:?} {spec:?}");
            assert!(
                spawns(&acts).is_empty(),
                "{cause:?} {spec:?}: not immediate"
            );
            if standby {
                assert!(
                    !has(&acts, "Revoke"),
                    "{cause:?}: Normal keeps its entry points"
                );
            }
        }
        let mut sim = b4();
        let (u, _) = sim.in_active(dev("d", 120_000));
        crash(&mut sim, u, cause);
        assert!(sim.st(u).is_failed(), "{cause:?} dev: {:?}", sim.st(u));
    }
}

// the restart delay × 2ⁿ: Builtin 3 s, 6 s, 12 s; Resident 1 s, 2 s, 4 s; Normal
// hold 1 s, 2 s, 4 s, 8 s.
#[test]
fn r_backoff_grows_per_class() {
    for (spec, want) in [
        (builtin("b"), vec![3_000, 6_000, 12_000]),
        (resident("r"), vec![1_000, 2_000, 4_000]),
        (normal("n"), vec![1_000, 2_000, 4_000, 8_000]),
    ] {
        let mut sim = b4();
        let (u, _) = sim.in_active(spec.clone());
        let got = delays(&mut sim, u, ExitCause::Exited, want.len());
        assert_eq!(got, want, "{spec:?}");
    }
}

// `AutoRestart{until}` and `Standby{hold_until}` = reap time + delay (the
// restart delay is armed once the stop finished), not exit time + delay.
#[test]
fn r_backoff_counts_from_the_reap() {
    for (spec, delay, standby) in [
        (builtin("b"), 3_000, false),
        (resident("r"), 1_000, false),
        (normal("n"), 1_000, true),
    ] {
        let mut sim = b4();
        let (u, pid) = sim.in_active(spec.clone());
        sim.exited(pid, ExitCause::Exited);
        sim.after(700);
        sim.reaped(pid);
        sim.after(100);
        let reap_done = sim.now;
        sim.cleaned(pid);
        let until = next_start(&sim.st(u)).unwrap_or_else(|| panic!("{:?}", sim.st(u)));
        // The process is gone once both halves are in: the delay counts from
        // there at the latest, never from the exit.
        assert!(
            until >= reap_done - 100 + delay,
            "{spec:?}: until {until}, exit {}, reap {}",
            reap_done - 800,
            reap_done - 100
        );
        assert!(until <= reap_done + delay, "{spec:?}: until {until}");
        assert_eq!(matches!(sim.st(u), S::Standby { .. }), standby, "{spec:?}");
    }
}

// the backoff counter resets after 60 s active, every class. The unit is
// touched half-way so the 60 s idle timeout (this profile's) does not start a
// Standby job in the middle.
#[test]
fn r_backoff_resets_after_60s_active() {
    for (spec, first) in [
        (builtin("b"), 3_000),
        (resident("r"), 1_000),
        (normal("n"), 1_000),
    ] {
        for (stable, want) in [(60_000, first), (59_999, 2 * first)] {
            let mut sim = b4();
            let (u, _) = sim.in_active(spec.clone());
            crash(&mut sim, u, ExitCause::Exited);
            let pid = back_up(&mut sim, u);
            sim.after(30_000);
            sim.touch(pid);
            sim.after(stable - 30_000);
            let now = sim.now;
            crash(&mut sim, u, ExitCause::Exited);
            let got = next_start(&sim.st(u)).unwrap() - now;
            assert_eq!(got, want, "{spec:?} after {stable} ms active");
        }
    }
}

// The restart delay is capped at 60 s (Normal hold). Reachable when
// the crashes are spread over more than 10 min (start limit) and every run
// lasts under 60 s (no reset): the unit waits in standby between them.
#[test]
fn r_normal_hold_caps_at_60s() {
    let mut sim = b4();
    let (u, _) = sim.in_active(normal("n"));
    let mut got = Vec::new();
    for _ in 0..8 {
        let round = sim.now;
        crash(&mut sim, u, ExitCause::Exited);
        let s = sim.st(u);
        assert!(!s.is_failed(), "4 crashes per 10 min at most: {s:?}");
        let until = next_start(&s).unwrap();
        got.push(until - round);
        // Wait in standby, then the next use 160 s after the last crash.
        sim.at(round + 160_000);
        sim.start_interactive(u).unwrap();
        sim.run();
        sim.bring_up(u);
    }
    assert_eq!(
        got,
        vec![1_000, 2_000, 4_000, 8_000, 16_000, 32_000, 60_000, 60_000]
    );
}

// Normal no restart — instead re-start on the next trigger: no process
// without a trigger, however long.
#[test]
fn r_normal_does_not_restart_by_itself() {
    let mut sim = b4();
    let (u, pid) = sim.in_active(normal("n"));
    crash(&mut sim, u, ExitCause::Fault);
    assert!(spawns(&sim.after(3_600_000)).is_empty());
    assert!(
        matches!(sim.st(u), S::Standby { holder, .. } if holder == pid),
        "{:?}",
        sim.st(u)
    );
}

// "keeps its menu entries and starts again on next use after a short pause".
#[test]
fn r_normal_starts_again_on_next_use_after_the_pause() {
    let mut sim = b4();
    let (u, _) = sim.in_active(normal("n"));
    sim.settle();
    crash(&mut sim, u, ExitCause::Exited);
    let S::Standby {
        holder,
        hold_until: Some(until),
    } = sim.st(u)
    else {
        panic!("{:?}", sim.st(u))
    };
    sim.at(until);
    assert!(matches!(sim.wake(holder, cmd(7)), Delivery::Started));
    let acts = sim.run();
    let new = spawns(&acts);
    assert_eq!(new.len(), 1, "{:?}", names(&acts));
    sim.spawned(new[0], true);
    let acts = sim.ready(new[0]);
    assert_eq!(deliveries(&acts), vec![7]);
}

// ---------------------------------------------------------------------------
// Start limits (crashes per window)
// ---------------------------------------------------------------------------

// Own-process Builtin, Resident, Normal: the fifth crash in 10 min →
// `Failed{start-limit-hit}`, a notice, and a failure text in Settings > Plugins
// ("turned off as failed").
#[test]
fn l_fifth_crash_in_10_min_fails_with_start_limit_hit() {
    for spec in [builtin("b"), resident("r"), normal("n")] {
        let mut sim = b4();
        let (u, _) = sim.in_active(spec.clone());
        let mut acts = Vec::new();
        for i in 1..=5 {
            acts = crash(&mut sim, u, ExitCause::Exited);
            if i < 5 {
                assert!(!sim.st(u).is_failed(), "{spec:?} crash {i}");
                back_up(&mut sim, u);
            }
        }
        assert_eq!(sim.st(u), S::Failed("startlimithit".into()), "{spec:?}");
        assert!(
            notices(&acts).iter().any(|n| n == "startlimithit"),
            "{spec:?}: {:?}",
            notices(&acts)
        );
        assert!(
            sim.listed_failed(u).is_some_and(|t| !t.is_empty()),
            "{spec:?}: packages.list shows failed with a text"
        );
        assert!(spawns(&sim.after(3_600_000)).is_empty(), "{spec:?}");
    }
}

// A Normal unit that fails for good loses the entry points it kept in standby
// (`Failed`: revoked; trigger while `Failed` → `Dropped`).
#[test]
fn l_start_limit_failure_revokes_normal_entry_points() {
    let mut sim = b4();
    let (u, _) = sim.in_active(normal("n"));
    sim.settle();
    for _ in 1..5 {
        crash(&mut sim, u, ExitCause::Exited);
        back_up(&mut sim, u);
    }
    let pid = sim.pid(u);
    let acts = crash(&mut sim, u, ExitCause::Exited);
    assert_eq!(sim.st(u), S::Failed("startlimithit".into()));
    // Revoked by a `Revoke`, or by the UI cleanup of a `Terminate` that does
    // not keep registrations (`stop`'s cleanup, Deactivating).
    let by_revoke = acts.iter().filter_map(revoked).any(|r| r == (u, pid));
    let by_cleanup = terminates(&acts).iter().any(|t| t.3 == pid && !t.1);
    assert!(by_revoke || by_cleanup, "{:?}", names(&acts));
    assert!(matches!(sim.wake(pid, cmd(1)), Delivery::Dropped));
}

// The window slides: a crash older than 10 min does not count.
#[test]
fn l_start_limit_window_slides() {
    let mut sim = b4();
    let (u, _) = sim.in_active(builtin("b"));
    for _ in 0..4 {
        crash(&mut sim, u, ExitCause::Exited);
        back_up(&mut sim, u);
    }
    sim.after(600_001);
    crash(&mut sim, u, ExitCause::Exited);
    assert!(matches!(sim.st(u), S::AutoRestart(_)), "{:?}", sim.st(u));
}

// "reload clears it".
#[test]
fn l_reload_clears_a_start_limit_failure() {
    let mut sim = b4();
    let (u, _) = sim.in_active(builtin("b"));
    for i in 1..=5 {
        crash(&mut sim, u, ExitCause::Exited);
        if i < 5 {
            back_up(&mut sim, u);
        }
    }
    assert!(sim.st(u).is_failed());
    sim.rpc_restart(u, true, None).unwrap();
    let acts = sim.run();
    assert_eq!(spawns(&acts).len(), 1, "{:?}", names(&acts));
    assert_eq!(sim.listed_failed(u), None, "failure text cleared");
}

// one counter, automatic restarts after a crash; a manual start (reload) resets
// it. Four crashes, a reload, one more crash within the same 10 min: no
// start-limit failure.
#[test]
fn l_reload_resets_the_crash_count() {
    let mut sim = b4();
    let (u, _) = sim.in_active(builtin("b"));
    for _ in 0..4 {
        crash(&mut sim, u, ExitCause::Exited);
        back_up(&mut sim, u);
    }
    sim.rpc_restart(u, true, None).unwrap();
    sim.run();
    let pid = sim.pid(u);
    sim.gone(pid);
    let (_, pid) = sim.bring_up(u);
    sim.classified(pid, true);
    let start = sim.now;
    crash(&mut sim, u, ExitCause::Exited);
    assert!(sim.now - T0 < 600_000, "still inside the first window");
    assert!(
        matches!(sim.st(u), S::AutoRestart(_)),
        "crash count reset by the reload: {:?} (at {start})",
        sim.st(u)
    );
}

// manual starts are not counted (only crashes count): six reloads in a
// minute and then a crash restart.
#[test]
fn l_manual_starts_do_not_count() {
    let mut sim = b4();
    let (u, _) = sim.in_active(builtin("b"));
    for _ in 0..6 {
        sim.rpc_restart(u, true, None).unwrap();
        sim.run();
        let pid = sim.pid(u);
        sim.gone(pid);
        let (_, pid) = sim.bring_up(u);
        sim.classified(pid, true);
        sim.after(1_000);
    }
    crash(&mut sim, u, ExitCause::Exited);
    assert!(matches!(sim.st(u), S::AutoRestart(_)), "{:?}", sim.st(u));
}

// Development: 1 failure → Failed; no start retry.
#[test]
fn l_development_fails_at_the_first_failure() {
    let mut sim = b4();
    let (u, _) = sim.in_active(dev("d", 120_000));
    crash(&mut sim, u, ExitCause::Fault);
    assert!(sim.st(u).is_failed(), "{:?}", sim.st(u));

    let mut sim = b4();
    let u = sim.add(dev("d", 30_000));
    sim.run();
    sim.spawned(sim.pid(u), false);
    assert!(sim.st(u).is_failed(), "{:?}", sim.st(u));
    assert!(spawns(&sim.after(60_000)).is_empty());
}

// ---------------------------------------------------------------------------
// Start retries ("Start retry", StartTimeout)
// ---------------------------------------------------------------------------

/// Fails the unit's spawns until it stops retrying; returns the delay of
/// each retry and every action.
fn fail_spawns(sim: &mut Sim, u: UnitId, limit: usize) -> (Vec<Ms>, Vec<Act>) {
    let mut gaps = Vec::new();
    let mut acts = Vec::new();
    for _ in 0..limit {
        let pid = match sim.st(u) {
            S::Spawn(pid) => pid,
            S::Verify => {
                acts.extend(sim.verified(u, true));
                sim.pid(u)
            }
            other => panic!("{other:?}"),
        };
        let t = sim.now;
        acts.extend(sim.spawned(pid, false));
        assert!(
            !matches!(
                sim.st(u),
                S::Standby {
                    hold_until: None,
                    ..
                }
            ),
            "{:?}",
            sim.st(u)
        );
        match sim.st(u) {
            S::AutoRestart(until) => {
                gaps.push(until - t);
                acts.extend(sim.at(until));
            }
            _ => break,
        }
    }
    (gaps, acts)
}

fn retry_gaps_ok(gaps: &[Ms]) -> bool {
    gaps.len() == 3
        && gaps
            .iter()
            .zip([250, 500, 1_000])
            .all(|(g, b)| (b..=b + b / 10).contains(g))
}

// A woken Normal unit whose spawn fails: 3 retries (250, 500, 1000 ms + ≤ 10
// %), then `Standby{hold_until}` with its entry points kept; the click is
// answered "could not start" only once the retries are spent; the Start job
// ends once.
#[test]
fn sr_spawn_failure_retries_three_times_then_holds() {
    let mut sim = b4();
    let (u, holder) = sim.in_standby(normal("a"));
    sim.settle();
    assert!(matches!(sim.wake(holder, cmd(1)), Delivery::Started));
    let mark = sim.log.len();
    sim.run();
    let (gaps, acts) = fail_spawns(&mut sim, u, 10);
    assert!(retry_gaps_ok(&gaps), "{gaps:?}");
    let S::Standby {
        hold_until: Some(until),
        ..
    } = sim.st(u)
    else {
        panic!("{:?}", sim.st(u))
    };
    assert!(until > sim.now);
    assert_eq!(answers(&acts), vec![(1, Why::CouldNotStart)]);
    assert!(notices(&acts).iter().any(|n| n == "couldnotstart"));
    assert!(!has(&sim.log[mark..], "Revoke"), "entry points kept");
    let ends = sim.log[mark..]
        .iter()
        .filter_map(finished)
        .filter(|f| f.0 == u)
        .count();
    assert_eq!(ends, 1, "one Start job across its retries");
}

// a Resident never lands in standby on a start failure; "else `Failed{spawn}`"
// once the retries are spent. Same for an own-process built-in (it is started,
// not woken).
#[test]
fn sr_resident_and_builtin_never_land_in_standby_on_start_failure() {
    for spec in [resident("r"), builtin("b")] {
        let mut sim = b4();
        let u = sim.add(spec.clone());
        sim.run();
        let (gaps, _) = fail_spawns(&mut sim, u, 10);
        assert!(retry_gaps_ok(&gaps), "{spec:?}: {gaps:?}");
        assert_eq!(sim.st(u), S::Failed("spawn".into()), "{spec:?}");
        assert!(!has(&sim.log, "Replay"), "{spec:?}");
    }
}

// StartTimeout: terminate; not retried (coordinator ruling, supervisor.md §7:
// start retries are for a process that could not be created; a start that
// times out is a stuck plugin). The click held for it is
// answered at the timeout, and no second spawn follows the reap within the
// retries' 1.1 s.
#[test]
fn sr_start_timeout_is_not_retried() {
    let mut sim = b4();
    let (u, holder) = sim.in_standby(normal("a"));
    sim.settle();
    sim.wake(holder, cmd(1));
    sim.run();
    let pid = sim.pid(u);
    sim.spawned(pid, true);
    let t = sim.now;
    assert!(terminates(&sim.at(t + START - 1)).is_empty());
    let acts = sim.at(t + START);
    assert_eq!(
        terminates(&acts),
        vec![(FAILURE_GRACE, terminates(&acts)[0].1, "failed".into(), pid)]
    );
    assert_eq!(answers(&acts), vec![(1, Why::CouldNotStart)]);
    sim.after(200);
    let reap = sim.now;
    sim.gone(pid);
    assert!(spawns(&sim.at(reap + 1_100)).is_empty(), "{:?}", sim.st(u));
}

// A timed-out start keeps its entry points (coordinator ruling) with the class
// policy (Normal: restarts on the next trigger): a wake whose start times out
// keeps its entry points in `Standby{hold_until}` and answers the click, after
// one try (no start retry after a timeout; was: after four).
#[test]
fn sr_start_timeouts_exhausted_keep_entry_points() {
    let mut sim = b4();
    let (u, holder) = sim.in_standby(normal("a"));
    sim.settle();
    sim.wake(holder, cmd(1));
    let mark = sim.log.len();
    sim.run();
    let pid = match sim.st(u) {
        S::Spawn(pid) => pid,
        other => panic!("{other:?}"),
    };
    sim.spawned(pid, true);
    sim.after(START);
    sim.gone(pid);
    assert!(
        matches!(
            sim.st(u),
            S::Standby {
                hold_until: Some(_),
                ..
            }
        ),
        "{:?}",
        sim.st(u)
    );
    assert!(!has(&sim.log[mark..], "Revoke"));
    assert_eq!(answers(&sim.log[mark..]), vec![(1, Why::CouldNotStart)]);
    assert_eq!(spawns(&sim.log[mark..]).len(), 1, "one try");
}

// A start retry is the start job's: a reload
// whose spawn fails keeps its job, and so stays "busy" for another package RPC,
// until the retried start is ready.
#[test]
fn sr_retry_belongs_to_the_reloads_job() {
    let mut sim = b4();
    let (u, pid) = sim.in_active(builtin("b"));
    let job = sim.rpc_restart(u, true, None).unwrap();
    sim.run();
    sim.gone(pid);
    let new = sim.pid(u);
    sim.spawned(new, false);
    let S::AutoRestart(until) = sim.st(u) else {
        panic!("{:?}", sim.st(u))
    };
    assert!(sim.package_busy(u));
    assert!(sim.rpc_restart(u, true, None).is_err(), "busy");
    sim.at(until);
    let new = sim.pid(u);
    sim.spawned(new, true);
    assert!(sim.package_busy(u), "until ready");
    sim.ready(new);
    assert!(!sim.package_busy(u));
    let ends = sim
        .log
        .iter()
        .filter_map(finished)
        .filter(|f| f.1 == job)
        .map(|f| f.2)
        .collect::<Vec<_>>();
    assert_eq!(ends, vec![R::Done]);
}

// ---------------------------------------------------------------------------
// Exit causes, stop reasons and notices (the heap cap)
// ---------------------------------------------------------------------------

// `runtime.failed` (`Refused`) is never retried or restarted, every class.
#[test]
fn x_runtime_failed_never_retried() {
    for spec in [
        builtin("b"),
        resident("r"),
        first_run("n"),
        dev("d", 120_000),
    ] {
        let mut sim = b4();
        let u = sim.add(spec.clone());
        sim.run();
        let pid = sim.pid(u);
        sim.spawned(pid, true);
        sim.failed(pid);
        sim.gone(pid);
        assert!(sim.st(u).is_failed(), "{spec:?}: {:?}", sim.st(u));
        assert!(spawns(&sim.after(3_600_000)).is_empty(), "{spec:?}");
    }
}

// A crash notice: "<name> stopped" for an exit, "…ran out of memory" for
// an OOM, every class with a process of its own.
#[test]
fn x_crash_notice() {
    for spec in [builtin("b"), resident("r"), normal("n")] {
        for (cause, want) in [
            (ExitCause::Exited, "crashedexited"),
            (ExitCause::Fault, "crashedfault"),
            (ExitCause::Oom, "crashedoom"),
        ] {
            let mut sim = b4();
            let (u, _) = sim.in_active(spec.clone());
            let acts = crash(&mut sim, u, cause);
            assert!(
                notices(&acts).iter().any(|n| n == want),
                "{spec:?} {cause:?}: {:?}",
                notices(&acts)
            );
        }
    }
}

// OOM (OOM policy): a crash like any other for the restart policy, with
// "ran out of memory" in the failure text where the unit fails
// (Development: `Failed`).
#[test]
fn x_oom_is_a_crash_and_says_out_of_memory() {
    let mut sim = b4();
    let (u, _) = sim.in_active(dev("d", 120_000));
    crash(&mut sim, u, ExitCause::Oom);
    assert!(
        sim.failure_text(u)
            .is_some_and(|t| t.contains("out of memory")),
        "{:?}",
        sim.failure_text(u)
    );
    let mut sim = b4();
    let (u, _) = sim.in_active(normal("n"));
    let first = delays(&mut sim, u, ExitCause::Oom, 2);
    assert_eq!(first, vec![1_000, 2_000], "OOM counts as a crash");
}

// every exit cause maps to the wire stop reason `Failed`. An exit and an OOM
// need no stop (the process is already gone; its reap follows); a request
// timeout terminates the live process first (`Terminate{Failed}`, grace 300 ms
// after a failure, KEEP), then `Kill`. DESIGN QUESTION: lists "UI cleanup" as
// an action; the core runs it as `Terminate{Failed, keep}` (a stop message to a
// process already gone). The brief reads as "Exited and OOM send no stop".
#[test]
fn x_stop_reasons_per_cause() {
    for cause in [ExitCause::Exited, ExitCause::Oom] {
        let mut sim = b4();
        let (_, pid) = sim.in_active(normal("n"));
        let acts = sim.exited(pid, cause);
        assert!(
            terminates(&acts).is_empty(),
            "{cause:?}: no stop sent to an exited process: {:?}",
            terminates(&acts)
        );
    }
    let mut sim = b4();
    let (_, pid) = sim.in_active(normal("n"));
    let acts = sim.exited(pid, ExitCause::RequestTimeout);
    let t = terminates(&acts);
    assert_eq!(t.len(), 1, "{:?}", names(&acts));
    assert_eq!(
        (t[0].0, t[0].2.as_str(), t[0].3),
        (FAILURE_GRACE, "failed", pid)
    );
    assert_eq!(
        sim.after(FAILURE_GRACE)
            .iter()
            .filter_map(kill_pid)
            .collect::<Vec<_>>(),
        vec![pid]
    );
    for cause in [ExitCause::Fault, ExitCause::Transport] {
        let mut sim = b4();
        let (_, pid) = sim.in_active(builtin("b"));
        let acts = sim.exited(pid, cause);
        assert!(
            terminates(&acts).iter().all(|t| t.2 == "failed"),
            "{cause:?}: {:?}",
            terminates(&acts)
        );
    }
}

// Transport, own process: `Failed` (KEEP), no restart, every class.
#[test]
fn x_transport_of_an_own_process_fails() {
    let mut wrong = Vec::new();
    for spec in [builtin("b"), resident("r"), normal("n")] {
        let mut sim = b4();
        let (u, _) = sim.in_active(spec.clone());
        crash(&mut sim, u, ExitCause::Transport);
        if !sim.st(u).is_failed() {
            wrong.push((spec.id.clone(), sim.st(u)));
        }
    }
    // DESIGN QUESTION: Transport "own process: `Failed`" (KEEP) vs
    // restart on failure (its list names exit, fault, OOM, start timeout,
    // request timeout — not transport).
    assert!(
        wrong.is_empty(),
        "not Failed after a transport failure: {wrong:?}"
    );
}

// A fault's own text reaches Settings > Plugins (`process_exited`).
#[test]
fn x_fault_text_kept_for_a_failed_unit() {
    let mut sim = b4();
    let (u, pid) = sim.in_active(dev("d", 120_000));
    sim.exited_with(pid, ExitCause::Fault, "TypeError: x is undefined");
    sim.gone(pid);
    assert_eq!(
        sim.listed_failed(u).as_deref(),
        Some("TypeError: x is undefined")
    );
}

// ---------------------------------------------------------------------------
// The crash hold (coordinator ruling)
// ---------------------------------------------------------------------------

/// A Normal unit (start-up settled) that crashed: `(unit, holder, until)`.
fn held(sim: &mut Sim) -> (UnitId, Pid, Ms) {
    let (u, _) = sim.in_active(normal("n"));
    sim.settle();
    crash(sim, u, ExitCause::Exited);
    match sim.st(u) {
        S::Standby {
            holder,
            hold_until: Some(until),
        } => (u, holder, until),
        other => panic!("{other:?}"),
    }
}

// An interactive trigger during the hold is answered at once with the
// reason and its end (the entry is shown disabled with the reason, ruling
// 4); it is not held and starts nothing.
#[test]
fn h_interactive_during_hold_answered_at_once() {
    let mut sim = b4();
    let (u, holder, until) = held(&mut sim);
    let d = sim.wake(holder, cmd(1));
    assert!(matches!(d, Delivery::Refused), "{d:?}");
    let acts = sim.run();
    assert_eq!(answers(&acts), vec![(1, Why::Held)]);
    assert_eq!(
        acts.iter().filter_map(held_until).collect::<Vec<_>>(),
        vec![until]
    );
    assert_eq!(sim.job(u), None, "no Start for an interactive trigger");
    assert_eq!(sim.backlog_len(u), 0);
    assert!(spawns(&sim.at(until + 600_000)).is_empty());
    assert!(matches!(sim.st(u), S::Standby { .. }));
}

// An event trigger during the hold is kept; a Start waits for `hold_until`,
// then delivers it.
#[test]
fn h_event_during_hold_waits_for_the_hold() {
    let mut sim = b4();
    let (u, holder, until) = held(&mut sim);
    let d = sim.wake(holder, event("playback.state", 2));
    assert!(!matches!(d, Delivery::Refused | Delivery::Dropped), "{d:?}");
    sim.run();
    assert!(
        matches!(sim.job(u), Some(J::Start { .. })),
        "{:?}",
        sim.job(u)
    );
    assert!(
        spawns(&sim.at(until - 1)).is_empty(),
        "no start in the hold"
    );
    let new = spawns(&sim.at(until));
    assert_eq!(new.len(), 1);
    sim.spawned(new[0], true);
    assert_eq!(deliveries(&sim.ready(new[0])), vec![2]);
}

// Once the hold is over, a click starts the plugin at once (no disabled
// entry any more).
#[test]
fn h_after_the_hold_a_click_starts_at_once() {
    let mut sim = b4();
    let (u, holder, until) = held(&mut sim);
    sim.at(until);
    assert!(matches!(sim.wake(holder, cmd(3)), Delivery::Started));
    assert_eq!(spawns(&sim.run()).len(), 1);
    assert!(matches!(sim.st(u), S::Spawn(_)));
}

// A crash that lands in a hold answers the held interactive triggers ("could
// not start") and keeps a Start only for the events.
#[test]
fn h_crash_into_hold_answers_clicks_keeps_events() {
    let mut sim = b4();
    let (u, holder) = sim.in_standby(normal("n"));
    sim.settle();
    sim.wake(holder, cmd(1));
    sim.wake(holder, event("playback.state", 2));
    sim.run();
    let pid = sim.pid(u);
    sim.spawned(pid, true);
    let mut acts = sim.exited(pid, ExitCause::Exited);
    acts.extend(sim.gone(pid));
    let S::Standby {
        hold_until: Some(until),
        ..
    } = sim.st(u)
    else {
        panic!("{:?}", sim.st(u))
    };
    assert_eq!(answers(&acts), vec![(1, Why::CouldNotStart)]);
    assert_eq!(sim.backlog_len(u), 1);
    assert!(
        matches!(sim.job(u), Some(J::Start { .. })),
        "{:?}",
        sim.job(u)
    );
    assert!(spawns(&sim.at(until - 1)).is_empty());
    let new = spawns(&sim.at(until));
    assert_eq!(new.len(), 1);
    sim.spawned(new[0], true);
    assert_eq!(deliveries(&sim.ready(new[0])), vec![2]);
}

#[test]
fn h_crash_into_hold_with_only_clicks_installs_no_start() {
    let mut sim = b4();
    let (u, holder) = sim.in_standby(normal("n"));
    sim.settle();
    sim.wake(holder, cmd(1));
    sim.run();
    let pid = sim.pid(u);
    sim.spawned(pid, true);
    let mut acts = sim.exited(pid, ExitCause::Exited);
    acts.extend(sim.gone(pid));
    assert!(
        matches!(
            sim.st(u),
            S::Standby {
                hold_until: Some(_),
                ..
            }
        ),
        "{:?}",
        sim.st(u)
    );
    assert_eq!(answers(&acts), vec![(1, Why::CouldNotStart)]);
    assert_eq!(sim.job(u), None);
    assert!(spawns(&sim.after(600_000)).is_empty());
}

// ---------------------------------------------------------------------------
// Runner restarts (`runner_restart_waits_cleanup`)
// ---------------------------------------------------------------------------

// Members wait for the playback worker's cleanup before they restart;
// the first restart is then immediate; the OSD says so.
#[test]
fn rn_first_restart_waits_for_cleanup_then_immediate() {
    let mut sim = b4();
    let (a, pa) = sim.in_active(builtin("a").shared(1));
    let (b, pb) = sim.in_active(builtin("b").shared(1));
    let acts = sim.runner_hung(1);
    assert_eq!(count(&acts, "KillRunner"), 1);
    assert!(spawns(&acts).is_empty(), "{:?}", names(&acts));
    assert!(matches!(sim.st(a), S::Cleanup(_)), "{:?}", sim.st(a));
    assert!(
        spawns(&sim.after(5_000)).is_empty(),
        "waits for the cleanup"
    );
    let acts = sim.cleaned(pa);
    assert!(matches!(sim.st(a), S::Spawn(_)), "{:?}", sim.st(a));
    assert_eq!(spawns(&acts).len(), 1);
    sim.cleaned(pb);
    assert!(matches!(sim.st(b), S::Spawn(_)), "{:?}", sim.st(b));
    assert!(
        notices(&sim.log).iter().any(|n| n == "runnerrestarted"),
        "{:?}",
        notices(&sim.log)
    );
}

// The second runner restart in the window waits the restart delay 3 s.
#[test]
fn rn_second_restart_after_3s() {
    let mut sim = b4();
    let (a, pa) = sim.in_active(builtin("a").shared(1));
    sim.runner_hung(1);
    sim.cleaned(pa);
    let (_, pa) = sim.bring_up(a);
    sim.after(1_000);
    sim.runner_hung(1);
    let t = sim.now;
    let acts = sim.cleaned(pa);
    assert!(spawns(&acts).is_empty());
    assert_eq!(sim.st(a), S::AutoRestart(t + 3_000));
}

// Third restart in 60 s → fail-open: own processes for the session, the
// OSD says so, placement rewritten before the restart spawns.
#[test]
fn rn_third_restart_in_60s_fails_open() {
    let mut sim = b4();
    let (a, _) = sim.in_active(builtin("a").shared(1));
    let (b, _) = sim.in_active(builtin("b").shared(1));
    let mark = sim.log.len();
    for _ in 0..3 {
        let (pa, pb) = (sim.pid(a), sim.pid(b));
        sim.runner_hung(1);
        sim.cleaned(pa);
        sim.cleaned(pb);
        if sim.failed_open() {
            break;
        }
        for u in [a, b] {
            if let S::AutoRestart(t) = sim.st(u) {
                sim.at(t);
            }
            sim.bring_up(u);
        }
    }
    assert!(sim.failed_open());
    assert!(
        notices(&sim.log).iter().any(|n| n == "runnerfailedopen"),
        "{:?}",
        notices(&sim.log)
    );
    for u in [a, b] {
        if let S::AutoRestart(t) = sim.st(u) {
            sim.at(t);
        }
    }
    let last = sim
        .log
        .iter()
        .filter_map(spawn_own)
        .rev()
        .take(2)
        .collect::<Vec<_>>();
    assert_eq!(last.len(), 2);
    assert!(last.iter().all(|s| s.1), "own processes: {last:?}");
    assert!(sim.own_process(a) && sim.own_process(b));
    assert!(sim.log.len() > mark);
}

// Attributed fault: that member alone stops (no runner kill) and its next
// process is its own (quarantined); the other member keeps running.
#[test]
fn rn_attributed_fault_stops_the_member_alone() {
    let mut sim = b4();
    let (a, pa) = sim.in_active(builtin("a").shared(1));
    let (b, pb) = sim.in_active(builtin("b").shared(1));
    let mut acts = sim.exited(pa, ExitCause::Fault);
    acts.extend(sim.gone(pa));
    assert!(!has(&acts, "KillRunner"), "{:?}", names(&acts));
    assert_eq!(sim.st(b), S::Active(pb));
    let until = next_start(&sim.st(a)).unwrap_or_else(|| panic!("{:?}", sim.st(a)));
    let acts = sim.at(until);
    let s = acts.iter().filter_map(spawn_own).collect::<Vec<_>>();
    assert_eq!(s.len(), 1, "{:?}", names(&acts));
    assert!(s[0].1, "quarantined member runs in its own process");
}

// a member's full queue (`transport_failed`) restarts the runner group.
#[test]
fn rn_member_transport_restarts_the_group() {
    let mut sim = b4();
    let (_, pa) = sim.in_active(builtin("a").shared(1));
    let (b, _) = sim.in_active(builtin("b").shared(1));
    let acts = sim.exited(pa, ExitCause::Transport);
    assert_eq!(count(&acts, "KillRunner"), 1, "{:?}", names(&acts));
    assert!(sim.st(b).is_deactivating(), "{:?}", sim.st(b));
}

// re-placement: a member with a standby checkpoint comes back in standby by
// `Replay` (no spawn); the others get a Restart.
#[test]
fn rn_member_with_standby_checkpoint_replays() {
    let mut sim = b4();
    let (m, pm) = sim.in_active(builtin_standby("m").shared(1));
    let (a, pa) = sim.in_active(builtin("a").shared(1));
    sim.runner_hung(1);
    let mut acts = sim.cleaned(pm);
    acts.extend(sim.cleaned(pa));
    assert!(has(&acts, "Replay"), "{:?}", names(&acts));
    assert!(matches!(sim.st(m), S::Standby { .. }), "{:?}", sim.st(m));
    assert!(matches!(sim.st(a), S::Spawn(_)), "{:?}", sim.st(a));
}

// ---------------------------------------------------------------------------
// Uninstall data removal (`remove_data_waits_cleanup`)
// ---------------------------------------------------------------------------

#[test]
fn rd_uninstall_removes_data_after_reap_and_cleanup() {
    for reap_first in [true, false] {
        let mut sim = b4();
        let (u, pid) = sim.in_active(normal("n"));
        sim.stop(u, StopReason::Uninstall, Then::Removed).unwrap();
        sim.run();
        let first = if reap_first {
            sim.reaped(pid)
        } else {
            sim.cleaned(pid)
        };
        assert!(
            !has(&first, "RemoveData"),
            "reap_first {reap_first}: one half is not enough"
        );
        let second = if reap_first {
            sim.cleaned(pid)
        } else {
            sim.reaped(pid)
        };
        assert_eq!(count(&second, "RemoveData"), 1, "reap_first {reap_first}");
    }
}

// ---------------------------------------------------------------------------
// Shutdown
// ---------------------------------------------------------------------------

/// Two Normal units (classified, so they are checkpointed at quit) and an
/// own-process built-in, all active.
fn three_running(sim: &mut Sim) -> Vec<Pid> {
    let (_, a) = sim.in_active(normal("a"));
    let (_, b) = sim.in_active(normal("b"));
    let (_, c) = sim.in_active(builtin("c"));
    sim.classified(a, false);
    sim.classified(b, false);
    sim.classified(c, true);
    vec![a, b, c]
}

// Quit: every Checkpoint before the first Terminate, every process terminated
// in the same call (parallel), one deadline at ≤ 1 s, then `Kill` for what is
// left; nothing after it.
#[test]
fn q_quit_parallel_within_one_second() {
    let mut sim = b4();
    let pids = three_running(&mut sim);
    let t = sim.now;
    let acts = sim.shutdown(1_000);
    let last_ck = acts.iter().rposition(|a| act_name(a) == "Checkpoint");
    let first_term = pos(&acts, "Terminate").expect("Terminate");
    assert!(
        last_ck.is_some_and(|c| c < first_term),
        "{:?}",
        names(&acts)
    );
    let term = terminates(&acts);
    assert!(
        same_set(&term.iter().map(|t| t.3).collect::<Vec<_>>(), &pids),
        "{term:?}"
    );
    assert!(
        term.iter().all(|t| t.0 <= 1_000 && t.2 == "quit"),
        "{term:?}"
    );
    assert!(sim.next_deadline().is_some_and(|d| d <= t + 1_000));
    sim.at(t + 100);
    sim.reaped(pids[0]);
    assert!(!has(&sim.at(t + 999), "Kill"));
    let killed = sim
        .at(t + 1_000)
        .iter()
        .filter_map(kill_pid)
        .collect::<Vec<_>>();
    assert!(same_set(&killed, &pids[1..]), "{killed:?}");
    assert!(!has(&sim.after(60_000), "Kill"), "one deadline");
}

// Device loss: the same with 300 ms.
#[test]
fn q_device_loss_within_300ms() {
    let mut sim = b4();
    let pids = three_running(&mut sim);
    let t = sim.now;
    let acts = sim.shutdown(300);
    assert!(terminates(&acts).iter().all(|t| t.0 <= 300));
    assert!(!has(&sim.at(t + 299), "Kill"));
    let killed = sim
        .at(t + 300)
        .iter()
        .filter_map(kill_pid)
        .collect::<Vec<_>>();
    assert!(same_set(&killed, &pids), "{killed:?}");
}

// A stuck old process the unit still tracks after its stop wait ran out is
// killed by the shutdown, within its budget.
#[test]
fn q_shutdown_kills_a_stuck_tracked_process() {
    let mut sim = b4();
    let (u, pid) = sim.in_active(normal("n"));
    sim.stop(u, StopReason::Disable, Then::Inactive).unwrap();
    sim.run();
    sim.after(STOP_WAIT);
    assert!(sim.stuck(u), "{:?}", sim.st(u));
    assert_eq!(sim.st(u), S::Inactive);
    let t = sim.now;
    let mark = sim.log.len();
    sim.shutdown(1_000);
    sim.at(t + 1_000);
    let killed = sim.log[mark..]
        .iter()
        .filter_map(kill_pid)
        .collect::<Vec<_>>();
    assert_eq!(killed, vec![pid], "{:?}", names(&sim.log[mark..]));
}

// Quit ends every pending restart: no `AutoRestart` or hold start spawns
// after the shutdown.
#[test]
fn q_shutdown_cancels_pending_restarts() {
    let mut sim = b4();
    let (b, _) = sim.in_active(builtin("b"));
    let (n, _) = sim.in_active(normal("n"));
    sim.settle();
    crash(&mut sim, b, ExitCause::Exited);
    crash(&mut sim, n, ExitCause::Exited);
    let holder = sim.pid(n);
    sim.wake(holder, event("playback.state", 1));
    sim.run();
    assert!(matches!(sim.st(b), S::AutoRestart(_)));
    assert!(sim.job(n).is_some());
    let mark = sim.log.len();
    sim.shutdown(1_000);
    sim.after(60_000);
    assert!(
        spawns(&sim.log[mark..]).is_empty(),
        "{:?}",
        names(&sim.log[mark..])
    );
}
