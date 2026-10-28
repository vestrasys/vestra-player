//! `rendering.state`, `rendering.setPreference` and `rendering.restart`
//! (capability `rendering.manage`, SDK 1.5.35, round record
//! 2026-10-render-modes §3.4): the mode now and why, the saved preference
//! and what the next start would choose with it, and a restart that
//! continues the current file. The preference is saved at once and applies
//! at the next start, restart or not (as Chrome's and Firefox's hardware
//! acceleration switch).
use super::*;
use crate::launch::Resume;
use crate::playback::render_denylist::Denylist;
use crate::playback::render_mode::{Decision, Facts, Mode, Preference, Reason, decide, env_choice};
use crate::playback::render_state::{self, GuardFile, GuardKey};

/// What the rendering API reads and changes; built in `main.rs`.
pub(crate) struct Rendering {
    /// The facts this run decided from.
    pub facts: Facts,
    /// The mode this run started with (after a window renderer fallback).
    pub started: Mode,
    /// The saved preference (`render.json`).
    pub preference: Preference,
    /// `render.json`; `None` keeps the preference in memory (headless).
    pub file: Option<PathBuf>,
    /// What a restart starts again with.
    pub relaunch: Relaunch,
    /// This player was started by a restart (`--vestra-restart`), at this
    /// time: it refuses another one for [`RESTART_COOLDOWN`], so a plugin
    /// that restarts on activation cannot loop.
    pub restarted_at: Option<std::time::Instant>,
}

/// How long a restarted player refuses `rendering.restart`.
pub(crate) const RESTART_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(30);

/// Whether a restart is refused now (`busy`): one is under way in this
/// process, or this process was itself started by a restart less than
/// [`RESTART_COOLDOWN`] ago.
fn restart_refused(
    restarting: bool,
    restarted_at: Option<std::time::Instant>,
    now: std::time::Instant,
) -> Option<&'static str> {
    if restarting {
        Some("the player is already restarting")
    } else if restarted_at.is_some_and(|at| now.saturating_duration_since(at) < RESTART_COOLDOWN) {
        Some("the player restarted less than 30 seconds ago")
    } else {
        None
    }
}

/// The player's own arguments a restart repeats.
#[derive(Clone, Debug, Default)]
pub(crate) struct Relaunch {
    pub directories: Vec<PathBuf>,
    pub silent: bool,
}

/// What `main.rs` decides from at start, and the crash guard it arms.
pub(crate) struct Startup {
    pub facts: Facts,
    pub preference: Preference,
    /// `render.json`.
    pub file: Option<PathBuf>,
    /// `render-state.json` and what it is keyed on.
    pub guard: GuardFile,
    pub key: GuardKey,
}

