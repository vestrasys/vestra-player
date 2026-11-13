//! Video upscaling (`video.upscaling.state`, `video.upscaling.set`;
//! group `video.upscaling`, SDK 1.5.37; doc/architecture/README.md
//! §4.4), on the control worker.
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
//! - **The publish gate** ([`Upscaling::follow_gate`]). While a new
//!   source's orientation is prepared (`source_geometry.rs`, the gate
//!   closed) the filter is not in the chain: it goes as soon as the gate
//!   closes and comes back once the geometry confirmed the output. A filter
//!   that scaled the output meanwhile (added while a slow stream started,
//!   or left from the last file) kept the gate from seeing the size it
//!   waited for, and the picture froze on the last frame.
//! - **No "active" signal.** mpv sets the extension and reports nothing the
//!   driver does with it; `requested` means the filter is in the chain. The
//!   NVIDIA app's Super Resolution switch must be on, and no program can
//!   read it.
//! - **Monitoring** (player 0.5.25). The state also has the video's size,
//!   the video area, the rendering GPU, mpv's `hwdec-current` and whether
//!   the filter is in mpv's chain; `video.upscaling.changed` carries it
//!   (without the plugin's own `enabled`) when it changes, at the worker's
//!   looks while upscaling is followed. A `decoding` reason says why
//!   (`cause`) and, for copy-back the host switched to, whose video chain
//!   (`plugins`, from `Processing::copy_back_owners`).
#![forbid(unsafe_code)]

use crate::host::{Event, Reply, RpcError};
use crate::platform::{self, UpscalerProbe};
use crate::playback::backend::Command;
use crate::playback::processing::ChainOwner;
use crate::playback::render_mode::Reason as RenderReason;
use crate::playback::renderer::Render;
use rsmpv::{Mpv, Node};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

/// The filter's label in mpv's chain.
pub const LABEL: &str = "vestra-upscale";
/// The event with the shared state (`UpscalingState` without `enabled`),
/// player 0.5.25 (`EVENTS` in capabilities/events.rs).
pub const CHANGED: &str = "video.upscaling.changed";
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
    /// not fixed: another file or filter changes it), and why
    /// (`UpscalingReason.cause`, player 0.5.25); with `copy-back` the
    /// plugins whose video chain made the host switch to it
    /// (`UpscalingReason.plugins`).
    Decoding {
        cause: Cause,
        message: String,
        plugins: Vec<ChainOwner>,
    },
    /// The system's check has not answered yet (`unchecked`): only in the
    /// answer to `set(false)`, which never starts the check.
    Unchecked,
}

/// Why the video is not decoded on the GPU that renders it: the optional
/// `cause` of an `UpscalingReason` with code `decoding` (player 0.5.25,
/// SDK 1.5.43 `UpscalingDecodingCause`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    /// `copy-back`: decoded on the GPU and copied back to memory, usually
    /// because a plugin's video filter chain needs the frames in memory
    /// (`Processing::copy_back_owners`).
    CopyBack,
    /// `hwdec-off`: mpv's `hwdec` option is `no`.
    HwdecOff,
    /// `software`: this video is decoded in software (a cover picture, a
    /// format the GPU does not decode, a hardware decoder that failed).
    Software,
    /// `other-hwdec`: decoded with another hardware decoder.
    OtherHwdec,
}

impl Cause {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CopyBack => "copy-back",
            Self::HwdecOff => "hwdec-off",
            Self::Software => "software",
            Self::OtherHwdec => "other-hwdec",
        }
    }
}

