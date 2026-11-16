//! Windows: the host's tick keeps running while a window is moved or sized.
//!
//! Holding a window's title bar or border puts the UI thread into the
//! system's modal move/size loop (`DefWindowProc`), which dispatches
//! messages itself; winit serves its timers only in its own wait, so
//! Slint's timers, the 16 ms tick among them (`main.rs`), stop until the
//! loop ends (10 s seen in a host log: plugin requests unanswered, tool
//! windows not laid out). Messages posted to the thread still arrive, so
//! inside the loop a thread timer (`vestra_win32::on_move_size`) posts the
//! tick through `slint::invoke_from_event_loop`, which runs it from winit's
//! own event handler as any posted call. This is the decision part: when a
//! timer post is due and when an arriving post runs the tick. It runs no
//! tick outside the loop, never two posts at once, never a tick sooner than
//! [`MIN_GAP`] after the last one began, and none at all once the normal
//! timer is seen ticking inside the loop (then nothing is stalled). Each
//! loop's numbers ([`LoopSummary`]) make one host log line under
//! `VESTRA_FRAME_LOG=1` (`windows.rs`).
#![forbid(unsafe_code)]

use std::time::{Duration, Instant};

/// The thread timer's period inside the loop. Windows rounds it up to its
/// timer resolution (15.6 ms by default).
pub(super) const PERIOD_MS: u32 = 10;
/// A tick posted from the loop runs only this long after the last one
/// began: about the normal tick's 16 ms period, never faster.
pub(super) const MIN_GAP: Duration = Duration::from_millis(14);

/// Where the keep-alive stands (UI thread).
#[derive(Debug, Default)]
pub(super) struct KeepAlive {
    /// Inside a move/size loop.
    moving: bool,
    /// The normal timer ticked inside this loop: it is not stalled.
    normal_inside: bool,
    /// A post is on its way (one at a time).
    posted: bool,
    /// When the last tick (either kind) began.
    last_began: Option<Instant>,
    /// When this loop started.
    entered_at: Option<Instant>,
    /// This loop's numbers so far.
    summary: LoopSummary,
}

/// What happened during one move/size loop.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct LoopSummary {
    /// From `WM_ENTERSIZEMOVE` to `WM_EXITSIZEMOVE`.
    pub(super) took: Duration,
    /// Ticks posted from the thread timer.
    pub(super) posts: u32,
    /// Ticks those posts ran.
    pub(super) kept: u32,
    /// Ticks of the normal timer inside the loop.
    pub(super) normal: u32,
    /// The longest time without a tick starting: from the last tick before
    /// the loop (or its start) through each tick to the loop's end.
    pub(super) longest_gap: Duration,
}

impl KeepAlive {
    /// A move/size loop started at `now`.
    pub(super) fn entered(&mut self, now: Instant) {
        self.moving = true;
        self.normal_inside = false;
        self.entered_at = Some(now);
        self.summary = LoopSummary::default();
    }
    /// It ended at `now`: a post still on its way runs no tick. The loop's
    /// numbers; `None` when no loop was on.
    pub(super) fn exited(&mut self, now: Instant) -> Option<LoopSummary> {
        if !self.moving {
            return None;
        }
        self.moving = false;
        self.gap_to(now);
        let entered = self.entered_at.take().unwrap_or(now);
        self.summary.took = now.saturating_duration_since(entered);
        Some(self.summary)
    }
    /// A tick began at `now`; `kept` when the keep-alive ran it.
    pub(super) fn tick_began(&mut self, now: Instant, kept: bool) {
        if self.moving {
            self.gap_to(now);
            if kept {
                self.summary.kept += 1;
            } else {
                self.summary.normal += 1;
                self.normal_inside = true;
            }
        }
        self.last_began = Some(now);
    }
    /// Inside a loop: the time without a tick up to `now` counts towards
    /// the longest gap.
    fn gap_to(&mut self, now: Instant) {
        if let Some(since) = self.last_began.or(self.entered_at) {
            let gap = now.saturating_duration_since(since);
            self.summary.longest_gap = self.summary.longest_gap.max(gap);
        }
    }
    fn due(&self, now: Instant) -> bool {
        self.moving
            && !self.normal_inside
            && self
                .last_began
                .is_none_or(|began| now.saturating_duration_since(began) >= MIN_GAP)
    }
    /// The thread timer fired at `now`: whether to post a tick.
    pub(super) fn timer(&mut self, now: Instant) -> bool {
        if self.posted || !self.due(now) {
            return false;
        }
        self.posted = true;
        self.summary.posts += 1;
        true
    }
    /// A post could not be sent (the event loop has ended).
    pub(super) fn not_posted(&mut self) {
        self.posted = false;
        self.summary.posts = self.summary.posts.saturating_sub(1);
    }
    /// The post arrived at `now`: whether it runs the tick.
    pub(super) fn arrived(&mut self, now: Instant) -> bool {
        self.posted = false;
        self.due(now)
    }
}

