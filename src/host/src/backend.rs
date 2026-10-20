//! One persistent libmpv core. Rendering and all control calls stay off the UI thread.
use crate::{Event, Reply, RpcError};
use anyhow::Result;
use rsmpv::{
    Mpv,
    render::{RenderContext, SwPixelFormat},
};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub sequence: u64,
}
pub const WIDTH: usize = 960;
pub const HEIGHT: usize = 540;
pub struct Command {
    pub reply: Reply,
    pub method: String,
    pub params: Value,
}
pub struct Backend {
    pub commands: mpsc::Sender<Command>,
    pub frame: Arc<Mutex<Option<Frame>>>,
    #[cfg(feature = "producer-audio")]
    pub input: Arc<Mutex<Option<crate::producer_input::Routing>>>,
    /// `{ mpv, ffmpeg }`: the linked libmpv's `mpv-version` / `ffmpeg-version`.
    pub versions: Value,
    stop: mpsc::Sender<()>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Backend {
    pub fn start(events: mpsc::Sender<Event>, silent: bool) -> Result<Self> {
        let (commands, receiver) = mpsc::channel::<Command>();
        let (stop, stopping) = mpsc::channel();
        let frame = Arc::new(Mutex::new(None));
        let out_frame = frame.clone();
        #[cfg(feature = "producer-audio")]
        let input = Arc::new(Mutex::new(None));
        #[cfg(feature = "producer-audio")]
        let out_input = input.clone();
        let (ready, initialized) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let run = || -> Result<()> {
                let fonts = crate::font_carrier::FontDirectory::bundled()?;
                let default_subtitle =
                    crate::subtitle_style::default_for(fonts.asset().metadata().font_id.into());
                let mut builder = Mpv::builder()?
                    .set_property("vo", "libmpv")?
                    .set_property("hwdec", "no")?
                    .set_property("keep-open", true)?
                    .set_property("sub-auto", "no")?
                    .set_property("vf", crate::source_geometry::INITIAL_FILTERS)?
                    .set_property("sub-fonts-dir", fonts.path().to_string_lossy().as_ref())?
                    .set_property("sub-font-provider", "none")?
                    .set_property("embeddedfonts", false)?;
                for (property, value) in crate::subtitle_style::mpv_properties(
                    &default_subtitle,
                    fonts.asset().metadata().family,
                )? {
                    builder = builder.set_property(property, value.as_str())?;
                }
                if silent {
                    builder = builder.set_property("ao", "null")?;
                }
                let mpv = Arc::new(builder.build()?);
                mpv.request_log_messages("v")?;
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
                let renderer = thread::spawn(move || {
                    let run = || -> Result<()> {
                        let mut render = RenderContext::new_software(render_core)?;
                        let (wake, woke) = mpsc::sync_channel(1);
                        // Callback only signals. It never calls into libmpv.
                        render.set_update_callback(move || {
                            let _ = wake.try_send(());
                        });
                        let _ = render_ready.send(Ok(()));
                        while !renderer_stop.load(Ordering::Relaxed) {
                            if render.update() {
                                let mut pixels = vec![0; WIDTH * HEIGHT * 3];
                                render.render_software(
                                    WIDTH as i32,
                                    HEIGHT as i32,
                                    SwPixelFormat::Rgb24,
                                    WIDTH * 3,
                                    &mut pixels,
                                )?;
                                // Consume safe normalized frames offscreen while source
                                // orientation is prepared; never publish an unrotated preview.
                                let mut output = render_frame.lock().unwrap();
                                if renderer_publish.load(Ordering::Acquire) {
                                    let sequence =
                                        render_sequence.fetch_add(1, Ordering::Relaxed) + 1;
                                    *output = Some(Frame {
                                        pixels,
                                        sequence,
                                        width: WIDTH as u32,
                                        height: HEIGHT as u32,
                                    });
                                }
                            }
                            let _ = woke.recv_timeout(Duration::from_millis(5));
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
                let mut sources = crate::source_backend::Sources::default();
                let mut geometry = crate::source_geometry::Geometry::new(publish_gate.clone());
                let mut geometry_error = false;
                #[cfg(feature = "producer-audio")]
                let mut producer_was_active = false;
                let mut processing = crate::processing::Processing::default();
                let mut subtitles = crate::subtitle_backend::Subtitles::new(fonts);
                let mut jobs = crate::media_jobs::Jobs::default();
                let mut push = StatePush::new();
                let mut actions = CoreActions::default();
                loop {
                    if stopping.try_recv().is_ok() {
                        break;
                    }
                    for (owner, event, data) in sources.resolver_events() {
                        let _ = events.send(Event::PluginMessage(owner, event, data));
                    }
                    jobs.poll();
                    processing.note_source(sources.generation(), sources.phase());
                    subtitles.note_source(sources.generation(), sources.phase());
                    // Bound work per iteration so continuous controls cannot starve video.
                    for command in receiver.try_iter().take(32) {
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
                        if command.method == "owner.disabled" {
                            jobs.disable(command.reply.owner);
                            sources.disable(command.reply.owner);
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
                            let _ = events.send(Event::BackendCleaned(command.reply.owner));
                            continue;
                        }
                        if command.method == "owner.registered" {
                            if let Some(id) = command.params["manifestId"].as_str() {
                                processing.register(command.reply.owner, id.into());
                                subtitles.register(command.reply.owner, id.into());
                                jobs.register(command.reply.owner);
                            }
                            continue;
                        }
                        if command.method.starts_with("snapshot.")
                            || command.method.starts_with("jobs.")
                        {
                            let result = jobs.dispatch(
                                crate::media_jobs::CaptureContext {
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
                            let ready = sources.snapshot(&mpv)["phase"] == "ready";
                            processing.note_source(sources.generation(), sources.phase());
                            subtitles.note_source(sources.generation(), sources.phase());
                            let result = processing.replace(
                                &mpv,
                                command.reply.owner,
                                sources.generation(),
                                ready,
                                &command.params,
                            );
                            let _ = events.send(Event::Reply(command.reply, result));
                            continue;
                        }
                        if command.method == "playback.stats" {
                            let stats =
                                stats(&mpv, sources.generation(), sequence.load(Ordering::Relaxed));
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
                            let done = Completion::Action(command.reply, action);
                            let result = actions.run(&mut context, &command.params, done);
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
                        let result =
                            context.call(command.reply.owner, &command.method, &command.params);
                        processing.note_source(sources.generation(), sources.phase());
                        subtitles.note_source(sources.generation(), sources.phase());
                        let _ = events.send(Event::Reply(command.reply, result));
                    }
                    sources.follow_device(&mpv);
                    if let Some((done, result)) = actions.poll(&mpv) {
                        done.send(&events, result);
                    }
                    while let Some(event) = mpv.poll_event() {
                        if let rsmpv::Event::LogMessage(message) = &event {
                            if message.prefix == "sub/ass"
                                && (message.text.contains("fontselect")
                                    || message.text.contains("Loading font"))
                            {
                                eprintln!("libmpv {}: {}", message.prefix, message.text.trim_end());
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
                            && let Err(error) = subtitles.ready(&mpv, entry, ticket)
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
                    #[cfg(feature = "producer-audio")]
                    {
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
                            *out_frame.lock().unwrap() = Some(Frame {
                                pixels: frame.pixels,
                                width: frame.width,
                                height: frame.height,
                                sequence,
                            });
                        }
                        // A stopped producer must not leave its last frame on the canvas.
                        let producer_active = sources.provider_active();
                        if producer_was_active && !producer_active {
                            let sequence = sequence.fetch_add(1, Ordering::Relaxed) + 1;
                            *out_frame.lock().unwrap() = Some(Frame {
                                pixels: vec![0; WIDTH * HEIGHT * 3],
                                width: WIDTH as u32,
                                height: HEIGHT as u32,
                                sequence,
                            });
                        }
                        producer_was_active = producer_active;
                    }
                    // A phase change from the commands and events above goes
                    // out in this iteration; otherwise every 100 ms.
                    if push.due(sources.generation(), sources.phase(), Instant::now()) {
                        let source_snapshot = sources.snapshot(&mpv);
                        processing.note_source(sources.generation(), sources.phase());
                        subtitles.note_source(sources.generation(), sources.phase());
                        // Natural EOF is found by the snapshot itself and is in it.
                        push.seen(sources.generation(), sources.phase());
                        let _ =
                            events.send(Event::Broadcast("source.state".into(), source_snapshot));
                    }
                    thread::sleep(Duration::from_millis(2));
                }
                drop(jobs);
                render_stop.store(true, Ordering::Relaxed);
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
            versions,
            #[cfg(feature = "producer-audio")]
            input,
            stop,
            worker: Some(worker),
        })
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
/// `playback.stats` (C10): what the playback information overlay shows.
fn stats(mpv: &Mpv, session: u64, frames: u64) -> Value {
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
            "bitrate":float("video-bitrate")})
    });
    let audio = text("audio-codec-name").map(|codec| {
        json!({"codec":codec,"sampleRate":int("audio-params/samplerate"),
            "channels":int("audio-params/channel-count"),
            "decoder":text("current-tracks/audio/decoder"),
            "format":text("audio-params/format"),
            "channelLayout":text("audio-params/hr-channels"),
            "bitrate":float("audio-bitrate")})
    });
    json!({"backend":"libmpv · 软件渲染","session":session,"framesShown":frames,
        "framesDropped":int("frame-drop-count"),"video":video,"audio":audio,
        "avSyncMs":signed("avsync").map(|seconds| seconds * 1000.),
        "cacheSeconds":signed("demuxer-cache-duration")})
}

/// What a source request touches on the control worker.
struct SourceContext<'a> {
    mpv: &'a Mpv,
    sources: &'a mut crate::source_backend::Sources,
    geometry: &'a mut crate::source_geometry::Geometry,
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
            } else if result.is_ok() && self.sources.phase() == crate::session::Phase::Loading {
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
}
impl CoreActions {
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
        match plan {
            Plan::Restart => self.submit(context, json!({"kind":"restart"}))?,
            Plan::Pause(paused) => self.submit(
                context,
                json!({"kind":"control","control":{"kind":"pause","paused":paused}}),
            )?,
            Plan::Seek(seconds) => {
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
        use crate::core_actions::{FIRST_FRAME, Facts, StepPlan, step_plan};
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
                return Ok(Some(json!({ "osd": FIRST_FRAME })));
            }
            StepPlan::Command(command) => context
                .mpv
                .command(&command)
                .map_err(|e| RpcError::new("backend", e.to_string()))?,
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
    /// Finish a step once the position moved and no seek is running, mpv hit
    /// the end, or 1.5 s passed (nothing moved: a boundary).
    fn poll(&mut self, mpv: &Mpv) -> Option<(Completion, Result<Value, RpcError>)> {
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

/// When to publish `source.state`: every 100 ms, and at once when the source
/// phase (or source) changes, so `ready → ended` does not wait for the timer.
pub struct StatePush {
    last: Option<(u64, crate::session::Phase)>,
    sent: Option<Instant>,
}
impl StatePush {
    pub const PERIOD: Duration = Duration::from_millis(100);
    pub fn new() -> Self {
        Self {
            last: None,
            sent: None,
        }
    }
    /// True when a broadcast is due now; records it as sent.
    pub fn due(&mut self, generation: u64, phase: crate::session::Phase, now: Instant) -> bool {
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
    /// The phase the published state carries.
    pub fn seen(&mut self, generation: u64, phase: crate::session::Phase) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Phase;
    #[test]
    fn a_phase_change_is_pushed_without_waiting_for_the_period() {
        let start = Instant::now();
        let mut push = StatePush::new();
        assert!(push.due(1, Phase::Ready, start));
        let soon = start + Duration::from_millis(5);
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
}
