//! C-SIZE (R-H2) and the render cadence: how large the software renderer
//! draws, when a new size is applied, and how a finished frame reaches the UI.
//! Pure decisions live here so they can be tested without libmpv or a window.
#![forbid(unsafe_code)]

use std::sync::{
    Arc, Mutex, PoisonError,
    atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

/// A pixel size (width, height).
pub type Size = (u32, u32);

/// Before any video area is known (headless, startup): the old fixed size.
pub const INITIAL_AREA: Size = (960, 540);
/// The source size assumed while mpv has not reported one.
pub const UNKNOWN_SOURCE: Size = (1920, 1080);
/// Absolute ceiling on rendered pixels: 3840×2160 (8.3 MP).
pub const MAX_PIXELS: u64 = 3840 * 2160;
/// A dragged size is applied at most this often; the last one within it.
pub const THROTTLE: Duration = Duration::from_millis(50);

/// Round to the nearest even number, at least 2.
fn even(value: f64) -> u32 {
    let rounded = (value / 2.).round() * 2.;
    if rounded.is_finite() && rounded >= 2. {
        rounded.min(f64::from(u32::MAX - 1)) as u32
    } else {
        2
    }
}

/// The physical video area: logical size × scale factor, even, at least 2×2.
/// `None` while the area is empty (minimised, no shell): keep the last target.
pub fn physical(logical: (f64, f64), scale: f64) -> Option<Size> {
    let (width, height) = logical;
    let scale = if scale.is_finite() && scale > 0. {
        scale
    } else {
        1.
    };
    (width.is_finite() && height.is_finite() && width > 0. && height > 0.)
        .then(|| (even(width * scale), even(height * scale)))
}

/// The render target for a physical area `area`, a source display size and the
/// display-layer zoom: the area scaled by `k = min(1, max(S.w/A.w, S.h/A.h) ×
/// max(1, zoom))`, so it keeps the area's aspect (keepaspect, panscan and
/// aspect overrides stay correct) but never draws more pixels than the source
/// supplies, except when zoomed in. Then the absolute pixel ceiling.
pub fn target(area: Size, source: Option<Size>, zoom: f64) -> Size {
    let (aw, ah) = (f64::from(area.0.max(2)), f64::from(area.1.max(2)));
    let (sw, sh) = source
        .filter(|(w, h)| *w > 0 && *h > 0)
        .unwrap_or(UNKNOWN_SOURCE);
    let zoom = if zoom.is_finite() { zoom.max(1.) } else { 1. };
    let mut k = ((f64::from(sw) / aw).max(f64::from(sh) / ah) * zoom).min(1.);
    let pixels = aw * ah * k * k;
    if pixels > MAX_PIXELS as f64 {
        k *= (MAX_PIXELS as f64 / pixels).sqrt();
    }
    let (mut width, mut height) = (even(aw * k), even(ah * k));
    // Rounding up to even can step over the ceiling by a row or column.
    while u64::from(width) * u64::from(height) > MAX_PIXELS && width > 2 && height > 2 {
        width -= 2;
        height = even(f64::from(width) * ah / aw);
    }
    (width, height)
}

/// `VESTRA_RENDER_TARGET=WxH` (diagnostics, next to `VESTRA_FRAME_LOG`): the
/// software renderer draws every frame at this size, whatever the window, so
/// the per-frame cost of a large target (mpv's render, the upload, the draw)
/// can be measured on a small screen; the picture is scaled into the video
/// area. Even sizes up to `MAX_PIXELS`; anything else is ignored.
pub fn forced() -> Option<Size> {
    static FORCED: std::sync::OnceLock<Option<Size>> = std::sync::OnceLock::new();
    *FORCED.get_or_init(|| {
        let value = std::env::var("VESTRA_RENDER_TARGET").ok()?;
        let size = parse_size(&value);
        if size.is_none() {
            diag!("VESTRA_RENDER_TARGET={value}: not an even WxH up to 3840x2160; ignored");
        }
        size
    })
}

/// `WxH` with even sides of at least 2 and at most `MAX_PIXELS` pixels.
fn parse_size(text: &str) -> Option<Size> {
    let (width, height) = text.trim().split_once(['x', 'X'])?;
    let (width, height): (u32, u32) = (width.trim().parse().ok()?, height.trim().parse().ok()?);
    (width >= 2
        && height >= 2
        && width % 2 == 0
        && height % 2 == 0
        && u64::from(width) * u64::from(height) <= MAX_PIXELS)
        .then_some((width, height))
}

/// Applies a changing area at most every `THROTTLE`, always the final value
/// within `THROTTLE` after the changes stop, and at once when asked to
/// (fullscreen, scale-factor change).
#[derive(Debug, Default)]
pub struct Throttle {
    applied: Option<Size>,
    pending: Option<Size>,
    at: Option<Instant>,
}
impl Throttle {
    /// Offer the newest area; returns the size to apply now, if any.
    pub fn offer(&mut self, size: Size, immediate: bool, now: Instant) -> Option<Size> {
        if self.applied == Some(size) {
            self.pending = None;
            return None;
        }
        let due = immediate || self.at.is_none_or(|at| now.duration_since(at) >= THROTTLE);
        if due {
            self.pending = None;
            self.applied = Some(size);
            self.at = Some(now);
            Some(size)
        } else {
            self.pending = Some(size);
            None
        }
    }
    /// Called every tick: the held value once its turn comes.
    pub fn poll(&mut self, now: Instant) -> Option<Size> {
        let size = self.pending?;
        self.offer(size, false, now)
    }
}

/// The UI side: the video area each tick → the area to send to the renderer.
#[derive(Debug, Default)]
pub struct Sizing {
    throttle: Throttle,
    /// (fullscreen, scale factor) last seen; a change applies at once.
    seen: Option<(bool, u64)>,
}
impl Sizing {
    pub fn observe(
        &mut self,
        logical: (f64, f64),
        scale: f64,
        fullscreen: bool,
        now: Instant,
    ) -> Option<Size> {
        let facts = (fullscreen, scale.to_bits());
        let immediate = self.seen.is_some_and(|seen| seen != facts);
        self.seen = Some(facts);
        match physical(logical, scale) {
            Some(area) => self.throttle.offer(area, immediate, now),
            None => self.throttle.poll(now),
        }
    }
}

/// What the UI asks the renderer for: the physical area and the display zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Request {
    pub area: Size,
    pub zoom: f64,
}