impl Reason {
    pub fn code(&self) -> &'static str {
        match self {
            Self::System { code, .. } => code,
            Self::Starting | Self::Rendering(_) => "rendering",
            Self::Adapter(_) => "adapter",
            Self::Decoding { .. } => "decoding",
            Self::Unchecked => "unchecked",
        }
    }
    pub fn message(&self) -> String {
        match self {
            Self::Starting => "upscaling needs GPU rendering, which is starting".into(),
            Self::Unchecked => {
                "the player has not checked the GPU yet; state() or set(true) checks it".into()
            }
            Self::System { message, .. }
            | Self::Rendering(message)
            | Self::Adapter(message)
            | Self::Decoding { message, .. } => message.clone(),
        }
    }
    /// Whether it holds for the rest of this run: `set(true)` is then
    /// refused (`unsupported`) instead of waiting for it.
    pub fn fixed(&self) -> bool {
        !matches!(
            self,
            Self::Starting | Self::Decoding { .. } | Self::Unchecked
        )
    }
    /// `UpscalingReason`: `cause` only with code `decoding`, `plugins` only
    /// with cause `copy-back` (empty: the copy-back mode is not one the
    /// host set for a plugin's chain).
    pub fn json(&self) -> Value {
        match self {
            Self::Decoding {
                cause,
                message,
                plugins,
            } => {
                let mut reason =
                    json!({"code":self.code(),"message":message,"cause":cause.as_str()});
                if *cause == Cause::CopyBack {
                    reason["plugins"] = plugins
                        .iter()
                        .map(|owner| json!({"id":owner.id,"name":owner.name}))
                        .collect();
                }
                reason
            }
            _ => json!({"code":self.code(),"message":self.message()}),
        }
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
    /// The plugins whose video filter chain made the host switch to
    /// copy-back decoding (`Processing::copy_back_owners`), for the message.
    pub copy_back_owners: &'a [ChainOwner],
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
            let because = match facts.copy_back_owners {
                [] => "for example because a video filter needs the frames in memory".to_owned(),
                [one] => format!(
                    "because the video filter of {} needs the frames in memory",
                    one.name
                ),
                many => format!(
                    "because the video filters of {} need the frames in memory",
                    many.iter()
                        .map(|owner| owner.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            };
            return Err(Reason::Decoding {
                cause: Cause::CopyBack,
                message: format!(
                    "the video is decoded with copy-back ({mode}), {because}; upscaling needs it decoded on the GPU that renders it ({needed})"
                ),
                plugins: facts.copy_back_owners.to_vec(),
            });
        }
        if mode == "no" {
            return Err(decoding(
                Cause::HwdecOff,
                format!(
                    "hardware decoding is off; upscaling needs the video decoded on the GPU that renders it ({needed})"
                ),
            ));
        }
    }
    match facts.hwdec_current.as_deref() {
        Some("no") => Err(decoding(
            Cause::Software,
            format!(
                "this video is decoded in software; upscaling needs it decoded on the GPU that renders it ({needed})"
            ),
        )),
        Some(current) if !current.is_empty() && current != needed => Err(decoding(
            Cause::OtherHwdec,
            format!(
                "this video is decoded with {current}; upscaling needs it decoded on the GPU that renders it ({needed})"
            ),
        )),
        _ => Ok(()),
    }
}

/// A `decoding` reason no plugin's chain is named in.
fn decoding(cause: Cause, message: String) -> Reason {
    Reason::Decoding {
        cause,
        message,
        plugins: Vec::new(),
    }
}

/// `rendering`: why the player renders in software, with what the user can
/// do, from the rendering state (`host/rendering.rs`).
fn rendering_reason(reason: &RenderReason, rendering: &Value) -> Reason {
    if *reason == RenderReason::Pending {
        return Reason::Starting;
    }
    Reason::Rendering(format!(
        "upscaling needs GPU rendering, and the player renders in software ({}); {}",
        reason.message(),
        rendering_advice(reason, rendering)
    ))
}

/// What the user can do to get GPU rendering, from the rendering state
/// (`host/rendering.rs`); also motion smoothing's Match Display's advice.
pub(crate) fn rendering_advice(reason: &RenderReason, rendering: &Value) -> String {
    if rendering["gpu"]["available"] == false {
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
        "turn on GPU Rendering in Picture ▸ Rendering and restart the player".into()
    }
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
    let target = area(mpv)?;
    let (size, display) = video_size(mpv)?;
    Some(Geometry {
        target,
        size,
        display,
    })
}

