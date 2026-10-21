//! Prototype (vestra-host feature `pip`): macOS system picture-in-picture fed
//! with the host's RGB frames.
//!
//! An `AVSampleBufferDisplayLayer` receives each frame. AVKit only offers PiP
//! for a layer inside a window, and lays out its own host view from that
//! layer's geometry (a bare 1×1 sublayer of the content view crashed it with
//! "x is NaN"), so the layer backs its own layer-hosting `NSView`, as large as
//! the content view and placed behind it in the window's frame view: real
//! geometry, but hidden by the opaque OpenGL content. The layer gets each frame
//! as a BGRA `CVPixelBuffer` wrapped in a `CMSampleBuffer` marked "display
//! immediately", so no timebase is needed. An `AVPictureInPictureController`
//! with a sample-buffer content source moves that layer's picture into the
//! system PiP window; its playback delegate reports play/pause and skip
//! requests back as [`Request`]s.
//!
//! macOS 14+ (the layer's `sampleBufferRenderer`), main thread only. All
//! `unsafe` stays in this crate; the public API is safe.
#![cfg(target_os = "macos")]

use block2::DynBlock;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{
    AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSView, NSWindow, NSWindowOrderingMode};
use objc2_av_foundation::{AVQueuedSampleBufferRendering, AVSampleBufferDisplayLayer};
use objc2_av_kit::AVPictureInPictureSampleBufferPlaybackDelegate;
use objc2_av_kit::{AVPictureInPictureController, AVPictureInPictureControllerContentSource};
use objc2_core_foundation::{CFDictionary, CFMutableDictionary, CFRetained, CFString};
use objc2_core_media::{CMSampleBuffer, CMSampleTimingInfo, CMTime, CMTimeFlags, CMTimeRange};
use objc2_core_media::{CMVideoDimensions, CMVideoFormatDescription};
use objc2_core_video::{CVPixelBuffer, CVPixelBufferLockFlags, kCVPixelFormatType_32BGRA};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use std::cell::{Cell, RefCell};
use std::ptr::{NonNull, null, null_mut};

/// What the user asked for in the PiP window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Request {
    /// Play (`false`) or pause (`true`).
    Pause(bool),
    /// Skip by this many seconds (negative: back).
    Skip(f64),
}

#[derive(Default)]
struct State {
    paused: Cell<bool>,
    requests: RefCell<Vec<Request>>,
    render_size: Cell<(i32, i32)>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; the class adds no
    // Drop-sensitive state beyond its Rust ivars and is used on the main thread.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "VestraPipPlaybackDelegate"]
    #[ivars = State]
    struct Delegate;

    // SAFETY: NSObjectProtocol has no required methods beyond NSObject's.
    unsafe impl NSObjectProtocol for Delegate {}

    // SAFETY: every method below has the selector, argument and return types
    // that AVKit declares for AVPictureInPictureSampleBufferPlaybackDelegate.
    unsafe impl AVPictureInPictureSampleBufferPlaybackDelegate for Delegate {
        #[unsafe(method(pictureInPictureController:setPlaying:))]
        fn set_playing(&self, _: &AVPictureInPictureController, playing: bool) {
            self.ivars()
                .requests
                .borrow_mut()
                .push(Request::Pause(!playing));
        }
        #[unsafe(method(pictureInPictureControllerTimeRangeForPlayback:))]
        fn time_range(&self, _: &AVPictureInPictureController) -> CMTimeRange {
            // Live content: no scrubber, only play/pause and skip.
            let flags = CMTimeFlags::Valid;
            let start = CMTime {
                value: 0,
                timescale: 1,
                flags: flags | CMTimeFlags::NegativeInfinity,
                epoch: 0,
            };
            let duration = CMTime {
                value: 0,
                timescale: 1,
                flags: flags | CMTimeFlags::PositiveInfinity,
                epoch: 0,
            };
            CMTimeRange { start, duration }
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
            if interval.timescale > 0 {
                let seconds = interval.value as f64 / f64::from(interval.timescale);
                self.ivars()
                    .requests
                    .borrow_mut()
                    .push(Request::Skip(seconds));
            }
            done.call(());
        }
    }
);

/// System picture-in-picture for one window. Main thread only (`!Send`).
pub struct Pip {
    host: Retained<NSView>,
    layer: Retained<AVSampleBufferDisplayLayer>,
    /// Frames enqueued so far (PiP starts only after a few).
    frames: Cell<u64>,
    controller: Retained<AVPictureInPictureController>,
    delegate: Retained<Delegate>,
}

