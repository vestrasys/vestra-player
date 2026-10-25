//! Which renderer draws the video (reference §5.4). GPU rendering (libmpv
//! OpenGL, through the platform module) is the default; software rendering
//! (`RenderContext::new_software`) is the fallback: headless runs,
//! `VESTRA_SOFTWARE_RENDER=1`, systems without a GPU implementation, and a GL
//! failure at start (mpv's context, or a window that cannot show textures).
#![forbid(unsafe_code)]

use crate::platform;
use std::ffi::OsStr;
use std::sync::{
    Arc, Mutex, OnceLock, PoisonError,
    atomic::{AtomicBool, Ordering},
};

/// `stats().backend` for each renderer, in the interface language.
pub fn gpu_name() -> String {
    crate::i18n::tr("libmpv · GPU rendering (OpenGL)")
}
pub fn software_name() -> String {
    crate::i18n::tr("libmpv · software rendering")
}

/// Shared by the backend's render thread, its control worker and the UI.
#[derive(Default)]
pub struct Render {
    wanted: bool,
    handle: OnceLock<platform::GpuHandle>,
    /// The GPU renderer draws now.
    active: AtomicBool,
    /// A window could not show GPU frames: the render thread switches.
    fallback: AtomicBool,
    /// A generated stream (`openStream`) owns the video area: GPU frames
    /// (mpv idle or the last file) must not replace its picture.
    producer: AtomicBool,
    /// The display refresh rate interpolation paces to (UI thread sets it).
    refresh: Mutex<Option<f64>>,
}
impl Render {
    pub fn new(wanted: bool) -> Arc<Self> {
        Arc::new(Self {
            wanted,
            ..Self::default()
        })
    }
    pub fn wanted(&self) -> bool {
        self.wanted
    }
    /// The GPU renderer while it draws.
    pub fn gpu(&self) -> Option<&platform::GpuHandle> {
        self.active
            .load(Ordering::Acquire)
            .then(|| self.handle.get())
            .flatten()
    }
    pub fn install(&self, handle: platform::GpuHandle) {
        let _ = self.handle.set(handle);
        self.active.store(true, Ordering::Release);
    }
    pub fn deactivate(&self) {
        self.active.store(false, Ordering::Release);
    }
    /// Ask the render thread to switch to software rendering (once).
    pub fn fall_back(&self, reason: &str) {
        if self.active.load(Ordering::Acquire) && !self.fallback.swap(true, Ordering::AcqRel) {
            eprintln!("render: {reason}; switching to software rendering");
        }
    }
    pub fn fallback_requested(&self) -> bool {
        self.fallback.load(Ordering::Acquire)
    }
    pub fn set_producer(&self, on: bool) {
        self.producer.store(on, Ordering::Release);
    }
    pub fn producer(&self) -> bool {
        self.producer.load(Ordering::Acquire)
    }
    pub fn set_refresh(&self, fps: Option<f64>) {
        *self.refresh.lock().unwrap_or_else(PoisonError::into_inner) =
            fps.filter(|f| f.is_finite() && *f >= 1.);
    }
    /// The refresh rate for display-synced output (60 when unknown).
    pub fn refresh(&self) -> f64 {
        self.refresh
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .unwrap_or(60.)
    }
    pub fn backend_name(&self) -> String {
        if self.gpu().is_some() {
            gpu_name()
        } else {
            software_name()
        }
    }
}

/// GPU unless headless, `VESTRA_SOFTWARE_RENDER` is set (to anything but
/// `0` or empty), or this system has no GPU implementation.
pub fn wanted(headless: bool, software: Option<&OsStr>) -> bool {
    let software = software.is_some_and(|value| !value.is_empty() && value != "0");
    platform::GPU_RENDER && !headless && !software
}

/// mpv `hwdec` for a renderer: `VESTRA_HWDEC`, else `auto-safe` (mpv
/// falls back to software decoding by itself when no method works).
///
/// The software renderer (`RenderContext::new_software`, the only renderer
/// on systems without `GPU_RENDER`, e.g. Windows) has no hardware-decoding
/// interop, so it gets the copy-back variant: `auto-copy-safe` picks
/// `d3d11va-copy`, `nvdec-copy`, `videotoolbox-copy`… (decoding on the GPU,
/// frames copied to memory), which mpv documents as working "with all video
/// filters and VOs". The zero-copy methods `auto-safe` lists need a GPU VO
/// (mpv's `--hwdec` table); asking for copy-back directly does not depend on
/// mpv working that out. Up to 0.4.2 software rendering kept `hwdec=no`.
pub fn hwdec(env: Option<String>, software: bool) -> String {
    let mode = env
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "auto-safe".into());
    if software {
        crate::processing::copy_back(&mode).unwrap_or(mode)
    } else {
        mode
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn software_rendering_when_headless_asked_for_or_unavailable() {
        assert_eq!(wanted(false, None), platform::GPU_RENDER);
        assert!(!wanted(true, None), "headless");
        assert!(!wanted(false, Some(OsStr::new("1"))));
        assert_eq!(wanted(false, Some(OsStr::new("0"))), platform::GPU_RENDER);
        assert_eq!(wanted(false, Some(OsStr::new(""))), platform::GPU_RENDER);
    }

    #[test]
    fn hwdec_is_hardware_for_both_renderers_and_copy_back_for_software() {
        assert_eq!(hwdec(None, false), "auto-safe");
        assert_eq!(hwdec(Some(String::new()), false), "auto-safe");
        assert_eq!(hwdec(Some("no".into()), false), "no");
        assert_eq!(hwdec(Some("d3d11va".into()), false), "d3d11va");
        // Software rendering (Windows, headless, VESTRA_SOFTWARE_RENDER).
        assert_eq!(hwdec(None, true), "auto-copy-safe");
        assert_eq!(hwdec(Some(String::new()), true), "auto-copy-safe");
        assert_eq!(hwdec(Some("no".into()), true), "no");
        assert_eq!(hwdec(Some("d3d11va".into()), true), "d3d11va-copy");
        assert_eq!(hwdec(Some("nvdec-copy".into()), true), "nvdec-copy");
        assert_eq!(hwdec(Some("auto-copy".into()), true), "auto-copy");
    }

    /// What the backend starts with on this system before any GPU renderer:
    /// never `no` (that was the Windows default up to 0.4.2).
    #[test]
    fn the_initial_mode_decodes_in_hardware() {
        let mode = hwdec(None, true);
        assert_ne!(mode, "no");
        assert!(mode.contains("copy"), "{mode}");
    }

    #[test]
    fn a_fallback_needs_an_active_gpu_and_is_asked_once() {
        let render = Render::new(true);
        assert!(render.gpu().is_none());
        assert_eq!(render.backend_name(), software_name());
        render.fall_back("no GPU yet");
        assert!(!render.fallback_requested(), "nothing to fall back from");
        render.active.store(true, Ordering::Release);
        render.fall_back("window");
        render.fall_back("again");
        assert!(render.fallback_requested());
        render.deactivate();
        assert!(render.gpu().is_none());
        assert_eq!(render.refresh(), 60.);
        render.set_refresh(Some(120.));
        assert_eq!(render.refresh(), 120.);
        render.set_refresh(Some(f64::NAN));
        assert_eq!(render.refresh(), 60.);
    }
}
