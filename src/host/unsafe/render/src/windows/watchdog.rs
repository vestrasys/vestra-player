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
//! render thread sits inside one call into mpv's renderer *and does not run*:
//! a thread that keeps using the CPU inside the call is compiling shaders
//! (mpv compiles a user shader inside the first render call that needs it;
//! Direct3D 11's compiler took 50 s to over a minute for the shader
//! upscaling plugin's ArtCNN networks on the RTX 4070 machine, gpu-probe,
//! 2026-11-01), which is not a stall. Within one call the looks at which the
//! thread did not run add up (a running look pauses the clock, it does not
//! reset it, so a driver that polls still stalls), and a call that keeps
//! running stalls only after [`Watch::call_cap`].
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
    /// The CPU time the render thread has used so far.
    pub cpu: Duration,
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
    /// A render call that does not return, its thread not running, for this
    /// long stalls (longer than `limit`: a driver may wait inside it).
    pub call_limit: Duration,
    /// A render call that does not return for this long stalls even though
    /// its thread keeps running (a shader that never finishes compiling).
    pub call_cap: Duration,
    /// The most one observation adds to a stall clock.
    pub cap: Duration,
    ready: Track,
    done: Track,
    /// Inside one call: the time the thread did not run, added up.
    call: Track,
    /// Inside one call, running or not.
    call_total: Track,
    last: Option<Instant>,
    last_cpu: Duration,
}

/// The render thread counts as running (compiling) between two looks when
/// it used at least this share of the time between them on the CPU.
const RUNNING_SHARE: u32 = 5;