impl Pip {
    /// Whether this Mac offers picture-in-picture at all.
    pub fn supported() -> bool {
        // SAFETY: a class property without arguments or preconditions.
        unsafe { AVPictureInPictureController::isPictureInPictureSupported() }
    }
    /// Prepare PiP for `window`: a hidden layer-hosting view behind its
    /// content view, sized and resized with it. `overlay` (diagnosis) puts a
    /// visible 160×90 view in the content view's corner instead.
    pub fn new(window: &NSWindow, overlay: bool) -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        let content = window.contentView()?;
        // SAFETY: a plain getter on the main thread; the window retains its
        // frame view, and the Retained keeps it for this function.
        let frame_view = unsafe { content.superview() }?;
        let delegate = Delegate::alloc(mtm).set_ivars(State::default());
        // SAFETY: NSObject's designated initializer on a freshly allocated
        // instance whose ivars are set.
        let delegate: Retained<Delegate> = unsafe { msg_send![super(delegate), init] };
        // SAFETY: a plain constructor of an AVFoundation layer on the main thread.
        let layer = unsafe { AVSampleBufferDisplayLayer::new() };
        let corner = NSRect::new(NSPoint::new(0., 0.), NSSize::new(160., 90.));
        let host = NSView::initWithFrame(
            NSView::alloc(mtm),
            if overlay { corner } else { content.frame() },
        );
        let sizable = NSAutoresizingMaskOptions::ViewWidthSizable;
        if !overlay {
            host.setAutoresizingMask(sizable | NSAutoresizingMaskOptions::ViewHeightSizable);
        }
        // setLayer before setWantsLayer: a layer-hosting view that AppKit
        // sizes but never draws into.
        host.setLayer(Some(&layer));
        host.setWantsLayer(true);
        if overlay {
            content.addSubview(&host);
        } else {
            frame_view.addSubview_positioned_relativeTo(
                &host,
                NSWindowOrderingMode::Below,
                Some(&content),
            );
        }
        // SAFETY: initializers of AVKit objects on the main thread, created
        // only now that the layer is in the window's layer tree with real
        // bounds; the content source and controller retain the layer and the
        // delegate, and `Pip` keeps them alive as well.
        let controller = unsafe {
            let protocol = ProtocolObject::from_ref(&*delegate);
            let source = AVPictureInPictureControllerContentSource::alloc();
            let source = AVPictureInPictureControllerContentSource::initWithSampleBufferDisplayLayer_playbackDelegate(
                source, &layer, protocol,
            );
            let controller = AVPictureInPictureController::alloc();
            AVPictureInPictureController::initWithContentSource(controller, &source)
        };
        Some(Self {
            host,
            layer,
            frames: Cell::new(0),
            controller,
            delegate,
        })
    }
    /// Whether PiP could start now (the system needs a frame and a window).
    pub fn possible(&self) -> bool {
        // SAFETY: a property getter on a live controller, on the main thread.
        unsafe { self.controller.isPictureInPicturePossible() }
    }
    /// Whether the PiP window is showing.
    pub fn active(&self) -> bool {
        // SAFETY: as above.
        unsafe { self.controller.isPictureInPictureActive() }
    }
    /// Start PiP if it is possible, a few frames are queued, the window is
    /// visible and the layer's geometry is finite and non-empty; `false` when
    /// it did not try (call again later).
    pub fn start(&self) -> bool {
        let frame = self.host.frame();
        let numbers = [
            frame.origin.x,
            frame.origin.y,
            frame.size.width,
            frame.size.height,
        ];
        let geometry = numbers.iter().all(|n| n.is_finite()) && frame.size.width >= 1.;
        let visible = self.host.window().is_some_and(|window| window.isVisible());
        if !(self.possible() && geometry && frame.size.height >= 1. && visible)
            || self.frames.get() < 3
        {
            return false;
        }
        // SAFETY: a method on a live controller, on the main thread.
        unsafe { self.controller.startPictureInPicture() };
        true
    }
    pub fn stop(&self) {
        // SAFETY: as above.
        unsafe { self.controller.stopPictureInPicture() }
    }
    /// Report the player's pause state to the PiP buttons.
    pub fn set_paused(&self, paused: bool) {
        if self.delegate.ivars().paused.replace(paused) != paused {
            // SAFETY: as above.
            unsafe { self.controller.invalidatePlaybackState() }
        }
    }
    /// Requests from the PiP window since the last call.
    pub fn requests(&self) -> Vec<Request> {
        std::mem::take(&mut *self.delegate.ivars().requests.borrow_mut())
    }
    /// The PiP window's render size in pixels, once known.
    pub fn render_size(&self) -> (i32, i32) {
        self.delegate.ivars().render_size.get()
    }
    /// Show one frame: `rgb` holds `height` rows of `width` RGB24 pixels.
    pub fn enqueue(&self, width: usize, height: usize, rgb: &[u8]) -> Result<(), String> {
        let pixels = pixel_buffer(width, height, rgb)?;
        let sample = sample_buffer(&pixels)?;
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
}
impl Drop for Pip {
    fn drop(&mut self) {
        self.stop();
        self.host.removeFromSuperview();
    }
}

