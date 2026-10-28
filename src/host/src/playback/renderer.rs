//! Which renderer draws the video now (SDK site, Concepts: render options).
//! The start-up choice and its reason are `render_mode::decide`; GPU
//! rendering (libmpv on the GPU, through the platform module's
//! `GpuBackend`) then falls back to software rendering
//! (`RenderContext::new_software`) when it cannot start or a window cannot
//! show its frames, never back. `Render` keeps the reason of the last change.
#![forbid(unsafe_code)]

use crate::platform;
use crate::playback::render_mode::{Decision, Reason};
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

/// What the host asks of a GPU renderer's UI-side handle, on every system
/// (round record 2026-10-render-modes §2.2). Each platform module implements
/// it for its `platform::GpuHandle` (macOS: `vestra-gpu-render`; elsewhere
/// the inert `no_gpu` stub), so a missing or different method fails to
/// compile on every system. Statically dispatched: the host sees the handle
/// only through `Render::gpu`, as `impl GpuBackend`.
pub trait GpuBackend {
    /// A published frame (`latest`), for picture-in-picture.
    type Frame: std::ops::Deref<Target = platform::GpuFrame>;
    /// Render at `size` pixels from the next frame on.
    fn set_size(&self, size: (u32, u32));
    /// Pace to the display (`Some(fps)`, interpolation) or to the video.
    fn set_display_fps(&self, fps: Option<f64>);
    /// Frames rendered so far.
    fn frames(&self) -> u64;
    /// Frames a window showed so far.
    fn shown(&self) -> u64;
    /// The size of the newest frame.
    fn size(&self) -> Option<(u32, u32)>;
    /// The windows show the newest frame again (after a generated stream's
    /// picture replaced it).
    fn reshow(&self);
    /// The newest frame and its sequence number.
    fn latest(&self) -> Option<(u64, Self::Frame)>;
    /// Run `notify` (any thread) after each new frame.
    fn on_frame(&self, notify: impl Fn() + Send + 'static);
    /// Show the frames in `component`'s window: `show` sets each image,
    /// `failed` runs once if the window cannot show them, `drawn` brackets
    /// each draw.
    fn attach<T: slint::ComponentHandle + 'static>(
        &self,
        component: &T,
        show: impl Fn(slint::Image) + 'static,
        failed: impl FnOnce(String) + 'static,
        drawn: impl FnMut(platform::GpuDraw) + 'static,
    ) -> Result<(), String>;
    /// A new shell took the attached window's place (same native window).
    fn retarget<T: slint::ComponentHandle + 'static>(&self, component: &T);
}

/// Shared by the backend's render thread, its control worker and the UI.
pub struct Render {
    wanted: bool,
    /// Why the player renders as it does now (`render_mode::Reason`):
    /// `pending` until the render thread installs GPU rendering. Held while
    /// `active` changes, so `now` reads the two together.
    reason: Mutex<Reason>,
    /// The decision's reason, for when GPU rendering is installed.
    intended: Reason,
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
    /// What the main window really draws (`stats().presentedFps`).
    pub presentation: crate::playback::render_target::Presentation,
}
impl Render {
    /// From the start-up decision: the render thread tries GPU rendering
    /// first when the decision is GPU.
    pub fn new(decision: &Decision) -> Arc<Self> {
        Arc::new(Self {
            wanted: decision.is_gpu(),
            reason: Mutex::new(if decision.is_gpu() {
                Reason::Pending
            } else {
                decision.reason.clone()
            }),
            intended: decision.reason.clone(),
            handle: OnceLock::new(),
            active: AtomicBool::new(false),
            fallback: AtomicBool::new(false),
            producer: AtomicBool::new(false),
            refresh: Mutex::new(None),
            presentation: Default::default(),
        })
    }
    pub fn wanted(&self) -> bool {
        self.wanted
    }
    /// Why the player renders as it does now.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn reason(&self) -> Reason {
        self.now().1
    }
    /// Whether the GPU renderer draws now, and why, read together.
    pub fn now(&self) -> (bool, Reason) {
        let reason = self.reason.lock().unwrap_or_else(PoisonError::into_inner);
        (self.active.load(Ordering::Acquire), reason.clone())
    }
    /// The GPU renderer while it draws, seen through its contract only.
    pub fn gpu(&self) -> Option<&impl GpuBackend> {
        self.active
            .load(Ordering::Acquire)
            .then(|| self.handle.get())
            .flatten()
    }
    /// GPU rendering was wanted but could not start: the render thread
    /// renders in software from the start.
    pub fn start_failed(&self, reason: Reason) {
        diag!("render: {reason}; software rendering");
        *self.reason.lock().unwrap_or_else(PoisonError::into_inner) = reason;
    }
    pub fn install(&self, handle: platform::GpuHandle) {
        let _ = self.handle.set(handle);
        let mut reason = self.reason.lock().unwrap_or_else(PoisonError::into_inner);
        self.active.store(true, Ordering::Release);
        *reason = self.intended.clone();
    }
    pub fn deactivate(&self) {
        let _reason = self.reason.lock().unwrap_or_else(PoisonError::into_inner);
        self.active.store(false, Ordering::Release);
    }
    /// Ask the render thread to switch to software rendering (once).
    pub fn fall_back(&self, reason: Reason) {
        if self.active.load(Ordering::Acquire) && !self.fallback.swap(true, Ordering::AcqRel) {
            diag!("render: {reason}; switching to software rendering");
            *self.reason.lock().unwrap_or_else(PoisonError::into_inner) = reason;
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
        crate::playback::processing::copy_back(&mode).unwrap_or(mode)
    } else {
        mode
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        use crate::playback::render_mode::{Facts, Preference, decide};
        let facts = Facts {
            gpu_backend: true,
            ..Facts::default()
        };
        let render = Render::new(&decide(&facts, Preference::Auto));
        assert!(render.wanted());
        assert!(render.gpu().is_none());
        assert_eq!(render.backend_name(), software_name());
        // Not installed yet: software now, GPU starting.
        assert_eq!(render.now(), (false, Reason::Pending));
        render.fall_back(Reason::WindowFallback("no GPU yet".into()));
        assert!(!render.fallback_requested(), "nothing to fall back from");
        assert_eq!(render.reason(), Reason::Pending);
        render.active.store(true, Ordering::Release);
        *render.reason.lock().unwrap() = render.intended.clone();
        assert_eq!(render.now(), (true, Reason::Default));
        render.fall_back(Reason::WindowFallback("window".into()));
        render.fall_back(Reason::WindowFallback("again".into()));
        assert!(render.fallback_requested());
        assert_eq!(render.reason(), Reason::WindowFallback("window".into()));
        render.start_failed(Reason::InitFailed("context".into()));
        assert_eq!(render.reason().code(), "init-failed");
        render.deactivate();
        assert!(render.gpu().is_none());
        assert_eq!(render.refresh(), 60.);
        render.set_refresh(Some(120.));
        assert_eq!(render.refresh(), 120.);
        render.set_refresh(Some(f64::NAN));
        assert_eq!(render.refresh(), 60.);
    }
}