/// Shared between the UI (area), the control worker (zoom) and the renderer.
pub struct Shared {
    request: Mutex<Request>,
    revision: AtomicU64,
    /// The picture's size inside the last rendered frame (the frame less
    /// mpv's margins, `osd-dimensions`), from the control worker.
    picture: Mutex<Option<(f64, f64)>>,
}
impl Default for Shared {
    fn default() -> Self {
        Self {
            request: Mutex::new(Request {
                area: INITIAL_AREA,
                zoom: 1.,
            }),
            revision: AtomicU64::new(0),
            picture: Mutex::new(None),
        }
    }
}
impl Shared {
    pub fn set_area(&self, area: Size) {
        self.update(|request| request.area = area);
    }
    /// The display layer's zoom (R-V9): zooming in renders more pixels.
    pub fn set_zoom(&self, zoom: f64) {
        self.update(|request| request.zoom = zoom);
    }
    fn update(&self, change: impl FnOnce(&mut Request)) {
        let mut request = self.request.lock().unwrap_or_else(PoisonError::into_inner);
        let before = *request;
        change(&mut request);
        if *request != before {
            self.revision.fetch_add(1, Ordering::AcqRel);
        }
    }
    /// The control worker: the picture's size in the last rendered frame.
    pub fn set_picture(&self, picture: Option<(f64, f64)>) {
        let picture =
            picture.filter(|(w, h)| w.is_finite() && h.is_finite() && *w >= 1. && *h >= 1.);
        *self.picture.lock().unwrap_or_else(PoisonError::into_inner) = picture;
    }
    pub fn picture(&self) -> Option<(f64, f64)> {
        *self.picture.lock().unwrap_or_else(PoisonError::into_inner)
    }
    /// The current request and its revision (bumped by every change).
    pub fn get(&self) -> (u64, Request) {
        let request = *self.request.lock().unwrap_or_else(PoisonError::into_inner);
        (self.revision.load(Ordering::Acquire), request)
    }
}

/// Picture-in-picture (R-H8): the largest area of the picture's shape inside
/// the physical video area, so the renderer draws the picture without bars
/// (the PiP window takes the frame's shape and shows the whole frame).
/// Rendering at it gives frames without margins, so the measured picture
/// keeps that shape: stable, and it follows a new source's shape.
pub fn fit(area: Size, picture: (f64, f64)) -> Size {
    let (aw, ah) = (f64::from(area.0.max(2)), f64::from(area.1.max(2)));
    let (pw, ph) = picture;
    if !(pw.is_finite() && ph.is_finite() && pw >= 1. && ph >= 1.) {
        return area;
    }
    let k = (aw / pw).min(ah / ph);
    let fitted = (
        even(pw * k).min(area.0.max(2)),
        even(ph * k).min(area.1.max(2)),
    );
    // Within a pixel or two of the area: the area (no needless resize).
    if area.0.abs_diff(fitted.0) <= 2 && area.1.abs_diff(fitted.1) <= 2 {
        area
    } else {
        fitted
    }
}

/// Whether a frame of `frame` pixels has the shape of the requested `sent`
/// size (the software renderer may draw smaller, never another shape).
pub fn same_shape(frame: Size, sent: Size) -> bool {
    let ratio = |(w, h): Size| f64::from(w.max(1)) / f64::from(h.max(1));
    (ratio(frame) / ratio(sent) - 1.).abs() < 0.01
}

/// When the renderer draws: on mpv's update, and also once after the target
/// changed while nothing new arrives (paused), so the picture follows the new
/// area instead of staying at the old aspect. Nothing is redrawn before the
/// first real frame.
#[derive(Debug, Default)]
pub struct Redraw {
    seen: u64,
    rendered: bool,
}
impl Redraw {
    pub fn due(&mut self, update: bool, revision: u64) -> bool {
        let resized = revision != self.seen;
        let due = update || (resized && self.rendered);
        if due {
            self.seen = revision;
            self.rendered = true;
        } else if resized {
            // Before the first frame the next update draws at the new size.
            self.seen = revision;
        }
        due
    }
}