impl Watch {
    pub fn new(limit: Duration) -> Self {
        Self {
            limit,
            call_limit: limit * 3,
            call_cap: limit * 30,
            cap: Duration::from_millis(250),
            ready: Track::default(),
            done: Track::default(),
            call: Track::default(),
            call_total: Track::default(),
            last: None,
            last_cpu: Duration::ZERO,
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
        let inside = sample.call % 2 == 1;
        // At least 1/RUNNING_SHARE of the time since the last look on the
        // CPU (20 % of one core): compiling, not waiting.
        let used = sample.cpu.saturating_sub(std::mem::replace(&mut self.last_cpu, sample.cpu));
        let running = !added.is_zero() && used * RUNNING_SHARE >= added;
        // Idle time inside one call adds up, running time pauses it: a
        // driver that polls (or a thread that runs now and then) does not
        // start the clock again, while a compile, running almost all the
        // time, stays far below the limit.
        let call_stuck =
            self.call.step(inside, sample.call, if running { Duration::ZERO } else { added });
        let call_total = self.call_total.step(inside, sample.call, added);
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
        if call_total >= self.call_cap {
            return Some(format!(
                "GPU rendering stalled: mpv's render call did not return for {} s while it kept the CPU busy (a shader that does not finish compiling)",
                seconds(call_total)
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
        let stuck = Sample { ready: (5, 4), done: (3, 3), call: 2, cpu: Duration::ZERO };
        assert_eq!(run(&mut watch, &mut now, 9.5, stuck), None);
        let reason = run(&mut watch, &mut now, 1., stuck).expect("stalled");
        assert!(reason.contains("mpv's frames") && reason.contains("4 of 5"), "{reason}");
        let mut watch = Watch::new(Duration::from_secs(10));
        let draws = Sample { ready: (5, 5), done: (9, 8), call: 2, cpu: Duration::ZERO };
        let reason = run(&mut watch, &mut now, 11., draws).expect("stalled");
        assert!(reason.contains("windows' draws"), "{reason}");
    }

    #[test]
    fn progress_pause_and_idle_never_stall() {
        let mut watch = Watch::new(Duration::from_secs(10));
        let mut now = Instant::now();
        // Paused, seeking, hidden: nothing outstanding for a minute.
        let idle = Sample { ready: (7, 7), done: (4, 4), call: 6, cpu: Duration::ZERO };
        assert_eq!(run(&mut watch, &mut now, 60., idle), None);
        // A slow GPU: always one frame behind, but each one finishes.
        for frame in 8..200u64 {
            let busy =
                Sample { ready: (frame, frame - 1), done: (4, 4), call: 6, cpu: Duration::ZERO };
            assert_eq!(run(&mut watch, &mut now, 1., busy), None, "frame {frame}");
        }
        // The fences released after a failure (u64::MAX) are never behind.
        let released =
            Sample { ready: (300, u64::MAX), done: (9, u64::MAX), call: 6, cpu: Duration::ZERO };
        assert_eq!(run(&mut watch, &mut now, 30., released), None);
    }

    #[test]
    fn a_render_call_that_does_not_return_stalls_after_three_limits() {
        let mut watch = Watch::new(Duration::from_secs(10));
        let mut now = Instant::now();
        let inside = Sample { ready: (5, 5), done: (5, 5), call: 11, cpu: Duration::ZERO };
        assert_eq!(run(&mut watch, &mut now, 29., inside), None, "a wait in the driver");
        let reason = run(&mut watch, &mut now, 2., inside).expect("stalled");
        assert!(reason.contains("render call"), "{reason}");
        // Calls that return (a new value each time) never stall.
        let mut watch = Watch::new(Duration::from_secs(10));
        for call in 0..2000u64 {
            let sample = Sample { ready: (5, 5), done: (5, 5), call, cpu: Duration::ZERO };
            assert_eq!(run(&mut watch, &mut now, 0.1, sample), None);
        }
    }

    /// The 0.6.2 "device loss" with the shader upscaling plugin: mpv compiled
    /// ArtCNN inside its first render call for about a minute, the render
    /// thread busy on the CPU all along, and the watchdog called that a stall
    /// after 30 s (a loss in the crash guard, the player restarted, which
    /// compiled again). A call whose thread runs is compiling: it stalls only
    /// at the cap; once the thread stops running, the usual limit applies.
    #[test]
    fn a_render_call_busy_compiling_stalls_only_at_the_cap() {
        let mut watch = Watch::new(Duration::from_secs(10));
        let mut now = Instant::now();
        let mut cpu = Duration::ZERO;
        let mut compiling = |watch: &mut Watch, now: &mut Instant, seconds: f64, share: f64| {
            let mut reason = None;
            for _ in 0..(seconds / POLL.as_secs_f64()).round() as u32 {
                *now += POLL;
                cpu += POLL.mul_f64(share);
                let sample = Sample { ready: (5, 5), done: (5, 5), call: 11, cpu };
                reason = reason.or(watch.observe(*now, sample));
            }
            reason
        };
        // A minute and more on one core, or a quarter of one: compiling.
        assert_eq!(compiling(&mut watch, &mut now, 70., 1.), None);
        assert_eq!(compiling(&mut watch, &mut now, 60., 0.25), None);
        // Then waiting in the driver, the thread idle: the usual limit.
        assert_eq!(compiling(&mut watch, &mut now, 29., 0.), None);
        let reason = compiling(&mut watch, &mut now, 2., 0.).expect("stalled");
        assert!(reason.contains("did not return for 30 s") && !reason.contains("CPU"), "{reason}");
        // Busy for the whole cap (300 s at the default limit): a stall.
        let mut watch = Watch::new(Duration::from_secs(10));
        assert_eq!(watch.call_cap, Duration::from_secs(300));
        assert_eq!(compiling(&mut watch, &mut now, 299., 1.), None);
        let reason = compiling(&mut watch, &mut now, 2., 1.).expect("stalled at the cap");
        assert!(reason.contains("kept the CPU busy"), "{reason}");
    }

    /// Review M3: a driver that polls inside the call (one busy look in ten)
    /// must not start the idle clock again each time: the idle looks add
    /// up, so the stall comes after about 30 s of idle time, not at the cap.
    #[test]
    fn a_polling_driver_inside_the_call_still_stalls_at_the_idle_limit() {
        let mut watch = Watch::new(Duration::from_secs(10));
        let mut now = Instant::now();
        let mut cpu = Duration::ZERO;
        let mut stalled_at = None;
        for look in 1..=600u32 {
            now += POLL;
            if look % 10 == 0 {
                cpu += POLL; // one look in ten fully on the CPU
            }
            let sample = Sample { ready: (5, 5), done: (5, 5), call: 11, cpu };
            if let Some(reason) = watch.observe(now, sample) {
                stalled_at = Some((look, reason));
                break;
            }
        }
        let (look, reason) = stalled_at.expect("a polling driver never stalled");
        // 30 s of idle looks among 90 % idle: about 33 s.
        assert!((330..=340).contains(&look), "after {look} looks: {reason}");
        assert!(!reason.contains("CPU"), "{reason}");
    }

    #[test]
    fn a_suspended_computer_adds_at_most_the_cap_per_look() {
        let mut watch = Watch::new(Duration::from_secs(10));
        let mut now = Instant::now();
        let behind = Sample { ready: (5, 4), done: (3, 3), call: 2, cpu: Duration::ZERO };
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
