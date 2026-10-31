//! Video upscaling (`video.upscaling.state`, `video.upscaling.set`;
//! capability `video.upscaling`, SDK 1.5.37; AI upscaling round record
//! doc/process/rounds/2026-10-ai-upscaling.md §8 V3), on the control worker.
//!
//! - **Availability.** The system's check (`platform::upscaler_probe`:
//!   on Windows an NVIDIA GPU, driver 530 or newer, and the driver's answer
//!   to NVIDIA's PPE stream extension query) runs once, on its own thread,
//!   when a plugin first asks, and is kept: it wakes a sleeping discrete GPU.
//!   Then, each time: GPU rendering draws now, on a GPU the check passed
//!   for, and the video is decoded on that GPU (`platform::UPSCALER_HWDEC`,
//!   not copy-back). [`check`] is the order; [`Reason`] the answers.
//! - **The filter.** While some plugin asked for upscaling and it is
//!   available, the host puts `@vestra-upscale:<platform::upscaler_filter>`
//!   last in mpv's video filter chain, scaling the video to the area it is
//!   shown in ([`factor`]; only above 1: a factor of 1 bypasses the
//!   upscaler). A new factor waits until the size holds for [`SETTLE`]
//!   (dragging a window border), then replaces the filter in place. It
//!   goes when the last plugin that asked turns it off or stops.
//! - **No "active" signal.** mpv sets the extension and reports nothing the
//!   driver does with it; `requested` means the filter is in the chain. The
//!   NVIDIA app's Super Resolution switch must be on, and no program can
//!   read it.
#![forbid(unsafe_code)]

use crate::host::{Event, Reply, RpcError};
use crate::platform::{self, UpscalerProbe};
use crate::playback::backend::Command;
use crate::playback::render_mode::Reason as RenderReason;
use crate::playback::renderer::Render;
use rsmpv::{Mpv, Node};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The filter's label in mpv's chain.
pub const LABEL: &str = "vestra-upscale";
/// How often the worker looks at the size and the decoding while upscaling
/// is asked for.
const CHECK_EVERY: Duration = Duration::from_millis(200);
/// How long a new factor must hold before the filter is rebuilt.
pub const SETTLE: Duration = Duration::from_millis(300);
/// A factor this much (relative) off the applied one rebuilds the filter;
/// mpv's own scaler covers smaller differences.
const STEP: f64 = 0.02;
/// The largest side the upscaler is asked to produce (the GPU renderer's
/// own limit).
const MAX_SIDE: f64 = 8192.;

/// Why upscaling does not work now: `UpscalingReason` in the API.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    /// The system's check failed: `platform`, `no-gpu`, `driver`,
    /// `gpu-unsupported` (`platform::UpscalerRefusal`).
    System { code: &'static str, message: String },
    /// GPU rendering is starting (`rendering`; not fixed).
    Starting,
    /// The player renders in software (`rendering`).
    Rendering(String),
    /// GPU rendering runs on a GPU the upscaler does not work on (`adapter`).
    Adapter(String),
    /// The video is not decoded on the GPU that renders it (`decoding`;
    /// not fixed: another file or filter changes it).
    Decoding(String),
}

impl Reason {
    pub fn code(&self) -> &'static str {
        match self {
            Self::System { code, .. } => code,
            Self::Starting | Self::Rendering(_) => "rendering",
            Self::Adapter(_) => "adapter",
            Self::Decoding(_) => "decoding",
        }
    }
    pub fn message(&self) -> String {
        match self {
            Self::Starting => "upscaling needs GPU rendering, which is starting".into(),
            Self::System { message, .. }
            | Self::Rendering(message)
            | Self::Adapter(message)
            | Self::Decoding(message) => message.clone(),
        }
    }
    /// Whether it holds for the rest of this run: `set(true)` is then
    /// refused (`unsupported`) instead of waiting for it.
    pub fn fixed(&self) -> bool {
        !matches!(self, Self::Starting | Self::Decoding(_))
    }
    pub fn json(&self) -> Value {
        json!({"code":self.code(),"message":self.message()})
    }
}