/// A size mpv reports, when it is one (finite, at least 1).
fn read_size(mpv: &Mpv, name: &str) -> Option<f64> {
    mpv.get_property::<f64>(name)
        .ok()
        .filter(|v| v.is_finite() && *v >= 1.)
}

/// The render target (`osd-dimensions`): the video area, in pixels.
fn area(mpv: &Mpv) -> Option<(f64, f64)> {
    Some((
        read_size(mpv, "osd-dimensions/w")?,
        read_size(mpv, "osd-dimensions/h")?,
    ))
}

/// The video's storage and display size (`video-params`) as the upscaler
/// gets it: swapped for a quarter rotation.
fn video_size(mpv: &Mpv) -> Option<((f64, f64), (f64, f64))> {
    let read = |name: &str| read_size(mpv, name);
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
    Some((size, display))
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
    /// The filter for a factor (`platform::upscaler_filter`; `None` where
    /// the platform has no upscaler). Tests put in a stand-in every mpv
    /// build accepts, so the chain handling is tested off Windows too.
    filter: fn(f64) -> Option<String>,
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
    /// The plugins whose video chain switched decoding to copy-back
    /// (`note_copy_back`), named in the `decoding` message.
    copy_back_owners: Vec<ChainOwner>,
    /// The last `video.upscaling.changed` sent (`publish`).
    published: Option<Value>,
    /// The renderer's publish gate (`follow_gate`; none: always open).
    gate: Option<Arc<AtomicBool>>,
    /// The gate as the last look at the chain saw it: a change is looked
    /// at at once (`due_in`).
    gate_seen: bool,
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
            filter: platform::upscaler_filter,
            pending: Vec::new(),
            applied: None,
            wanted: None,
            refused: None,
            checked: None,
            waker: None,
            copy_back_owners: Vec::new(),
            published: None,
            gate: None,
            gate_seen: true,
        }
    }

    /// Follow the renderer's publish gate: while it is closed (a new
    /// source's geometry is being prepared) the filter stays out of the
    /// chain; once it opens the filter goes back in at the next look, which
    /// is then at once (`due_in`).
    pub fn follow_gate(&mut self, gate: Arc<AtomicBool>) {
        self.gate = Some(gate);
    }

    fn gate_open(&self) -> bool {
        self.gate
            .as_ref()
            .is_none_or(|gate| gate.load(Ordering::Acquire))
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
                // Published once its chains are gone too (`stopped`).
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
                    // Answered now: turning off never needs the check, so
                    // it does not start it (it wakes a sleeping discrete GPU);
                    // before the check `available` is false with `unchecked`.
                    let state = self.state(mpv, render, command.reply.owner, &rendering());
                    let _ = events.send(Event::Reply(command.reply.clone(), Ok(state)));
                    self.publish(mpv, render, events);
                    return true;
                }
                self.pending
                    .push(Pending::Set(command.reply.clone(), enabled, rendering()));
                self.answer(mpv, render, events);
                true
            }
            _ => false,
        }
    }

    /// Every worker iteration, after [`Upscaling::take_check`]: answer
    /// requests once the system's check is taken; while upscaling is asked
    /// for, follow the size and the decoding (every [`CHECK_EVERY`]).
    /// `copy_back`: [`Upscaling::note_copy_back`]'s list, read only when
    /// this iteration checks the availability (the check answered, or a
    /// look is due), not every worker iteration.
    pub fn poll(
        &mut self,
        mpv: &Mpv,
        render: &Render,
        events: &mpsc::Sender<Event>,
        copy_back: impl Fn() -> Vec<ChainOwner>,
    ) {
        // The check is taken only by `take_check`, which the worker calls
        // before this, then lets motion smoothing step aside for a request
        // that will be accepted (`Motion::hand_over`). Taking it here could
        // answer a check that arrived after the hand-over looked.
        if matches!(self.probe, Probe::Done(_)) && !self.pending.is_empty() {
            self.copy_back_owners = copy_back();
            self.answer(mpv, render, events);
        }
        let now = Instant::now();
        if self.due_in(now).is_some_and(|due| due.is_zero()) {
            self.copy_back_owners = copy_back();
            self.update(mpv, render, now);
            self.publish(mpv, render, events);
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
        // The gate closed (a new source) or opened (its geometry confirmed)
        // since the last look: take the filter out or put it back now.
        if self.gate_open() != self.gate_seen {
            return Some(Duration::ZERO);
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

    /// The plugins whose video filter chain made the host decode with
    /// copy-back (`Processing::copy_back_owners`), kept current by the
    /// control worker for the `decoding` message.
    pub fn note_copy_back(&mut self, owners: Vec<ChainOwner>) {
        self.copy_back_owners = owners;
    }

    /// A plugin stopped and the worker revoked everything it held (its
    /// chains, which may have restored zero-copy decoding): publish the
    /// state now, so `video.upscaling.changed` never names a stopped
    /// plugin's chain.
    pub fn stopped(
        &mut self,
        mpv: &Mpv,
        render: &Render,
        events: &mpsc::Sender<Event>,
        copy_back: Vec<ChainOwner>,
    ) {
        self.copy_back_owners = copy_back;
        self.publish(mpv, render, events);
    }

    /// `owner` has upscaling on (or a request waiting for the check): a live
    /// resource, so lazy activation never parks it (ai-upscaling.md §8.5).
    pub fn holds(&self, owner: u64) -> bool {
        self.owners.contains(&owner)
            || self.pending.iter().any(|pending| match pending {
                Pending::State(reply, _) | Pending::Set(reply, ..) => reply.owner == owner,
            })
    }
    /// Some plugin has upscaling on: motion smoothing waits meanwhile
    /// (`playback/motion.rs`: the two filters cannot share the chain).
    pub fn asked(&self) -> bool {
        !self.owners.is_empty()
    }
    /// `owner` has upscaling on (not only a request waiting): what its next
    /// activation turns on again (lazy activation's `held`).
    pub fn owns(&self, owner: u64) -> bool {
        self.owners.contains(&owner)
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
        self.publish(mpv, render, events);
    }

    /// [`check`] with what the worker sees now; `Unchecked` before the
    /// system's check is known.
    fn availability(&self, mpv: &Mpv, render: &Render, rendering: &Value) -> Result<(), Reason> {
        let Probe::Done(probe) = &self.probe else {
            return Err(Reason::Unchecked);
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
            copy_back_owners: &self.copy_back_owners,
            os: std::env::consts::OS,
        })
    }

    /// Whether `set(true)` would be accepted now: the system's check
    /// answered, and no reason that holds for the run stands in the way.
    /// Motion smoothing yields to upscaling only then
    /// (`Motion::yield_to_upscaling`); where upscaling is refused (no
    /// NVIDIA GPU) it stays. Before the check nothing is decided: the
    /// request waits for it, and [`Upscaling::accepts_waiting`] decides.
    pub fn would_accept(&self, mpv: &Mpv, render: &Render) -> bool {
        matches!(self.probe, Probe::Done(_))
            && self
                .availability(mpv, render, &Value::Null)
                .map_or_else(|reason| !reason.fixed(), |()| true)
    }

    /// Take the system's check if its thread answered, without answering
    /// the requests waiting for it ([`Upscaling::poll`] does that).
    pub fn take_check(&mut self) {
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
        }
    }

    /// Whether a `set(true)` that waited for the system's check (the run's
    /// first request) will be accepted when [`Upscaling::poll`] answers
    /// it: the check is taken ([`Upscaling::take_check`]) and
    /// [`Upscaling::would_accept`]. A later `set(false)` from the same
    /// plugin, waiting too, cancels it. Motion smoothing steps aside then,
    /// before the answer (`Motion::hand_over`).
    pub fn accepts_waiting(&self, mpv: &Mpv, render: &Render) -> bool {
        self.pending
            .iter()
            .any(|pending| matches!(pending, Pending::Set(_, true, _)))
            && self.would_accept(mpv, render)
    }

    /// The `UpscalingState` for `owner`.
    fn state(&self, mpv: &Mpv, render: &Render, owner: u64, rendering: &Value) -> Value {
        let mut state = self.shared_state(mpv, render, rendering);
        state["enabled"] = self.owners.contains(&owner).into();
        state
    }

    /// The `UpscalingState` without `enabled` (each plugin's own): what
    /// `video.upscaling.changed` carries. The monitoring fields (player
    /// 0.5.25) are read from mpv each time and never start the system's
    /// check.
    fn shared_state(&self, mpv: &Mpv, render: &Render, rendering: &Value) -> Value {
        let reason = self.availability(mpv, render, rendering).err();
        // Whole pixels (mpv reports them as numbers).
        let px = |v: f64| v.round() as u64;
        let video = video_size(mpv);
        let area = area(mpv);
        let hwdec_current = mpv
            .get_property::<String>("hwdec-current")
            .ok()
            .filter(|value| !value.is_empty());
        let gpu = render
            .gpu()
            .and_then(|_| platform::gpu_render_adapter())
            .map(|(_, name)| name);
        json!({
            "available": reason.is_none(),
            "reason": reason.as_ref().map(Reason::json),
            "requested": self.applied.is_some(),
            "factor": self.applied,
            "video": video.map(|(size, display)| json!({
                "width": px(size.0),
                "height": px(size.1),
                "displayWidth": px(display.0),
                "displayHeight": px(display.1),
            })),
            "area": area.map(|(w, h)| json!({"width": px(w), "height": px(h)})),
            "gpu": gpu,
            "hwdecCurrent": hwdec_current,
            "inChain": labels(mpv).iter().any(|label| label == LABEL),
        })
    }

    /// `video.upscaling.changed` (player 0.5.25) when the shared state
    /// differs from the last one sent: after a request that changed it and
    /// at each look while upscaling is followed ([`Upscaling::due_in`]), so
    /// at most about 5 a second during a window resize, none while nobody
    /// has it on. Nothing before the system's check answered.
    fn publish(&mut self, mpv: &Mpv, render: &Render, events: &mpsc::Sender<Event>) {
        if !matches!(self.probe, Probe::Done(_)) {
            return;
        }
        let state = self.shared_state(mpv, render, &Value::Null);
        if self.published.as_ref() == Some(&state) {
            return;
        }
        self.published = Some(state.clone());
        let _ = events.send(Event::Broadcast(CHANGED.into(), state));
    }

    /// Bring mpv's chain to what is wanted now: no filter, or the filter
    /// with the factor of the current size, last in the chain.
    fn update(&mut self, mpv: &Mpv, render: &Render, now: Instant) {
        let wanted = !self.owners.is_empty()
            && self.gate_open()
            && self.availability(mpv, render, &Value::Null).is_ok();
        let factor = if wanted {
            geometry(mpv).and_then(factor)
        } else {
            None
        };
        self.follow(mpv, factor, now);
    }

    /// [`Upscaling::update`] with the factor the size asks for (`None`: no
    /// filter); none while the publish gate is closed.
    fn follow(&mut self, mpv: &Mpv, factor: Option<f64>, now: Instant) {
        self.checked = Some(now);
        let open = self.gate_open();
        self.gate_seen = open;
        if !open && self.applied.is_some() {
            diag!("upscaling: out of the chain until the new source's geometry is confirmed");
        }
        let desired = factor.filter(|_| open);
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
            (None, Some(next)) if self.refused == Some(next) => {
                self.wanted = None;
                Step::Stay
            }
            (None, Some(next)) => Step::Add(next),
            (Some(applied), Some(next)) if ((next - applied) / applied).abs() < STEP => {
                self.wanted = None;
                Step::KeepLast(applied)
            }
            // Back at a refused factor: no settle is pending any more (a
            // kept `wanted` would make `due_in` 0 and the worker spin).
            (Some(_), Some(next)) if self.refused == Some(next) => {
                self.wanted = None;
                Step::Stay
            }
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
        let Some(filter) = (self.filter)(factor) else {
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
