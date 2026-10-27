//! CPAL/rtrb adapter (host-internal), not an acoustic fence.
//! Producers write 48kHz stereo f32; the device stream is 48kHz f32 with the
//! device's channel count (stereo in the first two channels, mono as their
//! mean) and the device's buffer size unless it allows 1024 frames. Nothing
//! downstream depends on the callback size: producers pace on
//! `consumedFrames`. The data callback allocates nothing, takes only
//! a try-lock (contended only while R-H9 switches device: one silent buffer)
//! and performs no IO. Device/backend implementation details are outside
//! that claim. Reset cannot retract a buffer already handed to the device.
#![forbid(unsafe_code)]

use std::collections::VecDeque;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use anyhow::{Context, Result, ensure};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rtrb::{Consumer, Producer, RingBuffer};

const CAPACITY: usize = 9600;
/// Device buffer asked for when the device reports a range that holds it.
const PREFERRED_FRAMES: u32 = 1024;
const MAX_SAFE: u64 = (1_u64 << 53) - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Health {
    Ok,
    RecoverableXrun,
    DeviceChanged,
    StreamInvalidated,
    DeviceUnavailable,
    OtherError,
}
impl Health {
    pub fn fatal(self) -> bool {
        matches!(
            self,
            Self::StreamInvalidated | Self::DeviceUnavailable | Self::OtherError
        )
    }
}
#[derive(Clone, Debug)]
pub struct Facts {
    pub epoch: u64,
    pub acknowledged: bool,
    pub paused: bool,
    /// Absolute media position, initialized to reset's base_sample. Only actual
    /// matching PCM frames advance it, never silence or a wall/device clock.
    pub consumed_frames: u64,
    pub underrun_frames: u64,
    pub device_errors: u64,
    pub health: Health,
    pub xruns: u64,
    pub device_changes: u64,
    pub error_details: Vec<String>,
    pub error_overflow: u64,
    pub buffer: serde_json::Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Push {
    Accepted,
    Full,
    AwaitEpoch,
    Stale,
}

struct Shared {
    alive: AtomicBool,
    /// The stream plays on a chosen (not the default) device.
    chosen: AtomicBool,
    // Short control-only lock serializes concurrent reset callers. The callback
    // uses a bounded atomic snapshot and never touches this mutex.
    reset_lock: Mutex<()>,
    version: AtomicU64,
    desired: AtomicU64,
    base: AtomicU64,
    acknowledged: AtomicU64,
    consumed: AtomicU64,
    underrun: AtomicU64,
    errors: AtomicU64,
    xruns: AtomicU64,
    device_changes: AtomicU64,
    fatal: AtomicU64,
    error_overflow: AtomicU64,
    // Low32: f32 gain bits; bit32: mute. One coherent device-output setting.
    output: AtomicU64,
}

#[derive(Clone, Copy)]
struct Config {
    version: u64,
    packed: u64,
    base: u64,
}

impl Config {
    fn epoch(self) -> u64 {
        self.packed >> 1
    }
    fn paused(self) -> bool {
        self.packed & 1 != 0
    }
}

impl Shared {
    fn new() -> Self {
        Self {
            alive: AtomicBool::new(true),
            chosen: AtomicBool::new(false),
            reset_lock: Mutex::new(()),
            version: AtomicU64::new(0),
            desired: AtomicU64::new(1),
            base: AtomicU64::new(0),
            acknowledged: AtomicU64::new(0),
            consumed: AtomicU64::new(0),
            underrun: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            xruns: AtomicU64::new(0),
            device_changes: AtomicU64::new(0),
            fatal: AtomicU64::new(0),
            error_overflow: AtomicU64::new(0),
            output: AtomicU64::new(u64::from(1.0_f32.to_bits())),
        }
    }

    fn record_error(&self, kind: cpal::ErrorKind) {
        match kind {
            cpal::ErrorKind::Xrun => {
                self.xruns.fetch_add(1, Ordering::Relaxed);
            }
            cpal::ErrorKind::DeviceChanged => {
                self.device_changes.fetch_add(1, Ordering::Relaxed);
            }
            // A chosen device went away: the host moves the stream to the
            // system default (R-H9) instead of ending it.
            cpal::ErrorKind::DeviceNotAvailable if self.chosen.load(Ordering::Acquire) => {
                self.device_changes.fetch_add(1, Ordering::Relaxed);
            }
            cpal::ErrorKind::StreamInvalidated => {
                self.fatal
                    .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
                    .ok();
            }
            cpal::ErrorKind::DeviceNotAvailable => {
                self.fatal
                    .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire)
                    .ok();
            }
            _ => {
                self.fatal
                    .compare_exchange(0, 3, Ordering::AcqRel, Ordering::Acquire)
                    .ok();
            }
        }
        self.errors.fetch_add(1, Ordering::Release);
    }
    fn health(&self) -> Health {
        match self.fatal.load(Ordering::Acquire) {
            1 => Health::StreamInvalidated,
            2 => Health::DeviceUnavailable,
            3 => Health::OtherError,
            _ => {
                if self.xruns.load(Ordering::Relaxed) > 0 {
                    Health::RecoverableXrun
                } else if self.device_changes.load(Ordering::Relaxed) > 0 {
                    Health::DeviceChanged
                } else {
                    Health::Ok
                }
            }
        }
    }
    fn set_output(&self, gain: f32, muted: bool) -> Result<()> {
        ensure!(
            gain.is_finite() && (0.0..=2.0).contains(&gain),
            "audio output gain must be finite within 0..=2"
        );
        ensure!(
            self.alive.load(Ordering::Acquire) && self.fatal.load(Ordering::Acquire) == 0,
            "audio stream has stopped or is invalid"
        );
        self.output.store(
            u64::from(gain.to_bits()) | (u64::from(muted) << 32),
            Ordering::Release,
        );
        Ok(())
    }

