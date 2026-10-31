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
//!   animates; the clock stops only after `LINGER` without a draw, so while
//!   a video plays (a picture at least every few refreshes) it keeps running
//!   across a refresh that brings no new picture;
//! - with the clock stopped, the first request draws at once (no added
//!   latency for a single change) and starts the clock.
//!
//! The linger is for UI changes during playback (0.5.2). When the clock
//! stopped at the first refresh without a new picture, a hover change in that
//! refresh (a menu row lighting up, the seek bar's preview following the
//! pointer) drew at once, off the display's phase, and the next blank drew
//! again: two draws in one refresh, the second blocking in `SwapBuffers`
//! until the following blank, so the next picture was taken late.
//!
//! `VESTRA_FRAME_LOG` appends what the pacing did each second (`log_text`).
//!
//! Without a clock (`None` from the platform, a failed wait,
//! `VESTRA_FRAME_PACING=slint`) nothing is held: Slint's own pacing.
#![forbid(unsafe_code)]

use slint::ComponentHandle;
use slint::winit_030::{CustomApplicationHandler, EventResult, WinitWindowAccessor, winit};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// An owed draw that no vertical blank served within this time is drawn
/// anyway (the clock stalled, for example while the display sleeps).
const OVERDUE: Duration = Duration::from_millis(50);
/// The clock keeps running this long after the last draw before it stops:
/// longer than the gap between two pictures of a video down to 15 fps, so
/// during playback every draw, also one for a UI change, waits for a blank.
const LINGER: Duration = Duration::from_millis(100);
/// A draw dispatch this long or longer counts as `slow` in the frame log:
/// half a refresh at 60 Hz, more than the draw itself takes on a GPU that
/// keeps up (about 4 ms on the RTX 4070), so the rest waited in the
/// swapchain acquire or the present.
const SLOW: Duration = Duration::from_millis(8);

/// What a vertical blank does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blank {
    /// Ask winit to draw now.
    Draw,
    /// Nothing to draw; the clock keeps running (`LINGER`).
    Wait,
    /// Nothing drawn for `LINGER`: the clock stops.
    Stop,
}

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
    /// The last draw, for `LINGER`.
    drawn: Option<Instant>,
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
        self.drawn = Some(now);
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
    /// A vertical blank at `now`. `animating`: Slint runs an animation.
    pub fn vblank(&mut self, now: Instant, animating: bool) -> Blank {
        if self.owed.take().is_some() || animating {
            self.allowed = true;
            Blank::Draw
        } else if self
            .drawn
            .is_some_and(|at| now.saturating_duration_since(at) < LINGER)
        {
            Blank::Wait
        } else {
            self.armed = false;
            Blank::Stop
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
    /// Blanks not queued because the previous one was still waiting: the
    /// UI thread was busy for a whole refresh (`log_text`).
    skipped: AtomicU64,
    /// The window moved or changed its scale factor: the next wait checks
    /// which display it is on (`VBlankWaiter::wait`).
    moved: AtomicBool,
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
            let waited = waiter.wait(self.moved.swap(false, Ordering::AcqRel));
            let spinning = waited && spin.spinning(began.elapsed());
            let ok = waited && !spinning;
            if !waited {
                diag!("frame pacing: the display clock failed; Slint paces redraws");
            } else if spinning {
                diag!(
                    "frame pacing: the display clock returns without waiting (no real vertical blanks); Slint paces redraws"
                );
            }
            let blank = Instant::now();
            if !self.posted.swap(true, Ordering::AcqRel) || !ok {
                let clock = self.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    clock.posted.store(false, Ordering::Release);
                    PACING.with(|pacing| {
                        if let Some(pacing) = pacing.borrow_mut().as_mut()
                            && Arc::ptr_eq(&pacing.clock, &clock)
                        {
                            if ok {
                                pacing.vblank(blank);
                            } else {
                                pacing.fail();
                            }
                        }
                    });
                });
            } else {
                self.skipped.fetch_add(1, Ordering::Relaxed);
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
    /// `VESTRA_FRAME_PACING=slint`: no clock, Slint paces; the hook only
    /// measures the main window's draws for the frame log.
    passive: bool,
    stats: Stats,
    cycles: Cycles,
    counts: Counts,
}

