//! Redraw pacing of the main window on systems where Slint's winit backend
//! does not tie redraws to the display (Windows today; `platform::VBLANK_CLOCK`).
//!
//! Slint 1.18 paces redraws there with a repeated timer of `1000/refresh` ms
//! (16 ms at 60 Hz) that is re-armed from whenever it ran, not from the
//! display, and each change is drawn twice (the timer's tick after the draw
//! asks for another one). Drawn frames therefore drift against the screen's
//! refresh: at 60 fps some refreshes get two draws and some none, so new video
//! frames are skipped and old ones repeated, and animations judder.
//! (macOS and iOS use a display link; upstream Slint after 1.18.1 draws each
//! change once but still on the timer.)
//!
//! Here the main window draws at most once per vertical blank of its display:
//!
//! - a winit `RedrawRequested` for the main window (Slint's timer, the system)
//!   is held while the clock runs, and owed to the next vertical blank;
//! - a new video picture (`picture_set`) is owed directly, without waiting for
//!   Slint's timer; a newer one that arrives before that draw waits in the
//!   frame slot and is set right after it (`accept_picture`), so jitter around
//!   a vertical blank delays a picture by one refresh instead of dropping one;
//! - at each vertical blank (a thread waits on `platform::VBlankWaiter` and
//!   wakes the UI thread) the window draws if something is owed or Slint
//!   animates, otherwise the clock stops;
//! - with the clock stopped, the first request draws at once (no added
//!   latency for a single change) and starts the clock.
//!
//! Without a clock (`None` from the platform, a failed wait,
//! `VESTRA_FRAME_PACING=slint`) nothing is held: Slint's own pacing.
#![forbid(unsafe_code)]

use slint::ComponentHandle;
use slint::winit_030::{CustomApplicationHandler, EventResult, WinitWindowAccessor, winit};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// An owed draw that no vertical blank served within this time is drawn
/// anyway (the clock stalled, for example while the display sleeps).
const OVERDUE: Duration = Duration::from_millis(50);

/// What to do with a winit `RedrawRequested`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision {
    /// Let Slint draw now; otherwise hold it for the next vertical blank.
    pub draw: bool,
    /// Start the clock.
    pub arm: bool,
    /// A video picture waits for this draw: deliver it after the draw.
    pub deliver: bool,
}
const HOLD: Decision = Decision {
    draw: false,
    arm: false,
    deliver: false,
};

/// The pacing decisions, without a window or a clock (tested below).
#[derive(Debug, Default)]
pub struct Gate {
    /// A draw is owed to the next vertical blank, since when.
    owed: Option<Instant>,
    /// The pacer asked winit for this draw: let the next request through.
    allowed: bool,
    /// The clock runs (vertical blanks arrive).
    armed: bool,
    /// A video picture is set but not drawn yet, since when.
    picture: Option<Instant>,
    /// A newer picture was refused meanwhile (it waits in the frame slot).
    refused: bool,
}
impl Gate {
    /// winit asks the main window to draw.
    pub fn redraw_requested(&mut self, now: Instant) -> Decision {
        let arm = if std::mem::take(&mut self.allowed) {
            false
        } else if !self.armed {
            // Idle: draw this one at once, pace what follows.
            self.armed = true;
            true
        } else {
            self.owed.get_or_insert(now);
            return HOLD;
        };
        self.owed = None;
        self.picture = None;
        Decision {
            draw: true,
            arm,
            deliver: std::mem::take(&mut self.refused),
        }
    }
    /// Whether the main window may take a new video picture now: not while
    /// the previous one waits for its draw (unless that is overdue), so two
    /// pictures arriving within one refresh are both shown, one refresh
    /// apart, instead of the second replacing the first.
    pub fn accept_picture(&mut self, now: Instant) -> bool {
        let waiting = self
            .picture
            .is_some_and(|since| now.saturating_duration_since(since) < OVERDUE);
        self.refused |= waiting;
        !waiting
    }
    /// A new video picture was set (Slint asks to draw it only later, on
    /// its timer). Returns whether the clock must start.
    pub fn picture_set(&mut self, now: Instant) -> bool {
        self.picture.get_or_insert(now);
        self.owed.get_or_insert(now);
        !std::mem::replace(&mut self.armed, true)
    }
    /// A vertical blank. `animating`: Slint runs an animation. `true`: ask
    /// winit to draw now; `false`: nothing to draw, the clock stops.
    pub fn vblank(&mut self, animating: bool) -> bool {
        if self.owed.take().is_some() || animating {
            self.allowed = true;
            true
        } else {
            self.armed = false;
            false
        }
    }
    /// The host tick: `true` when an owed draw waited longer than `OVERDUE`
    /// (draw it now).
    pub fn overdue(&mut self, now: Instant) -> bool {
        if self
            .owed
            .is_some_and(|since| now.saturating_duration_since(since) >= OVERDUE)
        {
            self.owed = None;
            self.allowed = true;
            return true;
        }
        false
    }
}