    fn config(&self) -> Option<Config> {
        // SeqCst version accesses bracket payload loads. Version odd
        // denotes an in-progress control publication; no spinning in callback.
        let version = self.version.load(Ordering::SeqCst);
        if version & 1 != 0 {
            return None;
        }
        let packed = self.desired.load(Ordering::SeqCst);
        let base = self.base.load(Ordering::SeqCst);
        (self.version.load(Ordering::SeqCst) == version).then_some(Config {
            version,
            packed,
            base,
        })
    }

    fn configuration_current(&self, config: Config) -> bool {
        self.version.load(Ordering::SeqCst) == config.version
            && self.desired.load(Ordering::SeqCst) == config.packed
    }
    fn current(&self, config: Config) -> bool {
        self.configuration_current(config)
            && self.alive.load(Ordering::Acquire)
            && self.fatal.load(Ordering::Acquire) == 0
    }

    fn reset(&self, epoch: u64, paused: bool, base: u64) -> Result<()> {
        ensure!(
            epoch > 0 && epoch <= MAX_SAFE && base <= MAX_SAFE,
            "invalid audio epoch/base sample"
        );
        let _guard = self
            .reset_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("audio reset lock poisoned"))?;
        ensure!(
            self.alive.load(Ordering::Acquire) && self.fatal.load(Ordering::Acquire) == 0,
            "audio stream has stopped or is invalid"
        );
        ensure!(
            epoch > self.desired.load(Ordering::SeqCst) >> 1,
            "audio epoch must strictly increase"
        );
        let version = self.version.load(Ordering::SeqCst);
        ensure!(version <= u64::MAX - 2, "audio reset publication exhausted");
        self.version.store(version + 1, Ordering::SeqCst);
        self.base.store(base, Ordering::SeqCst);
        self.desired
            .store((epoch << 1) | u64::from(paused), Ordering::SeqCst);
        self.version.store(version + 2, Ordering::SeqCst);
        Ok(())
    }

    fn set_paused(&self, epoch: u64, paused: bool) -> Result<()> {
        let _guard = self
            .reset_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("audio reset lock poisoned"))?;
        ensure!(
            self.alive.load(Ordering::Acquire) && self.fatal.load(Ordering::Acquire) == 0,
            "audio stream has stopped or is invalid"
        );
        let desired = self.desired.load(Ordering::SeqCst);
        ensure!(
            epoch > 0 && epoch == desired >> 1,
            "audio pause epoch is stale"
        );
        let packed = (epoch << 1) | u64::from(paused);
        if packed == desired {
            return Ok(());
        }
        let version = self.version.load(Ordering::SeqCst);
        ensure!(version <= u64::MAX - 2, "audio reset publication exhausted");
        self.version.store(version + 1, Ordering::SeqCst);
        self.desired.store(packed, Ordering::SeqCst);
        self.version.store(version + 2, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
struct Frame {
    epoch: u64,
    samples: [f32; 2],
}

struct Callback {
    ring: Consumer<Frame>,
    shared: Arc<Shared>,
    /// Interleaved channels of the device buffer (>= 1).
    channels: usize,
    epoch: u64,
    clock: u64,
    underrun: u64,
}

impl Callback {
    /// One device buffer of a stream with `channels` interleaved channels:
    /// silence while the render state has another device's layout (during a
    /// device switch), without consuming anything.
    fn render_layout(&mut self, channels: usize, output: &mut [f32]) {
        if self.channels == channels {
            self.render(output);
        } else {
            output.fill(0.0);
        }
    }

    fn render(&mut self, output: &mut [f32]) {
        output.fill(0.0);
        let Some(config) = self.shared.config() else {
            return;
        };
        if !self.shared.current(config) || config.epoch() == 0 {
            return;
        }
        if self.epoch != config.epoch() {
            // Writer cannot admit this epoch until the ack below. Bound work
            // even if one old push was already in flight across the reset.
            for _ in 0..CAPACITY {
                if self.ring.pop().is_err() {
                    break;
                }
            }
            self.epoch = config.epoch();
            self.clock = config.base;
            self.underrun = 0;
            self.shared.consumed.store(self.clock, Ordering::Release);
            self.shared.underrun.store(0, Ordering::Release);
            if !self.shared.current(config) {
                return;
            }
        }
        if !self.shared.current(config) {
            return;
        }
        // Acknowledge the full epoch+pause request, including same-epoch resume.
        self.shared
            .acknowledged
            .store(config.packed, Ordering::Release);
        if config.paused() {
            return;
        }
        // Snapshot once per buffer. Ring PCM remains unmodified; this is output
        // gain/mute only, and muted matching frames still advance media time.
        let output_setting = self.shared.output.load(Ordering::Acquire);
        let gain = f32::from_bits(output_setting as u32);
        let muted = output_setting & (1_u64 << 32) != 0;
        // Old tagged frames may arrive AFTER flush from an already admitted
        // push. Discard them without advancing this epoch's media clock.
        let initial_clock = self.clock;
        let initial_underrun = self.underrun;
        let mut discard_budget = CAPACITY;
        for device_frame in output.chunks_exact_mut(self.channels) {
            if !self.shared.current(config) {
                break;
            }
            let frame = loop {
                match self.ring.pop() {
                    Ok(frame) if frame.epoch == self.epoch => break Some(frame),
                    Ok(_) if discard_budget > 0 => {
                        discard_budget -= 1;
                    }
                    _ => break None,
                }
            };
            if !self.shared.current(config) {
                break;
            }
            if let Some(frame) = frame {
                if !muted {
                    let [left, right] = frame.samples;
                    match device_frame {
                        [mono] => *mono = ((left + right) * 0.5 * gain).clamp(-1.0, 1.0),
                        // Front left/right; any further channels stay silent.
                        [l, r, ..] => {
                            *l = (left * gain).clamp(-1.0, 1.0);
                            *r = (right * gain).clamp(-1.0, 1.0);
                        }
                        [] => {}
                    }
                }
                self.clock = self.clock.saturating_add(1);
            } else {
                self.underrun = self.underrun.saturating_add(1);
            }
        }
        if !self.shared.current(config) {
            output.fill(0.);
            self.clock = initial_clock;
            self.underrun = initial_underrun;
            return;
        }
        self.shared.consumed.store(self.clock, Ordering::Release);
        self.shared.underrun.store(self.underrun, Ordering::Release);
    }
}

type ErrorDiagnostics = (Consumer<(u64, cpal::Error)>, VecDeque<String>);
pub struct Audio {
    shared: Arc<Shared>,
    stream: cpal::Stream,
    /// The render state, shared by the streams of successive devices.
    callback: Arc<Mutex<Callback>>,
    /// `player.setAudioDevice` id the stream plays on (`None` = system default).
    device: Option<String>,
    buffer: serde_json::Value,
    diagnostics: Mutex<ErrorDiagnostics>,
    errors: Arc<Mutex<rtrb::Producer<(u64, cpal::Error)>>>,
}

/// The prefix of mpv's device ids for the backend of cpal's default host.
const MPV_DEVICE_PREFIX: Option<&str> = if cfg!(target_os = "macos") {
    Some("coreaudio/")
} else if cfg!(windows) {
    Some("wasapi/")
} else {
    None
};

/// The part of a WASAPI render endpoint id (`IMMDevice::GetId`, cpal's
/// device id) that mpv drops from its device names and adds back on lookup
/// (ao_wasapi_utils.c).
const WASAPI_RENDER_ID_PREFIX: &str = "{0.0.0.00000000}.";

/// cpal's device id for an mpv device id, if it names a device of the default
/// host: CoreAudio `coreaudio/<UID>` → `<UID>`; WASAPI `wasapi/{guid}` →
/// `{0.0.0.00000000}.{guid}`.
fn cpal_device_id(mpv_id: &str) -> Option<std::borrow::Cow<'_, str>> {
    let prefix = MPV_DEVICE_PREFIX?;
    let id = mpv_id.strip_prefix(prefix).filter(|id| !id.is_empty())?;
    Some(
        if prefix == "wasapi/" && id.starts_with('{') && !id.starts_with(WASAPI_RENDER_ID_PREFIX) {
            format!("{WASAPI_RENDER_ID_PREFIX}{id}").into()
        } else {
            id.into()
        },
    )
}

