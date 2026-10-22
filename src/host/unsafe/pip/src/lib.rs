//! macOS system picture-in-picture (`window.setPictureInPicture`, R-W5 /
//! R-H8), fed with the host's frames.
//!
//! **AVKit does not scale (macOS 26).** With a sample-buffer content source,
//! the PiP window (`PIPPanel`, a window of this process) shows the source
//! layer through a `CALayerHost` mirror *one point to one point*: AVKit's
//! view inside the panel keeps the source layer's size and is not fitted to
//! the panel. A layer smaller than the panel leaves white around the picture
//! (the prototype's white edges); a larger one shows only its lower-left
//! corner, enlarged relative to the whole (the first integration's crop).
//! Measured 2026-10-21 with the offline probe, this crate's `check` example
//! and the host on macOS 26.2; Apple feedback FB22411168 describes the same.
//! So while the PiP window is open the layer takes the panel's content size
//! (`Pip::place` reads it each tick, so resizing the PiP window follows),
//! and the frames must have the picture's own shape (no bars, the host's
//! job): the panel opens with the frame's shape and the layer, of the same
//! shape, then shows the whole frame filling it.
//!
//! **Where the display layer lives.** The PiP window opens from, and closes
//! back to, the layer's on-screen rectangle, and AVKit's render size is the
//! layer's video rectangle. The host's content view is drawn by OpenGL into
//! its backing layer's contents, and every sublayer is composited *above*
//! that, so the layer cannot go into it without covering the player's
//! overlays. It goes into a borderless child window ordered just below the
//! main window instead, framed on the video area's screen rectangle, covered
//! by the main window's opaque video area (PiP starts although it is fully
//! covered). A rebuilt main window only changes the parent, so PiP survives
//! it.
//!
//! **Frames.** `Pip::enqueue` copies RGB24 into a pooled, IOSurface-backed
//! BGRA `CVPixelBuffer`; `Pip::enqueue_surface` wraps an IOSurface the GPU
//! renderer drew (no copy). Either is wrapped in a `CMSampleBuffer` marked
//! "display immediately". The renderer's timebase only tells AVKit's buttons
//! and scrubber where playback is (`Pip::set_playback`).
//!
//! `cargo run -p vestra-pip --example check` checks all this without Vestra
//! (see the example for the host-like options).
//!
//! macOS 14+ (the layer's `sampleBufferRenderer`), main thread only. All
//! `unsafe` stays in this crate; the public API is safe.
#![cfg(target_os = "macos")]

use block2::DynBlock;
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{
    AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
};
use objc2_app_kit::{
    NSApplication, NSBackingStoreType, NSColor, NSWindow, NSWindowAnimationBehavior,
};
use objc2_app_kit::{NSWindowCollectionBehavior, NSWindowOrderingMode, NSWindowStyleMask};
use objc2_av_foundation::{AVQueuedSampleBufferRendering, AVSampleBufferDisplayLayer};
use objc2_av_kit::{AVPictureInPictureController, AVPictureInPictureControllerContentSource};
use objc2_av_kit::{
    AVPictureInPictureControllerDelegate, AVPictureInPictureSampleBufferPlaybackDelegate,
};
use objc2_core_foundation::CFType;
use objc2_core_foundation::{CFDictionary, CFMutableDictionary, CFNumber, CFRetained, CFString};
use objc2_core_graphics::CGColor;
use objc2_core_media::{CMSampleBuffer, CMSampleTimingInfo, CMTime, CMTimeFlags, CMTimeRange};
use objc2_core_media::{CMVideoDimensions, CMVideoFormatDescription};
use objc2_core_video::{CVPixelBuffer, CVPixelBufferLockFlags, CVPixelBufferPool};
use objc2_core_video::{CVPixelBufferCreateWithIOSurface, kCVPixelFormatType_32BGRA};
use objc2_foundation::{NSError, NSOperatingSystemVersion, NSPoint, NSProcessInfo, NSRect, NSSize};
pub use objc2_io_surface::IOSurfaceRef;
use objc2_quartz_core::{CAAutoresizingMask, CATransaction};
use std::cell::{Cell, RefCell};
use std::ptr::{NonNull, null, null_mut};

/// What happened in the PiP window since the last [`Pip::events`].
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Its play (`false`) or pause (`true`) button.
    Pause(bool),
    /// Its skip buttons: this many seconds (negative: back).
    Skip(f64),
    /// The PiP window is showing.
    Started,
    /// The PiP window closed. `restored`: through its "back to the app"
    /// button; otherwise its close button or [`Pip::stop`].
    Stopped { restored: bool },
    /// [`Pip::start`] was tried and AVKit refused, with its reason.
    Failed(String),
}

