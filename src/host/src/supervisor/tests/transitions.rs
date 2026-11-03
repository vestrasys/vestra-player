//! Transition table, design one test per row. Rows whose expected values differ
//! between today's tunables and the target are generated for both profiles
//! (`::b1`, `::target`).
use super::*;

// (none) --add_unit, enabled, valid checkpoint, may go to standby--> Standby{holder: new pid, recorded}; Replay
spec! {
    fn t01_add_with_checkpoint_goes_to_standby_with_replay(p) {
        let mut sim = Sim::new(p);
        let u = sim.add(normal("a"));
        let acts = sim.run();
        let S::Standby { holder, hold_until } = sim.st(u) else { panic!("{:?}", sim.st(u)) };
        assert_eq!(hold_until, None);
        assert!(has(&acts, "Replay"), "{acts:?}");
        assert!(spawns(&acts).is_empty(), "no process for a unit in standby");
        assert_eq!(sim.unit_of(holder), Some(u), "holder pid is indexed by_pid");
        assert_eq!(sim.by_id("a"), Some(u));
        assert_eq!(sim.job(u), None);
    }
}

#[test]
fn t01_builtin_with_standby_checkpoint_goes_to_standby() {
    let mut sim = Sim::new(&Profile::other_timers());
    let u = sim.add(builtin_standby("b"));
    let acts = sim.run();
    assert!(matches!(sim.st(u), S::Standby { .. }));
    assert!(has(&acts, "Replay"));
}

// (none) --add_unit, enabled, no checkpoint / Resident / built-in not going to standby--> Inactive + Start job
#[test]
fn t02_add_without_checkpoint_installs_first_run_start() {
    let mut sim = Sim::new(&Profile::other_timers());
    let u = sim.add(first_run("a"));
    assert_eq!(sim.st(u), S::Inactive);
    assert!(
        matches!(sim.job(u), Some(J::Start { ref cause, running: false }) if cause == "firstrun"),
        "{:?}",
        sim.job(u)
    );
}

#[test]
fn t02_add_resident_installs_boot_start() {
    let mut sim = Sim::new(&Profile::other_timers());
    let u = sim.add(resident("r"));
    assert_eq!(sim.st(u), S::Inactive);
    assert!(
        matches!(sim.job(u), Some(J::Start { ref cause, .. }) if cause == "boot"),
        "{:?}",
        sim.job(u)
    );
}

#[test]
fn t02_add_builtin_not_going_to_standby_installs_start() {
    let mut sim = Sim::new(&Profile::other_timers());
    let u = sim.add(builtin("b"));
    assert_eq!(sim.st(u), S::Inactive);
    // A built-in recorded resident starts at boot; one without any
    // checkpoint would be a FirstRun. Either is a Start job.
    assert!(
        matches!(sim.job(u), Some(J::Start { ref cause, .. }) if cause == "boot" || cause == "firstrun")
    );
}

// (none) --add_unit, disabled--> Inactive, Disabled; no job
#[test]
fn t03_add_disabled_is_inactive_without_job() {
    let mut sim = Sim::new(&Profile::other_timers());
    let u = sim.add(normal("a").disabled());
    let acts = sim.run();
    assert_eq!(sim.st(u), S::Inactive);
    assert!(!sim.enabled(u));
    assert_eq!(sim.job(u), None);
    assert!(
        spawns(&acts).is_empty() && !has(&acts, "Replay"),
        "{acts:?}"
    );
}

// Inactive/Standby --Start runs, built-in needing digest check--> Activating{Verify}; Verify, deadline VERIFY_WAIT
#[test]
fn t04_builtin_start_with_digest_check_verifies_first() {
    let mut sim = Sim::new(&Profile::other_timers());
    let u = sim.add(builtin("b").shared(1).verify());
    let acts = sim.run();
    assert_eq!(sim.st(u), S::Verify);
    assert!(has(&acts, "Verify"));
    assert!(
        spawns(&acts).is_empty(),
        "no process before the digest check"
    );
    assert!(sim.next_deadline().is_some_and(|d| d <= T0 + VERIFY_WAIT));
}

// Activating{Verify} --verify done--> Activating{Spawn}; Spawn
#[test]
fn t05_verify_done_spawns() {
    let mut sim = Sim::new(&Profile::other_timers());
    let u = sim.add(builtin("b").shared(1).verify());
    sim.run();
    let acts = sim.verified(u, true);
    assert!(matches!(sim.st(u), S::Spawn(_)), "{:?}", sim.st(u));
    assert_eq!(spawns(&acts).len(), 1);
    assert!(
        !sim.own_process(u),
        "a verified built-in keeps its shared placement"
    );
}

