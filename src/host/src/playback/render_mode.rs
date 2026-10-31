//! Which renderer pair draws: the video renderer (libmpv on the GPU, or its
//! software renderer) and Slint's window renderer, decided once at start
//! from plain facts, with the reason (round record 2026-10-render-modes
//! §2.1, §3). Pure: `main.rs` gathers the facts (environment, the saved
//! preference, the adapter, the driver denylist, the crash guard) and
//! passes them in; nothing here calls the system.
//!
//! The decision is the *intended* mode. What draws later is
//! `renderer::Render`, which can only go from GPU to CPU (`fall_back`) and
//! keeps the reason of its last change.
#![forbid(unsafe_code)]

use crate::platform::GpuAdapter;
use crate::playback::render_denylist::{self, Action, Denylist, WindowApi};
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

impl Mode {
    /// `SystemInfo.rendering` and `rendering.state`'s modes.
    pub fn name(self) -> &'static str {
        match self {
            Self::Cpu => "software",
            Self::Gpu => "gpu",
        }
    }
}

/// Slint's window renderer, chosen together with the mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowKind {
    /// Slint's default renderer for the winit backend (CPU mode).
    Default,
    /// femtovg on OpenGL: what GPU mode's texture sharing needs on macOS.
    OpenGl,
    /// Slint's software renderer: a driver whose OpenGL cannot draw the
    /// windows (denylist action `software-windows`); CPU mode only.
    Software,
    /// femtovg on wgpu (Direct3D 12): GPU mode's windows on Windows, which
    /// show libmpv's Direct3D 11 textures directly. Not the driver's OpenGL,
    /// so a `software-windows` rule does not touch them (record §8.7).
    Wgpu,
}

/// The user's choice (`render.json`, `rendering.setPreference`), applied at
/// the next start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Preference {
    /// The player decides: GPU where this system has a GPU renderer and
    /// nothing rules it out.
    #[default]
    Auto,
    /// GPU rendering where it is available.
    Gpu,
    /// Software rendering.
    Software,
}

impl Preference {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "auto" => Some(Self::Auto),
            "gpu" => Some(Self::Gpu),
            "software" => Some(Self::Software),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Gpu => "gpu",
            Self::Software => "software",
        }
    }
}

/// Why the player renders the way it does: `render:` lines,
/// `SystemInfo.renderingReason` and `rendering.state`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    /// Nothing rules GPU rendering out.
    Default,
    /// This system's GPU renderer is experimental and off until the user
    /// turns it on (Windows): the platform's default is software rendering.
    /// Code `default`, as the SDK documents it ("the platform's default").
    OffByDefault,
    /// GPU rendering is starting (the render thread has not installed it).
    Pending,
    /// No windows (`--headless`).
    Headless,
    /// This system has no GPU renderer (`platform::GPU_RENDER`).
    NoGpuBackend,
    /// An environment variable asks for it (its name).
    Env(&'static str),
    /// The user's preference.
    Preference,
    /// A denylist rule (`render_denylist`) holds for the graphics driver.
    Denylist { rule: String, detail: String },
    /// GPU rendering crashed at an earlier start (`render_state`).
    PreviousCrash(String),
    /// GPU rendering could not start: the window renderer or libmpv's GPU
    /// context refused.
    InitFailed(String),
    /// A window could not show GPU frames (after start).
    WindowFallback(String),
}

impl Reason {
    /// A stable code (kebab case).
    pub fn code(&self) -> &'static str {
        match self {
            Self::Default | Self::OffByDefault => "default",
            Self::Pending => "pending",
            Self::Headless => "headless",
            Self::NoGpuBackend => "no-gpu-backend",
            Self::Env(_) => "env",
            Self::Preference => "preference",
            Self::Denylist { .. } => "denylist",
            Self::PreviousCrash(_) => "previous-crash",
            Self::InitFailed(_) => "init-failed",
            Self::WindowFallback(_) => "window-fallback",
        }
    }
    /// The human text (English).
    pub fn message(&self) -> String {
        match self {
            Self::Default => "nothing rules GPU rendering out".into(),
            Self::OffByDefault => format!(
                "GPU rendering on {} is experimental and off until it is turned on",
                system_name(std::env::consts::OS)
            ),
            Self::Pending => "GPU rendering is starting".into(),
            Self::Headless => "the player runs without windows".into(),
            Self::NoGpuBackend => format!(
                "this player has no GPU renderer on {} yet",
                system_name(std::env::consts::OS)
            ),
            Self::Env(variable) => format!("{variable} is set"),
            Self::Preference => "chosen in the player's settings".into(),
            Self::Denylist { detail, .. }
            | Self::PreviousCrash(detail)
            | Self::InitFailed(detail)
            | Self::WindowFallback(detail) => detail.clone(),
        }
    }
    /// `{ code, message }` for the API.
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({"code":self.code(),"message":self.message()})
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.code(), self.message())
    }
}

/// `windows` → `Windows`, as people write it.
pub(crate) fn system_name(os: &str) -> &str {
    match os {
        "windows" => "Windows",
        "macos" => "macOS",
        "linux" => "Linux",
        other => other,
    }
}

/// Everything the decision reads, gathered once at start (`main.rs`) and
/// kept, so `rendering.state` can tell what the next start would choose.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    pub headless: bool,
    /// `platform::GPU_RENDER`.
    pub gpu_backend: bool,
    /// A mode the environment forces (`env_choice`), with the variable.
    pub env: Option<(Preference, &'static str)>,
    /// `std::env::consts::OS` and the system build.
    pub os: &'static str,
    pub build: Option<String>,
    /// The system's adapters in its order.
    pub adapters: Vec<GpuAdapter>,
    /// The index in `adapters` of the one GPU rendering (and Windows'
    /// Direct3D 12 windows) would use: 0, the first, unless
    /// `VESTRA_GPU_ADAPTER` chooses another; out of range: none
    /// (`platform::gpu_adapters`).
    pub render_adapter: usize,
    pub denylist: Denylist,
    /// The crash guard keeps GPU rendering off: why, with until when.
    pub crashed: Option<String>,
    /// `SLINT_BACKEND` is set: Slint picks the window renderer itself and
    /// a denylist rule does not change it.
    pub slint_backend: bool,
    /// The windows draw with wgpu (Direct3D 12) where they can
    /// (`platform::GPU_WINDOWS_WGPU`, Windows): GPU mode's always, CPU mode's
    /// unless a Direct3D 12 window rule or the crash guard holds (0.5.19);
    /// an OpenGL window rule then changes only CPU mode's fallback windows.
    pub wgpu_windows: bool,
    /// GPU rendering only when the user or the environment asks for it
    /// (`platform::GPU_OPT_IN`, Windows): `auto` resolves to software.
    pub gpu_opt_in: bool,
    /// `VESTRA_WINDOW_RENDERER=gl|software` (a test switch,
    /// `window_env_choice`): these windows, and software rendering where GPU
    /// mode needs other ones.
    pub window_env: Option<WindowKind>,
}