/// A wait shorter than this did not wait for a vertical blank (the fastest
/// displays refresh every 2 ms).
const SPIN_SHORT: Duration = Duration::from_millis(1);
/// This many such waits in a row: the clock does not block at all (an
/// indirect display such as IddCx or DisplayLink, a display asleep), and
/// pacing on it would spin a core and draw without limit.
const SPIN_RUN: u32 = 16;

/// Detects a display clock that returns without waiting (tested below).
#[derive(Debug, Default)]
struct Spin {
    short: u32,
}
impl Spin {
    /// One wait took `waited`; `true` once `SPIN_RUN` waits in a row were
    /// shorter than `SPIN_SHORT`.
    fn spinning(&mut self, waited: Duration) -> bool {
        if waited < SPIN_SHORT {
            self.short += 1;
        } else {
            self.short = 0;
        }
        self.short >= SPIN_RUN
    }
}

/// The vertical blank thread's shared state.
#[derive(Default)]
struct Clock {
    state: Mutex<ClockState>,
    wake: Condvar,
    /// A vertical blank is queued on the UI thread (do not queue another).
    posted: AtomicBool,
}
#[derive(Default)]
struct ClockState {
    armed: bool,
    stop: bool,
}
impl Clock {
    fn set(&self, change: impl FnOnce(&mut ClockState)) {
        change(&mut self.state.lock().unwrap_or_else(PoisonError::into_inner));
        self.wake.notify_one();
    }
    /// The thread: while armed, wake the UI thread at each vertical blank.
    fn run(self: Arc<Self>, mut waiter: crate::platform::VBlankWaiter) {
        let mut spin = Spin::default();
        loop {
            {
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                while !state.armed && !state.stop {
                    state = self
                        .wake
                        .wait(state)
                        .unwrap_or_else(PoisonError::into_inner);
                }
                if state.stop {
                    return;
                }
            }
            let began = Instant::now();
            let waited = waiter.wait();
            let spinning = waited && spin.spinning(began.elapsed());
            let ok = waited && !spinning;
            if !waited {
                diag!("frame pacing: the display clock failed; Slint paces redraws");
            } else if spinning {
                diag!(
                    "frame pacing: the display clock returns without waiting (no real vertical blanks); Slint paces redraws"
                );
            }
            if !self.posted.swap(true, Ordering::AcqRel) || !ok {
                let clock = self.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    clock.posted.store(false, Ordering::Release);
                    PACING.with(|pacing| {
                        if let Some(pacing) = pacing.borrow_mut().as_mut()
                            && Arc::ptr_eq(&pacing.clock, &clock)
                        {
                            if ok {
                                pacing.vblank();
                            } else {
                                pacing.fail();
                            }
                        }
                    });
                });
            }
            if !ok {
                return;
            }
        }
    }
}

/// The main window's pacing (UI thread).
struct Pacing {
    gate: Gate,
    window: winit::window::WindowId,
    shell: slint::Weak<slint_interpreter::ComponentInstance>,
    clock: Arc<Clock>,
    /// The clock failed: hold nothing.
    failed: bool,
}
impl Pacing {
    fn request_redraw(&self) {
        if let Some(shell) = self.shell.upgrade() {
            shell
                .window()
                .with_winit_window(|window| window.request_redraw());
        }
    }
    fn arm(&self) {
        self.clock.set(|state| state.armed = true);
    }
    fn vblank(&mut self) {
        let animating = self
            .shell
            .upgrade()
            .is_some_and(|shell| shell.window().has_active_animations());
        if self.gate.vblank(animating) {
            self.request_redraw();
        } else {
            self.clock.set(|state| state.armed = false);
        }
    }
    fn fail(&mut self) {
        self.failed = true;
        self.request_redraw();
    }
}
impl Drop for Pacing {
    fn drop(&mut self) {
        self.clock.set(|state| state.stop = true);
    }
}

thread_local! {
    static PACING: RefCell<Option<Pacing>> = const { RefCell::new(None) };
    /// A main window shown before winit created its native window (at
    /// startup, before the event loop runs): paced once it exists.
    static PENDING: RefCell<Option<slint::Weak<slint_interpreter::ComponentInstance>>> =
        const { RefCell::new(None) };
    /// Hands the main window the newest video picture (`on_deliver`).
    static DELIVER: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
}

/// How to hand the main window the newest video picture again after a
/// refused one (`accept_picture`); `None` at the end of the event loop.
pub fn on_deliver(deliver: Option<Box<dyn Fn()>>) {
    DELIVER.with(|slot| *slot.borrow_mut() = deliver);
}