#[derive(Default)]
struct State {
    paused: Cell<bool>,
    /// The seekable length in seconds; `None`: live (no scrubber or skips).
    duration: Cell<Option<f64>>,
    /// AVKit asked to restore the app's interface (the "back" button).
    restored: Cell<bool>,
    events: RefCell<Vec<Event>>,
    render_size: Cell<(i32, i32)>,
}
impl State {
    fn push(&self, event: Event) {
        self.events.borrow_mut().push(event);
    }
}

/// `seconds` as a valid CMTime (millisecond timescale).
fn time(seconds: f64) -> CMTime {
    let value = (seconds * 1000.).round() as i64;
    CMTime { value, timescale: 1000, flags: CMTimeFlags::Valid, epoch: 0 }
}
fn infinite(flag: CMTimeFlags) -> CMTime {
    CMTime { value: 0, timescale: 1, flags: CMTimeFlags::Valid | flag, epoch: 0 }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; the class adds no
    // Drop-sensitive state beyond its Rust ivars and is used on the main thread.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "VestraPipDelegate"]
    #[ivars = State]
    struct Delegate;

    // SAFETY: NSObjectProtocol has no required methods beyond NSObject's.
    unsafe impl NSObjectProtocol for Delegate {}

    // SAFETY: every method below has the selector, argument and return types
    // that AVKit declares for AVPictureInPictureSampleBufferPlaybackDelegate.
    unsafe impl AVPictureInPictureSampleBufferPlaybackDelegate for Delegate {
        #[unsafe(method(pictureInPictureController:setPlaying:))]
        fn set_playing(&self, _: &AVPictureInPictureController, playing: bool) {
            self.ivars().push(Event::Pause(!playing));
        }
        #[unsafe(method(pictureInPictureControllerTimeRangeForPlayback:))]
        fn time_range(&self, _: &AVPictureInPictureController) -> CMTimeRange {
            match self.ivars().duration.get() {
                Some(duration) => CMTimeRange { start: time(0.), duration: time(duration) },
                // Live content: no scrubber, only play/pause.
                None => CMTimeRange {
                    start: infinite(CMTimeFlags::NegativeInfinity),
                    duration: infinite(CMTimeFlags::PositiveInfinity),
                },
            }
        }
        #[unsafe(method(pictureInPictureControllerIsPlaybackPaused:))]
        fn is_paused(&self, _: &AVPictureInPictureController) -> bool {
            self.ivars().paused.get()
        }
        #[unsafe(method(pictureInPictureController:didTransitionToRenderSize:))]
        fn render_size(&self, _: &AVPictureInPictureController, size: CMVideoDimensions) {
            self.ivars().render_size.set((size.width, size.height));
        }
        #[unsafe(method(pictureInPictureController:skipByInterval:completionHandler:))]
        fn skip(
            &self,
            _: &AVPictureInPictureController,
            interval: CMTime,
            done: &DynBlock<dyn Fn()>,
        ) {
            let (value, timescale, flags) = (interval.value, interval.timescale, interval.flags);
            if timescale > 0 && flags.contains(CMTimeFlags::Valid) {
                let seconds = value as f64 / f64::from(timescale);
                self.ivars().push(Event::Skip(seconds));
            }
            done.call(());
        }
    }

    // SAFETY: as above, for AVPictureInPictureControllerDelegate (all of its
    // methods are optional; these four have AVKit's declared types).
    unsafe impl AVPictureInPictureControllerDelegate for Delegate {
        #[unsafe(method(pictureInPictureControllerDidStartPictureInPicture:))]
        fn did_start(&self, _: &AVPictureInPictureController) {
            self.ivars().push(Event::Started);
        }
        #[unsafe(method(pictureInPictureController:failedToStartPictureInPictureWithError:))]
        fn failed(&self, _: &AVPictureInPictureController, error: &NSError) {
            self.ivars().push(Event::Failed(error.localizedDescription().to_string()));
        }
        #[unsafe(method(pictureInPictureController:restoreUserInterfaceForPictureInPictureStopWithCompletionHandler:))]
        fn restore(&self, _: &AVPictureInPictureController, done: &DynBlock<dyn Fn(Bool)>) {
            self.ivars().restored.set(true);
            done.call((Bool::YES,));
        }
        #[unsafe(method(pictureInPictureControllerDidStopPictureInPicture:))]
        fn did_stop(&self, _: &AVPictureInPictureController) {
            let restored = self.ivars().restored.replace(false);
            self.ivars().push(Event::Stopped { restored });
        }
    }
);

