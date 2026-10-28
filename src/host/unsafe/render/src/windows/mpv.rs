//! libmpv's Direct3D 11 render context (render API 2.6). rsmpv has no
//! binding for it, so this uses rsmpv-sys with the two structs of mpv's
//! `render_d3d11.h` (the parked `gpu-render` branch's code, round record
//! 2026-10-windows-render Appendix C).

use rsmpv::Mpv;
use rsmpv_sys as sys;
use std::ffi::{CStr, c_int, c_void};
use std::sync::Arc;
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11Texture2D};
use windows::core::Interface;

/// `MPV_RENDER_PARAM_D3D11_INIT_PARAMS` and `MPV_RENDER_PARAM_D3D11_FBO`
/// (render.h of the shipped libmpv, API 2.6; rsmpv-sys stops at 20).
const MPV_RENDER_PARAM_D3D11_INIT_PARAMS: sys::mpv_render_param_type = 21;
const MPV_RENDER_PARAM_D3D11_FBO: sys::mpv_render_param_type = 22;

/// `mpv_d3d11_init_params` (render_d3d11.h, mpv e471496c56).
#[repr(C)]
struct InitParams {
    device: *mut c_void,
}
/// `mpv_d3d11_fbo` (render_d3d11.h).
#[repr(C)]
struct Fbo {
    tex: *mut c_void,
}

fn error_text(code: c_int) -> String {
    // SAFETY: mpv_error_string returns a static NUL-terminated string for any code.
    unsafe { CStr::from_ptr(sys::mpv_error_string(code)) }.to_string_lossy().into_owned()
}

/// What mpv's update callback wakes (it may only signal, render.h).
pub trait Wake: Send + Sync {
    fn wake(&self);
}

/// mpv's render context on our device; render thread only. It must be
/// dropped before the `Mpv` it was made for and before the `Arc` given to
/// `on_update` (the render thread owns all three and drops this first).
pub struct RenderContext(*mut sys::mpv_render_context);

impl RenderContext {
    pub fn new(mpv: &Mpv, device: &ID3D11Device) -> Result<Self, String> {
        let mut init = InitParams { device: device.as_raw() };
        let mut params = [
            sys::mpv_render_param {
                type_: sys::MPV_RENDER_PARAM_API_TYPE,
                data: c"d3d11".as_ptr().cast_mut().cast(),
            },
            sys::mpv_render_param {
                type_: MPV_RENDER_PARAM_D3D11_INIT_PARAMS,
                data: (&raw mut init).cast(),
            },
            sys::mpv_render_param {
                type_: sys::MPV_RENDER_PARAM_INVALID,
                data: std::ptr::null_mut(),
            },
        ];
        let mut context = std::ptr::null_mut();
        // SAFETY: `mpv` is a live handle; the parameter list ends with
        // INVALID and its values outlive the call; mpv takes its own
        // reference to the device (render_d3d11.h).
        let result = unsafe {
            sys::mpv_render_context_create(&mut context, mpv.as_raw(), params.as_mut_ptr())
        };
        if result < 0 || context.is_null() {
            return Err(format!("mpv's Direct3D 11 render context: {}", error_text(result)));
        }
        Ok(Self(context))
    }

    /// Call `target.wake()` when mpv has something to render. `target`
    /// must outlive this context (see the type's contract).
    pub fn on_update<W: Wake>(&self, target: &Arc<W>) {
        extern "C" fn wake<W: Wake>(data: *mut c_void) {
            // SAFETY: `data` is the `W` registered below, alive until after
            // mpv_render_context_free (the type's contract), after which mpv
            // never calls this.
            unsafe { &*data.cast::<W>() }.wake();
        }
        let data = Arc::as_ptr(target).cast_mut().cast();
        // SAFETY: a live context; `data` outlives it (see `wake`).
        unsafe { sys::mpv_render_context_set_update_callback(self.0, Some(wake::<W>), data) };
    }

    /// Whether a new frame is due.
    pub fn update(&self) -> bool {
        // SAFETY: a live context, used on the render thread only.
        let flags = unsafe { sys::mpv_render_context_update(self.0) };
        flags & sys::MPV_RENDER_UPDATE_FRAME as u64 != 0
    }

    /// Draw the current frame into `texture`, a render target of the device
    /// mpv was given. `block`: wait for the frame's display time first.
    pub fn render(&self, texture: &ID3D11Texture2D, block: bool) -> Result<(), String> {
        let mut fbo = Fbo { tex: texture.as_raw() };
        let mut block = c_int::from(block);
        let mut params = [
            sys::mpv_render_param {
                type_: MPV_RENDER_PARAM_D3D11_FBO,
                data: (&raw mut fbo).cast(),
            },
            sys::mpv_render_param {
                type_: sys::MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME,
                data: (&raw mut block).cast(),
            },
            sys::mpv_render_param {
                type_: sys::MPV_RENDER_PARAM_INVALID,
                data: std::ptr::null_mut(),
            },
        ];
        // SAFETY: a live context, render thread only; the list ends with
        // INVALID and its values outlive the call; the texture is an
        // R8G8B8A8 render target of mpv's device. Nothing else uses the
        // device's immediate context meanwhile: only this thread does, and
        // mpv's decoder under the multithread protection.
        let result = unsafe { sys::mpv_render_context_render(self.0, params.as_mut_ptr()) };
        if result < 0 { Err(error_text(result)) } else { Ok(()) }
    }

    /// The frame drawn last was shown (display-synced timing).
    pub fn report_swap(&self) {
        // SAFETY: a live context, render thread only.
        unsafe { sys::mpv_render_context_report_swap(self.0) };
    }
}

impl Drop for RenderContext {
    fn drop(&mut self) {
        // SAFETY: created in `new`, freed exactly once; the update callback
        // never runs after this returns.
        unsafe { sys::mpv_render_context_free(self.0) };
    }
}
