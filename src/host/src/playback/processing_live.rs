//! When the player's video graph changes, beyond what each change itself
//! does (`processing.rs`): the paused preview, settling, the processing size
//! and its budget fallback. Pure decisions over times and sizes, so they are
//! tested without libmpv; `Processing::tick` does the mpv side.
//!
//! - **Paused preview.** mpv filters a frame only when one is decoded, so a
//!   live update (`vf-command`) while paused shows nothing, and a graph
//!   change makes mpv decode again from the previous keyframe (a "refresh
//!   seek": 139 ms at 1080p, 0.46 s at 4K, 1.5 s at 4K with a blur). While
//!   paused, live updates are shown by stepping forward (`frame-step 2
//!   mute`: the first frame is one mpv already filtered ahead, the second
//!   is filtered with the new values; 3 on the first step after pausing,
//!   when mpv holds two such frames), at most every
//!   [`STEP_EVERY`] and one at a time; a change that needs a rebuild is
//!   made at once when none was made within [`SETTLE`], otherwise once the
//!   changes settle.
//! - **Settling.** [`SETTLE`] after the last change: one exact rebuild if
//!   the graph text in mpv is out of date (a deferred rebuild, or live
//!   updates: mpv's lavfi wrapper rebuilds the graph from its text at every
//!   seek, which would undo them), and, when the preview stepped, an exact
//!   seek back to where the picture was paused (sent before the rebuild:
//!   mpv then makes no refresh seek of its own, so it decodes once).
//! - **Pass-through teardown.** A graph whose slots all pass frames through
//!   is removed after [`TEARDOWN`] without changes (back to zero-copy
//!   decoding under GPU rendering), not during a drag through 0.
//! - **Processing size.** The graph runs at min(source, shown size × zoom),
//!   in quarter steps of the source ([`factor`]), re-targeted once a resize
//!   has ended ([`Retarget`]).
//! - **Budget fallback.** While playing, a graph that keeps dropping frames
//!   steps the size down a quarter (twice at most, [`Budget`]).
#![forbid(unsafe_code)]

use std::time::{Duration, Instant};

use crate::playback::video_view::Fit;

/// The paused preview steps at most this often.
pub const STEP_EVERY: Duration = Duration::from_millis(150);
/// A preview step not over by then is taken as over (mpv may never pause
/// again: the end of the file, or the user pressed play).
pub const STEP_TIMEOUT: Duration = Duration::from_millis(1000);
/// No change for this long: the changes have settled.
pub const SETTLE: Duration = Duration::from_millis(300);
/// A graph that only passes frames through is removed after this long
/// without changes.
pub const TEARDOWN: Duration = Duration::from_secs(2);
/// A new processing size waits until the view held for this long after a
/// run of changes (a window border drag sends one every 50 ms) ...
pub const RETARGET_AFTER_DRAG: Duration = Duration::from_millis(500);
/// ... or this long after a single change (full screen, maximize).
pub const RETARGET_AFTER_JUMP: Duration = Duration::from_millis(150);
/// The budget fallback's measuring window.
pub const BUDGET_WINDOW: Duration = Duration::from_secs(2);
/// More than this share of the frames dropped in a window is over budget.
pub const BUDGET_DROPS: f64 = 0.15;
/// Over budget in this many windows in a row steps the size down.
pub const BUDGET_WINDOWS: u32 = 2;
/// At most this many budget steps below the shown size.
pub const BUDGET_MAX_STEPS: u32 = 2;
/// The processing size moves in steps of this share of the source.
pub const FACTOR_STEP: f64 = 0.25;

/// The video area and display layer the picture is shown in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewFacts {
    /// The video area in physical pixels.
    pub area: (u32, u32),
    /// The display layer's zoom (1: none).
    pub zoom: f64,
    pub fit: Fit,
    /// The display aspect override, `None`: the picture's own.
    pub aspect: Option<f64>,
}

