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
    pub state: Arc<Mutex<Value>>,
    #[cfg(feature = "producer-audio-probe")]
    pub input: Arc<Mutex<Option<crate::producer_input::Routing>>>,
    stop: mpsc::Sender<()>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Backend {
    pub fn start(events: mpsc::Sender<Event>, silent: bool) -> Result<Self> {
        let (commands, receiver) = mpsc::channel::<Command>();
        let (stop, stopping) = mpsc::channel();
        let frame = Arc::new(Mutex::new(None));
        let state = Arc::new(Mutex::new(
            json!({"session":0,"source":null,"paused":false,"position":null,"duration":null,"videoFrames":0}),
        ));
        let out_frame = frame.clone();
        let out_state = state.clone();
        #[cfg(feature = "producer-audio-probe")]
        let input = Arc::new(Mutex::new(None));
        #[cfg(feature = "producer-audio-probe")]
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
                let _ = ready.send(Ok(()));
                let mut sources = crate::source_backend::Sources::default();
                let mut geometry = crate::source_geometry::Geometry::new(publish_gate.clone());
                let mut geometry_error = false;
                let mut processing = crate::processing::Processing::default();
                let mut subtitles = crate::subtitle_backend::Subtitles::new(fonts);
                let mut jobs = crate::media_jobs::Jobs::default();
                let mut last_state = Instant::now() - Duration::from_secs(1);
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
                        #[cfg(feature = "producer-audio-probe")]
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
                        if command.method == "playback.set"
                            && command.params["property"] == "audio-delay"
                            && processing.owns_audio()
                        {
                            let _ = events.send(Event::Reply(
                                command.reply,
                                Err(RpcError::new(
                                    "conflict",
                                    "audio-delay is controlled by owned audio resources",
                                )),
                            ));
                            continue;
                        }
                        let starts_source = command.method == "playback.open"
                            || (command.method == "source.submit"
                                && matches!(
                                    command.params["intent"]["kind"].as_str(),
                                    Some("open" | "openProvider" | "restart")
                                ));
                        let previous_publish =
                            starts_source.then(|| publish_gate.swap(false, Ordering::AcqRel));
                        let result = match sources.dispatch(
                            &mpv,
                            command.reply.owner,
                            &command.method,
                            &command.params,
                        ) {
                            Ok(Some(value)) => Ok(value),
                            Ok(None) => execute(&mpv, &command.method, &command.params),
                            Err(error) => Err(error),
                        };
                        if let Some(previous) = previous_publish {
                            #[cfg(feature = "producer-audio-probe")]
                            let is_provider = sources.provider_active();
                            #[cfg(not(feature = "producer-audio-probe"))]
                            let is_provider = false;
                            if result.is_ok() && is_provider {
                                publish_gate.store(false, Ordering::Release);
                            } else if result.is_ok()
                                && sources.phase() == crate::session::Phase::Loading
                            {
                                geometry.begin();
                                geometry_error = false;
                            } else {
                                publish_gate.store(previous, Ordering::Release);
                            }
                        }
                        processing.note_source(sources.generation(), sources.phase());
                        subtitles.note_source(sources.generation(), sources.phase());
                        let _ = events.send(Event::Reply(command.reply, result));
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
                    #[cfg(feature = "producer-audio-probe")]
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
                    }
                    if last_state.elapsed() >= Duration::from_millis(100) {
                        let source_snapshot = sources.snapshot(&mpv);
                        processing.note_source(sources.generation(), sources.phase());
                        subtitles.note_source(sources.generation(), sources.phase());
                        let _ = events.send(Event::Broadcast(
                            "source.state".into(),
                            source_snapshot.clone(),
                        ));
                        let snapshot = json!({"session": sources.generation(), "source":sources.source(),
                            "paused":source_snapshot["paused"],
                            "position":source_snapshot["position"],
                            "duration":source_snapshot["duration"],
                            "volume":mpv.get_property::<f64>("volume").unwrap_or(100.0),
                            "rate":mpv.get_property::<f64>("speed").unwrap_or(1.0),
                            "videoFrames":sequence.load(Ordering::Relaxed)});
                        *out_state.lock().unwrap() = snapshot.clone();
                        let _ = events.send(Event::Broadcast("playback.state".into(), snapshot));
                        last_state = Instant::now();
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
        initialized
            .recv()
            .map_err(|_| anyhow::anyhow!("backend initialization channel closed"))?
            .map_err(anyhow::Error::msg)?;
        Ok(Self {
            commands,
            frame,
            state,
            #[cfg(feature = "producer-audio-probe")]
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
fn number(params: &Value, key: &str) -> std::result::Result<f64, RpcError> {
    params[key]
        .as_f64()
        .filter(|n| n.is_finite())
        .ok_or_else(|| RpcError::invalid(format!("{key} must be a finite number")))
}
fn execute(mpv: &Mpv, method: &str, params: &Value) -> std::result::Result<Value, RpcError> {
    let result = match method {
        "playback.set" => {
            let property = params["property"].as_str().unwrap_or("");
            if matches!(property, "pause" | "muted") {
                mpv.set_property(
                    if property == "muted" { "mute" } else { "pause" },
                    params["value"]
                        .as_bool()
                        .ok_or_else(|| RpcError::invalid("pause/muted requires boolean"))?,
                )
                .map(|_| Value::Null)
            } else {
                let value = number(params, "value")?;
                let range = match property {
                    "volume" => (0.0, 200.0),
                    "speed" => (0.05, 10.0),
                    "brightness" | "contrast" | "saturation" | "gamma" => (-100.0, 100.0),
                    "audio-delay" | "sub-delay" => (-60.0, 60.0),
                    _ => {
                        return Err(RpcError::new(
                            "unsupported",
                            "property is not exposed by this host",
                        ));
                    }
                };
                if value < range.0 || value > range.1 {
                    return Err(RpcError::invalid("property value outside published range"));
                }
                mpv.set_property(property, value).map(|_| Value::Null)
            }
        }
        _ => return Err(RpcError::new("unsupported", "unknown playback method")),
    };
    result.map_err(|e| RpcError::new("backend", e.to_string()))
}