/// Gather the facts (round record 2026-10-render-modes §3): the
/// environment, the system build, the first graphics adapter, the built-in
/// denylist with `<app data>/render-denylist.json`, the saved preference
/// (`render.json`) and the crash guard (`render-state.json`). A headless
/// player reads and writes none of the files and asks for no adapter.
pub(crate) fn startup(headless: bool) -> Startup {
    let directory = (!headless).then(platform::app_data_dir).flatten();
    let path = |name: &str| directory.as_ref().map(|directory| directory.join(name));
    let (env, warning) = env_choice(
        std::env::var_os("VESTRA_RENDER").as_deref(),
        std::env::var_os("VESTRA_SOFTWARE_RENDER").as_deref(),
    );
    if let Some(warning) = warning {
        diag!("{warning}");
    }
    let build = (!headless)
        .then(|| crate::system::os_facts().build.clone())
        .flatten();
    let adapters = if headless {
        Vec::new()
    } else {
        platform::gpu_adapters()
    };
    for adapter in &adapters {
        diag!(
            "render: adapter {} ({:04X}:{:04X}, driver {}{})",
            adapter.name,
            adapter.vendor_id,
            adapter.device_id,
            adapter.driver_version.as_deref().unwrap_or("unknown"),
            if adapter.software { ", software" } else { "" }
        );
    }
    let mut denylist = Denylist::built_in();
    if let Some(local) = path("render-denylist.json")
        && let Ok(text) = std::fs::read_to_string(&local)
    {
        match denylist.clone().merged(&text) {
            Ok(merged) => denylist = merged,
            Err(error) => diag!("render: {} is ignored ({error})", local.display()),
        }
    }
    let file = path("render.json");
    let (preference, warning) = render_state::read_preference(file.as_deref());
    if let Some(warning) = warning {
        diag!("render: render.json is ignored ({warning})");
    }
    let guard = GuardFile::new(path("render-state.json"));
    let adapter = adapters.first();
    let key = GuardKey {
        adapter: adapter
            .map(|adapter| adapter.name.clone())
            .unwrap_or_default(),
        driver: adapter
            .and_then(|adapter| adapter.driver_version.clone())
            .or_else(|| build.clone())
            .unwrap_or_default(),
        app: env!("CARGO_PKG_VERSION").into(),
    };
    let now = render_state::now();
    let crashed = guard.recover(now).blocks(&key, now).map(|until| {
        format!(
            "GPU rendering stopped an earlier start of the player; it is tried again after a graphics driver or player update, or in {} days",
            until.saturating_sub(now).div_ceil(24 * 60 * 60)
        )
    });
    Startup {
        facts: Facts {
            headless,
            gpu_backend: platform::GPU_RENDER,
            env,
            os: std::env::consts::OS,
            build,
            adapters,
            denylist,
            crashed,
            slint_backend: std::env::var_os("SLINT_BACKEND").is_some_and(|value| !value.is_empty()),
        },
        preference,
        file,
        guard,
        key,
    }
}

impl Rendering {
    fn next(&self) -> Decision {
        decide(&self.facts, self.preference)
    }
    /// The `rendering.state` value, given what draws now.
    pub fn state(&self, gpu_now: bool, reason: &Reason) -> Value {
        let next = self.next();
        let gpu = match self.facts.gpu_available() {
            Ok(()) => json!({"available":true,"reason":null}),
            Err(reason) => json!({"available":false,"reason":reason.json()}),
        };
        json!({
            "mode": if gpu_now { Mode::Gpu } else { Mode::Cpu }.name(),
            "reason": reason.json(),
            "preference": self.preference.name(),
            "nextMode": next.mode.name(),
            "nextReason": next.reason.json(),
            "restartPending": next.mode != self.started,
            "gpu": gpu,
        })
    }
    /// Check and save `preference` (applies at the next start).
    pub fn set_preference(&mut self, preference: Preference) -> Result<(), RpcError> {
        if preference == Preference::Gpu
            && let Err(reason) = self.facts.gpu_available()
        {
            let code = match reason {
                Reason::Headless | Reason::NoGpuBackend => "unsupported",
                _ => "unavailable",
            };
            return Err(RpcError::new(
                code,
                format!("GPU rendering cannot be chosen: {}", reason.message()),
            ));
        }
        render_state::write_preference(self.file.as_deref(), preference).map_err(|error| {
            RpcError::new("failed", format!("the preference cannot be saved: {error}"))
        })?;
        self.preference = preference;
        diag!(
            "render: preference {} saved; the next start renders with {}",
            preference.name(),
            self.next().mode.name()
        );
        Ok(())
    }
}