/// The processing factor for a graph whose output has display size
/// `output` at full size, shown in `view`: the shown size over the full
/// size, times the zoom, never above 1 (no upscaling), rounded up to a
/// quarter step ([`quantise`]) and then `steps` quarter steps lower (the
/// budget fallback, at least one quarter). Keep fits the picture inside
/// the area (the tighter side counts); stretch and fill cover it (the
/// looser side).
pub fn factor(output: (f64, f64), view: &ViewFacts, steps: u32) -> f64 {
    let (ow, oh) = output;
    let (aw, ah) = (f64::from(view.area.0), f64::from(view.area.1));
    if !(ow > 0. && oh > 0. && aw > 0. && ah > 0.) {
        return 1.;
    }
    // An aspect override reshapes the picture: same height, new width.
    let ow = view
        .aspect
        .filter(|a| a.is_finite() && *a > 0.)
        .map_or(ow, |a| oh * a);
    let (x, y) = (aw / ow, ah / oh);
    let shown = match view.fit {
        Fit::Keep => x.min(y),
        Fit::Stretch | Fit::Fill => x.max(y),
    };
    let zoom = if view.zoom.is_finite() && view.zoom > 0. {
        view.zoom
    } else {
        1.
    };
    let full = quantise(shown * zoom);
    (full - FACTOR_STEP * f64::from(steps)).max(FACTOR_STEP)
}

/// Rounded up to a quarter step, at least one quarter, at most 1.
pub fn quantise(factor: f64) -> f64 {
    if !factor.is_finite() || factor >= 1. {
        return 1.;
    }
    // A hair below a step stays on it (float noise in area / source).
    ((factor / FACTOR_STEP - 1e-9).ceil() * FACTOR_STEP).clamp(FACTOR_STEP, 1.)
}

/// Debounces the processing size: the view (area, zoom, fit, aspect,
/// source) is offered every turn; a new factor is due once its inputs held
/// for [`RETARGET_AFTER_DRAG`] after a run of changes, or
/// [`RETARGET_AFTER_JUMP`] after a single one.
#[derive(Debug, Default)]
pub struct Retarget {
    inputs: Option<Inputs>,
    /// When the inputs last changed, and the change before that.
    changed: Option<Instant>,
    before: Option<Instant>,
}

/// What the factor depends on, compared for changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Inputs {
    pub view: ViewFacts,
    /// The source's display size.
    pub source: (u32, u32),
}

impl Retarget {
    /// Offer the inputs now; `true` when they held long enough for the
    /// factor computed from them to be applied.
    pub fn settled(&mut self, inputs: Inputs, now: Instant) -> bool {
        if self.inputs != Some(inputs) {
            self.before = self.changed;
            self.changed = self.inputs.map(|_| now);
            self.inputs = Some(inputs);
        }
        let Some(changed) = self.changed else {
            // The first inputs seen: nothing to wait for.
            return true;
        };
        let jump = self
            .before
            .is_none_or(|before| changed.duration_since(before) >= RETARGET_AFTER_DRAG);
        let wait = if jump {
            RETARGET_AFTER_JUMP
        } else {
            RETARGET_AFTER_DRAG
        };
        now.duration_since(changed) >= wait
    }
    /// When `settled` can next turn true.
    pub fn due_in(&self, now: Instant) -> Option<Duration> {
        let changed = self.changed?;
        let wait = if self
            .before
            .is_none_or(|before| changed.duration_since(before) >= RETARGET_AFTER_DRAG)
        {
            RETARGET_AFTER_JUMP
        } else {
            RETARGET_AFTER_DRAG
        };
        Some((changed + wait).saturating_duration_since(now))
    }
}

/// The budget fallback: frames dropped (`frame-drop-count` plus
/// `decoder-frame-drop-count`) against frames played (`time-pos` × the
/// container rate) per [`BUDGET_WINDOW`] while playing.
#[derive(Debug, Default)]
pub struct Budget {
    /// Steps taken below the shown size.
    pub steps: u32,
    window: Option<(Instant, f64, f64)>,
    over: u32,
}