/// A pixel buffer pool and the frame size it makes buffers for.
type Pool = ((usize, usize), CFRetained<CVPixelBufferPool>);

/// System picture-in-picture for the host's main window. Main thread only
/// (`!Send`). Dropping it closes the PiP window.
pub struct Pip {
    /// The borderless child window that holds the display layer.
    window: Retained<NSWindow>,
    layer: Retained<AVSampleBufferDisplayLayer>,
    timebase: Retained<objc2_core_media::CMTimebase>,
    /// The rate last set on the timebase.
    rate: Cell<f64>,
    /// Pixel buffers for [`Pip::enqueue`], by frame size.
    pool: RefCell<Option<Pool>>,
    /// Frames enqueued so far.
    frames: Cell<u64>,
    /// Of those, IOSurfaces from the GPU renderer ([`Pip::enqueue_surface`]).
    surfaces: Cell<u64>,
    /// The size of the last frame enqueued (pixels).
    frame_size: Cell<(usize, usize)>,
    /// Created once the layer is on screen with real geometry ([`Pip::place`]).
    controller: RefCell<Option<Retained<AVPictureInPictureController>>>,
    delegate: Retained<Delegate>,
}

impl Pip {
    /// Whether this Mac offers picture-in-picture for sample buffers: AVKit
    /// says so and the system is macOS 14 or later.
    pub fn supported() -> bool {
        let version =
            NSOperatingSystemVersion { majorVersion: 14, minorVersion: 0, patchVersion: 0 };
        NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(version)
            // SAFETY: a class property without arguments or preconditions.
            && unsafe { AVPictureInPictureController::isPictureInPictureSupported() }
    }
    /// The display layer in its own (not yet shown) child window. `None` off
    /// the main thread or when the system lacks support.
    pub fn new() -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        if !Self::supported() {
            return None;
        }
        let delegate = Delegate::alloc(mtm).set_ivars(State::default());
        // SAFETY: NSObject's designated initializer on a freshly allocated
        // instance whose ivars are set.
        let delegate: Retained<Delegate> = unsafe { msg_send![super(delegate), init] };
        let rect = NSRect::new(NSPoint::new(0., 0.), NSSize::new(16., 9.));
        // SAFETY: NSWindow's designated initializer on a fresh allocation, on
        // the main thread. Not released when closed: `Pip` owns it through
        // `Retained` (the default `true` would release it a second time).
        let window = unsafe {
            let window = NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                rect,
                NSWindowStyleMask::Borderless,
                NSBackingStoreType::Buffered,
                false,
            );
            window.setReleasedWhenClosed(false);
            window
        };
        window.setIgnoresMouseEvents(true);
        window.setHasShadow(false);
        window.setExcludedFromWindowsMenu(true);
        window.setAnimationBehavior(NSWindowAnimationBehavior::None);
        window.setBackgroundColor(Some(&NSColor::blackColor()));
        window.setCollectionBehavior(
            NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        let view = window.contentView()?;
        view.setWantsLayer(true);
        // SAFETY: a plain constructor of an AVFoundation layer and getters of
        // its renderer and that renderer's timebase, on the main thread;
        // setting a timebase's time and rate has no other precondition.
        let (layer, timebase) = unsafe {
            let layer = AVSampleBufferDisplayLayer::new();
            let timebase = layer.sampleBufferRenderer().timebase();
            timebase.set_time(time(0.));
            timebase.set_rate(1.0);
            (layer, timebase)
        };
        layer.setFrame(view.bounds());
        let sizable =
            CAAutoresizingMask::LayerWidthSizable | CAAutoresizingMask::LayerHeightSizable;
        layer.setAutoresizingMask(sizable);
        layer.setBackgroundColor(Some(&CGColor::new_generic_gray(0., 1.)));
        // SAFETY: AVFoundation's constant gravity string, set on the main thread.
        unsafe {
            if let Some(aspect) = objc2_av_foundation::AVLayerVideoGravityResizeAspect {
                layer.setVideoGravity(aspect);
            }
        }
        view.layer()?.addSublayer(&layer);
        Some(Self {
            window,
            layer,
            timebase,
            rate: Cell::new(1.),
            pool: RefCell::new(None),
            frames: Cell::new(0),
            surfaces: Cell::new(0),
            frame_size: Cell::new((0, 0)),
            controller: RefCell::new(None),
            delegate,
        })
    }
    /// Put the layer on `area` of `parent`'s content (logical points,
    /// top-left origin): a child window just below `parent` on that screen
    /// rectangle. Call whenever the video area may have moved; a new `parent`
    /// (a rebuilt main window) takes the child window over. While the PiP
    /// window is open the layer has its content size instead of the area's
    /// (see the crate docs). `false` when the area is empty or not finite
    /// (nothing changed).
    pub fn place(&self, parent: &NSWindow, area: NSRect) -> bool {
        let content = parent.contentRectForFrameRect(parent.frame());
        let Some(frame) = screen_frame(content, area) else {
            return false;
        };
        let current = self.window.parentWindow();
        if current.as_deref().is_none_or(|current| !std::ptr::eq(current, parent)) {
            if let Some(old) = current {
                old.removeChildWindow(&self.window);
            }
            self.window.setFrame_display(frame, false);
            // SAFETY: both windows are live and on the main thread; the child
            // was just detached from any other parent and is never `parent`.
            unsafe { parent.addChildWindow_ordered(&self.window, NSWindowOrderingMode::Below) };
        } else if self.window.frame() != frame {
            self.window.setFrame_display(frame, false);
        }
        // While the PiP window is open the layer takes its size (AVKit
        // mirrors the layer one point to one point, see the crate docs);
        // otherwise it fills the video area.
        let target = match self.panel_size().filter(|_| self.active()) {
            Some(size) => Some(NSRect::new(NSPoint::new(0., 0.), size)),
            None => self.window.contentView().map(|view| view.bounds()),
        };
        if let Some(target) = target
            && self.layer.frame() != target
        {
            CATransaction::begin();
            CATransaction::setDisableActions(true);
            self.layer.setFrame(target);
            CATransaction::commit();
        }
        if self.controller.borrow().is_none() && self.window.isVisible() {
            *self.controller.borrow_mut() = Some(self.controller_now());
        }
        true
    }
    /// The controller, created only once the layer is in a visible window
    /// with real bounds (AVKit lays out its own views from that geometry).
    fn controller_now(&self) -> Retained<AVPictureInPictureController> {
        // SAFETY: initializers of AVKit objects on the main thread; the
        // content source and controller retain the layer, the delegate is a
        // weak property that `Pip` keeps alive as long as the controller.
        unsafe {
            let playback = ProtocolObject::from_ref(&*self.delegate);
            let source = AVPictureInPictureControllerContentSource::alloc();
            let source = AVPictureInPictureControllerContentSource::initWithSampleBufferDisplayLayer_playbackDelegate(
                source, &self.layer, playback,
            );
            let controller = AVPictureInPictureController::alloc();
            let controller =
                AVPictureInPictureController::initWithContentSource(controller, &source);
            controller.setDelegate(Some(ProtocolObject::from_ref(&*self.delegate)));
            controller
        }
    }
    /// Whether PiP could start now (a placed layer with a frame, per AVKit).
    pub fn possible(&self) -> bool {
        // SAFETY: a property getter on a live controller, on the main thread.
        self.controller.borrow().as_ref().is_some_and(|c| unsafe { c.isPictureInPicturePossible() })
    }
    /// Whether the PiP window is showing.
    pub fn active(&self) -> bool {
        // SAFETY: as above.
        self.controller.borrow().as_ref().is_some_and(|c| unsafe { c.isPictureInPictureActive() })
    }
    /// Ask AVKit to open the PiP window if it is possible, a frame is queued
    /// and the child window is on screen; `false` when it did not ask (call
    /// again later). The answer comes as [`Event::Started`] or [`Event::Failed`].
    pub fn start(&self) -> bool {
        if !(self.possible() && self.frames.get() > 0 && self.window.isVisible()) {
            return false;
        }
        if let Some(controller) = self.controller.borrow().as_ref() {
            // SAFETY: methods on a live controller, on the main thread.
            unsafe {
                controller.startPictureInPicture();
                controller.invalidatePlaybackState();
            }
        }
        true
    }
    /// Close the PiP window (answered by [`Event::Stopped`] when it was open).
    pub fn stop(&self) {
        if let Some(controller) = self.controller.borrow().as_ref() {
            // SAFETY: as above.
            unsafe { controller.stopPictureInPicture() }
        }
    }
    /// Report playback to the PiP window's buttons and scrubber: paused or
    /// not, the position and seekable length in seconds (`None`: live) and
    /// the rate. Cheap to call every tick.
    pub fn set_playback(
        &self,
        paused: bool,
        position: Option<f64>,
        duration: Option<f64>,
        rate: f64,
    ) {
        let state = self.delegate.ivars();
        let duration = duration.filter(|d| d.is_finite() && *d > 0.);
        let mut changed = state.paused.replace(paused) != paused;
        changed |= state.duration.replace(duration) != duration;
        let rate = match paused {
            true => 0.,
            false if rate.is_finite() && rate > 0. => rate,
            false => 1.,
        };
        // SAFETY: getters and setters on the layer renderer's own timebase and
        // a live controller, on the main thread.
        unsafe {
            if let Some(position) = position.filter(|p| p.is_finite() && duration.is_some()) {
                let now = self.timebase.time().seconds();
                if !now.is_finite() || (now - position).abs() > 0.5 {
                    self.timebase.set_time(time(position));
                }
            }
            if self.rate.replace(rate) != rate {
                self.timebase.set_rate(rate);
            }
            if changed && let Some(controller) = self.controller.borrow().as_ref() {
                controller.invalidatePlaybackState();
            }
        }
    }
    /// What happened in the PiP window since the last call.
    pub fn events(&self) -> Vec<Event> {
        std::mem::take(&mut *self.delegate.ivars().events.borrow_mut())
    }
    /// One line of geometry for diagnostics (`VESTRA_PIP_LOG=1`): the
    /// parent's content rectangle, the child window's frame and backing
    /// scale, the layer's frame in its superlayer's bounds, the last frame's
    /// size and AVKit's render size. Screen rectangles are Cocoa points
    /// (lower-left origin).
    pub fn describe(&self, parent: &NSWindow) -> String {
        let r = |r: NSRect| {
            let (o, s) = (r.origin, r.size);
            format!("({:.1},{:.1} {:.1}x{:.1})", o.x, o.y, s.width, s.height)
        };
        let content = parent.contentRectForFrameRect(parent.frame());
        let superlayer = self.layer.superlayer().map(|l| r(l.bounds())).unwrap_or_default();
        format!(
            "parent content {} scale {} | child frame {} scale {} visible {} | layer {} in {} | frame {:?} ({} frames, {} IOSurfaces) | renderSize {:?} | PiP panel {:?} | active {}",
            r(content),
            parent.backingScaleFactor(),
            r(self.window.frame()),
            self.window.backingScaleFactor(),
            self.window.isVisible(),
            r(self.layer.frame()),
            superlayer,
            self.frame_size.get(),
            self.frames.get(),
            self.surfaces.get(),
            self.render_size(),
            self.panel_size().map(|s| (s.width, s.height)),
            self.active(),
        )
    }
    /// The content size of the system PiP window (AVKit's `PIPPanel`, a
    /// window of this application), while it is open.
    fn panel_size(&self) -> Option<NSSize> {
        let mtm = MainThreadMarker::new()?;
        let windows = NSApplication::sharedApplication(mtm).windows();
        let panel = windows.iter().find(|w| w.class().name().to_bytes().ends_with(b"PIPPanel"))?;
        let size = panel.contentView()?.bounds().size;
        (size.width >= 1. && size.height >= 1.).then_some(size)
    }
    /// The PiP window's render size in pixels, once AVKit reported it.
    pub fn render_size(&self) -> (i32, i32) {
        self.delegate.ivars().render_size.get()
    }
    /// Show one frame: `rgb` holds `height` rows of `width` RGB24 pixels
    /// (copied into a pooled BGRA pixel buffer).
    pub fn enqueue(&self, width: usize, height: usize, rgb: &[u8]) -> Result<(), String> {
        let pixels = self.pixel_buffer(width, height)?;
        fill(&pixels, width, height, rgb)?;
        self.frame_size.set((width, height));
        self.enqueue_pixels(&pixels)
    }
    /// Show one frame drawn into `surface` (BGRA), without copying it: the
    /// zero-copy path for the GPU renderer's IOSurfaces.
    pub fn enqueue_surface(&self, surface: &IOSurfaceRef) -> Result<(), String> {
        let pixels = wrap(surface)?;
        let size = (
            objc2_core_video::CVPixelBufferGetWidth(&pixels),
            objc2_core_video::CVPixelBufferGetHeight(&pixels),
        );
        self.frame_size.set(size);
        self.surfaces.set(self.surfaces.get() + 1);
        self.enqueue_pixels(&pixels)
    }
    fn enqueue_pixels(&self, pixels: &CVPixelBuffer) -> Result<(), String> {
        let sample = sample_buffer(pixels)?;
        // SAFETY: a property getter on a live layer, on the main thread.
        let renderer = unsafe { self.layer.sampleBufferRenderer() };
        // SAFETY: the renderer is live; `sample` is a complete, ready sample
        // buffer. A failed renderer is flushed so it accepts frames again.
        unsafe {
            if renderer.status() == objc2_av_foundation::AVQueuedSampleBufferRenderingStatus::Failed
            {
                renderer.flush();
            }
            renderer.enqueueSampleBuffer(&sample);
        }
        self.frames.set(self.frames.get() + 1);
        Ok(())
    }
    /// A buffer from the pool for this frame size (a new pool when it changed).
    fn pixel_buffer(
        &self,
        width: usize,
        height: usize,
    ) -> Result<CFRetained<CVPixelBuffer>, String> {
        let mut pool = self.pool.borrow_mut();
        if pool.as_ref().is_none_or(|(size, _)| *size != (width, height)) {
            *pool = Some(((width, height), new_pool(width, height)?));
        }
        let (_, pool) = pool.as_ref().ok_or("no pixel buffer pool")?;
        let mut out: *mut CVPixelBuffer = null_mut();
        // SAFETY: the pool is live; `out` is a live local that receives a +1
        // reference on success, taken over just below.
        let status =
            unsafe { CVPixelBufferPool::create_pixel_buffer(None, pool, NonNull::from(&mut out)) };
        match NonNull::new(out) {
            // SAFETY: on success `out` is a +1 reference that we now own.
            Some(out) if status == 0 => Ok(unsafe { CFRetained::from_raw(out) }),
            _ => Err(format!("CVPixelBufferPoolCreatePixelBuffer failed ({status})")),
        }
    }
}
impl Drop for Pip {
    fn drop(&mut self) {
        self.stop();
        if let Some(parent) = self.window.parentWindow() {
            parent.removeChildWindow(&self.window);
        }
        self.window.orderOut(None);
        self.layer.removeFromSuperlayer();
    }
}

