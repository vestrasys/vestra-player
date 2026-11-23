//! Motion smoothing (`video.motion.state`, `video.motion.set`; capability
//! `video.motion`, SDK 1.7.19; doc/architecture/README.md §4.4a), on the
//! control worker: the video's frame rate doubled by a GPU vendor's frame-rate
//! converter, real in-between frames rather than blends. Per vendor
//! ([`Engine`]): `amd` is AMD AMF's `frc_amf`, `nvidia` FFmpeg's
//! `fruc_vulkan` (NVIDIA's optical flow hardware through Vulkan, RTX 30 and
//! newer). One converter is in the chain at a time: the engine asked for
//! first holds it while some plugin asks for it, and the other engine's
//! requests wait (`other-engine`, with a message that names the engine). Built like video upscaling
//! (`upscaling.rs`); the measurements are in the same section.
//!
//! - **Availability** ([`check`], [`Reason`]). The system's check per
//!   engine (`platform::motion_probe`: on Windows an AMD GPU and AMD's AMF
//!   runtime, or an NVIDIA RTX 30+ GPU with a driver of 573.38 or newer; no
//!   device is created and nothing of Vulkan is loaded) runs once, when a
//!   plugin first asks for that engine. The filter itself (mpv's libmpv must
//!   have FFmpeg's `frc_amf` or `fruc_vulkan`) shows when it goes in:
//!   refused by mpv, it is `filter` for the rest of the run. NVIDIA's
//!   converter gets a Vulkan device of its own, which the filter creates
//!   when it goes into the chain and frees when it leaves (libmpv's
//!   lavfi `hwdevice=`, src/packaging/libmpv): no Vulkan device otherwise,
//!   and the renderers are untouched. Then, each
//!   time: AI upscaling is off (the two filters cannot share the chain:
//!   upscaling needs Direct3D 11 frames, the converter frames in memory;
//!   measured, both orders fail to configure), the converter worked on this
//!   video (`failed`, `overload`), and the video's rate fits ([`rate`]).
//! - **Modes.** `double` puts `@vestra-motion:<platform::motion_filter>`
//!   last in mpv's chain: twice the frames, shown as they come. `display`
//!   also has mpv interpolate the doubled frames to the display
//!   (`video_render::Options::set_resample`: `display-resample`), so 24 fps
//!   doubled to 48 meets a 60 Hz display without judder; GPU rendering only.
//!   With software rendering, `double` only where twice the rate is the
//!   display's (30 fps on 60 Hz): other rates would judder and the software
//!   renderer cannot resample.
//! - **Frames in memory.** The converter fails on Direct3D 11 frames
//!   (`AMFFRC->Init() failed with error 4`), so while the filter is in the
//!   chain the decoding uses copy-back (`Processing::hold_frames_in_memory`,
//!   switched first, undone after the filter went). The filter goes in only
//!   once mpv's decoder hands over frames in memory (`frames_ready`): the
//!   switch reopens the decoder on mpv's thread, and a Direct3D 11 frame
//!   that reached the converter first failed the graph in the player.
//! - **Fallback** ([`Motion::watch`]). While the video plays, every
//!   [`WINDOW`] after a [`WARMUP`] (each pause, seek or new file starts
//!   again), the filter's output rate (`estimated-vf-fps`) and mpv's dropped
//!   frames (`frame-drop-count`, `decoder-frame-drop-count`) are compared to
//!   the doubled rate. Behind once: the faster motion search (AMD:
//!   `mv_search_mode=performance`; NVIDIA: `perf=fast`, which above 1440p
//!   it already uses, so there it goes straight on); behind again:
//!   the filter goes for this video, with an `overload` reason. mpv
//!   disabling the filter (its log line) takes it out with `failed` and the
//!   converter's own error; a converter that cannot start on this system
//!   ([`device_failure`]) is `runtime` for the rest of the run.
//! - **Who asked.** Each plugin's mode is its own; the strongest one asked
//!   for applies (`display` over `double`). A stopped plugin's request goes.
#![forbid(unsafe_code)]

use crate::host::{Event, RpcError};
use crate::platform::{self, MotionProbe};
use crate::playback::backend::Command;
use crate::playback::render_mode::Reason as RenderReason;
use crate::playback::renderer::Render;
use rsmpv::{Mpv, Node};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

/// Half the doubled estimate above the kept input by more than this is a
/// new input rate (`Motion::measure_input`).
const RERATE: f64 = 1.1;

/// The filter's label in mpv's chain.
pub const LABEL: &str = "vestra-motion";
/// The engines (`MotionEngine` in the API): AMD AMF's converter and
/// NVIDIA's (FFmpeg's `fruc_vulkan`).
pub const AMD: &str = "amd";
pub const NVIDIA: &str = "nvidia";

/// A vendor's frame-rate converter: `MotionEngine` in the API.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Engine {
    Amd,
    Nvidia,
}

impl Engine {
    pub const ALL: [Self; 2] = [Self::Amd, Self::Nvidia];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Amd => AMD,
            Self::Nvidia => NVIDIA,
        }
    }
    pub fn from_name(text: &str) -> Option<Self> {
        match text {
            AMD => Some(Self::Amd),
            NVIDIA => Some(Self::Nvidia),
            _ => None,
        }
    }
    /// The vendor, for the messages.
    pub fn vendor(self) -> &'static str {
        match self {
            Self::Amd => "AMD",
            Self::Nvidia => "NVIDIA",
        }
    }
    /// The vendor's converter in FFmpeg, for the messages.
    fn converter(self) -> &'static str {
        match self {
            Self::Amd => "frc_amf",
            Self::Nvidia => "fruc_vulkan",
        }
    }
    fn at(self) -> usize {
        self as usize
    }
}

/// One value per engine.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PerEngine<T>([T; 2]);

impl<T> std::ops::Index<Engine> for PerEngine<T> {
    type Output = T;
    fn index(&self, engine: Engine) -> &T {
        &self.0[engine.at()]
    }
}

impl<T> std::ops::IndexMut<Engine> for PerEngine<T> {
    fn index_mut(&mut self, engine: Engine) -> &mut T {
        &mut self.0[engine.at()]
    }
}
/// The event with the shared state (`MotionState` without `mode`).
pub const CHANGED: &str = "video.motion.changed";
/// How often the worker looks at the rate and the output while motion
/// smoothing is asked for.
const CHECK_EVERY: Duration = Duration::from_millis(500);
/// How often the worker looks while the filter waits for copy-back decoding.
const READY_EVERY: Duration = Duration::from_millis(50);
/// After the filter goes in, a seek, a resume or a new file: the converter
/// and mpv's estimate settle first.
pub const WARMUP: Duration = Duration::from_secs(2);
/// How long the output is measured before it is judged.
pub const WINDOW: Duration = Duration::from_secs(4);
/// The source rates the converter is offered for (frames per second): up to
/// 30 fps, what was measured (24 and 30 fps at 1080p and 4K). Not offered
/// above (no unmeasured mode): 1080p60 doubled on a high-refresh display
/// dropped 2–3 % of its frames, under the fallback's bar, and 60 → 120 at
/// 1080p dropped 25–41 frames in 10 s on a Radeon 780M (headless).
pub const MIN_FPS: f64 = 10.;
pub const MAX_FPS: f64 = 30.5;
/// "About the display rate": 1 %.
const SLACK: f64 = 0.01;
/// The output is behind below this share of the doubled rate ...
const SLOW: f64 = 0.9;
/// ... or with more dropped frames than this share of it.
const DROPPED: f64 = 0.05;
/// Above this height the converter searches deeper (`profile=super`, AMF's
/// advice for more than 1440p).
const LARGE: i64 = 1440;

/// What a plugin asks for: `MotionMode` in the API.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mode {
    Off,
    /// Twice the frames, shown as they come.
    Double,
    /// Twice the frames, interpolated to the display (GPU rendering).
    Display,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Double => "double",
            Self::Display => "display",
        }
    }
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "off" => Some(Self::Off),
            "double" => Some(Self::Double),
            "display" => Some(Self::Display),
            _ => None,
        }
    }
}

