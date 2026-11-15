#![forbid(unsafe_code)]
// Windows: the release build is a GUI program (no console window when
// started from Explorer); debug builds keep their console for the logs. Where
// the logs go: doc/architecture/packaging-proposal.md §10.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// `diag!`, used everywhere below (declared first for its textual scope).
#[macro_use]
mod diag;
mod binding_manager;
mod bindings;
mod capabilities;
mod clipboard;
mod command_renames;
mod commands;
mod conditions;
mod core_actions;
pub mod font_asset;
pub mod font_carrier;
pub mod host;
mod i18n;
mod input_trace;
mod launch;
mod open_files;
pub mod packages;
mod platform;
pub mod playback;
mod power;
mod process;
mod session_storage;
mod source_events;
mod supervisor;
mod system;
mod timeline;
mod triggers;
pub mod ui;
mod version;

use anyhow::{Result, bail};
use host::{
    Core, Event, FRAME_SINK, Host, InputShared, LAST_SOURCE, MenuMode, PIP_GPU_SINK, RESIZE_SINK,
    event_loop_ended, restart_log, restart_logged, restart_player, run_through_loss, teardown_step,
};
use packages::package_manager;
use playback::{backend::Backend, frame_pacing, render_mode, render_target};
use process::{Plugin, runtime_paths};
use serde_json::{Value, json};
use slint::ComponentHandle;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    path::PathBuf,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use ui::{
    bar_buttons, composition, menu, theme, ui_fonts, ui_state, view, window_control, window_state,
};

/// Opt-in bounded fixture diagnostics, not a plugin frame API or source/PTS fence.
struct FrameEvidence {
    directory: PathBuf,
    index: std::fs::File,
    count: usize,
    last: Instant,
}
impl FrameEvidence {
    fn new(directory: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&directory)?;
        let index = std::fs::File::create(directory.join("frames.jsonl"))?;
        Ok(Self {
            directory,
            index,
            count: 0,
            last: Instant::now() - Duration::from_secs(1),
        })
    }
    fn sample(&mut self, backend: &Backend, source: &Value, start: Instant) -> Result<()> {
        use std::io::Write;
        if self.count >= 64 || self.last.elapsed() < Duration::from_millis(200) {
            return Ok(());
        }
        let frame = backend.frame.peek(|f| {
            (
                f.sequence,
                f.buffer.as_bytes().to_vec(),
                f.buffer.width(),
                f.buffer.height(),
            )
        });
        if let Some((sequence, pixels, width, height)) = frame {
            let name = format!("frame-{:03}.ppm", self.count);
            let mut file = std::fs::File::create(self.directory.join(&name))?;
            write!(file, "P6\n{width} {height}\n255\n")?;
            file.write_all(&pixels)?;
            writeln!(
                self.index,
                "{}",
                json!({"file":name,"softwareSequence":sequence,"elapsedMs":start.elapsed().as_secs_f64()*1000.,"observedSource":source,"frameSourceAssociation":"none"})
            )?;
            self.index.flush()?;
            self.count += 1;
            self.last = Instant::now();
        }
        Ok(())
    }
}
/// Whether this player still takes a later launch's files (`Primary::serve`).
/// Cleared when quitting starts (Quit, or the event loop ending after the last
/// window closed): a launch during the player's last moments is refused, keeps
/// trying, and becomes the next player once this process exits and frees the
/// lock. If the teardown takes longer than the launch waits (5 s), the launch
/// runs on its own without the lock (`single_instance`'s error path).
static HANDOFFS: AtomicBool = AtomicBool::new(true);
fn stop_handoffs() {
    HANDOFFS.store(false, Ordering::Release);
}
/// CPU mode's windows other than wgpu ones (`render_mode::WindowKind`; wgpu
/// windows are selected in `main` and fall back to
/// `Decision::fallback_windows`): femtovg on OpenGL (Windows, named
/// explicitly since Slint's default there would be femtovg-wgpu;
/// `VESTRA_WINDOW_RENDERER=gl` elsewhere), with Slint's software renderer
/// when OpenGL fails; Slint's software renderer (an OpenGL window denylist
/// rule, `VESTRA_WINDOW_RENDERER=software`); or Slint's default (macOS and
/// Linux, `SLINT_BACKEND`). `Wgpu` here gets OpenGL ones.
fn select_cpu_windows(
    windows: render_mode::WindowKind,
    winit: impl Fn() -> slint::BackendSelector,
) -> std::result::Result<(), slint::PlatformError> {
    match windows {
        render_mode::WindowKind::OpenGl | render_mode::WindowKind::Wgpu => {
            if let Err(error) = winit().renderer_name("femtovg".into()).select() {
                diag!("render: no OpenGL windows ({error}); Slint's software renderer");
                winit().renderer_name("software".into()).select()?;
            }
            Ok(())
        }
        render_mode::WindowKind::Software => winit().renderer_name("software".into()).select(),
        render_mode::WindowKind::Default => winit().select(),
    }
}

fn main() -> Result<()> {
    // Tests that run the windowed host offscreen (`ui::offscreen`, no
    // winit, so no main-thread requirement): on a thread with room for the
    // debug build's Slint compiler, which overflows Windows' 1 MB main
    // thread when the shell compiles with an adjust panel.
    if ui::offscreen::requested() {
        return std::thread::Builder::new()
            .name("main".into())
            .stack_size(64 << 20)
            .spawn(player)?
            .join()
            .unwrap_or_else(|_| bail!("the player thread panicked"));
    }
    player()
}