/// What [`check`] decides from.
pub struct Facts<'a> {
    pub probe: &'a UpscalerProbe,
    /// GPU rendering draws now, and why it does or not (`Render::now`).
    pub gpu_now: bool,
    pub render_reason: &'a RenderReason,
    /// The host's `rendering.state` when the request came (`Null` inside
    /// the worker): only for the advice in the message.
    pub rendering: &'a Value,
    /// The adapter GPU rendering runs on (`platform::gpu_render_adapter`).
    pub render_gpu: Option<(u64, String)>,
    /// The hardware decoding the upscaler needs (`platform::UPSCALER_HWDEC`).
    pub needed_hwdec: &'a str,
    /// mpv's `hwdec` option and, while a video decodes, `hwdec-current`.
    pub hwdec: Option<String>,
    pub hwdec_current: Option<String>,
    /// The system (`std::env::consts::OS`), for the adapter advice.
    pub os: &'a str,
}

/// First failure wins: the system's check, GPU rendering, the adapter,
/// the decoding.
pub fn check(facts: &Facts<'_>) -> Result<(), Reason> {
    let gpus = facts.probe.as_ref().map_err(|refusal| Reason::System {
        code: refusal.code,
        message: refusal.message.clone(),
    })?;
    if !facts.gpu_now {
        return Err(rendering_reason(facts.render_reason, facts.rendering));
    }
    let names = gpus
        .iter()
        .map(|gpu| gpu.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    match &facts.render_gpu {
        Some((id, _)) if gpus.iter().any(|gpu| gpu.id == *id) => {}
        other => {
            let renders = other.as_ref().map_or("an unknown GPU", |(_, name)| name);
            let hint = if facts.os == "windows" {
                "; choose the high-performance GPU for Vestra in Windows Settings ▸ System ▸ Display ▸ Graphics and restart the player"
            } else {
                ""
            };
            return Err(Reason::Adapter(format!(
                "GPU rendering runs on {renders}, and the upscaler works only on {names}{hint}"
            )));
        }
    }
    let needed = facts.needed_hwdec;
    if let Some(mode) = facts.hwdec.as_deref() {
        if mode.contains("copy") {
            return Err(Reason::Decoding(format!(
                "the video is decoded with copy-back ({mode}), for example because a video filter needs the frames in memory; upscaling needs it decoded on the GPU that renders it ({needed})"
            )));
        }
        if mode == "no" {
            return Err(Reason::Decoding(format!(
                "hardware decoding is off; upscaling needs the video decoded on the GPU that renders it ({needed})"
            )));
        }
    }
    match facts.hwdec_current.as_deref() {
        Some("no") => Err(Reason::Decoding(format!(
            "this video is decoded in software; upscaling needs it decoded on the GPU that renders it ({needed})"
        ))),
        Some(current) if !current.is_empty() && current != needed => {
            Err(Reason::Decoding(format!(
                "this video is decoded with {current}; upscaling needs it decoded on the GPU that renders it ({needed})"
            )))
        }
        _ => Ok(()),
    }
}

/// `rendering`: why the player renders in software, with what the user can
/// do, from the rendering state (`host/rendering.rs`).
fn rendering_reason(reason: &RenderReason, rendering: &Value) -> Reason {
    if *reason == RenderReason::Pending {
        return Reason::Starting;
    }
    let advice = if rendering["gpu"]["available"] == false {
        format!(
            "GPU rendering cannot be turned on here: {}",
            rendering["gpu"]["reason"]["message"]
                .as_str()
                .unwrap_or("the player does not say why")
        )
    } else if matches!(
        reason,
        RenderReason::InitFailed(_) | RenderReason::WindowFallback(_)
    ) {
        "GPU rendering stopped during this run; restart the player to try it again".into()
    } else if rendering["nextMode"] == "gpu" {
        "GPU rendering is turned on for the next start: restart the player".into()
    } else {
        "turn on Video ▸ GPU Rendering (Experimental) and restart the player".into()
    };
    Reason::Rendering(format!(
        "upscaling needs GPU rendering, and the player renders in software ({}); {advice}",
        reason.message()
    ))
}

/// The video's size as the upscaler gets it (after the orientation filter)
/// and the area it is shown in, in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    /// The render target (`osd-dimensions`): the video area.
    pub target: (f64, f64),
    /// The frame's storage size (`video-params/w`, `h`).
    pub size: (f64, f64),
    /// Its display size, with the pixel aspect (`dw`, `dh`).
    pub display: (f64, f64),
}