// Activating{Verify} --3 s passed--> Activating{Spawn}, Placement::Own
#[test]
fn t05_verify_timeout_spawns_in_own_process() {
    let mut sim = Sim::new(&Profile::other_timers());
    let u = sim.add(builtin("b").shared(1).verify());
    sim.run();
    assert!(spawns(&sim.at(T0 + VERIFY_WAIT - 1)).is_empty());
    assert_eq!(sim.st(u), S::Verify);
    let acts = sim.at(T0 + VERIFY_WAIT);
    assert_eq!(spawns(&acts).len(), 1);
    assert!(matches!(sim.st(u), S::Spawn(_)));
    assert!(
        sim.own_process(u),
        "after 3 s without a verdict the built-in runs in its own process"
    );
}

// ANSWERED (supervisor.md), was a design question: says "verify done; or 3 s passed → Placement::Own". Does a
// *failed* verification also go on in an own process (today's `trust`), or does it fail the start? Best reading:
// own process.
#[test]
fn t05_verify_failed_spawns_in_own_process() {
    let mut sim = Sim::new(&Profile::other_timers());
    let u = sim.add(builtin("b").shared(1).verify());
    sim.run();
    let acts = sim.verified(u, false);
    assert_eq!(spawns(&acts).len(), 1);
    assert!(sim.own_process(u));
}

// Inactive --Start runs (no verify)--> Activating{Spawn, pid: alloc}; Spawn
#[test]
fn t06_start_from_inactive_spawns_new_pid() {
    let mut sim = Sim::new(&Profile::other_timers());
    let u = sim.add(resident("r"));
    let acts = sim.run();
    let S::Spawn(pid) = sim.st(u) else {
        panic!("{:?}", sim.st(u))
    };
    assert_eq!(spawns(&acts), vec![pid]);
    assert_eq!(sim.unit_of(pid), Some(u));
    assert!(matches!(sim.job(u), Some(J::Start { running: true, .. })));
}

// Standby --Start runs--> Activating{Spawn}; Spawn, Rekey(holder → pid), forward holder → pid
#[test]
fn t06_start_from_standby_rekeys_holder_to_new_pid() {
    let mut sim = Sim::new(&Profile::other_timers());
    let (u, holder) = sim.in_standby(normal("a"));
    sim.start_interactive(u).unwrap();
    let acts = sim.run();
    let S::Spawn(pid) = sim.st(u) else {
        panic!("{:?}", sim.st(u))
    };
    assert_ne!(pid, holder, "every run gets a new pid");
    assert_eq!(spawns(&acts), vec![pid]);
    assert!(
        acts.iter().any(|a| rekey(a) == Some((holder, pid))),
        "{acts:?}"
    );
    sim.spawned(pid, true);
    sim.ready(pid);
    assert!(
        matches!(sim.wake(holder, cmd(7)), Delivery::Forward(p) if p == pid),
        "old pid forwards to the new one"
    );
}

// Activating{Spawn} --process_spawned(Ok)--> Activating{Ready}; arm the start timeout
spec! {
    fn t07_spawned_ok_waits_for_ready_with_start_timeout(p) {
        let mut sim = Sim::new(p);
        let u = sim.add(resident("r"));
        sim.run();
        let pid = sim.pid(u);
        sim.spawned(pid, true);
        assert_eq!(sim.st(u), S::Ready(pid));
        sim.after(STARTING_OSD); // past any "Starting…" OSD timer
        assert_eq!(sim.next_deadline(), Some(T0 + p.start_timeout));
    }
}

// Activating{Spawn} --process_spawned(Err)--> retries left: AutoRestart{until: start backoff}
#[test]
fn t08_spawn_error_with_retries_left_waits_start_backoff() {
    let p = Profile::target();
    let mut sim = Sim::new(&p);
    let (u, _) = sim.in_standby(normal("a"));
    sim.start_interactive(u).unwrap();
    sim.run();
    let acts = sim.spawned(sim.pid(u), false);
    let S::AutoRestart(until) = sim.st(u) else {
        panic!("{:?}", sim.st(u))
    };
    // 250 ms × 2⁰ + up to 10 % jitter.
    assert!((T0 + 250..=T0 + 275).contains(&until), "until = {until}");
    assert!(
        answers(&acts).is_empty(),
        "no answer while retries are left"
    );
}

// … else Normal/Resident with recording → Standby{hold_until}; Answer held interactive triggers; Notify
#[test]
fn t08_spawn_error_without_retries_recorded_unit_holds_in_standby() {
    let p = Profile::target();
    let mut sim = Sim::new(&p);
    let (u, holder) = sim.in_standby(normal("a"));
    sim.settle();
    assert!(matches!(sim.wake(holder, cmd(1)), Delivery::Started));
    let mut acts = sim.run();
    for _ in 0..=p.start_retries {
        let pid = match sim.st(u) {
            S::Spawn(pid) => pid,
            S::AutoRestart(until) => {
                acts.extend(sim.at(until));
                sim.pid(u)
            }
            other => panic!("{other:?}"),
        };
        acts.extend(sim.spawned(pid, false));
    }
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
    assert!(has(&acts, "Notify"));
    assert!(!has(&acts, "Revoke"), "registrations kept");
}

