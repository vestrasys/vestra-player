//! C-SIZE (R-H2) and the render cadence: how large the software renderer
//! draws, when a new size is applied, and how a finished frame reaches the UI.
//! Pure decisions live here so they can be tested without libmpv or a window.
#![forbid(unsafe_code)]

use std::sync::{
    Arc, Mutex, PoisonError,
    atomic::{AtomicBool, AtomicU64, Ordering},
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
}
impl Default for Shared {
    fn default() -> Self {
        Self {
            request: Mutex::new(Request {
                area: INITIAL_AREA,
                zoom: 1.,
            }),
            revision: AtomicU64::new(0),
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
    /// The current request and its revision (bumped by every change).
    pub fn get(&self) -> (u64, Request) {
        let request = *self.request.lock().unwrap_or_else(PoisonError::into_inner);
        (self.revision.load(Ordering::Acquire), request)
    }
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

/// `VESTRA_FRAME_LOG=1`: one stderr line per second with produced and shown
/// frame counts (GUI acceptance diagnostics, not public API).
pub struct FrameLog {
    last: Option<(Instant, u64, u64)>,
}
impl FrameLog {
    pub fn from_env() -> Option<Self> {
        (std::env::var("VESTRA_FRAME_LOG").as_deref() == Ok("1")).then_some(Self { last: None })
    }
    pub fn tick(&mut self, counts: (u64, u64), size: Option<Size>, now: Instant) {
        let (produced, shown) = counts;
        match self.last {
            None => self.last = Some((now, produced, shown)),
            Some((at, p, s)) if now.duration_since(at) >= Duration::from_secs(1) => {
                let seconds = now.duration_since(at).as_secs_f64();
                let (dp, ds) = (produced - p, shown - s);
                let size = size.map_or("-".into(), |(w, h)| format!("{w}x{h}"));
                eprintln!(
                    "vestra frames: produced {dp} shown {ds} ({:.1}/{:.1} fps) target {size}",
                    dp as f64 / seconds,
                    ds as f64 / seconds
                );
                self.last = Some((now, produced, shown));
            }
            Some(_) => (),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
