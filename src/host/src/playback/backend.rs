//! One persistent libmpv core. Rendering and all control calls stay off the UI thread.
use crate::host::{Event, Reply, RpcError};
use crate::playback::renderer::GpuBackend as _;
use anyhow::Result;
use rsmpv::{
    Mpv,
    render::{RenderContext, SwPixelFormat},
};
use serde_json::{Value, json};
#[cfg(feature = "producer-audio")]
use std::sync::Mutex;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

pub use crate::playback::render_target::Frame;
use crate::playback::render_target::{FrameSlot, Redraw, Shared};
use slint::SharedPixelBuffer;
#[cfg(feature = "producer-audio")]
/// The cleared canvas a stopped producer leaves behind.
const CLEAR: (u32, u32) = (960, 540);
pub struct Command {
    pub reply: Reply,
    pub method: String,
    pub params: Value,
}
/// How often the control worker looks while something it serves ends
/// without an mpv event or a command (`Sources::polling`,
/// `Geometry::polling`, `CoreActions::pending`): the poll it always ran
/// before 0.5.15, with `thread::sleep` (a condvar timeout is rounded up to
/// the system timer tick on Windows, about 15.6 ms).
const POLL: Duration = Duration::from_millis(2);
/// Commands the control worker takes per turn (continuous controls must not
/// starve the rest); after a full turn it looks again after `POLL`.
const COMMANDS_PER_TURN: usize = 32;

/// Wakes the control worker: a command, mpv's wake-up callback (new
/// events), the stop, the render thread leaving GPU rendering. A wake-up
/// before the wait is kept, so none is lost.
#[derive(Default)]
pub struct Wake {
    woken: std::sync::Mutex<bool>,
    ready: std::sync::Condvar,
}
impl Wake {
    pub fn notify(&self) {
        *self
            .woken
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        self.ready.notify_one();
    }
    /// Block until woken or `timeout`; `true` when woken (the wake-up is
    /// taken).
    pub fn wait(&self, timeout: Duration) -> bool {
        let woken = self
            .woken
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut woken, _) = self
            .ready
            .wait_timeout_while(woken, timeout, |woken| !*woken)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::mem::take(&mut *woken)
    }
}

/// The control worker's command queue: a command sent also wakes it.
pub struct Commands {
    sender: mpsc::Sender<Command>,
    wake: Arc<Wake>,
}
impl Commands {
    pub fn send(&self, command: Command) -> std::result::Result<(), mpsc::SendError<Command>> {
        self.sender.send(command)?;
        self.wake.notify();
        Ok(())
    }
}

