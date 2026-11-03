//! Restart and backoff (systemd `Restart=`, `RestartSec=`,
//! `RestartSteps=`/`RestartMaxDelaySec=`, `StartLimitBurst=`/
//! `StartLimitIntervalSec=`; Erlang restart intensity), the runner's
//! fail-open counter (mirrors `shared_runner::Policy`), and the seeded
//! xorshift used for jitter (as `bindings.rs` test `Steps`).
#![forbid(unsafe_code)]

use std::collections::VecDeque;

use super::limits::{Backoff, Retry, StartLimit};
use super::unit::Ms;

/// Per-unit restart bookkeeping.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct RestartState {
    /// Consecutive automatic restarts (backoff exponent).
    pub streak: u32,
    /// Start retries used by the current start job.
    pub retries: u32,
    /// Crash times inside the start-limit window.
    pub crashes: VecDeque<Ms>,
    /// When the current run became active.
    pub active_since: Option<Ms>,
}
impl RestartState {
    /// Records a crash at `now`; whether the start limit is hit.
    pub fn crash(&mut self, now: Ms, limit: Option<StartLimit>) -> bool {
        let Some(limit) = limit else {
            return false;
        };
        self.crashes.push_back(now);
        while self
            .crashes
            .front()
            .is_some_and(|t| now.saturating_sub(*t) >= limit.interval)
        {
            self.crashes.pop_front();
        }
        self.crashes.len() >= limit.burst as usize
    }
    /// The delay before the next automatic restart (or allowed re-wake).
    pub fn next_delay(&mut self, now: Ms, backoff: Option<Backoff>) -> Ms {
        let Some(backoff) = backoff else {
            return 0;
        };
        if let (Some(reset), Some(since)) = (backoff.reset_after, self.active_since)
            && now.saturating_sub(since) >= reset
        {
            self.streak = 0;
        }
        let n = self.streak;
        self.streak = self.streak.saturating_add(1);
        if backoff.first_immediate && n == 0 {
            return 0;
        }
        let n = if backoff.first_immediate { n - 1 } else { n };
        let delay = backoff.base.saturating_mul(1_u64 << n.min(32));
        backoff.cap.map_or(delay, |cap| delay.min(cap))
    }
}

/// A start retry's delay: `base × 2ⁿ` plus up to `jitter_pct` % (seeded).
pub(super) fn retry_delay(retry: Retry, n: u32, rng: &mut Rng) -> Ms {
    let delay = retry.base.saturating_mul(1_u64 << n.min(32));
    let jitter = delay * u64::from(retry.jitter_pct) / 100;
    delay
        + if jitter == 0 {
            0
        } else {
            rng.next() % (jitter + 1)
        }
}

/// Runner restarts within the window (`shared_runner::Policy::restart`):
/// fail-open when the limit is reached; not reset by stability.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct RunnerPolicy {
    restarts: VecDeque<Ms>,
    pub failed_open: bool,
}
impl RunnerPolicy {
    /// Records a runner restart; whether members now run in their own
    /// processes for the session.
    pub fn restart(&mut self, now: Ms, limit: u32, window: Ms) -> bool {
        self.restarts.push_back(now);
        while self
            .restarts
            .front()
            .is_some_and(|t| now.saturating_sub(*t) >= window)
        {
            self.restarts.pop_front();
        }
        if self.restarts.len() >= limit as usize {
            self.failed_open = true;
        }
        self.failed_open
    }
}

/// xorshift64 (deterministic simulations; never a security use).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_caps_and_resets_after_stability() {
        let backoff = Backoff {
            base: 1_000,
            cap: Some(60_000),
            reset_after: Some(60_000),
            first_immediate: false,
        };
        let mut state = RestartState::default();
        let delays = (0..8)
            .map(|_| state.next_delay(0, Some(backoff)))
            .collect::<Vec<_>>();
        assert_eq!(
            delays,
            [1_000, 2_000, 4_000, 8_000, 16_000, 32_000, 60_000, 60_000]
        );
        state.active_since = Some(100_000);
        assert_eq!(state.next_delay(160_000, Some(backoff)), 1_000);
    }

    #[test]
    fn runner_first_restart_immediate_then_backoff() {
        let backoff = Backoff {
            base: 3_000,
            cap: None,
            reset_after: Some(60_000),
            first_immediate: true,
        };
        let mut state = RestartState::default();
        assert_eq!(state.next_delay(0, Some(backoff)), 0);
        assert_eq!(state.next_delay(0, Some(backoff)), 3_000);
        assert_eq!(state.next_delay(0, Some(backoff)), 6_000);
    }

    #[test]
    fn start_limit_window_and_fail_open() {
        let limit = StartLimit {
            burst: 5,
            interval: 600_000,
        };
        let mut state = RestartState::default();
        for t in 0..4 {
            assert!(!state.crash(t * 1_000, Some(limit)));
        }
        assert!(state.crash(5_000, Some(limit)));
        let mut runner = RunnerPolicy::default();
        assert!(!runner.restart(0, 3, 60_000));
        assert!(!runner.restart(70_000, 3, 60_000));
        assert!(!runner.restart(80_000, 3, 60_000));
        assert!(runner.restart(90_000, 3, 60_000));
        assert!(runner.restart(500_000, 3, 60_000));
    }

    #[test]
    fn seeded_jitter_is_deterministic_and_bounded() {
        let retry = Retry {
            count: 3,
            base: 250,
            jitter_pct: 10,
        };
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        for n in 0..3 {
            let d = retry_delay(retry, n, &mut a);
            assert_eq!(d, retry_delay(retry, n, &mut b));
            let base = 250 << n;
            assert!((base..=base + base / 10).contains(&d));
        }
    }
}