/// `area` (top-left origin, inside a content rectangle) on screen, given the
/// content rectangle in Cocoa screen coordinates (lower-left origin). `None`
/// when either is not finite or the area is empty.
fn screen_frame(content: NSRect, area: NSRect) -> Option<NSRect> {
    let numbers = [content.origin.x, content.origin.y, content.size.width, content.size.height];
    let numbers = numbers.into_iter().chain([
        area.origin.x,
        area.origin.y,
        area.size.width,
        area.size.height,
    ]);
    if !numbers.into_iter().all(f64::is_finite) || area.size.width < 1. || area.size.height < 1. {
        return None;
    }
    let y = content.origin.y + content.size.height - area.origin.y - area.size.height;
    Some(NSRect::new(NSPoint::new(content.origin.x + area.origin.x, y), area.size))
}

/// A pool of IOSurface-backed BGRA pixel buffers of one size.
fn new_pool(width: usize, height: usize) -> Result<CFRetained<CVPixelBufferPool>, String> {
    let side = |n: usize| i32::try_from(n).ok().filter(|n| *n > 0);
    let (Some(w), Some(h)) = (side(width), side(height)) else {
        return Err("frame size out of range".into());
    };
    let (w, h) = (CFNumber::new_i32(w), CFNumber::new_i32(h));
    let format = CFNumber::new_i32(kCVPixelFormatType_32BGRA as i32);
    let empty = CFDictionary::<CFString, CFString>::from_slices(&[], &[]);
    // SAFETY: CoreVideo's constant attribute keys (immutable statics).
    let keys = unsafe {
        use objc2_core_video::*;
        [
            kCVPixelBufferWidthKey,
            kCVPixelBufferHeightKey,
            kCVPixelBufferPixelFormatTypeKey,
            kCVPixelBufferIOSurfacePropertiesKey,
        ]
    };
    let values: [&CFType; 4] = [&w, &h, &format, &empty];
    let attributes = CFDictionary::<CFString, CFType>::from_slices(&keys, &values);
    let mut out: *mut CVPixelBufferPool = null_mut();
    // SAFETY: the attribute dictionary lives across the call; `out` is a live
    // local that receives a +1 reference on success, taken over below.
    let status = unsafe {
        CVPixelBufferPool::create(None, None, Some(attributes.as_opaque()), NonNull::from(&mut out))
    };
    match NonNull::new(out) {
        // SAFETY: on success `out` is a +1 reference that we now own.
        Some(out) if status == 0 => Ok(unsafe { CFRetained::from_raw(out) }),
        _ => Err(format!("CVPixelBufferPoolCreate failed ({status})")),
    }
}

