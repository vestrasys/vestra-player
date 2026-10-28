//! libmpv draws with OpenGL and Slint shows the result as a texture (the
//! host's GPU rendering, SDK site, Concepts: render options; the host reaches it only through its
//! macOS platform module).
//!
//! libmpv renders on its own thread, in a private offscreen CGL context, into a
//! ring of IOSurfaces. Each Slint window binds the newest surface in its own GL
//! context (Slint's femtovg renderer is OpenGL, one context per window) and
//! blits it into a texture it shows. The mpv render context therefore lives as
//! long as the `Renderer`, not as long as a window: libmpv ends playback when
//! its render context is freed, so a window rebuild must never free it.
//!
//! macOS only. All `unsafe` stays in this crate; the public API is safe.
#![cfg(target_os = "macos")]

use glow::{HasContext, NativeFramebuffer, NativeTexture};
use objc2_core_foundation::{CFDictionary, CFNumber, CFRetained, CFString};
pub use objc2_io_surface::IOSurfaceRef;
use rsmpv::{Mpv, render::OpenGlFbo, render::OwnedRenderContext};
use slint::{BorrowedOpenGLTextureBuilder as Borrowed, BorrowedOpenGLTextureOrigin};
use slint::{ComponentHandle, GraphicsAPI, Image, RenderingState};
use std::ffi::{CStr, CString, c_void};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, mpsc};
use std::{num::NonZeroU32, thread, time::Duration, time::Instant};

type Cgl = *mut c_void;
type Size = (u32, u32);
/// Frames in flight: one being drawn, one being shown, one spare.
const RING: usize = 3;
/// `'BGRA'`: the IOSurface layout CGL binds as an RGBA texture.
const BGRA: i32 = 0x4247_5241;

#[rustfmt::skip]
#[link(name = "OpenGL", kind = "framework")]
unsafe extern "C" {
    fn CGLChoosePixelFormat(attributes: *const i32, out: *mut Cgl, count: *mut i32) -> i32;
    fn CGLDestroyPixelFormat(format: Cgl) -> i32;
    fn CGLCreateContext(format: Cgl, share: Cgl, out: *mut Cgl) -> i32;
    fn CGLDestroyContext(context: Cgl) -> i32;
    fn CGLSetCurrentContext(context: Cgl) -> i32;
    fn CGLGetCurrentContext() -> Cgl;
    fn CGLTexImageIOSurface2D(c: Cgl, t: u32, i: u32, w: i32, h: i32, f: u32, k: u32, s: &IOSurfaceRef, p: u32) -> i32;
}
/// GL entry points. On macOS they are process-wide symbols of OpenGL.framework
/// (linked above), the same for every context, so one loader serves all.
fn gl_symbol(name: &CStr) -> *const c_void {
    // SAFETY: `name` is NUL-terminated and dlsym only reads it; RTLD_DEFAULT
    // searches the images already loaded. Unknown names give null.
    unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) }
}
fn gl() -> glow::Context {
    // SAFETY: gl_symbol yields the framework's entry points (null for missing
    // ones, which glow records as absent), valid for the whole process.
    unsafe { glow::Context::from_loader_function_cstr(gl_symbol) }
}
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The renderer thread's own offscreen GL context, current while it lives.
struct Offscreen(Cgl);
impl Offscreen {
    fn new() -> Result<Self, String> {
        // Accelerated, allow offline renderers, OpenGL 4.1 core profile.
        const ATTRIBUTES: [i32; 5] = [73, 96, 99, 0x4100, 0];
        let (mut format, mut context, mut count) = (std::ptr::null_mut(), std::ptr::null_mut(), 0);
        // SAFETY: the attribute list is 0-terminated and the out-pointers point
        // at live locals. The context retains the pixel format, so it is
        // released right after; a null format is never passed on.
        unsafe {
            CGLChoosePixelFormat(ATTRIBUTES.as_ptr(), &mut format, &mut count);
            if !format.is_null() {
                CGLCreateContext(format, std::ptr::null_mut(), &mut context);
                CGLDestroyPixelFormat(format);
            }
            if context.is_null() {
                return Err("no OpenGL 4.1 context".into());
            }
            CGLSetCurrentContext(context);
        }
        Ok(Self(context))
    }
}
impl Drop for Offscreen {
    fn drop(&mut self) {
        // SAFETY: self.0 is live; it stops being current before it is destroyed.
        unsafe {
            CGLSetCurrentContext(std::ptr::null_mut());
            CGLDestroyContext(self.0);
        }
    }
}