/// An IOSurface-backed BGRA pixel buffer holding `rgb`.
fn pixel_buffer(
    width: usize,
    height: usize,
    rgb: &[u8],
) -> Result<CFRetained<CVPixelBuffer>, String> {
    if width == 0 || height == 0 || rgb.len() < width * height * 3 {
        return Err("frame size does not match its bytes".into());
    }
    let empty = CFDictionary::<CFString, CFString>::from_slices(&[], &[]);
    let mut out: *mut CVPixelBuffer = null_mut();
    // SAFETY: the key is CoreVideo's constant CFString; the attribute
    // dictionary (IOSurface properties: an empty dictionary) lives across the
    // call; `out` is a live local that receives a +1 reference on success.
    let status = unsafe {
        let key = objc2_core_video::kCVPixelBufferIOSurfacePropertiesKey;
        let attributes = CFDictionary::from_slices(&[key], &[&*empty]);
        let format = kCVPixelFormatType_32BGRA;
        let attributes = Some(attributes.as_opaque());
        objc2_core_video::CVPixelBufferCreate(
            None,
            width,
            height,
            format,
            attributes,
            NonNull::from(&mut out),
        )
    };
    // SAFETY: on success `out` is a +1 reference that we now own.
    let buffer = match NonNull::new(out) {
        Some(out) if status == 0 => unsafe { CFRetained::from_raw(out) },
        _ => return Err(format!("CVPixelBufferCreate failed ({status})")),
    };
    let lock = CVPixelBufferLockFlags(0);
    // SAFETY: while locked, the base address points at `height` rows of
    // bytes_per_row bytes (≥ width × 4, BGRA as requested); every write stays
    // inside that, and the buffer is unlocked before it is used.
    unsafe {
        if objc2_core_video::CVPixelBufferLockBaseAddress(&buffer, lock) != 0 {
            return Err("CVPixelBufferLockBaseAddress failed".into());
        }
        let stride = objc2_core_video::CVPixelBufferGetBytesPerRow(&buffer);
        let base = objc2_core_video::CVPixelBufferGetBaseAddress(&buffer).cast::<u8>();
        for (y, row) in rgb.chunks_exact(width * 3).take(height).enumerate() {
            let line = std::slice::from_raw_parts_mut(base.add(y * stride), width * 4);
            for (bgra, rgb) in line
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(row.as_chunks::<3>().0)
            {
                *bgra = [rgb[2], rgb[1], rgb[0], 255];
            }
        }
        objc2_core_video::CVPixelBufferUnlockBaseAddress(&buffer, lock);
    }
    Ok(buffer)
}

/// A ready sample buffer for `pixels`, marked to display immediately.
fn sample_buffer(pixels: &CVPixelBuffer) -> Result<CFRetained<CMSampleBuffer>, String> {
    let mut format: *const CMVideoFormatDescription = null();
    let mut sample: *mut CMSampleBuffer = null_mut();
    let invalid = CMTime {
        value: 0,
        timescale: 0,
        flags: CMTimeFlags(0),
        epoch: 0,
    };
    let mut timing = CMSampleTimingInfo {
        duration: invalid,
        presentationTimeStamp: invalid,
        decodeTimeStamp: invalid,
    };
    // SAFETY: the out-pointers are live locals that receive +1 references on
    // success (taken over below, each exactly once); `timing` outlives the call.
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
            _ => {
                return Err(format!(
                    "CMSampleBufferCreateReadyWithImageBuffer failed ({status})"
                ));
            }
        };
        // The attachments array has one mutable dictionary per sample.
        let attachments = sample
            .sample_attachments_array(true)
            .ok_or("no sample attachments")?;
        let first = attachments
            .value_at_index(0)
            .cast::<CFMutableDictionary>()
            .cast_mut();
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

    #[test]
    fn frames_become_display_immediately_sample_buffers() {
        let rgb: Vec<u8> = (0..4 * 2 * 3).map(|i| i as u8).collect();
        let pixels = pixel_buffer(4, 2, &rgb).expect("pixel buffer");
        // SAFETY: test-only read of the buffer just written, under its lock.
        let first = unsafe {
            let lock = CVPixelBufferLockFlags(1); // read-only
            objc2_core_video::CVPixelBufferLockBaseAddress(&pixels, lock);
            let base = objc2_core_video::CVPixelBufferGetBaseAddress(&pixels).cast::<u8>();
            let first = std::slice::from_raw_parts(base, 4).to_vec();
            objc2_core_video::CVPixelBufferUnlockBaseAddress(&pixels, lock);
            first
        };
        assert_eq!(first, [2, 1, 0, 255]);
        assert!(sample_buffer(&pixels).is_ok());
        assert!(pixel_buffer(4, 2, &rgb[..5]).is_err());
    }
}