#[cfg(test)]
mod tests {
    use super::{KeepAlive, LoopSummary, MIN_GAP};
    use std::time::{Duration, Instant};

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn nothing_is_posted_outside_a_move_size_loop() {
        let mut keep = KeepAlive::default();
        let start = Instant::now();
        keep.tick_began(start, false);
        for step in 1..100 {
            assert!(!keep.timer(start + step * 10 * MS));
        }
        // A late timer message after the loop ended.
        keep.entered(start);
        keep.exited(start);
        assert!(!keep.timer(start + 2000 * MS));
    }

    #[test]
    fn inside_the_loop_one_post_at_a_time_and_no_faster_than_the_tick() {
        let mut keep = KeepAlive::default();
        let start = Instant::now();
        keep.tick_began(start, false);
        keep.entered(start);
        assert!(!keep.timer(start + 5 * MS), "a tick began 5 ms ago");
        assert!(keep.timer(start + MIN_GAP));
        assert!(!keep.timer(start + 30 * MS), "coalesced: one on its way");
        assert!(!keep.timer(start + 45 * MS));
        let arrived = start + 46 * MS;
        assert!(keep.arrived(arrived));
        keep.tick_began(arrived, true);
        assert!(!keep.timer(arrived + 10 * MS));
        assert!(keep.timer(arrived + 16 * MS));
    }

    #[test]
    fn a_post_arriving_after_the_loop_or_too_soon_runs_no_tick() {
        let mut keep = KeepAlive::default();
        let start = Instant::now();
        keep.entered(start);
        assert!(keep.timer(start), "no tick yet");
        keep.exited(start);
        assert!(!keep.arrived(start + MS), "the loop ended");
        // A tick began between the post and its arrival.
        keep.entered(start);
        assert!(keep.timer(start + 100 * MS));
        keep.tick_began(start + 101 * MS, true);
        assert!(!keep.arrived(start + 102 * MS));
        // Then posts resume.
        assert!(keep.timer(start + 120 * MS));
    }

    #[test]
    fn the_normal_timer_ticking_inside_the_loop_stands_the_keep_alive_down() {
        let mut keep = KeepAlive::default();
        let start = Instant::now();
        keep.entered(start);
        assert!(keep.timer(start));
        keep.tick_began(start + MS, false);
        assert!(!keep.arrived(start + 50 * MS));
        assert!(!keep.timer(start + 100 * MS));
        // The next loop starts afresh.
        keep.exited(start);
        keep.entered(start);
        assert!(keep.timer(start + 200 * MS));
    }

    #[test]
    fn a_post_that_could_not_be_sent_does_not_block_the_next() {
        let mut keep = KeepAlive::default();
        let start = Instant::now();
        keep.entered(start);
        assert!(keep.timer(start));
        keep.not_posted();
        assert!(keep.timer(start + MS));
    }

    #[test]
    fn a_loop_summary_counts_posts_ticks_and_the_longest_gap() {
        let mut keep = KeepAlive::default();
        let start = Instant::now();
        assert_eq!(keep.exited(start), None, "no loop on");
        keep.tick_began(start, false);
        keep.entered(start + 10 * MS);
        assert!(keep.timer(start + 20 * MS));
        assert!(keep.arrived(start + 22 * MS));
        keep.tick_began(start + 22 * MS, true);
        assert!(keep.timer(start + 40 * MS));
        assert!(keep.arrived(start + 41 * MS));
        keep.tick_began(start + 41 * MS, true);
        // The normal timer comes back once, after a long gap.
        keep.tick_began(start + 300 * MS, false);
        assert_eq!(
            keep.exited(start + 310 * MS),
            Some(LoopSummary {
                took: 300 * MS,
                posts: 2,
                kept: 2,
                normal: 1,
                longest_gap: 259 * MS,
            })
        );
        // The next loop starts from zero; its gap counts from the last tick.
        keep.entered(start + 400 * MS);
        let summary = keep.exited(start + 450 * MS).expect("a loop was on");
        assert_eq!((summary.posts, summary.kept, summary.normal), (0, 0, 0));
        assert_eq!(summary.longest_gap, 150 * MS);
    }
}
