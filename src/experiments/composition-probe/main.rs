#![forbid(unsafe_code)]
//! Native integration specimen; sample controls here are not product policy.
#[path = "../../host/src/backend.rs"]
mod backend;
#[path = "../../host/src/composition.rs"]
pub mod composition;
#[path = "../../host/src/effects.rs"]
mod effects;
#[path = "../../host/src/font_asset.rs"]
pub mod font_asset;
#[path = "../../host/src/font_carrier.rs"]
pub mod font_carrier;
#[path = "../../host/src/media_jobs.rs"]
mod media_jobs;
#[path = "../../host/src/media_tracks.rs"]
mod media_tracks;
#[path = "../../host/src/media_worker.rs"]
mod media_worker;
#[path = "../../host/src/native_subtitle.rs"]
mod native_subtitle;
#[path = "../../host/src/offline_audio.rs"]
mod offline_audio;
#[path = "../../host/src/offline_descriptor.rs"]
mod offline_descriptor;
#[path = "../../host/src/processing.rs"]
mod processing;
#[path = "../../host/src/processing_operations.rs"]
pub mod processing_operations;
#[cfg(feature = "producer-audio-probe")]
#[path = "../../host/src/producer.rs"]
mod producer;
#[cfg(feature = "producer-audio-probe")]
#[path = "../../host/src/producer_audio.rs"]
mod producer_audio;
#[cfg(feature = "producer-audio-probe")]
#[path = "../../host/src/producer_controls.rs"]
mod producer_controls;
#[cfg(feature = "producer-audio-probe")]
#[path = "../../host/src/producer_input.rs"]
pub mod producer_input;
#[cfg(feature = "producer-audio-probe")]
#[path = "../../host/src/producer_input_delivery.rs"]
pub mod producer_input_delivery;

#[cfg(feature = "producer-audio-probe")]
#[path = "../../host/src/producer_binary.rs"]
pub mod producer_binary;
#[path = "../../host/src/session.rs"]
mod session;
#[path = "../../host/src/source_backend.rs"]
mod source_backend;
#[path = "../../host/src/source_geometry.rs"]
mod source_geometry;
#[path = "../../host/src/source_lease.rs"]
pub mod source_lease;
#[path = "../../host/src/subtitle_backend.rs"]
mod subtitle_backend;
#[path = "../../host/src/subtitle_format.rs"]
pub mod subtitle_format;
#[path = "../../host/src/subtitle_state.rs"]
pub mod subtitle_state;
#[path = "../../host/src/subtitle_style.rs"]
pub mod subtitle_style;
#[path = "../../host/src/subtitle_upload.rs"]
pub mod subtitle_upload;
#[path = "../../host/src/ui_values.rs"]
pub mod ui_values;
use anyhow::Result;
use serde::Serialize;
use serde_json::{Value, json};
use slint::ComponentHandle;
use std::{
    path::Path,
    sync::mpsc,
    time::{Duration, Instant},
};
#[derive(Debug, Serialize)]
struct RpcError {
    code: String,
    message: String,
}
impl RpcError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
    fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid", message)
    }
}
#[derive(Clone)]
struct Reply {
    owner: u64,
    id: u64,
}
enum Event {
    Reply(Reply, std::result::Result<Value, RpcError>),
    Broadcast(String, Value),
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let media = args
        .get(1)
        .ok_or_else(|| anyhow::anyhow!("media path required"))?;
    let seconds = args
        .get(2)
        .map(|s| s.parse::<f64>())
        .transpose()?
        .unwrap_or(120.);
    let (events, input) = mpsc::channel();
    let backend = backend::Backend::start(events, true)?;
    #[cfg(feature = "producer-audio-probe")]
    let _native_input_routing = backend.input.clone();
    let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/composition-demo");
    let specs = vec![
        composition::CompileSpec {
            owner: 1,
            file: samples.join("bar/bar.slint"),
            component: "PlaybackBar".into(),
            global: Some("Bridge".into()),
            slot: composition::Slot::Bottom,
        },
        composition::CompileSpec {
            owner: 2,
            file: samples.join("sidebar/sidebar.slint"),
            component: "SourceSidebar".into(),
            global: Some("Bridge".into()),
            slot: composition::Slot::Sidebar,
        },
    ];
    let (callbacks, callback_input) = mpsc::channel();
    let window = composition::compile(&specs, callbacks)?;
    composition::set_global(
        &window,
        &specs[1],
        "count",
        composition::Primitive::Number(3.),
    )?;
    let commands = backend.commands.clone();
    commands.send(backend::Command {
        reply: Reply { owner: 1, id: 1 },
        method: "playback.open".into(),
        params: json!({"source":media}),
    })?;
    let weak = window.as_weak();
    let start = Instant::now();
    let timer = slint::Timer::default();
    let mut request = 1;
    let mut sidebar_visible = true;
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(16),
        move || {
            for event in input.try_iter().take(128) {
                match event {
                    Event::Reply(reply, result) => {
                        eprintln!("reply {}.{}: {:?}", reply.owner, reply.id, result)
                    }
                    Event::Broadcast(name, data) => {
                        if name == "host.error" {
                            eprintln!("{name}: {data}")
                        }
                    }
                }
            }
            if let Some(window) = weak.upgrade() {
                for event in callback_input.try_iter().take(64) {
                    eprintln!("native callback: {event:?}");
                    match (event.owner, event.callback.as_str(), event.args) {
                        (1, "set-paused", Ok(args)) => {
                            if let Some(composition::Primitive::Bool(paused)) = args.first() {
                                request += 1;
                                let _ = commands.send(backend::Command {
                                    reply: Reply {
                                        owner: 1,
                                        id: request,
                                    },
                                    method: "playback.set".into(),
                                    params: json!({"property":"pause","value":paused}),
                                });
                            }
                        }
                        (1, "toggle-sidebar", _) => {
                            sidebar_visible = !sidebar_visible;
                            let _ = window.set_property("sidebar-visible", sidebar_visible.into());
                        }
                        (2, "select-row", Ok(args)) => {
                            if let Some(composition::Primitive::String(row)) = args.first() {
                                let _ = composition::set_global(
                                    &window,
                                    &specs[1],
                                    "selected",
                                    composition::Primitive::String(format!("Selected: {row}")),
                                );
                            }
                        }
                        _ => (),
                    }
                }
                if let Some(frame) = backend.frame.lock().unwrap().take() {
                    let _rendered_sequence = frame.sequence;
                    let buffer = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::clone_from_slice(
                        &frame.pixels,
                        frame.width,
                        frame.height,
                    );
                    let _ =
                        window.set_property("video-frame", slint::Image::from_rgb8(buffer).into());
                }
                let state = backend.state.lock().unwrap();
                let paused = state["paused"].as_bool().unwrap_or(false);
                let status = format!(
                    "Session {} · {:.2}s · frame {}",
                    state["session"],
                    state["position"].as_f64().unwrap_or(0.),
                    state["videoFrames"]
                );
                let _ = composition::set_global(
                    &window,
                    &specs[0],
                    "paused",
                    composition::Primitive::Bool(paused),
                );
                let _ = composition::set_global(
                    &window,
                    &specs[0],
                    "status",
                    composition::Primitive::String(status.clone()),
                );
                let _ = window.set_property(
                    "diagnostic",
                    slint_interpreter::Value::String(status.as_str().into()),
                );
            }
            if start.elapsed().as_secs_f64() >= seconds {
                let _ = slint::quit_event_loop();
            }
        },
    );
    window.run()?;
    timer.stop();
    Ok(())
}