/// Whether to pace the main window here: the system has a display clock and
/// `VESTRA_FRAME_PACING=slint` (A/B tests, a fallback in the field) is not set.
pub fn wanted() -> bool {
    crate::platform::VBLANK_CLOCK && std::env::var("VESTRA_FRAME_PACING").as_deref() != Ok("slint")
}

/// The winit hook (installed with the backend when `wanted`): holds the
/// main window's redraw requests per the gate.
pub struct Handler;
impl CustomApplicationHandler for Handler {
    fn window_event(
        &mut self,
        _: &winit::event_loop::ActiveEventLoop,
        window_id: winit::window::WindowId,
        _: Option<&winit::window::Window>,
        _: Option<&slint::Window>,
        event: &winit::event::WindowEvent,
    ) -> EventResult {
        if !matches!(event, winit::event::WindowEvent::RedrawRequested) {
            return EventResult::Propagate;
        }
        PACING.with(|pacing| {
            let mut pacing = pacing.borrow_mut();
            let Some(pacing) = pacing
                .as_mut()
                .filter(|p| p.window == window_id && !p.failed)
            else {
                return EventResult::Propagate;
            };
            let decision = pacing.gate.redraw_requested(Instant::now());
            if decision.arm {
                pacing.arm();
            }
            if decision.deliver {
                // Runs after this draw (a posted message): the refused
                // picture is set and drawn at the next vertical blank.
                let _ = slint::invoke_from_event_loop(|| {
                    DELIVER.with(|deliver| {
                        if let Some(deliver) = &*deliver.borrow() {
                            deliver();
                        }
                    });
                });
            }
            if decision.draw {
                EventResult::Propagate
            } else {
                EventResult::PreventDefault
            }
        })
    }
}

/// Pace `shell` (the main window, after `show()`; when `wanted`); a rebuilt
/// main window replaces the previous one. Nothing without a display clock.
pub fn follow(shell: &slint_interpreter::ComponentInstance) {
    if !wanted() {
        return;
    }
    // Dropping the old one stops its thread.
    PACING.with(|pacing| pacing.borrow_mut().take());
    PENDING.with(|pending| *pending.borrow_mut() = Some(shell.as_weak()));
    start_pending();
}

/// Start pacing the pending main window once winit has its native window.
fn start_pending() {
    let Some(shell) = PENDING.with(|pending| pending.borrow().as_ref().map(|s| s.upgrade())) else {
        return;
    };
    let Some(shell) = shell else {
        PENDING.with(|pending| pending.borrow_mut().take());
        return;
    };
    let Some(window) = shell.window().with_winit_window(|window| window.id()) else {
        return;
    };
    PENDING.with(|pending| pending.borrow_mut().take());
    let waiter = crate::platform::vblank_waiter(shell.window());
    PACING.with(|pacing| {
        let mut pacing = pacing.borrow_mut();
        let Some(waiter) = waiter else {
            return;
        };
        let clock = Arc::new(Clock::default());
        let thread = clock.clone();
        if std::thread::Builder::new()
            .name("vestra-vblank".into())
            .spawn(move || thread.run(waiter))
            .is_err()
        {
            return;
        }
        *pacing = Some(Pacing {
            gate: Gate::default(),
            window,
            shell: shell.as_weak(),
            clock,
            failed: false,
        });
    });
}

/// Whether the main window takes a new video picture now (`Gate::accept_picture`);
/// when not, it stays in the frame slot and comes again after the draw
/// (`on_deliver`). Always without pacing.
pub fn accept_picture() -> bool {
    PACING.with(|pacing| {
        pacing
            .borrow_mut()
            .as_mut()
            .filter(|p| !p.failed)
            .is_none_or(|p| p.gate.accept_picture(Instant::now()))
    })
}

/// A new video picture was set on the main window: draw it at the next
/// vertical blank.
pub fn picture_set() {
    PACING.with(|pacing| {
        if let Some(pacing) = pacing.borrow_mut().as_mut().filter(|p| !p.failed)
            && pacing.gate.picture_set(Instant::now())
        {
            pacing.arm();
        }
    });
}

/// The host tick: start pacing a main window whose native window now
/// exists; draw what waited too long for a vertical blank.
pub fn tick() {
    start_pending();
    PACING.with(|pacing| {
        if let Some(pacing) = pacing.borrow_mut().as_mut()
            && pacing.gate.overdue(Instant::now())
        {
            pacing.request_redraw();
        }
    });
}