/// Why a mode does not work now: `MotionReason` in the API.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    /// No converter of this engine in this player on this system
    /// (`engine`).
    Engine(Engine),
    /// The system's check failed: `platform`, `no-gpu`, `runtime`
    /// (`platform::MotionRefusal`).
    System { code: &'static str, message: String },
    /// mpv refused the filter (`filter`): this libmpv has no `frc_amf`.
    Filter(String),
    /// GPU rendering is starting (`rendering`; not fixed): Match Display.
    Starting,
    /// The player renders in software (`rendering`): Match Display.
    Rendering(String),
    /// AI upscaling is on (`upscaling`; not fixed).
    Upscaling,
    /// The converter stopped on this video (`failed`; not fixed).
    Failed(String),
    /// The converter's output did not keep up on this video: the GPU, or the
    /// CPU drawing it with software rendering (`overload`; not fixed).
    Overload(String),
    /// The video's rate does not fit (`rate`; not fixed).
    Rate(String),
    /// The video is too large for this mode on this renderer (`size`; not
    /// fixed: another video can fit).
    Size(String),
    /// A plugin's video filter already raises the frame rate (`doubling`:
    /// a `frameBlend` node, Frame Doubling; not fixed).
    Doubling,
    /// The other engine's converter is in the chain for another plugin
    /// (`other-engine`; not fixed: it waits until that one is off).
    OtherEngine(Engine),
    /// The video's format was not measured (`format`: 10-bit or HDR; not
    /// fixed).
    Format(String),
}

/// The reason codes that hold for the whole run: `set` refuses a mode they
/// stand in the way of (`rendering` only once GPU rendering is not just
/// starting). The SDK's TestHost and docs list the same (agreement test).
pub const FIXED_CODES: &[&str] = &[
    "engine",
    "platform",
    "no-gpu",
    "runtime",
    "filter",
    "rendering",
];

impl Reason {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Engine(_) => "engine",
            Self::System { code, .. } => code,
            Self::Filter(_) => "filter",
            Self::Starting | Self::Rendering(_) => "rendering",
            Self::Upscaling => "upscaling",
            Self::Failed(_) => "failed",
            Self::Overload(_) => "overload",
            Self::Rate(_) => "rate",
            Self::Size(_) => "size",
            Self::Doubling => "doubling",
            Self::OtherEngine(_) => "other-engine",
            Self::Format(_) => "format",
        }
    }
    pub fn message(&self) -> String {
        match self {
            Self::Engine(engine) => format!(
                "this player has no {} frame-rate converter on this system",
                engine.vendor()
            ),
            Self::OtherEngine(engine) => format!(
                "{} motion smoothing is on (another plugin); one converter runs at a time, so this waits until it is off",
                engine.vendor()
            ),
            Self::Starting => "Match Display needs GPU rendering, which is starting".into(),
            Self::Upscaling => {
                "AI upscaling is on; motion smoothing and AI upscaling cannot run together".into()
            }
            Self::Doubling => {
                "a video filter already raises the frame rate (frame doubling); motion smoothing and frame doubling cannot run together".into()
            }
            Self::System { message, .. }
            | Self::Filter(message)
            | Self::Rendering(message)
            | Self::Failed(message)
            | Self::Overload(message)
            | Self::Rate(message)
            | Self::Size(message)
            | Self::Format(message) => message.clone(),
        }
    }
    /// Whether it holds for the rest of this run: `set` is then refused
    /// (`unsupported`) instead of waiting for it.
    pub fn fixed(&self) -> bool {
        *self != Self::Starting && FIXED_CODES.contains(&self.code())
    }
    pub fn json(&self) -> Value {
        json!({"code":self.code(),"message":self.message()})
    }
}

/// What [`check`] decides from.
pub struct Facts<'a> {
    /// The engine asked about.
    pub engine: Engine,
    pub probe: &'a MotionProbe,
    /// This player has the engine's filter on this system
    /// (`platform::motion_filter`).
    pub has_filter: bool,
    /// The other engine holds the converter (another plugin asked first).
    pub other: Option<Engine>,
    /// mpv refused the filter in this run (its error).
    pub refused: Option<&'a str>,
    /// GPU rendering draws now, and why it does or not (`Render::now`).
    pub gpu_now: bool,
    pub render_reason: &'a RenderReason,
    /// The host's `rendering.state` when the request came (`Null` inside
    /// the worker): only for the advice in the message.
    pub rendering: &'a Value,
    /// Some plugin has AI upscaling on.
    pub upscaling: bool,
    /// A plugin's video filter raises the frame rate (`frameBlend`).
    pub doubling: bool,
    /// The engine's converter cannot start on this system (AMD's failed the
    /// same way on two videos in this run, NVIDIA's had no Vulkan device;
    /// [`device_failure`]): `runtime` for the rest of the run.
    pub broken: Option<&'a str>,
    /// The video's format when it was not measured (10-bit, HDR), said.
    pub format: Option<String>,
    /// The converter stopped on this video (mpv's log), or did not keep up.
    pub failed: Option<&'a str>,
    pub overload: Option<&'a str>,
    /// The converter's input rate (`Motion::input`: the chain's output
    /// rate while the converter is out of it, so field-rate deinterlacing
    /// counts); `None` while unknown.
    pub source_fps: Option<f64>,
    /// The display's refresh rate (`Render::refresh`).
    pub display_fps: f64,
    /// `VESTRA_MOTION_RATE=any` (a test switch, for measurements): no
    /// display rule.
    pub any_rate: bool,
    /// `VESTRA_MOTION_SIZE=any` (a test switch, for measurements): no size
    /// rule.
    pub any_size: bool,
    /// The video's storage size (`video-params/w`, `h`); `None` while
    /// unknown.
    pub video: Option<(f64, f64)>,
}

/// First failure wins: the system's check, the filter, the renderer (Match
/// Display), AI upscaling, this video's failure, the rate. `Off` always
/// works.
pub fn check(facts: &Facts<'_>, mode: Mode) -> Result<(), Reason> {
    if mode == Mode::Off {
        return Ok(());
    }
    facts.probe.as_ref().map_err(|refusal| Reason::System {
        code: refusal.code,
        message: refusal.message.clone(),
    })?;
    if !facts.has_filter {
        return Err(Reason::Engine(facts.engine));
    }
    if let Some(error) = facts.broken {
        return Err(Reason::System {
            code: "runtime",
            message: match facts.engine {
                Engine::Amd => format!(
                    "AMD's frame-rate converter failed the same way on two videos ({error}); it is not tried again until the player restarts"
                ),
                Engine::Nvidia => format!(
                    "NVIDIA's frame-rate converter cannot start on this system ({error}); it needs the NVIDIA driver's Vulkan support (573.38 or newer) on an RTX 30 or newer GPU, and is not tried again until the player restarts"
                ),
            },
        });
    }
    if let Some(error) = facts.refused {
        return Err(Reason::Filter(format!(
            "this player's video library has no {} frame-rate converter ({}): {error}",
            facts.engine.vendor(),
            facts.engine.converter()
        )));
    }
    if mode == Mode::Display && !facts.gpu_now {
        if *facts.render_reason == RenderReason::Pending {
            return Err(Reason::Starting);
        }
        return Err(Reason::Rendering(format!(
            "Match Display needs GPU rendering, and the player renders in software ({}); {}",
            facts.render_reason.message(),
            crate::playback::upscaling::rendering_advice(facts.render_reason, facts.rendering)
        )));
    }
    if facts.upscaling {
        return Err(Reason::Upscaling);
    }
    if facts.doubling {
        return Err(Reason::Doubling);
    }
    if let Some(other) = facts.other {
        return Err(Reason::OtherEngine(other));
    }
    if let Some(error) = facts.failed {
        return Err(Reason::Failed(format!(
            "the frame-rate converter stopped on this video: {error}"
        )));
    }
    if let Some(message) = facts.overload {
        return Err(Reason::Overload(message.to_owned()));
    }
    rate(facts, mode)?;
    if let Some(format) = &facts.format {
        return Err(Reason::Format(format!(
            "this video is {format}; motion smoothing is offered for 8-bit SDR video, the formats measured"
        )));
    }
    size(facts, mode)
}