// … else Failed{spawn}
#[test]
fn t08_spawn_error_without_retries_or_recording_fails() {
    let p = Profile::target();
    let mut sim = Sim::new(&p);
    let u = sim.add(first_run("a"));
    sim.run();
    for _ in 0..=p.start_retries {
        let pid = match sim.st(u) {
            S::Spawn(pid) => pid,
            S::AutoRestart(until) => {
                sim.at(until);
                sim.pid(u)
            }
            other => panic!("{other:?}"),
        };
        sim.spawned(pid, false);
    }
    assert_eq!(sim.st(u), S::Failed("spawn".into()));
}

// Without spawn retries: a recorded unit keeps its entry points in
// Standby (no Revoke) and the held interactive trigger is answered.
#[test]
fn t08_spawn_error_answers_and_keeps_registrations() {
    let mut sim = Sim::new(&Profile::other_timers());
    let (u, holder) = sim.in_standby(normal("a"));
    sim.settle();
    sim.wake(holder, cmd(1));
    sim.wake(holder, event("playback.state", 2));
    sim.run();
    let acts = sim.spawn_fails(u);
    assert!(matches!(sim.st(u), S::Standby { .. }), "{:?}", sim.st(u));
    assert!(!has(&acts, "Revoke"), "{:?}", names(&acts));
    assert_eq!(answers(&acts), vec![(1, Why::CouldNotStart)]);
    assert!(acts.iter().any(is_could_not_start_notice));
}

// Activating{Ready} --process_ready--> Active; Deliver backlog in order; arm idle; job done
spec! {
    fn t09_ready_delivers_backlog_in_order_and_arms_idle(p) {
        let mut sim = Sim::new(p);
        let (u, holder) = sim.in_standby(normal("a"));
        sim.settle();
        sim.wake(holder, cmd(1));
        sim.run();
        sim.wake(holder, cmd(2));
        sim.wake(holder, resolve(3));
        let pid = sim.pid(u);
        sim.spawned(pid, true);
        sim.after(STARTING_OSD); // let the "Starting…" OSD timer go
        let t = sim.now;
        let acts = sim.ready(pid);
        assert_eq!(sim.st(u), S::Active(pid));
        assert_eq!(deliveries(&acts), vec![1, 2, 3]);
        assert!(acts.iter().filter_map(delivered).all(|(to, _)| to == pid));
        assert_eq!(sim.job(u), None, "Start job done");
        assert_eq!(sim.next_deadline(), Some(t + p.idle));
    }
}

// detail: "process_ready = runtime.ready + UI activation finished". ANSWERED (supervisor.md), was a design
// question: lists `ui_begin`/`ui_finish` and `process_ready` separately; best reading: a unit that began UI
// activation stays Activating{Ready} until `ui_finish` even after `process_ready`.
#[test]
fn t09_ready_waits_for_ui_activation() {
    let mut sim = Sim::new(&Profile::other_timers());
    let u = sim.add(resident("r"));
    sim.run();
    let pid = sim.pid(u);
    sim.spawned(pid, true);
    sim.ui_begin(pid);
    sim.ready(pid);
    assert_eq!(sim.st(u), S::Ready(pid));
    sim.ui_finish(pid);
    assert_eq!(sim.st(u), S::Active(pid));
}

// Activating{Ready} --process_failed (runtime.failed)--> Deactivating{Failed, then: Failed}; Terminate{Failed}; no retry
spec! {
    fn t10_runtime_failed_terminates_and_fails_without_retry(p) {
        let mut sim = Sim::new(p);
        let u = sim.add(builtin("b"));
        sim.run();
        let pid = sim.pid(u);
        sim.spawned(pid, true);
        let acts = sim.failed(pid);
        assert_eq!(sim.st(u).then(), Some(Then::Failed));
        let t = terminates(&acts);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].2, "failed");
        sim.gone(pid);
        assert!(sim.st(u).is_failed());
        // No retry, ever.
        let later = sim.after(3_600_000);
        assert!(spawns(&later).is_empty());
        assert!(sim.st(u).is_failed());
    }
}

// Activating{Spawn|Ready} --the start timeout--> Deactivating{Failed, then per restart rule}; Terminate{Failed}
spec! {
    fn t11_start_timeout_terminates(p) {
        let mut sim = Sim::new(p);
        let u = sim.add(resident("r"));
        sim.run();
        let pid = sim.pid(u);
        sim.spawned(pid, true);
        assert!(terminates(&sim.at(T0 + p.start_timeout - 1)).is_empty(), "not before the deadline");
        let acts = sim.at(T0 + p.start_timeout);
        assert!(sim.st(u).is_deactivating(), "{:?}", sim.st(u));
        let t = terminates(&acts);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].2, "failed");
        assert_eq!(t[0].0, p.failure_grace, "grace after a failure is 300 ms (KEEP)");
    }
}

