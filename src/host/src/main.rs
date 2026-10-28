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
mod commands;
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
mod system;
mod timeline;
pub mod ui;

use anyhow::{Result, bail};
use host::package_activation;
use host::{Event, FRAME_SINK, Host, InputShared, MenuMode, PIP_GPU_SINK};
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
fn main() -> Result<()> {
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
    if let Some(parent) = restart {
        diag!("restart: this player replaces process {parent}");
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
    // The renderer pair and why (round record 2026-10-render-modes): GPU
    // rendering (SDK site, Concepts: render options) needs Slint's OpenGL
    // renderer (femtovg); headless, `VESTRA_RENDER`, the saved preference,
    // the crash guard, the driver denylist and systems without a GPU
    // renderer can make it software.
    let startup = host::rendering_startup(headless);
    let mut render = render_mode::decide(&startup.facts, startup.preference);
    diag!("{}", render.line());
    for note in &render.notes {
        diag!("{note}");
    }
    // The crash guard: armed until GPU rendering ran a few seconds, gave up
    // on its own, or the player quit (`render_state`).
    // `_armed` lives until `main` returns, by any path: only a process
    // that dies leaves the guard armed.
    let guard = startup.guard.clone();
    let _armed = render
        .is_gpu()
        .then(|| guard.arm(startup.key.clone(), playback::render_state::now()));
    if !headless {
        // Windows: the main window draws at the display's vertical blanks
        // (`frame_pacing.rs`), not on Slint's timer.
        let winit = || {
            let selector = slint::BackendSelector::new().backend_name("winit".into());
            if frame_pacing::wanted() {
                selector.with_winit_custom_application_handler(frame_pacing::Handler)
            } else {
                selector
            }
        };
        match render.windows {
            render_mode::WindowKind::OpenGl => {
                if let Err(error) = winit().renderer_name("femtovg".into()).select() {
                    render = render.fall_back(render_mode::Reason::InitFailed(format!(
                        "the OpenGL window renderer is unavailable: {error}"
                    )));
                    diag!("{}", render.line());
                    guard.settle();
                    winit().select()?;
                }
            }
            render_mode::WindowKind::Default => winit().select()?,
            // A driver whose OpenGL cannot draw the windows (denylist).
            render_mode::WindowKind::Software => {
                winit().renderer_name("software".into()).select()?;
            }
        }
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
    process::set_app_info(&json!({"version":env!("CARGO_PKG_VERSION"),
        "build":option_env!("VESTRA_BUILD"),
        "components":{"mpv":backend.versions["mpv"],
            "libav":if ffmpeg.is_string() { json!({"ffmpeg":ffmpeg}) } else { json!({}) },
            "slint":env!("VESTRA_SLINT_VERSION")}}));
    let (mount_events, mount_input) = mpsc::sync_channel(64);
    let mut host = Host {
        backend,
        shared: None,
        shared_policy: Default::default(),
        plugins: BTreeMap::new(),
        views: BTreeMap::new(),
        events,
        next_generation: 0,
        headless,
        dialog_busy: false,
        shell: None,
        mounts: BTreeMap::new(),
        mount_queue: std::collections::VecDeque::new(),
        preparing: None,
        mount_singly: 0,
        mount_events,
        mount_input,
        last_image: None,
        video_texture: RefCell::new(None),
        video_target: Rc::default(),
        shell_generation: 1,
        next_shell_generation: 1,
        menu_path: Vec::new(),
        menu_scroll: Vec::new(),
        menu_aim: menu::HoverAim::default(),
        menu_panels: Vec::new(),
        policy: window_state::Policy::new(0),
        clock: Instant::now(),
        window_facts: Value::Null,
        measured_shell: None,
        menu_open: false,
        sidebar_popped: false,
        sidebar_active: None,
        tab_clicked: false,
        shell_tabs: Vec::new(),
        bottom_shown: true,
        native_window: None,
        native_before: None,
        native_unavailable: None,
        input: Rc::new(RefCell::new(InputShared::default())),
        ui_state: if headless {
            ui_state::Store::disabled()
        } else {
            ui_state::Store::open(&ui_state::default_path())
        },
        osd_until: None,
        info_panels: BTreeMap::new(),
        sequence: None,
        last_source: Value::Null,
        rendering: host::Rendering {
            facts: startup.facts,
            started: render.mode,
            preference: startup.preference,
            file: startup.file,
            relaunch: host::Relaunch {
                directories: directories.clone(),
                silent,
            },
            restarted_at: restart.map(|_| Instant::now()),
        },
        source_state_replies: Default::default(),
        floating: None,
        menu_mode: MenuMode::Main,
        menu_generation: 1,
        host_revision: 0,
        window_title: "Vestra Player".into(),
        activations: package_activation::Book::default(),
        activation_ui: Default::default(),
        activation_receipts: Default::default(),
        file_types: json!([]),
        held_mounts: Default::default(),
        warnings: BTreeMap::new(),
        packages: package_manager::Manager::open()?,
        cleanup: Default::default(),
        power: power::Power::system(),
        last_frame: None,
        sizing: Default::default(),
        frame_log: render_target::FrameLog::from_env(),
        shown_size: None,
        actions_busy: Default::default(),
        step_seek: BTreeMap::new(),
        step_writes: 0,
        on_top: window_control::LastWrite::new(false),
        cursor_playing: window_control::LastWrite::new(false),
        on_top_applied: None,
        area_replies: Vec::new(),
        development: BTreeMap::new(),
        dev_disabled: Default::default(),
        failures: BTreeMap::new(),
        data_removals: BTreeMap::new(),
        session_storage: Default::default(),
        bindings_sent: 0,
        packages_sent: Value::Null,
        notified_at: 0,
        timeline: Default::default(),
        images: Default::default(),
        no_video: true,
        video_absent: true,
        video_cleared: false,
        cover_art_off: false,
        pip: Default::default(),
        pip_log: None,
        render_area: None,
        render_sent: None,
        pip_gpu: Default::default(),
        drops: Default::default(),
        opens: Default::default(),
        settings: Default::default(),
        settings_category: None,
        theme_pending: false,
        placements: view::Placements::default(),
        bar: bar_buttons::Strip::default(),
        commands: commands::Registry::new(),
    };
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
        if core_actions::migrate_overlay(&mut host.ui_state.state.bindings) {
            // Old playback-plugin keys now name the core actions; saved once.
            let now = host.now_ms();
            host.ui_state.touch(now);
            host.ui_state.flush();
        }
        let overlay = host.ui_state.state.bindings.clone();
        host.input.borrow_mut().table.set_overlay(overlay);
        if let Some(sidebar) = &host.ui_state.state.sidebar {
            host.policy.set_sidebar_width(sidebar.width);
        }
    }
    for directory in directories {
        host.load(&directory, 120_000)?;
    }
    // A restart continues its first file where the replaced player was.
    if let (Some(resume), Some(file)) = (resume, files.first()) {
        let _ = host.backend.commands.send(playback::backend::Command {
            reply: host::Reply { owner: 0, id: 0 },
            method: "host.resume".into(),
            params: json!({"source":file.to_string_lossy(),"seconds":resume.seconds,"paused":resume.paused}),
        });
    }
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
            match received {
                Ok(event) => {
                    if let Event::Broadcast(name, data) = &event
                        && name == "source.state"
                    {
                        observed_source = data.clone();
                    }
                    host.handle(event)
                }
                Err(mpsc::RecvTimeoutError::Timeout) => (),
                Err(_) => break,
            }
            host.activation_tick();
            host.power_tick();
            host.package_tick();
            host.notify_tick();
            host.timeline_tick();
            host.open_files_tick();
            if let Some(log) = host.frame_log.as_mut() {
                log.tick_took(began.elapsed());
                let presentation = &host.backend.render.presentation;
                let (produced, shown) = host.backend.frame.counts();
                let (draws, presented) = presentation.counts();
                let counts = render_target::LogCounts {
                    produced,
                    shown,
                    draws,
                    presented,
                };
                if let Some(line) = log.line(counts, presentation, || None, None, Instant::now()) {
                    diag!("{line}{}", host.backend.frame.stages.text());
                }
            }
            if let Some(evidence) = evidence.as_mut() {
                evidence.sample(&host.backend, &observed_source, start)?;
            }
        }
        stop_handoffs();
    } else {
        let native_before = platform::capture_windows().ok();
        let window = composition::compile_json(&[], host.mount_events.clone(), 1)?;
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
        match position {
            Some(position) => host.placements.show_at(&window, position)?,
            None => window.show()?,
        }
        host.video_texture.replace(texture);
        platform::shown(&window);
        frame_pacing::follow(&window);
        // Interpolation (`video.render()`) resamples to the main display's rate.
        let displays = platform::displays();
        let main_display = displays.iter().find(|d| d.primary).or(displays.first());
        host.backend
            .render
            .set_refresh(main_display.and_then(|d| d.refresh_rate));
        host.native_before = native_before.clone();
        host.native_window = native_before
            .as_ref()
            .and_then(|before| platform::associate_new(before).ok());
        let host = Rc::new(RefCell::new(host));
        host.borrow().bind_shell(&window, 1)?;
        host.borrow().bind_window_events(&window, 1);
        host.borrow_mut().theme_view(&window);
        host.borrow_mut().shell = Some(window.clone_strong());
        host.borrow().video_target.replace(window.as_weak());
        // Only `host.shell` keeps the main shell: a composition change
        // replaces it (`view::replace_shell`) and its tree must be freed.
        drop(window);
        // Render cadence: each new frame wakes the event loop (coalesced)
        // instead of waiting for the 16 ms tick.
        let frames = host.borrow().backend.frame.clone();
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
            })
        })));
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
        let tick_host = host.clone();
        let timer = slint::Timer::default();
        let mut menu_revision = u64::MAX;
        let mut binding_revision = u64::MAX;
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(16),
            move || {
                let began = Instant::now();
                let mut host = tick_host.borrow_mut();
                for event in receiver.try_iter().take(128) {
                    host.handle(event);
                }
                host.placements.tick();
                host.mount_callbacks();
                host.prepare_mount_tick();
                host.window_tick();
                host.activation_tick();
                host.power_tick();
                host.package_tick();
                host.notify_tick();
                host.timeline_tick();
                host.drops_tick();
                host.open_files_tick();
                host.settings_tick();
                if host.theme_pending {
                    host.theme_all();
                }
                if host.commands.revision() != binding_revision {
                    binding_revision = host.commands.revision();
                    let disabled = host.commands.disabled();
                    host.input.borrow_mut().table.set_disabled(disabled);
                }
                if host.shell.is_some() {
                    let revision = host.commands.snapshot().revision;
                    if revision != menu_revision {
                        menu_revision = revision;
                        if let Err(error) = host.refresh_menu() {
                            host.broadcast(
                                "host.error",
                                json!({"code":"menu-projection","message":error.to_string()}),
                            );
                        }
                    }
                }
                // The renderer's wake-up delivers frames; this only catches
                // one whose wake-up found the host busy.
                if host.backend.frame.missed() {
                    host.show_frame();
                }
                frame_pacing::tick();
                if let Some(log) = host.frame_log.as_mut() {
                    log.tick_took(began.elapsed());
                }
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
        let ran = slint::run_event_loop();
        stop_handoffs();
        guard.settle();
        ran?;
        timer.stop();
        frame_pacing::stop();
        frame_pacing::on_deliver(None);
        FRAME_SINK.with(|sink| sink.borrow_mut().take());
        PIP_GPU_SINK.with(|sink| sink.borrow_mut().take());
        host.borrow_mut().ui_state.flush();
        // Native windows/resources must be released before process teardown.
        host.borrow_mut().close_secondary_windows();
        // `run()` used to hide the window it showed; the current shell now.
        let shell = host.borrow_mut().shell.take();
        if let Some(shell) = shell {
            let _ = shell.hide();
        }
    }
    Ok(())
}
