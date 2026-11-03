//! Per-class policy, design §3.3 (with §7.3 exit causes and §7.4 runner
//! restarts). Restart policy, backoff and start limits arrive in B4, so
//! most tests use the target profile; B1 (`Restart=no` for every class) is
//! covered by `t25_crash_b1_fails_every_class`.
use super::*;

fn target() -> Sim {
    Sim::new(&Profile::target())
}

/// Unexpected exit at `now`, reaped and cleaned at the same time.
fn crash(sim: &mut Sim, u: UnitId, cause: ExitCause) -> S {
    let pid = sim.pid(u);
    sim.exited(pid, cause);
    sim.gone(pid);
    sim.st(u)
}

/// Starts the unit again at the earliest allowed time and brings it up.
fn back_up(sim: &mut Sim, u: UnitId) {
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
    sim.bring_up(u);
}

fn delay(s: &S, now: Ms) -> Ms {
    match s {
        S::AutoRestart(t) => t - now,
        S::Standby {
            hold_until: Some(t),
            ..
        } => t - now,
        other => panic!("no backoff in {other:?}"),
    }
}

// Restart= on-failure (Builtin): exit, fault, OOM, request timeout
#[test]
fn p_builtin_restart_on_failure_for_every_cause() {
    for cause in [
        ExitCause::Exited,
        ExitCause::Fault,
        ExitCause::Oom,
        ExitCause::RequestTimeout,
    ] {
        let mut sim = target();
        let (u, _) = sim.in_active(builtin("b"));
        let s = crash(&mut sim, u, cause);
        assert!(matches!(s, S::AutoRestart(_)), "{cause:?}: {s:?}");
    }
}

// Restart= on-failure (Resident, A15b), RestartSec 1 s
#[test]
fn p_resident_restarts_after_one_second() {
    let mut sim = target();
    let (u, _) = sim.in_active(resident("r"));
    let now = sim.now;
    assert_eq!(
        crash(&mut sim, u, ExitCause::Fault),
        S::AutoRestart(now + 1_000)
    );
}

// Restart= no for Normal: re-start on the next trigger only (A15c)
#[test]
fn p_normal_does_not_restart_by_itself() {
    let mut sim = target();
    let (u, pid) = sim.in_active(normal("n"));
    let S::Standby {
        holder,
        hold_until: Some(until),
    } = crash(&mut sim, u, ExitCause::Exited)
    else {
        panic!()
    };
    assert_eq!(holder, pid, "registrations kept under the last run's pid");
    assert!(
        spawns(&sim.at(until + 600_000)).is_empty(),
        "no restart without a trigger"
    );
    assert!(matches!(sim.st(u), S::Standby { .. }));
}

// Restart= no → Failed (Development)
#[test]
fn p_development_crash_fails() {
    let mut sim = target();
    let (u, _) = sim.in_active(dev("d", 120_000));
    assert!(crash(&mut sim, u, ExitCause::Fault).is_failed());
}

// Not restarted when the plugin said it cannot start (runtime.failed), every class
#[test]
fn p_no_retry_after_runtime_failed_any_class() {
    for spec in [
        builtin("b"),
        resident("r"),
        first_run("n"),
        dev("d", 120_000),
    ] {
        let mut sim = target();
        let u = sim.add(spec.clone());
        sim.run();
        let pid = sim.pid(u);
        sim.spawned(pid, true);
        sim.failed(pid);
        sim.gone(pid);
        assert!(sim.st(u).is_failed(), "{spec:?}");
        assert!(spawns(&sim.after(3_600_000)).is_empty(), "{spec:?}");
    }
}

// RestartSec built-in own process: 3 s × 2ⁿ
// ANSWERED (supervisor.md §17.2 Q19), was a design question: §3.3 says "first runner restart immediate"; §11 says
// "+ 3 s ×2ⁿ after the first" for built-ins. Best reading: "immediate
// first" applies to runner restarts only; an own-process built-in waits
// 3 s, 6 s, 12 s.
#[test]
fn p_builtin_own_backoff_grows_3_6_12() {
    let mut sim = target();
    let (u, _) = sim.in_active(builtin("b"));
    let mut got = Vec::new();
    for _ in 0..3 {
        let now = sim.now;
        let s = crash(&mut sim, u, ExitCause::Exited);
        got.push(delay(&s, now));
        back_up(&mut sim, u);
    }
    assert_eq!(got, vec![3_000, 6_000, 12_000]);
}