/// An output device by mpv id (see [`cpal_device_id`]), else the system
/// default; `true` when it is the chosen one.
fn output_device(id: Option<&str>) -> Result<(cpal::Device, bool)> {
    let host = cpal::default_host();
    let wanted = id
        .and_then(cpal_device_id)
        .and_then(|uid| host.device_by_id(&cpal::DeviceId::new(host.id(), &*uid)));
    match wanted {
        Some(device) => Ok((device, true)),
        None => host
            .default_output_device()
            .map(|device| (device, false))
            .context("default audio output unavailable"),
    }
}

/// The buffer to ask for: 1024 frames when the device reports a range that
/// holds it, else the device's default. WASAPI shared mode reports only its
/// period (`Range { min: 480, max: 480 }` at 48 kHz, or `Unknown`) and calls
/// back at that period whatever is asked; mpv and other players likewise
/// take the device's period instead of insisting on a size.
fn buffer_request(supported: &cpal::SupportedBufferSize) -> cpal::BufferSize {
    match supported {
        cpal::SupportedBufferSize::Range { min, max }
            if (*min..=*max).contains(&PREFERRED_FRAMES) =>
        {
            cpal::BufferSize::Fixed(PREFERRED_FRAMES)
        }
        _ => cpal::BufferSize::Default,
    }
}

/// A 48 kHz f32 output config: stereo if offered, else the fewest channels
/// above two (a multichannel mix format), else mono.
fn choose_config(
    configs: impl IntoIterator<Item = cpal::SupportedStreamConfigRange>,
) -> Option<cpal::SupportedStreamConfig> {
    configs
        .into_iter()
        .filter(|config| config.sample_format() == cpal::SampleFormat::F32 && config.channels() > 0)
        .filter_map(|config| config.try_with_sample_rate(48_000))
        .min_by_key(|config| match config.channels() {
            2 => 0,
            1 => u32::MAX,
            channels => u32::from(channels),
        })
}