impl Host {
    pub(crate) fn rendering_dispatch(
        &mut self,
        method: &str,
        params: &Value,
    ) -> std::result::Result<Value, RpcError> {
        match method {
            "rendering.state" => Ok(self.rendering_state()),
            "rendering.setPreference" => {
                let preference = params["preference"]
                    .as_str()
                    .and_then(Preference::parse)
                    .ok_or_else(|| {
                        RpcError::new("invalid", "preference must be auto, gpu or software")
                    })?;
                self.rendering.set_preference(preference)?;
                Ok(self.rendering_state())
            }
            "rendering.restart" => self.restart_player().map(|()| Value::Null),
            _ => Err(RpcError::new("unsupported", "unknown rendering method")),
        }
    }
    fn rendering_state(&self) -> Value {
        let (gpu, reason) = self.backend.render.now();
        self.rendering.state(gpu, &reason)
    }
    /// Start a new player now (it waits for this one's instance lock), then
    /// quit as the Quit command does.
    fn restart_player(&mut self) -> std::result::Result<(), RpcError> {
        if self.headless {
            return Err(RpcError::new(
                "unsupported",
                "a player without windows does not restart",
            ));
        }
        if let Some(why) = restart_refused(
            RESTARTING.load(Ordering::Acquire),
            self.rendering.restarted_at,
            std::time::Instant::now(),
        ) {
            return Err(RpcError::new("busy", why));
        }
        let exe = std::env::current_exe()
            .map_err(|error| RpcError::new("failed", format!("the player's path: {error}")))?;
        let mut command = relaunch_command(&exe, &self.rendering.relaunch, &self.last_source);
        diag!("restart: {command:?}");
        // Windows: the new player may take the foreground from this one.
        platform::allow_handoff_foreground();
        let child = command
            .spawn()
            .map_err(|error| RpcError::new("failed", format!("the restart failed: {error}")))?;
        RESTARTING.store(true, Ordering::Release);
        diag!("restart: the new player is process {}", child.id());
        crate::stop_handoffs();
        self.ui_state.flush();
        // After this request's reply went out.
        let _ = slint::invoke_from_event_loop(|| {
            let _ = slint::quit_event_loop();
        });
        Ok(())
    }
}

/// A restart was started (once per process).
static RESTARTING: AtomicBool = AtomicBool::new(false);

/// The new player's command line: `--vestra-restart PID`, this run's
/// development plugins and `--silent`, and the current local file with
/// where it is (`--vestra-resume`); not a stream or another kind of source.
fn relaunch_command(exe: &Path, relaunch: &Relaunch, source: &Value) -> std::process::Command {
    let mut command = std::process::Command::new(exe);
    command.args([
        "--vestra-restart".to_string(),
        std::process::id().to_string(),
    ]);
    for directory in &relaunch.directories {
        command.arg("--plugin").arg(directory);
    }
    if relaunch.silent {
        command.arg("--silent");
    }
    if let Some((file, resume)) = resume_point(source) {
        command.args(["--vestra-resume".to_string(), resume.arg()]);
        command.arg("--").arg(file);
    }
    command
}

