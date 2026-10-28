//! Software-rendered video pictures in a Slint OpenGL (femtovg) window through
//! one reused texture: the host's software rendering path.
//!
//! Setting each picture as a new `slint::Image` makes femtovg 0.27 create a
//! new texture per frame: it allocates a zeroed buffer of the whole frame,
//! uploads it with `glTexImage2D`, uploads the pixels again with
//! `glTexSubImage2D` and deletes the previous texture. Measured on an AMD
//! 780M (GL 3.3, 2026-10-25, `glFinish` included): 7.4 ms per frame at
//! 2560x1600 and 14.3 ms (up to 34 ms) at 3840x2160, against 1.1 ms and
//! 2.1 ms for one `glTexSubImage2D` into a texture that is kept. This crate
//! keeps the texture: the window's rendering notifier uploads the newest
//! picture into it right before Slint draws, and the video area shows it as a
//! borrowed OpenGL texture.
//!
//! On Windows the windows draw with femtovg on wgpu (Direct3D 12): there the
//! same `Feed` keeps one wgpu texture per window instead (`wgpu_view.rs`,
//! safe code).
//!
//! Windows that do not draw with OpenGL or wgpu (Slint's software renderer)
//! have no rendering notifier or no such API; `Feed::put` then refuses and the
//! host sets the pixels as an image, as before. All `unsafe` stays in this
//! crate; the public API is safe.

#[cfg(windows)]
mod wgpu_view;

use glow::{HasContext, NativeTexture};
use slint::{
    BorrowedOpenGLTextureBuilder as Borrowed, BorrowedOpenGLTextureOrigin, ComponentHandle,
    GraphicsAPI, Image, RenderingState, Rgb8Pixel, SharedPixelBuffer,
};
use std::cell::RefCell;
use std::num::NonZeroU32;
use std::rc::Rc;

type Size = (u32, u32);

/// A draw of the window, for the host's presentation counters.
pub enum Draw {
    /// The window starts drawing.
    Begin,
    /// The window drew.
    End,
}

#[derive(Default)]
struct State {
    /// The window draws with OpenGL and its context is set up.
    ready: bool,
    /// The newest picture, uploaded at the next draw.
    pending: Option<SharedPixelBuffer<Rgb8Pixel>>,
    /// The texture's image and size, once one exists.
    image: Option<(Image, Size)>,
    /// Uploads since the frame log last read them: count, longest, total
    /// (UI thread, inside the draw's `BeforeRendering`).
    uploads: (u32, std::time::Duration, std::time::Duration),
    /// Windows, a wgpu window without a rendering notifier (`direct`): the
    /// texture, filled at once by `put`.
    #[cfg(windows)]
    direct: Option<wgpu_view::View>,
}

/// One window's video pictures (UI thread).
#[derive(Clone)]
pub struct Feed(Rc<RefCell<State>>);

/// Show the pictures handed to the returned `Feed` in `component`'s window
/// (call before it is shown; this is the window's only rendering notifier).
/// `show` gets the texture's image when the texture is created or resized,
/// to set on the video area of whatever component the window shows then
/// (the window outlives a component when another one replaces it in the
/// same window). `drawn` sees each draw. An error: the window's
/// renderer has no rendering notifier (Slint's software renderer).
pub fn attach<T>(
    component: &T,
    show: impl Fn(Image) + 'static,
    mut drawn: impl FnMut(Draw) + 'static,
) -> Result<Feed, String>
where
    T: ComponentHandle + 'static,
{
    let feed = Feed(Rc::default());
    let state = feed.0.clone();
    let mut view: Option<Target> = None;
    let notifier = move |rendering: RenderingState, api: &GraphicsAPI| match rendering {
        RenderingState::RenderingSetup => {
            view = match api {
                GraphicsAPI::NativeOpenGL { get_proc_address } => {
                    Some(Target::Gl(View::new(get_proc_address)))
                }
                #[cfg(windows)]
                GraphicsAPI::WGPU30 { device, queue, .. } => {
                    wgpu_view::View::new(device, queue).map(Target::Wgpu)
                }
                _ => None,
            };
            let mut state = state.borrow_mut();
            state.ready = view.is_some();
            state.image = None;
        }
        RenderingState::BeforeRendering => {
            drawn(Draw::Begin);
            let pending = state.borrow_mut().pending.take();
            if let (Some(view), Some(buffer)) = (view.as_mut(), pending) {
                let began = std::time::Instant::now();
                let image = view.upload(&buffer);
                let took = began.elapsed();
                let mut current = state.borrow_mut();
                let uploads = &mut current.uploads;
                *uploads = (uploads.0 + 1, uploads.1.max(took), uploads.2 + took);
                if let Some(image) = image {
                    current.image = Some((image.clone(), view.size()));
                    drop(current);
                    show(image);
                }
            }
        }
        RenderingState::AfterRendering => drawn(Draw::End),
        RenderingState::RenderingTeardown => {
            view = None;
            let mut state = state.borrow_mut();
            state.ready = false;
            state.image = None;
        }
        _ => (),
    };
    component
        .window()
        .set_rendering_notifier(notifier)
        .map_err(|e| e.to_string())?;
    Ok(feed)
}

