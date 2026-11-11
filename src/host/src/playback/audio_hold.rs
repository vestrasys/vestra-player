//! Keeping the audio device running through short pauses (Windows).
//!
//! A Realtek or Bluetooth output that goes idle during a pause of more than
//! a few seconds took 270–290 ms to start again on resume, and mpv held the
//! video frame for that long (Windows, 2026-11-11: libmpv and the mpv player
//! alike; `ao=null` smooth). PotPlayer keeps its audio session active while
//! paused and resumes smoothly; mpv's `audio-stream-silence` does the same
//! (plays silence while paused instead of stopping the device). Measured on
//! the laptop's speakers, 10 s pause: 267–333 ms → 21–34 ms longest frame
//! interval, 15–19 → 0–1 missing frames.
//!
//! mpv's manual warns: "This modifies certain subtle player behavior, like
//! A/V-sync and underrun handling. Enabling this option is strongly
//! discouraged." It is used anyway for the measured resume, and only for
//! `HOLD`: a running audio stream keeps Windows from sleeping, and a paused
//! or finished player must never do that (user ruling 2026-11-11). So:
//!
//! - the player starts with silence on (`audio-stream-silence`);
//! - a pause longer than `HOLD`, the end of the file (keep-open's pause) and
//!   no source release it: silence off, then `ao-reload` (mpv reads the
//!   option only when it opens the audio output), and the device stops;
//! - playback after a release arms it again, with the same reload: the
//!   device has to start again then anyway.
//!
//! Windows only; elsewhere mpv's default (no silence) stays untouched.
#![forbid(unsafe_code)]

use std::time::{Duration, Instant};

/// How long a pause keeps the audio device running.
pub const HOLD: Duration = Duration::from_secs(60);

/// What the source is doing, as the state push sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Ready and playing.
    Playing,
    /// Ready and paused (not at the end).
    Paused,
    /// At the end (keep-open's pause), closed, failed or never opened.
    Stopped,
    /// Loading: no change.
    Loading,
}

/// A change to apply to mpv (`apply`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Silence off and reload: the device stops.
    Release,
    /// Silence on and reload: the device keeps running through pauses again.
    Rearm,
}

/// The state machine (tested below); `enabled` only on Windows.
#[derive(Debug)]
pub struct AudioHold {
    enabled: bool,
    /// Silence is on (the device runs through a pause).
    held: bool,
    /// The current pause began then.
    paused_since: Option<Instant>,
}

impl AudioHold {
    /// `enabled`: the player's mpv was started with silence on.
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            held: enabled,
            paused_since: None,
        }
    }
    /// The player's mpv: Windows only.
    pub fn for_player() -> Self {
        Self::new(cfg!(windows))
    }
    /// The source is `state` at `now`: what to change, if anything.
    pub fn observe(&mut self, state: Source, now: Instant) -> Option<Change> {
        if !self.enabled {
            return None;
        }
        match state {
            Source::Loading => None,
            Source::Playing => {
                self.paused_since = None;
                (!std::mem::replace(&mut self.held, true)).then_some(Change::Rearm)
            }
            Source::Paused => {
                let since = *self.paused_since.get_or_insert(now);
                self.release_if(now.saturating_duration_since(since) >= HOLD)
            }
            Source::Stopped => {
                self.paused_since = None;
                self.release_if(true)
            }
        }
    }
    fn release_if(&mut self, due: bool) -> Option<Change> {
        (due && std::mem::replace(&mut self.held, false)).then_some(Change::Release)
    }
}

/// Apply `change` to the player's mpv.
pub fn apply(mpv: &rsmpv::Mpv, change: Change) -> Result<(), rsmpv::Error> {
    mpv.set_property("audio-stream-silence", change == Change::Rearm)?;
    mpv.command(&["ao-reload"])
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Duration = Duration::from_secs(1);

    #[test]
    fn a_short_pause_keeps_the_device_running() {
        let t = Instant::now();
        let mut hold = AudioHold::new(true);
        assert_eq!(hold.observe(Source::Playing, t), None);
        assert_eq!(hold.observe(Source::Paused, t + S), None);
        assert_eq!(hold.observe(Source::Paused, t + S + HOLD - S), None);
        assert_eq!(
            hold.observe(Source::Playing, t + HOLD),
            None,
            "nothing to rearm"
        );
    }

    #[test]
    fn a_long_pause_releases_once_and_playback_rearms() {
        let t = Instant::now();
        let mut hold = AudioHold::new(true);
        assert_eq!(hold.observe(Source::Paused, t), None);
        assert_eq!(
            hold.observe(Source::Paused, t + HOLD),
            Some(Change::Release)
        );
        assert_eq!(hold.observe(Source::Paused, t + HOLD * 2), None, "once");
        assert_eq!(
            hold.observe(Source::Playing, t + HOLD * 3),
            Some(Change::Rearm)
        );
        // A new pause starts its own `HOLD`.
        assert_eq!(hold.observe(Source::Paused, t + HOLD * 4), None);
        assert_eq!(hold.observe(Source::Paused, t + HOLD * 5 - S), None);
        assert_eq!(
            hold.observe(Source::Paused, t + HOLD * 5),
            Some(Change::Release)
        );
    }

    #[test]
    fn the_end_of_the_file_releases_at_once_and_a_new_file_rearms() {
        let t = Instant::now();
        let mut hold = AudioHold::new(true);
        assert_eq!(hold.observe(Source::Playing, t), None);
        assert_eq!(hold.observe(Source::Stopped, t + S), Some(Change::Release));
        assert_eq!(hold.observe(Source::Stopped, t + S * 2), None);
        // The next file: loading changes nothing, playing rearms.
        assert_eq!(hold.observe(Source::Loading, t + S * 3), None);
        assert_eq!(
            hold.observe(Source::Playing, t + S * 4),
            Some(Change::Rearm)
        );
    }

    #[test]
    fn a_pause_cut_short_by_the_end_does_not_carry_its_time_over() {
        let t = Instant::now();
        let mut hold = AudioHold::new(true);
        assert_eq!(hold.observe(Source::Paused, t), None);
        assert_eq!(hold.observe(Source::Playing, t + S), None);
        // The earlier pause's start is forgotten.
        assert_eq!(hold.observe(Source::Paused, t + HOLD), None);
        assert_eq!(hold.observe(Source::Paused, t + HOLD + S), None);
    }

    #[test]
    fn without_silence_nothing_changes() {
        let t = Instant::now();
        let mut hold = AudioHold::new(false);
        for state in [
            Source::Playing,
            Source::Paused,
            Source::Stopped,
            Source::Playing,
        ] {
            assert_eq!(hold.observe(state, t + HOLD * 2), None);
        }
        assert_eq!(hold.observe(Source::Paused, t + HOLD * 4), None);
    }

    #[test]
    fn only_windows_players_hold_the_device() {
        assert_eq!(AudioHold::for_player().enabled, cfg!(windows));
    }
}
