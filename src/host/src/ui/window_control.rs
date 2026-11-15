//! `plugin.window` (`window.control`): which plugin's choice is in effect, and
//! the window size that gives a requested video area. No OS calls here.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;

/// One value per plugin; the newest write wins, and a stopped plugin's write
/// gives way to the newest remaining one (else the default).
pub struct LastWrite<T> {
    writes: BTreeMap<u64, (u64, T)>,
    sequence: u64,
    default: T,
}
impl<T: Copy + PartialEq> LastWrite<T> {
    pub fn new(default: T) -> Self {
        Self {
            writes: BTreeMap::new(),
            sequence: 0,
            default,
        }
    }
    pub fn set(&mut self, owner: u64, value: T) {
        self.sequence += 1;
        self.writes.insert(owner, (self.sequence, value));
    }
    pub fn remove(&mut self, owner: u64) {
        self.writes.remove(&owner);
    }
    /// A parked owner woke as `to`: its write, kept through standby
    /// (events-and-triggers.md §4.8), is the new run's, in its place in the
    /// order of writes (a write by another plugin after it still wins).
    pub fn rekey(&mut self, from: u64, to: u64) {
        if from == to {
            return;
        }
        if let Some(write) = self.writes.remove(&from) {
            self.writes.insert(to, write);
        }
    }
    /// `owner` wrote a value that is in effect until it is removed: its run
    /// stops other than by going to standby, or its standby ends for good.
    pub fn holds(&self, owner: u64) -> bool {
        self.writes.contains_key(&owner)
    }
    pub fn get(&self) -> T {
        self.writes
            .values()
            .max_by_key(|(sequence, _)| *sequence)
            .map_or(self.default, |(_, value)| *value)
    }
}

/// UI-SHELL-06: the video column is at least 318 wide; the window at least
/// 320×182, which leaves the video 120 high under the 62px bottom bar.
pub const MIN_VIDEO_WIDTH: f64 = 318.;
pub const MIN_VIDEO_HEIGHT: f64 = 120.;
pub const MIN_WINDOW_WIDTH: f64 = 320.;
pub const MIN_WINDOW_HEIGHT: f64 = 182.;

/// What the shell adds around the video, and the room on screen (logical points).
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    /// Bottom bar height (window content height minus video height).
    pub extra_height: f64,
    /// Docked sidebar width plus the 3px splitter, 0 when not docked.
    pub extra_width: f64,
    /// The largest window content that fits the screen's visible area.
    pub max_content: (f64, f64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    pub content: (f64, f64),
    pub video: (f64, f64),
    pub clamped: bool,
}

/// The window content size for a video area of `width`×`height`. Too large
/// for the screen: scaled down keeping the aspect; below the minimum: raised
/// to it (letterboxed). Either way `clamped`.
pub fn fit(width: f64, height: f64, frame: Frame) -> Fit {
    let (mut w, mut h) = (width, height);
    let mut clamped = false;
    let max_w = frame.max_content.0 - frame.extra_width;
    let max_h = frame.max_content.1 - frame.extra_height;
    if (w > max_w || h > max_h) && max_w > 0. && max_h > 0. {
        let scale = (max_w / w).min(max_h / h);
        w = (w * scale).floor();
        h = (h * scale).floor();
        clamped = true;
    }
    let min_w = MIN_VIDEO_WIDTH.max(MIN_WINDOW_WIDTH - frame.extra_width);
    let min_h = MIN_VIDEO_HEIGHT.max(MIN_WINDOW_HEIGHT - frame.extra_height);
    if w < min_w || h < min_h {
        w = w.max(min_w);
        h = h.max(min_h);
        clamped = true;
    }
    Fit {
        content: (w + frame.extra_width, h + frame.extra_height),
        video: (w, h),
        clamped,
    }
}

/// The video area values a plugin may ask for.
pub fn valid_size(width: f64, height: f64) -> bool {
    [width, height]
        .iter()
        .all(|v| v.is_finite() && (1. ..=16384.).contains(v))
}