/// The software renderer's frame buffers, reused round-robin. A new
/// `SharedPixelBuffer` per frame costs a zero-filled allocation of the whole
/// frame plus its page faults, and the UI thread frees it again: measured
/// 2026-10-25 on the Windows test machine (Ryzen 9 7940HS, mpv software render
/// at 3840x2160, 10-bit AV1 and HEVC 4K60 through d3d11va-copy), about 28 ms a
/// frame (30 fps) against 8 ms (60 fps) with reused buffers.
///
/// `take` hands out the next buffer of the ring and the ring keeps a handle to
/// it (`keep`). The UI drops its handles a frame or two later, so when the
/// buffer comes round again (`RING` frames later) the ring's handle is the
/// only one and `make_mut_bytes` writes in place; if the UI still held it,
/// `make_mut_bytes` would copy (correct, only slower).
#[derive(Default)]
pub struct BufferRing {
    slots: Vec<Option<slint::SharedPixelBuffer<slint::Rgb8Pixel>>>,
    next: usize,
}
/// Buffers in the ring: one being drawn, one waiting in the frame slot, one
/// or two held by the UI (the shown picture, the one being uploaded).
pub const RING: usize = 4;
impl BufferRing {
    /// The next buffer for a `width`×`height` frame (a new one when the slot
    /// is empty or of another size).
    pub fn take(&mut self, width: u32, height: u32) -> slint::SharedPixelBuffer<slint::Rgb8Pixel> {
        if self.slots.len() != RING {
            self.slots = (0..RING).map(|_| None).collect();
        }
        self.next = (self.next + 1) % RING;
        match self.slots[self.next].take() {
            Some(buffer) if (buffer.width(), buffer.height()) == (width, height) => buffer,
            _ => slint::SharedPixelBuffer::new(width, height),
        }
    }
    /// Keep a handle to the buffer `take` just returned (after drawing into it).
    pub fn keep(&mut self, buffer: &slint::SharedPixelBuffer<slint::Rgb8Pixel>) {
        if let Some(slot) = self.slots.get_mut(self.next) {
            *slot = Some(buffer.clone());
        }
    }
}

/// One frame for the video area; `buffer` is drawn into directly and handed to
/// the UI without another copy.
pub struct Frame {
    pub buffer: slint::SharedPixelBuffer<slint::Rgb8Pixel>,
    pub sequence: u64,
}

type Waker = Box<dyn Fn() + Send + Sync>;

/// The newest-frame slot. Producing a frame wakes the UI at most once per
/// pending delivery (coalesced); a slow UI drops old frames instead of queueing.
#[derive(Default)]
pub struct FrameSlot {
    frame: Mutex<Option<Frame>>,
    wake_pending: AtomicBool,
    /// A delivery could not run (the UI was busy); the tick picks it up.
    missed: AtomicBool,
    waker: Mutex<Option<Waker>>,
    produced: AtomicU64,
    shown: AtomicU64,
    /// The software renderer's stages, for the frame log.
    pub stages: Stages,
}