/// The main window closed (end of the event loop): stop the clock.
pub fn stop() {
    PENDING.with(|pending| pending.borrow_mut().take());
    PACING.with(|pacing| pacing.borrow_mut().take());
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRAW: Decision = Decision {
        draw: true,
        arm: false,
        deliver: false,
    };
    const EAGER: Decision = Decision {
        draw: true,
        arm: true,
        deliver: false,
    };

    #[test]
    fn a_clock_that_never_blocks_is_given_up() {
        let mut spin = Spin::default();
        let (short, blank) = (Duration::from_micros(30), Duration::from_micros(16_700));
        // The first wait after arming can be short (the blank was near).
        assert!(!spin.spinning(short));
        assert!(!spin.spinning(blank));
        // Real blanks never trip it, even at 500 Hz.
        for _ in 0..100 {
            assert!(!spin.spinning(Duration::from_millis(2)));
        }
        // Short waits in a row: given up on the last of the run.
        for _ in 1..SPIN_RUN {
            assert!(!spin.spinning(short));
        }
        assert!(spin.spinning(short));
    }

    #[test]
    fn an_idle_window_draws_the_first_request_at_once_and_starts_the_clock() {
        let mut gate = Gate::default();
        let now = Instant::now();
        assert_eq!(gate.redraw_requested(now), EAGER);
        // Further requests wait for the vertical blank.
        assert_eq!(gate.redraw_requested(now), HOLD);
        assert_eq!(gate.redraw_requested(now), HOLD);
        assert!(gate.vblank(false), "owed: draw");
        // The draw the pacer asked for goes through.
        assert_eq!(gate.redraw_requested(now), DRAW);
        // Nothing owed at the next blank: the clock stops.
        assert!(!gate.vblank(false));
        // Idle again: the next request draws at once.
        assert_eq!(gate.redraw_requested(now), EAGER);
    }

    #[test]
    fn video_frames_draw_once_per_vertical_blank() {
        let mut gate = Gate::default();
        let now = Instant::now();
        assert!(gate.accept_picture(now));
        assert!(gate.picture_set(now), "first picture starts the clock");
        // Slint's timer ticks before the blank: held, still one draw.
        assert_eq!(gate.redraw_requested(now), HOLD);
        assert!(gate.vblank(false));
        assert_eq!(gate.redraw_requested(now), DRAW);
        // A blank with no new picture and no request: no draw, clock stops.
        assert!(!gate.vblank(false));
        // One picture per refresh: each blank draws the new one.
        for arms in [true, false, false] {
            assert!(gate.accept_picture(now));
            assert_eq!(gate.picture_set(now), arms);
            assert!(gate.vblank(false));
            assert_eq!(gate.redraw_requested(now), DRAW);
        }
    }

    #[test]
    fn a_second_picture_within_one_refresh_waits_for_the_next() {
        let mut gate = Gate::default();
        let now = Instant::now();
        assert!(gate.accept_picture(now));
        gate.picture_set(now);
        // The next picture arrives before the blank drew the first: it
        // stays in the slot ...
        assert!(!gate.accept_picture(now + Duration::from_millis(2)));
        assert!(gate.vblank(false));
        // ... and is delivered after this draw, for the next blank.
        assert_eq!(
            gate.redraw_requested(now),
            Decision {
                deliver: true,
                ..DRAW
            }
        );
        assert!(gate.accept_picture(now));
        assert!(!gate.picture_set(now), "the clock runs");
        assert!(gate.vblank(false));
        assert_eq!(gate.redraw_requested(now), DRAW, "delivered once");
    }

    #[test]
    fn animations_draw_at_every_vertical_blank() {
        let mut gate = Gate::default();
        let now = Instant::now();
        assert_eq!(gate.redraw_requested(now), EAGER);
        for _ in 0..3 {
            assert!(gate.vblank(true));
            assert_eq!(gate.redraw_requested(now), DRAW);
        }
        assert!(!gate.vblank(false), "animation over");
    }

    #[test]
    fn a_draw_the_pacer_asked_for_covers_what_was_owed() {
        let mut gate = Gate::default();
        let now = Instant::now();
        gate.picture_set(now);
        assert!(gate.vblank(false));
        // winit coalesces Slint's tick with the pacer's request: one draw,
        // nothing owed afterwards.
        assert_eq!(gate.redraw_requested(now), DRAW);
        assert!(!gate.vblank(false));
    }

    #[test]
    fn a_stalled_clock_still_draws_and_takes_pictures() {
        let mut gate = Gate::default();
        let start = Instant::now();
        gate.picture_set(start);
        assert!(!gate.accept_picture(start + Duration::from_millis(20)));
        assert!(!gate.overdue(start + Duration::from_millis(20)));
        // No blank came (the display sleeps, the window is minimized).
        assert!(gate.accept_picture(start + OVERDUE));
        assert!(gate.overdue(start + OVERDUE));
        assert_eq!(
            gate.redraw_requested(start + OVERDUE),
            Decision {
                deliver: true,
                ..DRAW
            },
            "the overdue draw goes through"
        );
        assert!(!gate.overdue(start + OVERDUE * 3), "nothing owed now");
    }
}