/// `VESTRA_TEST_UI_STALL=<start ms>,<length ms>`, a test switch (core
/// thread plan S1.2; tests/ui_stall.rs): `<start ms>` after the player
/// process started, the windowed tick's UI part sleeps `<length ms>` once,
/// as a UI thread held by Slint, the GPU or Windows would. Only on the
/// offscreen platform (`VESTRA_TEST_OFFSCREEN=1`); ignored, with a line
/// saying so, anywhere else. When and how long: `Some((due, length))`.
fn ui_stall_switch(launched: Instant, offscreen: bool) -> Option<(Instant, Duration)> {
    let value = std::env::var("VESTRA_TEST_UI_STALL")
        .ok()
        .filter(|value| !value.is_empty())?;
    if !offscreen {
        diag!("VESTRA_TEST_UI_STALL ignored: only with VESTRA_TEST_OFFSCREEN=1 and a window");
        return None;
    }
    let parsed = value.split_once(',').and_then(|(start, length)| {
        Some((
            start.trim().parse::<u64>().ok()?,
            length.trim().parse::<u64>().ok()?,
        ))
    });
    match parsed {
        Some((start, length)) => Some((
            launched + Duration::from_millis(start),
            Duration::from_millis(length),
        )),
        None => {
            diag!("VESTRA_TEST_UI_STALL={value}: not <start ms>,<length ms>; ignored");
            None
        }
    }
}