/// Windows: the feed of a wgpu window that has no rendering notifier (a
/// notifier makes Slint 1.18.1's femtovg-wgpu recompile its pipelines every
/// draw; `vestra-gpu-render` `uses_rendering_notifier`). Each picture is
/// uploaded and expanded at once in `put`, on the device and queue the
/// windows draw with; `put` always returns the texture's image to set.
/// `None` if the device refuses the pipeline.
#[cfg(windows)]
pub fn direct(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Feed> {
    let view = wgpu_view::View::new(device, queue)?;
    let state = State {
        ready: true,
        direct: Some(view),
        ..State::default()
    };
    Some(Feed(Rc::new(RefCell::new(state))))
}

impl Feed {
    /// `VESTRA_FRAME_LOG`: the picture uploads into the texture since the
    /// last call (` upload max A mean B ms`, the RGB copy and, with wgpu, the
    /// expansion's submit), or "" without any.
    pub fn take_upload_text(&self) -> String {
        let (count, max, total) = std::mem::take(&mut self.0.borrow_mut().uploads);
        if count == 0 {
            return String::new();
        }
        let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.;
        format!(
            " upload max {:.1} mean {:.1} ms",
            ms(max),
            ms(total) / f64::from(count)
        )
    }
    /// Hand over the newest picture; it is uploaded at the next draw (ask
    /// the window to redraw). `None`: not shown here (no OpenGL context yet,
    /// or none at all): set the pixels as an image instead. `Some(image)`:
    /// the texture's image to keep on the video area, `Some(None)` while the
    /// texture for this size does not exist yet (`show` gets it at the draw).
    pub fn put(&self, buffer: SharedPixelBuffer<Rgb8Pixel>) -> Option<Option<Image>> {
        let mut state = self.0.borrow_mut();
        if !state.ready {
            return None;
        }
        #[cfg(windows)]
        if let Some(view) = state.direct.as_mut() {
            // No notifier: upload now (between draws, on the UI thread).
            let began = std::time::Instant::now();
            let fresh = view.upload(&buffer);
            let size = view.size();
            let took = began.elapsed();
            let uploads = &mut state.uploads;
            *uploads = (uploads.0 + 1, uploads.1.max(took), uploads.2 + took);
            if let Some(image) = fresh {
                state.image = Some((image, size));
            }
            return Some(state.image.as_ref().map(|(image, _)| image.clone()));
        }
        let size = (buffer.width(), buffer.height());
        let image = state
            .image
            .as_ref()
            .filter(|(_, shown)| *shown == size)
            .map(|(image, _)| image.clone());
        state.pending = Some(buffer);
        Some(image)
    }
}

/// The window's texture: OpenGL (femtovg), or on Windows wgpu (femtovg on
/// Direct3D 12, `wgpu_view.rs`).
// One per window, made once: its size does not matter.
#[allow(clippy::large_enum_variant)]
enum Target {
    Gl(View),
    #[cfg(windows)]
    Wgpu(wgpu_view::View),
}
impl Target {
    fn size(&self) -> Size {
        match self {
            Self::Gl(view) => view.size(),
            #[cfg(windows)]
            Self::Wgpu(view) => view.size(),
        }
    }
    fn upload(&mut self, buffer: &SharedPixelBuffer<Rgb8Pixel>) -> Option<Image> {
        match self {
            Self::Gl(view) => view.upload(buffer),
            #[cfg(windows)]
            Self::Wgpu(view) => view.upload(buffer),
        }
    }
}

/// The window's texture, in that window's GL context.
struct View {
    gl: glow::Context,
    texture: Option<(NativeTexture, Size)>,
}
impl View {
    fn new(get_proc_address: &dyn Fn(&std::ffi::CStr) -> *const std::ffi::c_void) -> Self {
        // SAFETY: called from RenderingSetup, where Slint has this window's
        // context current; `get_proc_address` resolves functions of that
        // context, and glow loads them all here.
        let gl = unsafe { glow::Context::from_loader_function_cstr(get_proc_address) };
        Self { gl, texture: None }
    }
    fn size(&self) -> Size {
        self.texture.map_or((0, 0), |(_, size)| size)
    }
    /// Upload `buffer` into the texture (a new one when the size changed);
    /// the new texture's image when there is one.
    fn upload(&mut self, buffer: &SharedPixelBuffer<Rgb8Pixel>) -> Option<Image> {
        let size = (buffer.width(), buffer.height());
        let (w, h) = (size.0 as i32, size.1 as i32);
        let gl = &self.gl;
        let mut fresh = None;
        // Unpack row length, skips and buffer exist from OpenGL 2.1 / ES 3.0
        // on; an OpenGL ES 2 context has none of them (and only the defaults).
        let version = gl.version();
        let unpack_state = !version.is_embedded || version.major >= 3;
        // SAFETY: Slint calls BeforeRendering with this window's context
        // current, and the texture was created in it. Before the uploads the
        // unpack state is set explicitly: no pixel unpack buffer bound (so the
        // pointers below are client memory and `None` means no data), row
        // length 0 and no skipped pixels or rows (rows are `w` pixels, read
        // from the start), UNPACK_ALIGNMENT 1 (RGB rows are not padded). So
        // the upload reads exactly w*h*3 bytes from `buffer`, which holds them
        // and outlives the call (the driver copies before returning). The
        // texture binding and every unpack parameter changed here are
        // restored before femtovg draws. A replaced texture is deleted only
        // after its successor exists; Slint gets the new image before it
        // draws.
        unsafe {
            let bound = gl.get_parameter_i32(glow::TEXTURE_BINDING_2D);
            let alignment = gl.get_parameter_i32(glow::UNPACK_ALIGNMENT);
            let saved = unpack_state.then(|| {
                (
                    gl.get_parameter_i32(glow::PIXEL_UNPACK_BUFFER_BINDING),
                    [
                        glow::UNPACK_ROW_LENGTH,
                        glow::UNPACK_SKIP_PIXELS,
                        glow::UNPACK_SKIP_ROWS,
                    ]
                    .map(|name| (name, gl.get_parameter_i32(name))),
                )
            });
            if let Some((_, parameters)) = &saved {
                gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, None);
                for (name, _) in parameters {
                    gl.pixel_store_i32(*name, 0);
                }
            }
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            if self.texture.is_none_or(|(_, shown)| shown != size)
                && let Ok(texture) = gl.create_texture()
            {
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                let empty = glow::PixelUnpackData::Slice(None);
                let (plain, rgb) = (glow::TEXTURE_2D, glow::RGB);
                gl.tex_image_2d(
                    plain,
                    0,
                    rgb as i32,
                    w,
                    h,
                    0,
                    rgb,
                    glow::UNSIGNED_BYTE,
                    empty,
                );
                for (name, value) in [
                    (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                    (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                    (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                    (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
                ] {
                    gl.tex_parameter_i32(plain, name, value as i32);
                }
                if let Some((old, _)) = self.texture.replace((texture, size)) {
                    gl.delete_texture(old);
                }
                fresh = Some(texture);
            }
            // Only into a texture of this size (none when creating it failed).
            if let Some((texture, shown)) = self.texture
                && shown == size
            {
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                gl.tex_sub_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    0,
                    0,
                    w,
                    h,
                    glow::RGB,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(buffer.as_bytes())),
                );
            }
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, alignment);
            if let Some((unpack_buffer, parameters)) = saved {
                for (name, value) in parameters {
                    gl.pixel_store_i32(name, value);
                }
                let unpack_buffer = NonZeroU32::new(unpack_buffer as u32).map(glow::NativeBuffer);
                gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, unpack_buffer);
            }
            let bound = NonZeroU32::new(bound as u32).map(NativeTexture);
            gl.bind_texture(glow::TEXTURE_2D, bound);
        }
        let texture = fresh?;
        // SAFETY: `texture` is a complete GL_TEXTURE_2D of exactly `size` in
        // the context Slint renders this window with (RGB: sampled alpha is
        // 1). It is deleted only after Slint got its replacement (above) or
        // at RenderingTeardown.
        let builder =
            unsafe { Borrowed::new_gl_2d_rgba_texture(texture.0, [size.0, size.1].into()) };
        Some(builder.origin(BorrowedOpenGLTextureOrigin::TopLeft).build())
    }
}
impl Drop for View {
    fn drop(&mut self) {
        if let Some((texture, _)) = self.texture.take() {
            // SAFETY: the only owner is the notifier closure, which drops it
            // at RenderingTeardown with this window's context still current.
            unsafe { self.gl.delete_texture(texture) };
        }
    }
}