/// Whether the player's windows float (`window.setAlwaysOnTop`): when the
/// plugins' choice (`want`, [`LastWrite`]) says so and no system dialog
/// (file dialog or confirmation, `dialog_open`) is open. While one is open
/// the user may switch to other applications (Alt+Tab, the taskbar) to
/// look something up; the choice comes back when it closes.
pub fn floating(want: bool, dialog_open: bool) -> bool {
    want && !dialog_open
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(extra_width: f64) -> Frame {
        Frame {
            extra_height: 62.,
            extra_width,
            max_content: (2560., 1380.),
        }
    }

    #[test]
    fn newest_write_wins_and_a_stopped_plugin_gives_way() {
        let mut on_top = LastWrite::new(false);
        assert!(!on_top.get());
        on_top.set(1, true);
        on_top.set(2, false);
        assert!(!on_top.get(), "plugin 2 wrote last");
        on_top.set(1, true);
        assert!(on_top.get());
        on_top.remove(1);
        assert!(!on_top.get(), "back to plugin 2's value");
        on_top.remove(2);
        assert!(!on_top.get(), "back to the default");
    }

    /// Events-and-triggers.md §4.8: a parked plugin's write stays in effect
    /// under its holder and moves to its woken run in its place in the
    /// order: a later write by another plugin still wins, and the woken
    /// run's own next write replaces it. Removing the holder (its standby
    /// ended for good) gives way to the newest remaining one.
    #[test]
    fn a_kept_write_moves_to_the_woken_run_and_the_last_write_wins() {
        let mut on_top = LastWrite::new(false);
        on_top.set(1, true);
        // 1 parked, then woken as 5: still in effect.
        on_top.rekey(1, 5);
        assert!(!on_top.holds(1) && on_top.holds(5));
        assert!(on_top.get());
        // Another plugin writes after it: the last write wins.
        on_top.set(2, false);
        on_top.rekey(5, 6);
        assert!(
            !on_top.get(),
            "plugin 2 wrote last, the move changes nothing"
        );
        // The woken run writes again: its write is the newest.
        on_top.set(6, true);
        assert!(on_top.get());
        // A holder that wrote nothing, or the same owner: nothing happens.
        on_top.rekey(9, 10);
        on_top.rekey(6, 6);
        assert!(!on_top.holds(10) && on_top.holds(6));
        // Its standby ends for good: plugin 2's value again.
        on_top.remove(6);
        assert!(!on_top.get());
        let mut cursor = LastWrite::new(false);
        cursor.set(3, true);
        cursor.rekey(3, 4);
        assert!(cursor.get());
    }

    #[test]
    fn the_video_area_gets_the_bars_added_and_fits_the_screen() {
        let plain = fit(1920., 1080., frame(0.));
        assert_eq!(plain.content, (1920., 1142.));
        assert_eq!(plain.video, (1920., 1080.));
        assert!(!plain.clamped);
        // Docked sidebar of 317 + 3: window width = video + 320.
        let sidebar = fit(1024., 576., frame(320.));
        assert_eq!(sidebar.content, (1344., 638.));
        // 2x of 1080p does not fit: scaled down with the same aspect.
        let large = fit(3840., 2160., frame(0.));
        assert!(large.clamped);
        assert!(large.content.1 <= 1380. && large.content.0 <= 2560.);
        assert!(large.video.1 >= 1317. && (large.video.0 / large.video.1 - 16. / 9.).abs() < 0.01);
        // Below the minimum: raised, letterboxed.
        let small = fit(160., 90., frame(0.));
        assert_eq!(small.video, (320., 120.));
        assert!(small.clamped);
        let small = fit(160., 90., frame(320.));
        assert_eq!(small.video.0, 318.);
        assert!(valid_size(1., 16384.) && !valid_size(0., 10.) && !valid_size(f64::NAN, 10.));
    }

    /// The bug of 2026-11-15: a fullscreen player held on top kept its Save
    /// dialog and every other application behind it. Every combination: on
    /// top only when wanted and no dialog is open.
    #[test]
    fn no_floating_while_a_dialog_is_open() {
        assert!(!floating(false, false));
        assert!(floating(true, false));
        assert!(!floating(false, true));
        assert!(!floating(true, true), "a dialog is open: not on top");
    }
}