/// The renderer pair and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    pub mode: Mode,
    pub windows: WindowKind,
    /// CPU mode's windows (Windows: `Wgpu` where Direct3D 12 windows are
    /// allowed; `Software` under an OpenGL window rule): what a GPU start
    /// whose OpenGL windows fail falls back to.
    pub cpu_windows: WindowKind,
    /// CPU mode's windows when wgpu windows cannot be had (their device or
    /// Slint refused, or GPU mode's wgpu windows failed): never `Wgpu`.
    pub fallback_windows: WindowKind,
    pub reason: Reason,
    /// More `render:` lines: denylist rules that hold, overrides.
    pub notes: Vec<String>,
}

impl Decision {
    fn cpu(reason: Reason) -> Self {
        Self {
            mode: Mode::Cpu,
            windows: WindowKind::Default,
            cpu_windows: WindowKind::Default,
            fallback_windows: WindowKind::Default,
            reason,
            notes: Vec::new(),
        }
    }
    fn gpu(reason: Reason) -> Self {
        Self {
            mode: Mode::Gpu,
            windows: WindowKind::OpenGl,
            cpu_windows: WindowKind::Default,
            fallback_windows: WindowKind::Default,
            reason,
            notes: Vec::new(),
        }
    }
    pub fn is_gpu(&self) -> bool {
        self.mode == Mode::Gpu
    }
    /// GPU rendering could not start before the backend did (the window
    /// renderer refused): CPU mode with CPU mode's windows, but not the
    /// wgpu windows that just failed.
    pub fn fall_back(self, reason: Reason) -> Self {
        let windows = if self.windows == WindowKind::Wgpu {
            self.fallback_windows
        } else {
            self.cpu_windows
        };
        Self {
            windows,
            cpu_windows: windows,
            fallback_windows: self.fallback_windows,
            notes: self.notes,
            ..Self::cpu(reason)
        }
    }
    /// CPU mode's wgpu windows could not be had: its fallback windows (the
    /// mode and the reason stay).
    pub fn without_wgpu_windows(mut self) -> Self {
        if self.windows == WindowKind::Wgpu {
            self.windows = self.fallback_windows;
        }
        if self.cpu_windows == WindowKind::Wgpu {
            self.cpu_windows = self.fallback_windows;
        }
        self
    }
    /// Whether this run tries Direct3D 12 windows (GPU mode, or CPU mode's
    /// wgpu windows): the crash guard is armed for them.
    pub fn uses_wgpu_windows(&self) -> bool {
        self.windows == WindowKind::Wgpu
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
                WindowKind::Software => "Slint's software window renderer",
                WindowKind::Wgpu => "Direct3D 12 (wgpu) windows",
            },
        );
        format!("render: {mode}, {windows}; reason: {}", self.reason)
    }
}

/// A denylist rule that holds, and the adapter it holds for.
struct Hit<'a> {
    rule: &'a render_denylist::Rule,
    adapter: Option<&'a GpuAdapter>,
}

impl Facts {
    /// The adapter GPU rendering and Direct3D 12 windows would use.
    pub fn render_adapter(&self) -> Option<&GpuAdapter> {
        self.adapters.get(self.render_adapter)
    }
    /// Why the windows do not draw with Direct3D 12 where they could
    /// (Windows): a Direct3D 12 window rule, or the crash guard. For the
    /// `render:` notes and the software rendering reason (About).
    pub fn direct3d12_windows_off(&self) -> Option<String> {
        if !self.wgpu_windows {
            return None;
        }
        if let Some(hit) = self.window_rule(WindowApi::Direct3d12) {
            return Some(format!(
                "Direct3D 12 windows are off: denylist rule {}",
                hit.rule.id
            ));
        }
        self.crashed
            .as_ref()
            .map(|detail| format!("the crash guard keeps the windows off Direct3D 12 ({detail})"))
    }
    /// The API of GPU mode's windows.
    fn gpu_window_api(&self) -> WindowApi {
        if self.wgpu_windows {
            WindowApi::Direct3d12
        } else {
            WindowApi::Opengl
        }
    }
    /// What keeps GPU rendering off whatever the user prefers: the crash
    /// guard, then the first rule whose action is `cpu`, or a window rule
    /// for the windows GPU mode needs (OpenGL on macOS, Direct3D 12 on
    /// Windows; record §8.7, §11 item 2).
    fn blocked(&self) -> Option<Reason> {
        if let Some(detail) = &self.crashed {
            return Some(Reason::PreviousCrash(detail.clone()));
        }
        self.hits()
            .into_iter()
            .find(|hit| {
                hit.rule.action == Some(Action::Cpu)
                    || hit.rule.windows() == Some(self.gpu_window_api())
            })
            .map(|hit| self.denylist_reason(&hit))
    }
    /// The first window rule that holds for GPU mode's windows.
    fn gpu_window_rule(&self) -> Option<Hit<'_>> {
        self.window_rule(self.gpu_window_api())
    }
    /// The first `software-windows` rule that holds for windows of `api`.
    fn window_rule(&self, api: WindowApi) -> Option<Hit<'_>> {
        self.hits()
            .into_iter()
            .find(|hit| hit.rule.windows() == Some(api))
    }
    /// The rules that hold: every rule for the adapter GPU rendering would
    /// use (`render_adapter`; Direct3D 12 windows draw on it too); OpenGL
    /// window rules also for any other adapter with a GPU of its own,
    /// because OpenGL windows may draw on another GPU (a hybrid laptop's
    /// per-app choice).
    fn hits(&self) -> Vec<Hit<'_>> {
        let render = self.render_adapter();
        let matches = |adapter: Option<&GpuAdapter>| {
            self.denylist.matches(render_denylist::Facts {
                os: self.os,
                build: self.build.as_deref(),
                adapter,
            })
        };
        let mut hits: Vec<Hit<'_>> = matches(render)
            .into_iter()
            .map(|rule| Hit {
                rule,
                adapter: render,
            })
            .collect();
        let others = self
            .adapters
            .iter()
            .enumerate()
            .filter(|(index, adapter)| *index != self.render_adapter && !adapter.software);
        for (_, adapter) in others {
            for rule in matches(Some(adapter)) {
                if rule.windows() == Some(WindowApi::Opengl)
                    && !hits.iter().any(|hit| hit.rule.id == rule.id)
                {
                    hits.push(Hit {
                        rule,
                        adapter: Some(adapter),
                    });
                }
            }
        }
        hits
    }
    fn denylist_reason(&self, hit: &Hit<'_>) -> Reason {
        let hint = if self.os == "windows" {
            "; update the graphics driver, or on a computer with two GPUs choose the other one for Vestra in Windows Settings > System > Display > Graphics"
        } else {
            "; update the graphics driver"
        };
        Reason::Denylist {
            rule: hit.rule.id.clone(),
            detail: format!(
                "the graphics driver is on the player's denylist: {} ({}; rule {}){hint}",
                hit.rule.description,
                adapter_text(hit.adapter),
                hit.rule.id
            ),
        }
    }
    /// Whether `"gpu"` can be chosen now (ignoring the environment and the
    /// preference): `Err` with why not.
    pub fn gpu_available(&self) -> Result<(), Reason> {
        if self.headless {
            Err(Reason::Headless)
        } else if !self.gpu_backend {
            Err(Reason::NoGpuBackend)
        } else if let Some(reason) = self.blocked() {
            Err(reason)
        } else {
            Ok(())
        }
    }
}