/// What the software renderer did since the frame log's last line (render
/// thread and control worker write, the frame log reads): mpv render calls,
/// those for a new target only (no new frame), calls at another size than
/// the last (a new buffer and texture each), buffers copied because the UI
/// still held them, the time of each call and the work in it (the call
/// less mpv's wait for the frame's display time), mpv's frame updates, and
/// mpv's own counters of dropped and late frames.
#[derive(Debug, Default)]
pub struct Stages {
    calls: AtomicU64,
    resize_only: AtomicU64,
    resized: AtomicU64,
    copied: AtomicU64,
    updates: AtomicU64,
    took_total: AtomicU64,
    took_max: AtomicU64,
    work_total: AtomicU64,
    work_max: AtomicU64,
    /// mpv `frame-drop-count` and `vo-delayed-frame-count` (totals plus one; 0 while unread).
    dropped: AtomicI64,
    delayed: AtomicI64,
    /// The totals at the last line.
    dropped_seen: AtomicI64,
    delayed_seen: AtomicI64,
}
impl Stages {
    /// mpv said a new frame is ready.
    pub fn update(&self) {
        self.updates.fetch_add(1, Ordering::Relaxed);
    }
    /// One mpv render call: `fresh` for a new frame (else a new target
    /// only), at another size than the previous call, its buffer copied,
    /// how long it took and how much of that was work (not mpv's wait).
    pub fn call(&self, fresh: bool, resized: bool, copied: bool, took: Duration, work: Duration) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.resize_only
            .fetch_add(u64::from(!fresh), Ordering::Relaxed);
        self.resized
            .fetch_add(u64::from(resized), Ordering::Relaxed);
        self.copied.fetch_add(u64::from(copied), Ordering::Relaxed);
        let nanos = |d: Duration| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX);
        self.took_total.fetch_add(nanos(took), Ordering::Relaxed);
        self.took_max.fetch_max(nanos(took), Ordering::Relaxed);
        self.work_total.fetch_add(nanos(work), Ordering::Relaxed);
        self.work_max.fetch_max(nanos(work), Ordering::Relaxed);
    }
    /// The control worker: mpv's dropped and delayed frame totals.
    pub fn mpv_counts(&self, dropped: Option<i64>, delayed: Option<i64>) {
        self.dropped
            .store(dropped.map_or(0, |n| n.max(0) + 1), Ordering::Relaxed);
        self.delayed
            .store(delayed.map_or(0, |n| n.max(0) + 1), Ordering::Relaxed);
    }
    /// ` mpv: calls C resize-only T resized Z copied K updates U took max A mean B ms
    /// work max A mean B ms dropped D delayed L` since the last call (`-` for
    /// a counter mpv did not report).
    pub fn text(&self) -> String {
        let calls = self.calls.swap(0, Ordering::Relaxed);
        let ms = |nanos: u64| nanos as f64 / 1e6;
        let timing = |total: &AtomicU64, max: &AtomicU64| {
            let (total, max) = (
                total.swap(0, Ordering::Relaxed),
                max.swap(0, Ordering::Relaxed),
            );
            if calls == 0 {
                "-".to_owned()
            } else {
                format!("max {:.1} mean {:.1} ms", ms(max), ms(total) / calls as f64)
            }
        };
        let took = timing(&self.took_total, &self.took_max);
        let work = timing(&self.work_total, &self.work_max);
        let delta = |now: &AtomicI64, seen: &AtomicI64| {
            let now = now.load(Ordering::Relaxed);
            let before = seen.swap(now, Ordering::Relaxed);
            if now == 0 {
                "-".to_owned()
            } else {
                (now - before.max(1)).max(0).to_string()
            }
        };
        format!(
            " mpv: calls {calls} resize-only {} resized {} copied {} updates {} took {took} work {work} dropped {} delayed {}",
            self.resize_only.swap(0, Ordering::Relaxed),
            self.resized.swap(0, Ordering::Relaxed),
            self.copied.swap(0, Ordering::Relaxed),
            self.updates.swap(0, Ordering::Relaxed),
            delta(&self.dropped, &self.dropped_seen),
            delta(&self.delayed, &self.delayed_seen),
        )
    }
}
impl FrameSlot {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
    /// The UI's wake-up (headless runs have none and read the slot directly).
    pub fn set_waker(&self, waker: Waker) {
        *self.waker.lock().unwrap_or_else(PoisonError::into_inner) = Some(waker);
    }
    pub fn put(&self, frame: Frame) {
        *self.frame.lock().unwrap_or_else(PoisonError::into_inner) = Some(frame);
        self.produced.fetch_add(1, Ordering::Relaxed);
        if !self.wake_pending.swap(true, Ordering::AcqRel)
            && let Some(waker) = &*self.waker.lock().unwrap_or_else(PoisonError::into_inner)
        {
            waker();
        }
    }
    /// The delivery: clears the wake flag first, so a frame put after this
    /// take wakes the UI again.
    pub fn take(&self) -> Option<Frame> {
        self.wake_pending.store(false, Ordering::Release);
        self.missed.store(false, Ordering::Relaxed);
        let frame = self
            .frame
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if frame.is_some() {
            self.shown.fetch_add(1, Ordering::Relaxed);
        }
        frame
    }
    /// A wake-up arrived while the UI could not take the frame.
    pub fn miss(&self) {
        self.wake_pending.store(false, Ordering::Release);
        self.missed.store(true, Ordering::Relaxed);
    }
    pub fn missed(&self) -> bool {
        self.missed.load(Ordering::Relaxed)
    }
    /// Read the newest frame without taking it (headless diagnostics).
    pub fn peek<T>(&self, read: impl FnOnce(&Frame) -> T) -> Option<T> {
        self.frame
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(read)
    }
    /// (frames produced, frames set on the UI) since start.
    pub fn counts(&self) -> (u64, u64) {
        (
            self.produced.load(Ordering::Relaxed),
            self.shown.load(Ordering::Relaxed),
        )
    }
}

/// Longest and mean of a set of durations (the frame log's timings).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Timing {
    count: u64,
    total: Duration,
    max: Duration,
}
impl Timing {
    pub fn add(&mut self, duration: Duration) {
        self.count += 1;
        self.total += duration;
        self.max = self.max.max(duration);
    }
    pub fn count(&self) -> u64 {
        self.count
    }
    /// `max M mean A ms`, or `-` with nothing measured.
    fn text(&self) -> String {
        if self.count == 0 {
            return "-".into();
        }
        let ms = |d: Duration| d.as_secs_f64() * 1000.;
        format!(
            "max {:.1} mean {:.1} ms",
            ms(self.max),
            ms(self.total) / self.count as f64
        )
    }
}