#[test]
fn t11_start_timeout_target_restarts_by_its_class() {
    let p = Profile::target();
    let mut sim = Sim::new(&p);
    let u = sim.add(resident("r"));
    sim.run();
    let pid = sim.pid(u);
    sim.at(T0 + p.start_timeout);
    sim.gone(pid);
    // StartTimeout: no start retry; a Resident restarts by its class's
    // restart rule (on failure), so `AutoRestart`.
    assert!(matches!(sim.st(u), S::AutoRestart(_)), "{:?}", sim.st(u));
}

// Active --classified "can go to standby"--> Deactivating{Idle, then: Standby}; Checkpoint; Terminate{Idle, keep}
spec! {
    fn t12_first_run_classified_standby_checkpoints_and_stops(p) {
        let mut sim = Sim::new(p);
        let (u, pid) = sim.in_active(first_run("a"));
        let acts = sim.classified(pid, false);
        assert_eq!(sim.st(u).then(), Some(Then::Standby));
        let (c, t) = (pos(&acts, "Checkpoint").expect("Checkpoint"), pos(&acts, "Terminate").expect("Terminate"));
        assert!(c < t, "checkpoint before terminate: {:?}", names(&acts));
        let term = terminates(&acts)[0].clone();
        assert_eq!(term.2, "idle");
        assert!(term.1, "keep_registrations");
        assert_eq!(term.0, p.standby_grace);
        sim.gone(pid);
        assert!(matches!(sim.st(u), S::Standby { holder, hold_until: None } if holder == pid), "holder = last run's pid");
    }
}

// Active --classified resident--> Active, class Resident; Checkpoint (resident)
#[test]
fn t13_first_run_classified_resident_stays_active() {
    let mut sim = Sim::new(&Profile::other_timers());
    let (u, pid) = sim.in_active(first_run("a"));
    let acts = sim.classified(pid, true);
    assert_eq!(sim.st(u), S::Active(pid));
    assert_eq!(sim.class(u), Class::Resident);
    assert!(has(&acts, "Checkpoint"));
    assert!(terminates(&acts).is_empty());
}

// sidebar-tabs.md (user ruling 2026-11-03): twenty plugins whose only panel
// is a sidebar tab. The host answers "live" only for the current tab's
// (`sidebar_tabs::live`), so one is Resident and the nineteen background
// tabs' plugins are Normal: each goes to standby, and an idle one later too.
#[test]
fn twenty_sidebar_tab_plugins_are_not_resident_unless_current() {
    use crate::ui::{composition::Slot, sidebar_tabs};
    let p = Profile::other_timers();
    let mut sim = Sim::new(&p);
    let current = Some(0);
    let mut resident = 0;
    for n in 0..20u64 {
        let (u, pid) = sim.in_active(first_run(&format!("tab{n}")));
        let live = sidebar_tabs::live(Slot::Sidebar, n, current);
        let acts = sim.classified(pid, live);
        if live {
            assert_eq!(sim.class(u), Class::Resident);
            resident += 1;
        } else {
            assert_ne!(sim.class(u), Class::Resident, "tab{n}");
            assert_eq!(sim.st(u).then(), Some(Then::Standby), "tab{n}");
            assert_eq!(terminates(&acts)[0].2, "idle");
            sim.gone(pid);
            assert!(matches!(sim.st(u), S::Standby { .. }), "tab{n}");
        }
    }
    assert_eq!(resident, 1);
    // A background tab's plugin woken and idle again stands by again.
    let (u, pid) = sim.in_active(normal("woken"));
    sim.after(p.idle);
    let acts = sim.resources(pid, false);
    assert_eq!(sim.st(u).then(), Some(Then::Standby));
    assert_eq!(terminates(&acts)[0].2, "idle");
}

// Active --idle timer → Standby job → QueryResources → not busy--> Deactivating{Idle, then: Standby}; Checkpoint; Terminate{Idle, keep}
spec! {
    fn t14_idle_not_busy_goes_to_standby(p) {
        let mut sim = Sim::new(p);
        let (u, pid) = sim.in_active(normal("a"));
        let t = sim.now;
        assert!(!has(&sim.at(t + p.idle - 1), "QueryResources"), "not before idle timeout");
        let q = sim.at(t + p.idle);
        assert_eq!(q.iter().filter_map(query_pid).collect::<Vec<_>>(), vec![pid]);
        assert!(matches!(sim.job(u), Some(J::Standby { .. })));
        let acts = sim.resources(pid, false);
        assert_eq!(sim.st(u).then(), Some(Then::Standby));
        assert!(pos(&acts, "Checkpoint").unwrap() < pos(&acts, "Terminate").unwrap());
        let term = terminates(&acts)[0].clone();
        assert_eq!((term.0, term.1, term.2.as_str()), (p.standby_grace, true, "idle"));
    }
}

// Active --idle timer, busy--> Active; Standby job cancelled; idle re-armed
spec! {
    fn t15_idle_but_busy_stays_active_and_rearms(p) {
        let mut sim = Sim::new(p);
        let (u, pid) = sim.in_active(normal("a"));
        sim.after(p.idle);
        let t = sim.now;
        let acts = sim.resources(pid, true);
        assert_eq!(sim.st(u), S::Active(pid));
        assert!(terminates(&acts).is_empty());
        assert_eq!(sim.job(u), None, "Standby job cancelled");
        assert_eq!(sim.next_deadline(), Some(t + p.idle));
    }
}