pub struct Backend {
    pub commands: Commands,
    /// The newest frame for the video area (C-SIZE / render cadence).
    pub frame: Arc<FrameSlot>,
    /// What size the renderer draws at (UI: area; display layer: zoom).
    pub target: Arc<Shared>,
    #[cfg(feature = "producer-audio")]
    pub input: Arc<Mutex<Option<crate::playback::producer_input::Routing>>>,
    /// GPU or software rendering (SDK site, Concepts: render options).
    pub render: Arc<crate::playback::renderer::Render>,
    /// `{ mpv, ffmpeg }`: the linked libmpv's `mpv-version` / `ffmpeg-version`.
    pub versions: Value,
    stop: mpsc::Sender<()>,
    wake: Arc<Wake>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Backend {
    /// `render`: the start-up decision (`render_mode::decide`); GPU mode tries
    /// GPU rendering first.
    pub fn start(
        events: mpsc::Sender<Event>,
        silent: bool,
        render: &crate::playback::render_mode::Decision,
    ) -> Result<Self> {
        let (commands, receiver) = mpsc::channel::<Command>();
        let wake = Arc::new(Wake::default());
        let commands = Commands {
            sender: commands,
            wake: wake.clone(),
        };
        let worker_wake = wake.clone();
        let (stop, stopping) = mpsc::channel();
        let frame = FrameSlot::new();
        let out_frame = frame.clone();
        let target = Arc::new(Shared::default());
        let render_target = target.clone();
        let view_target = target.clone();
        #[cfg(feature = "producer-audio")]
        let input = Arc::new(Mutex::new(None));
        #[cfg(feature = "producer-audio")]
        let out_input = input.clone();
        let render = crate::playback::renderer::Render::new(render);
        let out_render = render.clone();
        let (ready, initialized) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let run = || -> Result<()> {
                let fonts = crate::font_carrier::FontDirectory::bundled()?;
                let default_subtitle = crate::playback::subtitle_style::default_for(
                    fonts.asset().metadata().font_id.into(),
                );
                let mut builder = Mpv::builder()?
                    .set_property("vo", "libmpv")?
                    // Software rendering's mode; the GPU renderer replaces it.
                    .set_property("hwdec", software_hwdec().as_str())?
                    .set_property(
                        "hwdec-software-fallback",
                        crate::playback::processing::HWDEC_SOFTWARE_FALLBACK,
                    )?
                    .set_property("keep-open", true)?
                    .set_property("sub-auto", "no")?
                    .set_property("vf", crate::playback::source_geometry::INITIAL_FILTERS)?
                    .set_property("sub-fonts-dir", fonts.path().to_string_lossy().as_ref())?
                    .set_property("sub-font-provider", "none")?
                    .set_property("embeddedfonts", false)?;
                for (property, value) in crate::playback::subtitle_style::mpv_properties(
                    &default_subtitle,
                    fonts.asset().metadata().family,
                )? {
                    builder = builder.set_property(property, value.as_str())?;
                }
                if silent {
                    builder = builder.set_property("ao", "null")?;
                }
                no_builtin_scripts(&builder)?;
                let mpv = Arc::new(builder.build()?);
                // New events wake the worker (it drains them with
                // `poll_event`); the callback only signals.
                let event_wake = worker_wake.clone();
                mpv.set_wakeup_callback(move || event_wake.notify());
                mpv.request_log_messages("v")?;
                crate::playback::processing::observe_hwdec(&mpv)?;
                // A pause from anywhere (a plugin's `source.submit`, a key,
                // keep-open at the end) wakes the worker and goes out at
                // once (`StatePush::paused`), not on the next 100 ms push.
                mpv.observe_property(PAUSE_OBSERVATION, "pause", rsmpv::Format::Flag)?;
                let version = |name: &str| mpv.get_property::<String>(name).ok();
                let versions =
                    json!({"mpv":version("mpv-version"),"ffmpeg":version("ffmpeg-version")});
                let sequence = Arc::new(AtomicU64::new(0));
                let render_stop = Arc::new(AtomicBool::new(false));
                let publish_gate = Arc::new(AtomicBool::new(true));
                let renderer_publish = publish_gate.clone();
                let render_core = mpv.clone();
                let render_sequence = sequence.clone();
                let renderer_stop = render_stop.clone();
                let render_events = events.clone();
                let (render_ready, render_initialized) = mpsc::sync_channel(1);
                let render_frame = out_frame.clone();
                let render_mode = out_render.clone();
                // Everything the software renderer waits for wakes it: mpv's
                // update callback, a new target (UI: area; display layer:
                // zoom), the stop. One slot: wake-ups coalesce, none is lost.
                let (render_wake, render_woke) = mpsc::sync_channel::<()>(1);
                let target_wake = render_wake.clone();
                render_target.set_waker(Box::new(move || {
                    let _ = target_wake.try_send(());
                }));
                let stop_wake = render_wake.clone();
                let switched = worker_wake.clone();
                let renderer = thread::spawn(move || {
                    let run = || -> Result<()> {
                        let mut ready = Some(&render_ready);
                        if render_mode.wanted() {
                            let area = render_target.get().1.area;
                            let gpu = GpuRun {
                                mpv: &render_core,
                                render: &render_mode,
                                publish: &renderer_publish,
                                stop: &renderer_stop,
                            };
                            if gpu.run(area, &mut ready) {
                                return Ok(());
                            }
                            // The control worker sets software rendering's
                            // `hwdec` when it sees the switch: now, not at
                            // its next state push.
                            switched.notify();
                        }
                        let mut render = RenderContext::new_software(render_core.clone())?;
                        let wake = render_wake;
                        // Callback only signals. It never calls into libmpv.
                        render.set_update_callback(move || {
                            let _ = wake.try_send(());
                        });
                        if let Some(ready) = ready.take() {
                            let _ = ready.send(Ok(()));
                        }
                        let mut redraw = Redraw::default();
                        let mut buffers = crate::playback::render_target::BufferRing::default();
                        let mut source: Option<(u32, u32)> = None;
                        let mut source_read: Option<Instant> = None;
                        let mut last_size = None;
                        let stages = &render_frame.stages;
                        while !renderer_stop.load(Ordering::Relaxed) {
                            let update = render.update();
                            if update {
                                stages.update();
                            }
                            let (revision, request) = render_target.get();
                            if redraw.due(update, revision) {
                                // The source display size changes only with a
                                // new load or filter; read it a few times a second.
                                if source_read
                                    .is_none_or(|at| at.elapsed() >= Duration::from_millis(200))
                                {
                                    source = source_size(&render_core);
                                    source_read = Some(Instant::now());
                                }
                                let (width, height) = crate::playback::render_target::forced()
                                    .unwrap_or_else(|| {
                                        crate::playback::render_target::target(
                                            request.area,
                                            source,
                                            request.zoom,
                                        )
                                    });
                                // Reused buffers: a new one per frame costs more
                                // than mpv's render at 4K (`BufferRing`).
                                let mut buffer = buffers.take(width, height);
                                // The frame's display time: mpv waits for it
                                // inside the render call (render.h,
                                // `MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME`,
                                // on by default), so the call takes about one
                                // frame interval; the work is what remains.
                                let target = render
                                    .next_frame_info()
                                    .ok()
                                    .map(|info| info.target_time)
                                    .filter(|time| *time > 0);
                                let began_us = render_core.get_time_us();
                                let began = Instant::now();
                                let held = buffer.as_bytes().as_ptr();
                                let pixels = buffer.make_mut_bytes();
                                let copied = pixels.as_ptr() != held;
                                render.render_software(
                                    width as i32,
                                    height as i32,
                                    SwPixelFormat::Rgb24,
                                    width as usize * 3,
                                    pixels,
                                )?;
                                let took = began.elapsed();
                                buffers.keep(&buffer);
                                let resized = last_size
                                    .replace((width, height))
                                    .is_some_and(|last| last != (width, height));
                                let waited = target
                                    .filter(|time| *time > began_us)
                                    .map_or(Duration::ZERO, |time| {
                                        Duration::from_micros((time - began_us) as u64)
                                    });
                                stages.call(
                                    update,
                                    resized,
                                    copied,
                                    took,
                                    took.saturating_sub(waited),
                                );
                                // Consume safe normalized frames offscreen while source
                                // orientation is prepared; never publish an unrotated preview.
                                if renderer_publish.load(Ordering::Acquire) {
                                    let sequence =
                                        render_sequence.fetch_add(1, Ordering::Relaxed) + 1;
                                    render_frame.put(Frame { buffer, sequence });
                                }
                            }
                            // No timeout: nothing here is due at a time of its
                            // own (mpv waits for a frame's display time inside
                            // the render call). Before 0.5.15 this polled
                            // every 5 ms (about 200 wake-ups a second).
                            if render_woke.recv().is_err() {
                                break;
                            }
                        }
                        Ok(())
                    };
                    if let Err(error) = run() {
                        let message = error.to_string();
                        let _ = render_ready.send(Err(message.clone()));
                        let _ = render_events.send(Event::Broadcast(
                            "host.error".into(),
                            json!({"message":message}),
                        ));
                    }
                });
                render_initialized.recv()?.map_err(anyhow::Error::msg)?;
                let _ = ready.send(Ok(versions));
                let mut sources = crate::playback::source_backend::Sources::default();
                sources.register_lease_streams(&mpv);
                // Pushed as `resolvers.fileTypes` when a file resolver comes or goes.
                let mut file_types = Vec::<String>::new();
                let mut geometry =
                    crate::playback::source_geometry::Geometry::new(publish_gate.clone());
                let mut geometry_error = false;
                #[cfg(feature = "producer-audio")]
                let mut producer_was_active = false;
                let mut processing = crate::playback::processing::Processing::default();
                // `VESTRA_FRAME_LOG`: mpv's dropped and late frames for the line.
                let frame_log = std::env::var("VESTRA_FRAME_LOG").as_deref() == Ok("1");
                let mut view = crate::playback::video_view::View::default();
                let mut render_options = crate::playback::video_render::Options::default();
                let mut shaders = crate::playback::video_shaders::Channel::default();
                let mut render_was_gpu = out_render.gpu().is_some();
                let mut cover_art = CoverArt::default();
                let mut subtitles = crate::playback::subtitle_backend::Subtitles::new(fonts);
                let mut jobs = crate::playback::media_jobs::Jobs::default();
                let mut push = StatePush::new();
                let mut actions = CoreActions::default();
                let mut upscaling = crate::playback::upscaling::Upscaling::default();
                let upscaling_wake = worker_wake.clone();
                upscaling.wake_with(move || upscaling_wake.notify());
                loop {
                    if stopping.try_recv().is_ok() {
                        break;
                    }
                    for (owner, event, data) in sources.resolver_events() {
                        let _ = events.send(Event::PluginMessage(owner, event, data));
                    }
                    jobs.poll();
                    upscaling.poll(&mpv, &out_render, &events, || processing.copy_back_owners());
                    processing.note_source(sources.generation(), sources.phase());
                    subtitles.note_source(sources.generation(), sources.phase());
                    // Bound work per iteration so continuous controls cannot starve video.
                    let mut taken = 0;
                    for command in receiver.try_iter().take(COMMANDS_PER_TURN) {
                        taken += 1;
                        #[cfg(feature = "producer-audio")]
                        if command.method == "host.physicalInput" {
                            if let Err(error) = sources.host_input(&command.params)
                                && !matches!(error.code.as_str(), "unavailable" | "stale" | "busy")
                            {
                                let _ = events.send(Event::Broadcast(
                                    "host.error".into(),
                                    json!({"code":error.code,"message":error.message}),
                                ));
                            }
                            continue;
                        }
                        // Video upscaling (`upscaling.rs`): its requests; a
                        // stopped plugin's filter goes, then on below. The
                        // chains that switched decoding to copy-back are
                        // named in a `decoding` reason, as they are now.
                        if command.method.starts_with("video.upscaling.") {
                            upscaling.note_copy_back(processing.copy_back_owners());
                        }
                        if upscaling.command(&mpv, &out_render, &command, &events) {
                            continue;
                        }
                        // Lazy activation (host/lazy.rs): what a plugin
                        // holds here, its resolvers kept while it is parked,
                        // moved when it wakes, pruned at its ready.
                        match command.method.as_str() {
                            "owner.resources" => {
                                let owner = command.reply.owner;
                                let mut live = sources.holds(owner);
                                if jobs.holds(owner) {
                                    live.push("a snapshot or a job");
                                }
                                if processing.holds(owner) {
                                    live.push("a processing chain");
                                }
                                if subtitles.holds(owner) {
                                    live.push("a subtitle track or style");
                                }
                                if view.owner() == Some(owner) {
                                    live.push("the video view");
                                }
                                if render_options.owner() == Some(owner) {
                                    live.push("the video render options");
                                }
                                if shaders.holds(owner) {
                                    live.push("video shaders");
                                }
                                if upscaling.holds(owner) {
                                    live.push("video upscaling");
                                }
                                let answer = json!({"live":live,
                                    "resolvers":sources.resolvers_of(owner)});
                                let _ = events.send(Event::OwnerResources(owner, answer));
                                continue;
                            }
                            "owner.rekey" => {
                                if let Some(to) = command.params["to"].as_u64() {
                                    sources.rekey(command.reply.owner, to);
                                }
                                continue;
                            }
                            "owner.prune" => {
                                sources.prune(command.reply.owner);
                                continue;
                            }
                            "owner.replay" => {
                                if let Err(error) =
                                    sources.replay_resolver(command.reply.owner, &command.params)
                                {
                                    diag!(
                                        "lazy: replayed resolver of {}: {}",
                                        command.reply.owner,
                                        error.message
                                    );
                                }
                                continue;
                            }
                            _ => (),
                        }
                        if matches!(command.method.as_str(), "owner.disabled" | "owner.parked") {
                            jobs.disable(command.reply.owner);
                            if command.method == "owner.parked" {
                                sources.park(command.reply.owner);
                            } else {
                                sources.disable(command.reply.owner);
                            }
                            if let Err(error) = subtitles.disable(&mpv, command.reply.owner) {
                                let _ = events.send(Event::Broadcast(
                                    "host.error".into(),
                                    json!({"code":error.code,"message":error.message}),
                                ));
                            }
                            if let Err(error) = processing.disable(&mpv, command.reply.owner) {
                                let _ = events.send(Event::Broadcast(
                                    "host.error".into(),
                                    json!({"code":error.code,"message":error.message}),
                                ));
                            }
                            // Its chains are gone (decoding may be zero-copy
                            // again): the upscaling state without them.
                            upscaling.stopped(
                                &mpv,
                                &out_render,
                                &events,
                                processing.copy_back_owners(),
                            );
                            match view.release(
                                command.reply.owner,
                                crate::playback::video_view::mpv_writer(&mpv),
                            ) {
                                Ok(true) => view_target.set_zoom(1.),
                                Ok(false) => (),
                                Err(error) => {
                                    let _ = events.send(Event::Broadcast(
                                        "host.error".into(),
                                        json!({"code":error.code,"message":error.message}),
                                    ));
                                }
                            }
                            if let Err(error) = render_options.release(
                                command.reply.owner,
                                &mut crate::playback::video_render::Mpv(&mpv),
                            ) {
                                let _ = events.send(Event::Broadcast(
                                    "host.error".into(),
                                    json!({"code":error.code,"message":error.message}),
                                ));
                            }
                            if let Err(error) = shaders.disable(
                                command.reply.owner,
                                &mut render_options,
                                &mut crate::playback::video_render::Mpv(&mpv),
                            ) {
                                let _ = events.send(Event::Broadcast(
                                    "host.error".into(),
                                    json!({"code":error.code,"message":error.message}),
                                ));
                            }
                            pace(&out_render, render_options.settings());
                            let _ = events.send(Event::BackendCleaned(command.reply.owner));
                            continue;
                        }
                        if command.method == "owner.registered" {
                            if let Some(id) = command.params["manifestId"].as_str() {
                                let name = command.params["manifestName"].as_str();
                                processing.register(
                                    command.reply.owner,
                                    id.into(),
                                    name.map(Into::into),
                                );
                                subtitles.register(command.reply.owner, id.into());
                                jobs.register(command.reply.owner);
                            }
                            continue;
                        }
                        if command.method.starts_with("snapshot.")
                            || command.method.starts_with("jobs.")
                        {
                            let result = jobs.dispatch(
                                crate::playback::media_jobs::CaptureContext {
                                    sources: &mut sources,
                                    mpv: &mpv,
                                    geometry: &geometry,
                                    processing: &processing,
                                    subtitles: &subtitles,
                                },
                                command.reply.owner,
                                &command.method,
                                &command.params,
                            );
                            let _ = events.send(Event::Reply(command.reply, result));
                            continue;
                        }
                        if command.method.starts_with("subtitles.") {
                            let _ = sources.snapshot(&mpv);
                            subtitles.note_source(sources.generation(), sources.phase());
                            let result = subtitles.dispatch(
                                &mpv,
                                command.reply.owner,
                                &command.method,
                                &command.params,
                            );
                            let _ = events.send(Event::Reply(command.reply, result));
                            continue;
                        }
                        if command.method == "processing.state" {
                            let _ = sources.snapshot(&mpv);
                            processing.note_source(sources.generation(), sources.phase());
                            subtitles.note_source(sources.generation(), sources.phase());
                            let _ =
                                events.send(Event::Reply(command.reply, Ok(processing.state())));
                            continue;
                        }
                        if command.method == "processing.operation" {
                            let result = processing.operation(command.reply.owner, &command.params);
                            let _ = events.send(Event::Reply(command.reply, result));
                            continue;
                        }
                        if command.method == "processing.replace" {
                            let snapshot = sources.snapshot(&mpv);
                            let source = match (
                                snapshot["phase"] == "ready",
                                snapshot["kind"] == "producer",
                            ) {
                                (false, _) => crate::playback::processing::Source::NotReady,
                                (true, true) => crate::playback::processing::Source::Generated,
                                (true, false) => crate::playback::processing::Source::Decoded,
                            };
                            processing.note_source(sources.generation(), sources.phase());
                            subtitles.note_source(sources.generation(), sources.phase());
                            let result = processing.replace(
                                &mpv,
                                command.reply.owner,
                                sources.generation(),
                                source,
                                &command.params,
                            );
                            let _ = events.send(Event::Reply(command.reply, result));
                            continue;
                        }
                        if command.method == "host.coverArt" {
                            let off = command.params["off"].as_bool().unwrap_or(false);
                            if let Err(error) = cover_art.set(off, &mut |name, value| {
                                mpv.set_property(name, value).map_err(|e| e.to_string())
                            }) {
                                let _ = events.send(Event::Broadcast(
                                    "host.error".into(),
                                    json!({"code":"cover-art","message":error}),
                                ));
                            }
                            continue;
                        }
                        if command.method.starts_with("video.view.") {
                            let owner = command.reply.owner;
                            let writer = crate::playback::video_view::mpv_writer(&mpv);
                            let result = if command.method == "video.view.set" {
                                let ready = sources.snapshot(&mpv)["phase"] == "ready";
                                view.set(owner, &command.params["values"], ready, writer)
                            } else {
                                view.release(owner, writer).map(|_| Value::Null)
                            };
                            // The view's zoom also widens the render target (C-SIZE).
                            view_target.set_zoom(view.settings().zoom);
                            let _ = events.send(Event::Reply(command.reply, result));
                            continue;
                        }
                        if command.method.starts_with("video.render.") {
                            let owner = command.reply.owner;
                            let mut player = crate::playback::video_render::Mpv(&mpv);
                            let result = if command.method != "video.render.set" {
                                render_options
                                    .release(owner, &mut player)
                                    .map(|_| Value::Null)
                            } else if out_render.gpu().is_none() {
                                Err(RpcError::new(
                                    "unsupported",
                                    "render options need GPU rendering; the player renders in software",
                                ))
                            } else {
                                let ready = sources.snapshot(&mpv)["phase"] == "ready";
                                let refresh = out_render.refresh();
                                let values = &command.params["values"];
                                render_options.set(owner, values, ready, refresh, &mut player)
                            };
                            pace(&out_render, render_options.settings());
                            let _ = events.send(Event::Reply(command.reply, result));
                            continue;
                        }
                        // The shader channel (`video_shaders.rs`); the
                        // capability layer checked the files.
                        if command.method == "video.shaders.set" {
                            let ready = sources.snapshot(&mpv)["phase"] == "ready";
                            let result = shaders.set(
                                command.reply.owner,
                                &command.params,
                                ready,
                                out_render.gpu().is_some(),
                                &mut render_options,
                                &mut crate::playback::video_render::Mpv(&mpv),
                            );
                            let _ = events.send(Event::Reply(command.reply, result));
                            continue;
                        }
                        // A restarted player continues its first file where
                        // the replaced one was (`host/rendering.rs`): the
                        // next open of that file loads with `start=` and its
                        // pause state (`source_backend::resume_claim`).
                        if command.method == "host.resume" {
                            sources.resume_next_open(
                                crate::playback::source_backend::ResumePoint {
                                    source: command.params["source"]
                                        .as_str()
                                        .unwrap_or_default()
                                        .into(),
                                    seconds: command.params["seconds"].as_f64().unwrap_or(0.),
                                    paused: command.params["paused"].as_bool() == Some(true),
                                },
                            );
                            continue;
                        }
                        if command.method == "playback.stats" {
                            let frames = sequence.load(Ordering::Relaxed)
                                + out_render.gpu().map_or(0, |gpu| gpu.frames());
                            let stats = stats(&mpv, sources.generation(), frames, &out_render);
                            let _ = events.send(Event::Reply(command.reply, Ok(stats)));
                            continue;
                        }
                        let mut context = SourceContext {
                            mpv: &mpv,
                            sources: &mut sources,
                            geometry: &mut geometry,
                            geometry_error: &mut geometry_error,
                            publish_gate: &publish_gate,
                        };
                        if command.method == "host.action" {
                            let action = command.params["action"].as_str().unwrap_or("").to_owned();
                            crate::input_trace::mark(&format!(
                                "playback worker: {action} received"
                            ));
                            let done = Completion::Action(command.reply, action);
                            let result = actions.run(&mut context, &command.params, done);
                            push.soon();
                            processing.note_source(sources.generation(), sources.phase());
                            subtitles.note_source(sources.generation(), sources.phase());
                            if let Some((done, result)) = result {
                                done.send(&events, result);
                            }
                            continue;
                        }
                        if command.method == "player.step" {
                            let done = Completion::Step(command.reply);
                            let result = actions.step(&mut context, &command.params, done);
                            processing.note_source(sources.generation(), sources.phase());
                            subtitles.note_source(sources.generation(), sources.phase());
                            if let Some((done, result)) = result {
                                done.send(&events, result);
                            }
                            continue;
                        }
                        actions.source_request(&command.method, &command.params);
                        let result =
                            context.call(command.reply.owner, &command.method, &command.params);
                        // A pause control (also a provider source's, which
                        // mpv does not observe) goes out at once.
                        if pause_control(&command.method, &command.params) {
                            push.soon();
                        }
                        processing.note_source(sources.generation(), sources.phase());
                        subtitles.note_source(sources.generation(), sources.phase());
                        let _ = events.send(Event::Reply(command.reply, result));
                    }
                    sources.follow_device(&mpv);
                    sources.late_cover_art(&mpv);
                    for (done, result) in actions.poll(&mpv, sources.generation()) {
                        done.send(&events, result);
                    }
                    while let Some(event) = mpv.poll_event() {
                        if let rsmpv::Event::LogMessage(message) = &event {
                            if crate::playback::video_render::compile_line(
                                &message.prefix,
                                &message.level,
                                &message.text,
                            ) {
                                let pushes =
                                    shaders.compile_failure(&mut render_options, &message.text);
                                if !pushes.is_empty() {
                                    diag!("libmpv: {}", message.text.trim_end());
                                }
                                for (owner, event, status) in pushes {
                                    let _ = events.send(Event::PluginMessage(
                                        owner,
                                        event.into(),
                                        status,
                                    ));
                                }
                            }
                            if message.prefix == "sub/ass"
                                && (message.text.contains("fontselect")
                                    || message.text.contains("Loading font"))
                            {
                                diag!("libmpv {}: {}", message.prefix, message.text.trim_end());
                            }
                            continue;
                        }
                        processing.hwdec_event(&mpv, &event);
                        if let rsmpv::Event::PropertyChange { userdata, .. } = &event
                            && *userdata == crate::playback::processing::HWDEC_OBSERVATION
                        {
                            continue;
                        }
                        if let rsmpv::Event::PropertyChange { userdata, data, .. } = &event
                            && *userdata == PAUSE_OBSERVATION
                        {
                            if let rsmpv::PropertyData::Flag(paused) = data {
                                push.paused(*paused);
                            }
                            continue;
                        }
                        let accepted = sources.event(&event);
                        processing.note_source(sources.generation(), sources.phase());
                        subtitles.note_source(sources.generation(), sources.phase());
                        if accepted
                            && matches!(event, rsmpv::Event::FileLoaded)
                            && let Some((entry, ticket)) = sources.ready_load()
                            && let Err(error) = processing.replay_ready(&mpv, entry, ticket)
                        {
                            let _=events.send(Event::Broadcast("host.error".into(),json!({"code":error.code,"message":error.message,"generation":ticket.stamp.generation,"cycle":ticket.cycle})));
                        }
                        if accepted
                            && matches!(event, rsmpv::Event::FileLoaded)
                            && let Some((entry, ticket)) = sources.ready_load()
                            && let Err(error) =
                                subtitles.ready(&mpv, entry, ticket, sources.leased(entry))
                        {
                            subtitles.failed(error.message.clone());
                            let _ = events.send(Event::Broadcast(
                                "host.error".into(),
                                json!({"code":error.code,"message":error.message}),
                            ));
                        }
                        let _ = events.send(Event::Broadcast(
                            "playback.backend".into(),
                            json!({"currentSession":sources.generation(),"event":format!("{event:?}")}),
                        ));
                    }
                    if !geometry_error && let Err(error) = geometry.tick(&mpv, &sources) {
                        geometry_error = true;
                        let _ = events.send(Event::Broadcast("host.error".into(),json!({"code":"source-geometry","message":error.message,"generation":sources.generation()})));
                    }
                    // A fallback to software rendering (a window could not
                    // show GPU frames) ends the render options' effect.
                    let render_is_gpu = out_render.gpu().is_some();
                    if render_was_gpu && !render_is_gpu {
                        // Set here, on the worker that also owns the
                        // processing chain's copy-back switch, so the two
                        // never undo each other.
                        processing.renderer_changed();
                        let hwdec = software_hwdec();
                        if let Err(error) = mpv.set_property("hwdec", hwdec.as_str()) {
                            diag!("render: hwdec={hwdec} refused: {error}");
                        }
                    }
                    if render_was_gpu
                        && !render_is_gpu
                        && let Some(owner) = render_options.owner()
                    {
                        let status = crate::playback::video_render::status_failed(
                            "unsupported",
                            "the player switched to software rendering",
                        );
                        let _ = events.send(Event::PluginMessage(
                            owner,
                            "video.render.status".into(),
                            status,
                        ));
                    }
                    if render_was_gpu && !render_is_gpu {
                        let (pushes, written) = shaders.software_fallback(
                            &mut render_options,
                            &mut crate::playback::video_render::Mpv(&mpv),
                        );
                        for (owner, status) in pushes {
                            let _ = events.send(Event::PluginMessage(
                                owner,
                                "video.shaders.status".into(),
                                status,
                            ));
                        }
                        if let Err(error) = written {
                            diag!("video shaders: {}", error.message);
                        }
                    }
                    render_was_gpu = render_is_gpu;
                    #[cfg(feature = "producer-audio")]
                    {
                        // Before its frames: the GPU picture must not cover them.
                        out_render.set_producer(sources.provider_active());
                        if let Some(error) = sources.producer_failure() {
                            let _=events.send(Event::Broadcast("host.error".into(),json!({"code":"producer-failed","message":error,"generation":sources.generation()})));
                        }
                        *out_input
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) =
                            sources.input_routing();
                        sources.producer_output(
                            mpv.get_property::<f64>("volume").unwrap_or(100.),
                            mpv.get_property::<bool>("mute").unwrap_or(false),
                        );
                        if let Some(frame) = sources.producer_frame() {
                            let sequence = sequence.fetch_add(1, Ordering::Relaxed) + 1;
                            // Generated streams keep their own size.
                            out_frame.put(Frame {
                                buffer: SharedPixelBuffer::clone_from_slice(
                                    &frame.pixels,
                                    frame.width,
                                    frame.height,
                                ),
                                sequence,
                            });
                        }
                        // A stopped producer must not leave its last frame on the canvas.
                        let producer_active = sources.provider_active();
                        if producer_was_active && !producer_active {
                            let sequence = sequence.fetch_add(1, Ordering::Relaxed) + 1;
                            out_frame.put(Frame {
                                buffer: SharedPixelBuffer::new(CLEAR.0, CLEAR.1),
                                sequence,
                            });
                        }
                        producer_was_active = producer_active;
                    }
                    let now_types = sources.file_types();
                    if now_types != file_types {
                        file_types = now_types;
                        let _ = events.send(Event::Broadcast(
                            "resolvers.fileTypes".into(),
                            json!({"fileTypes":file_types}),
                        ));
                    }
                    // A phase change from the commands and events above goes
                    // out in this iteration; otherwise every 100 ms.
                    if push.due(sources.generation(), sources.phase(), Instant::now()) {
                        view_target.set_picture(picture_size(&mpv));
                        if frame_log {
                            out_frame.stages.mpv_counts(
                                mpv.get_property::<i64>("frame-drop-count").ok(),
                                mpv.get_property::<i64>("vo-delayed-frame-count").ok(),
                            );
                        }
                        let source_snapshot = sources.snapshot(&mpv);
                        // A watched hwdec switch's window also ends without an event.
                        if processing.hwdec_watching() {
                            processing.hwdec_check(&mpv);
                        }
                        processing.note_source(sources.generation(), sources.phase());
                        subtitles.note_source(sources.generation(), sources.phase());
                        // Natural EOF is found by the snapshot itself and is in it.
                        push.seen(sources.generation(), sources.phase());
                        push.published_paused(source_snapshot["paused"].as_bool());
                        let _ =
                            events.send(Event::Broadcast("source.state".into(), source_snapshot));
                    }
                    // While something ends without an event (`polling`), look
                    // again after `POLL`, as before 0.5.15, with a sleep: on
                    // Windows a condvar's timeout (`Wake::wait`) is rounded up
                    // to the system timer tick (measured about 15.6 ms for
                    // 2 ms), while `thread::sleep` uses a high-resolution
                    // timer (about 2.4 ms), and a generated stream's frames
                    // are taken here. Otherwise (0.5.15) wait for a command,
                    // mpv's events, the stop or the render thread's switch,
                    // at most until the next state push or audio device check
                    // (before: a 2 ms sleep every turn, about 500 wake-ups a
                    // second). A wake-up left from a polling turn costs only
                    // one extra turn.
                    let polling = taken == COMMANDS_PER_TURN
                        || actions.pending()
                        || (!geometry_error && geometry.polling())
                        || sources.polling();
                    if polling {
                        thread::sleep(POLL);
                    } else {
                        let now = Instant::now();
                        // Upscaling follows the video area on a timer while it
                        // is on (a resize does not wake this thread).
                        let mut timeout = push.next(now).min(sources.device_check_in(now));
                        if let Some(due) = upscaling.due_in(now) {
                            timeout = timeout.min(due);
                        }
                        worker_wake.wait(timeout);
                    }
                }
                drop(jobs);
                render_stop.store(true, Ordering::Relaxed);
                let _ = stop_wake.try_send(());
                let _ = renderer.join();
                // Native fd:// streams must die before Sources releases their Files.
                drop(mpv);
                Ok(())
            };
            if let Err(error) = run() {
                let message = error.to_string();
                let _ = ready.send(Err(message.clone()));
                let _ = events.send(Event::Broadcast(
                    "host.error".into(),
                    json!({"message":message}),
                ));
            }
        });
        let versions = initialized
            .recv()
            .map_err(|_| anyhow::anyhow!("backend initialization channel closed"))?
            .map_err(anyhow::Error::msg)?;
        Ok(Self {
            commands,
            frame,
            target,
            versions,
            render,
            #[cfg(feature = "producer-audio")]
            input,
            stop,
            wake,
            worker: Some(worker),
        })
    }
}
/// The GPU part of the render thread: the renderer owns mpv's render
/// context until the backend stops (so a rebuilt window never interrupts
/// playback) or a window asks for software rendering.
struct GpuRun<'a> {
    mpv: &'a Arc<Mpv>,
    render: &'a crate::playback::renderer::Render,
    publish: &'a Arc<AtomicBool>,
    stop: &'a AtomicBool,
}
type Ready<'a> = Option<&'a mpsc::SyncSender<std::result::Result<(), String>>>;
impl GpuRun<'_> {
    /// `true` when the backend stopped; `false`: render in software now.
    fn run(&self, area: crate::playback::render_target::Size, ready: &mut Ready) -> bool {
        let renderer =
            match crate::platform::gpu_start(self.mpv.clone(), area, self.publish.clone()) {
                Ok(renderer) => renderer,
                Err(error) => {
                    self.render
                        .start_failed(crate::playback::render_mode::Reason::InitFailed(format!(
                            "GPU rendering unavailable: {error}"
                        )));
                    return false;
                }
            };
        // The renderer's own diagnostics (Windows: its device, failures).
        crate::platform::gpu_started(&renderer.handle());
        let hwdec = crate::playback::renderer::hwdec(std::env::var("VESTRA_HWDEC").ok(), false);
        if let Err(error) = self.mpv.set_property("hwdec", hwdec.as_str()) {
            diag!("render: hwdec={hwdec} refused: {error}");
        }
        self.render.install(renderer.handle());
        if let Some(ready) = ready.take() {
            let _ = ready.send(Ok(()));
        }
        while !self.stop.load(Ordering::Relaxed) && !self.render.fallback_requested() {
            // The renderer stopped on its own (Windows: its device was
            // removed): software rendering, as for a window's fallback.
            if let Some(reason) = crate::platform::gpu_failed(&renderer.handle()) {
                self.render
                    .fall_back(crate::playback::render_mode::Reason::WindowFallback(reason));
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        if self.stop.load(Ordering::Relaxed) {
            return true;
        }
        self.render.deactivate();
        // Stops the GPU renderer (macOS, Windows); a no-op type elsewhere.
        #[allow(clippy::drop_non_drop)]
        drop(renderer);
        // The control worker sets software rendering's `hwdec` when it sees
        // the switch (see `software_hwdec`).
        false
    }
}
/// `hwdec` while the software renderer draws (`renderer::hwdec`).
fn software_hwdec() -> String {
    crate::playback::renderer::hwdec(std::env::var("VESTRA_HWDEC").ok(), true)
}
/// Interpolation (`video.render()`) paces the GPU renderer to the display.
fn pace(
    render: &crate::playback::renderer::Render,
    settings: &crate::playback::video_render::Settings,
) {
    if let Some(gpu) = render.gpu() {
        gpu.set_display_fps(settings.interpolation.then(|| render.refresh()));
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        self.wake.notify();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
/// R-U3: while a backdrop is mounted, mpv does not pick cover art (embedded
/// pictures or a cover.jpg next to the file) as a video track, so audio files
/// leave the video area to the backdrop. The defaults come back when it goes.
/// mpv picks tracks at load, so a change shows from the next file opened.
#[derive(Default)]
struct CoverArt {
    off: bool,
}
impl CoverArt {
    const DEFAULTS: [(&'static str, &'static str); 2] = [
        ("audio-display", "embedded-first"),
        ("cover-art-auto", "exact"),
    ];
    fn set(
        &mut self,
        off: bool,
        write: &mut dyn FnMut(&str, &str) -> Result<(), String>,
    ) -> Result<(), String> {
        if off == self.off {
            return Ok(());
        }
        for (name, default) in Self::DEFAULTS {
            write(name, if off { "no" } else { default })?;
        }
        self.off = off;
        Ok(())
    }
}
/// The picture's size in the last rendered frame: the frame less the
/// margins mpv left around the video (`osd-dimensions`; both renderers use
/// `vo=libmpv`, so it describes their last frame). Every display setting
/// (aspect override, rotation, zoom, panscan) is already in it.
fn picture_size(mpv: &Mpv) -> Option<(f64, f64)> {
    let read = |name: &str| {
        mpv.get_property::<f64>(&format!("osd-dimensions/{name}"))
            .ok()
            .filter(|v| v.is_finite())
    };
    let (width, height) = (read("w")?, read("h")?);
    let width = width - read("ml").unwrap_or(0.) - read("mr").unwrap_or(0.);
    let height = height - read("mt").unwrap_or(0.) - read("mb").unwrap_or(0.);
    (width >= 1. && height >= 1.).then_some((width, height))
}
/// The source's display size after the filter chain (`video-out-params/dw`, `dh`).
fn source_size(mpv: &Mpv) -> Option<(u32, u32)> {
    let read = |name: &str| {
        mpv.get_property::<i64>(name)
            .ok()
            .and_then(|v| u32::try_from(v).ok())
            .filter(|v| *v > 0)
    };
    Some((read("video-out-params/dw")?, read("video-out-params/dh")?))
}
/// `stats().video.decoding` from mpv `hwdec-current`: `"no"` is software
/// decoding, any other method hardware; `None` before a decoder runs.
fn decoding(hwdec: Option<&str>) -> Option<&'static str> {
    hwdec.map(|method| {
        if method == "no" {
            "software"
        } else {
            "hardware"
        }
    })
}
/// `playback.stats` (C10): what the playback information overlay shows.
fn stats(
    mpv: &Mpv,
    session: u64,
    frames: u64,
    render: &crate::playback::renderer::Render,
) -> Value {
    let text = |name: &str| {
        mpv.get_property::<String>(name)
            .ok()
            .filter(|s| !s.is_empty())
    };
    let int = |name: &str| mpv.get_property::<i64>(name).ok();
    let float = |name: &str| {
        mpv.get_property::<f64>(name)
            .ok()
            .filter(|v| v.is_finite() && *v > 0.)
    };
    // Signed or zero values (A/V sync, cache) are passed through as read.
    let signed = |name: &str| mpv.get_property::<f64>(name).ok().filter(|v| v.is_finite());
    let video = text("video-codec").map(|codec| {
        json!({"codec":codec,"width":int("video-params/w"),"height":int("video-params/h"),
            "fps":float("estimated-vf-fps").or_else(|| float("container-fps")),
            "decoder":text("current-tracks/video/decoder"),
            "pixelFormat":text("video-params/pixelformat"),
            "colorspace":text("video-params/colormatrix"),
            "displayWidth":int("video-out-params/dw"),"displayHeight":int("video-out-params/dh"),
            "displayFps":float("estimated-display-fps"),
            "bitrate":float("video-bitrate"),
            "hwdec":text("hwdec-current"),"decoding":decoding(text("hwdec-current").as_deref()),
            "transfer":text("video-params/gamma")})
    });
    let audio = text("audio-codec-name").map(|codec| {
        json!({"codec":codec,"sampleRate":int("audio-params/samplerate"),
            "channels":int("audio-params/channel-count"),
            "decoder":text("current-tracks/audio/decoder"),
            "format":text("audio-params/format"),
            "channelLayout":text("audio-params/hr-channels"),
            "bitrate":float("audio-bitrate")})
    });
    json!({"backend":render.backend_name(),"session":session,"framesShown":frames,
        "presentedFps":render.presentation.presented_fps(),
        "framesDropped":int("frame-drop-count"),"video":video,"audio":audio,
        "avSyncMs":signed("avsync").map(|seconds| seconds * 1000.),
        "cacheSeconds":signed("demuxer-cache-duration")})
}

/// What a source request touches on the control worker.
struct SourceContext<'a> {
    mpv: &'a Mpv,
    sources: &'a mut crate::playback::source_backend::Sources,
    geometry: &'a mut crate::playback::source_geometry::Geometry,
    geometry_error: &'a mut bool,
    publish_gate: &'a AtomicBool,
}
impl SourceContext<'_> {
    /// One source/playback request; a new load closes the publish gate until
    /// its orientation is prepared.
    fn call(&mut self, owner: u64, method: &str, params: &Value) -> Result<Value, RpcError> {
        let starts_source = method == "source.submit"
            && matches!(
                params["intent"]["kind"].as_str(),
                Some("open" | "openProvider" | "restart")
            );
        let previous_publish =
            starts_source.then(|| self.publish_gate.swap(false, Ordering::AcqRel));
        let result = match self.sources.dispatch(self.mpv, owner, method, params) {
            Ok(Some(value)) => Ok(value),
            Ok(None) => execute(self.mpv, method, params),
            Err(error) => Err(error),
        };
        if let Some(previous) = previous_publish {
            #[cfg(feature = "producer-audio")]
            let is_provider = self.sources.provider_active();
            #[cfg(not(feature = "producer-audio"))]
            let is_provider = false;
            if result.is_ok() && is_provider {
                self.publish_gate.store(false, Ordering::Release);
            } else if result.is_ok()
                && self.sources.phase() == crate::playback::session::Phase::Loading
            {
                self.geometry.begin();
                *self.geometry_error = false;
            } else {
                self.publish_gate.store(previous, Ordering::Release);
            }
        }
        result
    }
}

