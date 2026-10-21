//! Prototype (vestra-host feature `gpu-render`): libmpv draws with OpenGL and
//! Slint shows the result as a texture.
//!
//! libmpv renders on its own thread, in a private offscreen CGL context, into a
//! ring of IOSurfaces. Each Slint window binds the newest surface in its own GL
//! context (Slint's femtovg renderer is OpenGL, one context per window) and
//! blits it into a texture it shows. The mpv render context therefore lives as
//! long as the [`Renderer`], not as long as a window: libmpv ends playback when
//! its render context is freed, so a window rebuild must never free it.
//!
//! macOS only. All `unsafe` stays in this crate; the public API is safe.
#![cfg(target_os = "macos")]

use glow::{HasContext, NativeFramebuffer, NativeTexture};
use objc2_core_foundation::{CFDictionary, CFNumber, CFRetained, CFString};
use objc2_io_surface::IOSurfaceRef;
use rsmpv::{Mpv, render::OpenGlFbo, render::OwnedRenderContext};
use slint::{BorrowedOpenGLTextureBuilder as Borrowed, BorrowedOpenGLTextureOrigin};
use slint::{ComponentHandle, GraphicsAPI, Image, RenderingState};
use std::ffi::{CStr, CString, c_void};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, atomic::AtomicBool, mpsc};
use std::{num::NonZeroU32, sync::atomic::Ordering, thread, time::Duration};

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
#[derive(Default)]
struct Shared {
    stop: AtomicBool,
    size: Mutex<Size>,
    latest: Mutex<Option<Published>>,
    /// Set by mpv's update callback, a resize and stop; cleared by the renderer.
    pending: (Mutex<bool>, Condvar),
    /// Ask the last few attached windows to redraw (a dropped one ignores it).
    redraw: Mutex<Vec<Box<dyn Fn() + Send>>>,
}
impl Shared {
    fn wake(&self) {
        *lock(&self.pending.0) = true;
        self.pending.1.notify_one();
    }
    fn wait(&self) {
        let (pending, timeout) = (lock(&self.pending.0), Duration::from_millis(20));
        let wait = self.pending.1.wait_timeout_while(pending, timeout, |pending| !*pending);
        *wait.unwrap_or_else(PoisonError::into_inner).0 = false;
    }
}

/// Owns the render thread and with it mpv's (only) render context.
pub struct Renderer {
    shared: Arc<Shared>,
    thread: Option<thread::JoinHandle<()>>,
}
/// The UI side of a [`Renderer`]: clonable and `Send`.
#[derive(Clone)]
pub struct Handle(Arc<Shared>);