/// Copy `rgb` (`height` rows of `width` RGB24 pixels) into `pixels` as BGRA.
fn fill(pixels: &CVPixelBuffer, width: usize, height: usize, rgb: &[u8]) -> Result<(), String> {
    if width == 0 || height == 0 || rgb.len() < width * height * 3 {
        return Err("frame size does not match its bytes".into());
    }
    let lock = CVPixelBufferLockFlags(0);
    // SAFETY: while locked, the base address points at the buffer's height
    // rows of bytes_per_row bytes; the size is checked against the frame
    // first, so every write stays inside, and the buffer is unlocked before
    // it is used.
    unsafe {
        if objc2_core_video::CVPixelBufferLockBaseAddress(pixels, lock) != 0 {
            return Err("CVPixelBufferLockBaseAddress failed".into());
        }
        let stride = objc2_core_video::CVPixelBufferGetBytesPerRow(pixels);
        let base = objc2_core_video::CVPixelBufferGetBaseAddress(pixels).cast::<u8>();
        let fits = objc2_core_video::CVPixelBufferGetWidth(pixels) == width
            && objc2_core_video::CVPixelBufferGetHeight(pixels) == height
            && stride >= width * 4
            && !base.is_null();
        if fits {
            for (y, row) in rgb.chunks_exact(width * 3).take(height).enumerate() {
                let line = std::slice::from_raw_parts_mut(base.add(y * stride), width * 4);
                for (bgra, rgb) in
                    line.as_chunks_mut::<4>().0.iter_mut().zip(row.as_chunks::<3>().0)
                {
                    *bgra = [rgb[2], rgb[1], rgb[0], 255];
                }
            }
        }
        objc2_core_video::CVPixelBufferUnlockBaseAddress(pixels, lock);
        if !fits {
            return Err("pixel buffer does not fit the frame".into());
        }
    }
    Ok(())
}