// Backoff counter reset after 60 s stable (Builtin)
#[test]
fn p_builtin_backoff_resets_after_60s_stable() {
    let mut sim = target();
    let (u, _) = sim.in_active(builtin("b"));
    crash(&mut sim, u, ExitCause::Exited);
    back_up(&mut sim, u);
    sim.after(60_000);
    let now = sim.now;
    let s = crash(&mut sim, u, ExitCause::Exited);
    assert_eq!(
        delay(&s, now),
        3_000,
        "stable for 60 s: back to the first step"
    );
}

#[test]
fn p_builtin_backoff_not_reset_before_60s_stable() {
    let mut sim = target();
    let (u, _) = sim.in_active(builtin("b"));
    crash(&mut sim, u, ExitCause::Exited);
    back_up(&mut sim, u);
    sim.after(59_999);
    let now = sim.now;
    let s = crash(&mut sim, u, ExitCause::Exited);
    assert_eq!(delay(&s, now), 6_000);
}

// RestartSec / hold Normal: 1 s × 2ⁿ (cap 60 s)
// ANSWERED (supervisor.md §17.2 Q20), was a design question: the 60 s cap cannot be reached within the 5-in-10-min
// start limit (1+2+4+8 s) unless crashes are spread over more than 10
// minutes, and §3.3 states the "reset after 60 s stable" rule only for
// Builtin. Does the Normal/Resident backoff reset after a stable run? Not
// tested here beyond the first four steps.
#[test]
fn p_normal_hold_grows_1_2_4_8() {
    let mut sim = target();
    let (u, _) = sim.in_active(normal("n"));
    let mut got = Vec::new();
    for _ in 0..4 {
        let now = sim.now;
        let s = crash(&mut sim, u, ExitCause::Exited);
        got.push(delay(&s, now));
        back_up(&mut sim, u);
    }
    assert_eq!(got, vec![1_000, 2_000, 4_000, 8_000]);
}

#[test]
fn p_resident_backoff_grows_1_2_4() {
    let mut sim = target();
    let (u, _) = sim.in_active(resident("r"));
    let mut got = Vec::new();
    for _ in 0..3 {
        let now = sim.now;
        let s = crash(&mut sim, u, ExitCause::Fault);
        got.push(delay(&s, now));
        back_up(&mut sim, u);
    }
    assert_eq!(got, vec![1_000, 2_000, 4_000]);
}

// Runner: first restart immediate (A4), then RestartSec 3 s
#[test]
fn p_first_runner_restart_immediate_then_3s() {
    let mut sim = target();
    let (a, pa) = sim.in_active(builtin("a").shared(1));
    sim.runner_hung(1);
    assert!(matches!(sim.st(a), S::Cleanup(_)));
    sim.cleaned(pa);
    assert!(
        matches!(sim.st(a), S::Spawn(_)),
        "first restart immediate: {:?}",
        sim.st(a)
    );
    let (_, pa) = sim.bring_up(a);
    sim.after(1_000);
    sim.runner_hung(1);
    let now = sim.now;
    let acts = sim.cleaned(pa);
    assert!(spawns(&acts).is_empty(), "second restart waits");
    assert_eq!(sim.st(a), S::AutoRestart(now + 3_000));
}

// StartLimit shared members: 3 runner restarts in 60 s → fail-open (own processes for the session)
#[test]
fn p_runner_fail_open_after_three_in_60s() {
    let mut sim = target();
    let (a, _) = sim.in_active(builtin("a").shared(1));
    let (b, _) = sim.in_active(builtin("b").shared(1));
    for round in 0..3 {
        let (pa, pb) = (sim.pid(a), sim.pid(b));
        sim.runner_hung(1);
        sim.cleaned(pa);
        sim.cleaned(pb);
        if round < 2 {
            for u in [a, b] {
                if let S::AutoRestart(t) = sim.st(u) {
                    sim.at(t);
                }
                sim.bring_up(u);
            }
            assert!(!sim.own_process(a), "round {round}: still shared");
        }
    }
    assert!(
        sim.failed_open(),
        "fail-open after the third restart in 60 s"
    );
    // Placement is rewritten before the restart jobs run (§7.4).
    for u in [a, b] {
        if let S::AutoRestart(t) = sim.st(u) {
            sim.at(t);
        }
    }
    assert!(sim.own_process(a) && sim.own_process(b));
}