/// The core's own playback actions (`core_actions.rs`) and `player.step`,
/// executed here in order with every other source request.
#[derive(Default)]
struct CoreActions {
    /// The host's own operation ids in the source operation book.
    operation: u64,
    /// The newest relative-seek target: (source generation, target, when).
    seek: Option<(u64, f64, Instant)>,
    /// The one step waiting for its new position.
    stepping: Option<Stepping>,
    /// The keyframe step seek in flight (`setStepSeek({ keyframes })`).
    key_seek: Option<KeyFlight>,
    /// Completions decided outside `poll` (a keyframe seek ended early),
    /// sent by the next `poll`.
    finished: Vec<(Completion, Result<Value, RpcError>)>,
}
/// A keyframe step seek waiting for mpv to land, and every press it carries.
struct KeyFlight {
    generation: u64,
    state: crate::core_actions::KeySeek,
    done: Vec<Completion>,
    /// Where the issued seek started, and when it was sent.
    before: f64,
    started: Instant,
}
impl CoreActions {
    /// A keyframe step seek: one mpv seek at a time from the landed
    /// position; presses meanwhile are owed (`KeySeek`). Answered by `poll`.
    fn key_seek(
        &mut self,
        context: &mut SourceContext,
        facts: &crate::core_actions::Facts,
        forward: bool,
        step: f64,
        done: &mut Option<Completion>,
    ) -> Result<Option<Value>, RpcError> {
        let generation = context.sources.generation();
        if let Some(flight) = self
            .key_seek
            .as_mut()
            .filter(|f| f.generation == generation)
        {
            flight.state.press(forward, step);
            flight.done.extend(done.take());
            return Ok(None);
        }
        self.end_key_seek();
        let position = context
            .mpv
            .get_property::<f64>("time-pos")
            .ok()
            .or(facts.position)
            .unwrap_or(0.0);
        let (state, amount) =
            crate::core_actions::KeySeek::start(position, facts.duration, forward, step);
        let Some(amount) = amount else {
            let text =
                crate::core_actions::key_seek_notice(forward, step, position, facts.duration);
            return Ok(Some(json!({ "osd": text })));
        };
        context.sources.seek_elsewhere();
        key_command(context.mpv, &crate::core_actions::key_seek_command(amount))?;
        self.key_seek = Some(KeyFlight {
            generation,
            state,
            done: done.take().into_iter().collect(),
            before: position,
            started: Instant::now(),
        });
        Ok(None)
    }
    /// A source change or another seek ends the keyframe seek in flight:
    /// its presses resolve with no text.
    fn end_key_seek(&mut self) {
        if let Some(flight) = self.key_seek.take() {
            self.finished.extend(
                flight
                    .done
                    .into_iter()
                    .map(|done| (done, Ok(json!({ "osd": null })))),
            );
        }
    }
    /// A plugin's source request: a seek or a new source ends a keyframe
    /// step seek in flight (pause, rate and the like do not).
    fn source_request(&mut self, method: &str, params: &Value) {
        let intent = &params["intent"];
        if method == "source.submit"
            && (intent["kind"] != "control" || intent["control"]["kind"] == "seek")
        {
            self.end_key_seek();
        }
    }
    /// Settle the keyframe seek in flight: the next owed seek, the one exact
    /// fallback, or the presses' completions (one text, on the last; an
    /// mpv error rejects only the last).
    fn poll_key_seek(&mut self, mpv: &Mpv, generation: u64) {
        use crate::core_actions::KeyNext;
        let Some(flight) = self.key_seek.as_mut() else {
            return;
        };
        if flight.generation != generation {
            self.end_key_seek();
            return;
        }
        let position = mpv.get_property::<f64>("time-pos").ok();
        let seeking = mpv.get_property::<bool>("seeking").unwrap_or(false);
        let moved = position.is_some_and(|now| (now - flight.before).abs() > 0.0001);
        let at_end = mpv.get_property::<bool>("eof-reached").unwrap_or(false);
        if !(moved && !seeking) && !at_end && flight.started.elapsed() < Duration::from_millis(1500)
        {
            return;
        }
        let landed = position.unwrap_or(flight.before);
        let duration = mpv.get_property::<f64>("duration").ok();
        let next = flight.state.settled(landed, duration, at_end);
        let sent = match next {
            KeyNext::Seek(amount) => {
                key_command(mpv, &crate::core_actions::key_seek_command(amount)).map(|_| true)
            }
            KeyNext::Exact(target) => key_command(
                mpv,
                &[
                    "seek".into(),
                    format!("{target:.6}"),
                    "absolute+exact".into(),
                ],
            )
            .map(|_| true),
            KeyNext::Done => Ok(false),
        };
        match sent {
            Ok(true) => {
                flight.before = landed;
                flight.started = Instant::now();
            }
            outcome => {
                let flight = self.key_seek.take().expect("a flight");
                let last = flight.done.len().saturating_sub(1);
                let text = crate::core_actions::key_seek_notice(
                    flight.state.forward,
                    flight.state.step,
                    landed,
                    duration,
                );
                let mut outcome = Some(outcome.map(|_| json!({ "osd": text })));
                for (index, done) in flight.done.into_iter().enumerate() {
                    let result = if index == last {
                        outcome.take().expect("one outcome")
                    } else {
                        Ok(json!({ "osd": null }))
                    };
                    self.finished.push((done, result));
                }
            }
        }
    }
    fn submit(&mut self, context: &mut SourceContext, intent: Value) -> Result<(), RpcError> {
        self.operation += 1;
        context
            .call(
                crate::bindings::HOST,
                "source.submit",
                &json!({"operationId":self.operation,"intent":intent}),
            )
            .map(|_| ())
    }
    /// `host.action { action, step }` → `{ osd }`; refused with `unavailable`.
    fn run(
        &mut self,
        context: &mut SourceContext,
        params: &Value,
        done: Completion,
    ) -> Option<(Completion, Result<Value, RpcError>)> {
        let mut done = Some(done);
        let result = self.run_now(context, params, &mut done);
        finished(done, result)
    }
    fn run_now(
        &mut self,
        context: &mut SourceContext,
        params: &Value,
        done: &mut Option<Completion>,
    ) -> Result<Option<Value>, RpcError> {
        use crate::core_actions::{Facts, Plan, plan};
        let action = params["action"].as_str().unwrap_or("");
        let step = params["step"]
            .as_f64()
            .unwrap_or(crate::core_actions::DEFAULT_STEP);
        let state = context.sources.snapshot(context.mpv);
        let mut facts = Facts::from_state(&state);
        facts.next = params["sequence"]["next"].as_bool().unwrap_or(false);
        facts.previous = params["sequence"]["previous"].as_bool().unwrap_or(false);
        let generation = context.sources.generation();
        // Presses while the previous relative seek is still going add up.
        let pending = self.seek.filter(|(seen, _, at)| {
            *seen == generation
                && (context.mpv.get_property::<bool>("seeking").unwrap_or(false)
                    || at.elapsed() < Duration::from_millis(300))
        });
        let (plan, osd) = plan(action, &facts, step, pending.map(|(_, target, _)| target))
            .map_err(|reason| RpcError::new("unavailable", reason))?;
        let keyframes = params["keyframes"].as_bool().unwrap_or(false);
        if crate::core_actions::keyframe_seek(action, &facts, keyframes) {
            return self.key_seek(context, &facts, action == "seek-forward", step, done);
        }
        match plan {
            Plan::Restart => self.submit(context, json!({"kind":"restart"}))?,
            Plan::Pause(paused) => self.submit(
                context,
                json!({"kind":"control","control":{"kind":"pause","paused":paused}}),
            )?,
            Plan::Seek(seconds) => {
                self.end_key_seek();
                self.submit(
                    context,
                    json!({"kind":"control","control":{"kind":"seek","seconds":seconds}}),
                )?;
                self.seek = Some((generation, seconds, Instant::now()));
            }
            Plan::Rate(rate) => self.submit(
                context,
                json!({"kind":"control","control":{"kind":"rate","rate":rate}}),
            )?,
            Plan::Close => self.submit(context, json!({"kind":"close"}))?,
            Plan::Output { volume, muted } => {
                if let Some(volume) = volume {
                    execute(
                        context.mpv,
                        "playback.set",
                        &json!({"property":"volume","value":volume}),
                    )?;
                }
                execute(
                    context.mpv,
                    "playback.set",
                    &json!({"property":"muted","value":muted}),
                )?;
            }
            Plan::Frame { forward } => {
                let params =
                    json!({"unit":"frame","direction":if forward {"forward"} else {"backward"}});
                return self.step_now(context, &params, done);
            }
            Plan::Sequence { .. } => {
                return Err(RpcError::new(
                    "unsupported",
                    "the host routes next/previous",
                ));
            }
        }
        Ok(Some(json!({ "osd": osd })))
    }
    /// `player.step { unit, direction, expected? }`: answered once the new
    /// position is in place (see `poll`), so the caller's next state read has it.
    fn step(
        &mut self,
        context: &mut SourceContext,
        params: &Value,
        done: Completion,
    ) -> Option<(Completion, Result<Value, RpcError>)> {
        let mut done = Some(done);
        let result = self.step_now(context, params, &mut done);
        finished(done, result)
    }
    /// Takes `done` only when the step waits for its position.
    fn step_now(
        &mut self,
        context: &mut SourceContext,
        params: &Value,
        done: &mut Option<Completion>,
    ) -> Result<Option<Value>, RpcError> {
        use crate::core_actions::{Facts, StepPlan, first_frame, step_plan};
        if self.stepping.is_some() {
            return Err(RpcError::new("busy", "the previous step is still running"));
        }
        let state = context.sources.snapshot(context.mpv);
        if !params["expected"].is_null() && params["expected"] != state["stamp"] {
            return Err(RpcError::new("stale", "source changed"));
        }
        let facts = Facts::from_state(&state);
        let unit = params["unit"].as_str().unwrap_or("");
        let forward = params["direction"] == "forward";
        let plan = step_plan(&facts, unit, params["direction"].as_str().unwrap_or(""))
            .map_err(|(code, reason)| RpcError::new(code, reason))?;
        let before = context.mpv.get_property::<f64>("time-pos").ok();
        match plan {
            StepPlan::AtStart => {
                return Ok(Some(json!({ "osd": first_frame() })));
            }
            StepPlan::Command(command) => {
                context.sources.seek_elsewhere();
                context
                    .mpv
                    .command(&command)
                    .map_err(|e| RpcError::new("backend", e.to_string()))?
            }
            StepPlan::FromEnd => {
                let fps = ["estimated-vf-fps", "container-fps"]
                    .iter()
                    .find_map(|name| {
                        context
                            .mpv
                            .get_property::<f64>(name)
                            .ok()
                            .filter(|fps| fps.is_finite() && *fps > 0.)
                    })
                    .unwrap_or(30.);
                let target = (before.or(facts.position).unwrap_or(0.) - 1. / fps).max(0.);
                self.submit(
                    context,
                    json!({"kind":"control","control":{"kind":"seek","seconds":target}}),
                )?;
            }
        }
        self.stepping = Some(Stepping {
            done: done.take().expect("a completion for each step"),
            forward,
            notice: unit == "frame",
            before,
            started: Instant::now(),
        });
        Ok(None)
    }
    /// Whether `poll` waits for something mpv announces with no event (a
    /// step or keyframe seek landing, read from properties), or has an
    /// outcome to send.
    fn pending(&self) -> bool {
        self.stepping.is_some() || self.key_seek.is_some() || !self.finished.is_empty()
    }
    /// Finish a step once the position moved and no seek is running, mpv hit
    /// the end, or 1.5 s passed (nothing moved: a boundary).
    /// What finished since the last call: a step, keyframe step seeks.
    fn poll(&mut self, mpv: &Mpv, generation: u64) -> Vec<(Completion, Result<Value, RpcError>)> {
        self.poll_key_seek(mpv, generation);
        let mut done = std::mem::take(&mut self.finished);
        done.extend(self.poll_step(mpv));
        done
    }
    fn poll_step(&mut self, mpv: &Mpv) -> Option<(Completion, Result<Value, RpcError>)> {
        let stepping = self.stepping.as_ref()?;
        let position = mpv.get_property::<f64>("time-pos").ok();
        let seeking = mpv.get_property::<bool>("seeking").unwrap_or(false);
        let moved = match (stepping.before, position) {
            (Some(before), Some(now)) => (now - before).abs() > 0.0001,
            _ => false,
        };
        let at_end = stepping.forward && mpv.get_property::<bool>("eof-reached").unwrap_or(false);
        if !(moved && !seeking)
            && !at_end
            && stepping.started.elapsed() < Duration::from_millis(1500)
        {
            return None;
        }
        let stepping = self.stepping.take()?;
        let osd = stepping
            .notice
            .then(|| crate::core_actions::frame_notice(stepping.forward, moved, position));
        Some((stepping.done, Ok(json!({ "osd": osd }))))
    }
}