// "`work.rs` empty" is part of the busy check.
spec! {
    fn t15_in_flight_work_counts_as_busy(p) {
        let mut sim = Sim::new(p);
        let (u, pid) = sim.in_active(normal("a"));
        sim.begin_work(pid);
        sim.after(p.idle);
        let acts = sim.resources(pid, false);
        assert_eq!(sim.st(u), S::Active(pid), "in-flight work keeps it active");
        assert!(terminates(&acts).is_empty());
    }
}

// Deactivating --process_reaped and backend_cleaned both in--> then; waiting job runnable
spec! {
    fn t16_both_halves_complete_deactivation(p) {
        let mut sim = Sim::new(p);
        let (u, pid) = sim.in_active(normal("a"));
        sim.stop(u, StopReason::Disable, Then::Inactive).unwrap();
        sim.run();
        sim.reaped(pid);
        assert!(sim.st(u).is_deactivating(), "reaped alone is not enough: {:?}", sim.st(u));
        sim.cleaned(pid);
        assert_eq!(sim.st(u), S::Inactive);
    }
}

// Deactivating{Term} --the stop grace--> Deactivating{Kill}; Kill
spec! {
    fn t17_stop_grace_expiry_kills(p) {
        let mut sim = Sim::new(p);
        let (u, pid) = sim.in_active(normal("a"));
        sim.stop(u, StopReason::Disable, Then::Inactive).unwrap();
        sim.run();
        let t = sim.now;
        assert_eq!(sim.st(u), S::Term(Then::Inactive));
        assert!(!has(&sim.at(t + p.stop_grace - 1), "Kill"));
        let acts = sim.at(t + p.stop_grace);
        assert_eq!(acts.iter().filter_map(kill_pid).collect::<Vec<_>>(), vec![pid]);
        assert_eq!(sim.st(u), S::Kill(Then::Inactive));
    }
}

// Deactivating --20 s, not reaped/cleaned--> then, flagged stuck; waiting job fails with today's text
#[test]
fn t18_stuck_stop_fails_waiting_reload_with_text() {
    let mut sim = Sim::new(&Profile::other_timers());
    let (u, pid) = sim.in_active(normal("a"));
    sim.stop(u, StopReason::Disable, Then::Inactive).unwrap();
    sim.run();
    let t = sim.now;
    let job = sim.restart(u, true, None).unwrap();
    sim.reaped(pid); // backend never cleans
    sim.at(t + STOP_WAIT - 1);
    assert!(sim.st(u).is_deactivating());
    sim.at(t + STOP_WAIT);
    assert!(sim.stuck(u));
    assert_eq!(sim.result(job), Some(R::Failed(TEXT_RELOAD_STUCK.into())));
    // ANSWERED (supervisor.md), was a design question: says "`Failed` for a reload" while says the unit "stays where
    // `then` put it". Best reading: a reload whose stop got stuck leaves the unit Failed.
    assert!(sim.st(u).is_failed(), "{:?}", sim.st(u));
}

// The interactive trigger is answered.
#[test]
fn t18_stuck_standby_stop_fails_waiting_wake_and_answers() {
    let mut sim = Sim::new(&Profile::other_timers());
    let (u, pid) = sim.deactivating_to_standby(normal("a"));
    let t = sim.now;
    sim.wake(pid, cmd(1));
    sim.cleaned(pid); // never reaped
    let acts = sim.at(t + STOP_WAIT);
    assert!(sim.stuck(u));
    assert_eq!(answers(&acts), vec![(1, Why::CouldNotStart)]);
    assert!(
        spawns(&acts).is_empty(),
        "no second instance while the old one may still run"
    );
    // lands at `then`, then the wake's failed Start marks the unit Failed with
    // the wake text (today `wake_failed`). A failed start keeps its entry
    // points after a spawn failure; a stuck stop
    // is not one: the old process may still run, so keeping its entry points in
    // Standby would let the next trigger start a second instance on the same
    // data.
    assert_eq!(sim.st(u), S::Failed("stuck".into()));
    assert_eq!(sim.failure_text(u).as_deref(), Some(TEXT_WAKE_STUCK));
    assert!(
        has(&acts, "Revoke"),
        "entry points revoked: {:?}",
        names(&acts)
    );
}

// Standby --wake(pid, trigger)--> Standby + Start job (trigger in backlog)
spec! {
    fn t19_wake_in_standby_queues_start(p) {
        let mut sim = Sim::new(p);
        let (u, holder) = sim.in_standby(normal("a"));
        sim.settle();
        assert!(matches!(sim.wake(holder, cmd(1)), Delivery::Started));
        assert!(matches!(sim.st(u), S::Standby { .. }), "state unchanged until the job runs");
        assert!(matches!(sim.job(u), Some(J::Start { running: false, .. })));
        assert_eq!(sim.backlog_len(u), 1);
        assert!(matches!(sim.wake(holder, cmd(2)), Delivery::Held));
        assert_eq!(sim.backlog_len(u), 2);
    }
}