/// A pixel buffer over `surface`'s memory (no copy).
fn wrap(surface: &IOSurfaceRef) -> Result<CFRetained<CVPixelBuffer>, String> {
    let mut out: *mut CVPixelBuffer = null_mut();
    // SAFETY: `surface` is live; no attributes; `out` is a live local that
    // receives a +1 reference on success, taken over just below.
    let status =
        unsafe { CVPixelBufferCreateWithIOSurface(None, surface, None, NonNull::from(&mut out)) };
    match NonNull::new(out) {
        // SAFETY: on success `out` is a +1 reference that we now own.
        Some(out) if status == 0 => Ok(unsafe { CFRetained::from_raw(out) }),
        _ => Err(format!("CVPixelBufferCreateWithIOSurface failed ({status})")),
    }
}

/// A ready sample buffer for `pixels`, marked to display immediately.
fn sample_buffer(pixels: &CVPixelBuffer) -> Result<CFRetained<CMSampleBuffer>, String> {
    let mut format: *const CMVideoFormatDescription = null();
    let mut sample: *mut CMSampleBuffer = null_mut();
    let invalid = CMTime { value: 0, timescale: 0, flags: CMTimeFlags(0), epoch: 0 };
    let mut timing = CMSampleTimingInfo {
        duration: invalid,
        presentationTimeStamp: invalid,
        decodeTimeStamp: invalid,
    };
    // SAFETY: the out-pointers are live locals that receive +1 references on
    // success (taken over below, each exactly once); `timing` outlives the
    // call. The attachments array of a fresh single-sample buffer holds one
    // mutable dictionary, which we set one CFBoolean value in.
    unsafe {
        let status = objc2_core_media::CMVideoFormatDescriptionCreateForImageBuffer(
            None,
            pixels,
            NonNull::from(&mut format),
        );
        let format = match NonNull::new(format.cast_mut()) {
            Some(format) if status == 0 => CFRetained::from_raw(format),
            _ => {
                return Err(format!(
                    "CMVideoFormatDescriptionCreateForImageBuffer failed ({status})"
                ));
            }
        };
        let status = CMSampleBuffer::create_ready_with_image_buffer(
            None,
            pixels,
            &format,
            NonNull::from(&mut timing),
            NonNull::from(&mut sample),
        );
        let sample = match NonNull::new(sample) {
            Some(sample) if status == 0 => CFRetained::from_raw(sample),
            _ => return Err(format!("CMSampleBufferCreateReadyWithImageBuffer failed ({status})")),
        };
        let attachments = sample.sample_attachments_array(true).ok_or("no sample attachments")?;
        let first = attachments.value_at_index(0).cast::<CFMutableDictionary>().cast_mut();
        let first = first.as_ref().ok_or("no sample attachment dictionary")?;
        let key = objc2_core_media::kCMSampleAttachmentKey_DisplayImmediately;
        let yes = objc2_core_foundation::kCFBooleanTrue.ok_or("no kCFBooleanTrue")?;
        CFMutableDictionary::set_value(
            Some(first),
            (key as *const CFString).cast(),
            (yes as *const _ as *const ()).cast(),
        );
        Ok(sample)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test-only: the first `n` bytes of a pixel buffer, under a read lock.
    fn first_bytes(pixels: &CVPixelBuffer, n: usize) -> Vec<u8> {
        // SAFETY: a read-only lock; the buffer holds at least one row of
        // `n` ≤ bytes_per_row bytes while locked.
        unsafe {
            let lock = CVPixelBufferLockFlags(1);
            objc2_core_video::CVPixelBufferLockBaseAddress(pixels, lock);
            let base = objc2_core_video::CVPixelBufferGetBaseAddress(pixels).cast::<u8>();
            let first = std::slice::from_raw_parts(base, n).to_vec();
            objc2_core_video::CVPixelBufferUnlockBaseAddress(pixels, lock);
            first
        }
    }

    #[test]
    fn rgb_frames_become_pooled_display_immediately_sample_buffers() {
        let rgb: Vec<u8> = (0..4 * 2 * 3).map(|i| i as u8).collect();
        let pool = new_pool(4, 2).expect("pool");
        let mut out: *mut CVPixelBuffer = null_mut();
        // SAFETY: as in `Pip::pixel_buffer`.
        let pixels = unsafe {
            assert_eq!(
                CVPixelBufferPool::create_pixel_buffer(None, &pool, NonNull::from(&mut out)),
                0
            );
            CFRetained::from_raw(NonNull::new(out).expect("pixel buffer"))
        };
        fill(&pixels, 4, 2, &rgb).expect("filled");
        assert_eq!(first_bytes(&pixels, 8), [2, 1, 0, 255, 5, 4, 3, 255]);
        assert!(sample_buffer(&pixels).is_ok());
        // A frame of another size, or too few bytes, is refused.
        assert!(fill(&pixels, 2, 2, &rgb).is_err());
        assert!(fill(&pixels, 4, 2, &rgb[..5]).is_err());
        assert!(new_pool(0, 2).is_err());
        // The zero-copy path: the same memory, wrapped again.
        let surface = objc2_core_video::CVPixelBufferGetIOSurface(Some(&pixels)).expect("surface");
        let again = wrap(&surface).expect("wrapped");
        assert_eq!(
            objc2_core_video::CVPixelBufferGetBaseAddress(&again),
            objc2_core_video::CVPixelBufferGetBaseAddress(&pixels)
        );
        assert!(sample_buffer(&again).is_ok());
    }

    #[test]
    fn the_video_area_maps_to_a_lower_left_screen_rectangle() {
        let rect = |x, y, w, h| NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
        // Content 800×600 at (100, 50); video 500×400 at the top-left corner.
        let content = rect(100., 50., 800., 600.);
        assert_eq!(
            screen_frame(content, rect(0., 0., 500., 400.)),
            Some(rect(100., 250., 500., 400.))
        );
        assert_eq!(
            screen_frame(content, rect(10., 20., 30., 40.)),
            Some(rect(110., 590., 30., 40.))
        );
        assert_eq!(screen_frame(content, rect(0., 0., 0., 400.)), None);
        assert_eq!(screen_frame(content, rect(f64::NAN, 0., 10., 10.)), None);
        assert_eq!(screen_frame(rect(0., f64::INFINITY, 1., 1.), rect(0., 0., 10., 10.)), None);
    }

    #[test]
    fn time_ranges_and_times() {
        // CMTime is packed: copy fields out before comparing.
        let t = time(1.5);
        let (value, timescale, flags) = (t.value, t.timescale, t.flags);
        assert_eq!((value, timescale), (1500, 1000));
        assert!(flags.contains(CMTimeFlags::Valid));
        let flags = infinite(CMTimeFlags::PositiveInfinity).flags;
        assert!(flags.contains(CMTimeFlags::Valid | CMTimeFlags::PositiveInfinity));
    }
}