#[test]
fn p_runner_restarts_spread_over_60s_do_not_fail_open() {
    let mut sim = target();
    let (a, _) = sim.in_active(builtin("a").shared(1));
    for _ in 0..3 {
        let pa = sim.pid(a);
        sim.runner_hung(1);
        sim.cleaned(pa);
        if let S::AutoRestart(t) = sim.st(a) {
            sim.at(t);
        }
        sim.bring_up(a);
        sim.after(61_000);
    }
    assert!(!sim.own_process(a));
}

// StartLimit own-process built-in: 5 starts in 10 min → Failed{start-limit-hit}; reload clears it
#[test]
fn p_builtin_own_start_limit_5_in_10_min() {
    let mut sim = target();
    let (u, _) = sim.in_active(builtin("b"));
    for i in 1..=5 {
        let s = crash(&mut sim, u, ExitCause::Exited);
        if i < 5 {
            assert!(!s.is_failed(), "crash {i}");
            back_up(&mut sim, u);
        } else {
            assert_eq!(s, S::Failed("startlimithit".into()));
        }
    }
    sim.rpc_restart(u, true, None).unwrap();
    assert_eq!(
        spawns(&sim.run()).len(),
        1,
        "reload clears the start-limit failure"
    );
}

// StartLimit Resident: 5 crashes in 10 min → Failed{start-limit-hit}
#[test]
fn p_resident_start_limit_5_crashes() {
    let mut sim = target();
    let (u, _) = sim.in_active(resident("r"));
    for i in 1..=5 {
        let s = crash(&mut sim, u, ExitCause::Fault);
        if i < 5 {
            back_up(&mut sim, u);
        } else {
            assert_eq!(s, S::Failed("startlimithit".into()));
        }
    }
}

// The start-limit window slides: crashes older than 10 min do not count
#[test]
fn p_start_limit_window_slides() {
    let mut sim = target();
    let (u, _) = sim.in_active(normal("n"));
    for _ in 0..4 {
        crash(&mut sim, u, ExitCause::Exited);
        back_up(&mut sim, u);
    }
    sim.after(10 * 60_000 + 1);
    let s = crash(&mut sim, u, ExitCause::Exited);
    assert!(!s.is_failed(), "{s:?}");
}

// StartLimit Development: 1 failure → Failed
#[test]
fn p_development_one_failure_fails() {
    let mut sim = target();
    let u = sim.add(dev("d", 30_000));
    sim.run();
    sim.spawned(sim.pid(u), false);
    assert!(
        sim.st(u).is_failed(),
        "no start retry for Development: {:?}",
        sim.st(u)
    );
}

// Start retry: 3 × (250 ms × 2ⁿ + 10 % jitter)
#[test]
fn p_start_retry_250_500_1000_with_jitter() {
    let mut sim = target();
    let u = sim.add(resident("r"));
    sim.run();
    let mut got = Vec::new();
    for _ in 0..3 {
        let now = sim.now;
        sim.spawned(sim.pid(u), false);
        let S::AutoRestart(t) = sim.st(u) else {
            panic!("{:?}", sim.st(u))
        };
        got.push(t - now);
        sim.at(t);
    }
    for (d, base) in got.iter().zip([250, 500, 1_000]) {
        assert!((base..=base + base / 10).contains(d), "{got:?}");
    }
    sim.spawned(sim.pid(u), false);
    assert!(
        !matches!(sim.st(u), S::AutoRestart(t) if t - sim.now < 2_000),
        "only 3 retries: {:?}",
        sim.st(u)
    );
}

// B1: no start retry
#[test]
fn p_b1_no_start_retry() {
    let mut sim = Sim::new(&Profile::b1());
    let u = sim.add(resident("r"));
    sim.run();
    sim.spawned(sim.pid(u), false);
    assert!(sim.st(u).is_failed(), "{:?}", sim.st(u));
}