/// What the window really draws (the presented counter): every draw of the
/// main window, and the draws that put a *new* video picture on screen.
/// mpv's counts stop at the renderer; frames lost after it (coalesced in the
/// slot, a missed vsync, a draw that repeats the old picture) show only here.
/// Counted on the UI thread; the control worker reads `presented_fps`.
pub struct Presentation {
    /// Bumped each time a new picture is set on the video area (pixels).
    picture: AtomicU64,
    /// Whether the main window's draws are counted (a hook is installed).
    counting: AtomicBool,
    /// Presented new-frame rate over the last second (f64 bits), `NONE`
    /// before the first second is measured.
    rate: AtomicU64,
    state: Mutex<DrawState>,
}
const NONE: u64 = u64::MAX;
#[derive(Default)]
struct DrawState {
    /// The picture number at the last draw.
    drawn_picture: u64,
    draws: u64,
    presented: u64,
    begun: Option<Instant>,
    /// Draw times (BeforeRendering → AfterRendering) and picture hand-offs
    /// (`show_frame`) since the frame log last read them.
    render: Timing,
    set: Timing,
    /// The rate's window: its start and the presented count then.
    window: Option<(Instant, u64)>,
}
impl Default for Presentation {
    fn default() -> Self {
        Self {
            picture: AtomicU64::new(0),
            counting: AtomicBool::new(false),
            rate: AtomicU64::new(NONE),
            state: Mutex::default(),
        }
    }
}
impl Presentation {
    fn state(&self) -> std::sync::MutexGuard<'_, DrawState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
    /// A hook counts the main window's draws from now on.
    pub fn start_counting(&self) {
        self.counting.store(true, Ordering::Release);
    }
    /// A new picture was set on the video area; `took` is how long setting it took.
    pub fn picture_set(&self, took: Duration) {
        self.picture.fetch_add(1, Ordering::AcqRel);
        self.state().set.add(took);
    }
    /// The window starts drawing (rendering notifier: BeforeRendering).
    pub fn draw_begin(&self, now: Instant) {
        self.state().begun = Some(now);
    }
    /// The window drew (AfterRendering, or winit's RedrawRequested where no
    /// notifier exists). `copied`: the GPU renderer copied a new frame into
    /// the window for this draw. A draw presents a new frame when a picture
    /// was set or copied since the last draw.
    pub fn draw_end(&self, now: Instant, copied: bool) {
        let picture = self.picture.load(Ordering::Acquire);
        let mut state = self.state();
        state.draws += 1;
        if copied || picture != state.drawn_picture {
            state.presented += 1;
        }
        state.drawn_picture = picture;
        if let Some(begun) = state.begun.take() {
            state.render.add(now.saturating_duration_since(begun));
        }
    }
    /// (draws, presented new frames) since start.
    pub fn counts(&self) -> (u64, u64) {
        let state = self.state();
        (state.draws, state.presented)
    }
    /// The draw and hand-off timings since the last call (the frame log).
    pub fn take_timings(&self) -> (Timing, Timing) {
        let mut state = self.state();
        (
            std::mem::take(&mut state.render),
            std::mem::take(&mut state.set),
        )
    }
    /// Called every host tick: closes the rate's one-second window.
    pub fn sample(&self, now: Instant) {
        if !self.counting.load(Ordering::Acquire) {
            return;
        }
        let mut state = self.state();
        let presented = state.presented;
        match state.window {
            None => state.window = Some((now, presented)),
            Some((at, from)) if now.duration_since(at) >= Duration::from_secs(1) => {
                let rate = (presented - from) as f64 / now.duration_since(at).as_secs_f64();
                self.rate.store(rate.to_bits(), Ordering::Release);
                state.window = Some((now, presented));
            }
            Some(_) => (),
        }
    }
    /// New video frames the window showed per second over the last second;
    /// `None` while not measured (headless, the first second).
    pub fn presented_fps(&self) -> Option<f64> {
        let bits = self.rate.load(Ordering::Acquire);
        (bits != NONE && self.counting.load(Ordering::Acquire)).then(|| f64::from_bits(bits))
    }
}

/// One second of the window's numbers for the frame log.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LogCounts {
    /// Frames the renderer produced (software sequence + GPU publishes).
    pub produced: u64,
    /// Frames the UI took (software) or copied into a window (GPU).
    pub shown: u64,
    /// Draws of the main window.
    pub draws: u64,
    /// Draws that showed a new video frame.
    pub presented: u64,
}

