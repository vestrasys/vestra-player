//! Per-class policy (systemd unit settings, §3.3) and the tunables (§11).
//! `Tunables::today()` reproduces today's decisions (B1–B2); every later
//! batch's change is a value here, not a code path of its own.
#![forbid(unsafe_code)]

use super::unit::{Class, Ms};

/// systemd `Restart=` (§3.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestartMode {
    /// Not restarted; registrations revoked; `Failed` (today, every class).
    No,
    /// Restarted by the supervisor after `RestartSec` (Builtin, Resident; B4).
    OnFailure,
    /// Kept in standby with its entry points; starts again on the next
    /// trigger after a hold (Normal; B4). Needs a checkpoint, else `No`.
    OnNextTrigger,
}

/// `RestartSec=` growing on repeats (`RestartSteps=`, `RestartMaxDelaySec=`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Backoff {
    pub base: Ms,
    pub cap: Option<Ms>,
    /// The streak resets after this long active.
    pub reset_after: Option<Ms>,
    /// The first restart is immediate (the runner, A4).
    pub first_immediate: bool,
}

/// Retry of a failed spawn / start timeout within one start job.
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
    /// `TimeoutStopSec` for standby (today `PARK_WAIT`).
    pub standby_grace: Ms,
    /// `TimeoutStopSec` after a failure.
    pub failure_grace: Ms,
    /// Goes to standby when idle (never for Resident / Development).
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

/// §11.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tunables {
    /// On-demand start (today `VESTRA_LAZY=1`): replay at `add_unit`, first
    /// runs, idle standby. Off: every enabled unit starts at boot.
    pub on_demand: bool,
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
    /// "Starting <name>…" after (`STARTING_OSD_MS`).
    pub starting_notice: Ms,
    /// Wake events wake nothing this long after start-up settled.
    pub quiet_after_start: Ms,
    /// How long a Standby job waits for the resources answer (`None`: as
    /// today, until it comes).
    pub resources_wait: Option<Ms>,
    /// First runs in flight (`FIRST_RUNS` 8; `None` from B3).
    pub first_run_limit: Option<usize>,
    /// Third-party running start jobs (`None` = today's unlimited wakes;
    /// B3: `min(4, max(2, cores/2))`).
    pub max_active: Option<usize>,
    /// Extra slots for `Interactive` (B3: 1) and its overrun (B3: 1, not
    /// under critical pressure).
    pub interactive_reserve: usize,
    pub interactive_overrun: usize,
    /// Held triggers per unit (`None` today; B3: 64).
    pub backlog_max: Option<usize>,
    /// Waiting third-party start jobs (`None` today; B3: 256).
    pub lane_max: Option<usize>,
    /// Third-party process budget (`None` until B5).
    pub process_budget: Option<usize>,
    /// "Recently used" for importance (−100).
    pub recent: Ms,
    /// A failed start (spawn error, start timeout, stuck stop wait) keeps
    /// the entry points in standby (T8, B3) instead of revoking (today).
    pub start_failure_keeps_registrations: bool,
    /// Runner fail-open: restarts within the window.
    pub runner_restart_limit: u32,
    pub runner_restart_window: Ms,
    /// Runner restart backoff (B4: first immediate, then 3 s × 2ⁿ).
    pub runner_backoff: Option<Backoff>,
}

impl Policy {
    fn today(class: Class) -> Self {
        Self {
            restart: RestartMode::No,
            restart_sec: None,
            start_retry: None,
            start_limit: None,
            start_timeout: match class {
                Class::Development => 120_000,
                _ => 15_000,
            },
            stop_grace: 300,
            standby_grace: 2_000,
            failure_grace: 300,
            idle_standby: matches!(class, Class::Builtin | Class::Normal),
            request_timeout: None,
            importance: match class {
                Class::Builtin => -1000,
                Class::Resident => -500,
                Class::Normal => 0,
                Class::Development => -1000,
            },
            in_budget: matches!(class, Class::Resident | Class::Normal),
            reclaimable: class == Class::Normal,
            memory_max_mib: None,
        }
    }
}