/// The scale factor that makes the video as large as it is shown: the
/// display size fitted into the target, over the storage size (the larger
/// of the two axes for non-square pixels), at most [`MAX_SIDE`] per side,
/// rounded to hundredths. `None` at 1 or below: the video is shown at its
/// own size or smaller, and the upscaler has nothing to do.
pub fn factor(geometry: Geometry) -> Option<f64> {
    let Geometry {
        target: (tw, th),
        size: (w, h),
        display: (dw, dh),
    } = geometry;
    if ![tw, th, w, h, dw, dh]
        .iter()
        .all(|v| v.is_finite() && *v >= 1.)
    {
        return None;
    }
    let k = (tw / dw).min(th / dh);
    let factor = (dw * k / w)
        .max(dh * k / h)
        .min(MAX_SIDE / w)
        .min(MAX_SIDE / h);
    let factor = (factor * 100.).round() / 100.;
    (factor > 1.).then_some(factor)
}

/// [`Geometry`] from mpv: `osd-dimensions` and `video-params`, swapped for a
/// quarter rotation (the orientation filter turns the frames before the
/// upscaler sees them).
fn geometry(mpv: &Mpv) -> Option<Geometry> {
    let read = |name: &str| {
        mpv.get_property::<f64>(name)
            .ok()
            .filter(|v| v.is_finite() && *v >= 1.)
    };
    let target = (read("osd-dimensions/w")?, read("osd-dimensions/h")?);
    let mut size = (read("video-params/w")?, read("video-params/h")?);
    let mut display = (read("video-params/dw")?, read("video-params/dh")?);
    if mpv
        .get_property::<i64>("video-params/rotate")
        .unwrap_or(0)
        .rem_euclid(180)
        == 90
    {
        size = (size.1, size.0);
        display = (display.1, display.0);
    }
    Some(Geometry {
        target,
        size,
        display,
    })
}

/// The labels of mpv's video filter chain, in order.
fn labels(mpv: &Mpv) -> Vec<String> {
    let Ok(Node::Array(filters)) = mpv.get_property::<Node>("vf") else {
        return Vec::new();
    };
    filters
        .iter()
        .map(|filter| match filter {
            Node::Map(fields) => fields
                .iter()
                .find(|(key, _)| key == "label")
                .and_then(|(_, value)| match value {
                    Node::String(label) => Some(label.clone()),
                    _ => None,
                })
                .unwrap_or_default(),
            _ => String::new(),
        })
        .collect()
}

/// What [`Upscaling::update`] does with the filter.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Step {
    Stay,
    Remove,
    Add(f64),
    KeepLast(f64),
}

enum Probe {
    Idle,
    Running(mpsc::Receiver<UpscalerProbe>),
    Done(UpscalerProbe),
}

/// A request waiting for the system's check.
enum Pending {
    /// `video.upscaling.state`, with the host's rendering state.
    State(Reply, Value),
    /// `video.upscaling.set(enabled)`.
    Set(Reply, bool, Value),
}

/// The control worker's upscaling: who asked, the system's check, the
/// filter in mpv's chain.
pub struct Upscaling {
    /// The plugins that asked for upscaling.
    owners: BTreeSet<u64>,
    probe: Probe,
    run_probe: fn() -> UpscalerProbe,
    pending: Vec<Pending>,
    /// The factor of the filter in mpv's chain.
    applied: Option<f64>,
    /// Another factor wanted since then (applied once it holds for SETTLE).
    wanted: Option<(f64, Instant)>,
    /// A factor mpv refused: not tried again until the factor changes.
    refused: Option<f64>,
    checked: Option<Instant>,
    /// Wakes the control worker when the check answers (`wake_with`).
    waker: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
}

