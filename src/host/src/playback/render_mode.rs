//! Which renderer pair draws: the video renderer (libmpv on the GPU, or its
//! software renderer) and Slint's window renderer, decided once at start
//! from plain facts, with the reason (round record 2026-10-render-modes
//! §2.1). Pure: `main.rs` reads the environment and the platform constants
//! and passes them in; nothing here calls the system.
//!
//! The decision is the *intended* mode. What draws later is
//! `renderer::Render`, which can only go from GPU to CPU (`fall_back`) and
//! keeps the reason of its last change.
#![forbid(unsafe_code)]

use std::ffi::OsStr;
use std::fmt;

/// Who renders the video.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// libmpv's software renderer (`RenderContext::new_software`), hardware
    /// decoding with copy-back (`renderer::hwdec`).
    Cpu,
    /// libmpv renders on the GPU (`platform::gpu_start`) and the windows show
    /// its texture.
    Gpu,
}

/// Slint's window renderer, chosen together with the mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowKind {
    /// Slint's default renderer for the winit backend (CPU mode).
    Default,
    /// femtovg on OpenGL: what GPU mode's texture sharing needs.
    OpenGl,
}

/// Why the player renders the way it does: `render:` lines, and later
/// `SystemInfo` (M2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    /// Nothing rules GPU rendering out.
    Default,
    /// No windows (`--headless`).
    Headless,
    /// This system has no GPU renderer (`platform::GPU_RENDER`).
    NoGpuBackend,
    /// `VESTRA_SOFTWARE_RENDER` asks for software rendering.
    Env,
    /// GPU rendering could not start: the window renderer or libmpv's GPU
    /// context refused.
    InitFailed(String),
    /// A window could not show GPU frames (after start).
    WindowFallback(String),
}

impl Reason {
    /// A stable code (kebab case) for logs and, from M2, `SystemInfo`.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Headless => "headless",
            Self::NoGpuBackend => "no-gpu-backend",
            Self::Env => "env",
            Self::InitFailed(_) => "init-failed",
            Self::WindowFallback(_) => "window-fallback",
        }
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code = self.code();
        match self {
            Self::Default => write!(f, "{code}"),
            Self::Headless => write!(f, "{code} (no windows)"),
            Self::NoGpuBackend => write!(
                f,
                "{code} (no GPU rendering on {} yet)",
                std::env::consts::OS
            ),
            Self::Env => write!(f, "{code} (VESTRA_SOFTWARE_RENDER)"),
            Self::InitFailed(detail) | Self::WindowFallback(detail) => {
                write!(f, "{code} ({detail})")
            }
        }
    }
}

/// The start-up facts the decision reads.
#[derive(Clone, Copy, Debug)]
pub struct Inputs {
    pub headless: bool,
    /// `platform::GPU_RENDER`.
    pub gpu_backend: bool,
    /// `VESTRA_SOFTWARE_RENDER` set (`software_requested`).
    pub software_env: bool,
}

/// The renderer pair and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    pub mode: Mode,
    pub windows: WindowKind,
    pub reason: Reason,
}

impl Decision {
    fn cpu(reason: Reason) -> Self {
        Self {
            mode: Mode::Cpu,
            windows: WindowKind::Default,
            reason,
        }
    }
    pub fn gpu(&self) -> bool {
        self.mode == Mode::Gpu
    }
    /// GPU rendering could not start before the backend did (the window
    /// renderer refused): CPU mode with Slint's default windows.
    pub fn fall_back(self, reason: Reason) -> Self {
        Self::cpu(reason)
    }
    /// The start-up `render:` line.
    pub fn line(&self) -> String {
        let (mode, windows) = (
            match self.mode {
                Mode::Cpu => "software rendering",
                Mode::Gpu => "GPU rendering",
            },
            match self.windows {
                WindowKind::Default => "Slint's default window renderer",
                WindowKind::OpenGl => "OpenGL windows",
            },
        );
        format!("render: {mode}, {windows}; reason: {}", self.reason)
    }
}

/// First match wins: headless, no GPU renderer on this system, the
/// environment, else GPU.
pub fn decide(inputs: Inputs) -> Decision {
    if inputs.headless {
        Decision::cpu(Reason::Headless)
    } else if !inputs.gpu_backend {
        Decision::cpu(Reason::NoGpuBackend)
    } else if inputs.software_env {
        Decision::cpu(Reason::Env)
    } else {
        Decision {
            mode: Mode::Gpu,
            windows: WindowKind::OpenGl,
            reason: Reason::Default,
        }
    }
}

/// `VESTRA_SOFTWARE_RENDER` is set to anything but `0` or empty.
pub fn software_requested(value: Option<&OsStr>) -> bool {
    value.is_some_and(|value| !value.is_empty() && value != "0")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_matching_fact_decides() {
        use Mode::{Cpu, Gpu};
        use WindowKind::{Default, OpenGl};
        // (headless, GPU backend, software env) → (mode, windows, reason)
        let table = [
            ((false, true, false), (Gpu, OpenGl, Reason::Default)),
            ((true, true, false), (Cpu, Default, Reason::Headless)),
            ((true, false, true), (Cpu, Default, Reason::Headless)),
            ((false, false, false), (Cpu, Default, Reason::NoGpuBackend)),
            ((false, false, true), (Cpu, Default, Reason::NoGpuBackend)),
            ((false, true, true), (Cpu, Default, Reason::Env)),
        ];
        for ((headless, gpu_backend, software_env), (mode, windows, reason)) in table {
            let inputs = Inputs {
                headless,
                gpu_backend,
                software_env,
            };
            assert_eq!(
                decide(inputs),
                Decision {
                    mode,
                    windows,
                    reason
                },
                "{inputs:?}"
            );
        }
    }

    #[test]
    fn the_software_variable_counts_unless_empty_or_zero() {
        assert!(!software_requested(None));
        assert!(!software_requested(Some(OsStr::new(""))));
        assert!(!software_requested(Some(OsStr::new("0"))));
        assert!(software_requested(Some(OsStr::new("1"))));
        assert!(software_requested(Some(OsStr::new("yes"))));
    }

    /// The same as `renderer::wanted` before M1, for every input.
    #[test]
    fn gpu_exactly_when_it_was_wanted_before() {
        for headless in [false, true] {
            for gpu_backend in [false, true] {
                for software_env in [false, true] {
                    let decision = decide(Inputs {
                        headless,
                        gpu_backend,
                        software_env,
                    });
                    assert_eq!(decision.gpu(), gpu_backend && !headless && !software_env);
                    assert_eq!(decision.windows == WindowKind::OpenGl, decision.gpu());
                }
            }
        }
    }

    #[test]
    fn a_start_failure_keeps_its_detail_and_the_line_names_everything() {
        let gpu = decide(Inputs {
            headless: false,
            gpu_backend: true,
            software_env: false,
        });
        assert_eq!(
            gpu.line(),
            "render: GPU rendering, OpenGL windows; reason: default"
        );
        let cpu = gpu.fall_back(Reason::InitFailed("no OpenGL".into()));
        assert_eq!((cpu.mode, cpu.windows), (Mode::Cpu, WindowKind::Default));
        assert_eq!(
            cpu.line(),
            "render: software rendering, Slint's default window renderer; reason: init-failed (no OpenGL)"
        );
        assert_eq!(Reason::Env.to_string(), "env (VESTRA_SOFTWARE_RENDER)");
        assert_eq!(Reason::WindowFallback("x".into()).code(), "window-fallback");
    }
}