impl Tunables {
    /// Today's values (B1–B2): the core reproduces today's decisions.
    pub fn today() -> Self {
        Self {
            on_demand: true,
            builtin: Policy::today(Class::Builtin),
            resident: Policy::today(Class::Resident),
            normal: Policy::today(Class::Normal),
            development: Policy::today(Class::Development),
            idle_timeout: 60_000,
            stop_wait: 20_000,
            verify_wait: 3_000,
            classify_wait: 5_000,
            starting_notice: 400,
            quiet_after_start: 1_000,
            resources_wait: None,
            first_run_limit: Some(8),
            max_active: None,
            interactive_reserve: 0,
            interactive_overrun: 0,
            backlog_max: None,
            lane_max: None,
            process_budget: None,
            recent: 60_000,
            start_failure_keeps_registrations: false,
            runner_restart_limit: 3,
            runner_restart_window: 60_000,
            runner_backoff: None,
        }
    }

    /// B3 values on top of `self`: queue limits live.
    pub fn with_b3(mut self, cores: usize) -> Self {
        self.first_run_limit = None;
        self.max_active = Some(max_active_for(cores));
        self.interactive_reserve = 1;
        self.interactive_overrun = 1;
        self.backlog_max = Some(64);
        self.lane_max = Some(256);
        self.start_failure_keeps_registrations = true;
        self
    }

    /// B4 values on top of `self`: restart policy, backoff, start limits,
    /// heap cap.
    pub fn with_b4(mut self) -> Self {
        let retry = Some(Retry {
            count: 3,
            base: 250,
            jitter_pct: 10,
        });
        let limit = Some(StartLimit {
            burst: 5,
            interval: 600_000,
        });
        let third = Backoff {
            base: 1_000,
            cap: Some(60_000),
            reset_after: Some(60_000),
            first_immediate: false,
        };
        self.builtin.restart = RestartMode::OnFailure;
        self.builtin.restart_sec = Some(Backoff {
            base: 3_000,
            cap: None,
            reset_after: Some(60_000),
            first_immediate: false,
        });
        self.builtin.start_retry = retry;
        self.builtin.start_limit = limit;
        self.builtin.memory_max_mib = Some(512);
        self.resident.restart = RestartMode::OnFailure;
        self.resident.restart_sec = Some(third);
        self.resident.start_retry = retry;
        self.resident.start_limit = limit;
        self.resident.memory_max_mib = Some(512);
        self.normal.restart = RestartMode::OnNextTrigger;
        self.normal.restart_sec = Some(third);
        self.normal.start_retry = retry;
        self.normal.start_limit = limit;
        self.normal.memory_max_mib = Some(512);
        self.development.start_limit = Some(StartLimit {
            burst: 1,
            interval: 600_000,
        });
        self.runner_backoff = Some(Backoff {
            base: 3_000,
            cap: None,
            reset_after: Some(60_000),
            first_immediate: true,
        });
        self
    }

    /// B5: the process budget for this machine.
    pub fn with_b5(mut self, physical_mib: u64) -> Self {
        self.process_budget = Some(process_budget_for(physical_mib));
        self
    }

    /// B6 targets (§11) on top of B3–B5.
    pub fn target(cores: usize, physical_mib: u64) -> Self {
        let mut t = Self::today().with_b3(cores).with_b4().with_b5(physical_mib);
        t.idle_timeout = 30_000;
        for policy in [&mut t.builtin, &mut t.resident, &mut t.normal] {
            policy.start_timeout = 60_000;
            policy.stop_grace = 5_000;
            policy.standby_grace = 5_000;
            policy.request_timeout = Some(300_000);
        }
        t
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
impl Default for Tunables {
    fn default() -> Self {
        Self::today()
    }
}

/// `min(4, max(2, cores/2))` (§4.4).
pub fn max_active_for(cores: usize) -> usize {
    (cores / 2).clamp(2, 4)
}

/// `clamp(physMiB / 2 × 0.30 / 85, 3, 16)` (§8).
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
    fn today_reproduces_todays_values() {
        let t = Tunables::today();
        assert_eq!(t.idle_timeout, 60_000);
        assert_eq!(t.normal.start_timeout, 15_000);
        assert_eq!(t.normal.stop_grace, 300);
        assert_eq!(t.normal.standby_grace, 2_000);
        assert_eq!(t.first_run_limit, Some(8));
        assert_eq!(t.max_active, None);
        assert_eq!(t.process_budget, None);
        assert_eq!(t.backlog_max, None);
        for class in [
            Class::Builtin,
            Class::Resident,
            Class::Normal,
            Class::Development,
        ] {
            assert_eq!(t.policy(class).restart, RestartMode::No);
            assert_eq!(t.policy(class).restart_sec, None);
        }
        assert!(!t.start_failure_keeps_registrations);
    }
}