impl Budget {
    /// Forget the windows (paused, a seek, a rebuild, no graph); the steps
    /// stay until `reset`.
    pub fn pause(&mut self) {
        self.window = None;
        self.over = 0;
    }
    /// Another source: full size again.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    /// A sample while playing: `dropped` so far, `position` in seconds,
    /// `fps` the container rate. `true` when the size should step down.
    pub fn sample(&mut self, now: Instant, dropped: f64, position: f64, fps: f64) -> bool {
        let Some((start, dropped0, position0)) = self.window else {
            self.window = Some((now, dropped, position));
            return false;
        };
        if now.duration_since(start) < BUDGET_WINDOW {
            return false;
        }
        self.window = Some((now, dropped, position));
        let played = (position - position0) * fps;
        let lost = dropped - dropped0;
        // A seek or a new count (backwards) says nothing.
        if !(played > 0. && lost >= 0. && fps > 0.) {
            self.over = 0;
            return false;
        }
        if lost / (played + lost) > BUDGET_DROPS {
            self.over += 1;
        } else {
            self.over = 0;
        }
        if self.over >= BUDGET_WINDOWS && self.steps < BUDGET_MAX_STEPS {
            self.over = 0;
            self.steps += 1;
            return true;
        }
        false
    }
}

/// The live-change clock (see the module documentation).
#[derive(Debug, Default)]
pub struct Live {
    /// The last change of the graph (a live update, a rebuild).
    pub changed: Option<Instant>,
    /// The last rebuild (`vf add`).
    pub rebuilt: Option<Instant>,
    /// The graph text in mpv is older than the graph: live updates were
    /// sent since the last rebuild.
    pub stale: bool,
    /// A rebuild is waiting for the paused preview to settle.
    pub pending: bool,
    /// Live updates while paused that no preview step has shown yet.
    pub unseen: bool,
    /// The preview step in flight, since when.
    pub stepping: Option<Instant>,
    /// The last preview step.
    pub stepped: Option<Instant>,
    /// Where the paused picture was before the first preview step.
    pub anchor: Option<f64>,
    /// Where the last preview step left it (`time-pos`): elsewhere at the
    /// settle, someone else moved it, and it is not taken back.
    pub stepped_to: Option<f64>,
    pub retarget: Retarget,
    pub budget: Budget,
    pub view: Option<ViewFacts>,
}

/// What `Live::due` says to do now.
#[derive(Debug, PartialEq)]
pub enum Due {
    Nothing,
    /// Show the live updates while paused: step two frames.
    Step,
    /// The changes settled: rebuild if `rebuild`, then seek back to the
    /// anchor if there is one.
    Settle {
        rebuild: bool,
    },
    /// The graph only passes frames through: remove it.
    Teardown,
}