impl Renderer {
    /// Render `mpv` (`vo=libmpv`, no other render context) at `size` pixels.
    pub fn start(mpv: Arc<Mpv>, size: Size) -> Result<Self, String> {
        let shared = Arc::new(Shared { size: Mutex::new(size), ..Shared::default() });
        let (ready, started) = mpsc::sync_channel(1);
        let inner = shared.clone();
        let thread = thread::Builder::new().name("vestra-gpu-render".into());
        let thread = thread.spawn(move || produce(mpv, inner, ready)).map_err(|e| e.to_string())?;
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
fn produce(mpv: Arc<Mpv>, shared: Arc<Shared>, ready: mpsc::SyncSender<Result<(), String>>) {
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
    let (mut ring, mut next, mut sequence) = (Vec::new(), 0, 0);
    while !shared.stop.load(Ordering::Acquire) {
        shared.wait();
        let size = *lock(&shared.size);
        let resized = ring.first().is_none_or(|t: &Target| t.0.size() != size);
        if resized {
            // SAFETY: the offscreen context is current (see above).
            unsafe { delete(&gl, &mut ring) };
            // SAFETY: as above.
            ring.extend((0..RING).map_while(|_| unsafe { target(&gl, size) }));
        }
        if ring.len() == RING && (render.update() || (resized && sequence > 0)) {
            let Target(surface, _, fbo) = &ring[next];
            next = (next + 1) % RING;
            let (width, height) = (size.0 as i32, size.1 as i32);
            let fbo = OpenGlFbo { fbo: fbo.0.get() as i32, width, height, internal_format: 0 };
            if render.render_opengl(fbo, false, true).is_ok() {
                // SAFETY: the offscreen context is current. glFinish makes the
                // pixels complete before any window binds the surface.
                unsafe { gl.finish() };
                sequence += 1;
                *lock(&shared.latest) = Some(Published { surface: surface.clone(), sequence });
                lock(&shared.redraw).iter().for_each(|redraw| redraw());
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
    /// Frames drawn so far.
    pub fn frames(&self) -> u64 {
        lock(&self.0.latest).as_ref().map_or(0, |frame| frame.sequence)
    }
    /// Show the video in `component`'s window (call before it is shown):
    /// `show` gets each new texture image (first frame, resize) to set on the
    /// video area. Each new frame asks the last four attached windows to redraw.
    pub fn attach<T>(&self, component: &T, show: impl Fn(&T, Image) + 'static) -> Result<(), String>
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
        drop(redraw);
        let (shared, weak, mut view) = (self.0.clone(), component.as_weak(), None::<View>);
        let notifier = move |state, api: &GraphicsAPI| match state {
            RenderingState::RenderingSetup if matches!(api, GraphicsAPI::NativeOpenGL { .. }) => {
                view = View::new();
            }
            RenderingState::BeforeRendering => {
                let frame = lock(&shared.latest).clone();
                if let (Some(view), Some(frame), Some(component)) =
                    (view.as_mut(), frame, weak.upgrade())
                    && let Some(image) = view.update(&frame)
                {
                    show(&component, image);
                }
            }
            RenderingState::RenderingTeardown => view = None,
            _ => (),
        };
        component.window().set_rendering_notifier(notifier).map_err(|e| e.to_string())
    }
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
        Some(Self { gl, source, read, draw, shown: None, sequence: 0 })
    }
    /// Copy a new frame into the shown texture; a new image when that changed.
    fn update(&mut self, frame: &Published) -> Option<Image> {
        if std::mem::replace(&mut self.sequence, frame.sequence) == frame.sequence {
            return None;
        }
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
            }
            if scissor {
                gl.enable(glow::SCISSOR_TEST);
            }
            let id = |index: usize| NonZeroU32::new(saved[index] as u32);
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, id(0).map(NativeFramebuffer));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, id(1).map(NativeFramebuffer));
            gl.bind_texture(glow::TEXTURE_2D, id(2).map(NativeTexture));
        }
        // SAFETY: `fresh` is a complete RGBA8 GL_TEXTURE_2D of exactly `size` in
        // the context Slint renders this window with. It is deleted only after
        // Slint got its replacement (above) or at RenderingTeardown.
        let builder =
            unsafe { Borrowed::new_gl_2d_rgba_texture(fresh?.0, [size.0, size.1].into()) };
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
        let start = std::time::Instant::now();
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
    #[test]
    fn draws_offscreen_top_row_first_and_survives_resizes_and_new_sources() {
        let mpv = Mpv::builder()
            .and_then(|b| b.set_property("vo", "libmpv")?.set_property("ao", "null")?.build());
        let mpv = Arc::new(mpv.expect("mpv"));
        let renderer = Renderer::start(mpv.clone(), (64, 64)).expect("renderer");
        let handle = renderer.handle();
        let source =
            "av://lavfi:color=c=white:s=64x64:d=30,drawbox=y=32:w=64:h=32:color=black:t=fill";
        mpv.command(&["loadfile", source]).expect("loadfile");
        assert!(until(|| handle.frames() > 0), "no frame");
        let (top, bottom) = (red(&handle, 2).expect("frame").1, red(&handle, 61).expect("frame").1);
        assert!(top > 200 && bottom < 50, "top {top} bottom {bottom}");
        handle.set_size((32, 16));
        assert!(until(|| red(&handle, 0).is_some_and(|(size, _)| size == (32, 16))), "no resize");
        let before = handle.frames();
        mpv.command(&["loadfile", source]).expect("reload");
        assert!(until(|| handle.frames() > before), "no frame after a new source");
    }
}