// Idle timeout: Development never
both! {
    fn p_development_never_stands_by(p) {
        let mut sim = Sim::new(p);
        let (u, pid) = sim.in_active(dev("d", 120_000));
        let acts = sim.after(10 * p.idle);
        assert!(!has(&acts, "QueryResources"));
        assert!(!has(&sim.pressure_low(), "QueryResources"));
        sim.classified(pid, false);
        sim.after(10 * p.idle);
        assert_eq!(sim.st(u), S::Active(pid), "Development never goes to standby");
        // Never classified or checkpointed either (§17.7 item 2): no
        // standby record for a development load, across its whole run.
        assert!(!has(&sim.log, "Checkpoint"), "{:?}", names(&sim.log));
    }
}

// Idle timeout: Resident never (§3.3) — the target. Today's `lazy_idle`
// does not look at the class (coordinator ruling D3, §17.7): the `…_legacy`
// twin below.
#[test]
fn p_resident_never_idles() {
    let p = Profile::target();
    let mut sim = Sim::new(&p);
    let (u, pid) = sim.in_active(resident("r"));
    assert!(!has(&sim.after(10 * p.idle), "QueryResources"));
    assert_eq!(sim.st(u), S::Active(pid));
}

// Today a resident plugin idles like any other: asked for its resources
// after the idle timeout, and parked when it has none (`park` keeps
// `resident` in the checkpoint) (D3).
#[test]
fn p_resident_idles_legacy() {
    let mut sim = Sim::new(&Profile::b1());
    let (u, pid) = sim.in_active(resident("r"));
    sim.idle_out(pid);
    assert_eq!(sim.st(u).then(), Some(Then::Standby), "{:?}", sim.st(u));
}

// Idle timeout: Builtin yes (own process and shared members)
both! {
    fn p_builtin_idles_like_normal(p) {
        for spec in [builtin_standby("b"), builtin_standby("m").shared(1)] {
            let mut sim = Sim::new(p);
            let (u, pid) = sim.in_active(spec);
            sim.idle_out(pid);
            assert_eq!(sim.st(u).then(), Some(Then::Standby));
        }
    }
}

// TimeoutStopSec after a failure: 300 ms in both profiles (KEEP)
both! {
    fn p_failure_stop_grace_300ms(p) {
        let mut sim = Sim::new(p);
        let u = sim.add(resident("r"));
        sim.run();
        let pid = sim.pid(u);
        sim.spawned(pid, true);
        let acts = sim.failed(pid);
        assert_eq!(terminates(&acts)[0].0, 300);
    }
}

// B6 review (§17.15, "A start that times out is not retried"): a built-in
// clicked while it starts, whose start then times out. The click is
// answered "could not start" at the timeout, not before; the built-in
// restarts by its class (`Restart=on-failure`) 3 s after the reap, with no
// start retry 250 ms after it; and the fifth such timeout within 10 min
// ends in `Failed{start-limit-hit}`, after which nothing starts it again.
#[test]
fn p_builtin_clicked_while_its_start_times_out() {
    let mut sim = target();
    let start = sim.p.start_timeout;
    let u = sim.add(builtin("b"));
    sim.run();
    for i in 1..=5_u32 {
        if sim.st(u) == S::Verify {
            sim.verified(u, true);
        }
        let pid = match sim.st(u) {
            S::Spawn(pid) => pid,
            other => panic!("start {i}: {other:?}"),
        };
        sim.spawned(pid, true);
        let click = 10 + i;
        assert_eq!(sim.wake(pid, cmd(click)), Delivery::Held, "start {i}");
        let t = sim.now;
        assert!(
            answers(&sim.at(t + start - 1)).is_empty(),
            "start {i}: answered before the timeout"
        );
        let acts = sim.at(t + start);
        assert_eq!(
            answers(&acts),
            vec![(click, Why::CouldNotStart)],
            "start {i}"
        );
        assert_eq!(terminates(&acts).len(), 1, "start {i}");
        sim.after(200);
        let reap = sim.now;
        sim.gone(pid);
        if i < 5 {
            let S::AutoRestart(until) = sim.st(u) else {
                panic!("start {i}: {:?}", sim.st(u));
            };
            if i == 1 {
                assert_eq!(until, reap + 3_000, "the built-in's first backoff");
            }
            assert!(
                spawns(&sim.at(reap + 250)).is_empty(),
                "start {i}: no start retry after a timeout"
            );
            assert!(!spawns(&sim.at(until)).is_empty(), "start {i}: restarted");
        } else {
            assert_eq!(sim.st(u), S::Failed("startlimithit".into()));
            assert!(spawns(&sim.after(3_600_000)).is_empty());
        }
    }
}
