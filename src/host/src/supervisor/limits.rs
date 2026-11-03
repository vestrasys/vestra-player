//! Per-class policy (systemd unit settings) and the tunables. `Tunables::new`
//! is the one preset, for this machine's cores and memory; tests change single
//! values (short timers, small limits).
#![forbid(unsafe_code)]

use super::unit::{Class, Ms};

/// systemd `Restart=`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestartMode {
    /// Not restarted; registrations revoked; `Failed` (Development).
    No,
    /// Restarted by the supervisor after `RestartSec` (Builtin, Resident).
    OnFailure,
    /// Kept in standby with its entry points; starts again on the next
    /// trigger after a hold (Normal). Needs a checkpoint, else `No`.
    OnNextTrigger,
}

/// `RestartSec=` growing on repeats (`RestartSteps=`, `RestartMaxDelaySec=`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Backoff {
    pub base: Ms,
    pub cap: Option<Ms>,
    /// The streak resets after this long active.
    pub reset_after: Option<Ms>,
    /// The first restart is immediate (the runner).
    pub first_immediate: bool,
}

/// Retry of a failed spawn within one start job (a start timeout is not retried
/// since the B6 review).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Retry {
    pub count: u32,
    pub base: Ms,
    pub jitter_pct: u32,
}

/// `StartLimitBurst=` in `StartLimitIntervalSec=`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartLimit {
    pub burst: u32,
    pub interval: Ms,
}

/// One class's fixed settings (no per-plugin override).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    pub restart: RestartMode,
    pub restart_sec: Option<Backoff>,
    pub start_retry: Option<Retry>,
    pub start_limit: Option<StartLimit>,
    /// `TimeoutStartSec`.
    pub start_timeout: Ms,
    /// `TimeoutStopSec` for disable/reload/uninstall/upgrade.
    pub stop_grace: Ms,
    /// `TimeoutStopSec` for standby.
    pub standby_grace: Ms,
    /// `TimeoutStopSec` after a failure.
    pub failure_grace: Ms,
    /// Goes to standby when idle (never for Resident / Development, nor a
    /// unit classified resident).
    pub idle_standby: bool,
    /// Request timeout of in-flight work.
    pub request_timeout: Option<Ms>,
    /// `OOMScoreAdjust=`: higher goes to standby first under pressure.
    pub importance: i32,
    /// Counted in the third-party process budget.
    pub in_budget: bool,
    /// May be put in standby by the budget or pressure.
    pub reclaimable: bool,
    /// `MemoryMax=` (JS heap, MiB) of an own process; the adapter applies it.
    pub memory_max_mib: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tunables {
    pub builtin: Policy,
    pub resident: Policy,
    pub normal: Policy,
    pub development: Policy,
    /// Idle timeout (`IDLE_MS`).
    pub idle_timeout: Ms,
    /// Reap + backend cleanup (`STOP_WAIT` 20 s, one constant).
    pub stop_wait: Ms,
    pub verify_wait: Ms,
    pub classify_wait: Ms,
    /// `"Starting <name>…"` after (`STARTING_OSD_MS`).
    pub starting_notice: Ms,
    /// Wake events wake nothing this long after start-up settled.
    pub quiet_after_start: Ms,
    /// Third-party running start jobs: `min(4, max(2, cores/2))`.
    pub max_active: usize,
    /// Extra slots for `Interactive` (1) and its overrun (1, not under
    /// critical pressure).
    pub interactive_reserve: usize,
    pub interactive_overrun: usize,
    /// Held triggers per unit (64).
    pub backlog_max: usize,
    /// Waiting third-party start jobs (256).
    pub lane_max: usize,
    /// Third-party process budget.
    pub process_budget: usize,
    /// "Recently used" for importance (−100).
    pub recent: Ms,
    /// Runner fail-open: restarts within the window.
    pub runner_restart_limit: u32,
    pub runner_restart_window: Ms,
    /// Runner restart backoff: first immediate, then 3 s × 2ⁿ.
    pub runner_backoff: Backoff,
}

impl Policy {
    fn for_class(class: Class) -> Self {
        let retry = Some(Retry {
            count: 3,
            base: 250,
            jitter_pct: 10,
        });
        let limit = Some(StartLimit {
            burst: 5,
            interval: 600_000,
        });
        let third = Some(Backoff {
            base: 1_000,
            cap: Some(60_000),
            reset_after: Some(60_000),
            first_immediate: false,
        });
        let (restart, restart_sec) = match class {
            Class::Builtin => (
                RestartMode::OnFailure,
                Some(Backoff {
                    base: 3_000,
                    cap: None,
                    reset_after: Some(60_000),
                    first_immediate: false,
                }),
            ),
            Class::Resident => (RestartMode::OnFailure, third),
            Class::Normal => (RestartMode::OnNextTrigger, third),
            Class::Development => (RestartMode::No, None),
        };
        let installed = class != Class::Development;
        Self {
            restart,
            restart_sec,
            start_retry: retry.filter(|_| installed),
            start_limit: if installed {
                limit
            } else {
                Some(StartLimit {
                    burst: 1,
                    interval: 600_000,
                })
            },
            // A development load's start timeout is its own (1–120 s).
            start_timeout: if installed { 60_000 } else { 120_000 },
            stop_grace: 5_000,
            // `TimeoutStopSec` for standby; a development load never stands
            // by.
            standby_grace: if installed { 5_000 } else { 2_000 },
            failure_grace: 300,
            idle_standby: matches!(class, Class::Builtin | Class::Normal),
            request_timeout: installed.then_some(300_000),
            importance: match class {
                Class::Builtin => -1000,
                Class::Resident => -500,
                Class::Normal => 0,
                Class::Development => -1000,
            },
            in_budget: matches!(class, Class::Resident | Class::Normal),
            reclaimable: class == Class::Normal,
            memory_max_mib: installed.then_some(512),
        }
    }
}