/// The most pixels a second the converter's output may carry, by renderer
/// and mode, and how it is said: measured 2026-11-12 (Radeon 780M converting,
/// RTX 4070 Laptop rendering, 60 Hz; doc/architecture/README.md §4.4a). The
/// cost is the copy-back and drawing of the doubled frames, not the
/// converter: software rendering held 1080p30 → 60 and fell behind at
/// 4K24 → 48; GPU rendering held 4K24 → 48 (Double) and fell behind at
/// 4K30 → 60; Match Display held 1080p30 and fell behind at 4K24. 1 % slack.
pub const SOFTWARE_LIMIT: (f64, &str) = (1920. * 1080. * 60. * 1.01, "1080p at 30 fps");
pub const GPU_DOUBLE_LIMIT: (f64, &str) = (3840. * 2160. * 48. * 1.01, "4K at 24 fps");
pub const GPU_DISPLAY_LIMIT: (f64, &str) = (1920. * 1080. * 60. * 1.01, "1080p on a 60 Hz display");
/// NVIDIA's converter: measured 2026-11-22 (RTX 4070 Laptop converting,
/// `fruc_vulkan` `fast`/4 above 1440p; 60 Hz display;
/// doc/architecture/README.md §4.4a). GPU rendering's Double and Match
/// Display held 4K30 → 60 with no frame dropped; software rendering held
/// 4K24 → 48 (1 dropped in 13 s) and was at the edge at 4K30 → 60 (8, 9
/// and 24 dropped in 13 s; the last run fell back to the faster search).
/// One limit for every renderer and mode all the same (user ruling, T8:
/// stronger machines keep up, and the plugin tells the user when frames
/// are dropped). Its frames reach the renderer from the discrete GPU's copy-back, not
/// from an integrated GPU sharing the CPU's memory as AMD's on this laptop,
/// which is where AMD's 4K cases fell behind.
pub const NVIDIA_LIMIT: (f64, &str) = (3840. * 2160. * 60. * 1.01, "4K at 30 fps");

/// The size rule: width × height × the frames a second the renderer then
/// draws (the doubled rate; with Match Display the display's refresh rate,
/// which it interpolates to) within the limit of
/// this renderer and mode ([`SOFTWARE_LIMIT`], [`GPU_DOUBLE_LIMIT`],
/// [`GPU_DISPLAY_LIMIT`]), so a mode is not offered where the measurements
/// show it cannot keep the display rate (the runtime fallback stays the
/// safety net). Unknown size or rate: nothing to judge yet.
fn size(facts: &Facts<'_>, mode: Mode) -> Result<(), Reason> {
    if facts.any_size {
        return Ok(());
    }
    let (Some((width, height)), Some(fps)) = (facts.video, facts.source_fps) else {
        return Ok(());
    };
    let (limit, words, renderer) = match (facts.gpu_now, mode) {
        (false, _) => (SOFTWARE_LIMIT.0, SOFTWARE_LIMIT.1, "software rendering"),
        (true, Mode::Display) => (GPU_DISPLAY_LIMIT.0, GPU_DISPLAY_LIMIT.1, "Match Display"),
        (true, _) => (GPU_DOUBLE_LIMIT.0, GPU_DOUBLE_LIMIT.1, "GPU rendering"),
    };
    let (limit, words) = match facts.engine {
        Engine::Amd => (limit, words),
        Engine::Nvidia => NVIDIA_LIMIT,
    };
    let drawn = if facts.gpu_now && mode == Mode::Display {
        facts.display_fps
    } else {
        fps * 2.
    };
    if width * height * drawn <= limit {
        return Ok(());
    }
    Err(Reason::Size(format!(
        "this video ({}×{} at {} fps) is too large for motion smoothing with {renderer}; it works up to {words}",
        number(width),
        number(height),
        number(fps)
    )))
}

/// The video's storage size (`video-params/w`, `h`), when mpv knows it.
fn video_size(mpv: &Mpv) -> Option<(f64, f64)> {
    let side = |name: &str| {
        mpv.get_property::<i64>(name)
            .ok()
            .filter(|v| *v > 0)
            .map(|v| v as f64)
    };
    Some((side("video-params/w")?, side("video-params/h")?))
}

/// The rate rule: a known source rate within [`MIN_FPS`]..[`MAX_FPS`], its
/// double at most the display's (doubling past what the display shows
/// adds nothing), and with software rendering about the display's (no
/// resampling there, so any other double judders). Unknown: nothing to
/// judge yet.
fn rate(facts: &Facts<'_>, mode: Mode) -> Result<(), Reason> {
    let Some(fps) = facts.source_fps else {
        return Ok(());
    };
    if !(MIN_FPS..=MAX_FPS).contains(&fps) {
        return Err(Reason::Rate(format!(
            "this video plays at {} fps; motion smoothing doubles videos of 10 to 30 fps",
            number(fps)
        )));
    }
    if facts.any_rate {
        return Ok(());
    }
    let (doubled, display) = (fps * 2., facts.display_fps);
    if doubled > display * (1. + SLACK) {
        return Err(Reason::Rate(format!(
            "this video plays at {} fps; doubled it would be {} fps, more than the display's {} Hz",
            number(fps),
            number(doubled),
            number(display)
        )));
    }
    if mode == Mode::Double && !facts.gpu_now && (doubled - display).abs() > display * SLACK {
        return Err(Reason::Rate(format!(
            "with software rendering, doubling works only when twice the video's rate is the display's: {} fps doubled is {} fps on a {} Hz display; GPU rendering offers Match Display",
            number(fps),
            number(doubled),
            number(display)
        )));
    }
    Ok(())
}