/// One IOSurface of the ring.
struct Surface(CFRetained<IOSurfaceRef>);
// SAFETY: an IOSurface is a reference-counted kernel object that Apple documents
// as usable from any thread. Across threads it is only bound as a GL texture
// (after the drawing thread's glFinish) and read under its lock in tests.
unsafe impl Send for Surface {}
// SAFETY: as for Send; nothing mutates it through `&`.
unsafe impl Sync for Surface {}
impl Surface {
    fn new((width, height): Size) -> Option<Self> {
        let values = [width as i32, height as i32, 4, BGRA].map(CFNumber::new_i32);
        let values = values.each_ref().map(|value| &**value);
        // SAFETY: the keys are constant CFStrings exported by IOSurface; the
        // values are the CFNumbers IOSurfaceCreate documents for them.
        unsafe {
            use objc2_io_surface::{kIOSurfaceBytesPerElement, kIOSurfacePixelFormat};
            use objc2_io_surface::{kIOSurfaceHeight, kIOSurfaceWidth};
            let keys: [&CFString; 4] = [
                kIOSurfaceWidth,
                kIOSurfaceHeight,
                kIOSurfaceBytesPerElement,
                kIOSurfacePixelFormat,
            ];
            IOSurfaceRef::new(CFDictionary::from_slices(&keys, &values).as_opaque()).map(Self)
        }
    }
    fn size(&self) -> Size {
        (self.0.width() as u32, self.0.height() as u32)
    }
    /// Make this surface the image of `texture` (GL_TEXTURE_RECTANGLE, the
    /// only target CGL binds IOSurfaces to; GL_TEXTURE_2D fails).
    ///
    /// # Safety
    /// A GL context must be current and `texture` must be one of its textures.
    unsafe fn bind(&self, gl: &glow::Context, texture: NativeTexture) -> bool {
        let ((w, h), kind) = (self.size(), glow::UNSIGNED_INT_8_8_8_8_REV);
        // SAFETY: per the contract; the surface has the size and BGRA layout
        // declared here (it was created that way in `new`).
        unsafe {
            gl.bind_texture(glow::TEXTURE_RECTANGLE, Some(texture));
            let (rectangle, context) = (glow::TEXTURE_RECTANGLE, CGLGetCurrentContext());
            let (w, h) = (w as i32, h as i32);
            CGLTexImageIOSurface2D(
                context,
                rectangle,
                glow::RGBA,
                w,
                h,
                glow::BGRA,
                kind,
                &self.0,
                0,
            ) == 0
        }
    }
}

/// # Safety
/// A GL context must be current and both objects must be its own.
unsafe fn attach(
    gl: &glow::Context,
    fbo: u32,
    kind: u32,
    texture: NativeTexture,
    framebuffer: NativeFramebuffer,
) {
    // SAFETY: per the contract.
    unsafe {
        gl.bind_framebuffer(fbo, Some(framebuffer));
        gl.framebuffer_texture_2d(fbo, glow::COLOR_ATTACHMENT0, kind, Some(texture), 0);
    }
}

#[derive(Clone)]
struct Published {
    surface: Arc<Surface>,
    sequence: u64,
}
type Notify = Box<dyn Fn() + Send>;
#[derive(Default)]
struct Shared {
    stop: AtomicBool,
    size: Mutex<Size>,
    /// `Some(period)`: draw at most once per period and report each frame as
    /// presented (mpv's display-synced timing, for interpolation).
    pace: Mutex<Option<Duration>>,
    latest: Mutex<Option<Published>>,
    /// Set by mpv's update callback, a resize and stop; cleared by the renderer.
    pending: (Mutex<bool>, Condvar),
    /// Ask the last few attached windows to redraw (a dropped one ignores it).
    redraw: Mutex<Vec<Notify>>,
    /// Called after each new frame ([`Handle::on_frame`]).
    notify: Mutex<Vec<Notify>>,
    /// (epoch, hold): after [`Handle::reshow`], windows show nothing up to
    /// frame `hold`, then set the next frame's image again.
    reshow: Mutex<(u64, u64)>,
    /// Bumped by [`Handle::retarget`]: windows hand their current image to
    /// `show` again at their next draw.
    retargets: AtomicU64,
    /// Frames copied into windows.
    shown: AtomicU64,
}
impl Shared {
    fn wake(&self) {
        *lock(&self.pending.0) = true;
        self.pending.1.notify_one();
    }
    fn wait(&self, timeout: Duration) {
        let pending = lock(&self.pending.0);
        let wait = self.pending.1.wait_timeout_while(pending, timeout, |pending| !*pending);
        *wait.unwrap_or_else(PoisonError::into_inner).0 = false;
    }
}