/// The main window's draws per frame-log line: requests (winit
/// `RedrawRequested`), draws let through, new pictures owed a draw
/// (`picture_set`), and how long after the last vertical blank each draw
/// started (` counts:` segment).
#[derive(Debug, Default)]
struct Counts {
    requested: u64,
    drawn: u64,
    pictures: u64,
    after_blank: Bucket,
}
impl Counts {
    fn drawn(&mut self, now: Instant, last_blank: Option<Instant>, period: Option<Duration>) {
        self.drawn += 1;
        let period = period.unwrap_or(Duration::from_micros(16_667));
        if let Some(blank) = last_blank {
            let after = now.saturating_duration_since(blank);
            if after < period * 2 {
                self.after_blank.add(after);
            }
        }
    }
    fn text(&mut self) -> String {
        let text = format!(
            " counts: requested {} drawn {} pictures {} blank-to-draw {}",
            self.requested,
            self.drawn,
            self.pictures,
            self.after_blank.text()
        );
        *self = Self::default();
        text
    }
}

/// `VESTRA_FRAME_PACING=every`: draw at every vertical blank while the
/// clock runs, new picture or not (diagnostics).
fn every_blank() -> bool {
    static EVERY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *EVERY.get_or_init(|| std::env::var("VESTRA_FRAME_PACING").as_deref() == Ok("every"))
}

/// The UI thread's CPU cycles in the main window's draws
/// (`platform::thread_cycles`; 0 where there is no counter): a draw that
/// takes long but few cycles waited, one with many cycles worked.
#[derive(Debug, Default)]
struct Cycles {
    /// The open draw's dispatch and Slint draw started at these counts.
    paint_at: Option<u64>,
    draw_at: Option<u64>,
    /// (count, total) since the frame log's last line.
    paint: (u64, u64),
    draw: (u64, u64),
}
impl Cycles {
    /// The draw's dispatch ended (winit's next callback).
    fn close_paint(&mut self) {
        if let Some(at) = self.paint_at.take() {
            // The draw (and its present) went to the GPU: time its completion.
            crate::platform::ui_draw_submitted();
            let used = crate::platform::thread_cycles().saturating_sub(at);
            self.paint = (self.paint.0 + 1, self.paint.1 + used);
        }
    }
    fn text(&mut self) -> String {
        let mean = |(count, total): (u64, u64)| {
            if count == 0 {
                "-".to_owned()
            } else {
                format!("{:.1}", total as f64 / count as f64 / 1e6)
            }
        };
        let text = format!(
            " cpu paint {} draw {} Mcycles",
            mean(self.paint),
            mean(self.draw)
        );
        (self.paint, self.draw) = ((0, 0), (0, 0));
        text
    }
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
    /// The vertical blank at `blank` reached the UI thread.
    fn vblank(&mut self, blank: Instant) {
        let now = Instant::now();
        self.stats.blank(blank, now);
        let animating = self
            .shell
            .upgrade()
            .is_some_and(|shell| shell.window().has_active_animations())
            || every_blank();
        match self.gate.vblank(now, animating) {
            Blank::Draw => self.request_redraw(),
            Blank::Wait => (),
            Blank::Stop => self.clock.set(|state| state.armed = false),
        }
    }
    fn fail(&mut self) {
        self.failed = true;
        self.request_redraw();
    }
}
/// Longest, total and count of a kind of UI-thread time (`log_text`).
#[derive(Debug, Default)]
struct Bucket {
    max: Duration,
    total: Duration,
    count: u64,
}
impl Bucket {
    fn add(&mut self, took: Duration) {
        self.max = self.max.max(took);
        self.total += took;
        self.count += 1;
    }
    fn max_text(&self) -> String {
        format!("{:.1}", self.max.as_secs_f64() * 1000.)
    }
    /// `max A mean B ms`, or `-` with nothing measured.
    fn text(&self) -> String {
        if self.count == 0 {
            return "-".to_owned();
        }
        let ms = |d: Duration| d.as_secs_f64() * 1000.;
        format!(
            "max {:.1} mean {:.1} ms",
            ms(self.max),
            ms(self.total) / self.count as f64
        )
    }
}

/// A stretch of UI-thread time between two of the hook's callbacks, by
/// what started it: Slint's timers (the host tick among them), a draw
/// (Slint's draw and `SwapBuffers`), a pointer move, any other window
/// event. User events (`invoke_from_event_loop`) are not hooked and count
/// with the segment they interrupt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Segment {
    Timers = 0,
    Paint = 1,
    Cursor = 2,
    Other = 3,
    /// A draw request the gate held: Slint did not run for it (no draw, no
    /// pointer flush), so the stretch is the hook's own bookkeeping. Not
    /// reported; without it every held request would dilute `paint`.
    Held = 4,
}