impl Tunables {
    /// The values for a machine with `cores` logical cores and `physical_mib`
    /// of memory (`host/supervisor/mod.rs` `new`).
    pub fn new(cores: usize, physical_mib: u64) -> Self {
        Self {
            builtin: Policy::for_class(Class::Builtin),
            resident: Policy::for_class(Class::Resident),
            normal: Policy::for_class(Class::Normal),
            development: Policy::for_class(Class::Development),
            idle_timeout: 30_000,
            stop_wait: 20_000,
            verify_wait: 3_000,
            classify_wait: 5_000,
            starting_notice: 400,
            quiet_after_start: 1_000,
            max_active: max_active_for(cores),
            interactive_reserve: 1,
            interactive_overrun: 1,
            backlog_max: 64,
            lane_max: 256,
            process_budget: process_budget_for(physical_mib),
            recent: 60_000,
            runner_restart_limit: 3,
            runner_restart_window: 60_000,
            runner_backoff: Backoff {
                base: 3_000,
                cap: None,
                reset_after: Some(60_000),
                first_immediate: true,
            },
        }
    }

    /// The preset with other timer values, for tests written against them:
    /// idle 60 s, start 15 s, stop 300 ms, standby 2 s, no request timeout
    /// and no process budget. The rules are the preset's.
    #[cfg(test)]
    pub fn with_other_timers(mut self) -> Self {
        self.idle_timeout = 60_000;
        for policy in [&mut self.builtin, &mut self.resident, &mut self.normal] {
            policy.start_timeout = 15_000;
            policy.stop_grace = 300;
            policy.standby_grace = 2_000;
            policy.request_timeout = None;
        }
        self.process_budget = usize::MAX;
        self
    }

    pub fn policy(&self, class: Class) -> &Policy {
        match class {
            Class::Builtin => &self.builtin,
            Class::Resident => &self.resident,
            Class::Normal => &self.normal,
            Class::Development => &self.development,
        }
    }
}

/// `min(4, max(2, cores/2))`.
pub fn max_active_for(cores: usize) -> usize {
    (cores / 2).clamp(2, 4)
}

/// `clamp(physMiB / 2 × 0.30 / 85, 3, 16)`.
pub fn process_budget_for(physical_mib: u64) -> usize {
    let n = physical_mib / 2 * 30 / 100 / 85;
    (n as usize).clamp(3, 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formulas() {
        assert_eq!(max_active_for(1), 2);
        assert_eq!(max_active_for(8), 4);
        assert_eq!(max_active_for(16), 4);
        assert_eq!(process_budget_for(4_096), 7);
        assert_eq!(process_budget_for(1_024), 3);
        assert_eq!(process_budget_for(65_536), 16);
    }

    #[test]
    fn the_preset_carries_the_design_values() {
        let t = Tunables::new(8, 16_384);
        assert_eq!(t.idle_timeout, 30_000);
        assert_eq!(t.max_active, 4);
        assert_eq!(t.process_budget, process_budget_for(16_384));
        assert_eq!((t.backlog_max, t.lane_max), (64, 256));
        for class in [Class::Builtin, Class::Resident, Class::Normal] {
            let p = t.policy(class);
            assert_eq!(p.start_timeout, 60_000);
            assert_eq!((p.stop_grace, p.standby_grace), (5_000, 5_000));
            assert_eq!(p.request_timeout, Some(300_000));
            assert_eq!(p.start_retry.map(|r| r.count), Some(3));
            assert_eq!(p.memory_max_mib, Some(512));
        }
        assert_eq!(t.builtin.restart, RestartMode::OnFailure);
        assert_eq!(t.resident.restart, RestartMode::OnFailure);
        assert_eq!(t.normal.restart, RestartMode::OnNextTrigger);
        let d = t.policy(Class::Development);
        assert_eq!(d.restart, RestartMode::No);
        assert_eq!((d.start_timeout, d.stop_grace), (120_000, 5_000));
        assert_eq!(d.start_limit.map(|l| l.burst), Some(1));
        assert!(!t.resident.idle_standby && !d.idle_standby);
    }
}