/// A paused 48 kHz f32 stream on `device` (see [`choose_config`],
/// [`buffer_request`]) rendering from `callback`, and its channel count.
/// It renders only once the caller sets `callback`'s channels to that count.
fn build_stream(
    device: &cpal::Device,
    shared: &Arc<Shared>,
    callback: &Arc<Mutex<Callback>>,
    errors: &Arc<Mutex<rtrb::Producer<(u64, cpal::Error)>>>,
) -> Result<(cpal::Stream, serde_json::Value, usize)> {
    let default_config = format!("{:?}", device.default_output_config()?);
    let supported = choose_config(device.supported_output_configs()?)
        .context("output device does not support 48kHz f32")?;
    let supported_buffer = format!("{:?}", supported.buffer_size());
    let mut config = supported.config();
    config.buffer_size = buffer_request(supported.buffer_size());
    let fixed = match config.buffer_size {
        cpal::BufferSize::Fixed(frames) => Some(frames),
        cpal::BufferSize::Default => None,
    };
    let channels = usize::from(config.channels);
    let render = callback.clone();
    let recorded = shared.clone();
    let queue = errors.clone();
    let stream = device.build_output_stream(
        config,
        // Only the stream being replaced contends for this lock (while the
        // device switches); a missed lock renders one buffer of silence, as
        // does a buffer while the render state has another device's layout.
        move |output: &mut [f32], _| match render.try_lock() {
            Ok(mut callback) => callback.render_layout(channels, output),
            Err(_) => output.fill(0.0),
        },
        move |error| {
            recorded.record_error(error.kind());
            let state = recorded.desired.load(Ordering::SeqCst);
            let full = queue
                .lock()
                .map_or(true, |mut queue| queue.push((state, error)).is_err());
            if full {
                recorded.error_overflow.fetch_add(1, Ordering::Relaxed);
            }
        },
        None,
    )?;
    let actual_buffer = stream.buffer_size()?;
    // `requestedFrames` stays a number (SDK `producer.ts`): the device's own
    // period when its default was asked for, marked by `"requested":"default"`.
    let buffer = serde_json::json!({"device":device.to_string(),"channels":channels,"supported":supported_buffer,"defaultConfig":default_config,"requested":if fixed.is_some(){"fixed"}else{"default"},"requestedFrames":fixed.unwrap_or(actual_buffer),"actualFrames":actual_buffer,"oneBufferMilliseconds":actual_buffer as f64/48.});
    Ok((stream, buffer, channels))
}

/// Sets the layout the render state writes (`build_stream`'s channel count).
fn set_channels(callback: &Mutex<Callback>, channels: usize) {
    callback
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .channels = channels;
}
pub struct Writer {
    shared: Arc<Shared>,
    ring: Producer<Frame>,
    epoch: u64,
    next: u64,
}

impl Audio {
    /// Open on the device `player.setAudioDevice` chose (mpv id), or the
    /// system default when it is `None` or absent.
    pub fn open(device: Option<&str>) -> Result<(Self, Writer)> {
        let shared = Arc::new(Shared::new());
        let (ring, consumer) = RingBuffer::new(CAPACITY);
        let callback = Arc::new(Mutex::new(Callback {
            ring: consumer,
            shared: shared.clone(),
            channels: 2,
            epoch: 0,
            clock: 0,
            underrun: 0,
        }));
        let (error_send, error_receive) = RingBuffer::<(u64, cpal::Error)>::new(8);
        let errors = Arc::new(Mutex::new(error_send));
        let (target, chosen) = output_device(device)?;
        shared.chosen.store(chosen, Ordering::Release);
        let (stream, buffer, channels) = build_stream(&target, &shared, &callback, &errors)?;
        set_channels(&callback, channels);
        stream.play()?;
        let writer = Writer {
            shared: shared.clone(),
            ring,
            epoch: 0,
            next: 0,
        };
        Ok((
            Self {
                shared,
                stream,
                callback,
                device: device.map(str::to_owned),
                buffer,
                diagnostics: Mutex::new((error_receive, VecDeque::new())),
                errors,
            },
            writer,
        ))
    }

    /// R-H9: move the stream to another device (`None` = system default),
    /// keeping its queue, clock and pause state. The old stream stops first;
    /// on failure it plays on.
    pub fn retarget(&mut self, device: Option<&str>) -> Result<()> {
        if self.device.as_deref() == device {
            return Ok(());
        }
        let (target, chosen) = output_device(device)?;
        self.stream.pause()?;
        let previous = self
            .callback
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .channels;
        match build_stream(&target, &self.shared, &self.callback, &self.errors).and_then(
            |(stream, buffer, channels)| {
                set_channels(&self.callback, channels);
                stream.play().map(|()| (stream, buffer)).map_err(Into::into)
            },
        ) {
            Ok((stream, buffer)) => {
                self.stream = stream;
                self.buffer = buffer;
                self.device = device.map(str::to_owned);
                self.shared.chosen.store(chosen, Ordering::Release);
                // A lost device does not end a stream that moved on.
                let _ =
                    self.shared
                        .fatal
                        .compare_exchange(2, 0, Ordering::AcqRel, Ordering::Acquire);
                Ok(())
            }
            Err(error) => {
                // Stay on the old device; not retried until the choice changes.
                self.device = device.map(str::to_owned);
                set_channels(&self.callback, previous);
                let _ = self.stream.play();
                Err(error)
            }
        }
    }

    pub fn reset(&self, epoch: u64, paused: bool, base_sample: u64) -> Result<()> {
        self.shared.reset(epoch, paused, base_sample)
    }

