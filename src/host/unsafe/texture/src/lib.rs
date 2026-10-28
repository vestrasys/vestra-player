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
//! Windows that do not draw with OpenGL (Slint's software renderer) have no
//! rendering notifier or no OpenGL; `Feed::put` then refuses and the host
//! sets the pixels as an image, as before. All `unsafe` stays in this crate;
//! the public API is safe.

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
    let mut view: Option<View> = None;
    let notifier = move |rendering: RenderingState, api: &GraphicsAPI| match rendering {
        RenderingState::RenderingSetup => {
            view = match api {
                GraphicsAPI::NativeOpenGL { get_proc_address } => Some(View::new(get_proc_address)),
                _ => None,
            };
            let mut state = state.borrow_mut();
            state.ready = view.is_some();
            state.image = None;
        }
        RenderingState::BeforeRendering => {
            drawn(Draw::Begin);
            let pending = state.borrow_mut().pending.take();
            if let (Some(view), Some(buffer)) = (view.as_mut(), pending)
                && let Some(image) = view.upload(&buffer)
            {
                state.borrow_mut().image = Some((image.clone(), view.size()));
                show(image);
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

impl Feed {
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