/// `VESTRA_FRAME_LOG=1`: one stderr line per second (GUI acceptance and
/// field diagnostics, not public API):
///
/// `vestra frames: produced P shown S (p/s fps) presented N redraws R
/// refresh 60.0 Hz ticks T tick max/mean render max/mean set max/mean target WxH`
///
/// - produced: frames the video renderer made; shown: frames the UI took
///   from it (software) or copied into a window (GPU);
/// - presented: draws of the main window that showed a new video frame;
///   redraws: draws that repeated the old picture;
/// - refresh: the window's display refresh rate; ticks: host ticks (16 ms
///   timer) run; tick, render (one window draw, which includes uploading the
///   new picture) and set (handing a picture to the video area): longest and
///   mean milliseconds on the UI thread.
pub struct FrameLog {
    last: Option<(Instant, LogCounts)>,
    ticks: Timing,
}
impl FrameLog {
    pub fn from_env() -> Option<Self> {
        (std::env::var("VESTRA_FRAME_LOG").as_deref() == Ok("1")).then(Self::new)
    }
    fn new() -> Self {
        Self {
            last: None,
            ticks: Timing::default(),
        }
    }
    /// One host tick's duration.
    pub fn tick_took(&mut self, duration: Duration) {
        self.ticks.add(duration);
    }
    /// The line for one interval, or `None` while the second is not over.
    pub fn line(
        &mut self,
        counts: LogCounts,
        presentation: &Presentation,
        refresh: impl FnOnce() -> Option<f64>,
        size: Option<Size>,
        now: Instant,
    ) -> Option<String> {
        let Some((at, last)) = self.last else {
            self.last = Some((now, counts));
            let _ = presentation.take_timings();
            self.ticks = Timing::default();
            return None;
        };
        if now.duration_since(at) < Duration::from_secs(1) {
            return None;
        }
        let seconds = now.duration_since(at).as_secs_f64();
        let delta = |now: u64, then: u64| now.saturating_sub(then);
        let (dp, ds) = (
            delta(counts.produced, last.produced),
            delta(counts.shown, last.shown),
        );
        let presented = delta(counts.presented, last.presented);
        let redraws = delta(counts.draws, last.draws).saturating_sub(presented);
        let size = size.map_or("-".into(), |(w, h)| format!("{w}x{h}"));
        let refresh = refresh().map_or("-".into(), |hz| format!("{hz:.1} Hz"));
        let (render, set) = presentation.take_timings();
        let ticks = std::mem::take(&mut self.ticks);
        self.last = Some((now, counts));
        Some(format!(
            "vestra frames: produced {dp} shown {ds} ({:.1}/{:.1} fps) presented {presented} redraws {redraws} refresh {refresh} ticks {} tick {} render {} set {} target {size}",
            dp as f64 / seconds,
            ds as f64 / seconds,
            ticks.count(),
            ticks.text(),
            render.text(),
            set.text(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stages_count_per_line() {
        let stages = Stages::default();
        assert_eq!(
            stages.text(),
            " mpv: calls 0 resize-only 0 resized 0 copied 0 updates 0 took - work - dropped - delayed -"
        );
        stages.mpv_counts(Some(5), Some(1));
        let _ = stages.text();
        stages.update();
        stages.call(
            true,
            false,
            false,
            Duration::from_millis(4),
            Duration::from_millis(3),
        );
        stages.call(
            false,
            true,
            true,
            Duration::from_millis(8),
            Duration::from_millis(5),
        );
        stages.mpv_counts(Some(9), Some(1));
        assert_eq!(
            stages.text(),
            " mpv: calls 2 resize-only 1 resized 1 copied 1 updates 1 took max 8.0 mean 6.0 ms work max 5.0 mean 4.0 ms dropped 4 delayed 0"
        );
    }

    #[test]
    fn the_buffer_ring_reuses_released_buffers_in_place() {
        let mut ring = BufferRing::default();
        let mut first = ring.take(64, 32);
        let address = first.make_mut_bytes().as_ptr();
        ring.keep(&first);
        // The UI holds `first` for a while, then drops it.
        let shown = first;
        for _ in 1..RING {
            let mut other = ring.take(64, 32);
            assert_ne!(other.make_mut_bytes().as_ptr(), address);
            ring.keep(&other);
        }
        drop(shown);
        // Round again: the same memory, written in place (no copy).
        let mut again = ring.take(64, 32);
        assert_eq!(again.make_mut_bytes().as_ptr(), address);
        ring.keep(&again);
        // Another size: a new buffer.
        for _ in 1..RING {
            let other = ring.take(64, 32);
            ring.keep(&other);
        }
        let resized = ring.take(32, 32);
        assert_eq!((resized.width(), resized.height()), (32, 32));
    }

    #[test]
    fn a_forced_target_is_an_even_size_within_the_ceiling() {
        assert_eq!(parse_size("3840x2160"), Some((3840, 2160)));
        assert_eq!(parse_size(" 2560 X 1600 "), Some((2560, 1600)));
        assert_eq!(parse_size("3841x2160"), None, "odd");
        assert_eq!(parse_size("3842x2160"), None, "over the ceiling");
        assert_eq!(parse_size("0x0"), None);
        assert_eq!(parse_size("1920"), None);
        assert_eq!(parse_size("wide"), None);
    }

    #[test]
    fn picture_in_picture_renders_the_picture_shape_inside_the_area() {
        // A 16:9 picture letterboxed in a 1560×1342 area: render 1560×878.
        assert_eq!(fit((1560, 1342), (1560., 877.5)), (1560, 878));
        // Pillarboxed (4:3 in a wide area).
        assert_eq!(fit((1920, 1080), (1440., 1080.)), (1440, 1080));
        // Rendering at the fitted size measures the same shape: stable.
        assert_eq!(fit((1560, 878), (1560., 878.)), (1560, 878));
        assert_eq!(fit((1560, 1342), (1560., 878.)), (1560, 878));
        // No margins, or within a pixel: the area itself.
        assert_eq!(fit((1920, 1080), (1919., 1080.)), (1920, 1080));
        // Nothing measured yet, or nonsense: the area.
        assert_eq!(fit((800, 600), (f64::NAN, 1.)), (800, 600));
        assert_eq!(fit((800, 600), (0., 600.)), (800, 600));
        // Portrait.
        assert_eq!(fit((1000, 1000), (1080., 1920.)), (562, 1000));
        assert!(same_shape((780, 439), (1560, 878)));
        assert!(!same_shape((1560, 1342), (1560, 878)));
        let shared = Shared::default();
        assert_eq!(shared.picture(), None);
        shared.set_picture(Some((1560., 878.)));
        assert_eq!(shared.picture(), Some((1560., 878.)));
        shared.set_picture(Some((0., 878.)));
        assert_eq!(shared.picture(), None);
    }

    #[test]
    fn the_target_follows_the_physical_area_capped_by_the_source() {
        // HiDPI: 780×500 logical points on a 2× screen.
        assert_eq!(physical((780., 500.), 2.), Some((1560, 1000)));
        assert_eq!(physical((781.3, 500.6), 1.), Some((782, 500)), "even");
        assert_eq!(physical((0.4, 0.4), 1.), Some((2, 2)), "at least 2×2");
        assert_eq!(physical((0., 500.), 2.), None, "empty keeps the last");
        // A 1080p source in a 1560×1000 area: smaller than the source, so the area.
        assert_eq!(target((1560, 1000), Some((1920, 1080)), 1.), (1560, 1000));
        // Retina fullscreen 2560×1500: capped to the source's larger ratio,
        // keeping the area's aspect.
        let (w, h) = target((2560, 1500), Some((1920, 1080)), 1.);
        assert_eq!((w, h), (1920, 1126));
        assert!((f64::from(w) / f64::from(h) - 2560. / 1500.).abs() < 0.01);
        // A square area keeps its shape, so `keepaspect` letterboxes correctly.
        assert_eq!(target((800, 800), Some((640, 360)), 1.), (640, 640));
        // Unknown source: 1920×1080 cap.
        assert_eq!(target((2560, 1500), None, 1.), (1920, 1126));
        // Zoom > 1 lets the target grow toward the area.
        assert_eq!(target((2560, 1500), Some((1280, 720)), 2.), (2560, 1500));
        assert_eq!(target((2560, 1500), Some((1280, 720)), 0.5), (1280, 750));
        // Absolute ceiling of 3840×2160 pixels.
        let (w, h) = target((7680, 4320), Some((7680, 4320)), 1.);
        assert!(u64::from(w) * u64::from(h) <= MAX_PIXELS);
        assert!(w % 2 == 0 && h % 2 == 0 && w >= 3830);
        let (w, h) = target((6000, 1000), Some((8000, 4000)), 4.);
        assert!(u64::from(w) * u64::from(h) <= MAX_PIXELS && w % 2 == 0 && h % 2 == 0);
    }

    #[test]
    fn drags_are_throttled_and_the_final_size_always_lands() {
        let start = Instant::now();
        let ms = |n| start + Duration::from_millis(n);
        let mut throttle = Throttle::default();
        assert_eq!(throttle.offer((100, 100), false, ms(0)), Some((100, 100)));
        assert_eq!(throttle.offer((100, 100), false, ms(5)), None, "unchanged");
        // Continuous changes within 50 ms are held.
        assert_eq!(throttle.offer((110, 100), false, ms(16)), None);
        assert_eq!(throttle.offer((120, 100), false, ms(32)), None);
        assert_eq!(throttle.poll(ms(48)), None);
        assert_eq!(throttle.poll(ms(50)), Some((120, 100)), "the newest one");
        assert_eq!(throttle.offer((130, 100), false, ms(60)), None);
        // Changes stop: the held final value lands within 50 ms.
        assert_eq!(throttle.poll(ms(100)), Some((130, 100)));
        assert_eq!(throttle.poll(ms(200)), None);
        // Fullscreen or a scale-factor change applies at once.
        assert_eq!(throttle.offer((140, 100), false, ms(110)), None);
        assert_eq!(
            throttle.offer((2560, 1600), true, ms(111)),
            Some((2560, 1600))
        );
        assert_eq!(throttle.poll(ms(400)), None, "the held value was replaced");
        // Back to the applied size before the turn: nothing to do.
        assert_eq!(throttle.offer((150, 100), false, ms(420)), Some((150, 100)));
        assert_eq!(throttle.offer((160, 100), false, ms(430)), None);
        assert_eq!(throttle.offer((150, 100), false, ms(440)), None);
        assert_eq!(throttle.poll(ms(500)), None);
    }

    #[test]
    fn fullscreen_and_scale_changes_skip_the_throttle() {
        let start = Instant::now();
        let ms = |n| start + Duration::from_millis(n);
        let mut sizing = Sizing::default();
        assert_eq!(
            sizing.observe((780., 500.), 2., false, ms(0)),
            Some((1560, 1000))
        );
        assert_eq!(
            sizing.observe((790., 500.), 2., false, ms(16)),
            None,
            "dragging"
        );
        assert_eq!(
            sizing.observe((1280., 800.), 2., true, ms(20)),
            Some((2560, 1600)),
            "fullscreen"
        );
        assert_eq!(
            sizing.observe((1280., 800.), 1., true, ms(30)),
            Some((1280, 800)),
            "moved to a 1x screen"
        );
        assert_eq!(
            sizing.observe((0., 0.), 1., true, ms(40)),
            None,
            "minimised"
        );
        assert_eq!(sizing.observe((1000., 800.), 1., true, ms(45)), None);
        assert_eq!(
            sizing.observe((0., 0.), 1., true, ms(90)),
            Some((1000, 800))
        );
    }

    #[test]
    fn a_changed_target_redraws_once_while_paused() {
        let shared = Shared::default();
        let mut redraw = Redraw::default();
        let (revision, request) = shared.get();
        assert_eq!(request.area, INITIAL_AREA);
        // A resize before the first frame does not draw a black picture.
        shared.set_area((1000, 600));
        assert!(!redraw.due(false, shared.get().0));
        assert!(redraw.due(true, shared.get().0), "first real frame");
        assert!(
            !redraw.due(false, shared.get().0),
            "paused, nothing changed"
        );
        shared.set_area((1000, 600));
        assert!(
            !redraw.due(false, shared.get().0),
            "same size is not a change"
        );
        shared.set_area((1200, 600));
        assert!(
            redraw.due(false, shared.get().0),
            "paused: redrawn at the new size"
        );
        assert!(!redraw.due(false, shared.get().0), "once");
        shared.set_zoom(2.);
        assert!(
            redraw.due(false, shared.get().0),
            "zoom changes the target too"
        );
        assert_eq!(shared.get().1.zoom, 2.);
        assert!(shared.get().0 > revision);
    }

    #[test]
    fn frames_wake_the_ui_once_per_delivery_and_keep_only_the_newest() {
        let slot = FrameSlot::new();
        let wakes = Arc::new(AtomicU64::new(0));
        let counter = wakes.clone();
        slot.set_waker(Box::new(move || {
            counter.fetch_add(1, Ordering::Relaxed);
        }));
        let frame = |sequence| Frame {
            buffer: slint::SharedPixelBuffer::new(2, 2),
            sequence,
        };
        slot.put(frame(1));
        slot.put(frame(2));
        slot.put(frame(3));
        assert_eq!(wakes.load(Ordering::Relaxed), 1, "one wake in flight");
        assert_eq!(slot.take().map(|f| f.sequence), Some(3), "newest only");
        assert!(slot.take().is_none());
        slot.put(frame(4));
        assert_eq!(
            wakes.load(Ordering::Relaxed),
            2,
            "taken: the next frame wakes"
        );
        slot.miss();
        assert!(slot.missed());
        assert_eq!(slot.peek(|f| f.sequence), Some(4));
        assert_eq!(slot.take().map(|f| f.sequence), Some(4));
        assert!(!slot.missed());
        assert_eq!(slot.counts(), (4, 2));
    }

    #[test]
    fn draws_present_a_frame_only_when_the_picture_changed() {
        let start = Instant::now();
        let ms = |n| start + Duration::from_millis(n);
        let presentation = Presentation::default();
        assert_eq!(presentation.presented_fps(), None, "not counting");
        presentation.start_counting();
        presentation.sample(ms(0));
        // 60 new pictures in a second, but only every other one is drawn
        // (vsync halving): 30 presented; plus 10 draws of the old picture.
        for n in 0..60u64 {
            presentation.picture_set(Duration::from_micros(200));
            if n % 2 == 1 {
                presentation.draw_begin(ms(n * 16));
                presentation.draw_end(ms(n * 16 + 8), false);
            }
        }
        for n in 0..10u64 {
            presentation.draw_begin(ms(970 + n));
            presentation.draw_end(ms(970 + n), false);
        }
        assert_eq!(presentation.counts(), (40, 30));
        presentation.sample(ms(500));
        assert_eq!(presentation.presented_fps(), None, "first second not over");
        presentation.sample(ms(1000));
        assert_eq!(presentation.presented_fps(), Some(30.));
        // The GPU renderer's copies present a frame without a set picture.
        presentation.draw_end(ms(1010), true);
        presentation.draw_end(ms(1020), false);
        assert_eq!(presentation.counts(), (42, 31));
        let (render, set) = presentation.take_timings();
        assert_eq!((render.count(), set.count()), (40, 60));
        assert_eq!(render.text(), "max 8.0 mean 6.0 ms");
        assert_eq!(presentation.take_timings().0.count(), 0, "taken");
        // Paused: nothing new is presented.
        presentation.sample(ms(2000));
        presentation.sample(ms(3000));
        assert_eq!(presentation.presented_fps(), Some(0.));
    }

    #[test]
    fn the_frame_log_line_has_every_column() {
        let start = Instant::now();
        let ms = |n| start + Duration::from_millis(n);
        let presentation = Presentation::default();
        let mut log = FrameLog::new();
        let counts = |produced, shown, draws, presented| LogCounts {
            produced,
            shown,
            draws,
            presented,
        };
        assert_eq!(
            log.line(counts(0, 0, 0, 0), &presentation, || Some(60.), None, ms(0)),
            None
        );
        log.tick_took(Duration::from_millis(2));
        log.tick_took(Duration::from_millis(4));
        presentation.draw_begin(ms(10));
        presentation.draw_end(ms(22), false);
        presentation.picture_set(Duration::from_micros(500));
        assert_eq!(
            log.line(
                counts(30, 15, 20, 15),
                &presentation,
                || Some(60.),
                None,
                ms(500)
            ),
            None
        );
        let line = log
            .line(
                counts(60, 30, 40, 28),
                &presentation,
                || Some(59.94),
                Some((1920, 1080)),
                ms(1000),
            )
            .expect("a line after a second");
        assert_eq!(
            line,
            "vestra frames: produced 60 shown 30 (60.0/30.0 fps) presented 28 redraws 12 refresh 59.9 Hz ticks 2 tick max 4.0 mean 3.0 ms render max 12.0 mean 12.0 ms set max 0.5 mean 0.5 ms target 1920x1080"
        );
        let line = log
            .line(
                counts(60, 30, 40, 28),
                &presentation,
                || None,
                None,
                ms(2000),
            )
            .unwrap();
        assert!(
            line.ends_with(
                "presented 0 redraws 0 refresh - ticks 0 tick - render - set - target -"
            ),
            "{line}"
        );
    }
}
