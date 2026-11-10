//! Display pacing for mpv's display-synced timing (`video-sync=display-*`,
//! which interpolation uses): the render thread starts frames on a grid one
//! display period apart. The period counts from one frame's start to the
//! next, so the time a render takes is inside it, not added to it (counted
//! from a frame's end, a 60 Hz display got about 35 frames a second when a
//! render took 12 ms, macOS M2, 2026-11-10). A frame that comes more than a
//! period late (mpv had nothing to draw, a pause, a seek) starts a new grid.

use std::time::{Duration, Instant};

#[derive(Debug, Default)]
pub(crate) struct Cadence {
    /// When the next frame may start.
    due: Option<Instant>,
}
impl Cadence {
    /// How long to wait before asking mpv for the next frame (zero: now,
    /// and always when unpaced).
    pub(crate) fn wait(&self, period: Option<Duration>, now: Instant) -> Duration {
        match (period, self.due) {
            (Some(_), Some(due)) => due.saturating_duration_since(now),
            _ => Duration::ZERO,
        }
    }
    /// A frame starts rendering at `now`.
    pub(crate) fn started(&mut self, period: Option<Duration>, now: Instant) {
        self.due = period.map(|period| match self.due {
            Some(due) if now >= due && now < due + period => due + period,
            _ => now + period,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PERIOD: Duration = Duration::from_millis(16);
    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn frames_start_a_period_apart_whatever_the_render_takes() {
        let (mut cadence, t0) = (Cadence::default(), Instant::now());
        assert_eq!(cadence.wait(Some(PERIOD), t0), Duration::ZERO, "the first frame at once");
        cadence.started(Some(PERIOD), t0);
        // The render took 12 ms: 4 ms to the next start, not 16.
        assert_eq!(cadence.wait(Some(PERIOD), t0 + ms(12)), ms(4));
        cadence.started(Some(PERIOD), t0 + ms(16));
        // mpv's frame came 3 ms after the slot: the grid holds.
        cadence.started(Some(PERIOD), t0 + ms(35));
        assert_eq!(cadence.wait(Some(PERIOD), t0 + ms(40)), ms(8));
        let starts: Vec<_> = (0..60).map(|i| t0 + ms(48) + PERIOD * i).collect();
        for &start in &starts {
            assert_eq!(cadence.wait(Some(PERIOD), start), Duration::ZERO);
            cadence.started(Some(PERIOD), start);
        }
        assert_eq!(cadence.wait(Some(PERIOD), *starts.last().unwrap()), PERIOD);
    }

    #[test]
    fn a_late_frame_starts_a_new_grid_and_unpaced_never_waits() {
        let (mut cadence, t0) = (Cadence::default(), Instant::now());
        cadence.started(Some(PERIOD), t0);
        // Paused for a second: no burst of catch-up frames after it.
        cadence.started(Some(PERIOD), t0 + ms(1000));
        assert_eq!(cadence.wait(Some(PERIOD), t0 + ms(1001)), ms(15));
        cadence.started(None, t0 + ms(1016));
        assert_eq!(cadence.wait(None, t0 + ms(1017)), Duration::ZERO);
        assert_eq!(cadence.wait(Some(PERIOD), t0 + ms(1017)), Duration::ZERO);
    }
}