    /// Same-epoch pause/resume preserves queued PCM, cursor and reset base.
    /// The callback acknowledges this exact pause request on its next turn.
    pub fn set_paused(&self, epoch: u64, paused: bool) -> Result<()> {
        self.shared.set_paused(epoch, paused)
    }

    /// Device output only. Does not edit queued PCM, change pause or stop the
    /// consumption clock. Values take effect at a subsequent callback buffer.
    pub fn set_output(&self, gain: f32, muted: bool) -> Result<()> {
        self.shared.set_output(gain, muted)
    }

    pub fn facts(&self) -> Facts {
        // A bounded snapshot; on a concurrent reset, report unacknowledged with
        // base rather than mix the old callback clock into a new epoch.
        let desired = self.shared.desired.load(Ordering::SeqCst);
        let config = self.shared.config();
        let before = self.shared.acknowledged.load(Ordering::Acquire);
        let consumed = self.shared.consumed.load(Ordering::Acquire);
        let underrun = self.shared.underrun.load(Ordering::Acquire);
        let after = self.shared.acknowledged.load(Ordering::Acquire);
        let packed = config.map_or(desired, |c| c.packed);
        let clock_valid = config.is_some_and(|c| {
            self.shared.configuration_current(c)
                && c.epoch() > 0
                && before >> 1 == c.epoch()
                && after == before
        });
        let acknowledged = clock_valid && before == packed;
        let mut diagnostics = self
            .diagnostics
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while let Ok((state, error)) = diagnostics.0.pop() {
            if diagnostics.1.len() == 8 {
                diagnostics.1.pop_front();
            }
            diagnostics.1.push_back(
                format!(
                    "epoch={} paused={} {:?}: {}",
                    state >> 1,
                    state & 1 != 0,
                    error.kind(),
                    error
                )
                .chars()
                .take(1024)
                .collect(),
            );
        }
        let error_details = diagnostics.1.iter().cloned().collect();
        Facts {
            epoch: packed >> 1,
            paused: packed & 1 != 0,
            acknowledged,
            consumed_frames: if clock_valid {
                consumed
            } else {
                config.map_or(0, |c| c.base)
            },
            underrun_frames: if clock_valid { underrun } else { 0 },
            device_errors: self.shared.errors.load(Ordering::Acquire),
            health: self.shared.health(),
            xruns: self.shared.xruns.load(Ordering::Relaxed),
            device_changes: self.shared.device_changes.load(Ordering::Relaxed),
            error_details,
            error_overflow: self.shared.error_overflow.load(Ordering::Relaxed),
            buffer: self.buffer.clone(),
        }
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        self.shared.alive.store(false, Ordering::Release);
    }
}

fn validate_frames(start: u64, frames: &[[f32; 2]]) -> Result<u64> {
    ensure!(
        (1..=4800).contains(&frames.len()),
        "audio chunk must contain 1..=4800 frames"
    );
    let end = start
        .checked_add(frames.len() as u64)
        .filter(|end| *end <= MAX_SAFE)
        .context("audio sample range exceeds safe integers")?;
    ensure!(
        frames
            .iter()
            .flatten()
            .all(|sample| sample.is_finite() && sample.abs() <= 1.0),
        "audio samples must be finite within -1..=1"
    );
    Ok(end)
}