fn player() -> Result<()> {
    let launched = Instant::now();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let launch::Launch {
        directories,
        headless,
        silent,
        seconds,
        snapshot,
        frame_evidence,
        files,
        urls,
        restart,
        resume,
    } = launch::parse(&args, &std::env::current_dir()?)?;
    for url in &urls {
        diag!("host.warning open: {url} is not a file; open links from a plugin's command");
    }
    // One player per user (`platform::single_instance`): a later launch
    // hands its files to the running one and exits. Development runs
    // (`--plugin`) and headless ones stay on their own unless
    // `VESTRA_INSTANCE_DIR` names the directory to coordinate in (tests).
    let instance_dir = std::env::var_os("VESTRA_INSTANCE_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            (!headless && directories.is_empty())
                .then(platform::app_data_dir)
                .flatten()
        });
    // A restart (`rendering.restart`, or after the windows' GPU device was
    // lost) first waits for the process it replaces to end: that one's
    // crash guard and files are final then (Windows; elsewhere at once).
    if let Some(parent) = restart {
        if platform::wait_for_process_exit(parent, Duration::from_secs(20)) {
            restart_log(&format!("render: the restart of process {parent} started"));
        } else {
            restart_log(&format!(
                "render: the player this restart replaces (process {parent}) did not end in 20 s"
            ));
        }
    }
    let mut primary = None;
    if let Some(directory) = instance_dir {
        // A player that restarted itself (`rendering.restart`) is replaced
        // by this one: wait up to 20 s for its lock and never hand over.
        let claimed = match restart {
            Some(_) => platform::single_instance_restarted(&directory, Duration::from_secs(20)),
            None => platform::single_instance(&directory, &files, Duration::from_secs(5)),
        };
        match claimed {
            Ok(platform::Launch::HandedOff) => return Ok(()),
            Ok(platform::Launch::Primary(claimed)) => primary = Some(claimed),
            // As any launch that cannot claim the lock: run on its own.
            Err(error) => diag!("single instance: {error}; running on its own"),
        }
    }
    // The host log (`diag.rs`): a release build with windows writes a few
    // lines to `<app data>/logs/host.log`; opened once this process is the
    // player (a launch that handed its files over has returned above).
    if let Some(path) = diag::log_path(
        std::env::var_os("VESTRA_LOG_FILE"),
        cfg!(not(debug_assertions)),
        headless,
        !directories.is_empty(),
        platform::app_data_dir(),
    ) {
        // The plugin log beside it: what plugins write to their console.
        if let Err(error) = diag::open_plugin_log(diag::plugin_log_path(&path)) {
            diag!("log: the plugin log cannot be opened ({error})");
        }
        match diag::open_log(path) {
            Ok(()) => diag::log_panics(),
            Err(error) => diag!("log: the host log cannot be opened ({error})"),
        }
    }
    logged!(
        "start: Vestra Player {} on {} {}{}",
        version::describe(version::player(), option_env!("VESTRA_BUILD")),
        std::env::consts::OS,
        std::env::consts::ARCH,
        if headless { ", headless" } else { "" }
    );
    if let Some(parent) = restart {
        logged!("restart: this player replaces process {parent}");
    }
    let mut evidence = frame_evidence.map(FrameEvidence::new).transpose()?;
    // A packaged player missing its own files (the bundled Node above all)
    // stops here, not with every plugin failing on its own.
    let missing = runtime_paths::get().missing(std::path::Path::is_file);
    if !missing.is_empty() {
        let text = format!(
            "This installation of Vestra Player is incomplete; reinstall it.\n\n{}",
            missing.join("\n")
        );
        if !headless {
            rfd::MessageDialog::new()
                .set_title("Vestra Player")
                .set_description(&text)
                .set_level(rfd::MessageLevel::Error)
                .set_buttons(rfd::MessageButtons::Ok)
                .show();
        }
        bail!("{text}");
    }
    // The renderer pair and why (doc/architecture/README.md §4.3): GPU
    // rendering (SDK site, Concepts: render options) needs the windows the
    // platform pairs with it (macOS: femtovg on OpenGL; Windows: femtovg on
    // wgpu/Direct3D 12, and only when the user turned it on); headless,
    // `VESTRA_RENDER`, the saved preference, the crash guard, the driver
    // denylist and systems without a GPU renderer can make it software.
    // Tests that run the windowed host with no window on screen
    // (`ui::offscreen`): software rendering, as headless runs.
    let offscreen = !headless && ui::offscreen::requested();
    let mut ui_stall = ui_stall_switch(launched, offscreen);
    let startup = host::rendering_startup(headless || offscreen);
    let mut render = render_mode::decide(&startup.facts, startup.preference);
    logged!("{}", render.line());
    for note in &render.notes {
        diag!("{note}");
    }
    // The crash guard: armed until GPU rendering (or, on Windows, CPU mode's
    // Direct3D 12 windows: a driver that crashes there would crash every
    // start) ran a few seconds, gave up on its own, or the player quit
    // (`render_state`). `_armed` lives until `main` returns, by any path:
    // only a process that dies leaves the guard armed.
    let guard = startup.guard.clone();
    // Keyed on the adapter the mode uses: GPU rendering's, or CPU mode's
    // Direct3D 12 windows' (they may differ on a hybrid laptop).
    let keys = if render.is_gpu() {
        startup.keys.clone()
    } else {
        startup.windows_keys.clone()
    };
    let _armed = (render.is_gpu() || render.uses_wgpu_windows())
        .then(|| guard.arm(&keys, playback::render_state::now()));
    // What a restart after a lost window device needs (Windows wgpu
    // windows; `host/restart.rs`).
    // When this player was started by a restart: another is refused for a
    // while (`rendering::RESTART_COOLDOWN`).
    let restarted_at = restart.map(|_| Instant::now());
    let lost_device = host::LostDevice {
        relaunch: host::Relaunch {
            directories: directories.clone(),
            silent,
        },
        guard: guard.clone(),
        keys,
        restarted_at,
    };
    // The GPU the mode uses (GPU rendering's, else CPU mode's windows'
    // adapter); macOS names no adapter yet, only its system build.
    if !headless {
        let key = &lost_device.keys.key;
        logged!(
            "render: this mode's adapter is {} (driver {})",
            if key.adapter.is_empty() {
                "unknown"
            } else {
                &key.adapter
            },
            if key.driver.is_empty() {
                "unknown"
            } else {
                &key.driver
            }
        );
    }
    if offscreen {
        ui::offscreen::install().map_err(|error| anyhow::anyhow!("offscreen platform: {error}"))?;
    } else if !headless {
        // Windows: the main window draws at the display's vertical blanks
        // (`frame_pacing.rs`), not on Slint's timer.
        // Every window: the tool windows are owned by the main window
        // (`platform::window_attributes`, Windows).
        let winit = || {
            let selector = slint::BackendSelector::new()
                .backend_name("winit".into())
                .with_winit_window_attributes_hook(platform::window_attributes);
            if frame_pacing::hooked() {
                selector.with_winit_custom_application_handler(frame_pacing::Handler)
            } else {
                selector
            }
        };
        // The windows the decision chose; a GPU mode whose windows cannot be
        // had falls back to software rendering with CPU mode's fallback
        // windows, and CPU mode's wgpu windows fall back to them too.
        let wgpu_windows = |gpu_mode: bool| {
            // A lost window device (or a stalled GPU renderer) restarts the
            // player at once, from the device's callback, after recording
            // the loss in the crash guard: the new one waits for this
            // process to end, however it ends (a lost device may also fail
            // Slint's teardown).
            let lost = lost_device.clone();
            platform::on_window_device_lost(move || {
                let source = LAST_SOURCE
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone();
                restart_player(&lost, &source);
            });
            // The adapter resolved once at start, for the keys and the denylist.
            let adapter = if gpu_mode {
                &startup.gpu_adapter
            } else {
                &startup.windows_adapter
            };
            platform::select_wgpu_windows(winit, adapter)
        };
        let gpu_windows = match render.windows {
            render_mode::WindowKind::Wgpu if render.is_gpu() => wgpu_windows(true),
            render_mode::WindowKind::OpenGl if render.is_gpu() => winit()
                .renderer_name("femtovg".into())
                .select()
                .map_err(|error| format!("the OpenGL window renderer is unavailable: {error}")),
            _ => Ok(()),
        };
        if let Err(error) = gpu_windows {
            render = render.fall_back(render_mode::Reason::InitFailed(error));
            logged!("{}", render.line());
            guard.settle();
        }
        if !render.is_gpu() && render.windows == render_mode::WindowKind::Wgpu {
            // CPU mode in Direct3D 12 windows: software
            // video uploaded once per frame, flip model with vsync.
            if let Err(error) = wgpu_windows(false) {
                diag!("render: no Direct3D 12 windows for software rendering ({error})");
                render = render.without_wgpu_windows();
                logged!("{}", render.line());
                guard.settle();
            }
        }
        if !render.is_gpu() && render.windows != render_mode::WindowKind::Wgpu {
            select_cpu_windows(render.windows, winit)?;
        }
    }
    // The plugin shaders' crash guard (`render_state`, `host/diagnostics.rs`):
    // GPU rendering only, keyed on its adapter. A run that ended while
    // plugin shaders were on keeps them off here.
    let shader_block = render
        .is_gpu()
        .then(|| guard.shader_start(&startup.keys.key, playback::render_state::now()))
        .flatten();
    if let Some(block) = &shader_block {
        logged!(
            "shaders: plugin shaders are off for {} (the last run ended while those of {} were on)",
            if block.until.is_some() {
                "a while"
            } else {
                "this start"
            },
            block.plugins.join(", ")
        );
    }
    let shader_guard =
        host::ShaderGuard::new(guard.clone(), startup.keys.key.clone(), shader_block);
    if !headless {
        // Shutdown, restart, logoff (Windows): the shader guard settles and
        // the exit line is written before the system ends the process.
        let writer = shader_guard.writer();
        platform::on_session_end(move |event| host::session_end(&writer, event));
        // A device loss restarts the player from any thread: the guard's
        // pending write goes to disk first (`host::persist_shader_guard`).
        host::register_shader_writer(shader_guard.writer());
    }
    let mut observed_source = Value::Null;
    let (events, receiver) = mpsc::channel();
    if !headless {
        // Before the event loop: the launch's own open-documents event (macOS).
        let events = events.clone();
        platform::on_open_files(move |files| {
            let _ = events.send(Event::OpenFiles(files, false));
        });
    }
    if let Some(primary) = primary {
        let events = events.clone();
        primary.serve(move |files| {
            HANDOFFS.load(Ordering::Acquire) && events.send(Event::OpenFiles(files, true)).is_ok()
        });
    }
    let backend = Backend::start(events.clone(), silent, &render)?;
    // `plugin.app`, handed to every plugin process when it starts.
    let ffmpeg = &backend.versions["ffmpeg"];
    // `features`: protocol additions the SDK detects (not part of the plugin
    // API): row patches of property writes (ui_patch.rs).
    // `version` is the release version (`version.rs`), not the host's.
    process::set_app_info(&json!({"version":version::player(),
        "build":option_env!("VESTRA_BUILD"),
        "features":[ui::ui_patch::FEATURE],
        "components":{"mpv":backend.versions["mpv"],
            "libav":if ffmpeg.is_string() { json!({"ffmpeg":ffmpeg}) } else { json!({}) },
            "slint":env!("VESTRA_SLINT_VERSION")}}));
    let (mount_events, mount_input) = mpsc::sync_channel(64);
    let reaper = process::reaper::Reaper::spawn(events.clone());
    let on_demand = host::supervisor::OnDemand::from_env();
    // Read before `ui_state` opens, as in the `Host` literal before plan
    // step S2b split `Core` out.
    let clock = Instant::now();
    // Opened before the core's package store, as before `Core` was split
    // out of `Host` (plan step S2b).
    let ui_state = if headless {
        ui_state::Store::disabled()
    } else {
        ui_state::Store::open(&ui_state::default_path())
    };
    let core = Core {
        backend,
        shared: None,
        shared_policy: Default::default(),
        plugins: BTreeMap::new(),
        plugins_revision: 0,
        events,
        headless,
        panel_plugins: Default::default(),
        command_runs: Default::default(),
        clock,
        sequence: None,
        last_source: Value::Null,
        rendering: host::Rendering {
            facts: startup.facts,
            started: render.mode,
            windows: render.windows,
            preference: startup.preference,
            file: startup.file,
            relaunch: host::Relaunch {
                directories: directories.clone(),
                silent,
            },
            restarted_at,
        },
        source_state_replies: Default::default(),
        host_revision: 0,
        supervisor: host::supervisor::new(),
        activation_receipts: Default::default(),
        file_types: json!([]),
        window_facts: Value::Null,
        window_facts_seq: None,
        held_mounts: Default::default(),
        mounts: BTreeMap::new(),
        mount_queue: std::collections::VecDeque::new(),
        settings: Default::default(),
        prepared: None,
        warnings: Default::default(),
        packages: package_manager::Manager::open()?,
        reaper,
        actions_busy: Default::default(),
        step_seek: BTreeMap::new(),
        step_writes: 0,
        development: Default::default(),
        dev_disabled: Default::default(),
        failures: Default::default(),
        start_error: None,
        data_removals: BTreeMap::new(),
        dev_loading: None,
        session_storage: Default::default(),
        bindings_sent: 0,
        bindings_list_sent: Value::Null,
        bindings_seen: [0; 2],
        packages_sent: Value::Null,
        packages_seen: None,
        notified_at: 0,
        keys_checked: 0,
        keys_sent: BTreeMap::new(),
        timeline: Default::default(),
        opens: Default::default(),
        triggers: Default::default(),
        trigger_state: Default::default(),
        commands: commands::Registry::new(),
        tables: Default::default(),
        shader_guard,
        memory_logged: None,
        // Made above, before the clock, as before plan step S2c moved it
        // from `Host`.
        on_demand,
    };
    let mut host = Host {
        core,
        views: BTreeMap::new(),
        dialog_busy: false,
        origins: Default::default(),
        confirm_open: false,
        shell: None,
        preparing: None,
        mount_singly: 0,
        mount_events,
        mount_input,
        last_image: None,
        video_texture: RefCell::new(None),
        video_target: Rc::default(),
        winit_draws: Rc::default(),
        shell_generation: 1,
        next_shell_generation: 1,
        menu_path: Vec::new(),
        menu_scroll: Vec::new(),
        menu_aim: menu::HoverAim::default(),
        menu_panels: Vec::new(),
        menu_shown: Default::default(),
        palette: Default::default(),
        lead: None,
        lead_strip: Default::default(),
        review: None,
        policy: window_state::Policy::new(0),
        window_facts: Value::Null,
        facts_seq: 0,
        measured_shell: None,
        menu_open: false,
        title_strip: Default::default(),
        columns: Default::default(),
        panels: Default::default(),
        compiled_now: Default::default(),
        restarts: Default::default(),
        // Headless runs mount nothing.
        startup_panels: headless,
        tabs_failed: Default::default(),
        bottom_shown: true,
        native_window: None,
        native_before: None,
        native_unavailable: None,
        input: Rc::new(RefCell::new(InputShared::default())),
        ui_state,
        osd_until: None,
        osd_actions: None,
        osd_shown_at: 0,
        osd_clear_at: None,
        osd_shown: Default::default(),
        osd_from: None,
        info_panels: BTreeMap::new(),
        floating: None,
        menu_mode: MenuMode::Main,
        menu_generation: 1,
        window_title: "Vestra Player".into(),
        power: power::Power::system(),
        last_frame: None,
        sizing: Default::default(),
        frame_log: render_target::FrameLog::from_env(),
        shown_size: None,
        on_top: window_control::LastWrite::new(false),
        cursor_playing: window_control::LastWrite::new(false),
        on_top_applied: None,
        area_replies: Vec::new(),
        images: Default::default(),
        no_video: true,
        video_absent: true,
        video_cleared: false,
        cover_art_off: false,
        pip: Default::default(),
        pip_log: None,
        render_area: None,
        render_sent: None,
        render_continuous: false,
        pip_gpu: Default::default(),
        gpu_paced: false,
        drops: Default::default(),
        settings: Default::default(),
        settings_category: None,
        theme_pending: false,
        placements: view::Placements::default(),
        bar: bar_buttons::Strip::default(),
        registry: Default::default(),
    };
    // Kept trigger conditions: one file per installed instance.
    host.core.triggers = triggers::Triggers::new(Some(host.core.triggers_dir()));
    // The interface language of this run (doc/architecture/i18n.md §2):
    // `VESTRA_LANGUAGE` (development and tests), else the saved choice, else
    // the system's languages. Headless runs ignore the system: English
    // unless asked, so diagnostics read the same on every machine.
    let preference = std::env::var("VESTRA_LANGUAGE")
        .ok()
        .or_else(|| host.language_preference());
    let system = if headless {
        Vec::new()
    } else {
        platform::preferred_languages()
    };
    i18n::init(i18n::resolve(preference.as_deref(), &system));
    if !headless {
        // CJK fallback fonts for every window, before the first one exists
        // (`ui_fonts`), ordered for the interface language (Japanese fonts
        // first in `ja`, Korean in `ko`, Simplified Chinese otherwise).
        ui_fonts::install(i18n::locale());
    }
    host.register_host_commands()?;
    {
        // Commands whose full id changed (`command_renames`): keys and
        // recent palette entries saved under the old id follow, once.
        let renamed = {
            let state = &mut host.ui_state.state;
            let keys = command_renames::migrate_overlay(&mut state.bindings);
            command_renames::migrate_recent(&mut state.palette_recent) || keys
        };
        // Host 0.6.0: an override of Toggle Fullscreen gets its new
        // default Escape, once (`migrate_fullscreen_escape`).
        let holders = bindings::parse("Escape")
            .map(|escape| host.core.bindings().default_holders(&escape))
            .unwrap_or_default();
        let state = &mut host.ui_state.state;
        let escape = host::migrate_fullscreen_escape(
            &mut state.bindings,
            &mut state.bindings_migrated,
            &holders,
        );
        if renamed || escape {
            // Saved ids now name the commands' current ids, and the
            // migrations are recorded; saved once.
            let now = host.core.now_ms();
            host.ui_state.touch(now);
            host.ui_state.flush();
        }
        let overlay = host.ui_state.state.bindings.clone();
        host.core.bindings_mut().set_overlay(overlay);
        if let Some(sidebar) = &host.ui_state.state.sidebar {
            host.policy.set_sidebar_width(sidebar.width);
        }
        if let Some(adjust) = &host.ui_state.state.adjust_slot {
            host.policy.set_adjust_width(adjust.width);
        }
    }
    for directory in &directories {
        host.load(directory, 120_000)?;
    }
    // A restart continues its first file where the replaced player was.
    if let (Some(resume), Some(file)) = (resume, files.first()) {
        let _ = host.core.backend.commands.send(playback::backend::Command {
            reply: host::Reply { owner: 0, id: 0 },
            method: "host.resume".into(),
            params: json!({"source":file.to_string_lossy(),"seconds":resume.seconds,"paused":resume.paused}),
        });
    }
    // A start with a file: CPU mode's Direct3D 12 windows count as started
    // only once its first picture was shown (the crash guard, below).
    let opened_at_start = !files.is_empty();
    host.open_files(files, false);
    let start = Instant::now();
    if headless {
        loop {
            if seconds.is_some_and(|seconds| start.elapsed().as_secs_f64() >= seconds) {
                break;
            }
            let received = receiver.recv_timeout(Duration::from_millis(10));
            // A tick is the work after the wait (`VESTRA_FRAME_LOG=1`).
            let began = Instant::now();
            let first = match received {
                Ok(event) => Some(event),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(_) => break,
            };
            // At least 128 events a tick, then what its time budget allows,
            // as the windowed tick (`host::drain_events`): one a tick fell
            // behind while the start-up queue held each tick (`START_BUDGET`),
            // and the shared runner's answered pings waited past its
            // heartbeat limit.
            let mut handle = |event: Event| {
                if let Event::Broadcast(name, data) = &event
                    && name == "source.state"
                {
                    observed_source = data.clone();
                }
                host.handle(event);
            };
            let handled = first.map(&mut handle).map_or(0, |()| 1);
            host::drain_events(&receiver, began, handled, handle);
            host.activation_tick();
            host.on_demand_tick();
            host.power_tick();
            host.diagnostics_tick();
            host.package_tick();
            host.notify_tick();
            host.timeline_tick();
            host.open_files_tick();
            if let Some(log) = host.frame_log.as_mut() {
                log.tick_took(began.elapsed());
                let presentation = &host.core.backend.render.presentation;
                let (produced, shown) = host.core.backend.frame.counts();
                let (draws, presented) = presentation.counts();
                let counts = render_target::LogCounts {
                    produced,
                    shown,
                    draws,
                    presented,
                };
                if let Some(line) = log.line(counts, presentation, || None, None, Instant::now()) {
                    diag!("{line}{}", host.core.backend.frame.stages.text());
                }
            }
            if let Some(evidence) = evidence.as_mut() {
                evidence.sample(&host.core.backend, &observed_source, start)?;
            }
        }
        // Every plugin process stops within one deadline (supervisor.md
        // T31), each running plugin recorded first.
        host.shutdown_plugins(host::QUIT_BUDGET);
        stop_handoffs();
        host.core.shader_guard.quit();
        logged!("exit: normal");
    } else {
        // Windows: no window opens white (cloaked until its first draw).
        platform::cloak_until_drawn();
        let native_before = platform::capture_windows().ok();
        // Compiled once for the run: every slot is a container, and the
        // panels compile on their own when first shown (`ui/panels.rs`).
        let window = composition::shell_in(None)?;
        let position = host.ui_state.state.window.map(|frame| {
            window.window().set_size(slint::LogicalSize::new(
                frame.width.max(320.) as f32,
                frame.height.max(182.) as f32,
            ));
            slint::LogicalPosition::new(frame.x as f32, frame.y as f32)
        });
        let texture = host.attach_gpu(&window);
        // Title bars follow the palette from the first window on.
        platform::set_dark_title_bars(!theme::is_light(host.theme()));
        // IA-CORE-03: the saved frame (Windows: in desktop pixels) and
        // maximized state, never fullscreen. The user never sees the window
        // at another frame first: Windows keeps it cloaked until it is placed
        // (after Slint's first draw), macOS zooms it at its creation, before
        // it is visible (`view::Placements`).
        let restore = view::Restore {
            maximized: host.ui_state.state.window_maximized,
            physical: host.ui_state.state.window_physical,
        };
        match position {
            Some(position) => {
                platform::hold_next_shown();
                let mut position = position;
                if cfg!(target_os = "macos") && restore.maximized {
                    window.window().set_maximized(true);
                    // Zoomed at its creation, the window is never moved
                    // again (`view::place`), and Restore returns it to its
                    // creation frame: that frame must be the saved one, not
                    // one title bar higher (winit creates it with the
                    // content, not the frame, at the position).
                    position = platform::creation_position(position);
                }
                host.placements.show_restored(&window, position, restore)?;
            }
            None => {
                window.window().set_maximized(restore.maximized);
                window.show()?;
            }
        }
        host.video_texture.replace(texture);
        platform::shown(&window);
        // The windows created from here on are its tool windows.
        platform::set_main_window(&window);
        frame_pacing::follow(&window);
        // Interpolation (`video.render()`) resamples to the main display's rate.
        let displays = platform::displays();
        let main_display = displays.iter().find(|d| d.primary).or(displays.first());
        host.core
            .backend
            .render
            .set_refresh(main_display.and_then(|d| d.refresh_rate));
        host.native_before = native_before.clone();
        host.native_window = native_before
            .as_ref()
            .and_then(|before| platform::associate_new(before).ok());
        // The interface's first copies of the tables (rule R8): the host's
        // commands, the overlay and what start-up registered.
        host.sync_tables();
        let host = Rc::new(RefCell::new(host));
        host.borrow().bind_shell(&window, 1)?;
        host.borrow().bind_tabs(&window, 1)?;
        host.borrow().bind_window_events(&window, 1);
        host.borrow_mut().theme_view(&window);
        host.borrow_mut().shell = Some(window.clone_strong());
        host.borrow().video_target.replace(window.as_weak());
        // Only `host.shell` keeps the main shell: a composition change
        // replaces it (`view::replace_shell`) and its tree must be freed.
        drop(window);
        // Render cadence: each new frame wakes the event loop (coalesced)
        // instead of waiting for the 16 ms tick.
        let frames = host.borrow().core.backend.frame.clone();
        let frame_host = host.clone();
        let busy = frames.clone();
        FRAME_SINK.with(|sink| {
            *sink.borrow_mut() = Some(Box::new(move || match frame_host.try_borrow_mut() {
                Ok(mut host) => host.show_frame(),
                Err(_) => busy.miss(),
            }))
        });
        frame_pacing::on_deliver(Some(Box::new(|| {
            FRAME_SINK.with(|sink| {
                if let Some(deliver) = &*sink.borrow() {
                    deliver();
                }
            });
            // A GPU frame a window without a notifier refused (Windows).
            platform::update_gpu_frames();
        })));
        let resize_host = host.clone();
        RESIZE_SINK.with(|sink| {
            *sink.borrow_mut() = Some(Box::new(move || {
                // Busy (a tick runs): that tick or the next follows the area.
                if let Ok(mut host) = resize_host.try_borrow_mut() {
                    host.resized();
                }
            }))
        });
        let pip_host = host.clone();
        PIP_GPU_SINK.with(|sink| {
            *sink.borrow_mut() = Some(Box::new(move || {
                if let Ok(mut host) = pip_host.try_borrow_mut() {
                    host.pip_gpu_frame();
                }
            }))
        });
        frames.set_waker(Box::new(|| {
            let _ = slint::invoke_from_event_loop(|| {
                FRAME_SINK.with(|sink| {
                    if let Some(deliver) = &*sink.borrow() {
                        deliver();
                    }
                })
            });
        }));
        // CPU mode's Direct3D 12 windows (doc/architecture/README.md §4.3): the crash
        // guard counts a crash only while they are made, draw their first
        // frames and take the first video picture (a start with a file, or
        // a source loading), not one unrelated to the GPU later in the first
        // 5 s of a start; the 5 s timer below stays the backstop.
        let mut first_draws = (!render.is_gpu() && render.windows == render_mode::WindowKind::Wgpu)
            .then(|| guard.clone());
        let tick_host = host.clone();
        let timer = slint::Timer::default();
        let mut menu_revision = u64::MAX;
        let mut binding_revision = u64::MAX;
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(16),
            move || {
                let began = Instant::now();
                // A slow turn, or a gap before it, goes to the host log
                // (`host/stall.rs`).
                host::stall::tick_began(began);
                let mut turn = host::stall::Turn::new(began);
                let mut host = tick_host.borrow_mut();
                host::drain_events(&receiver, began, 0, |event| {
                    let (label, at) = (event.label(), Instant::now());
                    host.handle(event);
                    turn.event(label, at.elapsed());
                });
                turn.step("events");
                // The UI part of the turn starts here. The UI stall test
                // switch holds it once (`ui_stall_switch`).
                if let Some((due, length)) = ui_stall
                    && began >= due
                {
                    ui_stall = None;
                    diag!("test: UI stall of {} ms began", length.as_millis());
                    std::thread::sleep(length);
                    diag!("test: UI stall ended");
                    turn.step("test UI stall");
                }
                if first_draws.is_some() {
                    let (draws, pictures) = host.core.backend.render.presentation.counts();
                    let opening = opened_at_start || host.core.last_source["phase"] == "loading";
                    if playback::render_state::windows_started(draws, pictures, opening)
                        && let Some(guard) = first_draws.take()
                    {
                        guard.settle();
                    }
                    turn.step("crash guard first draws");
                }
                host.placements.tick();
                host.mount_callbacks();
                host.prepare_mount_tick();
                turn.step("mounts");
                host.panel_restart_tick();
                turn.step("panel restarts");
                host.window_tick();
                turn.step("window tick");
                platform::tool_windows_tick();
                turn.step("tool windows");
                host.activation_tick();
                host.on_demand_tick();
                turn.step("plugin activation");
                host.power_tick();
                host.diagnostics_tick();
                turn.step("power and diagnostics");
                host.package_tick();
                turn.step("packages");
                host.notify_tick();
                host.timeline_tick();
                host.drops_tick();
                host.open_files_tick();
                turn.step("notifications, timeline, drops, open files");
                host.settings_tick();
                turn.step("settings");
                if host.theme_pending {
                    host.theme_all();
                    turn.step("theme");
                }
                // The core's step: the binding table follows the registry's
                // disabled commands (keys bound to them pass on).
                if host.core.commands.revision() != binding_revision {
                    binding_revision = host.core.commands.revision();
                    let disabled = host.core.commands.disabled();
                    // Only a change is a write: the interface's copy of the
                    // table is made again only then (`host/tables.rs`).
                    if *host.core.bindings().disabled() != disabled {
                        host.core.bindings_mut().set_disabled(disabled);
                    }
                }
                // The interface's step, on its copy of the registry (rule
                // R8, `host/tables.rs`). The revision alone, not a snapshot:
                // a snapshot builds the whole menu tree, longer than a frame
                // with many plugins. A closed menu is built when it opens
                // (`open_menu`).
                host.sync_tables();
                if host.shell.is_some() {
                    let revision = host.registry.revision();
                    if revision != menu_revision {
                        menu_revision = revision;
                        if host.menu_open
                            && let Err(error) = host.refresh_menu()
                        {
                            host.broadcast(
                                "host.error",
                                json!({"code":"menu-projection","message":error.to_string()}),
                            );
                        }
                    }
                }
                turn.step("bindings and menu");
                // The renderer's wake-up delivers frames; this only catches
                // one whose wake-up found the host busy.
                if host.core.backend.frame.missed() {
                    host.show_frame();
                }
                frame_pacing::tick();
                turn.step("frame");
                // The key and mouse callbacks that Slint runs before the
                // next tick read the tables as this tick left them (R8).
                host.sync_tables();
                if let Some(log) = host.frame_log.as_mut() {
                    log.tick_took(began.elapsed());
                }
                host::stall::tick_ended(&turn, |owner| host.core.plugin_id(owner));
                if seconds.is_some_and(|seconds| start.elapsed().as_secs_f64() >= seconds) {
                    if let Some(path) = snapshot.as_ref()
                        && let Some(window) = host.shell.as_ref()
                    {
                        let save = || -> Result<()> {
                            let pixels = window.window().take_snapshot()?;
                            let mut encoder = png::Encoder::new(
                                std::fs::File::create(path)?,
                                pixels.width(),
                                pixels.height(),
                            );
                            encoder.set_color(png::ColorType::Rgba);
                            encoder.set_depth(png::BitDepth::Eight);
                            encoder
                                .write_header()?
                                .write_image_data(pixels.as_bytes())?;
                            Ok(())
                        };
                        if let Err(error) = save() {
                            diag!("snapshot failed: {error}");
                        }
                    }
                    let _ = slint::quit_event_loop();
                }
            },
        );
        // GPU rendering that ran this long started: settle the crash guard.
        let started = guard.clone();
        slint::Timer::single_shot(Duration::from_secs(5), move || started.settle());
        // The loop also ends with an error when the windows' GPU device was
        // lost (a driver reset): tear down as usual first. A draw on the
        // lost device may panic in Slint's renderer before the loop sees the
        // quit; that panic ends the loop too (`run_through_loss`), so the
        // teardown below still runs.
        let ran = run_through_loss(slint::run_event_loop, platform::window_device_lost)
            .unwrap_or_else(|| {
                restart_logged(
                    "render: the event loop panicked after the GPU device loss (see the panic: line); the event loop ended, the teardown runs",
                );
                Ok(())
            });
        // The steps below name themselves (`teardown_step`): a player ended
        // because its teardown after a device loss took too long says in
        // restart.log where it was. After a device loss the teardown has 5 s
        // from here (`host/restart.rs`).
        event_loop_ended();
        teardown_step("releasing the GPU renderer's fences");
        // However the loop ended (Quit, the last window closed, a restart):
        // release the GPU renderer's fences before the windows, their
        // device and the renderer go, so a stalled frame cannot hold up the
        // teardown.
        platform::release_gpu_waits();
        // Every plugin process stops within one deadline (supervisor.md
        // §9: 1 s at quit, 300 ms after a device loss), each running
        // plugin recorded first.
        teardown_step("stopping the plugins");
        let budget = if platform::window_device_lost() {
            host::DEVICE_LOSS_BUDGET
        } else {
            host::QUIT_BUDGET
        };
        host.borrow_mut().shutdown_plugins(budget);
        stop_handoffs();
        // Read now: only a loss that ended the loop restarts the player.
        if platform::window_device_lost() {
            // Standard practice is a new device; Slint cannot give its
            // windows one, so the player starts again where it was (already
            // started from the device's callback; this is the fallback).
            teardown_step("saving the UI state");
            host.borrow_mut().ui_state.flush();
            let source = host.borrow().core.last_source.clone();
            // Plugin shaders stay `applying` if they were: a device lost
            // while they were on keeps them off in the restarted player.
            logged!("exit: restarting, the windows' GPU device was lost");
            restart_player(&lost_device, &source);
        } else {
            guard.settle();
            host.borrow_mut().core.shader_guard.quit();
            if let Err(error) = &ran {
                logged!("exit: the event loop failed ({error})");
            }
            ran?;
            logged!("exit: normal");
        }
        teardown_step("stopping frame pacing");
        timer.stop();
        frame_pacing::stop();
        frame_pacing::on_deliver(None);
        FRAME_SINK.with(|sink| sink.borrow_mut().take());
        PIP_GPU_SINK.with(|sink| sink.borrow_mut().take());
        RESIZE_SINK.with(|sink| sink.borrow_mut().take());
        host.borrow_mut().ui_state.flush();
        // Native windows/resources must be released before process teardown.
        teardown_step("closing the secondary windows");
        host.borrow_mut().close_secondary_windows();
        // `run()` used to hide the window it showed; the current shell now.
        let shell = host.borrow_mut().shell.take();
        if let Some(shell) = shell {
            teardown_step("hiding the main window");
            let _ = shell.hide();
        }
        // The host (the backend with mpv and its renderer, the plugins)
        // drops here, inside this block that wrapped it in an `Rc`.
        teardown_step("dropping the windows, mpv and the plugins");
        drop(host);
    }
    // Headless: the host drops when `main` returns.
    teardown_step("dropping the windows, mpv and the plugins");
    Ok(())
}
