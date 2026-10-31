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
use crate::playback::render_denylist::{self, Action, Denylist};
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
    /// femtovg on OpenGL: what GPU mode's texture sharing needs.
    OpenGl,
    /// Slint's software renderer: a driver whose OpenGL cannot draw the
    /// windows (denylist action `software-windows`); CPU mode only.
    Software,
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
            Self::Default => "default",
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
fn system_name(os: &str) -> &str {
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
    /// The system's adapters in its order; the first is what GPU rendering
    /// would use.
    pub adapters: Vec<GpuAdapter>,
    pub denylist: Denylist,
    /// The crash guard keeps GPU rendering off: why, with until when.
    pub crashed: Option<String>,
    /// `SLINT_BACKEND` is set: Slint picks the window renderer itself and
    /// a denylist rule does not change it.
    pub slint_backend: bool,
}

/// The renderer pair and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    pub mode: Mode,
    pub windows: WindowKind,
    pub reason: Reason,
    /// More `render:` lines: denylist rules that hold, overrides.
    pub notes: Vec<String>,
}

impl Decision {
    fn cpu(reason: Reason) -> Self {
        Self {
            mode: Mode::Cpu,
            windows: WindowKind::Default,
            reason,
            notes: Vec::new(),
        }
    }
    fn gpu(reason: Reason) -> Self {
        Self {
            mode: Mode::Gpu,
            windows: WindowKind::OpenGl,
            reason,
            notes: Vec::new(),
        }
    }
    pub fn is_gpu(&self) -> bool {
        self.mode == Mode::Gpu
    }
    /// GPU rendering could not start before the backend did (the window
    /// renderer refused): CPU mode with Slint's default windows.
    pub fn fall_back(self, reason: Reason) -> Self {
        Self {
            notes: self.notes,
            ..Self::cpu(reason)
        }
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
    /// What keeps GPU rendering off whatever the user prefers: the crash
    /// guard, then the first rule whose action is `cpu` or
    /// `software-windows`.
    fn blocked(&self) -> Option<Reason> {
        if let Some(detail) = &self.crashed {
            return Some(Reason::PreviousCrash(detail.clone()));
        }
        self.hits()
            .into_iter()
            .find(|hit| matches!(hit.rule.action, Some(Action::Cpu | Action::SoftwareWindows)))
            .map(|hit| self.denylist_reason(&hit))
    }
    /// The first `software-windows` rule that holds.
    fn window_rule(&self) -> Option<Hit<'_>> {
        self.hits()
            .into_iter()
            .find(|hit| hit.rule.action == Some(Action::SoftwareWindows))
    }
    /// The rules that hold: `cpu` and `warn` rules for the first adapter
    /// (the one GPU rendering would use); `software-windows` rules for any
    /// adapter with a GPU of its own, because OpenGL windows may draw on
    /// another GPU than the first (a hybrid laptop's per-app choice).
    fn hits(&self) -> Vec<Hit<'_>> {
        let first = self.adapters.first();
        let matches = |adapter: Option<&GpuAdapter>| {
            self.denylist.matches(render_denylist::Facts {
                os: self.os,
                build: self.build.as_deref(),
                adapter,
            })
        };
        let mut hits: Vec<Hit<'_>> = matches(first)
            .into_iter()
            .map(|rule| Hit {
                rule,
                adapter: first,
            })
            .collect();
        for adapter in self.adapters.iter().skip(1).filter(|a| !a.software) {
            for rule in matches(Some(adapter)) {
                if rule.action == Some(Action::SoftwareWindows)
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
/// else GPU. Denylist rules that hold are noted even when something earlier
/// decided (Windows without a GPU renderer reports them). Then the windows:
/// a `software-windows` rule gives CPU mode Slint's software renderer,
/// whatever decided the mode, unless `SLINT_BACKEND` is set.
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
            Some((Preference::Gpu, variable)) => match facts.window_rule() {
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
            _ => blocked.map_or_else(|| Decision::gpu(Reason::Default), Decision::cpu),
        }
    };
    if let Some(Hit { rule, .. }) = facts.window_rule()
        && !facts.headless
    {
        if facts.slint_backend {
            decision.notes.push(format!(
                "render: SLINT_BACKEND is set; the windows keep it (denylist rule {} not applied to them)",
                rule.id
            ));
        } else if decision.mode == Mode::Cpu {
            decision.windows = WindowKind::Software;
        }
    }
    for Hit { rule, adapter } in facts.hits() {
        let action = match rule.action {
            Some(Action::Cpu) => "software rendering",
            Some(Action::SoftwareWindows) => "software rendering and software windows",
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