/// One mpv command of a keyframe step seek; a failure is `backend`.
fn key_command(mpv: &Mpv, command: &[String]) -> Result<(), RpcError> {
    mpv.command(command)
        .map_err(|e| RpcError::new("backend", e.to_string()))
}

/// An immediate outcome goes back with its completion; `None` is deferred.
fn finished(
    done: Option<Completion>,
    result: Result<Option<Value>, RpcError>,
) -> Option<(Completion, Result<Value, RpcError>)> {
    match result {
        Ok(None) => None,
        Ok(Some(value)) => done.map(|done| (done, Ok(value))),
        Err(error) => done.map(|done| (done, Err(error))),
    }
}

/// Who waits for a deferred step.
pub enum Completion {
    /// A core action (`ActionDone` shows its notice).
    Action(Reply, String),
    /// A plugin's `player.step`.
    Step(Reply),
}
impl Completion {
    pub fn send(self, events: &mpsc::Sender<Event>, result: Result<Value, RpcError>) {
        let _ = events.send(match self {
            Self::Action(reply, action) => Event::ActionDone(reply, action, result),
            Self::Step(reply) => Event::Reply(reply, result.map(|_| Value::Null)),
        });
    }
}
struct Stepping {
    done: Completion,
    forward: bool,
    /// Frame steps show where they landed; keyframe steps are plugin calls.
    notice: bool,
    before: Option<f64>,
    started: Instant,
}