// Standby{hold_until > now} --wake--> Standby; Start waits until hold_until; interactive answered, event kept
#[test]
fn t20_wake_during_hold_waits_and_answers_interactive() {
    let p = Profile::target();
    let mut sim = Sim::new(&p);
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
    assert!(until > sim.now);
    sim.wake(holder, cmd(1));
    sim.wake(holder, event("playback.state", 2));
    let acts = sim.run();
    assert_eq!(
        answers(&acts),
        vec![(1, Why::Held)],
        "interactive answered with the reason (the hold)"
    );
    // Coordinator ruling: the interactive one is not held; the event is.
    assert_eq!(sim.backlog_len(u), 1);
    assert!(matches!(sim.job(u), Some(J::Start { .. })));
    assert!(spawns(&acts).is_empty());
    assert!(
        spawns(&sim.at(until - 1)).is_empty(),
        "no start before hold_until"
    );
    let acts = sim.at(until);
    let new = spawns(&acts);
    assert_eq!(new.len(), 1);
    sim.spawned(new[0], true);
    let acts = sim.ready(new[0]);
    assert_eq!(
        deliveries(&acts),
        vec![2],
        "event kept in backlog and delivered"
    );
}

// Standby --Stop job (disable)--> Inactive, disabled; Revoke; checkpoint kept; backlog answered
#[test]
fn t21_disable_in_standby_revokes_keeps_checkpoint() {
    let mut sim = Sim::new(&Profile::other_timers());
    let (u, holder) = sim.in_standby(normal("a"));
    sim.settle();
    sim.wake(holder, cmd(1));
    sim.set_enabled(u, false);
    sim.stop(u, StopReason::Disable, Then::Inactive).unwrap();
    let acts = sim.run();
    assert_eq!(sim.st(u), S::Inactive);
    assert!(!sim.enabled(u));
    assert!(has(&acts, "Revoke"));
    assert!(!has(&acts, "DropCheckpoint"), "checkpoint kept");
    assert!(spawns(&acts).is_empty());
    assert_eq!(answers(&acts), vec![(1, Why::Disabled)]);
}

// Standby --Restart fresh (reload, upgrade)--> Inactive + Start part; Revoke; DropCheckpoint
#[test]
fn t22_reload_in_standby_drops_checkpoint_and_starts_fresh() {
    let mut sim = Sim::new(&Profile::other_timers());
    let (u, _) = sim.in_standby(normal("a"));
    sim.restart(u, true, None).unwrap();
    let acts = sim.run();
    assert!(has(&acts, "Revoke"));
    assert!(has(&acts, "DropCheckpoint"));
    assert!(pos(&acts, "DropCheckpoint").unwrap() < pos(&acts, "Spawn").unwrap_or(usize::MAX));
    assert!(
        matches!(sim.st(u), S::Inactive | S::Spawn(_)),
        "{:?}",
        sim.st(u)
    );
}

// Standby --Stop then: Removed (uninstall)--> removed after RemoveData; Revoke; DropCheckpoint; RemoveData
#[test]
fn t23_uninstall_in_standby_removes_data() {
    let mut sim = Sim::new(&Profile::other_timers());
    let (u, _) = sim.in_standby(normal("a"));
    sim.stop(u, StopReason::Uninstall, Then::Removed).unwrap();
    let acts = sim.run();
    for n in ["Revoke", "DropCheckpoint", "RemoveData"] {
        assert!(has(&acts, n), "{n} missing: {:?}", names(&acts));
    }
    assert!(pos(&acts, "Revoke").unwrap() < pos(&acts, "RemoveData").unwrap());
    sim.remove(u);
    assert_eq!(sim.by_id("a"), None);
}

// Active/Activating --Stop job--> Deactivating{reason, then}; Terminate{reason, grace: the stop grace}
spec! {
    fn t24_stop_active_terminates_with_stop_grace(p) {
        let mut sim = Sim::new(p);
        let (u, pid) = sim.in_active(normal("a"));
        sim.stop(u, StopReason::Disable, Then::Inactive).unwrap();
        let acts = sim.run();
        assert_eq!(sim.st(u), S::Term(Then::Inactive));
        assert_eq!(terminates(&acts), vec![(p.stop_grace, false, "disable".to_string(), pid)]);
    }
}

spec! {
    fn t24_stop_activating_terminates(p) {
        let mut sim = Sim::new(p);
        let u = sim.add(resident("r"));
        sim.run();
        let pid = sim.pid(u);
        sim.spawned(pid, true);
        sim.stop(u, StopReason::Uninstall, Then::Removed).unwrap();
        let acts = sim.run();
        assert_eq!(sim.st(u), S::Term(Then::Removed));
        assert_eq!(terminates(&acts), vec![(p.stop_grace, false, "uninstall".to_string(), pid)]);
    }
}