/// `name, 1002:15BF, driver x` for logs and messages.
fn adapter_text(adapter: Option<&GpuAdapter>) -> String {
    adapter.map_or_else(
        || "unknown adapter".into(),
        |adapter| {
            format!(
                "{}, {:04X}:{:04X}, driver {}",
                adapter.name,
                adapter.vendor_id,
                adapter.device_id,
                adapter.driver_version.as_deref().unwrap_or("unknown")
            )
        },
    )
}

/// First match wins: headless, no GPU renderer on this system, the
/// environment, the preference `software`, the crash guard, a denylist rule,
/// the preference `auto` where GPU rendering is opt-in (Windows: software),
/// else GPU. Denylist rules that hold are noted even when something earlier
/// decided. Then the windows ([`windows`]): a window rule for the API GPU
/// mode's windows need rules GPU mode out (record §8.7, §11 item 2); an
/// OpenGL window rule gives CPU mode Slint's software renderer where it
/// would use OpenGL, unless `SLINT_BACKEND` is set; on Windows CPU mode
/// draws in Direct3D 12 (wgpu) windows unless a Direct3D 12 rule or the
/// crash guard holds (§11 item 5).
pub fn decide(facts: &Facts, preference: Preference) -> Decision {
    let blocked = facts.blocked();
    let mut decision = if facts.headless {
        Decision::cpu(Reason::Headless)
    } else if !facts.gpu_backend {
        Decision::cpu(Reason::NoGpuBackend)
    } else {
        match facts.env {
            Some((Preference::Software, variable)) => Decision::cpu(Reason::Env(variable)),
            // It overrides the crash guard and `cpu` rules, not a driver
            // that cannot draw OpenGL windows.
            Some((Preference::Gpu, variable)) => match facts.gpu_window_rule() {
                Some(hit) => {
                    let mut decision = Decision::cpu(facts.denylist_reason(&hit));
                    decision.notes.push(format!(
                        "render: {variable}=gpu does not override denylist rule {}",
                        hit.rule.id
                    ));
                    decision
                }
                None => {
                    let mut decision = Decision::gpu(Reason::Env(variable));
                    if let Some(reason) = &blocked {
                        decision
                            .notes
                            .push(format!("render: {variable} overrides {reason}"));
                    }
                    decision
                }
            },
            _ if preference == Preference::Software => Decision::cpu(Reason::Preference),
            // GPU rendering is off until it is turned on: the driver check
            // (crash guard, denylist) runs only then (user ruling, M3).
            _ if facts.gpu_opt_in && preference == Preference::Auto => {
                Decision::cpu(Reason::OffByDefault)
            }
            _ => blocked.map_or_else(|| Decision::gpu(Reason::Default), Decision::cpu),
        }
    };
    if !facts.headless {
        windows(facts, &mut decision);
    }
    // Off by default: only a rule that changes the windows matters.
    let off = decision.reason == Reason::OffByDefault;
    for Hit { rule, adapter } in facts.hits() {
        if off && rule.action != Some(Action::SoftwareWindows) {
            continue;
        }
        let action = match (rule.action, rule.windows()) {
            (Some(Action::Cpu), _) => "software rendering",
            (_, Some(WindowApi::Direct3d12)) => {
                "no Direct3D 12 windows: OpenGL ones, and software rendering"
            }
            (_, Some(WindowApi::Opengl)) if facts.wgpu_windows => {
                "no OpenGL windows: Slint's software renderer where Direct3D 12 windows cannot be had"
            }
            (_, Some(WindowApi::Opengl)) => "software rendering and software windows",
            _ => "a note only",
        };
        decision.notes.push(format!(
            "render: denylist rule {} holds for {} ({action}): {}",
            rule.id,
            adapter_text(adapter),
            rule.description
        ));
    }
    decision
}