/// What the pacing did since the frame log's last line (UI thread).
#[derive(Debug, Default)]
struct Stats {
    /// Pointer moves over the main window.
    moves: u64,
    /// Redraw requests held for a blank.
    held: u64,
    /// Draws at once, the clock stopped (off the display's phase).
    eager: u64,
    /// Draws less than half a refresh after the previous draw.
    close: u64,
    /// Pictures that waited in the frame slot for the next refresh.
    refused: u64,
    /// The longest a blank waited for the UI thread.
    late: Duration,
    /// UI-thread time by segment kind (`Segment` as the index).
    segments: [Bucket; 5],
    /// Segments in which Slint processed the pending pointer moves (it
    /// coalesces them and processes them at the next other event or when
    /// the loop is about to wait): hit testing, hover changes, callbacks.
    pointer: Bucket,
    /// One loop turn: from winit's first callback to the wait.
    turn: Bucket,
    /// From a draw request's callback to Slint's `BeforeRendering`: the
    /// pointer flush when one is pending, then Slint's own preparation; with
    /// wgpu windows also the swapchain acquire (`get_current_texture`, which
    /// waits on DXGI's frame-latency object) and the clear's submit.
    pre: Bucket,
    /// `BeforeRendering` to `AfterRendering`: Slint's draw and its submits.
    draw: Bucket,
    /// `AfterRendering` to the end of the draw's dispatch: the present
    /// (`SwapBuffers`, or wgpu's `Queue::present`) and Slint's bookkeeping.
    present: Bucket,
    /// When the open draw's `AfterRendering` came.
    after: Option<Instant>,
    /// The open segment: kind, start, whether the pointer flush is in it.
    open: Option<(Segment, Instant, bool)>,
    turn_began: Option<Instant>,
    /// A pointer move was stored and not processed yet.
    pending_move: bool,
    last_draw: Option<Instant>,
    last_blank: Option<Instant>,
    /// The refresh period, from consecutive blanks.
    period: Option<Duration>,
    /// The open draw dispatch: when it started, and the vertical blank it
    /// followed.
    dispatch: Option<(Instant, Option<Instant>)>,
    /// Draw dispatches that took `SLOW` or longer (the swapchain acquire
    /// blocked, or the draw itself was long).
    slow: u64,
    /// Draw dispatches that ended more than a refresh after the blank they
    /// followed: the present went out after the next blank, so that blank
    /// most likely showed the previous picture again (a missed refresh).
    past: u64,
}
impl Stats {
    fn blank(&mut self, blank: Instant, now: Instant) {
        self.late = self.late.max(now.saturating_duration_since(blank));
        if let Some(last) = self.last_blank.replace(blank) {
            let period = blank.saturating_duration_since(last);
            // Consecutive blanks only (the clock may have stopped between).
            if period > Duration::from_millis(2) && period < Duration::from_millis(50) {
                self.period = Some(period);
            }
        }
    }
    /// A draw goes ahead now.
    fn draw(&mut self, now: Instant, eager: bool) {
        // An eager draw follows no blank (the clock was stopped).
        self.dispatch = Some((now, self.last_blank.filter(|_| !eager)));
        self.eager += u64::from(eager);
        let half = self.period.unwrap_or(Duration::from_micros(16_667)) / 2;
        if let Some(last) = self.last_draw.replace(now)
            && now.saturating_duration_since(last) < half
        {
            self.close += 1;
        }
    }
    fn end_segment(&mut self, now: Instant) {
        if let Some((kind, began, flush)) = self.open.take() {
            if let Some(after) = self.after.take().filter(|_| kind == Segment::Paint) {
                self.present.add(now.saturating_duration_since(after));
            }
            if kind == Segment::Paint
                && let Some((start, blank)) = self.dispatch.take()
            {
                let took = now.saturating_duration_since(start);
                self.slow += u64::from(took >= SLOW);
                let period = self.period.unwrap_or(Duration::from_micros(16_667));
                self.past += u64::from(
                    blank.is_some_and(|blank| now.saturating_duration_since(blank) > period),
                );
            }
            let took = now.saturating_duration_since(began);
            self.segments[kind as usize].add(took);
            if flush {
                self.pointer.add(took);
            }
        }
    }
    /// The draw request just recorded was held for a blank: its stretch is
    /// relabelled, and a pointer move it would have flushed stays pending.
    fn held(&mut self) {
        if let Some((Segment::Paint, began, flush)) = self.open {
            self.open = Some((Segment::Held, began, false));
            self.pending_move |= flush;
        }
        self.held += 1;
    }
    /// Slint begins drawing (`BeforeRendering`): what the draw's segment
    /// held so far was preparation, the pointer flush among it.
    fn draw_began(&mut self, now: Instant) {
        if let Some((Segment::Paint, began, flush)) = self.open {
            let pre = now.saturating_duration_since(began);
            self.pre.add(pre);
            if flush {
                self.pointer.add(pre);
            }
            self.open = Some((Segment::Paint, now, false));
        }
    }
    /// Slint drew (`AfterRendering`); the present follows.
    fn draw_ended(&mut self, now: Instant) {
        if let Some((Segment::Paint, began, _)) = self.open {
            self.draw.add(now.saturating_duration_since(began));
            self.after = Some(now);
        }
    }
    /// winit starts a loop turn (Slint's timers run next).
    fn new_events(&mut self, now: Instant) {
        self.end_segment(now);
        self.turn_began = Some(now);
        self.open = Some((Segment::Timers, now, false));
    }
    /// A window event is about to be dispatched.
    fn event(&mut self, now: Instant, kind: Segment) {
        self.end_segment(now);
        let flush = kind != Segment::Cursor && std::mem::take(&mut self.pending_move);
        if kind == Segment::Cursor {
            self.moves += 1;
            self.pending_move = true;
        }
        self.open = Some((kind, now, flush));
    }
    /// The loop is about to wait (Slint processed pending pointer moves
    /// just before this).
    fn about_to_wait(&mut self, now: Instant) {
        if std::mem::take(&mut self.pending_move)
            && let Some(open) = self.open.as_mut()
        {
            open.2 = true;
        }
        self.end_segment(now);
        if let Some(began) = self.turn_began.take() {
            self.turn.add(now.saturating_duration_since(began));
        }
    }
    /// The counters as text, then zeroed.
    fn text(&mut self, skipped: u64, passive: bool) -> String {
        let ms = |d: Duration| d.as_secs_f64() * 1000.;
        let seg = |kind: Segment| &self.segments[kind as usize];
        let text = format!(
            " pacing: moves {} held {} eager {} close {} refused {} skipped {skipped} late max {:.1} ms paint {} slow {} past-blank {} ui: turn max {} timers max {} cursor max {} pre {} pointer {} other max {} ms draw {} present {} paced {}",
            self.moves,
            self.held,
            self.eager,
            self.close,
            self.refused,
            ms(self.late),
            seg(Segment::Paint).text(),
            self.slow,
            self.past,
            self.turn.max_text(),
            seg(Segment::Timers).max_text(),
            seg(Segment::Cursor).max_text(),
            self.pre.text(),
            self.pointer.text(),
            seg(Segment::Other).max_text(),
            self.draw.text(),
            self.present.text(),
            if passive { "no" } else { "yes" },
        );
        *self = Self {
            open: self.open,
            turn_began: self.turn_began,
            pending_move: self.pending_move,
            last_draw: self.last_draw,
            last_blank: self.last_blank,
            period: self.period,
            dispatch: self.dispatch,
            ..Self::default()
        };
        text
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
    /// `on_draw`.
    static ON_DRAW: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
}

/// Called for each draw of the main window the hook lets through (UI
/// thread): the host's draw count where the window has no rendering
/// notifier (`platform::window_notifiers`). `None` removes it.
pub fn on_draw(callback: Option<Box<dyn Fn()>>) {
    ON_DRAW.with(|slot| *slot.borrow_mut() = callback);
}
fn drew() {
    // Diagnostics: the GPU reaching this draw (` ui-gpu wait`).
    crate::platform::ui_draw_started();
    ON_DRAW.with(|slot| {
        if let Some(callback) = &*slot.borrow() {
            callback();
        }
    });
}

/// How to hand the main window the newest video picture again after a
/// refused one (`accept_picture`); `None` at the end of the event loop.
pub fn on_deliver(deliver: Option<Box<dyn Fn()>>) {
    DELIVER.with(|slot| *slot.borrow_mut() = deliver);
}

/// Whether to pace the main window here: the system has a display clock and
/// `VESTRA_FRAME_PACING=slint` (A/B tests, a fallback in the field) is not set.
/// (`=every` paces too, drawing at every vertical blank: `every_blank`.)
pub fn wanted() -> bool {
    crate::platform::VBLANK_CLOCK && std::env::var("VESTRA_FRAME_PACING").as_deref() != Ok("slint")
}

/// Whether GPU frames shown without a rendering notifier (Windows wgpu
/// windows) go through this pacing like software pictures: the window takes
/// a new frame only when the previous one was drawn (`accept_picture`), and
/// the draw is owed when the frame's image is set (`picture_set`), not when
/// the renderer publishes it. `VESTRA_GPU_FRAME_GATE=0` (A/B tests): the
/// window takes the newest frame at once and each published frame owes a
/// draw (wgpu round record §17).
pub fn gpu_frames_paced() -> bool {
    static PACED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    wanted() && *PACED.get_or_init(|| std::env::var("VESTRA_GPU_FRAME_GATE").as_deref() != Ok("0"))
}

/// Whether the winit hook is installed (`Handler`): where the host can pace
/// (`platform::VBLANK_CLOCK`). With `VESTRA_FRAME_PACING=slint` it only
/// measures the main window's draws (`paced no` in the frame log).
pub fn hooked() -> bool {
    crate::platform::VBLANK_CLOCK
}

/// The winit hook (installed with the backend when `hooked`): holds the
/// main window's redraw requests per the gate.
pub struct Handler;
impl CustomApplicationHandler for Handler {
    fn new_events(
        &mut self,
        _: &winit::event_loop::ActiveEventLoop,
        _: winit::event::StartCause,
    ) -> EventResult {
        PACING.with(|pacing| {
            if let Some(pacing) = pacing.borrow_mut().as_mut() {
                pacing.cycles.close_paint();
                pacing.stats.new_events(Instant::now());
            }
        });
        EventResult::Propagate
    }
    fn window_event(
        &mut self,
        _: &winit::event_loop::ActiveEventLoop,
        window_id: winit::window::WindowId,
        _: Option<&winit::window::Window>,
        _: Option<&slint::Window>,
        event: &winit::event::WindowEvent,
    ) -> EventResult {
        PACING.with(|pacing| {
            let mut pacing = pacing.borrow_mut();
            let Some(pacing) = pacing
                .as_mut()
                .filter(|p| p.window == window_id && !p.failed)
            else {
                return EventResult::Propagate;
            };
            pacing.cycles.close_paint();
            let now = Instant::now();
            if matches!(
                event,
                winit::event::WindowEvent::Moved(_)
                    | winit::event::WindowEvent::ScaleFactorChanged { .. }
            ) {
                // Perhaps onto another display: its vertical blanks from the
                // next wait on, not up to a second later.
                pacing.clock.moved.store(true, Ordering::Release);
            }
            if matches!(event, winit::event::WindowEvent::RedrawRequested) {
                pacing.counts.requested += 1;
            }
            let kind = match event {
                winit::event::WindowEvent::RedrawRequested => Segment::Paint,
                winit::event::WindowEvent::CursorMoved { .. } => Segment::Cursor,
                _ => Segment::Other,
            };
            pacing.stats.event(now, kind);
            if kind != Segment::Paint {
                return EventResult::Propagate;
            }
            if pacing.passive {
                pacing.stats.draw(now, false);
                pacing
                    .counts
                    .drawn(now, pacing.stats.last_blank, pacing.stats.period);
                pacing.cycles.paint_at = Some(crate::platform::thread_cycles());
                drew();
                return EventResult::Propagate;
            }
            let decision = pacing.gate.redraw_requested(now);
            if decision.arm {
                pacing.arm();
            }
            if decision.draw {
                pacing.stats.draw(now, decision.arm);
                pacing
                    .counts
                    .drawn(now, pacing.stats.last_blank, pacing.stats.period);
                pacing.cycles.paint_at = Some(crate::platform::thread_cycles());
                drew();
            } else {
                pacing.stats.held();
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
    fn about_to_wait(&mut self, _: &winit::event_loop::ActiveEventLoop) -> EventResult {
        PACING.with(|pacing| {
            if let Some(pacing) = pacing.borrow_mut().as_mut() {
                pacing.cycles.close_paint();
                pacing.stats.about_to_wait(Instant::now());
            }
        });
        EventResult::Propagate
    }
}

/// Slint begins drawing the main window (its rendering notifier's
/// `BeforeRendering`): splits the draw's UI time from its preparation.
pub fn draw_began() {
    PACING.with(|pacing| {
        if let Some(pacing) = pacing.borrow_mut().as_mut() {
            pacing.stats.draw_began(Instant::now());
            pacing.cycles.draw_at = Some(crate::platform::thread_cycles());
        }
    });
}

/// Slint drew the main window (its rendering notifier's `AfterRendering`):
/// splits the draw (`draw`) from the present after it (`present`).
pub fn draw_ended() {
    PACING.with(|pacing| {
        if let Some(pacing) = pacing.borrow_mut().as_mut() {
            pacing.stats.draw_ended(Instant::now());
            if let Some(at) = pacing.cycles.draw_at.take() {
                let used = crate::platform::thread_cycles().saturating_sub(at);
                let draw = pacing.cycles.draw;
                pacing.cycles.draw = (draw.0 + 1, draw.1 + used);
            }
        }
    });
}

/// `VESTRA_FRAME_LOG`: what the pacing did since the last call, appended to
/// the frame log's line ("" without pacing, or after the clock failed): pointer moves over the main
/// window, held redraw requests, draws at once with the clock stopped
/// (`eager`), draws less than half a refresh apart (`close`), blanks the
/// UI thread was too busy to take (`skipped`), the longest a blank waited
/// for the UI thread (`late`), and each draw dispatch until winit's next
/// callback (`paint`: Slint's draw and `SwapBuffers`), dispatches of `SLOW`
/// or longer (`slow`) and those that ended after the next blank
/// (`past-blank`).
pub fn log_text() -> String {
    PACING.with(|pacing| {
        pacing
            .borrow_mut()
            .as_mut()
            .filter(|p| !p.failed)
            .map_or_else(String::new, |pacing| {
                let skipped = pacing.clock.skipped.swap(0, Ordering::Relaxed);
                pacing.stats.text(skipped, pacing.passive)
                    + &pacing.cycles.text()
                    + &pacing.counts.text()
                    + &crate::platform::ui_gpu_text()
            })
    })
}

/// Pace `shell` (the main window, after `show()`; when `hooked`); a rebuilt
/// main window replaces the previous one. Nothing without a display clock.
/// A shell shown in the native window already paced (a new composition
/// replaces the tree in the same window) keeps its clock and gate.
pub fn follow(shell: &slint_interpreter::ComponentInstance) {
    if !hooked() {
        return;
    }
    let window = shell.window().with_winit_window(|window| window.id());
    let kept = PACING.with(|pacing| {
        let mut pacing = pacing.borrow_mut();
        match pacing.as_mut() {
            Some(pacing) if Some(pacing.window) == window => {
                pacing.shell = shell.as_weak();
                true
            }
            _ => false,
        }
    });
    if kept {
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
    if !wanted() {
        // Measure only: no clock, nothing held.
        PACING.with(|pacing| {
            *pacing.borrow_mut() = Some(Pacing {
                gate: Gate::default(),
                window,
                shell: shell.as_weak(),
                clock: Arc::new(Clock::default()),
                failed: false,
                passive: true,
                stats: Stats::default(),
                cycles: Cycles::default(),
                counts: Counts::default(),
            });
        });
        return;
    }
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
            passive: false,
            stats: Stats::default(),
            cycles: Cycles::default(),
            counts: Counts::default(),
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
            .filter(|p| !p.failed && !p.passive)
            .is_none_or(|p| {
                let accepted = p.gate.accept_picture(Instant::now());
                p.stats.refused += u64::from(!accepted);
                accepted
            })
    })
}

/// A new video picture was set on the main window: draw it at the next
/// vertical blank.
pub fn picture_set() {
    PACING.with(|pacing| {
        if let Some(pacing) = pacing.borrow_mut().as_mut() {
            pacing.counts.pictures += 1;
        }
        if let Some(pacing) = pacing
            .borrow_mut()
            .as_mut()
            .filter(|p| !p.failed && !p.passive)
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
        if let Some(pacing) = pacing.borrow_mut().as_mut().filter(|p| !p.passive)
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
    ON_DRAW.with(|slot| slot.borrow_mut().take());
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

    /// One refresh at 60 Hz.
    const REFRESH: Duration = Duration::from_micros(16_667);

    #[test]
    fn an_idle_window_draws_the_first_request_at_once_and_starts_the_clock() {
        let mut gate = Gate::default();
        let mut now = Instant::now();
        assert_eq!(gate.redraw_requested(now), EAGER);
        // Further requests wait for the vertical blank.
        assert_eq!(gate.redraw_requested(now), HOLD);
        assert_eq!(gate.redraw_requested(now), HOLD);
        now += REFRESH;
        assert_eq!(gate.vblank(now, false), Blank::Draw, "owed: draw");
        // The draw the pacer asked for goes through.
        assert_eq!(gate.redraw_requested(now), DRAW);
        // Nothing owed: the clock keeps running for LINGER, then stops.
        let drawn = now;
        loop {
            now += REFRESH;
            let blank = gate.vblank(now, false);
            if now - drawn < LINGER {
                assert_eq!(blank, Blank::Wait);
            } else {
                assert_eq!(blank, Blank::Stop);
                break;
            }
        }
        // Idle again: the next request draws at once.
        assert_eq!(gate.redraw_requested(now), EAGER);
    }

    #[test]
    fn video_frames_draw_once_per_vertical_blank() {
        let mut gate = Gate::default();
        let mut now = Instant::now();
        assert!(gate.accept_picture(now));
        assert!(gate.picture_set(now), "first picture starts the clock");
        // Slint's timer ticks before the blank: held, still one draw.
        assert_eq!(gate.redraw_requested(now), HOLD);
        now += REFRESH;
        assert_eq!(gate.vblank(now, false), Blank::Draw);
        assert_eq!(gate.redraw_requested(now), DRAW);
        // A blank with no new picture and no request: no draw, the clock runs on.
        now += REFRESH;
        assert_eq!(gate.vblank(now, false), Blank::Wait);
        // One picture per refresh: each blank draws the new one.
        for _ in 0..3 {
            assert!(gate.accept_picture(now));
            assert!(!gate.picture_set(now), "the clock runs");
            now += REFRESH;
            assert_eq!(gate.vblank(now, false), Blank::Draw);
            assert_eq!(gate.redraw_requested(now), DRAW);
        }
    }

    /// 0.5.2: a UI change (a hover) in a refresh that follows one without
    /// a new picture waits for the blank like the pictures do. Before, the
    /// clock had stopped, the hover drew at once, and the picture that came
    /// later in the same refresh drew again at the blank: two draws in one
    /// refresh, the second blocking in `SwapBuffers`.
    #[test]
    fn a_ui_change_between_pictures_waits_for_the_blank() {
        let mut gate = Gate::default();
        let mut now = Instant::now();
        // Playing already: a picture drawn at a blank.
        gate.picture_set(now);
        now += REFRESH;
        assert_eq!(gate.vblank(now, false), Blank::Draw);
        assert_eq!(gate.redraw_requested(now), DRAW);
        let mut draws = vec![now];
        // 24 fps on 60 Hz: a picture in 2 of 5 refreshes, at 0.8 of it; a
        // hover change at 0.5 of each such refresh.
        for refresh in 0..60u32 {
            let start = now;
            if refresh * 24 / 60 != (refresh + 1) * 24 / 60 {
                let hover = start + REFRESH / 2;
                if gate.redraw_requested(hover).draw {
                    draws.push(hover);
                }
                let picture = start + REFRESH * 4 / 5;
                assert!(gate.accept_picture(picture));
                gate.picture_set(picture);
            }
            now += REFRESH;
            if gate.vblank(now, false) == Blank::Draw {
                assert_eq!(gate.redraw_requested(now), DRAW);
                draws.push(now);
            }
        }
        assert_eq!(draws.len(), 1 + 24, "one draw per picture");
        assert!(
            draws.windows(2).all(|pair| pair[1] - pair[0] >= REFRESH),
            "two draws within one refresh: {draws:?}"
        );
    }

    #[test]
    fn a_second_picture_within_one_refresh_waits_for_the_next() {
        let mut gate = Gate::default();
        let mut now = Instant::now();
        assert!(gate.accept_picture(now));
        gate.picture_set(now);
        // The next picture arrives before the blank drew the first: it
        // stays in the slot ...
        assert!(!gate.accept_picture(now + Duration::from_millis(2)));
        now += REFRESH;
        assert_eq!(gate.vblank(now, false), Blank::Draw);
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
        now += REFRESH;
        assert_eq!(gate.vblank(now, false), Blank::Draw);
        assert_eq!(gate.redraw_requested(now), DRAW, "delivered once");
    }

    #[test]
    fn animations_draw_at_every_vertical_blank() {
        let mut gate = Gate::default();
        let mut now = Instant::now();
        assert_eq!(gate.redraw_requested(now), EAGER);
        for _ in 0..3 {
            now += REFRESH;
            assert_eq!(gate.vblank(now, true), Blank::Draw);
            assert_eq!(gate.redraw_requested(now), DRAW);
        }
        now += REFRESH;
        assert_eq!(gate.vblank(now, false), Blank::Wait, "animation over");
        assert_eq!(gate.vblank(now + LINGER, false), Blank::Stop);
    }

    #[test]
    fn a_draw_the_pacer_asked_for_covers_what_was_owed() {
        let mut gate = Gate::default();
        let now = Instant::now();
        gate.picture_set(now);
        assert_eq!(gate.vblank(now + REFRESH, false), Blank::Draw);
        // winit coalesces Slint's tick with the pacer's request: one draw,
        // nothing owed afterwards.
        assert_eq!(gate.redraw_requested(now + REFRESH), DRAW);
        assert_eq!(gate.vblank(now + REFRESH * 2, false), Blank::Wait);
    }

    #[test]
    fn the_log_counts_close_and_eager_draws() {
        let mut stats = Stats::default();
        let ms = |n: u64| Duration::from_millis(n);
        let now = Instant::now();
        stats.blank(now, now + ms(3));
        stats.blank(now + REFRESH, now + REFRESH);
        let t = now + REFRESH;
        // A turn: timers 2 ms, a pointer move stored, a draw of 5 ms (eager,
        // the clock was stopped) which processes the pending move first (1 ms).
        stats.new_events(t);
        stats.event(t + ms(2), Segment::Cursor);
        stats.event(t + ms(3), Segment::Paint);
        stats.draw(t + ms(3), true);
        stats.draw_began(t + ms(4));
        stats.event(t + ms(8), Segment::Other);
        stats.about_to_wait(t + ms(9));
        // A second draw within half a refresh.
        stats.new_events(t + ms(10));
        stats.event(t + ms(10), Segment::Paint);
        stats.draw(t + ms(10), false);
        stats.about_to_wait(t + ms(11));
        // Two held requests: Slint does not run for them, so their stretches
        // count neither as paint nor as pointer time ...
        stats.new_events(t + ms(12));
        stats.event(t + ms(12), Segment::Paint);
        stats.held();
        stats.about_to_wait(t + ms(13));
        // ... and a pointer move stored before one stays pending: Slint
        // processes it when the loop is about to wait (2 ms here).
        stats.new_events(t + ms(14));
        stats.event(t + ms(14), Segment::Cursor);
        stats.event(t + ms(14), Segment::Paint);
        stats.held();
        stats.about_to_wait(t + ms(16));
        assert_eq!(
            stats.text(1, false),
            " pacing: moves 2 held 2 eager 1 close 1 refused 0 skipped 1 late max 3.0 ms paint max 4.0 mean 2.5 ms slow 0 past-blank 0 ui: turn max 9.0 timers max 2.0 cursor max 1.0 pre max 1.0 mean 1.0 ms pointer max 2.0 mean 1.5 ms other max 1.0 ms draw - present - paced yes"
        );
        // Zeroed; a pointer move processed at the wait counts as pointer time.
        stats.new_events(t + ms(20));
        stats.event(t + ms(20), Segment::Cursor);
        stats.about_to_wait(t + ms(24));
        assert!(
            stats
                .text(0, false)
                .ends_with("paint - slow 0 past-blank 0 ui: turn max 4.0 timers max 0.0 cursor max 4.0 pre - pointer max 4.0 mean 4.0 ms other max 0.0 ms draw - present - paced yes"),
            "{}",
            stats.text(0, false)
        );
    }

    /// A draw dispatch of `SLOW` or more is `slow`; one that ends more than
    /// a refresh after the blank it followed is `past-blank`; an eager draw
    /// (the clock was stopped) follows no blank.
    #[test]
    fn the_log_counts_slow_draws_and_draws_past_the_next_blank() {
        let (mut stats, t) = (Stats::default(), Instant::now());
        let ms = Duration::from_millis;
        stats.blank(t, t);
        stats.blank(t + REFRESH, t + REFRESH);
        let blank = t + REFRESH;
        // 3 ms: neither.
        stats.event(blank, Segment::Paint);
        stats.draw(blank, false);
        stats.new_events(blank + ms(3));
        // 10 ms: slow, still before the next blank.
        stats.event(blank + ms(4), Segment::Paint);
        stats.draw(blank + ms(4), false);
        stats.new_events(blank + ms(14));
        // 3 ms, but it ends 18 ms after its blank: past it.
        stats.event(blank + ms(15), Segment::Paint);
        stats.draw(blank + ms(15), false);
        stats.new_events(blank + ms(18));
        // Eager and long: slow, but no blank to be past.
        stats.event(blank + ms(40), Segment::Paint);
        stats.draw(blank + ms(40), true);
        stats.new_events(blank + ms(60));
        let text = stats.text(0, false);
        assert!(text.contains(" slow 2 past-blank 1 ui:"), "{text}");
        assert!(stats.text(0, false).contains(" slow 0 past-blank 0 ui:"));
    }

    /// A draw's dispatch splits into preparation (pointer flush, Slint's
    /// preparation, the wgpu acquire), Slint's draw, and the present.
    #[test]
    fn a_draw_splits_into_preparation_draw_and_present() {
        let (mut stats, t) = (Stats::default(), Instant::now());
        let ms = Duration::from_millis;
        stats.event(t, Segment::Paint);
        stats.draw_began(t + ms(3));
        stats.draw_ended(t + ms(5));
        stats.new_events(t + ms(12));
        let text = stats.text(0, true);
        assert!(text.contains(" pre max 3.0 mean 3.0 ms "), "{text}");
        assert!(
            text.ends_with("draw max 2.0 mean 2.0 ms present max 7.0 mean 7.0 ms paced no"),
            "{text}"
        );
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
