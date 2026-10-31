//! When GPU rendering stopped making progress (pure; tested below; round
//! record 2026-10-render-modes §11).
//!
//! A driver can stop a queue without resetting the GPU: a shared fence that
//! is waited for but never reached (a cross-API deadlock) is not a hang to
//! Windows' timeout detection (TDR), so no device is lost and nothing else
//! notices. The renderer's two shared fences tell whether work is moving:
//!
//! - `ready`: mpv's device signals it after each frame;
//! - `done`: the windows' queue signals it after each draw that showed a
//!   frame.
//!
//! A fence *stalls* while a value was enqueued, the GPU has not reached it,
//! and the value the GPU reached does not change. Paused, seeking or
//! buffering playback and hidden windows enqueue nothing, so nothing is
//! outstanding and nothing stalls; a slow GPU still reaches each frame's
//! value, so the stall clock starts again. A stall also counts while the
//! render thread sits inside one call into mpv's renderer.
//!
//! The stall clock adds at most [`Watch::cap`] per observation, so a
//! suspended computer (no observations for minutes) does not count as a
//! stall on its first observation after waking.

use std::time::{Duration, Instant};

/// One look at the renderer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sample {
    /// `ready`: the last value enqueued, and the value the GPU reached.
    pub ready: (u64, u64),
    /// `done`: the same.
    pub done: (u64, u64),
    /// mpv's render calls: odd while the render thread is inside one, and
    /// a new value for each call.
    pub call: u64,
}

#[derive(Clone, Copy, Debug, Default)]
struct Track {
    /// What was last seen (the reached fence value, or the call counter).
    seen: u64,
    stuck: Duration,
}

impl Track {
    /// `outstanding`: work is waiting for the GPU (or the thread is inside
    /// a call); `seen`: the value that shows progress when it changes.
    fn step(&mut self, outstanding: bool, seen: u64, added: Duration) -> Duration {
        if !outstanding || seen != self.seen {
            *self = Self { seen, stuck: Duration::ZERO };
        } else {
            self.stuck += added;
        }
        self.stuck
    }
}

/// The stall clocks of one renderer.
#[derive(Clone, Debug)]
pub struct Watch {
    /// A fence that does not move for this long stalls.
    pub limit: Duration,
    /// A render call that does not return for this long stalls (longer:
    /// the first frame after a new shader compiles inside the call).
    pub call_limit: Duration,
    /// The most one observation adds to a stall clock.
    pub cap: Duration,
    ready: Track,
    done: Track,
    call: Track,
    last: Option<Instant>,
}

impl Watch {
    pub fn new(limit: Duration) -> Self {
        Self {
            limit,
            call_limit: limit * 3,
            cap: Duration::from_millis(250),
            ready: Track::default(),
            done: Track::default(),
            call: Track::default(),
            last: None,
        }
    }

    /// Look at the renderer now: why it stalled, once a clock reached its
    /// limit.
    pub fn observe(&mut self, now: Instant, sample: Sample) -> Option<String> {
        let added = self
            .last
            .replace(now)
            .map_or(Duration::ZERO, |last| now.saturating_duration_since(last).min(self.cap));
        let (ready, reached) = sample.ready;
        let ready_stuck = self.ready.step(reached < ready, reached, added);
        let (done, drawn) = sample.done;
        let done_stuck = self.done.step(drawn < done, drawn, added);
        let call_stuck = self.call.step(sample.call % 2 == 1, sample.call, added);
        let seconds = |d: Duration| d.as_secs();
        if ready_stuck >= self.limit {
            return Some(format!(
                "GPU rendering stalled: mpv's frames did not finish on the GPU for {} s (fence ready at {reached} of {ready})",
                seconds(ready_stuck)
            ));
        }
        if done_stuck >= self.limit {
            return Some(format!(
                "GPU rendering stalled: the windows' draws did not finish on the GPU for {} s (fence done at {drawn} of {done})",
                seconds(done_stuck)
            ));
        }
        if call_stuck >= self.call_limit {
            return Some(format!(
                "GPU rendering stalled: mpv's render call did not return for {} s",
                seconds(call_stuck)
            ));
        }
        None
    }
}

/// `VESTRA_GPU_WATCHDOG`: seconds without progress before GPU rendering
/// counts as stalled; `0` turns the watchdog off; unset or not a number:
/// [`DEFAULT_LIMIT`].
pub fn limit(value: Option<&str>) -> Option<Duration> {
    match value.map(str::trim).map(str::parse::<f64>) {
        Some(Ok(seconds)) if seconds.is_finite() && seconds >= 0. => {
            (seconds > 0.).then(|| Duration::from_secs_f64(seconds.min(3600.)))
        }
        _ => Some(DEFAULT_LIMIT),
    }
}