/// `Event::PropertyChange` userdata of the observed `pause` property
/// (`StatePush::paused`); `processing::HWDEC_OBSERVATION` is 1.
pub const PAUSE_OBSERVATION: u64 = 2;

/// A `source.submit` that pauses or resumes the source.
fn pause_control(method: &str, params: &Value) -> bool {
    let intent = &params["intent"];
    method == "source.submit" && intent["kind"] == "control" && intent["control"]["kind"] == "pause"
}

/// When to publish `source.state`: every 100 ms, and at once when the source
/// phase (or source) changes, so `ready → ended` does not wait for the timer,
/// or when its pause state changes, so a play/pause icon follows a click
/// without the period's delay.
pub struct StatePush {
    last: Option<(u64, crate::playback::session::Phase)>,
    sent: Option<Instant>,
    /// The `paused` of the last published state.
    published_paused: Option<bool>,
}
impl StatePush {
    pub const PERIOD: Duration = Duration::from_millis(100);
    pub fn new() -> Self {
        Self {
            last: None,
            sent: None,
            published_paused: None,
        }
    }
    /// mpv's `pause` is now `paused`: when the last published state says
    /// otherwise, the next check publishes at once.
    pub fn paused(&mut self, paused: bool) {
        if self.published_paused != Some(paused) {
            self.soon();
        }
    }
    /// The `paused` the state just published carries.
    pub fn published_paused(&mut self, paused: Option<bool>) {
        self.published_paused = paused;
    }
    /// True when a broadcast is due now; records it as sent.
    pub fn due(
        &mut self,
        generation: u64,
        phase: crate::playback::session::Phase,
        now: Instant,
    ) -> bool {
        let changed = self.last != Some((generation, phase));
        let periodic = self
            .sent
            .is_none_or(|sent| now.duration_since(sent) >= Self::PERIOD);
        if changed || periodic {
            self.last = Some((generation, phase));
            self.sent = Some(now);
        }
        changed || periodic
    }
    /// A core action changed the source (paused, rate, volume): the next
    /// check publishes at once, so the controls follow without the period.
    pub fn soon(&mut self) {
        self.sent = None;
    }
    /// How long until the periodic broadcast is due (zero after `soon`).
    pub fn next(&self, now: Instant) -> Duration {
        self.sent.map_or(Duration::ZERO, |sent| {
            (sent + Self::PERIOD).saturating_duration_since(now)
        })
    }
    /// The phase the published state carries.
    pub fn seen(&mut self, generation: u64, phase: crate::playback::session::Phase) {
        self.last = Some((generation, phase));
    }
}