impl Default for Upscaling {
    fn default() -> Self {
        Self::with_probe(platform::upscaler_probe)
    }
}

impl Upscaling {
    /// With another system check (tests).
    pub fn with_probe(run_probe: fn() -> UpscalerProbe) -> Self {
        Self {
            owners: BTreeSet::new(),
            probe: Probe::Idle,
            run_probe,
            pending: Vec::new(),
            applied: None,
            wanted: None,
            refused: None,
            checked: None,
            waker: None,
        }
    }

    /// A worker command: the two requests (answered once the system's
    /// check is known; `true`), and a stopped plugin's release (`false`:
    /// the worker's own handling goes on).
    pub fn command(
        &mut self,
        mpv: &Mpv,
        render: &Render,
        command: &Command,
        events: &mpsc::Sender<Event>,
    ) -> bool {
        let rendering = || command.params["rendering"].clone();
        match command.method.as_str() {
            "owner.disabled" => {
                self.release(mpv, render, command.reply.owner);
                false
            }
            "video.upscaling.state" => {
                self.pending
                    .push(Pending::State(command.reply.clone(), rendering()));
                self.answer(mpv, render, events);
                true
            }
            "video.upscaling.set" => {
                let Some(enabled) = command.params["enabled"].as_bool() else {
                    let error = RpcError::invalid("enabled must be a boolean");
                    let _ = events.send(Event::Reply(command.reply.clone(), Err(error)));
                    return true;
                };
                if !enabled {
                    // At once, whatever the check says; an earlier `true`
                    // still waiting for the check no longer turns it on.
                    for pending in &mut self.pending {
                        if let Pending::Set(reply, on, _) = pending
                            && reply.owner == command.reply.owner
                        {
                            *on = false;
                        }
                    }
                    self.owners.remove(&command.reply.owner);
                    self.update(mpv, render, Instant::now());
                }
                self.pending
                    .push(Pending::Set(command.reply.clone(), enabled, rendering()));
                self.answer(mpv, render, events);
                true
            }
            _ => false,
        }
    }