/// The windows of `decision`'s mode (not headless). GPU mode: wgpu ones on
/// Windows, OpenGL ones elsewhere. CPU mode (0.5.19): on Windows wgpu
/// (Direct3D 12) windows, with software video uploaded into them once per
/// frame and presented with the flip model and vsync, unless a Direct3D 12
/// window rule or the crash guard holds; else (and when they cannot be
/// had, `fallback_windows`) femtovg on OpenGL, named explicitly (with wgpu
/// compiled in, Slint's default would be femtovg-wgpu), or Slint's software
/// renderer under an OpenGL window rule; elsewhere Slint's default.
/// `SLINT_BACKEND` leaves CPU mode's windows to Slint (an explicit renderer
/// would win over it); `VESTRA_WINDOW_RENDERER=gl|software` chooses them,
/// and makes GPU mode software where it needs other ones.
fn windows(facts: &Facts, decision: &mut Decision) {
    let gl_rule = facts.window_rule(WindowApi::Opengl);
    let wgpu_allowed = facts.wgpu_windows
        && facts.window_rule(WindowApi::Direct3d12).is_none()
        && facts.crashed.is_none();
    let fallback = if facts.slint_backend {
        WindowKind::Default
    } else if gl_rule.is_some() {
        WindowKind::Software
    } else if facts.wgpu_windows {
        WindowKind::OpenGl
    } else {
        WindowKind::Default
    };
    let cpu = if wgpu_allowed && !facts.slint_backend {
        WindowKind::Wgpu
    } else {
        fallback
    };
    if let Some(Hit { rule, .. }) = &gl_rule
        && facts.slint_backend
    {
        decision.notes.push(format!(
            "render: SLINT_BACKEND is set; Slint chooses CPU mode's windows (denylist rule {} not applied to them)",
            rule.id
        ));
    }
    let gpu_windows = if facts.wgpu_windows {
        WindowKind::Wgpu
    } else {
        WindowKind::OpenGl
    };
    if decision.mode == Mode::Gpu && facts.wgpu_windows && facts.slint_backend {
        decision.notes.push(
            "render: SLINT_BACKEND is set; GPU rendering needs Direct3D 12 (wgpu) windows, so it is not applied to them".into(),
        );
    }
    (decision.cpu_windows, decision.fallback_windows) = (cpu, fallback);
    decision.windows = match decision.mode {
        Mode::Gpu => gpu_windows,
        Mode::Cpu => cpu,
    };
    if let Some(kind) = facts.window_env {
        if decision.mode == Mode::Gpu && kind != gpu_windows {
            *decision = Decision {
                notes: std::mem::take(&mut decision.notes),
                ..Decision::cpu(Reason::Env(WINDOW_RENDERER))
            };
        }
        if decision.mode == Mode::Cpu {
            decision.windows = kind;
        }
        // The fallback is never wgpu windows (the ones that failed).
        let kind_fallback = if kind == WindowKind::Wgpu {
            fallback
        } else {
            kind
        };
        (decision.cpu_windows, decision.fallback_windows) = (kind, kind_fallback);
    }
    // Why CPU mode left Direct3D 12 windows, where the crash guard did it
    // (a denylist rule has its own note below).
    if decision.mode == Mode::Cpu
        && decision.windows != WindowKind::Wgpu
        && facts.window_env.is_none()
        && !facts.slint_backend
        && facts.window_rule(WindowApi::Direct3d12).is_none()
        && let Some(why) = facts.direct3d12_windows_off()
    {
        decision.notes.push(format!("render: {why}"));
    }
}

const WINDOW_RENDERER: &str = "VESTRA_WINDOW_RENDERER";

/// `VESTRA_WINDOW_RENDERER` (testing the window fallbacks): `gl` (femtovg on
/// OpenGL) or `software` (Slint's software renderer) choose the windows;
/// `wgpu` and empty choose nothing here (`wgpu` lets Windows' wgpu windows
/// use an adapter without a GPU, `platform::select_wgpu_windows`); anything
/// else is a warning.
pub fn window_env_choice(value: Option<&OsStr>) -> (Option<WindowKind>, Option<String>) {
    match value
        .map(|value| value.to_string_lossy())
        .as_deref()
        .map(str::trim)
    {
        None | Some("" | "wgpu") => (None, None),
        Some("gl") => (Some(WindowKind::OpenGl), None),
        Some("software") => (Some(WindowKind::Software), None),
        Some(other) => (
            None,
            Some(format!(
                "render: {WINDOW_RENDERER}={other} is not gl, software or wgpu; ignored"
            )),
        ),
    }
}