fn number(params: &Value, key: &str) -> std::result::Result<f64, RpcError> {
    params[key]
        .as_f64()
        .filter(|n| n.is_finite())
        .ok_or_else(|| RpcError::invalid(format!("{key} must be a finite number")))
}
/// `playback.set` covers only the shared device output (volume, mute); source
/// intent (pause, seek, rate) goes through `source.submit`.
fn execute(mpv: &Mpv, method: &str, params: &Value) -> std::result::Result<Value, RpcError> {
    if method != "playback.set" {
        return Err(RpcError::new("unsupported", "unknown playback method"));
    }
    let result = match params["property"].as_str().unwrap_or("") {
        "muted" => mpv.set_property(
            "mute",
            params["value"]
                .as_bool()
                .ok_or_else(|| RpcError::invalid("muted requires boolean"))?,
        ),
        "volume" => {
            let value = number(params, "value")?;
            if !(0.0..=200.0).contains(&value) {
                return Err(RpcError::invalid("volume must be 0..200"));
            }
            mpv.set_property("volume", value)
        }
        _ => {
            return Err(RpcError::new(
                "unsupported",
                "only volume and muted are settable",
            ));
        }
    };
    result
        .map(|_| Value::Null)
        .map_err(|e| RpcError::new("backend", e.to_string()))
}