// target: Builtin/Resident → Restart job (→ AutoRestart)
#[test]
fn t25_crash_target_builtin_and_resident_auto_restart() {
    for spec in [builtin("b"), resident("r")] {
        let mut sim = Sim::new(&Profile::target());
        let (u, pid) = sim.in_active(spec.clone());
        let acts = sim.exited(pid, ExitCause::Fault);
        assert!(has(&acts, "Notify"), "{spec:?}");
        sim.gone(pid);
        assert!(
            matches!(sim.st(u), S::AutoRestart(_)),
            "{spec:?}: {:?}",
            sim.st(u)
        );
    }
}

// target: Normal → Standby{hold_until}, registrations kept
#[test]
fn t25_crash_target_normal_holds_in_standby_keeping_registrations() {
    let mut sim = Sim::new(&Profile::target());
    let (u, pid) = sim.in_active(normal("n"));
    let t = sim.now;
    let mut acts = sim.exited(pid, ExitCause::Exited);
    acts.extend(sim.gone(pid));
    assert_eq!(
        sim.st(u),
        S::Standby {
            holder: pid,
            hold_until: Some(t + 1_000)
        }
    );
    assert!(!has(&acts, "Revoke"));
    assert!(
        terminates(&acts).iter().all(|t| t.1),
        "UI cleanup keeps registrations"
    );
    assert!(has(&acts, "Notify"));
}

// target: Development → Failed
#[test]
fn t25_crash_target_development_fails() {
    let mut sim = Sim::new(&Profile::target());
    let (u, pid) = sim.in_active(dev("d", 120_000));
    sim.exited(pid, ExitCause::Exited);
    sim.gone(pid);
    assert!(sim.st(u).is_failed());
}

// target: Oom notifies "ran out of memory"
#[test]
fn t25_oom_notifies_out_of_memory() {
    let mut sim = Sim::new(&Profile::target());
    let (_, pid) = sim.in_active(normal("n"));
    let acts = sim.exited(pid, ExitCause::Oom);
    assert!(acts.iter().any(is_oom_notice), "{acts:?}");
}

// target: RequestTimeout (a timer; Terminate first)
#[test]
fn t25_request_timeout_terminates_then_as_crash() {
    let p = Profile::target();
    let mut sim = Sim::new(&p);
    let (u, pid) = sim.in_active(normal("n"));
    sim.begin_work(pid);
    let t = sim.now;
    let acts = sim.at(t + p.request_timeout.unwrap());
    assert_eq!(terminates(&acts).len(), 1, "Terminate first");
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
}

// Activating{AutoRestart} --the restart delay timer--> Activating{Verify|Spawn}; Spawn
#[test]
fn t26_auto_restart_spawns_when_due() {
    let mut sim = Sim::new(&Profile::target());
    let (u, pid) = sim.in_active(builtin("b"));
    sim.exited(pid, ExitCause::Exited);
    sim.gone(pid);
    let S::AutoRestart(until) = sim.st(u) else {
        panic!("{:?}", sim.st(u))
    };
    assert!(spawns(&sim.at(until - 1)).is_empty());
    let acts = sim.at(until);
    assert_eq!(spawns(&acts).len(), 1);
    assert!(matches!(sim.st(u), S::Spawn(_)));
}

// any start path --start limit exceeded--> Failed{start-limit-hit}; Notify; failures text
#[test]
fn t27_start_limit_hit_fails_normal() {
    let mut sim = Sim::new(&Profile::target());
    let (u, _) = sim.in_active(normal("n"));
    for crash in 1..=5 {
        let pid = sim.pid(u);
        sim.exited(pid, ExitCause::Exited);
        sim.gone(pid);
        if crash < 5 {
            let S::Standby {
                hold_until: Some(until),
                ..
            } = sim.st(u)
            else {
                panic!("crash {crash}: {:?}", sim.st(u))
            };
            sim.start_interactive(u).unwrap();
            sim.at(until);
            sim.bring_up(u);
        }
    }
    assert_eq!(sim.st(u), S::Failed("startlimithit".into()));
    assert!(sim.failure_text(u).is_some_and(|t| !t.is_empty()));
    assert!(has(&sim.log, "Notify"));
}

// Failed --reset_failed--> Inactive; failure text cleared
#[test]
fn t28_reset_failed_clears_failure() {
    let mut sim = Sim::new(&Profile::other_timers());
    let (u, pid) = sim.in_active(normal("n"));
    // An own process's transport failure fails it for good.
    sim.exited(pid, ExitCause::Transport);
    sim.gone(pid);
    assert!(sim.st(u).is_failed());
    sim.reset_failed(u);
    assert_eq!(sim.st(u), S::Inactive);
    assert_eq!(sim.failure_text(u), None);
}