/// Conservative: Windows resets a GPU that hangs for 2 s (TDR), and no
/// single frame of a working GPU takes seconds.
pub const DEFAULT_LIMIT: Duration = Duration::from_secs(10);

#[cfg(test)]
mod tests {
    use super::*;

    const POLL: Duration = Duration::from_millis(100);

    /// Observe `sample` every `POLL` for `seconds`; the first reason.
    fn run(watch: &mut Watch, start: &mut Instant, seconds: f64, sample: Sample) -> Option<String> {
        let mut reason = None;
        for _ in 0..(seconds / POLL.as_secs_f64()).round() as u32 {
            *start += POLL;
            reason = reason.or(watch.observe(*start, sample));
        }
        reason
    }

    #[test]
    fn a_fence_that_does_not_move_stalls_after_the_limit() {
        let mut watch = Watch::new(Duration::from_secs(10));
        let mut now = Instant::now();
        let stuck = Sample { ready: (5, 4), done: (3, 3), call: 2 };
        assert_eq!(run(&mut watch, &mut now, 9.5, stuck), None);
        let reason = run(&mut watch, &mut now, 1., stuck).expect("stalled");
        assert!(reason.contains("mpv's frames") && reason.contains("4 of 5"), "{reason}");
        let mut watch = Watch::new(Duration::from_secs(10));
        let draws = Sample { ready: (5, 5), done: (9, 8), call: 2 };
        let reason = run(&mut watch, &mut now, 11., draws).expect("stalled");
        assert!(reason.contains("windows' draws"), "{reason}");
    }

    #[test]
    fn progress_pause_and_idle_never_stall() {
        let mut watch = Watch::new(Duration::from_secs(10));
        let mut now = Instant::now();
        // Paused, seeking, hidden: nothing outstanding for a minute.
        let idle = Sample { ready: (7, 7), done: (4, 4), call: 6 };
        assert_eq!(run(&mut watch, &mut now, 60., idle), None);
        // A slow GPU: always one frame behind, but each one finishes.
        for frame in 8..200u64 {
            let busy = Sample { ready: (frame, frame - 1), done: (4, 4), call: 6 };
            assert_eq!(run(&mut watch, &mut now, 1., busy), None, "frame {frame}");
        }
        // The fences released after a failure (u64::MAX) are never behind.
        let released = Sample { ready: (300, u64::MAX), done: (9, u64::MAX), call: 6 };
        assert_eq!(run(&mut watch, &mut now, 30., released), None);
    }

    #[test]
    fn a_render_call_that_does_not_return_stalls_after_three_limits() {
        let mut watch = Watch::new(Duration::from_secs(10));
        let mut now = Instant::now();
        let inside = Sample { ready: (5, 5), done: (5, 5), call: 11 };
        assert_eq!(run(&mut watch, &mut now, 29., inside), None, "a long shader compile");
        let reason = run(&mut watch, &mut now, 2., inside).expect("stalled");
        assert!(reason.contains("render call"), "{reason}");
        // Calls that return (a new value each time) never stall.
        let mut watch = Watch::new(Duration::from_secs(10));
        for call in 0..2000u64 {
            let sample = Sample { ready: (5, 5), done: (5, 5), call };
            assert_eq!(run(&mut watch, &mut now, 0.1, sample), None);
        }
    }

    #[test]
    fn a_suspended_computer_adds_at_most_the_cap_per_look() {
        let mut watch = Watch::new(Duration::from_secs(10));
        let mut now = Instant::now();
        let behind = Sample { ready: (5, 4), done: (3, 3), call: 2 };
        assert_eq!(watch.observe(now, behind), None);
        // Asleep for an hour; on waking the GPU has not caught up yet.
        now += Duration::from_secs(3600);
        assert_eq!(watch.observe(now, behind), None);
        // It catches up at once: the clock starts again.
        now += POLL;
        let caught_up = Sample { ready: (5, 5), ..behind };
        assert_eq!(watch.observe(now, caught_up), None);
        assert_eq!(run(&mut watch, &mut now, 20., caught_up), None);
    }

    #[test]
    fn the_limit_reads_the_environment_value() {
        assert_eq!(limit(None), Some(DEFAULT_LIMIT));
        assert_eq!(limit(Some("0")), None, "off");
        assert_eq!(limit(Some(" 3 ")), Some(Duration::from_secs(3)));
        assert_eq!(limit(Some("0.5")), Some(Duration::from_millis(500)));
        assert_eq!(limit(Some("soon")), Some(DEFAULT_LIMIT));
        assert_eq!(limit(Some("-1")), Some(DEFAULT_LIMIT));
    }
}