    /// Every worker iteration: answer requests once the system's check is
    /// known; while upscaling is asked for, follow the size and the
    /// decoding (every [`CHECK_EVERY`]).
    pub fn poll(&mut self, mpv: &Mpv, render: &Render, events: &mpsc::Sender<Event>) {
        if let Probe::Running(receiver) = &self.probe {
            match receiver.try_recv() {
                Ok(probe) => self.probe = Probe::Done(probe),
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.probe = Probe::Done(Err(platform::UpscalerRefusal {
                        code: "gpu-unsupported",
                        message: "the upscaler check stopped without an answer".into(),
                    }));
                }
            }
            if matches!(self.probe, Probe::Done(_)) {
                self.answer(mpv, render, events);
            }
        }
        let now = Instant::now();
        if self.due_in(now).is_some_and(|due| due.is_zero()) {
            self.update(mpv, render, now);
        }
    }

    /// How long the control worker may wait before [`Upscaling::poll`] has
    /// work: `None` while nobody asked for upscaling and no filter is in
    /// the chain (nothing to follow); else the next look at the size and
    /// the decoding ([`CHECK_EVERY`]) or the end of a new size's
    /// [`SETTLE`], whichever comes first. A window resize does not wake
    /// the worker, so it waits at most this long (a timer while upscaling
    /// is on: about 5 wake-ups a second, not a 2 ms poll).
    pub fn due_in(&self, now: Instant) -> Option<Duration> {
        if self.owners.is_empty() && self.applied.is_none() {
            return None;
        }
        let check = self.checked.map_or(Duration::ZERO, |at| {
            CHECK_EVERY.saturating_sub(now.saturating_duration_since(at))
        });
        let settle = self
            .wanted
            .map(|(_, since)| SETTLE.saturating_sub(now.saturating_duration_since(since)));
        Some(settle.map_or(check, |settle| settle.min(check)))
    }

    /// Wake the control worker with `wake` when the system's check answers
    /// (its thread calls it), so waiting requests are answered at once.
    pub fn wake_with(&mut self, wake: impl Fn() + Send + Sync + 'static) {
        self.waker = Some(std::sync::Arc::new(wake));
    }

    /// The plugin stopped: its request goes, and with the last one the
    /// filter (plugin independence: a stopped plugin's resources are
    /// revoked); its unanswered requests are dropped.
    pub fn release(&mut self, mpv: &Mpv, render: &Render, owner: u64) {
        self.pending.retain(|pending| match pending {
            Pending::State(reply, _) | Pending::Set(reply, _, _) => reply.owner != owner,
        });
        if self.owners.remove(&owner) {
            self.update(mpv, render, Instant::now());
        }
    }

    /// Start the system's check once, on its own thread.
    fn start_probe(&mut self) {
        if !matches!(self.probe, Probe::Idle) {
            return;
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        let run = self.run_probe;
        let waker = self.waker.clone();
        let spawned = std::thread::Builder::new()
            .name("vestra-upscaler-check".into())
            .spawn(move || {
                let _ = sender.send(run());
                if let Some(wake) = waker {
                    wake();
                }
            });
        self.probe = match spawned {
            Ok(_) => Probe::Running(receiver),
            Err(error) => Probe::Done(Err(platform::UpscalerRefusal {
                code: "gpu-unsupported",
                message: format!("the upscaler check could not start ({error})"),
            })),
        };
    }

    /// Answer the waiting requests once the system's check is known.
    fn answer(&mut self, mpv: &Mpv, render: &Render, events: &mpsc::Sender<Event>) {
        self.start_probe();
        if !matches!(self.probe, Probe::Done(_)) || self.pending.is_empty() {
            return;
        }
        for pending in std::mem::take(&mut self.pending) {
            let (reply, result) = match pending {
                Pending::State(reply, rendering) => {
                    let state = self.state(mpv, render, reply.owner, &rendering);
                    (reply, Ok(state))
                }
                Pending::Set(reply, enabled, rendering) => {
                    let result = match self.availability(mpv, render, &rendering) {
                        Err(reason) if enabled && reason.fixed() => Err(RpcError::new(
                            "unsupported",
                            format!("video upscaling cannot work here: {}", reason.message()),
                        )),
                        _ => {
                            if enabled && self.owners.insert(reply.owner) {
                                self.update(mpv, render, Instant::now());
                            }
                            Ok(self.state(mpv, render, reply.owner, &rendering))
                        }
                    };
                    (reply, result)
                }
            };
            let _ = events.send(Event::Reply(reply, result));
        }
    }

    /// [`check`] with what the worker sees now; `Starting` before the
    /// system's check is known.
    fn availability(&self, mpv: &Mpv, render: &Render, rendering: &Value) -> Result<(), Reason> {
        let Probe::Done(probe) = &self.probe else {
            return Err(Reason::Starting);
        };
        let (gpu_now, render_reason) = render.now();
        let gpu_now = gpu_now && render.gpu().is_some();
        let text = |name: &str| {
            mpv.get_property::<String>(name)
                .ok()
                .filter(|value| !value.is_empty())
        };
        check(&Facts {
            probe,
            gpu_now,
            render_reason: &render_reason,
            rendering,
            render_gpu: platform::gpu_render_adapter(),
            needed_hwdec: platform::UPSCALER_HWDEC,
            hwdec: text("hwdec"),
            hwdec_current: text("hwdec-current"),
            os: std::env::consts::OS,
        })
    }

    /// The `UpscalingState` for `owner`.
    fn state(&self, mpv: &Mpv, render: &Render, owner: u64, rendering: &Value) -> Value {
        let reason = self.availability(mpv, render, rendering).err();
        json!({
            "available": reason.is_none(),
            "reason": reason.as_ref().map(Reason::json),
            "enabled": self.owners.contains(&owner),
            "requested": self.applied.is_some(),
            "factor": self.applied,
        })
    }

    /// Bring mpv's chain to what is wanted now: no filter, or the filter
    /// with the factor of the current size, last in the chain.
    fn update(&mut self, mpv: &Mpv, render: &Render, now: Instant) {
        self.checked = Some(now);
        let desired =
            if self.owners.is_empty() || self.availability(mpv, render, &Value::Null).is_err() {
                None
            } else {
                geometry(mpv).and_then(factor)
            };
        match self.step(desired, now) {
            Step::Stay => {}
            Step::Remove => self.remove(mpv),
            Step::Add(next) => {
                if self.add(mpv, next) {
                    self.keep_last(mpv, next);
                }
            }
            Step::KeepLast(applied) => self.keep_last(mpv, applied),
        }
    }

    /// What to do for the `desired` factor (`None`: no filter): a first
    /// filter at once, a removal at once, a new factor once it held for
    /// [`SETTLE`] and differs by [`STEP`] or more, not again at a factor mpv
    /// refused.
    fn step(&mut self, desired: Option<f64>, now: Instant) -> Step {
        match (self.applied, desired) {
            (None, None) => {
                self.wanted = None;
                Step::Stay
            }
            (Some(_), None) => Step::Remove,
            (None, Some(next)) if self.refused == Some(next) => Step::Stay,
            (None, Some(next)) => Step::Add(next),
            (Some(applied), Some(next)) if ((next - applied) / applied).abs() < STEP => {
                self.wanted = None;
                Step::KeepLast(applied)
            }
            (Some(_), Some(next)) if self.refused == Some(next) => Step::Stay,
            (Some(_), Some(next)) => match self.wanted {
                Some((wanted, since)) if wanted == next => {
                    if now.duration_since(since) >= SETTLE {
                        Step::Add(next)
                    } else {
                        Step::Stay
                    }
                }
                _ => {
                    self.wanted = Some((next, now));
                    Step::Stay
                }
            },
        }
    }

    /// Put the filter with `factor` in the chain: in place when it is there
    /// (`vf add` replaces a label where it stands), else last. `false`: mpv
    /// refused it (not tried again at this factor).
    fn add(&mut self, mpv: &Mpv, factor: f64) -> bool {
        self.wanted = None;
        let Some(filter) = platform::upscaler_filter(factor) else {
            return false;
        };
        match mpv.command(&["vf", "add", &format!("@{LABEL}:{filter}")]) {
            Ok(()) => {
                diag!(
                    "upscaling: {filter} (was {})",
                    self.applied
                        .map_or_else(|| "off".into(), |f| format!("{f:.2}"))
                );
                self.applied = Some(factor);
                self.refused = None;
                true
            }
            Err(error) => {
                diag!("upscaling: mpv refused {filter}: {error}");
                self.refused = Some(factor);
                false
            }
        }
    }

    /// Take the filter out of the chain.
    fn remove(&mut self, mpv: &Mpv) {
        self.wanted = None;
        self.refused = None;
        if self.applied.take().is_some() {
            match mpv.command(&["vf", "remove", &format!("@{LABEL}")]) {
                Ok(()) => diag!("upscaling: off"),
                Err(error) => diag!("upscaling: removing the filter failed: {error}"),
            }
        }
    }

    /// The filter must stay last (a filter added after it, such as a
    /// plugin's first video filter chain, would get its frames): move it
    /// back, or add it again when it is gone.
    fn keep_last(&mut self, mpv: &Mpv, factor: f64) {
        let labels = labels(mpv);
        if labels.last().is_some_and(|last| last == LABEL) {
            return;
        }
        diag!("upscaling: the filter is not last in {labels:?}; moving it");
        if labels.iter().any(|label| label == LABEL) {
            let _ = mpv.command(&["vf", "remove", &format!("@{LABEL}")]);
        }
        self.applied = None;
        // Not present now: `vf add` appends it.
        self.add(mpv, factor);
    }
}

#[cfg(test)]
mod tests;