// reset is implicit in reload (Restart{fresh}).
#[test]
fn t28_reload_of_failed_unit_resets_and_starts() {
    let mut sim = Sim::new(&Profile::other_timers());
    let (u, pid) = sim.in_active(normal("n"));
    sim.exited(pid, ExitCause::Exited);
    sim.gone(pid);
    sim.restart(u, true, None).unwrap();
    let acts = sim.run();
    assert_eq!(spawns(&acts).len(), 1);
    assert!(matches!(sim.st(u), S::Spawn(_)));
}

// Active (member) --runner hang--> Deactivating{Failed, Cleanup}, reaped set; KillRunner once; Restart jobs, first immediate
#[test]
fn t29_runner_failure_kills_runner_once_and_restarts_members() {
    let mut sim = Sim::new(&Profile::target());
    let (a, pa) = sim.in_active(builtin("a").shared(1));
    let (b, pb) = sim.in_active(builtin("b").shared(1));
    let acts = sim.runner_hung(1);
    assert_eq!(count(&acts, "KillRunner"), 1);
    assert_eq!(sim.st(a), S::Cleanup(sim.st(a).then().unwrap()));
    assert_eq!(sim.st(b), S::Cleanup(sim.st(b).then().unwrap()));
    sim.cleaned(pa);
    sim.cleaned(pb);
    // First runner restart: immediate, no time passes.
    assert!(
        matches!(sim.st(a), S::Spawn(_)) && matches!(sim.st(b), S::Spawn(_)),
        "{:?} {:?}",
        sim.st(a),
        sim.st(b)
    );
}

// Active --pressure or over budget; selected; not busy--> as (see budget.rs for selection)
#[test]
fn t30_pressure_standby_uses_the_standby_job() {
    let mut sim = Sim::new(&Profile::target());
    let (u, pid) = sim.in_active(normal("n"));
    sim.after(1_000); // not in the same millisecond as its start
    let acts = sim.pressure_low();
    assert_eq!(
        acts.iter().filter_map(query_pid).collect::<Vec<_>>(),
        vec![pid]
    );
    assert!(matches!(sim.job(u), Some(J::Standby { .. })));
    let acts = sim.resources(pid, false);
    assert_eq!(sim.st(u).then(), Some(Then::Standby));
    assert!(pos(&acts, "Checkpoint").unwrap() < pos(&acts, "Terminate").unwrap());
}

// any --shutdown(now, budget)--> Deactivating{Quit}; Checkpoint; Terminate{Quit} all in parallel; one deadline; Kill
#[test]
fn t31_shutdown_terminates_all_in_one_call() {
    let mut sim = Sim::new(&Profile::other_timers());
    let (_, pa) = sim.in_active(normal("a"));
    let (_, pb) = sim.in_active(builtin("b"));
    let acts = sim.shutdown(1_000);
    let t = terminates(&acts);
    assert_eq!(t.len(), 2);
    assert!(t.iter().all(|t| t.2 == "quit"));
    let pids = t.iter().map(|t| t.3).collect::<Vec<_>>();
    assert!(same_set(&pids, &[pa, pb]), "{pids:?}");
}

// Inactive, disabled --set_enabled(true)--> Inactive, enabled + Start job ANSWERED (supervisor.md), was a design
// question: says `set_enabled` "does not enqueue by itself", says `set_enabled(true)` → "enabled + Start job". Best
// reading: the caller (package RPC) does `set_enabled(true)` then `start(u, Enable)`.
#[test]
fn t32_enable_then_start_installs_start_job() {
    let mut sim = Sim::new(&Profile::other_timers());
    let u = sim.add(normal("a").disabled());
    sim.run();
    sim.set_enabled(u, true);
    assert!(sim.enabled(u));
    assert_eq!(sim.job(u), None, "set_enabled does not enqueue by itself");
    sim.start(u, Cause::Enable).unwrap();
    assert!(matches!(sim.job(u), Some(J::Start { ref cause, .. }) if cause == "enable"));
    let acts = sim.run();
    // Recorded unit: enable goes back to standby via replay or starts;
    // either way something happens and no error.
    assert!(has(&acts, "Spawn") || has(&acts, "Replay"), "{acts:?}");
}

// "(waits if a previous process is still Deactivating)" — bug fix.
spec! {
    fn t32_enable_while_disabling_waits_for_reap(p) {
        let mut sim = Sim::new(p);
        let (u, pid) = sim.in_active(normal("a"));
        sim.set_enabled(u, false);
        sim.stop(u, StopReason::Disable, Then::Inactive).unwrap();
        sim.run();
        sim.set_enabled(u, true);
        sim.start(u, Cause::Enable).unwrap();
        let acts = sim.after(p.stop_grace);
        assert!(spawns(&acts).is_empty(), "no second process on the same data directory");
        assert!(spawns(&sim.reaped(pid)).is_empty(), "reaped alone is not enough");
        let acts = sim.cleaned(pid);
        assert!(has(&acts, "Spawn") || has(&acts, "Replay"), "start runs once the old process is gone: {acts:?}");
    }
}