/// A rate for people: at most three decimals, no trailing zeros.
fn number(value: f64) -> String {
    let text = format!("{value:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// What the filter in mpv's chain is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Applied {
    engine: Engine,
    mode: Mode,
    /// Above 1440p (AMD: `profile=super`; NVIDIA: `perf=fast`).
    large: bool,
    /// The first fallback step (AMD: `mv_search_mode=performance`; NVIDIA:
    /// `perf=fast`).
    performance: bool,
}

/// What motion smoothing changes besides the chain: the backend's
/// processing (copy-back decoding) and render options (Match Display).
pub trait Effects {
    /// Frames in memory while the filter is in the chain
    /// (`Processing::hold_frames_in_memory`).
    fn frames_in_memory(&mut self, held: bool);
    /// Interpolation to the display at this rate, or not
    /// (`video_render::Options::set_resample`, then the GPU renderer's
    /// pacing).
    fn resample(&mut self, refresh: Option<f64>);
}

/// The output measured since the warm-up ended.
#[derive(Debug, Default)]
struct Watch {
    /// Playing since (a pause, a seek or a new file resets it).
    since: Option<Instant>,
    /// The window's start and mpv's dropped frames then.
    window: Option<(Instant, i64)>,
    /// `estimated-vf-fps` at each look in the window.
    samples: Vec<f64>,
}

/// The control worker's motion smoothing: who asked for what, the system's
/// check, the filter in mpv's chain, the fallback.
pub struct Motion {
    /// Each plugin's engine and mode.
    owners: BTreeMap<u64, (Engine, Mode)>,
    /// The engine that holds the converter: the first asked for, while some
    /// plugin asks for it ([`Motion::holder`]).
    holder: Option<Engine>,
    /// Each engine's system check, run when a plugin first asks for it.
    probe: PerEngine<Option<MotionProbe>>,
    run_probe: PerEngine<fn() -> MotionProbe>,
    /// The filter (`platform::motion_filter`; `None` where the platform has
    /// none). Tests put in a stand-in every mpv build accepts.
    filter: fn(Engine, bool, bool) -> Option<String>,
    applied: Option<Applied>,
    /// mpv refused the engine's filter: for the rest of the run.
    refused: PerEngine<Option<String>>,
    /// The engine whose filter mpv refused last: its log line that follows
    /// may say the converter cannot start here (`Motion::log`).
    refusing: Option<Engine>,
    /// mpv disabled the filter on this video (its log), with the
    /// converter's last error.
    failed: Option<String>,
    /// mpv said it disabled the filter: handled at the next look.
    disabled: bool,
    /// The converter's (or the filter graph's) last error line.
    last_error: Option<String>,
    /// The output did not keep up on this video ([`overload_message`]).
    overload: Option<String>,
    /// This video's fallback step: the motion search on downscaled frames.
    performance: bool,
    /// The source the per-video state belongs to.
    generation: Option<u64>,
    /// The engine the per-video state (`failed`, `overload`,
    /// `performance`) belongs to: the last that held the converter.
    video_engine: Option<Engine>,
    watch: Watch,
    checked: Option<Instant>,
    /// Some plugin has AI upscaling on (`Upscaling::asked`).
    upscaling: bool,
    /// The last `video.motion.changed` sent, per engine.
    published: PerEngine<Option<Value>>,
    /// `VESTRA_MOTION_RATE=any` (see [`Facts::any_rate`]).
    any_rate: bool,
    /// `VESTRA_MOTION_SIZE=any` (see [`Facts::any_size`]).
    any_size: bool,
    /// Frames in memory are held for the filter (`Effects::frames_in_memory`).
    holding: bool,
    /// Waiting for the copy-back decoder (said once in the log).
    waiting: bool,
    /// Match Display's resampling asked of the render options, at this
    /// refresh (`Effects::resample`).
    resampling: Option<f64>,
    /// Some plugin's video filter raises the frame rate
    /// (`Processing::doubles_frames`).
    doubling: bool,
    /// The converter's input rate on this video: `estimated-vf-fps` read
    /// while the converter is out of the chain (after every other filter
    /// and mpv's own deinterlacer, so 25i deinterlaced by field reads 50),
    /// kept while it is in (then mpv's estimate is the doubled rate).
    input: Option<f64>,
    /// The converters' failures in this run, by engine and error, and the
    /// one that means the engine cannot start here (`runtime` for the rest
    /// of the run; [`device_failure`]).
    failures: BTreeMap<(Engine, String), u32>,
    broken: PerEngine<Option<String>>,
    /// The renderer's publish gate (`follow_gate`; none: always open).
    gate: Option<Arc<AtomicBool>>,
    /// The gate as the last look saw it: a change is looked at at once.
    gate_seen: bool,
}

impl Default for Motion {
    fn default() -> Self {
        Self::with_probes(
            || platform::motion_probe(Engine::Amd),
            || platform::motion_probe(Engine::Nvidia),
        )
    }
}

impl Motion {
    /// With another system check, the same for both engines (tests).
    #[cfg(test)]
    pub fn with_probe(run_probe: fn() -> MotionProbe) -> Self {
        Self::with_probes(run_probe, run_probe)
    }

    /// With other system checks (AMD's, NVIDIA's).
    pub fn with_probes(amd: fn() -> MotionProbe, nvidia: fn() -> MotionProbe) -> Self {
        Self {
            owners: BTreeMap::new(),
            holder: None,
            probe: PerEngine::default(),
            run_probe: PerEngine([amd, nvidia]),
            filter: platform::motion_filter,
            applied: None,
            refused: PerEngine::default(),
            refusing: None,
            failed: None,
            disabled: false,
            last_error: None,
            overload: None,
            performance: false,
            generation: None,
            video_engine: None,
            watch: Watch::default(),
            checked: None,
            upscaling: false,
            published: PerEngine::default(),
            any_rate: std::env::var("VESTRA_MOTION_RATE").is_ok_and(|v| v.trim() == "any"),
            any_size: std::env::var("VESTRA_MOTION_SIZE").is_ok_and(|v| v.trim() == "any"),
            holding: false,
            waiting: false,
            resampling: None,
            input: None,
            doubling: false,
            failures: BTreeMap::new(),
            broken: PerEngine::default(),
            gate: None,
            gate_seen: true,
        }
    }

    /// Follow the renderer's publish gate, as upscaling does: while a new
    /// source is being opened (the gate closed) the converter is not in the
    /// chain, and it goes back in once the gate opens on the new source's
    /// confirmed geometry.
    pub fn follow_gate(&mut self, gate: Arc<AtomicBool>) {
        self.gate = Some(gate);
    }

    fn gate_open(&self) -> bool {
        self.gate
            .as_ref()
            .is_none_or(|gate| gate.load(Ordering::Acquire))
    }

    /// Whether some plugin's video filter raises the frame rate
    /// (Frame Doubling), read before each request and look.
    pub fn note_doubling(&mut self, doubling: bool) {
        self.doubling = doubling;
    }

    /// Every worker iteration, after AI upscaling took its system's check
    /// (`Upscaling::take_check`) and before it answers the requests that
    /// waited for it (`Upscaling::poll`): a waiting `set(true)` that will
    /// be accepted makes the converter step aside first
    /// ([`Motion::yield_to_upscaling`]), as a request after the check does.
    /// One that will be refused (no NVIDIA GPU) leaves it in the chain.
    pub fn hand_over(
        &mut self,
        upscaling: &crate::playback::upscaling::Upscaling,
        mpv: &Mpv,
        render: &Render,
        effects: &mut impl Effects,
    ) {
        if upscaling.accepts_waiting(mpv, render) {
            self.yield_to_upscaling(mpv, render, effects);
        }
    }

    /// AI upscaling is about to be turned on: the converter leaves the chain
    /// and releases copy-back now, before upscaling answers, so its state
    /// does not name a copy-back decoding that is about to end.
    pub fn yield_to_upscaling(&mut self, mpv: &Mpv, render: &Render, effects: &mut impl Effects) {
        if self.upscaling || (self.applied.is_none() && !self.holding) {
            return;
        }
        self.upscaling = true;
        self.update(mpv, render, effects, Instant::now());
    }

    /// Whether some plugin has AI upscaling on, read before each request
    /// and look.
    pub fn note_upscaling(&mut self, asked: bool) {
        self.upscaling = asked;
    }

    /// A worker command: the two requests (`true`), and a stopped plugin's
    /// release (`false`: the worker's own handling goes on).
    pub fn command(
        &mut self,
        mpv: &Mpv,
        render: &Render,
        command: &Command,
        events: &mpsc::Sender<Event>,
        effects: &mut impl Effects,
    ) -> bool {
        let rendering = &command.params["rendering"];
        let owner = command.reply.owner;
        match command.method.as_str() {
            "owner.disabled" => {
                self.release(mpv, render, owner, effects);
                self.publish(mpv, render, events);
                false
            }
            "video.motion.state" | "video.motion.set"
                if command.params["engine"]
                    .as_str()
                    .and_then(Engine::from_name)
                    .is_none() =>
            {
                let result = Err(RpcError::invalid(r#"engine must be "amd" or "nvidia""#));
                let _ = events.send(Event::Reply(command.reply.clone(), result));
                true
            }
            "video.motion.state" => {
                let engine = engine_of(&command.params);
                let state = self.state(mpv, render, owner, engine, rendering);
                let _ = events.send(Event::Reply(command.reply.clone(), Ok(state)));
                self.publish(mpv, render, events);
                true
            }
            "video.motion.set" => {
                let result = self.set(mpv, render, command, effects);
                let _ = events.send(Event::Reply(command.reply.clone(), result));
                self.publish(mpv, render, events);
                true
            }
            _ => false,
        }
    }

    /// `video.motion.set(mode)`: off at once; a mode refused (`unsupported`)
    /// where it cannot work in this run, else kept and applied as soon as
    /// it can work.
    fn set(
        &mut self,
        mpv: &Mpv,
        render: &Render,
        command: &Command,
        effects: &mut impl Effects,
    ) -> Result<Value, RpcError> {
        let (owner, rendering) = (command.reply.owner, &command.params["rendering"]);
        let engine = engine_of(&command.params);
        let Some(mode) = command.params["mode"].as_str().and_then(Mode::parse) else {
            return Err(RpcError::invalid(
                "mode must be \"off\", \"double\" or \"display\"",
            ));
        };
        if mode == Mode::Off {
            // Only this engine's request: a plugin asking the other engine
            // for off leaves its own.
            if self.owners.get(&owner).is_some_and(|(e, _)| *e == engine) {
                self.owners.remove(&owner);
            }
        } else {
            if let Err(reason) = self.availability(mpv, render, rendering, engine, mode)
                && reason.fixed()
            {
                return Err(RpcError::new(
                    "unsupported",
                    format!("motion smoothing cannot work here: {}", reason.message()),
                ));
            }
            self.owners.insert(owner, (engine, mode));
        }
        self.update(mpv, render, effects, Instant::now());
        Ok(self.state(mpv, render, owner, engine, rendering))
    }

    /// Every worker iteration: a new source starts this video's state
    /// afresh; while motion smoothing is asked for, a look every
    /// [`CHECK_EVERY`] (the rate, the output, mpv's verdict on the filter).
    pub fn poll(
        &mut self,
        mpv: &Mpv,
        render: &Render,
        events: &mpsc::Sender<Event>,
        effects: &mut impl Effects,
        generation: u64,
    ) {
        if self.generation != Some(generation) {
            self.generation = Some(generation);
            self.new_video();
        }
        let now = Instant::now();
        if self.due_in(now).is_some_and(|due| due.is_zero()) {
            self.update(mpv, render, effects, now);
            self.publish(mpv, render, events);
        }
    }

    /// Another source: its failure, fallback step and measurement start
    /// afresh.
    fn new_video(&mut self) {
        self.input = None;
        self.failed = None;
        self.overload = None;
        self.performance = false;
        self.disabled = false;
        self.last_error = None;
        self.watch = Watch::default();
        self.checked = None;
    }

    /// Another engine holds the converter: this video's failure and
    /// fallback step were the other one's.
    fn new_engine(&mut self) {
        self.failed = None;
        self.overload = None;
        self.performance = false;
        self.disabled = false;
        self.last_error = None;
        self.watch = Watch::default();
    }

    /// How long the control worker may wait before [`Motion::poll`] has
    /// work: `None` while nobody asked and no filter is in the chain.
    pub fn due_in(&self, now: Instant) -> Option<Duration> {
        if self.owners.is_empty()
            && self.applied.is_none()
            && !self.holding
            && self.resampling.is_none()
        {
            return None;
        }
        if self.gate_open() != self.gate_seen {
            return Some(Duration::ZERO);
        }
        Some(self.checked.map_or(Duration::ZERO, |at| {
            CHECK_EVERY.saturating_sub(now.saturating_duration_since(at))
        }))
    }

    /// One of mpv's log lines (every line the worker gets): while the
    /// filter is in the chain, mpv disabling it (`Disabling filter
    /// vestra-motion because it has failed.`) and the converter's error
    /// before it (`AMFFRC->Init() failed with error 4`). Cheap for every
    /// other line.
    pub fn log(&mut self, prefix: &str, level: &str, text: &str) {
        // NVIDIA's converter creates its Vulkan device when the filter is
        // created, so a device that cannot be had makes mpv refuse the
        // `vf add`; its reason comes after, in this line: `runtime`
        // rather than `filter` (this player has the converter).
        if let Some(engine) = self.refusing
            && matches!(level, "error" | "fatal")
            && device_failure(engine, text, 1)
        {
            let line = text.trim().to_owned();
            diag!(
                "motion: the {} converter cannot start here: {line}",
                engine.as_str()
            );
            self.refusing = None;
            self.refused[engine] = None;
            self.broken[engine] = Some(line);
            self.checked = None;
            return;
        }
        if self.applied.is_none() {
            return;
        }
        if prefix == "vf" && text.contains("Disabling filter") && text.contains(LABEL) {
            self.disabled = true;
            // Looked at at once.
            self.checked = None;
        } else if matches!(level, "error" | "fatal" | "warn")
            && (prefix.starts_with("ffmpeg") || prefix == "lavfi")
            && !ffmpeg_noise(text)
        {
            // The converter's own line comes before the graph's summary:
            // keep both (the last three).
            let line = text.trim();
            diag!("motion: libmpv {prefix}: {line}");
            let mut lines: Vec<&str> = self
                .last_error
                .as_deref()
                .map_or_else(Vec::new, |e| e.split("; ").collect());
            lines.push(line);
            let keep = lines.len().saturating_sub(3);
            let joined = lines[keep..].join("; ");
            self.last_error = Some(joined);
        }
    }

    /// `owner` asked for motion smoothing: a live resource, so lazy
    /// activation never parks it.
    pub fn holds(&self, owner: u64) -> bool {
        self.owners.contains_key(&owner)
    }

    /// The plugin stopped: its request goes, and with the last one the
    /// filter.
    pub fn release(&mut self, mpv: &Mpv, render: &Render, owner: u64, effects: &mut impl Effects) {
        if self.owners.remove(&owner).is_some() {
            self.update(mpv, render, effects, Instant::now());
        }
    }

    /// The engine that holds the converter: the one it was held by, while
    /// some plugin asks for it; else the engine of the first request (by
    /// plugin). A change of engine starts the per-video state afresh (the
    /// failure, the fallback step: they were the other converter's).
    fn refresh_holder(&mut self) {
        let asked = |engine: Engine| {
            self.owners
                .values()
                .any(|(e, mode)| *e == engine && *mode != Mode::Off)
        };
        if self.holder.is_some_and(asked) {
            return;
        }
        let next = self
            .owners
            .values()
            .find(|(_, mode)| *mode != Mode::Off)
            .map(|(engine, _)| *engine);
        if next.is_some() && next != self.video_engine {
            if self.video_engine.is_some() {
                diag!(
                    "motion: the {} engine now holds the converter",
                    next.map_or("", Engine::as_str)
                );
                self.new_engine();
            }
            self.video_engine = next;
        }
        self.holder = next;
    }

    /// The strongest mode some plugin asked of the engine that holds the
    /// converter.
    fn wanted(&self) -> Mode {
        let Some(holder) = self.holder else {
            return Mode::Off;
        };
        self.owners
            .values()
            .filter(|(engine, _)| *engine == holder)
            .map(|(_, mode)| *mode)
            .max()
            .unwrap_or(Mode::Off)
    }

    /// [`check`] with what the worker sees now; the engine's system check
    /// runs the first time.
    fn availability(
        &mut self,
        mpv: &Mpv,
        render: &Render,
        rendering: &Value,
        engine: Engine,
        mode: Mode,
    ) -> Result<(), Reason> {
        let run = self.run_probe[engine];
        self.probe[engine].get_or_insert_with(run);
        self.check_now(mpv, render, rendering, engine, mode)
    }

    /// [`check`] without starting the system's check (`platform` until it
    /// ran is never said: every caller ran it).
    fn check_now(
        &self,
        mpv: &Mpv,
        render: &Render,
        rendering: &Value,
        engine: Engine,
        mode: Mode,
    ) -> Result<(), Reason> {
        let Some(probe) = &self.probe[engine] else {
            return Ok(());
        };
        let (gpu_now, render_reason) = render.now();
        // This video's failure and fallback are the converter's that ran on
        // it.
        let this_video = self.video_engine.is_none_or(|e| e == engine);
        check(
            &Facts {
                engine,
                probe,
                has_filter: (self.filter)(engine, false, false).is_some(),
                other: self.holder.filter(|holder| *holder != engine),
                refused: self.refused[engine].as_deref(),
                gpu_now: gpu_now && render.gpu().is_some(),
                render_reason: &render_reason,
                rendering,
                upscaling: self.upscaling,
                doubling: self.doubling,
                broken: self.broken[engine].as_deref(),
                format: unmeasured_format(mpv),
                failed: self.failed.as_deref().filter(|_| this_video),
                overload: self.overload.as_deref().filter(|_| this_video),
                source_fps: self.rate_in(mpv),
                display_fps: render.refresh(),
                any_rate: self.any_rate,
                any_size: self.any_size,
                video: video_size(mpv),
            },
            mode,
        )
    }

    /// The `MotionState` of `engine` for `owner`.
    fn state(
        &mut self,
        mpv: &Mpv,
        render: &Render,
        owner: u64,
        engine: Engine,
        rendering: &Value,
    ) -> Value {
        let run = self.run_probe[engine];
        self.probe[engine].get_or_insert_with(run);
        let mut state = self.shared_state(mpv, render, engine, rendering);
        state["mode"] = self
            .owners
            .get(&owner)
            .filter(|(e, _)| *e == engine)
            .map_or(Mode::Off, |(_, mode)| *mode)
            .as_str()
            .into();
        state
    }

    /// The `MotionState` of `engine` without `mode` (each plugin's own):
    /// what `video.motion.changed` carries.
    fn shared_state(&self, mpv: &Mpv, render: &Render, engine: Engine, rendering: &Value) -> Value {
        let reason = |mode| {
            self.check_now(mpv, render, rendering, engine, mode)
                .err()
                .map(|reason| reason.json())
        };
        let round = |value: f64, places: i32| {
            let scale = 10f64.powi(places);
            (value * scale).round() / scale
        };
        // What is in the chain is this engine's only while it holds it.
        let applied = self.applied.filter(|applied| applied.engine == engine);
        let in_chain = applied.is_some() && labels(mpv).iter().any(|label| label == LABEL);
        json!({
            "engine": engine.as_str(),
            "active": applied.map_or("off", |applied| applied.mode.as_str()),
            "double": reason(Mode::Double),
            "display": reason(Mode::Display),
            "searchMode": applied.map(|applied| if applied.performance { "performance" } else { "native" }),
            "sourceFps": self.input.or_else(|| source_fps(mpv)).map(|fps| round(fps, 3)),
            "outputFps": applied
                .and_then(|_| mpv.get_property::<f64>("estimated-vf-fps").ok())
                .filter(|fps| fps.is_finite() && *fps > 0.)
                .map(|fps| round(fps, 1)),
            "displayFps": round(render.refresh(), 3),
            "gpu": self.probe[engine].as_ref().and_then(|probe| probe.as_ref().ok()).map(|gpu| gpu.name.clone()),
            "inChain": in_chain,
        })
    }

    /// `video.motion.changed` for each engine whose shared state differs
    /// from the last one sent. Nothing before the engine's system check
    /// ran.
    fn publish(&mut self, mpv: &Mpv, render: &Render, events: &mpsc::Sender<Event>) {
        for engine in Engine::ALL {
            if self.probe[engine].is_none() {
                continue;
            }
            let state = self.shared_state(mpv, render, engine, &Value::Null);
            if self.published[engine].as_ref() == Some(&state) {
                continue;
            }
            self.published[engine] = Some(state.clone());
            let _ = events.send(Event::Broadcast(CHANGED.into(), state));
        }
    }

    /// The converter's input rate (`input`), read while it is out of the
    /// chain; kept while it is in, unless half the doubled estimate rises
    /// more than `RERATE` above it (mpv's deinterlacer starting on a later
    /// interlaced frame, a plugin raising the rate): then that half is the
    /// input and the rules judge it again. A fall is left to the fallback,
    /// which tells a slower input from a converter that cannot keep up.
    fn measure_input(&mut self, mpv: &Mpv) {
        if self.applied.is_some() || labels(mpv).iter().any(|label| label == LABEL) {
            if let (Some(input), Ok(fps)) =
                (self.input, mpv.get_property::<f64>("estimated-vf-fps"))
                && fps.is_finite()
                && fps / 2. > input * RERATE
            {
                diag!(
                    "motion: the input rose from {input:.2} to {:.2} fps",
                    fps / 2.
                );
                self.input = Some(fps / 2.);
            }
            return;
        }
        if let Ok(fps) = mpv.get_property::<f64>("estimated-vf-fps")
            && fps.is_finite()
            && fps > 0.
        {
            self.input = Some(fps);
        }
    }

    /// The rate the rules judge: the measured input, or, while the converter
    /// is in the chain before one was measured (it never is measured then),
    /// the container's rate, so a filter in the chain is never left unlooked.
    fn rate_in(&self, mpv: &Mpv) -> Option<f64> {
        self.input
            .or_else(|| self.applied.and_then(|_| source_fps(mpv)))
    }

    /// The converter's input rate on this video, while known: what
    /// `player.stats().video.fps` reports while the converter is in the chain
    /// (`stats_fps`).
    pub fn input_fps(&self) -> Option<f64> {
        self.input
    }

    /// Bring mpv's chain to what is wanted now, and judge the output.
    fn update(&mut self, mpv: &Mpv, render: &Render, effects: &mut impl Effects, now: Instant) {
        self.checked = Some(now);
        self.refresh_holder();
        let open = self.gate_open();
        self.gate_seen = open;
        if !open {
            // The new source's rate is measured once it is confirmed.
            self.input = None;
            // Nothing asked any more (Off, or the plugin stopped): all of
            // it goes now, not when the gate opens.
            if self.wanted() == Mode::Off {
                self.remove(mpv, effects);
                return;
            }
            // Out of the chain while the new source opens; copy-back
            // decoding and Match Display stay, so nothing else switches.
            self.watch = Watch::default();
            if self.applied.take().is_some() {
                diag!("motion: out of the chain while the new source opens");
                let _ = mpv.command(&["vf", "remove", &format!("@{LABEL}")]);
            }
            return;
        }
        self.measure_input(mpv);
        if self.disabled {
            self.disabled = false;
            let error = self
                .last_error
                .take()
                .unwrap_or_else(|| "mpv disabled the filter".into());
            diag!("motion: mpv disabled the filter: {error}");
            // A converter that cannot start on this system (AMF failing the
            // same way on two videos, an old AMD GPU; NVIDIA's without a
            // Vulkan device or its optical flow at once): not tried again in
            // this run, each try costing two decoder reopens. Other
            // failures stay per video.
            if let Some(engine) = self.applied.map(|a| a.engine).or(self.holder) {
                let count = self.failures.entry((engine, error.clone())).or_insert(0);
                *count += 1;
                if device_failure(engine, &error, *count) {
                    diag!(
                        "motion: the {} converter cannot start here; off for this run",
                        engine.as_str()
                    );
                    self.broken[engine] = Some(error.clone());
                }
            }
            self.failed = Some(error);
            self.remove(mpv, effects);
            return;
        }
        let wanted = self.wanted();
        let Some(engine) = self.holder.filter(|_| wanted != Mode::Off) else {
            self.remove(mpv, effects);
            return;
        };
        if let Err(reason) = self.availability(mpv, render, &Value::Null, engine, wanted) {
            // Match Display that cannot work falls back to Double when that
            // can (software rendering after a GPU fallback).
            let double = wanted == Mode::Display
                && reason.code() == "rendering"
                && self
                    .check_now(mpv, render, &Value::Null, engine, Mode::Double)
                    .is_ok();
            if !double {
                self.remove(mpv, effects);
                return;
            }
            return self.apply(mpv, render, effects, engine, Mode::Double, now);
        }
        self.apply(mpv, render, effects, engine, wanted, now);
    }

    /// `mode` can work: the filter in the chain (unless the rate is still
    /// unknown: then what is there stays), Match Display's resampling, the
    /// output judged.
    fn apply(
        &mut self,
        mpv: &Mpv,
        render: &Render,
        effects: &mut impl Effects,
        engine: Engine,
        mode: Mode,
        now: Instant,
    ) {
        // The converter's input rate is measured first (`measure_input`):
        // until then nothing is judged and nothing goes in.
        if self.rate_in(mpv).is_none() {
            return;
        }
        let large = mpv
            .get_property::<i64>("video-params/h")
            .ok()
            .map_or_else(|| self.applied.is_some_and(|a| a.large), |h| h > LARGE);
        let next = Applied {
            engine,
            mode,
            large,
            performance: self.performance,
        };
        // Another engine's filter is replaced where it stands (`vf add`
        // with the same label), its converter freed with it.
        // A step that leaves the filter's text as it is (NVIDIA above 1440p
        // has no faster setting) does not rebuild it.
        let text = |a: Applied| (self.filter)(a.engine, a.large, a.performance);
        let filter_differs = self.applied.is_none_or(|a| text(a) != text(next));
        if filter_differs {
            // Copy-back first, and the filter only once the decoder hands
            // over frames in memory: the switch reopens the decoder on mpv's
            // own thread, and a Direct3D 11 frame reaching the converter
            // before it fails the graph (measured in the player: "failed to
            // configure the filter graph" right after the switch). Looked at
            // again at the next turn.
            self.hold(effects, true);
            if self.applied.is_none() && !frames_ready(mpv) {
                // Again in 50 ms (not at once: the worker would spin).
                self.checked = now.checked_sub(CHECK_EVERY - READY_EVERY);
                if !self.waiting {
                    self.waiting = true;
                    diag!(
                        "motion: waiting for copy-back decoding (hwdec-current={})",
                        mpv.get_property::<String>("hwdec-current")
                            .unwrap_or_default()
                    );
                }
                return;
            }
            self.waiting = false;
            if !self.add(mpv, effects, next) {
                return;
            }
        } else {
            self.applied = Some(next);
            self.keep_last(mpv, effects, next);
        }
        self.resample(effects, (mode == Mode::Display).then(|| render.refresh()));
        if let Some(behind) = self.watch(mpv, now) {
            self.watch = Watch::default();
            if self.performance {
                diag!(
                    "motion: still behind with the faster motion search ({behind}); off for this video"
                );
                let gpu_rendering = render.now().0 && render.gpu().is_some();
                self.overload = Some(overload_message(gpu_rendering, &behind));
                self.remove(mpv, effects);
            } else {
                diag!("motion: behind ({behind}); the faster motion search");
                self.performance = true;
                // Rebuilt with it at the next look.
                self.checked = None;
            }
        }
    }

    /// The output since the warm-up: `Some(what)` when a full window was
    /// behind the doubled rate or dropped too many frames.
    fn watch(&mut self, mpv: &Mpv, now: Instant) -> Option<String> {
        let flag = |name: &str| mpv.get_property::<bool>(name).unwrap_or(false);
        let playing = !flag("core-idle") && !flag("seeking") && !flag("pause");
        let fps = self.rate_in(mpv);
        let (true, Some(fps)) = (playing, fps) else {
            self.watch = Watch::default();
            return None;
        };
        let dropped = mpv.get_property::<i64>("frame-drop-count").unwrap_or(0)
            + mpv
                .get_property::<i64>("decoder-frame-drop-count")
                .unwrap_or(0);
        let since = *self.watch.since.get_or_insert(now);
        if now.saturating_duration_since(since) < WARMUP {
            return None;
        }
        let Some((start, base)) = self.watch.window else {
            self.watch.window = Some((now, dropped));
            self.watch.samples.clear();
            return None;
        };
        if let Ok(output) = mpv.get_property::<f64>("estimated-vf-fps")
            && output.is_finite()
        {
            self.watch.samples.push(output);
        }
        let seconds = now.saturating_duration_since(start).as_secs_f64();
        if seconds < WINDOW.as_secs_f64() {
            return None;
        }
        self.watch.window = Some((now, dropped));
        let samples = std::mem::take(&mut self.watch.samples);
        judge(fps * 2., &samples, (dropped - base).max(0), seconds)
    }

    /// Put the filter in the chain: in place when it is there (`vf add`
    /// replaces a label where it stands), else last, with frames in memory
    /// first. `false`: mpv refused it (for the rest of the run).
    fn add(&mut self, mpv: &Mpv, effects: &mut impl Effects, next: Applied) -> bool {
        let Some(filter) = (self.filter)(next.engine, next.large, next.performance) else {
            return false;
        };
        self.watch = Watch::default();
        platform::prepare_motion(next.engine);
        match mpv.command(&["vf", "add", &format!("@{LABEL}:{filter}")]) {
            Ok(()) => {
                diag!("motion: {filter} ({})", next.mode.as_str());
                self.applied = Some(next);
                self.move_last(mpv, &filter);
                true
            }
            Err(error) => {
                diag!("motion: mpv refused {filter}: {error}");
                self.refused[next.engine] = Some(error.to_string());
                self.refusing = Some(next.engine);
                self.remove(mpv, effects);
                false
            }
        }
    }

    /// Take the filter out of the chain, then Match Display's resampling
    /// and the frames in memory.
    fn remove(&mut self, mpv: &Mpv, effects: &mut impl Effects) {
        self.watch = Watch::default();
        if self.applied.take().is_some() {
            match mpv.command(&["vf", "remove", &format!("@{LABEL}")]) {
                Ok(()) => diag!("motion: off"),
                Err(error) => diag!("motion: removing the filter failed: {error}"),
            }
        }
        // Whatever left the chain before (the gate, a failed add): Match
        // Display's resampling and the frames in memory end here too.
        self.resample(effects, None);
        self.hold(effects, false);
    }

    /// Match Display's resampling at `refresh`, or none.
    fn resample(&mut self, effects: &mut impl Effects, refresh: Option<f64>) {
        if self.resampling != refresh {
            self.resampling = refresh;
            effects.resample(refresh);
        }
    }

    /// Frames in memory for the filter, or back to the decoding before.
    fn hold(&mut self, effects: &mut impl Effects, held: bool) {
        if self.holding != held {
            self.holding = held;
            effects.frames_in_memory(held);
        }
    }

    /// The filter stays last (a plugin's first video chain added after it
    /// would get twice the frames): moved back when another filter came
    /// after it, put back when someone removed it.
    fn keep_last(&mut self, mpv: &Mpv, effects: &mut impl Effects, applied: Applied) {
        if !labels(mpv).iter().any(|label| label == LABEL) {
            diag!("motion: the filter left the chain; putting it back");
            self.applied = None;
            self.add(mpv, effects, applied);
            return;
        }
        if let Some(filter) = (self.filter)(applied.engine, applied.large, applied.performance) {
            self.move_last(mpv, &filter);
        }
    }

    /// Move the filter to the end of the chain when another filter follows
    /// it. Only the upscaler may (the two exclude each other, so that never
    /// lasts).
    fn move_last(&self, mpv: &Mpv, filter: &str) {
        let labels = labels(mpv);
        let Some(at) = labels.iter().position(|label| label == LABEL) else {
            return;
        };
        if labels[at + 1..]
            .iter()
            .all(|label| label == crate::playback::upscaling::LABEL)
        {
            return;
        }
        diag!("motion: the filter is not last in {labels:?}; moving it");
        let _ = mpv.command(&["vf", "remove", &format!("@{LABEL}")]);
        if let Err(error) = mpv.command(&["vf", "add", &format!("@{LABEL}:{filter}")]) {
            diag!("motion: moving the filter failed: {error}");
        }
    }
}

/// FFmpeg's own complaint about a log call without a context, which
/// FFmpeg 9.1's `fruc_vulkan` triggers with a verbose line at every start
/// (its context's class is not set): not the converter's error, so it is
/// neither logged nor kept as one.
fn ffmpeg_noise(text: &str) -> bool {
    text.contains("av_log callback called with bad parameters")
        || text.contains("This is a bug in one of FFmpeg libraries used")
}

/// The engine a request names (checked before: `amd` or `nvidia`).
fn engine_of(params: &Value) -> Engine {
    params["engine"]
        .as_str()
        .and_then(Engine::from_name)
        .unwrap_or(Engine::Amd)
}

/// The verdict on one window: the mean output rate below [`SLOW`] of the
/// target, or more dropped frames than [`DROPPED`] of the target's frames.
pub fn judge(target: f64, samples: &[f64], dropped: i64, seconds: f64) -> Option<String> {
    if samples.is_empty() || seconds <= 0. {
        return None;
    }
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    let allowed = DROPPED * target * seconds;
    let slow = mean < SLOW * target;
    let dropping = dropped as f64 > allowed;
    (slow || dropping).then(|| {
        format!(
            "{} of {} frames a second, {dropped} frames dropped in {} s",
            number((mean * 10.).round() / 10.),
            number(target),
            number(seconds.round())
        )
    })
}

/// Whether the decoder hands over frames in memory: mpv's `hwdec` is a
/// copy-back mode (or `no`), and the decoder in use, if any, is one too.
/// `hwdec-current` keeps the old decoder until the switch reopened it.
fn frames_ready(mpv: &Mpv) -> bool {
    let in_memory = |mode: &str| mode == "no" || mode.contains("copy");
    let text = |name: &str| mpv.get_property::<String>(name).unwrap_or_default();
    let option = text("hwdec");
    let current = text("hwdec-current");
    // While a video plays, its decoder must already be a copy-back one: an
    // empty `hwdec-current` then means the decoder is being reopened.
    let video = mpv.get_property::<i64>("video-params/w").is_ok();
    option.split(',').map(str::trim).all(in_memory)
        && if video {
            !current.is_empty() && in_memory(&current)
        } else {
            current.is_empty() || in_memory(&current)
        }
}

/// The video's format when it is one motion smoothing was not measured on:
/// 10-bit (or deeper) or HDR (PQ, HLG), said for the reason; `None` for
/// 8-bit SDR video or while unknown.
fn unmeasured_format(mpv: &Mpv) -> Option<String> {
    let text = |name: &str| mpv.get_property::<String>(name).unwrap_or_default();
    let hdr = matches!(text("video-params/gamma").as_str(), "pq" | "hlg");
    let depth = stream_depth(
        &text("video-params/pixelformat"),
        &text("video-params/hw-pixelformat"),
    );
    match (depth > 8, hdr) {
        (_, true) => Some("HDR".into()),
        (true, false) => Some(format!("{depth}-bit")),
        _ => None,
    }
}

/// The video's bits per component whichever way it is decoded: under
/// hardware decoding `pixelformat` names the surface (`d3d11`,
/// `dxva2_vld`) and `hw-pixelformat` the format in it (`p010`); with
/// copy-back or software decoding `pixelformat` is the format and
/// `hw-pixelformat` is empty. The same answer either way, or motion
/// smoothing would turn copy-back on for a 10-bit video (native: 8-bit
/// read), then off again (copy-back: 10-bit read), every look.
pub fn stream_depth(pixelformat: &str, hw_pixelformat: &str) -> u32 {
    if hw_pixelformat.is_empty() {
        bit_depth(pixelformat)
    } else {
        bit_depth(hw_pixelformat)
    }
}

/// The bits per component an FFmpeg/mpv pixel format name says: the number
/// after its last `p` (`yuv420p10`, `p010`, `gbrp12le`), else 8 (`nv12`,
/// `yuv420p`, `rgb24`).
pub fn bit_depth(format: &str) -> u32 {
    let name = format.trim_end_matches("le").trim_end_matches("be");
    name.rfind('p')
        .and_then(|at| name[at + 1..].parse::<u32>().ok())
        .unwrap_or(8)
}

/// The `overload` message: what could not keep up with the doubled frames.
/// With software rendering that is the player's CPU copying and drawing
/// them, not the converter (measured 2026-11-23, NVIDIA 4K30 → 60: 300–500
/// frames dropped in 20 s with the GPU 25–40 % busy), so it names that and
/// what helps; with GPU rendering it is the GPU. The same for either engine.
pub fn overload_message(gpu_rendering: bool, behind: &str) -> String {
    if gpu_rendering {
        format!(
            "the GPU could not keep up with this video, even with the faster motion search: {behind}"
        )
    } else {
        format!(
            "rendering on the CPU could not keep up with this video's doubled frames ({behind}); GPU Rendering (Picture ▸ Rendering) helps"
        )
    }
}

/// Whether a converter failure means the engine cannot start on this
/// system, so it becomes `runtime` for the run: AMF failing to start
/// (`AMFFRC->Init() failed with error N`), the one that repeats on a GPU the
/// runtime does not serve, the second time; NVIDIA's converter without a
/// Vulkan device (no Vulkan driver, lavfi's `could not create hwdevice`) or
/// without the optical flow it needs (`fruc_vulkan`'s device checks), the
/// first time: neither depends on the video.
///
/// Why one failure is enough for NVIDIA: the device is created by FFmpeg's
/// `av_hwdevice_ctx_create` from the Vulkan loader, with the loader scoped
/// to NVIDIA's driver (`platform::prepare_motion`) and on the adapter the
/// system check chose. Nothing about the video goes into it, so a second try
/// on the next video meets the same driver, loader and adapter, and fails
/// the same way (a missing or broken Vulkan driver, a driver selection that
/// leaves NVIDIA's out, a GPU without optical flow). After a driver is
/// installed or updated, a restart of the player tries again.
pub fn device_failure(engine: Engine, error: &str, count: u32) -> bool {
    match engine {
        Engine::Amd => count >= 2 && error.contains("AMFFRC->Init() failed"),
        Engine::Nvidia => [
            "could not create hwdevice",
            "Vulkan device does not support",
            "Vulkan device was created without",
            "has no optical flow queues",
            "Device has no compute queues",
            "Optical flow R8 input format unavailable",
        ]
        .iter()
        .any(|needle| error.contains(needle)),
    }
}

/// The video's frame rate for `player.stats()`: `estimated-vf-fps` (after
/// the filters, so a plugin's own rate change shows), except while motion
/// smoothing's converter is in the chain: then the rate going into it
/// (`input`, `Motion::input_fps`; `container-fps` while unknown), so a
/// plugin measuring the source (Frame Doubling) does not read the doubled
/// rate and back off, and Frame Doubling wins.
pub fn stats_fps(mpv: &Mpv, input: Option<f64>) -> Option<f64> {
    let float = |name: &str| {
        mpv.get_property::<f64>(name)
            .ok()
            .filter(|v| v.is_finite() && *v > 0.)
    };
    if labels(mpv).iter().any(|label| label == LABEL) {
        return input.or_else(|| float("container-fps"));
    }
    float("estimated-vf-fps").or_else(|| float("container-fps"))
}

/// The frame rate one frame step back is computed from: as `stats_fps`
/// without the converter's input (half the converter's output while it is
/// in the chain, so a step is one frame of what goes into it, not half).
pub fn step_fps(mpv: &Mpv) -> Option<f64> {
    let float = |name: &str| {
        mpv.get_property::<f64>(name)
            .ok()
            .filter(|v| v.is_finite() && *v > 0.)
    };
    let output = float("estimated-vf-fps");
    if labels(mpv).iter().any(|label| label == LABEL) {
        return output
            .map(|fps| fps / 2.)
            .or_else(|| float("container-fps"));
    }
    output.or_else(|| float("container-fps"))
}

/// The video's frame rate (`container-fps`), when mpv knows it.
fn source_fps(mpv: &Mpv) -> Option<f64> {
    mpv.get_property::<f64>("container-fps")
        .ok()
        .filter(|fps| fps.is_finite() && *fps > 0.)
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

#[cfg(test)]
mod tests;