/// mpv's own Lua scripts (S7 decision 7): the on-screen controller, ytdl
/// (network sources are resolved by plugins), stats, console, auto profiles,
/// select, positioning, commands, the context menu, and scripts from mpv's
/// configuration directory. The host draws its own interface and binds its
/// own keys, so none of them is used; off, libmpv also starts no LuaJIT (no
/// JIT under the macOS Hardened Runtime). Options an older libmpv does not
/// have are skipped: that libmpv does not have the script either.
pub const BUILTIN_SCRIPTS_OFF: [&str; 10] = [
    "osc",
    "ytdl",
    "load-scripts",
    "load-stats-overlay",
    "load-console",
    "load-auto-profiles",
    "load-select",
    "load-positioning",
    "load-commands",
    "load-context-menu",
];

fn no_builtin_scripts(builder: &rsmpv::Builder) -> Result<()> {
    for option in BUILTIN_SCRIPTS_OFF {
        match rsmpv::Handle::set_property(builder, option, "no") {
            Ok(()) | Err(rsmpv::Error::OptionNotFound) => {}
            Err(error) => return Err(anyhow::anyhow!("mpv option {option}: {error}")),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playback::session::Phase;
    #[test]
    fn decoding_is_software_for_no_and_hardware_for_a_method() {
        assert_eq!(decoding(Some("no")), Some("software"));
        assert_eq!(decoding(Some("d3d11va-copy")), Some("hardware"));
        assert_eq!(decoding(Some("videotoolbox")), Some("hardware"));
        assert_eq!(decoding(None), None);
    }
    #[test]
    fn mpv_starts_without_its_builtin_scripts() {
        let builder = Mpv::builder().unwrap();
        no_builtin_scripts(&builder).unwrap();
        let mpv = builder.build().unwrap();
        for option in BUILTIN_SCRIPTS_OFF {
            // Options this libmpv does not have were skipped.
            if let Ok(value) = mpv.get_property::<String>(option) {
                assert_eq!(value, "no", "{option}");
            }
        }
        for option in ["osc", "ytdl", "load-scripts", "load-stats-overlay"] {
            assert_eq!(
                mpv.get_property::<String>(option).unwrap(),
                "no",
                "{option}"
            );
        }
        // No script client answers: stats.lua would take this binding.
        std::thread::sleep(Duration::from_millis(200));
        assert!(
            mpv.command(&["script-binding", "stats/display-stats-toggle"])
                .is_err()
        );
    }
    #[test]
    fn a_backdrop_turns_cover_art_off_and_back_to_the_defaults() {
        let mut written = Vec::new();
        let mut write = |name: &str, value: &str| -> Result<(), String> {
            written.push(format!("{name}={value}"));
            Ok(())
        };
        let mut cover = CoverArt::default();
        cover.set(false, &mut write).unwrap();
        cover.set(true, &mut write).unwrap();
        cover.set(true, &mut write).unwrap();
        cover.set(false, &mut write).unwrap();
        assert_eq!(
            written,
            [
                "audio-display=no",
                "cover-art-auto=no",
                "audio-display=embedded-first",
                "cover-art-auto=exact"
            ]
        );
    }
    #[test]
    fn a_phase_change_is_pushed_without_waiting_for_the_period() {
        let start = Instant::now();
        let mut push = StatePush::new();
        assert!(push.due(1, Phase::Ready, start));
        let soon = start + Duration::from_millis(5);
        assert!(!push.due(1, Phase::Ready, soon));
        push.soon();
        assert!(
            push.due(1, Phase::Ready, soon),
            "after a core action at once"
        );
        assert!(!push.due(1, Phase::Ready, soon));
        assert!(
            push.due(1, Phase::Ended, soon),
            "ready → ended goes out at once"
        );
        assert!(!push.due(1, Phase::Ended, soon + Duration::from_millis(1)));
        assert!(
            push.due(2, Phase::Ended, soon + Duration::from_millis(2)),
            "new source"
        );
        assert!(
            push.due(2, Phase::Ended, soon + Duration::from_millis(102)),
            "periodic"
        );
    }
    /// 0.5.26: a pause or resume (mpv's observed `pause`, or a pause
    /// control) is published at once, not up to 100 ms later; mpv's
    /// repeats of the published value are not.
    #[test]
    fn a_pause_change_is_pushed_without_waiting_for_the_period() {
        let start = Instant::now();
        let mut push = StatePush::new();
        assert!(push.due(1, Phase::Ready, start));
        push.published_paused(Some(false));
        let soon = start + Duration::from_millis(5);
        push.paused(false);
        assert!(!push.due(1, Phase::Ready, soon), "unchanged: periodic only");
        push.paused(true);
        assert_eq!(push.next(soon), Duration::ZERO, "the worker wakes at once");
        assert!(push.due(1, Phase::Ready, soon), "paused goes out at once");
        push.published_paused(Some(true));
        push.paused(true);
        assert!(!push.due(1, Phase::Ready, soon + Duration::from_millis(1)));
        push.paused(false);
        assert!(
            push.due(1, Phase::Ready, soon + Duration::from_millis(2)),
            "resumed goes out at once"
        );
        let pause = json!({"intent":{"kind":"control","control":{"kind":"pause","paused":true}}});
        let seek = json!({"intent":{"kind":"control","control":{"kind":"seek","seconds":1}}});
        assert!(pause_control("source.submit", &pause));
        assert!(!pause_control("source.submit", &seek));
        assert!(!pause_control("source.query", &pause));
    }
    /// 0.5.15: the control worker sleeps until the next push instead of
    /// polling every 2 ms.
    #[test]
    fn the_worker_sleeps_until_the_next_push() {
        let start = Instant::now();
        let mut push = StatePush::new();
        assert_eq!(push.next(start), Duration::ZERO, "never sent: due now");
        assert!(push.due(1, Phase::Ready, start));
        assert_eq!(push.next(start), StatePush::PERIOD);
        let later = start + Duration::from_millis(30);
        assert_eq!(push.next(later), Duration::from_millis(70));
        assert_eq!(push.next(start + Duration::from_secs(1)), Duration::ZERO);
        push.soon();
        assert_eq!(push.next(later), Duration::ZERO, "after a core action");
    }
    #[test]
    fn a_wake_up_is_kept_until_the_wait_takes_it() {
        let wake = Arc::new(Wake::default());
        wake.notify();
        assert!(wake.wait(Duration::from_secs(5)), "woken before the wait");
        assert!(!wake.wait(Duration::from_millis(1)), "taken");
        let from = wake.clone();
        let notifier = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            from.notify();
        });
        let began = Instant::now();
        assert!(wake.wait(Duration::from_secs(5)));
        assert!(began.elapsed() < Duration::from_secs(5), "not the timeout");
        let _ = notifier.join();
    }
    #[test]
    fn commands_wake_the_worker() {
        let (sender, receiver) = mpsc::channel();
        let wake = Arc::new(Wake::default());
        let commands = Commands {
            sender,
            wake: wake.clone(),
        };
        assert!(!wake.wait(Duration::ZERO));
        let reply = Reply { owner: 1, id: 1 };
        assert!(
            commands
                .send(Command {
                    reply,
                    method: "playback.stats".into(),
                    params: Value::Null,
                })
                .is_ok()
        );
        assert!(wake.wait(Duration::ZERO));
        assert_eq!(receiver.try_iter().count(), 1);
    }
}
