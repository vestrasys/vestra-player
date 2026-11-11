//! Keeping the GPU near playback load through short pauses (Windows, GPU
//! rendering).
//!
//! While paused, GPU rendering draws nothing new: mpv asks for no frame and
//! the windows present nothing, so the GPU drops to its idle clocks and power
//! state. On resume the first frames missed their vertical blanks while it
//! clocked up again: 100–150 ms of stutter in GPU mode on NVIDIA-driven
//! displays (user report, 2026-11-11), the GPU's counterpart of the audio
//! device's restart (`audio_hold`; software rendering keeps the CPU busy
//! enough and needed only that one).
//!
//! So for the first `audio_hold::HOLD` of a pause the render thread keeps
//! rendering mpv's current (paused) frame at the video's frame rate and
//! publishing it, as during playback: each one takes the normal path (the
//! windows draw it at the next vertical blank and present it). It stops after
//! `HOLD`, on resume (playback renders anyway), at the end of the file
//! (keep-open's pause holds nothing), with no source and while a source
//! loads. Presenting frames never holds off system or display sleep (no
//! execution-state request is made); a minimized window draws nothing, as
//! during playback.
//!
//! Windows only (`for_player`); the render thread's side is
//! `vestra_gpu_render::Handle::keep_warm`.
#![forbid(unsafe_code)]

use crate::playback::audio_hold::{HOLD, Source};
use std::time::Instant;

/// A change to apply to the GPU renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Render and present the paused frame at the video's frame rate.
    Start,
    /// Render only what mpv asks for again.
    Stop,
}

/// The state machine (tested below); `enabled` only on Windows.
#[derive(Debug)]
pub struct GpuWarm {
    enabled: bool,
    /// The renderer keeps the GPU busy now.
    warm: bool,
    /// The current pause began then.
    paused_since: Option<Instant>,
}

impl GpuWarm {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            warm: false,
            paused_since: None,
        }
    }
    /// The player: Windows only.
    pub fn for_player() -> Self {
        Self::new(cfg!(windows))
    }
    /// The source is `state` at `now`: what to change, if anything.
    pub fn observe(&mut self, state: Source, now: Instant) -> Option<Change> {
        if !self.enabled {
            return None;
        }
        let want = match state {
            Source::Paused => {
                let since = *self.paused_since.get_or_insert(now);
                now.saturating_duration_since(since) < HOLD
            }
            Source::Playing | Source::Loading | Source::Stopped => {
                self.paused_since = None;
                false
            }
        };
        (std::mem::replace(&mut self.warm, want) != want).then_some(if want {
            Change::Start
        } else {
            Change::Stop
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const S: Duration = Duration::from_secs(1);

    #[test]
    fn the_hold_is_180_seconds_for_both() {
        assert_eq!(HOLD, Duration::from_secs(180));
    }

    #[test]
    fn a_pause_starts_at_once_and_stops_after_the_hold() {
        let t = Instant::now();
        let mut warm = GpuWarm::new(true);
        assert_eq!(warm.observe(Source::Playing, t), None);
        assert_eq!(warm.observe(Source::Paused, t + S), Some(Change::Start));
        assert!(warm.warm);
        assert_eq!(warm.observe(Source::Paused, t + HOLD), None);
        assert_eq!(warm.observe(Source::Paused, t + S + HOLD - S / 10), None);
        assert_eq!(
            warm.observe(Source::Paused, t + S + HOLD),
            Some(Change::Stop)
        );
        assert!(!warm.warm);
        assert_eq!(warm.observe(Source::Paused, t + HOLD * 2), None, "once");
        // Playing renders anyway: nothing to stop or start.
        assert_eq!(warm.observe(Source::Playing, t + HOLD * 3), None);
        // A new pause starts its own `HOLD`.
        assert_eq!(
            warm.observe(Source::Paused, t + HOLD * 4),
            Some(Change::Start)
        );
        assert_eq!(warm.observe(Source::Paused, t + HOLD * 5 - S), None);
        assert_eq!(
            warm.observe(Source::Paused, t + HOLD * 5),
            Some(Change::Stop)
        );
    }

    #[test]
    fn resume_stops_it_and_forgets_the_pause() {
        let t = Instant::now();
        let mut warm = GpuWarm::new(true);
        assert_eq!(warm.observe(Source::Paused, t), Some(Change::Start));
        assert_eq!(warm.observe(Source::Playing, t + S), Some(Change::Stop));
        // The earlier pause's start is forgotten.
        assert_eq!(warm.observe(Source::Paused, t + HOLD), Some(Change::Start));
        assert_eq!(warm.observe(Source::Paused, t + HOLD + S), None);
    }

    #[test]
    fn the_end_no_source_and_a_load_stop_it() {
        for state in [Source::Stopped, Source::Loading] {
            let t = Instant::now();
            let mut warm = GpuWarm::new(true);
            assert_eq!(warm.observe(Source::Paused, t), Some(Change::Start));
            assert_eq!(warm.observe(state, t + S), Some(Change::Stop), "{state:?}");
            assert_eq!(warm.observe(state, t + S * 2), None);
            // A pause after it starts afresh.
            assert_eq!(warm.observe(Source::Paused, t + S * 3), Some(Change::Start));
            assert_eq!(
                warm.observe(Source::Paused, t + S * 3 + HOLD),
                Some(Change::Stop)
            );
        }
    }

    #[test]
    fn the_end_or_no_source_never_starts_it() {
        let t = Instant::now();
        let mut warm = GpuWarm::new(true);
        for state in [Source::Stopped, Source::Loading, Source::Playing] {
            assert_eq!(warm.observe(state, t), None);
            assert!(!warm.warm);
        }
    }

    #[test]
    fn disabled_nothing_changes() {
        let t = Instant::now();
        let mut warm = GpuWarm::new(false);
        for state in [Source::Paused, Source::Playing, Source::Paused] {
            assert_eq!(warm.observe(state, t), None);
        }
        assert!(!warm.warm);
    }

    #[test]
    fn only_windows_players_keep_the_gpu_busy() {
        assert_eq!(GpuWarm::for_player().enabled, cfg!(windows));
    }
}