/// Owns the render thread and with it mpv's (only) render context.
pub struct Renderer {
    shared: Arc<Shared>,
    thread: Option<thread::JoinHandle<()>>,
}
/// The UI side of a [`Renderer`]: cloneable and `Send`.
#[derive(Clone)]
pub struct Handle(Arc<Shared>);

impl Renderer {
    /// Render `mpv` (`vo=libmpv`, no other render context) at `size` pixels.
    /// While `publish` is false frames are drawn but not shown (the source's
    /// orientation is still being prepared).
    pub fn start(mpv: Arc<Mpv>, size: Size, publish: Arc<AtomicBool>) -> Result<Self, String> {
        let shared = Arc::new(Shared { size: Mutex::new(size), ..Shared::default() });
        let (ready, started) = mpsc::sync_channel(1);
        let inner = shared.clone();
        let thread = thread::Builder::new().name("vestra-gpu-render".into());
        let thread =
            thread.spawn(move || produce(mpv, inner, publish, ready)).map_err(|e| e.to_string())?;
        let renderer = Self { shared, thread: Some(thread) };
        started.recv().map_err(|_| "render thread died".to_string())??;
        Ok(renderer)
    }
    pub fn handle(&self) -> Handle {
        Handle(self.shared.clone())
    }
}
impl Drop for Renderer {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        self.shared.wake();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// One drawing target of the ring, in the offscreen context.
struct Target(Arc<Surface>, NativeTexture, NativeFramebuffer);
fn produce(
    mpv: Arc<Mpv>,
    shared: Arc<Shared>,
    publish: Arc<AtomicBool>,
    ready: mpsc::SyncSender<Result<(), String>>,
) {
    let _context = match Offscreen::new() {
        Ok(context) => context,
        Err(error) => return drop(ready.send(Err(error))),
    };
    let gl = gl();
    let resolve = |name: &str| CString::new(name).map_or(std::ptr::null(), |name| gl_symbol(&name));
    // SAFETY: `_context` is current on this thread from here on (this thread
    // makes no other context current), and `render`, declared later, drops
    // before `_context` at the end of this function.
    let made = unsafe {
        OwnedRenderContext::new_opengl(mpv, false, Box::new(move |n| resolve(n).cast_mut()))
    };
    let mut render = match made {
        Ok(render) => render,
        Err(error) => return drop(ready.send(Err(error.to_string()))),
    };
    let notify = shared.clone();
    render.set_update_callback(move || notify.wake()); // Only signals (rsmpv's rule).
    let _ = ready.send(Ok(()));
    let (mut ring, mut next, mut sequence, mut drawn) = (Vec::new(), 0, 0, None::<Instant>);
    while !shared.stop.load(Ordering::Acquire) {
        let pace = *lock(&shared.pace);
        // Paced: wait for the rest of the period, so frames come at most at
        // the display rate however often mpv asks.
        let left = pace.zip(drawn).map(|(period, at)| period.saturating_sub(at.elapsed()));
        match left {
            Some(left) if !left.is_zero() => thread::sleep(left),
            _ => shared.wait(Duration::from_millis(20)),
        }
        let size = *lock(&shared.size);
        let resized = ring.first().is_none_or(|t: &Target| t.0.size() != size);
        if resized {
            // SAFETY: the offscreen context is current (see above).
            unsafe { delete(&gl, &mut ring) };
            // SAFETY: as above.
            ring.extend((0..RING).map_while(|_| unsafe { target(&gl, size) }));
        }
        if ring.len() == RING && (render.update() || (resized && sequence > 0)) {
            // Skip a surface the window server still shows.
            let free = (0..RING).map(|i| (next + i) % RING).find(|&i| !ring[i].0.0.is_in_use());
            let Target(surface, _, fbo) = &ring[free.unwrap_or(next)];
            next = (free.unwrap_or(next) + 1) % RING;
            let (width, height) = (size.0 as i32, size.1 as i32);
            let fbo = OpenGlFbo { fbo: fbo.0.get() as i32, width, height, internal_format: 0 };
            if render.render_opengl(fbo, false, pace.is_none()).is_ok() {
                // SAFETY: the offscreen context is current. glFinish makes the
                // pixels complete before any window binds the surface.
                unsafe { gl.finish() };
                if pace.is_some() {
                    render.report_swap();
                }
                drawn = Some(Instant::now());
                if publish.load(Ordering::Acquire) {
                    sequence += 1;
                    *lock(&shared.latest) = Some(Published { surface: surface.clone(), sequence });
                    lock(&shared.redraw).iter().for_each(|redraw| redraw());
                    lock(&shared.notify).iter().for_each(|notify| notify());
                }
            }
        }
    }
    // SAFETY: the offscreen context is still current.
    unsafe { delete(&gl, &mut ring) };
}

/// # Safety
/// The offscreen context must be current.
unsafe fn target(gl: &glow::Context, size: Size) -> Option<Target> {
    let surface = Arc::new(Surface::new(size)?);
    // SAFETY: per the contract; the objects belong to the current context and
    // are deleted in it by `delete`.
    unsafe {
        let target = Target(surface, gl.create_texture().ok()?, gl.create_framebuffer().ok()?);
        let bound = target.0.bind(gl, target.1);
        attach(gl, glow::FRAMEBUFFER, glow::TEXTURE_RECTANGLE, target.1, target.2);
        let complete = gl.check_framebuffer_status(glow::FRAMEBUFFER) == glow::FRAMEBUFFER_COMPLETE;
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        if bound && complete {
            return Some(target);
        }
        delete(gl, &mut vec![target]);
        None
    }
}
/// # Safety
/// The offscreen context must be current.
unsafe fn delete(gl: &glow::Context, ring: &mut Vec<Target>) {
    for Target(_, texture, fbo) in ring.drain(..) {
        // SAFETY: per the contract; the objects were created in that context.
        unsafe {
            gl.delete_framebuffer(fbo);
            gl.delete_texture(texture);
        }
    }
}

impl Handle {
    /// The physical size to draw at (the video area).
    pub fn set_size(&self, size: Size) {
        let size = (size.0.clamp(1, 8192), size.1.clamp(1, 8192));
        if std::mem::replace(&mut *lock(&self.0.size), size) != size {
            self.0.wake();
        }
    }
    /// `Some(fps)`: draw at most `fps` frames a second and report each as
    /// presented (mpv `video-sync=display-*`); `None`: mpv's own timing.
    pub fn set_display_fps(&self, fps: Option<f64>) {
        let period =
            fps.filter(|f| f.is_finite() && *f >= 1.).map(|f| Duration::from_secs_f64(1. / f));
        *lock(&self.0.pace) = period;
        self.0.wake();
    }
    /// Frames published so far.
    pub fn frames(&self) -> u64 {
        lock(&self.0.latest).as_ref().map_or(0, |frame| frame.sequence)
    }
    /// Frames copied into windows so far.
    pub fn shown(&self) -> u64 {
        self.0.shown.load(Ordering::Relaxed)
    }
    /// The newest frame's size in pixels.
    pub fn size(&self) -> Option<Size> {
        lock(&self.0.latest).as_ref().map(|frame| frame.surface.size())
    }
    /// The caller replaced the image on the video area: windows show no frame
    /// published so far and set the next one's image again.
    pub fn reshow(&self) {
        let hold = self.frames();
        let mut reshow = lock(&self.0.reshow);
        *reshow = (reshow.0 + 1, hold);
    }
    /// The newest frame: its number and surface (for picture-in-picture).
    pub fn latest(&self) -> Option<(u64, CFRetained<IOSurfaceRef>)> {
        lock(&self.0.latest).as_ref().map(|frame| (frame.sequence, frame.surface.0.clone()))
    }
    /// Call `notify` on the render thread after each new frame.
    pub fn on_frame(&self, notify: impl Fn() + Send + 'static) {
        lock(&self.0.notify).push(Box::new(notify));
    }
    /// Show the video in `component`'s window (call before it is shown):
    /// `show` gets each new texture image (first frame, resize, after
    /// [`reshow`](Self::reshow), after [`retarget`](Self::retarget)) to set on
    /// the video area of whatever component the window shows then. Each new
    /// frame asks the last four attached windows to redraw. `failed` runs
    /// once if the window cannot show textures (its renderer is not OpenGL).
    /// `drawn` sees each draw of the window (this notifier is the window's
    /// only one).
    pub fn attach<T>(
        &self,
        component: &T,
        show: impl Fn(Image) + 'static,
        failed: impl FnOnce(String) + 'static,
        mut drawn: impl FnMut(Draw) + 'static,
    ) -> Result<(), String>
    where
        T: ComponentHandle + 'static,
    {
        self.redraw_on(component);
        let (shared, mut view) = (self.0.clone(), None::<View>);
        let (mut failed, mut copied) = (Some(failed), false);
        let (mut current, mut retargets) =
            (None::<Image>, self.0.retargets.load(Ordering::Relaxed));
        let notifier = move |state, api: &GraphicsAPI| match state {
            RenderingState::RenderingSetup => {
                view = matches!(api, GraphicsAPI::NativeOpenGL { .. }).then(View::new).flatten();
                if view.is_none()
                    && let Some(failed) = failed.take()
                {
                    failed("the window renderer cannot show OpenGL textures".into());
                }
            }
            RenderingState::BeforeRendering => {
                drawn(Draw::Begin);
                let copies = shared.shown.load(Ordering::Relaxed);
                let frame = lock(&shared.latest).clone();
                let reshow = *lock(&shared.reshow);
                let now = shared.retargets.load(Ordering::Relaxed);
                let retargeted = std::mem::replace(&mut retargets, now) != now;
                if let (Some(view), Some(frame)) = (view.as_mut(), frame)
                    && let Some(image) = view.update(&frame, reshow, &shared.shown)
                {
                    current = Some(image.clone());
                    show(image);
                } else if retargeted && let Some(image) = &current {
                    // Another component took the window's place: same texture.
                    show(image.clone());
                }
                copied = shared.shown.load(Ordering::Relaxed) != copies;
            }
            RenderingState::AfterRendering => {
                drawn(Draw::End { copied: std::mem::take(&mut copied) })
            }
            RenderingState::RenderingTeardown => {
                view = None;
                current = None;
            }
            _ => (),
        };
        component.window().set_rendering_notifier(notifier).map_err(|e| e.to_string())
    }
    /// Another component took the place of an attached one inside the same
    /// window (its rendering notifier stays): new frames ask it to redraw,
    /// and the window hands its current image to `show` again at its next
    /// draw, for the new component's video area.
    pub fn retarget<T>(&self, component: &T)
    where
        T: ComponentHandle + 'static,
    {
        self.redraw_on(component);
        self.0.retargets.fetch_add(1, Ordering::Relaxed);
    }
    /// Each new frame asks `component`'s window to redraw (the last four
    /// components given here; a dropped one ignores it).
    fn redraw_on<T>(&self, component: &T)
    where
        T: ComponentHandle + 'static,
    {
        let weak = component.as_weak();
        let mut redraw = lock(&self.0.redraw);
        redraw.push(Box::new(move || {
            let _ = weak.upgrade_in_event_loop(|component| component.window().request_redraw());
        }));
        let stale = redraw.len().saturating_sub(4);
        drop(redraw.drain(..stale));
    }
}

/// A draw of an attached window, for the host's presentation counters.
pub enum Draw {
    /// The window starts drawing.
    Begin,
    /// The window drew; `copied`: a new frame was copied into it first.
    End { copied: bool },
}

/// One window's side, in that window's GL context.
struct View {
    gl: glow::Context,
    source: NativeTexture,
    read: NativeFramebuffer,
    draw: NativeFramebuffer,
    /// The texture Slint shows and its size.
    shown: Option<(NativeTexture, Size)>,
    sequence: u64,
    epoch: u64,
}
impl View {
    fn new() -> Option<Self> {
        let gl = gl();
        // SAFETY: called from RenderingSetup, where Slint has this window's
        // context current; Drop deletes the objects in it at Teardown.
        let (source, read, draw) = unsafe {
            (
                gl.create_texture().ok()?,
                gl.create_framebuffer().ok()?,
                gl.create_framebuffer().ok()?,
            )
        };
        Some(Self { gl, source, read, draw, shown: None, sequence: 0, epoch: 0 })
    }
    /// Copy a new frame into the shown texture; an image to set when the
    /// texture changed or after a reshow.
    fn update(
        &mut self,
        frame: &Published,
        (epoch, hold): (u64, u64),
        count: &AtomicU64,
    ) -> Option<Image> {
        let stale = self.epoch != epoch;
        if frame.sequence <= hold && stale
            || std::mem::replace(&mut self.sequence, frame.sequence) == frame.sequence && !stale
        {
            return None;
        }
        self.epoch = epoch;
        let (gl, size) = (&self.gl, frame.surface.size());
        let (w, h, mut fresh) = (size.0 as i32, size.1 as i32, None);
        // SAFETY: Slint calls BeforeRendering with this window's context
        // current, and every object used was created in it. The bindings and
        // the scissor test changed here are restored before femtovg draws. A
        // replaced texture is deleted only after its successor exists; Slint
        // gets the new image before it draws again.
        unsafe {
            let names = [
                glow::READ_FRAMEBUFFER_BINDING,
                glow::DRAW_FRAMEBUFFER_BINDING,
                glow::TEXTURE_BINDING_2D,
            ];
            let (saved, scissor) =
                (names.map(|name| gl.get_parameter_i32(name)), gl.is_enabled(glow::SCISSOR_TEST));
            if self.shown.is_none_or(|(_, shown)| shown != size) {
                let texture = gl.create_texture().ok()?;
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                let (plain, rgba8, empty) = (glow::TEXTURE_2D, glow::RGBA8 as i32, None);
                let empty = glow::PixelUnpackData::Slice(empty);
                gl.tex_image_2d(plain, 0, rgba8, w, h, 0, glow::RGBA, glow::UNSIGNED_BYTE, empty);
                gl.tex_parameter_i32(plain, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
                attach(gl, glow::DRAW_FRAMEBUFFER, glow::TEXTURE_2D, texture, self.draw);
                if let Some((old, _)) = self.shown.replace((texture, size)) {
                    gl.delete_texture(old);
                }
                fresh = Some(texture);
            }
            if frame.surface.bind(gl, self.source) {
                attach(gl, glow::READ_FRAMEBUFFER, glow::TEXTURE_RECTANGLE, self.source, self.read);
                gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(self.draw));
                gl.disable(glow::SCISSOR_TEST);
                gl.blit_framebuffer(0, 0, w, h, 0, 0, w, h, glow::COLOR_BUFFER_BIT, glow::NEAREST);
                count.fetch_add(1, Ordering::Relaxed);
            }
            if scissor {
                gl.enable(glow::SCISSOR_TEST);
            }
            let id = |index: usize| NonZeroU32::new(saved[index] as u32);
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, id(0).map(NativeFramebuffer));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, id(1).map(NativeFramebuffer));
            gl.bind_texture(glow::TEXTURE_2D, id(2).map(NativeTexture));
        }
        let texture = fresh.or_else(|| self.shown.map(|(texture, _)| texture).filter(|_| stale))?;
        // SAFETY: `texture` is a complete RGBA8 GL_TEXTURE_2D of exactly `size`
        // in the context Slint renders this window with. It is deleted only
        // after Slint got its replacement (above) or at RenderingTeardown.
        let builder =
            unsafe { Borrowed::new_gl_2d_rgba_texture(texture.0, [size.0, size.1].into()) };
        Some(builder.origin(BorrowedOpenGLTextureOrigin::TopLeft).build())
    }
}
impl Drop for View {
    fn drop(&mut self) {
        let gl = &self.gl;
        // SAFETY: the only owner is the notifier closure, which drops it at
        // RenderingTeardown with this window's context still current (Slint
        // sends Teardown before it destroys a window's renderer).
        unsafe {
            for texture in
                [Some(self.source), self.shown.map(|(texture, _)| texture)].into_iter().flatten()
            {
                gl.delete_texture(texture);
            }
            gl.delete_framebuffer(self.read);
            gl.delete_framebuffer(self.draw);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn until(what: impl Fn() -> bool) -> bool {
        let start = Instant::now();
        while !what() && start.elapsed() < Duration::from_secs(10) {
            thread::sleep(Duration::from_millis(10));
        }
        what()
    }
    /// The newest frame's size and the red value at the left of row `y`.
    fn red(handle: &Handle, y: usize) -> Option<(Size, u8)> {
        let frame = lock(&handle.0.latest).clone()?;
        let (surface, read) = (&frame.surface.0, objc2_io_surface::IOSurfaceLockOptions::ReadOnly);
        // SAFETY: the read-only lock pins the surface's memory (BGRA rows of
        // bytes_per_row bytes); `y` is a row inside it; read before the unlock.
        let red = unsafe {
            surface.lock(read, std::ptr::null_mut());
            let pixel =
                surface.base_address().as_ptr().cast::<u8>().add(y * surface.bytes_per_row());
            let red = pixel.add(2).read();
            surface.unlock(read, std::ptr::null_mut());
            red
        };
        Some((frame.surface.size(), red))
    }
    fn player() -> Arc<Mpv> {
        let mpv = Mpv::builder()
            .and_then(|b| b.set_property("vo", "libmpv")?.set_property("ao", "null")?.build());
        Arc::new(mpv.expect("mpv"))
    }
    const SOURCE: &str =
        "av://lavfi:color=c=white:s=64x64:d=30,drawbox=y=32:w=64:h=32:color=black:t=fill";
    #[test]
    #[ignore = "needs a GPU OpenGL 4.1 context, which CI's macOS VMs lack; `npm run ci:local` runs it"]
    fn draws_offscreen_top_row_first_and_survives_resizes_and_new_sources() {
        let mpv = player();
        let renderer = Renderer::start(mpv.clone(), (64, 64), Arc::new(true.into())).expect("gl");
        let handle = renderer.handle();
        mpv.command(&["loadfile", SOURCE]).expect("loadfile");
        assert!(until(|| handle.frames() > 0), "no frame");
        let (top, bottom) = (red(&handle, 2).expect("frame").1, red(&handle, 61).expect("frame").1);
        assert!(top > 200 && bottom < 50, "top {top} bottom {bottom}");
        handle.set_size((32, 16));
        assert!(until(|| red(&handle, 0).is_some_and(|(size, _)| size == (32, 16))), "no resize");
        assert_eq!(handle.size(), Some((32, 16)));
        let before = handle.frames();
        mpv.command(&["loadfile", SOURCE]).expect("reload");
        assert!(until(|| handle.frames() > before), "no frame after a new source");
    }
    #[test]
    #[ignore = "needs a GPU OpenGL 4.1 context, which CI's macOS VMs lack; `npm run ci:local` runs it"]
    fn a_closed_gate_publishes_nothing_and_pacing_caps_the_rate() {
        let mpv = player();
        let gate = Arc::new(AtomicBool::new(false));
        let renderer = Renderer::start(mpv.clone(), (32, 32), gate.clone()).expect("gl");
        let handle = renderer.handle();
        mpv.set_property("video-sync", "display-resample").expect("sync");
        mpv.set_property("display-fps-override", "30").expect("fps");
        handle.set_display_fps(Some(30.));
        mpv.command(&["loadfile", "av://lavfi:testsrc2=s=32x32:r=60:d=30"]).expect("loadfile");
        thread::sleep(Duration::from_millis(500));
        assert_eq!(handle.frames(), 0, "gate closed");
        gate.store(true, Ordering::Release);
        assert!(until(|| handle.frames() > 0), "no frame once open");
        let start = (Instant::now(), handle.frames());
        thread::sleep(Duration::from_secs(1));
        let rate = (handle.frames() - start.1) as f64 / start.0.elapsed().as_secs_f64();
        assert!((15. ..=36.).contains(&rate), "paced rate {rate}");
        handle.reshow();
        let (epoch, hold) = *lock(&handle.0.reshow);
        assert!(epoch == 1 && hold > 0 && hold <= handle.frames(), "{epoch} {hold}");
    }
}