impl Writer {
    pub fn push(&mut self, epoch: u64, start_sample: u64, frames: &[[f32; 2]]) -> Result<Push> {
        let end = validate_frames(start_sample, frames)?;
        ensure!(epoch > 0 && epoch <= MAX_SAFE, "invalid audio epoch");
        if !self.shared.alive.load(Ordering::Acquire)
            || self.shared.fatal.load(Ordering::Acquire) != 0
        {
            return Ok(Push::Stale);
        }
        let Some(config) = self.shared.config() else {
            return Ok(Push::AwaitEpoch);
        };
        if config.epoch() != epoch {
            return Ok(Push::Stale);
        }
        if self.shared.acknowledged.load(Ordering::Acquire) != config.packed {
            return Ok(Push::AwaitEpoch);
        }
        if self.epoch != epoch {
            self.epoch = epoch;
            self.next = config.base;
        }
        ensure!(
            start_sample == self.next,
            "audio chunks must be contiguous from reset base sample"
        );
        if self.ring.slots() < frames.len() {
            return Ok(Push::Full);
        }
        for samples in frames {
            // Single producer: consumers only increase free capacity after slots.
            self.ring
                .push(Frame {
                    epoch,
                    samples: *samples,
                })
                .map_err(|_| anyhow::anyhow!("audio ring capacity invariant failed"))?;
        }
        // A same-epoch pause transition preserves these frames, so it must not
        // report Stale and invite re-sending an already queued chunk. New-epoch
        // transitions instead invalidate the tagged frames and the writer cursor.
        if !self.shared.alive.load(Ordering::Acquire)
            || self.shared.fatal.load(Ordering::Acquire) != 0
            || self.shared.desired.load(Ordering::SeqCst) >> 1 != epoch
        {
            return Ok(Push::Stale);
        }
        self.next = end;
        Ok(Push::Accepted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recoverable_notifications_keep_consuming_but_sticky_fatal_stops_without_clock_rollback() {
        let (shared, mut writer, mut callback) = pair();
        shared.reset(1, false, 0).unwrap();
        callback.render(&mut []);
        writer.push(1, 0, &[[0.2, 0.3]; 4]).unwrap();
        shared.record_error(cpal::ErrorKind::Xrun);
        assert_eq!(shared.health(), Health::RecoverableXrun);
        let mut output = [0.; 2];
        callback.render(&mut output);
        assert_eq!(output, [0.2, 0.3]);
        assert_eq!(shared.consumed.load(Ordering::Acquire), 1);
        shared.record_error(cpal::ErrorKind::StreamInvalidated);
        shared.record_error(cpal::ErrorKind::DeviceChanged);
        assert_eq!(shared.health(), Health::StreamInvalidated);
        callback.render(&mut output);
        assert_eq!(output, [0.; 2]);
        assert_eq!(shared.consumed.load(Ordering::Acquire), 1);
        assert_eq!(writer.push(1, 4, &[[0.2; 2]]).unwrap(), Push::Stale);
        assert!(shared.configuration_current(shared.config().unwrap()));
        assert_eq!(shared.errors.load(Ordering::Acquire), 3);
    }
    #[test]
    fn output_gain_clamps_and_mute_consumes_without_modifying_queued_pcm() {
        let (shared, mut writer, mut callback) = pair();
        assert_eq!(
            shared.output.load(Ordering::Acquire),
            u64::from(1.0_f32.to_bits())
        );
        shared.reset(1, false, 0).unwrap();
        callback.render(&mut []);
        writer
            .push(1, 0, &[[0.75, -0.75], [0.5, -0.5], [0.8, -0.8]])
            .unwrap();
        let mut output = [0.0; 2];
        shared.set_output(2.0, false).unwrap();
        callback.render(&mut output);
        assert_eq!(output, [1.0, -1.0]);
        assert_eq!(callback.clock, 1);
        shared.set_output(2.0, true).unwrap();
        callback.render(&mut output);
        assert_eq!(output, [0.0; 2]);
        assert_eq!(callback.clock, 2);
        shared.set_output(0.5, false).unwrap();
        callback.render(&mut output);
        assert_eq!(output, [0.4, -0.4]);
        assert_eq!((callback.clock, callback.underrun), (3, 0));
        let retained = shared.output.load(Ordering::Acquire);
        for gain in [f32::NAN, f32::INFINITY, -0.01, 2.01] {
            assert!(shared.set_output(gain, true).is_err());
            assert_eq!(shared.output.load(Ordering::Acquire), retained);
        }
        shared.set_output(0.0, false).unwrap();
        writer.push(1, 3, &[[1.0; 2]]).unwrap();
        callback.render(&mut output);
        assert_eq!(output, [0.0; 2]);
        assert_eq!((callback.clock, callback.underrun), (4, 0));
    }
    fn pair() -> (Arc<Shared>, Writer, Callback) {
        let shared = Arc::new(Shared::new());
        let (ring, consumer) = RingBuffer::new(CAPACITY);
        (
            shared.clone(),
            Writer {
                shared: shared.clone(),
                ring,
                epoch: 0,
                next: 0,
            },
            Callback {
                shared,
                ring: consumer,
                channels: 2,
                epoch: 0,
                clock: 0,
                underrun: 0,
            },
        )
    }
    #[test]
    fn epoch_ack_flush_and_late_old_packet_never_advance_new_clock() {
        let (shared, mut writer, mut callback) = pair();
        shared.reset(1, false, 100).unwrap();
        assert_eq!(writer.push(1, 100, &[[0.5; 2]]).unwrap(), Push::AwaitEpoch);
        callback.render(&mut [0.0; 2]);
        assert_eq!(writer.push(1, 100, &[[0.5; 2]]).unwrap(), Push::Accepted);
        shared.reset(2, false, 200).unwrap();
        callback.render(&mut [0.0; 2]);
        writer
            .ring
            .push(Frame {
                epoch: 1,
                samples: [0.9; 2],
            })
            .unwrap();
        assert_eq!(writer.push(2, 200, &[[0.25; 2]]).unwrap(), Push::Accepted);
        let mut output = [0.0; 2];
        callback.render(&mut output);
        assert_eq!(output, [0.25; 2]);
        assert_eq!(callback.clock, 201);
        assert_eq!(writer.push(1, 101, &[[0.5; 2]]).unwrap(), Push::Stale);
    }
    #[test]
    fn paused_output_retains_pcm_and_underrun_does_not_advance_media_clock() {
        let (shared, mut writer, mut callback) = pair();
        shared.reset(1, true, 10).unwrap();
        callback.render(&mut [1.0; 4]);
        writer.push(1, 10, &[[0.5; 2]]).unwrap();
        let mut output = [1.0; 4];
        callback.render(&mut output);
        assert_eq!(output, [0.0; 4]);
        assert_eq!(callback.ring.slots(), 1);
        assert_eq!((callback.clock, callback.underrun), (10, 0));
        shared.reset(2, false, 20).unwrap();
        callback.render(&mut output);
        assert_eq!((callback.clock, callback.underrun), (20, 2));
        writer.push(2, 20, &[[1.0, -1.0]]).unwrap();
        callback.render(&mut output);
        assert_eq!(&output[..2], &[1.0, -1.0]);
        assert_eq!((callback.clock, callback.underrun), (21, 3));
    }
    #[test]
    fn full_is_all_or_nothing_and_dropped_audio_rejects_writer() {
        let (shared, mut writer, mut callback) = pair();
        shared.reset(1, true, 0).unwrap();
        callback.render(&mut []);
        let chunk = vec![[0.0; 2]; 4800];
        assert_eq!(writer.push(1, 0, &chunk).unwrap(), Push::Accepted);
        assert_eq!(writer.push(1, 4800, &chunk).unwrap(), Push::Accepted);
        assert_eq!(writer.push(1, 9600, &[[0.0; 2]]).unwrap(), Push::Full);
        assert_eq!(writer.next, 9600);
        assert_eq!(callback.ring.slots(), CAPACITY);
        shared.alive.store(false, Ordering::Release);
        assert_eq!(writer.push(1, 9600, &[[0.0; 2]]).unwrap(), Push::Stale);
    }
    #[test]
    fn same_epoch_resume_keeps_prepared_pcm_and_acks_full_pause_state() {
        let (shared, mut writer, mut callback) = pair();
        shared.reset(1, true, 50).unwrap();
        callback.render(&mut []);
        assert_eq!(shared.acknowledged.load(Ordering::Acquire), 3);
        assert_eq!(writer.push(1, 50, &[[0.75; 2]]).unwrap(), Push::Accepted);
        shared.set_paused(1, false).unwrap();
        shared.set_paused(1, false).unwrap();
        assert_eq!(writer.push(1, 51, &[[0.5; 2]]).unwrap(), Push::AwaitEpoch);
        assert!(shared.set_paused(2, true).is_err());
        let mut output = [0.0; 2];
        callback.render(&mut output);
        assert_eq!(output, [0.75; 2]);
        assert_eq!(callback.clock, 51);
        assert_eq!(shared.acknowledged.load(Ordering::Acquire), 2);
        assert_eq!(writer.push(1, 51, &[[0.5; 2]]).unwrap(), Push::Accepted);
        shared.set_paused(1, true).unwrap();
        callback.render(&mut output);
        assert_eq!(output, [0.0; 2]);
        assert_eq!(callback.ring.slots(), 1);
        assert_eq!(callback.clock, 51);
        assert_eq!(shared.acknowledged.load(Ordering::Acquire), 3);
    }

    #[test]
    fn buffer_request_takes_1024_only_from_a_range_that_holds_it() {
        use cpal::{BufferSize::*, SupportedBufferSize};
        let range = |min, max| buffer_request(&SupportedBufferSize::Range { min, max });
        assert_eq!(range(15, 4096), Fixed(1024));
        assert_eq!(range(1024, 1024), Fixed(1024));
        // WASAPI shared mode: only the device period (10 ms at 48 kHz), or nothing.
        assert_eq!(range(480, 480), Default);
        assert_eq!(buffer_request(&SupportedBufferSize::Unknown), Default);
        assert_eq!(range(1025, 8192), Default);
        assert_eq!(range(64, 512), Default);
    }

    #[test]
    fn config_prefers_48k_stereo_f32_then_a_multichannel_mix_then_mono() {
        use cpal::{SampleFormat::*, SupportedBufferSize::Unknown};
        let range = |channels, min, max, format| {
            cpal::SupportedStreamConfigRange::new(channels, min, max, Unknown, format)
        };
        let chosen = |configs: Vec<_>| choose_config(configs).map(|c| c.channels());
        assert_eq!(
            chosen(vec![
                range(6, 48_000, 48_000, F32),
                range(2, 44_100, 44_100, F32),
                range(2, 48_000, 48_000, I16),
                range(2, 8_000, 384_000, F32),
            ]),
            Some(2)
        );
        let picked = choose_config(vec![range(2, 8_000, 384_000, F32)]).unwrap();
        assert_eq!(picked.sample_rate(), 48_000);
        // WASAPI lists only the mix format's channel count.
        assert_eq!(
            chosen(vec![
                range(1, 48_000, 48_000, F32),
                range(8, 48_000, 48_000, F32),
                range(6, 48_000, 48_000, F32),
            ]),
            Some(6)
        );
        assert_eq!(chosen(vec![range(1, 48_000, 48_000, F32)]), Some(1));
        // Resampling is not done here: a device without 48 kHz is refused.
        assert_eq!(chosen(vec![range(2, 44_100, 44_100, F32)]), None);
        assert_eq!(chosen(vec![range(0, 48_000, 48_000, F32)]), None);
    }

    #[test]
    fn multichannel_and_mono_devices_get_stereo_in_front_or_its_mean() {
        let (shared, mut writer, mut callback) = pair();
        shared.reset(1, false, 0).unwrap();
        callback.render(&mut []);
        writer
            .push(1, 0, &[[0.5, -0.25], [0.75, 0.25], [1.0, 1.0]])
            .unwrap();
        callback.channels = 6;
        let mut output = [1.0; 6];
        callback.render(&mut output);
        assert_eq!(output, [0.5, -0.25, 0.0, 0.0, 0.0, 0.0]);
        callback.channels = 1;
        let mut output = [1.0; 1];
        callback.render(&mut output);
        assert_eq!(output, [0.5]);
        shared.set_output(2.0, false).unwrap();
        callback.render(&mut output);
        assert_eq!(output, [1.0]);
        assert_eq!((callback.clock, callback.underrun), (3, 0));
    }

    #[test]
    fn another_layout_is_silent_and_an_odd_buffer_ignores_its_partial_frame() {
        let (shared, mut writer, mut callback) = pair();
        shared.reset(1, false, 0).unwrap();
        callback.render(&mut []);
        writer.push(1, 0, &[[0.5, -0.5], [0.25, -0.25]]).unwrap();
        // A 6-channel stream while the state still has the old stereo layout.
        let mut output = [1.0; 6];
        callback.render_layout(6, &mut output);
        assert_eq!(output, [0.0; 6]);
        assert_eq!(
            (callback.clock, callback.underrun, callback.ring.slots()),
            (0, 0, 2)
        );
        let mut output = [1.0; 5];
        callback.render_layout(2, &mut output);
        assert_eq!(output, [0.5, -0.5, 0.25, -0.25, 0.0]);
        assert_eq!((callback.clock, callback.underrun), (2, 0));
    }

    #[test]
    fn mpv_device_ids_map_to_the_default_hosts_ids_only() {
        // mpv lists WASAPI render endpoints without `{0.0.0.00000000}.`.
        let wasapi = "wasapi/{6f1a0d4e-5b2c-4d3e-9f80-0123456789ab}";
        let full = "{0.0.0.00000000}.{6f1a0d4e-5b2c-4d3e-9f80-0123456789ab}";
        let id = |mpv: &str| cpal_device_id(mpv).map(|id| id.into_owned());
        if cfg!(windows) {
            assert_eq!(id(wasapi).as_deref(), Some(full));
            assert_eq!(id(&format!("wasapi/{full}")).as_deref(), Some(full));
            assert_eq!(id("coreaudio/BuiltInSpeakerDevice"), None);
        }
        if cfg!(target_os = "macos") {
            assert_eq!(
                id("coreaudio/BuiltInSpeakerDevice").as_deref(),
                Some("BuiltInSpeakerDevice")
            );
            assert_eq!(id(wasapi), None);
        }
        for other in ["auto", "openal", "sdl/x", "wasapi/", "coreaudio/", ""] {
            assert_eq!(id(other), None, "{other}");
        }
    }

    /// Real output device: the stream starts and reports its buffer.
    /// `cargo test real_output_device -- --ignored --nocapture`
    #[test]
    #[ignore = "needs an audio output device"]
    // Test output for --nocapture, not the host's stdout.
    #[allow(clippy::disallowed_macros)]
    fn real_output_device_opens_and_consumes() {
        let device = cpal::default_host().default_output_device().unwrap();
        let supported: Vec<_> = device.supported_output_configs().unwrap().collect();
        println!("default config: {:?}", device.default_output_config());
        for config in &supported {
            println!("supported: {config:?}");
        }
        let (audio, mut writer) = Audio::open(None).unwrap();
        audio.reset(1, false, 0).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !audio.facts().acknowledged {
            assert!(std::time::Instant::now() < deadline, "no callback");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let silence = [[0.0; 2]; 4800];
        assert_eq!(writer.push(1, 0, &silence).unwrap(), Push::Accepted);
        while audio.facts().consumed_frames < 4800 {
            assert!(std::time::Instant::now() < deadline, "not consumed");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let facts = audio.facts();
        println!("buffer: {}", facts.buffer);
        assert!(!facts.health.fatal(), "{:?}", facts.health);
        assert!(facts.buffer["actualFrames"].as_u64().unwrap() > 0);
        assert!(facts.buffer["requestedFrames"].is_u64());
        // Every output device mpv lists for cpal's host resolves to a cpal device.
        let mpv = rsmpv::Mpv::builder().unwrap().build().unwrap();
        let Ok(rsmpv::Node::Array(entries)) = mpv.get_property::<rsmpv::Node>("audio-device-list")
        else {
            panic!("no audio-device-list");
        };
        let host = cpal::default_host();
        let mut resolved = 0;
        for entry in entries {
            let rsmpv::Node::Map(fields) = entry else {
                continue;
            };
            let Some((_, rsmpv::Node::String(name))) = fields.iter().find(|(k, _)| k == "name")
            else {
                continue;
            };
            let Some(id) = cpal_device_id(name) else {
                continue;
            };
            let device = host.device_by_id(&cpal::DeviceId::new(host.id(), &*id));
            println!("mpv {name} -> cpal {id}: {}", device.is_some());
            assert!(device.is_some(), "{name}");
            resolved += 1;
        }
        if MPV_DEVICE_PREFIX.is_some() {
            assert!(resolved > 0, "mpv listed no device of cpal's host");
        }
    }

    #[test]
    fn invalid_chunks_and_nonmonotone_reset_do_not_mutate_state() {
        let (shared, mut writer, mut callback) = pair();
        assert!(shared.reset(0, false, 0).is_err());
        shared.reset(1, false, MAX_SAFE - 1).unwrap();
        assert!(shared.reset(1, true, 0).is_err());
        assert!(shared.reset(2, false, MAX_SAFE + 1).is_err());
        callback.render(&mut []);
        for sample in [f32::NAN, f32::INFINITY, 1.01, -1.01] {
            assert!(writer.push(1, MAX_SAFE - 1, &[[sample, 0.0]]).is_err());
        }
        assert!(writer.push(1, MAX_SAFE - 1, &[]).is_err());
        assert!(writer.push(1, MAX_SAFE - 1, &[[0.0; 2]; 2]).is_err());
        assert!(writer.push(1, 0, &[[0.0; 2]]).is_err());
        assert_eq!(
            writer.push(1, MAX_SAFE - 1, &[[0.0; 2]]).unwrap(),
            Push::Accepted
        );
    }
}