impl Live {
    /// Live updates were sent.
    pub fn updated(&mut self, now: Instant, paused: bool) {
        self.changed = Some(now);
        self.stale = true;
        self.pending = false;
        if paused {
            self.unseen = true;
        }
    }
    /// The graph was rebuilt: its text is current.
    pub fn rebuilt(&mut self, now: Instant) {
        self.changed = Some(now);
        self.rebuilt = Some(now);
        self.stale = false;
        self.pending = false;
        self.budget.pause();
    }
    /// A rebuild while paused: `true` when it should wait for the changes to
    /// settle (one was made within [`SETTLE`]); then it is pending.
    pub fn defer(&mut self, now: Instant) -> bool {
        let recent = self
            .rebuilt
            .is_some_and(|at| now.duration_since(at) < SETTLE);
        if recent {
            self.pending = true;
            self.changed = Some(now);
        }
        recent
    }
    /// A step was sent.
    pub fn step(&mut self, now: Instant) {
        self.stepping = Some(now);
        self.stepped = Some(now);
        self.unseen = false;
    }
    /// The step in flight ended (mpv paused again) or timed out.
    pub fn step_over(&mut self, paused: bool, now: Instant) -> bool {
        let Some(since) = self.stepping else {
            return true;
        };
        if paused || now.duration_since(since) >= STEP_TIMEOUT {
            self.stepping = None;
            return true;
        }
        false
    }
    /// Another source, or a seek someone else made: the anchor and the
    /// preview no longer apply.
    pub fn forget_position(&mut self) {
        self.anchor = None;
        self.stepped_to = None;
        self.unseen = false;
    }
    /// What to do now. `paused`: mpv's pause; `installed`: there is a graph;
    /// `acts`: some slot changes frames.
    pub fn due(&self, now: Instant, paused: bool, installed: bool, acts: bool) -> Due {
        if self.stepping.is_some() {
            return Due::Nothing;
        }
        let quiet = self
            .changed
            .is_none_or(|at| now.duration_since(at) >= SETTLE);
        if paused
            && self.unseen
            && !self.pending
            && self
                .stepped
                .is_none_or(|at| now.duration_since(at) >= STEP_EVERY)
        {
            return Due::Step;
        }
        // Playing: a pending rebuild is made at once; live updates are
        // written back once they settle. Paused: both once the changes
        // settled and the preview showed them.
        if (self.pending || (installed && self.stale))
            && !(paused && self.unseen)
            && (quiet || (self.pending && !paused))
        {
            return Due::Settle { rebuild: true };
        }
        if paused && quiet && self.anchor.is_some() {
            return Due::Settle { rebuild: false };
        }
        if installed
            && !acts
            && self
                .changed
                .is_none_or(|at| now.duration_since(at) >= TEARDOWN)
        {
            return Due::Teardown;
        }
        Due::Nothing
    }
    /// When `due` may next say something other than `Nothing` (a cap on the
    /// worker's wait), `None` when only an event or a command can change it.
    pub fn due_in(
        &self,
        now: Instant,
        paused: bool,
        installed: bool,
        acts: bool,
    ) -> Option<Duration> {
        let after = |at: Option<Instant>, wait: Duration| {
            at.map_or(Duration::ZERO, |at| {
                (at + wait).saturating_duration_since(now)
            })
        };
        let mut due: Option<Duration> = None;
        let mut take = |d: Duration| due = Some(due.map_or(d, |old| old.min(d)));
        if let Some(since) = self.stepping {
            // Polled: mpv's pause event also wakes the worker.
            take(
                Duration::from_millis(10)
                    .min((since + STEP_TIMEOUT).saturating_duration_since(now)),
            );
            return due;
        }
        if paused && self.unseen {
            take(after(self.stepped, STEP_EVERY));
        }
        if self.pending || (installed && self.stale) || (paused && self.anchor.is_some()) {
            take(after(self.changed, SETTLE));
        }
        if installed && !acts {
            take(after(self.changed, TEARDOWN));
        }
        if installed && acts {
            if let Some(retarget) = self.retarget.due_in(now) {
                take(retarget);
            }
            if !paused {
                take(BUDGET_WINDOW);
            }
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(area: (u32, u32)) -> ViewFacts {
        ViewFacts {
            area,
            zoom: 1.,
            fit: Fit::Keep,
            aspect: None,
        }
    }
    fn ms(base: Instant, n: u64) -> Instant {
        base + Duration::from_millis(n)
    }

    #[test]
    fn the_factor_is_the_shown_size_in_quarter_steps_never_above_the_source() {
        let uhd = (3840., 2160.);
        // 4K in a 2560x1440 area: 2/3 → 3/4 (rounded up, never below).
        assert_eq!(factor(uhd, &view((2560, 1440)), 0), 0.75);
        // 1080p area: exactly a half.
        assert_eq!(factor(uhd, &view((1920, 1080)), 0), 0.5);
        // A 4K area, or a smaller source: full size, never upscaled.
        assert_eq!(factor(uhd, &view((3840, 2160)), 0), 1.);
        assert_eq!(factor((1920., 1080.), &view((2560, 1440)), 0), 1.);
        // A small window: at least a quarter.
        assert_eq!(factor(uhd, &view((320, 180)), 0), 0.25);
        // Zoomed in twice: twice the pixels are shown.
        let zoomed = ViewFacts {
            zoom: 2.,
            ..view((1920, 1080))
        };
        assert_eq!(factor(uhd, &zoomed, 0), 1.);
        let out = ViewFacts {
            zoom: 0.5,
            ..view((2560, 1440))
        };
        assert_eq!(factor(uhd, &out, 0), 0.5);
        // A scope film letterboxed in 16:9: the width binds under keep, the
        // height under fill.
        let scope = (3840., 1606.);
        assert_eq!(factor(scope, &view((2560, 1440)), 0), 0.75);
        let fill = ViewFacts {
            fit: Fit::Fill,
            ..view((2560, 1440))
        };
        assert_eq!(factor(scope, &fill, 0), 1.);
        // A portrait area: the width binds.
        assert_eq!(factor(uhd, &view((1000, 2000)), 0), 0.5);
        // The budget fallback: quarters below, at least one quarter.
        assert_eq!(factor(uhd, &view((2560, 1440)), 1), 0.5);
        assert_eq!(factor(uhd, &view((2560, 1440)), 2), 0.25);
        assert_eq!(factor(uhd, &view((960, 540)), 2), 0.25);
        // Nothing known: full size.
        assert_eq!(factor((0., 0.), &view((2560, 1440)), 0), 1.);
        assert_eq!(factor(uhd, &view((0, 0)), 0), 1.);
    }

    #[test]
    fn quantising_rounds_up_to_quarters() {
        for (raw, quarter) in [
            (0.01, 0.25),
            (0.25, 0.25),
            (0.2500000001, 0.25),
            (0.26, 0.5),
            (0.5, 0.5),
            (2. / 3., 0.75),
            (0.76, 1.),
            (1., 1.),
            (3., 1.),
            (f64::NAN, 1.),
        ] {
            assert_eq!(quantise(raw), quarter, "{raw}");
        }
    }

    #[test]
    fn a_resize_drag_retargets_once_it_ends_a_jump_soon() {
        let t = Instant::now();
        let inputs = |w: u32| Inputs {
            view: view((w, 1000)),
            source: (3840, 2160),
        };
        let mut retarget = Retarget::default();
        assert!(retarget.settled(inputs(1000), t), "the first view applies");
        // A drag: a new area every 50 ms.
        for (i, w) in (1010..1200).step_by(10).enumerate() {
            assert!(!retarget.settled(inputs(w), ms(t, 1000 + 50 * i as u64)));
        }
        let last = ms(t, 1000 + 50 * 18);
        assert!(!retarget.settled(inputs(1190), last + Duration::from_millis(499)));
        assert!(retarget.settled(inputs(1190), last + Duration::from_millis(500)));
        // A single change after a quiet time (full screen): 150 ms.
        let jump = last + Duration::from_secs(5);
        assert!(!retarget.settled(inputs(2560), jump));
        assert_eq!(retarget.due_in(jump), Some(RETARGET_AFTER_JUMP));
        assert!(retarget.settled(inputs(2560), jump + RETARGET_AFTER_JUMP));
    }

    #[test]
    fn the_budget_steps_down_after_two_windows_over_and_twice_at_most() {
        let t = Instant::now();
        let mut budget = Budget::default();
        // 60 fps, 30 % dropped per 2 s window.
        let mut dropped = 0.;
        let mut position = 0.;
        let mut steps = Vec::new();
        for window in 0..8u64 {
            steps.push(budget.sample(ms(t, window * 2000), dropped, position, 60.));
            dropped += 36.;
            position += 1.4; // 84 of 120 frames played
        }
        assert_eq!(
            steps,
            [false, false, true, false, true, false, false, false]
        );
        assert_eq!(budget.steps, 2);
        // Keeping up resets the run.
        let mut budget = Budget::default();
        assert!(!budget.sample(t, 0., 0., 60.));
        assert!(!budget.sample(ms(t, 2000), 40., 1.4, 60.));
        assert!(!budget.sample(ms(t, 4000), 41., 3.4, 60.));
        assert!(!budget.sample(ms(t, 6000), 80., 4.8, 60.));
        assert_eq!(budget.steps, 0);
        // A seek backwards says nothing.
        assert!(!budget.sample(ms(t, 8000), 200., 1.0, 60.));
        assert_eq!(budget.steps, 0);
    }

    #[test]
    fn a_paused_drag_steps_throttled_then_settles_once() {
        let t = Instant::now();
        let mut live = Live::default();
        live.rebuilt(t);
        // A live update while paused: a step at once.
        live.updated(ms(t, 1000), true);
        assert_eq!(live.due(ms(t, 1000), true, true, true), Due::Step);
        live.step(ms(t, 1000));
        live.anchor = Some(12.5);
        // Nothing while the step is in flight.
        live.updated(ms(t, 1030), true);
        assert_eq!(live.due(ms(t, 1040), true, true, true), Due::Nothing);
        assert!(!live.step_over(false, ms(t, 1040)));
        assert!(live.step_over(true, ms(t, 1060)));
        // The next step waits for STEP_EVERY after the last.
        assert_eq!(live.due(ms(t, 1100), true, true, true), Due::Nothing);
        assert_eq!(
            live.due_in(ms(t, 1100), true, true, true),
            Some(Duration::from_millis(50))
        );
        assert_eq!(live.due(ms(t, 1150), true, true, true), Due::Step);
        live.step(ms(t, 1150));
        assert!(live.step_over(true, ms(t, 1200)));
        // Quiet for SETTLE after the last update: one rebuild (the text
        // is stale) and the seek back.
        assert_eq!(live.due(ms(t, 1329), true, true, true), Due::Nothing);
        assert_eq!(
            live.due(ms(t, 1330), true, true, true),
            Due::Settle { rebuild: true }
        );
        live.rebuilt(ms(t, 1330));
        // The seek back clears the anchor (the caller); then nothing.
        assert_eq!(
            live.due(ms(t, 1640), true, true, true),
            Due::Settle { rebuild: false }
        );
        live.forget_position();
        assert_eq!(live.due(ms(t, 1640), true, true, true), Due::Nothing);
    }

    #[test]
    fn paused_rebuilds_are_throttled_and_playing_updates_written_back() {
        let t = Instant::now();
        let mut live = Live::default();
        // No rebuild recently: at once.
        assert!(!live.defer(t));
        live.rebuilt(t);
        // Another within SETTLE waits, then comes as the settle.
        assert!(live.defer(ms(t, 100)));
        assert!(live.pending);
        assert_eq!(live.due(ms(t, 399), true, true, true), Due::Nothing);
        assert_eq!(
            live.due(ms(t, 400), true, true, true),
            Due::Settle { rebuild: true }
        );
        // Playing, the pending rebuild is made at once.
        assert_eq!(
            live.due(ms(t, 101), false, true, true),
            Due::Settle { rebuild: true }
        );
        // Playing live updates: written back once they settle.
        let mut live = Live::default();
        live.updated(t, false);
        assert_eq!(live.due(ms(t, 200), false, true, true), Due::Nothing);
        assert_eq!(
            live.due(ms(t, 300), false, true, true),
            Due::Settle { rebuild: true }
        );
    }

    #[test]
    fn a_pass_through_graph_is_removed_after_the_teardown_time() {
        let t = Instant::now();
        let mut live = Live::default();
        live.rebuilt(t);
        live.updated(ms(t, 100), false);
        live.rebuilt(ms(t, 400));
        assert_eq!(live.due(ms(t, 2399), false, true, false), Due::Nothing);
        assert_eq!(live.due(ms(t, 2400), false, true, false), Due::Teardown);
        // A graph that acts stays.
        assert_eq!(live.due(ms(t, 9000), false, true, true), Due::Nothing);
        assert_eq!(
            live.due_in(ms(t, 2000), false, true, false),
            Some(Duration::from_millis(400))
        );
    }
}