/// The local file `source` (a `source.state`) plays or pauses in, and
/// where; nothing for no source, an ended one, or one that is not a file.
fn resume_point(source: &Value) -> Option<(PathBuf, Resume)> {
    let file = source["source"]
        .as_str()
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.is_file())?;
    matches!(source["phase"].as_str(), Some("ready" | "loading")).then(|| {
        let resume = Resume {
            seconds: source["position"]
                .as_f64()
                .filter(|s| s.is_finite() && *s >= 0.)
                .unwrap_or(0.),
            paused: source["paused"].as_bool().unwrap_or(false),
        };
        (file, resume)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::GpuAdapter;

    fn rendering(gpu_backend: bool, file: Option<PathBuf>) -> Rendering {
        let facts = Facts {
            gpu_backend,
            os: "windows",
            denylist: Denylist::built_in(),
            ..Facts::default()
        };
        Rendering {
            started: decide(&facts, Preference::Auto).mode,
            facts,
            preference: Preference::Auto,
            file,
            relaunch: Relaunch::default(),
            restarted_at: None,
        }
    }

    #[test]
    fn a_restarted_player_refuses_another_restart_for_a_while() {
        let start = std::time::Instant::now();
        assert_eq!(restart_refused(false, None, start), None);
        assert!(restart_refused(true, None, start).is_some(), "under way");
        let soon = start + std::time::Duration::from_secs(5);
        assert!(restart_refused(false, Some(start), soon).is_some());
        let later = start + RESTART_COOLDOWN;
        assert_eq!(restart_refused(false, Some(start), later), None);
    }

    #[test]
    fn a_saved_preference_shows_as_pending_until_it_matches_the_started_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("render.json");
        let mut gpu = rendering(true, Some(path.clone()));
        let state = gpu.state(true, &Reason::Default);
        assert_eq!(
            (state["mode"].as_str(), state["nextMode"].as_str()),
            (Some("gpu"), Some("gpu"))
        );
        assert_eq!(state["restartPending"], false);
        assert_eq!(state["gpu"], json!({"available":true,"reason":null}));
        gpu.set_preference(Preference::Software).unwrap();
        let state = gpu.state(true, &Reason::Default);
        assert_eq!(state["preference"], "software");
        assert_eq!(state["nextMode"], "software");
        assert_eq!(state["nextReason"]["code"], "preference");
        assert_eq!(state["restartPending"], true);
        assert_eq!(state["mode"], "gpu", "the current run does not change");
        assert_eq!(
            render_state::read_preference(Some(&path)).0,
            Preference::Software,
            "saved at once"
        );
        // Changed back: nothing pending.
        gpu.set_preference(Preference::Gpu).unwrap();
        assert_eq!(gpu.state(true, &Reason::Default)["restartPending"], false);
    }

    #[test]
    fn gpu_is_refused_with_the_reason_where_it_is_not_available() {
        let mut windows = rendering(false, None);
        let state = windows.state(false, &Reason::NoGpuBackend);
        assert_eq!(state["gpu"]["available"], false);
        assert_eq!(state["gpu"]["reason"]["code"], "no-gpu-backend");
        assert_eq!(state["reason"]["code"], "no-gpu-backend");
        let error = windows.set_preference(Preference::Gpu).unwrap_err();
        assert_eq!(error.code, "unsupported");
        assert!(windows.set_preference(Preference::Software).is_ok());
        assert_eq!(
            windows.state(false, &Reason::NoGpuBackend)["restartPending"],
            false
        );
        let mut denied = rendering(true, None);
        denied.facts.adapters = vec![GpuAdapter {
            name: "AMD Radeon 780M".into(),
            vendor_id: 0x1002,
            device_id: 0x15BF,
            driver_version: Some("31.0.14005.21001".into()),
            software: false,
        }];
        let error = denied.set_preference(Preference::Gpu).unwrap_err();
        assert_eq!(error.code, "unavailable");
        assert!(error.message.contains("denylist"), "{}", error.message);
        assert_eq!(denied.preference, Preference::Auto, "not saved");
    }

    #[test]
    fn a_restart_repeats_the_plugins_and_continues_a_local_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.mkv");
        std::fs::write(&file, b"x").unwrap();
        let relaunch = Relaunch {
            directories: vec![PathBuf::from("/dev/plugin")],
            silent: true,
        };
        let source =
            json!({"source":file.to_str().unwrap(),"phase":"ready","position":12.5,"paused":true});
        let command = relaunch_command(Path::new("player"), &relaunch, &source);
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[0], "--vestra-restart");
        assert_eq!(args[1], std::process::id().to_string());
        assert_eq!(&args[2..5], ["--plugin", "/dev/plugin", "--silent"]);
        assert_eq!(&args[5..7], ["--vestra-resume", "12.5:paused"]);
        assert_eq!(&args[7..], ["--", file.to_str().unwrap()]);
        for other in [
            json!({"source":"https://example.com/a.mp4","phase":"ready"}),
            json!({"source":file.to_str().unwrap(),"phase":"ended"}),
            Value::Null,
        ] {
            assert!(resume_point(&other).is_none(), "{other}");
        }
    }
}