/// `VESTRA_RENDER` (`cpu`/`software`, `gpu`, `auto`/empty) and its older
/// alias `VESTRA_SOFTWARE_RENDER` (anything but `0` or empty): the mode the
/// environment forces, and a warning for a value it does not know.
pub fn env_choice(
    render: Option<&OsStr>,
    software: Option<&OsStr>,
) -> (Option<(Preference, &'static str)>, Option<String>) {
    const RENDER: &str = "VESTRA_RENDER";
    let (choice, warning) = match render.map(|value| value.to_string_lossy()) {
        None => (None, None),
        Some(value) => match value.trim() {
            "" | "auto" => (None, None),
            "cpu" | "software" => (Some((Preference::Software, RENDER)), None),
            "gpu" => (Some((Preference::Gpu, RENDER)), None),
            other => (
                None,
                Some(format!(
                    "render: {RENDER}={other} is not cpu, software, gpu or auto; ignored"
                )),
            ),
        },
    };
    let alias = software.is_some_and(|value| !value.is_empty() && value != "0");
    let choice =
        choice.or_else(|| alias.then_some((Preference::Software, "VESTRA_SOFTWARE_RENDER")));
    (choice, warning)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(headless: bool, gpu_backend: bool) -> Facts {
        Facts {
            headless,
            gpu_backend,
            os: "windows",
            denylist: Denylist::built_in(),
            ..Facts::default()
        }
    }
    fn bad_amd() -> GpuAdapter {
        GpuAdapter {
            name: "AMD Radeon 780M".into(),
            vendor_id: 0x1002,
            device_id: 0x15BF,
            driver_version: Some("31.0.14005.21001".into()),
            software: false,
            luid: 0,
        }
    }
    const SOFTWARE_ENV: Option<(Preference, &str)> =
        Some((Preference::Software, "VESTRA_SOFTWARE_RENDER"));

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
            (
                (false, true, true),
                (Cpu, Default, Reason::Env("VESTRA_SOFTWARE_RENDER")),
            ),
        ];
        for ((headless, gpu_backend, software_env), (mode, windows, reason)) in table {
            let mut facts = facts(headless, gpu_backend);
            facts.env = software_env.then_some(SOFTWARE_ENV).flatten();
            let decision = decide(&facts, Preference::Auto);
            assert_eq!(
                (decision.mode, decision.windows, decision.reason),
                (mode, windows, reason),
                "{headless} {gpu_backend} {software_env}"
            );
        }
    }

    #[test]
    fn the_preference_the_guard_and_the_denylist_come_after_the_environment() {
        let mut gpu = facts(false, true);
        assert!(decide(&gpu, Preference::Gpu).is_gpu());
        assert_eq!(
            decide(&gpu, Preference::Software).reason,
            Reason::Preference
        );
        gpu.adapters = vec![bad_amd()];
        let denied = decide(&gpu, Preference::Gpu);
        assert_eq!(denied.mode, Mode::Cpu);
        assert_eq!(denied.reason.code(), "denylist");
        let message = denied.reason.message();
        assert!(message.contains("amd-780m-draw-stalls"), "{message}");
        assert!(message.contains("AMD Radeon 780M, 1002:15BF, driver 31.0.14005.21001"));
        assert!(message.contains("Display > Graphics"), "the Windows hint");
        assert_eq!(gpu.gpu_available().unwrap_err().code(), "denylist");
        // The environment overrides it, with a note.
        gpu.env = Some((Preference::Gpu, "VESTRA_RENDER"));
        let forced = decide(&gpu, Preference::Software);
        assert_eq!(forced.reason, Reason::Env("VESTRA_RENDER"));
        assert!(forced.notes[0].contains("VESTRA_RENDER overrides denylist"));
        // The crash guard comes before the denylist.
        gpu.env = None;
        gpu.crashed = Some("crashed before".into());
        assert_eq!(
            decide(&gpu, Preference::Auto).reason,
            Reason::PreviousCrash("crashed before".into())
        );
        assert_eq!(gpu.gpu_available().unwrap_err().code(), "previous-crash");
    }

    /// Windows on main: no GPU renderer, but a rule that holds is reported.
    #[test]
    fn without_a_gpu_renderer_a_holding_rule_is_only_reported() {
        let mut windows = facts(false, false);
        windows.adapters = vec![bad_amd()];
        let decision = decide(&windows, Preference::Gpu);
        assert_eq!(decision.reason, Reason::NoGpuBackend);
        assert_eq!(decision.notes.len(), 1);
        assert!(decision.notes[0].starts_with("render: denylist rule amd-780m-draw-stalls holds"));
        assert_eq!(windows.gpu_available(), Err(Reason::NoGpuBackend));
        assert_eq!(
            Reason::NoGpuBackend.message(),
            format!(
                "this player has no GPU renderer on {} yet",
                system_name(std::env::consts::OS)
            )
        );
        assert_eq!(system_name("windows"), "Windows");
        assert_eq!(system_name("macos"), "macOS");
    }

    /// Adrenalin 26.10.1 on the 780M: white OpenGL windows (round record §9).
    #[test]
    fn a_software_windows_rule_changes_the_windows_of_cpu_mode_unless_slint_backend_is_set() {
        let white = GpuAdapter {
            driver_version: Some("32.0.32015.7029".into()),
            ..bad_amd()
        };
        // Windows on main: no GPU renderer, the rule still changes the windows.
        let mut windows = facts(false, false);
        windows.adapters = vec![white.clone()];
        let decision = decide(&windows, Preference::Auto);
        assert_eq!(
            (decision.mode, decision.windows, decision.reason.clone()),
            (Mode::Cpu, WindowKind::Software, Reason::NoGpuBackend)
        );
        assert!(decision.line().contains("Slint's software window renderer"));
        assert!(decision.notes[0].contains("amd-780m-white-opengl-windows"));
        // With a GPU renderer it also rules GPU out.
        let mut gpu = facts(false, true);
        gpu.adapters = vec![white.clone()];
        let decision = decide(&gpu, Preference::Gpu);
        assert_eq!(
            (decision.mode, decision.windows),
            (Mode::Cpu, WindowKind::Software)
        );
        assert_eq!(decision.reason.code(), "denylist");
        assert_eq!(gpu.gpu_available().unwrap_err().code(), "denylist");
        // SLINT_BACKEND wins; headless has no windows.
        windows.slint_backend = true;
        let decision = decide(&windows, Preference::Auto);
        assert_eq!(decision.windows, WindowKind::Default);
        assert!(decision.notes[0].contains("SLINT_BACKEND is set"));
        let mut headless = facts(true, false);
        headless.adapters = vec![white];
        assert_eq!(
            decide(&headless, Preference::Auto).windows,
            WindowKind::Default
        );
    }

    /// A hybrid laptop with Vestra set to "High performance": the RTX 4070
    /// comes first, but OpenGL windows may still draw on the 780M.
    #[test]
    fn a_window_rule_holds_for_any_gpu_and_the_environment_cannot_override_it() {
        let nvidia = GpuAdapter {
            name: "NVIDIA GeForce RTX 4070 Laptop GPU".into(),
            vendor_id: 0x10DE,
            device_id: 0x2820,
            driver_version: Some("32.0.16.1062".into()),
            software: false,
            luid: 0,
        };
        let white = GpuAdapter {
            driver_version: Some("32.0.32015.7029".into()),
            ..bad_amd()
        };
        let mut hybrid = facts(false, true);
        hybrid.adapters = vec![nvidia.clone(), white];
        let decision = decide(&hybrid, Preference::Auto);
        assert_eq!(
            (decision.mode, decision.windows),
            (Mode::Cpu, WindowKind::Software)
        );
        assert!(decision.reason.message().contains("AMD Radeon 780M"));
        // A `cpu` rule counts for the first adapter only.
        hybrid.adapters = vec![nvidia, bad_amd()];
        assert!(decide(&hybrid, Preference::Auto).is_gpu());
        // A software adapter second is not looked at.
        let mut basic = facts(false, true);
        basic.adapters = vec![
            bad_amd(),
            GpuAdapter {
                software: true,
                driver_version: Some("32.0.32015.7029".into()),
                ..bad_amd()
            },
        ];
        assert_eq!(
            decide(&basic, Preference::Auto).windows,
            WindowKind::Default
        );
        // VESTRA_RENDER=gpu overrides a `cpu` rule, not a window rule.
        basic.env = Some((Preference::Gpu, "VESTRA_RENDER"));
        assert!(decide(&basic, Preference::Auto).is_gpu());
        let mut white_only = facts(false, true);
        white_only.adapters = vec![GpuAdapter {
            driver_version: Some("32.0.32015.7029".into()),
            ..bad_amd()
        }];
        white_only.env = Some((Preference::Gpu, "VESTRA_RENDER"));
        let decision = decide(&white_only, Preference::Auto);
        assert_eq!(
            (decision.mode, decision.windows),
            (Mode::Cpu, WindowKind::Software)
        );
        assert!(decision.notes[0].contains("does not override"));
    }

    /// M3 (record §4, §8.7): Windows has wgpu GPU windows and GPU rendering
    /// only on request.
    fn windows_gpu() -> Facts {
        Facts {
            wgpu_windows: true,
            gpu_opt_in: true,
            ..facts(false, true)
        }
    }
    fn white_amd() -> GpuAdapter {
        GpuAdapter {
            driver_version: Some("32.0.32015.7029".into()),
            ..bad_amd()
        }
    }

    #[test]
    fn on_windows_gpu_rendering_is_opt_in_and_draws_with_wgpu_windows() {
        let windows = windows_gpu();
        let auto = decide(&windows, Preference::Auto);
        assert_eq!(
            (auto.mode, auto.windows, auto.reason.clone()),
            (Mode::Cpu, WindowKind::Wgpu, Reason::OffByDefault)
        );
        assert_eq!(auto.fallback_windows, WindowKind::OpenGl);
        assert!(
            auto.line()
                .contains("software rendering, Direct3D 12 (wgpu) windows")
        );
        assert_eq!(auto.reason.code(), "default", "the SDK's code");
        assert!(auto.reason.message().contains("experimental"));
        assert_eq!(windows.gpu_available(), Ok(()), "the switch can turn it on");
        let chosen = decide(&windows, Preference::Gpu);
        assert_eq!(
            (chosen.mode, chosen.windows, chosen.reason.clone()),
            (Mode::Gpu, WindowKind::Wgpu, Reason::Default)
        );
        assert_eq!(chosen.cpu_windows, WindowKind::Wgpu);
        assert_eq!(
            chosen
                .clone()
                .fall_back(Reason::InitFailed("no Direct3D 12".into()))
                .windows,
            WindowKind::OpenGl
        );
        assert!(chosen.line().contains("Direct3D 12 (wgpu) windows"));
        let mut env = windows_gpu();
        env.env = Some((Preference::Gpu, "VESTRA_RENDER"));
        let forced = decide(&env, Preference::Auto);
        assert_eq!(
            (forced.mode, forced.windows, forced.reason.clone()),
            (Mode::Gpu, WindowKind::Wgpu, Reason::Env("VESTRA_RENDER"))
        );
        assert_eq!(
            decide(&windows, Preference::Software).reason,
            Reason::Preference
        );
        // The crash guard (a crash or the second device loss) still wins
        // once GPU rendering is turned on.
        let mut crashed = windows_gpu();
        crashed.crashed = Some("lost twice".into());
        assert_eq!(
            decide(&crashed, Preference::Gpu).reason.code(),
            "previous-crash"
        );
        // Off by default, the driver check does not run: neither the guard
        // nor a `cpu` rule is the reason, and the rule is not logged.
        assert_eq!(
            decide(&crashed, Preference::Auto).reason,
            Reason::OffByDefault
        );
        let mut stalls = windows_gpu();
        stalls.adapters = vec![bad_amd()];
        let auto = decide(&stalls, Preference::Auto);
        assert_eq!(
            (auto.windows, auto.reason),
            (WindowKind::Wgpu, Reason::OffByDefault)
        );
        // After a crash or the second device loss CPU mode keeps away from
        // Direct3D 12 too, and a note says why.
        let off = decide(&crashed, Preference::Auto);
        assert_eq!(off.windows, WindowKind::OpenGl);
        assert!(
            off.notes
                .iter()
                .any(|note| note.contains("crash guard keeps the windows off Direct3D 12")),
            "{:?}",
            off.notes
        );
        assert!(auto.notes.is_empty(), "{:?}", auto.notes);
        assert_eq!(stalls.gpu_available().unwrap_err().code(), "denylist");
    }

    /// N2 (M3 re-review): `SLINT_BACKEND` leaves CPU mode's windows to Slint
    /// on Windows too (`WindowKind::Default`, `winit().select()`), with or
    /// without a `software-windows` rule; the lines say so.
    #[test]
    fn slint_backend_leaves_cpu_windows_to_slint_on_windows() {
        let mut windows = windows_gpu();
        windows.slint_backend = true;
        let auto = decide(&windows, Preference::Auto);
        assert_eq!(
            (auto.mode, auto.windows, auto.cpu_windows),
            (Mode::Cpu, WindowKind::Default, WindowKind::Default)
        );
        assert!(auto.notes.is_empty(), "{:?}", auto.notes);
        windows.adapters = vec![white_amd()];
        let white = decide(&windows, Preference::Auto);
        assert_eq!(white.windows, WindowKind::Default);
        assert!(
            white.notes[0].contains("Slint chooses CPU mode's windows"),
            "{:?}",
            white.notes
        );
        let gpu = decide(&windows, Preference::Gpu);
        assert_eq!(
            (gpu.windows, gpu.cpu_windows),
            (WindowKind::Wgpu, WindowKind::Default)
        );
        assert!(
            gpu.notes
                .iter()
                .any(|note| note.contains("not applied to them") && note.contains("wgpu"))
        );
        assert_eq!(
            gpu.fall_back(Reason::InitFailed("x".into())).windows,
            WindowKind::Default
        );
    }

    /// H1 (M3 review), item 5 (record §11): on Windows CPU mode draws in
    /// wgpu windows exactly when nothing keeps it from Direct3D 12; its
    /// fallback windows, and a failed GPU start's, are never wgpu nor
    /// Slint's default (which would be femtovg-wgpu there).
    #[test]
    fn cpu_mode_uses_wgpu_windows_unless_something_keeps_it_from_direct3d_12() {
        let adapters = [vec![], vec![bad_amd()], vec![white_amd()]];
        let envs = [None, SOFTWARE_ENV, Some((Preference::Gpu, "VESTRA_RENDER"))];
        let window_envs = [None, Some(WindowKind::OpenGl), Some(WindowKind::Software)];
        for adapters in &adapters {
            for env in envs {
                for window_env in window_envs {
                    for crashed in [None, Some("crashed".to_string())] {
                        for preference in [Preference::Auto, Preference::Gpu, Preference::Software]
                        {
                            let facts = Facts {
                                adapters: adapters.clone(),
                                env,
                                window_env,
                                crashed: crashed.clone(),
                                ..windows_gpu()
                            };
                            let decision = decide(&facts, preference);
                            let fallback = [WindowKind::OpenGl, WindowKind::Software];
                            assert!(
                                fallback.contains(&decision.fallback_windows),
                                "{decision:?}"
                            );
                            if decision.mode == Mode::Cpu {
                                let wgpu = window_env.is_none() && crashed.is_none();
                                let expected = if wgpu {
                                    WindowKind::Wgpu
                                } else {
                                    decision.fallback_windows
                                };
                                assert_eq!(decision.windows, expected, "{decision:?}");
                                let failed = decision.without_wgpu_windows();
                                assert!(fallback.contains(&failed.windows), "{failed:?}");
                            } else {
                                assert_eq!(decision.windows, WindowKind::Wgpu);
                                let failed = decision.fall_back(Reason::InitFailed("x".into()));
                                assert!(fallback.contains(&failed.windows), "{failed:?}");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn with_wgpu_windows_a_software_windows_rule_changes_only_cpu_windows() {
        let mut windows = windows_gpu();
        windows.adapters = vec![white_amd()];
        assert_eq!(windows.gpu_available(), Ok(()), "§8.7: not ruled out");
        let gpu = decide(&windows, Preference::Gpu);
        assert_eq!(
            (gpu.mode, gpu.windows, gpu.cpu_windows, gpu.fallback_windows),
            (
                Mode::Gpu,
                WindowKind::Wgpu,
                WindowKind::Wgpu,
                WindowKind::Software
            )
        );
        // A GPU start that fails falls back to software windows.
        let failed = gpu.fall_back(Reason::InitFailed("no Direct3D 12".into()));
        assert_eq!(
            (failed.mode, failed.windows),
            (Mode::Cpu, WindowKind::Software)
        );
        // Auto: CPU mode in wgpu windows; Slint's software renderer only
        // where Direct3D 12 windows cannot be had (the rule stays meaningful).
        let auto = decide(&windows, Preference::Auto);
        assert_eq!(
            (auto.mode, auto.windows, auto.reason.clone()),
            (Mode::Cpu, WindowKind::Wgpu, Reason::OffByDefault)
        );
        assert_eq!(auto.without_wgpu_windows().windows, WindowKind::Software);
        // VESTRA_RENDER=gpu works with it too.
        windows.env = Some((Preference::Gpu, "VESTRA_RENDER"));
        assert!(decide(&windows, Preference::Auto).is_gpu());
        // The 31.0.14005 draw stalls stay a `cpu` rule.
        let mut stalls = windows_gpu();
        stalls.adapters = vec![bad_amd()];
        let denied = decide(&stalls, Preference::Gpu);
        assert_eq!((denied.mode, denied.windows), (Mode::Cpu, WindowKind::Wgpu));
        assert_eq!(denied.reason.code(), "denylist");
        assert_eq!(stalls.gpu_available().unwrap_err().code(), "denylist");
    }

    /// Item 2 (record §11): a rule for Direct3D 12 windows rules GPU mode
    /// out on Windows (even `VESTRA_RENDER=gpu`) and keeps CPU mode on
    /// OpenGL windows; it is matched against the adapter GPU rendering uses.
    #[test]
    fn a_direct3d_12_window_rule_keeps_both_modes_off_wgpu_windows() {
        let denylist = Denylist::built_in()
            .merged(
                r#"{"version":1,"rules":[{"id":"d3d12-test","description":"test",
                    "action":"software-windows","window_api":"direct3d12",
                    "vendor_id":"0x10de","device_id":["0x2820"]}]}"#,
            )
            .unwrap();
        let nvidia = GpuAdapter {
            name: "NVIDIA GeForce RTX 4070 Laptop GPU".into(),
            vendor_id: 0x10DE,
            device_id: 0x2820,
            driver_version: Some("32.0.16.1062".into()),
            ..GpuAdapter::default()
        };
        let amd = GpuAdapter {
            driver_version: Some("32.0.21000.1".into()),
            ..bad_amd()
        };
        let mut windows = Facts {
            denylist,
            adapters: vec![nvidia, amd],
            ..windows_gpu()
        };
        let auto = decide(&windows, Preference::Auto);
        assert_eq!(
            (auto.mode, auto.windows, auto.reason.clone()),
            (Mode::Cpu, WindowKind::OpenGl, Reason::OffByDefault)
        );
        assert!(
            auto.notes
                .iter()
                .any(|note| note.contains("d3d12-test") && note.contains("no Direct3D 12 windows")),
            "{:?}",
            auto.notes
        );
        let gpu = decide(&windows, Preference::Gpu);
        assert_eq!((gpu.mode, gpu.windows), (Mode::Cpu, WindowKind::OpenGl));
        assert_eq!(gpu.reason.code(), "denylist");
        assert_eq!(windows.gpu_available().unwrap_err().code(), "denylist");
        windows.env = Some((Preference::Gpu, "VESTRA_RENDER"));
        let forced = decide(&windows, Preference::Auto);
        assert_eq!(forced.mode, Mode::Cpu);
        assert!(
            forced.notes[0].contains("does not override"),
            "{:?}",
            forced.notes
        );
        // The windows draw on the AMD GPU (`VESTRA_GPU_ADAPTER=1`): the
        // NVIDIA rule does not hold for them.
        windows.env = None;
        windows.render_adapter = 1;
        assert_eq!(decide(&windows, Preference::Gpu).windows, WindowKind::Wgpu);
        assert_eq!(decide(&windows, Preference::Auto).windows, WindowKind::Wgpu);
        // macOS: a Direct3D 12 rule means nothing to OpenGL windows.
        let mac = Facts {
            os: "macos",
            denylist: windows.denylist.clone(),
            adapters: windows.adapters.clone(),
            ..facts(false, true)
        };
        assert!(decide(&mac, Preference::Auto).is_gpu());
    }

    /// L1 (record §11): `cpu` rules hold for the adapter GPU rendering uses
    /// (`VESTRA_GPU_ADAPTER`), not for the first one.
    #[test]
    fn a_cpu_rule_holds_for_the_adapter_gpu_rendering_uses() {
        let nvidia = GpuAdapter {
            name: "NVIDIA".into(),
            vendor_id: 0x10DE,
            device_id: 0x2820,
            ..GpuAdapter::default()
        };
        let mut hybrid = Facts {
            adapters: vec![nvidia, bad_amd()],
            ..windows_gpu()
        };
        assert!(
            decide(&hybrid, Preference::Gpu).is_gpu(),
            "the first: NVIDIA"
        );
        hybrid.render_adapter = 1;
        let denied = decide(&hybrid, Preference::Gpu);
        assert_eq!(denied.reason.code(), "denylist");
        assert!(denied.reason.message().contains("AMD Radeon 780M"));
        // No adapter (an index past the list): no rule holds.
        hybrid.render_adapter = 9;
        assert!(decide(&hybrid, Preference::Gpu).is_gpu());
    }

    #[test]
    fn the_window_renderer_switch_chooses_the_windows_and_software_where_it_must() {
        let os = |text: &'static str| Some(OsStr::new(text));
        assert_eq!(window_env_choice(None), (None, None));
        assert_eq!(window_env_choice(os(" wgpu ")), (None, None));
        assert_eq!(window_env_choice(os("gl")).0, Some(WindowKind::OpenGl));
        assert_eq!(
            window_env_choice(os("software")).0,
            Some(WindowKind::Software)
        );
        // Even forced wgpu windows never fall back to wgpu ones.
        let mut forced = windows_gpu();
        forced.window_env = Some(WindowKind::Wgpu);
        let wgpu = decide(&forced, Preference::Auto);
        assert_eq!(
            (wgpu.windows, wgpu.fallback_windows),
            (WindowKind::Wgpu, WindowKind::OpenGl)
        );
        let (choice, warning) = window_env_choice(os("vulkan"));
        assert_eq!(choice, None);
        assert!(warning.unwrap().contains("VESTRA_WINDOW_RENDERER=vulkan"));
        let mut windows = windows_gpu();
        windows.window_env = Some(WindowKind::OpenGl);
        let gl = decide(&windows, Preference::Gpu);
        assert_eq!(
            (gl.mode, gl.windows, gl.reason),
            (
                Mode::Cpu,
                WindowKind::OpenGl,
                Reason::Env("VESTRA_WINDOW_RENDERER")
            )
        );
        // macOS: OpenGL windows are GPU mode's own.
        let mut mac = facts(false, true);
        mac.window_env = Some(WindowKind::OpenGl);
        assert!(decide(&mac, Preference::Auto).is_gpu());
        mac.window_env = Some(WindowKind::Software);
        let software = decide(&mac, Preference::Auto);
        assert_eq!(
            (software.mode, software.windows),
            (Mode::Cpu, WindowKind::Software)
        );
    }

    #[test]
    fn the_environment_variables_parse() {
        let os = |text: &'static str| Some(OsStr::new(text));
        assert_eq!(env_choice(None, None), (None, None));
        assert_eq!(env_choice(os("auto"), os("1")), (SOFTWARE_ENV, None));
        assert_eq!(
            env_choice(os("cpu"), None).0,
            Some((Preference::Software, "VESTRA_RENDER"))
        );
        assert_eq!(
            env_choice(os("gpu"), os("1")).0,
            Some((Preference::Gpu, "VESTRA_RENDER"))
        );
        let (choice, warning) = env_choice(os("vulkan"), None);
        assert_eq!(choice, None);
        assert!(warning.unwrap().contains("VESTRA_RENDER=vulkan"));
        for off in ["", "0"] {
            assert_eq!(env_choice(None, os(off)), (None, None));
        }
        assert_eq!(env_choice(None, os("yes")).0, SOFTWARE_ENV);
        for name in ["auto", "gpu", "software"] {
            assert_eq!(Preference::parse(name).unwrap().name(), name);
        }
        assert_eq!(Preference::parse("cpu"), None);
    }

    /// The same as `renderer::wanted` before M1 for the M1 inputs.
    #[test]
    fn gpu_exactly_when_it_was_wanted_before() {
        for headless in [false, true] {
            for gpu_backend in [false, true] {
                for software_env in [false, true] {
                    let mut facts = facts(headless, gpu_backend);
                    facts.env = software_env.then_some(SOFTWARE_ENV).flatten();
                    let decision = decide(&facts, Preference::Auto);
                    assert_eq!(decision.is_gpu(), gpu_backend && !headless && !software_env);
                    assert_eq!(decision.windows == WindowKind::OpenGl, decision.is_gpu());
                }
            }
        }
    }

    #[test]
    fn a_start_failure_keeps_its_detail_and_the_line_names_everything() {
        let gpu = decide(&facts(false, true), Preference::Auto);
        assert_eq!(
            gpu.line(),
            "render: GPU rendering, OpenGL windows; reason: default (nothing rules GPU rendering out)"
        );
        let cpu = gpu.fall_back(Reason::InitFailed("no OpenGL".into()));
        assert_eq!((cpu.mode, cpu.windows), (Mode::Cpu, WindowKind::Default));
        assert_eq!(
            cpu.line(),
            "render: software rendering, Slint's default window renderer; reason: init-failed (no OpenGL)"
        );
        assert_eq!(
            Reason::Env("VESTRA_SOFTWARE_RENDER").to_string(),
            "env (VESTRA_SOFTWARE_RENDER is set)"
        );
        assert_eq!(Reason::WindowFallback("x".into()).code(), "window-fallback");
        assert_eq!(
            Reason::Pending.json(),
            serde_json::json!({"code":"pending","message":"GPU rendering is starting"})
        );
        assert_eq!((Mode::Gpu.name(), Mode::Cpu.name()), ("gpu", "software"));
    }
}
