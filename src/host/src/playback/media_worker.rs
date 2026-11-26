//! Finite offline raw-PNG worker (host-internal).
//! Run only on a background worker. It never controls the realtime backend.
//! File leases are metadata-checked, not strong immutable snapshots. Publication
//! uses tempfile's no-clobber facility, not a universal filesystem transaction or
//! a cross-platform all-or-nothing/durability guarantee.
#![forbid(unsafe_code)]

use std::{
    io::{self, BufReader, Read},
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    thread,
    time::Duration,
};

use crate::playback::source_lease::{Lease, Reader};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

// Distinguish queued cancellation from an active worker cancelling, so a
// duplicate runner cannot report terminal cancellation before the owner reaps.
const CANCELLED_BEFORE_START: u8 = 7;
const POLL: Duration = Duration::from_millis(20);
const STDERR_TAIL: usize = 64 * 1024;
const PNG_BYTES: usize = 128 * 1024 * 1024;
/// `facts.preroll` of an export without effects or with stateless ones:
/// decoding starts at the keyframe before the start (an input seek).
const SEEK_PREROLL: &str = "keyframe-before-start";
/// `facts.preroll` of an export with stateful effects: decoding starts at the
/// keyframe before a bounded history window (`HISTORY_SECONDS`).
const BOUNDED_PREROLL: &str = "bounded-history";

/// The still-image format of a one-frame export. FFmpeg always writes a PNG;
/// JPEG and BMP are converted from it (validated) before publication.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Still {
    Png,
    /// Quality 1–100.
    Jpeg {
        quality: u8,
    },
    Bmp,
}

pub enum Output {
    Still(Still),
    /// Re-encoded H.264 + AAC from the exact frame (`media_jobs` "Precise").
    Mp4 {
        end: f64,
        audio: Option<crate::playback::media_tracks::AudioTrack>,
        encoding: Encoding,
    },
    /// Stream copy from the keyframe at or before the start (`media_copy.rs`).
    Copy {
        end: f64,
        audio: Option<crate::playback::media_tracks::AudioTrack>,
        container: Container,
    },
}
impl Output {
    fn end(&self) -> Option<f64> {
        match self {
            Self::Still(_) => None,
            Self::Mp4 { end, .. } | Self::Copy { end, .. } => Some(*end),
        }
    }
    fn has_audio(&self) -> bool {
        matches!(
            self,
            Self::Mp4 { audio: Some(_), .. } | Self::Copy { audio: Some(_), .. }
        )
    }
}

/// The typed options of a re-encoded clip: a closed set, never FFmpeg
/// strings. Every field is optional; the defaults are the encoder's settings
/// before these options existed (CRF 18, x264 `veryfast`, the source's size
/// and rate, AAC 192 kb/s, one thread).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Encoding {
    /// x264 CRF 14–30 (lower: better picture, bigger file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crf: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<Speed>,
    /// A ceiling: 1080, 720 or 480; a shorter picture is left as it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_height: Option<u32>,
    /// A ceiling: 60 or 30 frames per second; a slower source is left as it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<u32>,
    /// AAC kb/s: 128, 192 or 256.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_bitrate: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threads: Option<Threads>,
    /// `source`: aim at the source video's average bitrate (one-pass ABR,
    /// peaks up to 1.5×) instead of a CRF, for a file about the source's size.
    /// Not with `crf`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_bitrate: Option<VideoBitrate>,
}
/// The video bitrate target of a re-encode: only the source's for now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoBitrate {
    Source,
}
/// x264 presets by name: `veryfast`, `medium`, `slow`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Speed {
    Faster,
    Balanced,
    Smaller,
}
/// `single` (the default) or `auto`: the host picks the count
/// ([`auto_threads`]), leaving the other half of the cores to playback.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Threads {
    Single,
    Auto,
}
/// A stream copy's container. `auto`: MP4 when the streams fit
/// ([`media_copy::fits_mp4`](crate::playback::media_copy::fits_mp4)), else MKV.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Container {
    #[default]
    Auto,
    Mp4,
    Mkv,
}
impl Encoding {
    pub fn valid(&self) -> bool {
        self.crf.is_none_or(|crf| (14..=30).contains(&crf))
            && (self.crf.is_none() || self.video_bitrate.is_none())
            && self
                .max_height
                .is_none_or(|h| matches!(h, 1080 | 720 | 480))
            && self.fps.is_none_or(|fps| matches!(fps, 60 | 30))
            && self
                .audio_bitrate
                .is_none_or(|rate| matches!(rate, 128 | 192 | 256))
    }
    pub(super) fn crf(&self) -> u8 {
        self.crf.unwrap_or(18)
    }
    pub(super) fn preset(&self) -> &'static str {
        match self.speed {
            None | Some(Speed::Faster) => "veryfast",
            Some(Speed::Balanced) => "medium",
            Some(Speed::Smaller) => "slow",
        }
    }
    pub(super) fn audio_bitrate(&self) -> u32 {
        self.audio_bitrate.unwrap_or(192)
    }
    pub fn threads(&self) -> usize {
        match self.threads {
            None | Some(Threads::Single) => 1,
            Some(Threads::Auto) => auto_threads(),
        }
    }
}
/// Half the logical processors, 1–8: the rest stay with playback and the UI,
/// which must keep the display rate while an export runs.
pub fn auto_threads() -> usize {
    thread::available_parallelism().map_or(1, |n| (n.get() / 2).clamp(1, 8))
}

/// A request the export cannot do as asked (a stream that cannot be copied,
/// or not into the asked container): the job fails with code `unsupported`.
#[derive(Debug)]
pub struct Refused(pub String);
impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Refused {}
pub struct RenderRequest {
    pub lease: Arc<Lease>,
    pub time: f64,
    pub destination: PathBuf,
    pub ffmpeg: PathBuf,
    pub processed: Option<Arc<crate::playback::media_jobs::FrozenProcessed>>,
    pub output: Output,
    pub video_stream: u32,
    pub source_origin: f64,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Queued,
    Running,
    Cancelling,
    Publishing,
    Completed,
    Failed,
    Cancelled,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Completed {
        destination: PathBuf,
        /// A clip's first and last-frame-end source times as written.
        range: Option<(f64, f64)>,
    },
    Failed {
        /// `export-failed`, or `unsupported` for a [`Refused`] request.
        code: &'static str,
        message: String,
    },
    Cancelled,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelResult {
    Requested,
    TooLate,
    Terminal,
}

pub struct Control {
    state: AtomicU8,
    publication_started: AtomicBool,
    result: Mutex<Option<Outcome>>,
    facts: Mutex<serde_json::Value>,
    /// Seconds written and to write, from FFmpeg's `-progress` (clips only).
    progress: Mutex<Option<(f64, f64)>>,
    /// A join's current part (`media_join.rs`): `progress` is then that
    /// part's, mapped onto the whole output.
    segment: Mutex<Option<Segment>>,
}
/// The part of a join being written: `progress` reports
/// `before + min(done, length)` of `total`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    /// 1-based.
    pub part: usize,
    pub parts: usize,
    /// Output seconds of the parts before it.
    pub before: f64,
    /// Its own output seconds.
    pub length: f64,
    /// Output seconds of the whole join.
    pub total: f64,
}
impl Default for Control {
    fn default() -> Self {
        Self::new()
    }
}
impl Control {
    pub fn new() -> Self {
        Self {
            state: AtomicU8::new(Status::Queued as u8),
            publication_started: AtomicBool::new(false),
            result: Mutex::new(None),
            facts: Mutex::new(serde_json::Value::Null),
            progress: Mutex::new(None),
            segment: Mutex::new(None),
        }
    }
    /// `(done, total)` seconds of a clip being written; `None` while unknown
    /// (a frame, or before the encoder starts). A join reports its whole
    /// output from its first part on.
    pub fn progress(&self) -> Option<(f64, f64)> {
        let raw = *self
            .progress
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match *self
            .segment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            None => raw,
            Some(segment) => Some((
                (segment.before + raw.map_or(0., |(done, _)| done.min(segment.length)))
                    .min(segment.total),
                segment.total,
            )),
        }
    }
    /// A join's current part and part count (1-based).
    pub fn part(&self) -> Option<(usize, usize)> {
        self.segment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .map(|segment| (segment.part, segment.parts))
    }
    /// A join moves to `segment` (its own progress starts unknown); `None`
    /// ends the mapping.
    pub(super) fn set_segment(&self, segment: Option<Segment>) {
        let mut slot = self
            .segment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *self
            .progress
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        *slot = segment;
    }
    /// Never moves back for the same `total` (FFmpeg's `out_time` can dip,
    /// for example around B-frame reordering at the start).
    pub(super) fn set_progress(&self, done: f64, total: f64) {
        let mut progress = self
            .progress
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let done = match *progress {
            Some((old, t)) if t == total => old.max(done),
            _ => done,
        };
        *progress = Some((done.clamp(0., total), total));
    }
    pub fn facts(&self) -> serde_json::Value {
        self.facts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
    pub(super) fn record_facts(&self, facts: serde_json::Value) {
        *self
            .facts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = facts;
    }
    pub fn status(&self) -> Status {
        match self.state.load(Ordering::Acquire) {
            0 => Status::Queued,
            1 => Status::Running,
            2 | CANCELLED_BEFORE_START => Status::Cancelling,
            3 => Status::Publishing,
            4 => Status::Completed,
            5 => Status::Failed,
            6 => Status::Cancelled,
            _ => unreachable!("private worker state is invalid"),
        }
    }
    /// A coherent terminal status/result pair. Publishing remains observable
    /// after a publication error; it must not revert to "not started" on failure.
    pub fn snapshot(&self) -> (Status, Option<Outcome>, bool) {
        let result = self
            .result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let status = self.status();
        let published = self.publication_started.load(Ordering::Acquire)
            || matches!(status, Status::Publishing | Status::Completed);
        (status, result.clone(), published)
    }
    pub fn cancel(&self) -> CancelResult {
        loop {
            let current = self.status();
            match current {
                Status::Queued | Status::Running => {
                    if self
                        .state
                        .compare_exchange(
                            current as u8,
                            if current == Status::Queued {
                                CANCELLED_BEFORE_START
                            } else {
                                Status::Cancelling as u8
                            },
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_ok()
                    {
                        return CancelResult::Requested;
                    }
                }
                Status::Cancelling => return CancelResult::Requested,
                Status::Publishing => return CancelResult::TooLate,
                Status::Completed | Status::Failed | Status::Cancelled => {
                    return CancelResult::Terminal;
                }
            }
        }
    }
    /// Registry panic recovery only: call after the worker has actually stopped
    /// and its child/lease cleanup has unwound. Never overwrite a terminal result.
    #[cfg(test)]
    fn fail(&self, message: String) {
        self.finish(Outcome::Failed {
            code: "export-failed",
            message,
        });
    }
    fn start(&self) -> bool {
        self.state
            .compare_exchange(
                Status::Queued as u8,
                Status::Running as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }
    pub(super) fn publish(&self) -> bool {
        let won = self
            .state
            .compare_exchange(
                Status::Running as u8,
                Status::Publishing as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok();
        if won {
            self.publication_started.store(true, Ordering::Release);
        }
        won
    }
    fn finish(&self, outcome: Outcome) {
        let mut result = self
            .result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if result.is_some() {
            return;
        }
        let status = match &outcome {
            Outcome::Completed { .. } => Status::Completed,
            Outcome::Failed { .. } => Status::Failed,
            Outcome::Cancelled => Status::Cancelled,
        };
        *result = Some(outcome);
        self.state.store(status as u8, Ordering::Release);
    }
}

/// One run owns this Control. Duplicate starts cannot launch another encoder.
/// Accepted cancellation prevents publication; cancellation during Publishing is
/// TooLate. A Failed result can still report a cleanup/OS error after cancel.
pub fn run(request: RenderRequest, control: Arc<Control>) {
    run_with(&control, |control| encode(&request, control));
}

/// A failure with its own outcome code (`source-missing`, `source-changed`).
#[derive(Debug)]
pub struct Coded(pub &'static str, pub String);
impl std::fmt::Display for Coded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.1)
    }
}
impl std::error::Error for Coded {}

/// Runs one job's `work` under its Control (see [`run`]): a [`Refused`]
/// error fails `unsupported`, a [`Coded`] one with its code, others
/// `export-failed`.
pub(super) fn run_with(control: &Control, work: impl FnOnce(&Control) -> Result<Outcome>) {
    if !control.start() {
        if control.state.load(Ordering::Acquire) == CANCELLED_BEFORE_START {
            control.finish(Outcome::Cancelled);
        }
        return;
    }
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(control))) {
        Ok(Ok(outcome)) => control.finish(outcome),
        Ok(Err(error)) => control.finish(Outcome::Failed {
            code: if error.downcast_ref::<Refused>().is_some() {
                "unsupported"
            } else if let Some(coded) = error.downcast_ref::<Coded>() {
                coded.0
            } else {
                "export-failed"
            },
            message: format!("{error:#}"),
        }),
        Err(_) => control.finish(Outcome::Failed {
            code: "export-failed",
            message: "offline media worker panicked".into(),
        }),
    }
}

fn acquire_reader<'a>(lease: &'a Lease, control: &Control) -> Result<Option<Reader<'a>>> {
    loop {
        if control.status() == Status::Cancelling {
            return Ok(None);
        }
        if let Some(reader) = lease.try_reader()? {
            return Ok(Some(reader));
        }
        thread::sleep(POLL);
    }
}

pub(super) struct ReapedChild {
    pub(super) child: Child,
    pub(super) reaped: bool,
}
impl ReapedChild {
    pub(super) fn wait(&mut self) -> io::Result<ExitStatus> {
        let status = self.child.wait()?;
        self.reaped = true;
        Ok(status)
    }
    pub(super) fn kill_wait(&mut self) -> io::Result<ExitStatus> {
        // It may have exited between the cancellation check and kill(). wait()
        // still reaps it; no early return on the benign already-exited kill error.
        let _ = self.child.kill();
        self.wait()
    }
}
impl Drop for ReapedChild {
    fn drop(&mut self) {
        if !self.reaped
            && let Err(error) = self.kill_wait()
        {
            diag!("media worker could not reap child: {error}");
        }
    }
}

pub(super) enum ChildResult {
    Exited(ExitStatus),
    Cancelled,
}
pub(super) fn poll_child(child: &mut ReapedChild, control: &Control) -> Result<ChildResult> {
    loop {
        if control.status() == Status::Cancelling {
            child
                .kill_wait()
                .context("cancelled encoder could not be reaped")?;
            return Ok(ChildResult::Cancelled);
        }
        match child.child.try_wait() {
            Ok(Some(_)) => {
                return Ok(ChildResult::Exited(
                    child.wait().context("encoder wait failed")?,
                ));
            }
            Ok(None) => thread::sleep(POLL),
            Err(error) => {
                child
                    .kill_wait()
                    .context("encoder polling failed and cleanup could not reap it")?;
                return Err(error).context("encoder polling failed");
            }
        }
    }
}

pub(super) fn drain_tail(mut reader: impl Read) -> io::Result<Vec<u8>> {
    let mut tail = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        let count = match reader.read(&mut chunk) {
            Ok(0) => return Ok(tail),
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        let discard = (tail.len() + count).saturating_sub(STDERR_TAIL);
        if discard > 0 {
            tail.drain(..discard);
        }
        tail.extend_from_slice(&chunk[..count]);
    }
}

/// Re-encode a validated PNG as JPEG (RGB, the given quality) or BMP (24-bit
/// RGB, pixel for pixel). Alpha is dropped; the encoder writes RGB24 anyway.
pub fn convert_still(png: &std::path::Path, still: Still, out: &std::fs::File) -> Result<()> {
    use image::ImageEncoder;
    use std::io::Write;
    let mut reader = image::ImageReader::open(png).context("PNG could not be reopened")?;
    reader.set_format(image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(PNG_BYTES as u64 * 2);
    reader.limits(limits);
    let pixels = reader
        .decode()
        .context("PNG could not be decoded for conversion")?
        .to_rgb8();
    let (width, height) = pixels.dimensions();
    let mut writer = io::BufWriter::new(out);
    match still {
        Still::Png => bail!("PNG needs no conversion"),
        Still::Jpeg { quality } => {
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut writer, quality.clamp(1, 100))
                .write_image(&pixels, width, height, image::ExtendedColorType::Rgb8)
                .context("JPEG encoding failed")?
        }
        Still::Bmp => image::codecs::bmp::BmpEncoder::new(&mut writer)
            .write_image(&pixels, width, height, image::ExtendedColorType::Rgb8)
            .context("BMP encoding failed")?,
    }
    writer
        .flush()
        .context("converted image could not be written")?;
    Ok(())
}

fn validate_png(temp: &tempfile::NamedTempFile) -> Result<()> {
    let file = temp
        .reopen()
        .context("temporary PNG could not be reopened")?;
    let decoder =
        png::Decoder::new_with_limits(BufReader::new(file), png::Limits { bytes: PNG_BYTES });
    let mut reader = decoder
        .read_info()
        .context("encoder output is not a valid PNG")?;
    if reader.info().animation_control.is_some() {
        bail!("raw PNG output unexpectedly contains animation");
    }
    let bytes = reader
        .output_buffer_size()
        .filter(|bytes| *bytes <= PNG_BYTES)
        .context("PNG exceeds the 128 MiB decoded-frame budget")?;
    let mut pixels = vec![0; bytes];
    reader
        .next_frame(&mut pixels)
        .context("PNG pixel data could not be fully decoded")?;
    reader.finish().context("PNG end/chunk validation failed")?;
    Ok(())
}

pub(super) fn capture_command(
    command: &mut Command,
    control: &Control,
) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
    capture_input_command(command, Stdio::null(), control)
}
pub(super) fn capture_input_command(
    command: &mut Command,
    stdin: Stdio,
    control: &Control,
) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
    if control.status() == Status::Cancelling {
        return Ok(None);
    }
    let child = command
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("media diagnostic command could not start")?;
    let mut child = ReapedChild {
        child,
        reaped: false,
    };
    let stdout = child
        .child
        .stdout
        .take()
        .context("diagnostic stdout unavailable")?;
    let stderr = child
        .child
        .stderr
        .take()
        .context("diagnostic stderr unavailable")?;
    let out = thread::Builder::new()
        .name("media-diagnostic-out".into())
        .spawn(move || drain_tail(stdout));
    let out = match out {
        Ok(thread) => thread,
        Err(error) => {
            child.kill_wait()?;
            return Err(error).context("diagnostic stdout drain startup");
        }
    };
    let err = thread::Builder::new()
        .name("media-diagnostic-err".into())
        .spawn(move || drain_tail(stderr));
    let err = match err {
        Ok(thread) => thread,
        Err(error) => {
            child.kill_wait()?;
            let _ = out.join();
            return Err(error).context("diagnostic stderr drain startup");
        }
    };
    let result = poll_child(&mut child, control);
    if !child.reaped {
        child.kill_wait()?;
    }
    let stdout = out
        .join()
        .map_err(|_| anyhow::anyhow!("diagnostic stdout drain panic"))??;
    let stderr = err
        .join()
        .map_err(|_| anyhow::anyhow!("diagnostic stderr drain panic"))??;
    match result? {
        ChildResult::Cancelled => Ok(None),
        ChildResult::Exited(status) if status.success() => Ok(Some((stdout, stderr))),
        ChildResult::Exited(status) => bail!(
            "media diagnostic failed ({status}): {}",
            String::from_utf8_lossy(&stderr)
        ),
    }
}

/// Run an FFmpeg child to its end on this worker, reaped on every path. Its
/// stderr keeps a bounded tail. With `total`, the command writes `-progress
/// pipe:1` (FFmpeg's `key=value` lines) to stdout and each positive
/// `out_time_us` moves [`Control::progress`] toward `total` seconds (it stays
/// `None` until the first frame is written). `Ok(None)`: cancelled.
pub(super) fn run_tool(
    command: &mut Command,
    stdin: Stdio,
    control: &Control,
    total: Option<f64>,
) -> Result<Option<(ExitStatus, Vec<u8>)>> {
    if control.status() == Status::Cancelling {
        return Ok(None);
    }
    let child = command
        .stdin(stdin)
        .stdout(if total.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::piped())
        .spawn()
        .context("FFmpeg could not be started")?;
    let mut child = ReapedChild {
        child,
        reaped: false,
    };
    let stderr = child
        .child
        .stderr
        .take()
        .context("FFmpeg stderr pipe is unavailable")?;
    let drain = match thread::Builder::new()
        .name("media-stderr".into())
        .spawn(move || drain_tail(stderr))
    {
        Ok(drain) => drain,
        Err(error) => {
            child
                .kill_wait()
                .context("FFmpeg cleanup after stderr worker failure failed")?;
            return Err(error).context("FFmpeg stderr drain could not be started");
        }
    };
    let stdout = child.child.stdout.take();
    let waited = thread::scope(|scope| {
        let reader = match (stdout, total) {
            (Some(stdout), Some(total)) => {
                match thread::Builder::new()
                    .name("media-progress".into())
                    .spawn_scoped(scope, move || read_progress(stdout, control, total))
                {
                    Ok(reader) => Some(reader),
                    Err(error) => {
                        let _ = child.kill_wait();
                        return Err(error).context("FFmpeg progress reader could not be started");
                    }
                }
            }
            _ => None,
        };
        let waited = poll_child(&mut child, control);
        // Keep the child guard (and the caller's source Reader) until it is
        // reaped, on cancellation and error paths too.
        let reaped = if child.reaped {
            Ok(())
        } else {
            child.kill_wait().map(|_| ())
        };
        if let Some(reader) = reader {
            let _ = reader.join();
        }
        reaped.context("FFmpeg cleanup could not reap the child")?;
        waited
    });
    let stderr = drain
        .join()
        .map_err(|_| anyhow::anyhow!("FFmpeg stderr drain panicked"))?
        .context("FFmpeg stderr drain failed")?;
    match waited? {
        ChildResult::Cancelled => Ok(None),
        ChildResult::Exited(status) => Ok(Some((status, stderr))),
    }
}

/// Read FFmpeg's `-progress` stream to its end (stopping early would close the
/// pipe under FFmpeg). Only `out_time_us` is used; `N/A` and negative values
/// are skipped.
fn read_progress(stdout: impl Read, control: &Control, total: f64) {
    use std::io::BufRead;
    let mut reader = BufReader::new(stdout);
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        // Unknown (`null`) until the first frame is written: before it FFmpeg
        // decodes up to the start (from a keyframe, or from 0 s for an effects
        // chain) and reports 0, which would read as a stuck bar.
        if let Some(done) =
            progress_seconds(&String::from_utf8_lossy(&line)).filter(|done| *done > 0.)
        {
            control.set_progress(done, total);
        }
    }
}
fn progress_seconds(line: &str) -> Option<f64> {
    let micros = line
        .trim()
        .strip_prefix("out_time_us=")?
        .parse::<i64>()
        .ok()
        .filter(|us| *us >= 0)?;
    Some(micros as f64 / 1_000_000.)
}

/// Decode the written clip before publication, on this cancellable worker:
/// encoder success and header bytes alone do not establish a readable file.
/// `limit`: decode only the first seconds (a stream copy of decodable source
/// packets; its muxing is what is checked). `Ok(None)`: cancelled.
pub(super) fn decode_check(
    ffmpeg: &std::path::Path,
    path: &std::path::Path,
    threads: usize,
    limit: Option<f64>,
    control: &Control,
) -> Result<Option<()>> {
    let threads = threads.to_string();
    let mut command = crate::platform::background_tool_command(ffmpeg);
    command.args(["-nostdin", "-v", "error", "-threads", &threads]);
    if let Some(limit) = limit {
        command.args(["-t", &limit.to_string()]);
    }
    command.arg("-i").arg(path).args([
        "-map", "0:v:0", "-map", "0:a?", "-threads", &threads, "-f", "null", "-",
    ]);
    match run_tool(&mut command, Stdio::null(), control, None)? {
        None => Ok(None),
        Some((status, _)) if status.success() => Ok(Some(())),
        Some((status, stderr)) => bail!(
            "validation decode failed ({status}): {}",
            String::from_utf8_lossy(&stderr)
        ),
    }
}

/// Returns the encoded video's duration; `None`: cancelled.
fn validate_mp4(
    request: &RenderRequest,
    temp: &tempfile::NamedTempFile,
    threads: usize,
    control: &Control,
) -> Result<Option<f64>> {
    // Decode every encoded stream to completion before publication.
    // ffprobe/independent consumers verify codec/time.
    if decode_check(&request.ffmpeg, temp.path(), threads, None, control)?.is_none() {
        return Ok(None);
    }
    let probe = sibling(&request.ffmpeg, "ffprobe")?;
    let mut command = crate::platform::tool_command(probe);
    command.args(["-v","error","-show_entries","stream=index,codec_name,codec_type,width,height,sample_rate,channels,channel_layout,duration,r_frame_rate:format=duration","-of","json"]).arg(temp.path());
    let Some((stdout, _)) = capture_command(&mut command, control)? else {
        return Ok(None);
    };
    let output: serde_json::Value =
        serde_json::from_slice(&stdout).context("encoded MP4 probe metadata invalid")?;
    // Retain actual video/audio track facts even when coverage rejects publication.
    let mut facts = control.facts();
    facts["output"] = output.clone();
    control.record_facts(facts);
    let end = request.output.end().context("MP4 end missing")?;
    let video = output["streams"]
        .as_array()
        .and_then(|streams| streams.iter().find(|s| s["codec_type"] == "video"))
        .context("encoded MP4 video stream missing")?;
    let actual = video["duration"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.)
        .context("encoded MP4 video duration unavailable")?;
    let rate =
        rate(&video["r_frame_rate"]).context("encoded MP4 video frame interval unavailable")?;
    let tolerance = 1. / rate + 0.0001;
    if actual + tolerance < end - request.time {
        bail!(
            "source-unavailable: encoded video covers {actual}s, shorter than requested {}s beyond one frame ({tolerance}s)",
            end - request.time
        );
    }
    Ok(Some(actual))
}

/// A positive frame rate from ffprobe's `"n/d"`.
pub(super) fn rate(value: &serde_json::Value) -> Option<f64> {
    value
        .as_str()
        .and_then(|r| r.split_once('/'))
        .and_then(|(n, d)| Some(n.parse::<f64>().ok()? / d.parse::<f64>().ok()?))
        .filter(|r| r.is_finite() && *r > 0.)
}

/// FFmpeg's sibling tool (`ffprobe`, with `.exe` on Windows).
pub(super) fn sibling(ffmpeg: &std::path::Path, tool: &str) -> Result<std::path::PathBuf> {
    let directory = ffmpeg.parent().context("FFmpeg directory missing")?;
    Ok(directory.join(format!("{tool}{}", std::env::consts::EXE_SUFFIX)))
}

fn encode(request: &RenderRequest, control: &Control) -> Result<Outcome> {
    if !request.time.is_finite() || request.time < 0.0 {
        bail!("raw PNG time must be finite and nonnegative");
    }
    let Some(mut reader) = acquire_reader(&request.lease, control)? else {
        return Ok(Outcome::Cancelled);
    };
    let Some((version, _)) = capture_command(
        crate::platform::tool_command(&request.ffmpeg).arg("-version"),
        control,
    )?
    else {
        return Ok(Outcome::Cancelled);
    };
    let version = String::from_utf8(version).context("FFmpeg version is not UTF8")?;
    let ffmpeg_version = version
        .lines()
        .find(|s| s.starts_with("ffmpeg version "))
        .context("FFmpeg version missing")?;
    let libavfilter = version
        .lines()
        .find(|s| s.trim_start().starts_with("libavfilter"))
        .context("libavfilter version missing")?;
    let copy = matches!(&request.output, Output::Copy { .. });
    control.record_facts(serde_json::json!({"descriptorVersion":"vestra-finite/1","ffmpegVersion":ffmpeg_version,"libavfilterVersion":libavfilter.trim(),"videoStreamIndex":request.video_stream,"range":{"start":request.time,"end":request.output.end(),"interval":if copy {"stream copy; first video packet is the keyframe at or before start"} else {"half-open; first video PTS >= start"}},"preroll":null,"hasAudio":request.output.has_audio(),"audioTiming":if copy && request.output.has_audio() {"stream copy: source packets from the video start, unchanged"} else if request.output.has_audio() {"DSP source0 -> signed PTS shift -> 48kHz resample first_pts=0 -> zero pad -> range trim; device gain/mute excluded"}else{"not applicable: no audio output"},"processed":request.processed.is_some()}));
    reader
        .validate()
        .context("source-unavailable before encoding")?;
    if control.status() == Status::Cancelling {
        return Ok(Outcome::Cancelled);
    }
    let probe = sibling(&request.ffmpeg, "ffprobe")?;
    let mut origin_command = crate::platform::tool_command(probe);
    origin_command.args([
        "-v",
        "error",
        "-show_entries",
        if matches!(&request.output, Output::Still(_)) {
            "format=start_time,format_name"
        } else {
            // Clips also need the streams: the codecs and time bases of a
            // copy, the frame rate of a capped re-encode.
            "format=start_time,format_name,bit_rate:stream=index,codec_type,codec_name,time_base,avg_frame_rate,r_frame_rate,bit_rate:stream_disposition=attached_pic"
        },
        "-of",
        "json",
        crate::platform::HELD_FILE_INPUT,
    ]);
    let Some((probed, _)) = capture_input_command(&mut origin_command, reader.stdin()?, control)?
    else {
        return Ok(Outcome::Cancelled);
    };
    let probed: serde_json::Value =
        serde_json::from_slice(&probed).context("source origin probe JSON invalid")?;
    let origin = probed["format"]["start_time"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|v| v.is_finite());
    let valid = origin.is_some_and(|v| (v - request.source_origin).abs() <= 0.000001);
    let mut facts = control.facts();
    facts["sourceTimeline"] = serde_json::json!({"domain":"mpv-media","validation":if valid{"normalized-origin-matched"}else{"unsupported-file-origin"},"containerStartTime":origin,"expectedContainerStartTime":request.source_origin});
    control.record_facts(facts);
    if !valid {
        bail!(
            "unsupported: held source file origin {:?} does not match frozen normalized playback origin {}",
            origin,
            request.source_origin
        );
    }
    reader
        .validate()
        .context("source-unavailable after origin verification")?;
    let seek = seek_plan(
        request,
        probed["format"]["format_name"].as_str().unwrap_or(""),
    );
    let mut facts = control.facts();
    facts["preroll"] = match seek {
        Some(seek) => seek.preroll.into(),
        None if !copy
            && (request.processed.is_some() || matches!(&request.output, Output::Mp4 { .. })) =>
        {
            crate::playback::offline_audio::PREROLL.into()
        }
        None => serde_json::Value::Null,
    };
    control.record_facts(facts);
    let filename = request
        .destination
        .file_name()
        .context("PNG destination needs a filename")?;
    let parent = request
        .destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    let parent = dunce::canonicalize(parent).context("PNG destination directory is unavailable")?;
    let destination = parent.join(filename);
    let temp = tempfile::NamedTempFile::new_in(&parent)
        .context("owned PNG temporary output could not be created")?;
    if let Output::Copy {
        end,
        audio,
        container,
    } = &request.output
    {
        let Some(copied) = crate::playback::media_copy::write(
            &crate::playback::media_copy::Job {
                ffmpeg: &request.ffmpeg,
                video_stream: request.video_stream,
                start: request.time,
                end: *end,
                audio: audio.as_ref(),
                container: *container,
                origin: origin.unwrap_or(request.source_origin),
                streams: &probed["streams"],
                destination: &destination,
            },
            &mut reader,
            temp.path(),
            control,
        )?
        else {
            return Ok(Outcome::Cancelled);
        };
        let destination = crate::playback::media_copy::named(&destination, copied.container);
        return publish(
            &reader,
            control,
            temp,
            destination,
            Some((copied.start, copied.end)),
        );
    }
    // Native extraction and video decoding share this OFFLINE reader only in
    // sequence. Each stdin rewind occurs after the earlier child is reaped.
    let native = if let Some(frozen) = &request.processed
        && let Some(subtitle) = &frozen.subtitle
        && let crate::playback::subtitle_backend::FrozenSubtitleInput::Native {
            stream_index,
            codec,
        } = &subtitle.input
    {
        let stdin = reader
            .stdin()
            .context("native subtitle source stdin could not be prepared")?;
        let Some(native) = crate::playback::native_subtitle::extract(
            &request.ffmpeg,
            stdin,
            *stream_index,
            subtitle.fonts.asset(),
            || control.status() == Status::Cancelling,
        )?
        else {
            return Ok(Outcome::Cancelled);
        };
        reader
            .validate()
            .context("source-unavailable after subtitle extraction")?;
        let mut facts = control.facts();
        facts["nativeSubtitle"] = serde_json::json!({"codec":codec,"streamIndex":stream_index,"canonical":{"format":"srt","digest":native.canonical_digest},"derived":{"format":"webvtt","digest":native.derived_digest},"cueCount":native.cue_count,"coverageValidated":true});
        control.record_facts(facts);
        Some(native)
    } else {
        None
    };
    let stdin = reader
        .stdin()
        .context("source-unavailable: offline stdin could not be prepared")?;
    let mut filter = if let Some(frozen) = &request.processed {
        let subtitle = frozen.subtitle.as_ref().map(|subtitle| {
            crate::playback::offline_descriptor::Subtitle {
                carrier: match &subtitle.input {
                    crate::playback::subtitle_backend::FrozenSubtitleInput::Carrier(carrier) => {
                        carrier.path()
                    }
                    crate::playback::subtitle_backend::FrozenSubtitleInput::Native { .. } => native
                        .as_ref()
                        .expect("native input was prepared above")
                        .carrier
                        .path(),
                },
                fonts_dir: subtitle.fonts.path(),
                family: subtitle.fonts.asset().metadata().family,
                style: &subtitle.style,
                display_width: frozen.width,
                display_height: frozen.height,
            }
        });
        match &request.output {
            Output::Still(_) => crate::playback::offline_descriptor::video(
                &frozen.processing.video,
                subtitle,
                request.time,
            )?,
            Output::Mp4 { end, .. } | Output::Copy { end, .. } => {
                crate::playback::offline_descriptor::video_range(
                    &frozen.processing.video,
                    subtitle,
                    request.time,
                    Some(*end),
                )?
            }
        }
    } else {
        "scale=w='max(1,round(iw*sar))':h=ih,setsar=1,settb=AVTB".into()
    };
    let threads = match &request.output {
        Output::Mp4 { encoding, .. } => encoding.threads(),
        _ => 1,
    }
    .to_string();
    let mut command = crate::platform::background_tool_command(&request.ffmpeg);
    command.args([
        "-nostdin",
        "-v",
        "error",
        "-threads",
        &threads,
        "-filter_threads",
        &threads,
        "-filter_complex_threads",
        &threads,
    ]);
    if request.processed.is_none() && matches!(&request.output, Output::Still(_)) {
        command.args(["-ss", &request.time.to_string()]);
    }
    if let Some(seek) = seek {
        // No state to warm up beyond `seek.at`: seek the held file (FFmpeg
        // decodes from the keyframe before it) instead of decoding from 0 s.
        // `-copyts -start_at_zero` keep the normalized source timeline, so
        // the trims, the subtitle times and the audio graph below are
        // unchanged and the frames and samples are those of decoding from
        // 0 s.
        command.args(["-ss", &seek.at.to_string(), "-copyts", "-start_at_zero"]);
    }
    command.args(["-i", crate::platform::HELD_FILE_INPUT]);
    let mut total = None;
    match &request.output {
        Output::Still(_) => {
            command.args(["-map", &format!("0:{}", request.video_stream)]);
            command.args([
                "-frames:v",
                "1",
                "-an",
                "-vf",
                &filter,
                "-pix_fmt",
                "rgb24",
                "-c:v",
                "png",
                "-threads",
                "1",
                "-f",
                "image2",
                "-update",
                "1",
                "-y",
            ]);
        }
        Output::Copy { .. } => unreachable!("stream copies return above"),
        Output::Mp4 {
            end,
            audio,
            encoding,
        } => {
            let duration = end - request.time;
            if !end.is_finite() || duration <= 0. || *end > 86400. {
                bail!("invalid frozen MP4 range");
            }
            if request.processed.is_none() {
                filter.push_str(&format!(
                    ",trim=start={}:end={end},setpts=PTS-STARTPTS",
                    request.time
                ));
            }
            let source_rate = probed["streams"]
                .as_array()
                .and_then(|streams| {
                    streams
                        .iter()
                        .find(|s| s["index"].as_u64() == Some(u64::from(request.video_stream)))
                })
                .and_then(|video| rate(&video["avg_frame_rate"]).or(rate(&video["r_frame_rate"])));
            filter.push_str(&caps(encoding, source_rate));
            // H.264 yuv420 output needs even dimensions. Preserve the complete
            // presented image and pad only an odd last row/column with black.
            filter.push_str(",pad=ceil(iw/2)*2:ceil(ih/2)*2:color=black");
            let mut graph = format!("[0:{}]{filter}[v]", request.video_stream);
            if let Some(audio) = audio {
                let (resources, offset) = request
                    .processed
                    .as_ref()
                    .map(|p| (p.processing.audio.as_slice(), p.processing.audio_offset_ms))
                    .unwrap_or((&[], 0));
                let af = crate::playback::offline_audio::audio_from(
                    resources,
                    offset,
                    request.time,
                    *end,
                    audio_base(seek, offset),
                )?;
                let stream = audio
                    .stream_index
                    .context("frozen audio stream mapping unavailable")?;
                graph.push_str(&format!(";[0:{stream}]{af}[a]"));
            }
            command.args(["-filter_complex", &graph, "-map", "[v]"]);
            if audio.is_some() {
                command.args([
                    "-map",
                    "[a]",
                    "-c:a",
                    "aac",
                    "-ar",
                    "48000",
                    "-b:a",
                    &format!("{}k", encoding.audio_bitrate()),
                ]);
            } else {
                command.arg("-an");
            }
            match encoding.video_bitrate {
                None => command.args(["-crf", &encoding.crf().to_string()]),
                Some(VideoBitrate::Source) => {
                    let bits = source_bitrate(&probed, request.video_stream).ok_or_else(|| {
                        Refused("unsupported: the source's video bitrate is unknown".into())
                    })?;
                    command.args([
                        "-b:v",
                        &bits.to_string(),
                        "-maxrate",
                        &(bits * 3 / 2).to_string(),
                        "-bufsize",
                        &(bits * 2).to_string(),
                    ])
                }
            };
            command.args([
                "-t",
                &duration.to_string(),
                "-c:v",
                "libx264",
                "-preset",
                encoding.preset(),
                "-pix_fmt",
                "yuv420p",
                "-threads",
                &threads,
                "-movflags",
                "+faststart",
                "-progress",
                "pipe:1",
                "-stats_period",
                "0.25",
                "-f",
                "mp4",
                "-y",
            ]);
            total = Some(duration);
        }
    }
    command.arg(temp.path());
    // The source Reader stays held until the child is reaped (run_tool).
    match run_tool(&mut command, stdin, control, total)? {
        None => return Ok(Outcome::Cancelled),
        Some((status, stderr)) if !status.success() => bail!(
            "encoder failed ({status}): {}",
            String::from_utf8_lossy(&stderr)
        ),
        Some(_) => {}
    }
    if control.status() == Status::Cancelling {
        return Ok(Outcome::Cancelled);
    }
    let (temp, range) = match &request.output {
        Output::Still(still) => {
            validate_png(&temp)?;
            if *still == Still::Png {
                (temp, None)
            } else {
                // The converted file replaces the PNG (removed when dropped).
                let converted = tempfile::NamedTempFile::new_in(&parent)
                    .context("owned temporary output could not be created")?;
                convert_still(temp.path(), *still, converted.as_file())?;
                (converted, None)
            }
        }
        Output::Mp4 { encoding, .. } => {
            let Some(actual) = validate_mp4(request, &temp, encoding.threads(), control)? else {
                return Ok(Outcome::Cancelled);
            };
            (temp, Some((request.time, request.time + actual)))
        }
        Output::Copy { .. } => unreachable!("stream copies return above"),
    };
    publish(&reader, control, temp, destination, range)
}

/// The source video's average bitrate in bit/s: the stream's own, else the
/// container's minus the other streams' known rates (at least 100 kb/s).
pub(super) fn source_bitrate(probed: &serde_json::Value, video_stream: u32) -> Option<u64> {
    let bits = |v: &serde_json::Value| {
        v["bit_rate"]
            .as_str()
            .and_then(|b| b.parse::<u64>().ok())
            .filter(|b| *b > 0)
    };
    let streams = probed["streams"].as_array()?;
    let video = streams
        .iter()
        .find(|s| s["index"].as_u64() == Some(u64::from(video_stream)))?;
    bits(video).or_else(|| {
        let others: u64 = streams
            .iter()
            .filter(|s| s["index"] != video["index"])
            .filter_map(bits)
            .sum();
        Some(bits(&probed["format"])?.saturating_sub(others).max(100_000))
    })
}

/// History an export keeps before its start when the chain has stateful
/// nodes (`effects::History::Bounded`): 10 s, from the keyframe before it.
const HISTORY_SECONDS: f64 = 10.;

/// Where an export starts decoding, when not at 0 s.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Seek {
    /// Normalized source seconds; FFmpeg decodes from the keyframe before.
    at: f64,
    /// `facts.preroll`: `keyframe-before-start` or `bounded-history`.
    preroll: &'static str,
}

/// Audio decoded before the seek point when the export has audio: an AAC or
/// MP3 decoder started cold writes different first samples (priming).
pub(super) const AUDIO_WARMUP_SECONDS: f64 = 1.;

/// Containers whose input seek lands on the keyframe at or before the target
/// (an index: MP4/MOV, Matroska/WebM). Others (MPEG-TS, raw streams) seek by
/// estimate and can land after it, losing frames: they decode from 0 s.
pub(super) fn seeks_exactly(format_name: &str) -> bool {
    format_name
        .split(',')
        .any(|name| matches!(name, "mov" | "mp4" | "matroska" | "webm"))
}

/// The input seek of a frame or a re-encoded clip that needs no history from
/// 0 s; `None` keeps decoding from 0 s (`offline_audio::PREROLL`). A raw
/// still keeps its own plain `-ss`; a stream copy seeks in `media_copy`.
/// With effects (user ruling 2026-11-07, by effect type): a stateless chain
/// seeks to the start, a chain with stateful nodes keeps `HISTORY_SECONDS`
/// before it, a frameBlend chain decodes from 0 s. Audio needs
/// `AUDIO_WARMUP_SECONDS` more, and a positive audio offset (audio played
/// later) the source that much earlier. Only in containers that seek
/// exactly (`seeks_exactly`); a seek that would reach 0 s is no seek.
fn seek_plan(request: &RenderRequest, format_name: &str) -> Option<Seek> {
    if request.processed.is_none() && matches!(&request.output, Output::Still(_))
        || matches!(&request.output, Output::Copy { .. })
        || !seeks_exactly(format_name)
    {
        return None;
    }
    let (window, preroll, offset_ms) = match &request.processed {
        None => (0., SEEK_PREROLL, 0),
        Some(frozen) => {
            let processing = &frozen.processing;
            let history = processing
                .video
                .iter()
                .flat_map(|r| {
                    r.nodes
                        .iter()
                        .map(crate::playback::effects::VideoNode::history)
                })
                .chain(processing.audio.iter().flat_map(|r| {
                    r.nodes
                        .iter()
                        .map(crate::playback::effects::AudioNode::history)
                }))
                .max()
                .unwrap_or(crate::playback::effects::History::None);
            match history {
                crate::playback::effects::History::None => {
                    (0., SEEK_PREROLL, processing.audio_offset_ms)
                }
                crate::playback::effects::History::Bounded => {
                    (HISTORY_SECONDS, BOUNDED_PREROLL, processing.audio_offset_ms)
                }
                crate::playback::effects::History::FromZero => return None,
            }
        }
    };
    let audio = if request.output.has_audio() {
        AUDIO_WARMUP_SECONDS + f64::from(offset_ms.max(0)) / 1000.
    } else {
        0.
    };
    // Whole seconds: a sample boundary at every integer rate (`audio_base`).
    let at = (request.time - window - audio).floor();
    (at > 0.).then_some(Seek { at, preroll })
}

/// Where the audio graph's timeline starts after an input seek
/// (`offline_audio::audio_from`): the seek point shifted by a negative audio
/// offset (audio played earlier), in whole seconds, rounded down so it is
/// never after the first decoded sample; 0 without a seek.
fn audio_base(seek: Option<Seek>, offset_ms: i32) -> f64 {
    seek.map_or(0., |seek| {
        (seek.at + f64::from(offset_ms.min(0)) / 1000.)
            .floor()
            .max(0.)
    })
}

/// The R6 ceilings as filters after the range trim: a frame-rate cap only
/// when the source is faster, and a height cap that leaves a shorter picture
/// alone (the width follows the aspect, kept even).
fn caps(encoding: &Encoding, source_rate: Option<f64>) -> String {
    let mut filter = String::new();
    if let Some(fps) = encoding.fps
        && source_rate.is_none_or(|rate| rate > f64::from(fps) * 1.001)
    {
        filter.push_str(&format!(",fps={fps}"));
    }
    if let Some(height) = encoding.max_height {
        filter.push_str(&format!(
            ",scale=w='if(gt(ih,{height}),-2,iw)':h='min(ih,{height})'"
        ));
    }
    filter
}

/// Publish the validated temporary output under `destination`, never over an
/// existing file. The CAS is the publication/cancellation boundary: no
/// filesystem publish is attempted if a cancellation request has already won.
fn publish(
    reader: &Reader<'_>,
    control: &Control,
    temp: tempfile::NamedTempFile,
    destination: PathBuf,
    range: Option<(f64, f64)>,
) -> Result<Outcome> {
    if control.status() == Status::Cancelling {
        return Ok(Outcome::Cancelled);
    }
    reader
        .validate()
        .context("source-unavailable after encoding")?;
    if !control.publish() {
        return Ok(Outcome::Cancelled);
    }
    temp.as_file()
        .sync_all()
        .context("temporary output could not be flushed")?;
    temp.persist_noclobber(&destination)
        .map_err(|error| anyhow::anyhow!("no-clobber publication failed: {}", error.error))?;
    // FFmpeg's last report may stop short of the clip's length, and a clip
    // written before its first report has none at all.
    if let Some(total) = control
        .progress()
        .map(|(_, total)| total)
        .or(range.map(|(start, end)| end - start))
    {
        control.set_progress(total, total);
    }
    Ok(Outcome::Completed { destination, range })
}

/// Helpers of the real-FFmpeg export tests (`#[ignore]`d; `ci:local` runs
/// them with `--include-ignored`).
#[cfg(test)]
pub(super) mod real {
    use super::*;
    use std::time::Instant;

    /// `VESTRA_FFMPEG` (`env.ps1 -Ffmpeg`), else `ffmpeg` on PATH; FFprobe beside it.
    pub fn ffmpeg() -> PathBuf {
        std::env::var_os("VESTRA_FFMPEG").map_or_else(
            || {
                let name = if cfg!(windows) {
                    "ffmpeg.exe"
                } else {
                    "ffmpeg"
                };
                std::env::split_paths(&std::env::var_os("PATH").unwrap())
                    .map(|dir| dir.join(name))
                    .find(|path| path.is_file())
                    .expect("ffmpeg on PATH")
            },
            PathBuf::from,
        )
    }

    /// A clip of the test-media folder `VESTRA_TEST_MEDIA`. `None` (the test
    /// is skipped with a note) where the variable is not set or the clip
    /// does not exist.
    pub fn clip(name: &str) -> Option<PathBuf> {
        let Some(dir) = std::env::var_os("VESTRA_TEST_MEDIA").map(PathBuf::from) else {
            diag!("skipped: VESTRA_TEST_MEDIA is not set");
            return None;
        };
        let path = dir.join(name);
        if path.is_file() {
            Some(path)
        } else {
            diag!(
                "skipped: {} is missing (set VESTRA_TEST_MEDIA)",
                path.display()
            );
            None
        }
    }

    /// ffprobe's JSON for `entries` of `file`.
    pub fn probe(file: &std::path::Path, args: &[&str]) -> serde_json::Value {
        let out = Command::new(sibling(&ffmpeg(), "ffprobe").unwrap())
            .args(["-v", "error"])
            .args(args)
            .args(["-of", "json"])
            .arg(file)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }

    pub fn audio(stream_index: u32) -> crate::playback::media_tracks::AudioTrack {
        crate::playback::media_tracks::AudioTrack {
            track_id: 1,
            stream_index: Some(stream_index),
            selected: true,
            external: false,
            codec: None,
            title: None,
            language: None,
        }
    }

    pub fn request(
        source: &std::path::Path,
        time: f64,
        destination: PathBuf,
        output: Output,
    ) -> RenderRequest {
        RenderRequest {
            lease: Arc::new(Lease::open(source).unwrap()),
            time,
            destination,
            ffmpeg: ffmpeg(),
            processed: None,
            output,
            video_stream: 0,
            source_origin: 0.,
        }
    }

    /// Run a job on a worker thread as `media_jobs` does; the highest
    /// progress seen while it ran comes back with the control.
    pub fn run_job(request: RenderRequest) -> (Arc<Control>, Option<(f64, f64)>) {
        let control = Arc::new(Control::new());
        let worker = control.clone();
        let thread = thread::spawn(move || run(request, worker));
        let mut seen: Option<(f64, f64)> = None;
        let deadline = Instant::now() + Duration::from_secs(600);
        while !thread.is_finished() && Instant::now() < deadline {
            if let Some((done, total)) = control.progress()
                && seen.is_none_or(|(d, _)| done > d)
            {
                seen = Some((done, total));
            }
            thread::sleep(Duration::from_millis(5));
        }
        thread.join().unwrap();
        // A short job can finish between two polls.
        let seen = seen.or(control.progress());
        (control, seen)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{Barrier, mpsc},
        time::Instant,
    };

    /// Precise options: a 720p60 source capped to 480p and 30 fps, CRF 26,
    /// AAC 128 kb/s, with threads chosen by the host. The range written and
    /// the progress (seconds of the 2 s clip) are reported.
    #[test]
    #[ignore = "needs ffmpeg with libx264"]
    fn precise_options_cap_size_and_rate_and_report_the_range() {
        let ffmpeg = real::ffmpeg();
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.mp4");
        let made = Command::new(&ffmpeg)
            .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i"])
            .arg("testsrc2=size=1280x720:rate=60")
            .args(["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000"])
            .args(["-t", "4", "-c:v", "libx264", "-preset", "ultrafast"])
            .args(["-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
            .arg(&source)
            .status()
            .unwrap();
        assert!(made.success());
        let destination = dir.path().join("out").join("precise.mp4");
        std::fs::create_dir(destination.parent().unwrap()).unwrap();
        let encoding: Encoding = serde_json::from_value(serde_json::json!({"crf":26,"speed":"balanced","maxHeight":480,"fps":30,"audioBitrate":128,"threads":"auto"})).unwrap();
        let (control, progress) = real::run_job(real::request(
            &source,
            1.0,
            destination.clone(),
            Output::Mp4 {
                end: 3.0,
                audio: Some(real::audio(1)),
                encoding,
            },
        ));
        let Some(Outcome::Completed {
            destination: written,
            range: Some((start, end)),
        }) = control.snapshot().1
        else {
            panic!("{:?}", control.snapshot().1);
        };
        assert_eq!(written, dunce::canonicalize(&destination).unwrap());
        assert_eq!(start, 1.0);
        assert!((end - 3.0).abs() < 0.05, "end {end}");
        let (done, total) = progress.expect("progress while encoding");
        assert_eq!(total, 2.0);
        assert!(done > 0. && done <= total, "{done}/{total}");
        let output = real::probe(
            &destination,
            &[
                "-show_entries",
                "stream=codec_type,width,height,r_frame_rate,bit_rate",
            ],
        );
        let video = &output["streams"][0];
        assert_eq!(
            (video["width"].as_u64(), video["height"].as_u64()),
            (Some(854), Some(480))
        );
        assert_eq!(video["r_frame_rate"], "30/1");
        assert_eq!(output["streams"][1]["codec_type"], "audio");
    }

    /// Cancelling a running re-encode stops FFmpeg: the job ends cancelled,
    /// after progress was reported, and leaves no file behind.
    #[test]
    #[ignore = "needs ffmpeg with libx264 and the testmedia clips"]
    fn cancelling_a_running_encode_leaves_nothing() {
        let Some(source) = real::clip("bbb-480p-web.mp4") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("whole.mp4");
        let control = Arc::new(Control::new());
        let worker = control.clone();
        let request = real::request(
            &source,
            0.,
            destination.clone(),
            Output::Mp4 {
                end: 90.,
                audio: None,
                encoding: Encoding::default(),
            },
        );
        let thread = thread::spawn(move || run(request, worker));
        let deadline = Instant::now() + Duration::from_secs(60);
        while control.progress().is_none_or(|(done, _)| done <= 0.) {
            assert!(Instant::now() < deadline, "no progress");
            assert!(!thread.is_finished(), "{:?}", control.snapshot());
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(control.progress().unwrap().1, 90.);
        assert_eq!(control.cancel(), CancelResult::Requested);
        thread.join().unwrap();
        assert_eq!(control.snapshot().1, Some(Outcome::Cancelled));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn progress_reads_out_time_in_microseconds() {
        assert_eq!(progress_seconds("out_time_us=1500000\n"), Some(1.5));
        assert_eq!(progress_seconds("out_time_us=N/A"), None);
        assert_eq!(progress_seconds("out_time_us=-23"), None);
        assert_eq!(progress_seconds("out_time=00:00:01.500000"), None);
        let control = Control::new();
        read_progress(
            io::Cursor::new(b"frame=1\nout_time_us=N/A\nout_time_us=2000000\nprogress=continue\nout_time_us=9000000\nprogress=end\n".to_vec()),
            &control,
            5.,
        );
        // Clamped to the clip's length.
        assert_eq!(control.progress(), Some((5., 5.)));
        assert_eq!(Control::new().progress(), None);
        // Never backwards for the same clip.
        let dips = Control::new();
        read_progress(
            io::Cursor::new(b"out_time_us=2000000\nout_time_us=1500000\n".to_vec()),
            &dips,
            5.,
        );
        assert_eq!(dips.progress(), Some((2., 5.)));
        // While FFmpeg decodes up to the start it reports 0: still unknown.
        let preroll = Control::new();
        read_progress(
            io::Cursor::new(b"out_time_us=N/A\nout_time_us=0\nprogress=continue\n".to_vec()),
            &preroll,
            5.,
        );
        assert_eq!(preroll.progress(), None);
    }

    /// `ffmpeg … -f <format> -` of `file`'s first video or audio stream,
    /// decoded: per-frame md5 lines (time and hash) or raw PCM bytes.
    fn decoded(file: &std::path::Path, stream: &str, format: &str) -> Vec<u8> {
        let out = Command::new(real::ffmpeg())
            .args(["-v", "error", "-i"])
            .arg(file)
            .args(["-map", stream])
            .args(if format == "framemd5" {
                &["-f", "framemd5"][..]
            } else {
                &["-f", "s16le", "-ac", "2"][..]
            })
            .arg("-")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        if format == "framemd5" {
            // Keep presentation time and hash; drop the comment header.
            String::from_utf8(out.stdout)
                .unwrap()
                .lines()
                .filter(|l| !l.starts_with('#'))
                .map(|l| {
                    let f: Vec<_> = l.split(',').map(str::trim).collect();
                    format!("{} {}\n", f[2], f[5])
                })
                .collect::<String>()
                .into_bytes()
        } else {
            out.stdout
        }
    }

    /// A raw clip input-seeks to its start (`-ss -copyts -start_at_zero`)
    /// instead of decoding from 0 s and writes what decoding from 0 s
    /// writes. The MP4 starts at about 10 s (`-output_ts_offset`), so the
    /// normalized timeline is checked too. A TS of the same streams seeks by
    /// estimate, so it still decodes from 0 s.
    #[test]
    #[ignore = "needs ffmpeg with libx264"]
    fn a_raw_clip_seeks_and_writes_what_decoding_from_zero_writes() {
        let dir = tempfile::tempdir().unwrap();
        let noisy = noisy_source(dir.path());
        for (name, format, preroll) in [
            ("offset.mp4", "mp4", SEEK_PREROLL),
            (
                "offset.ts",
                "mpegts",
                crate::playback::offline_audio::PREROLL,
            ),
        ] {
            let source = dir.path().join(name);
            let made = Command::new(real::ffmpeg())
                .args(["-loglevel", "error", "-y", "-i"])
                .arg(&noisy)
                .args(["-c", "copy", "-output_ts_offset", "10", "-f", format])
                .arg(&source)
                .status()
                .unwrap();
            assert!(made.success());
            let origin: f64 =
                real::probe(&source, &["-show_entries", "format=start_time"])["format"]
                    ["start_time"]
                    .as_str()
                    .unwrap()
                    .parse()
                    .unwrap();
            assert!(origin > 9., "{name} starts at {origin} s");
            let out = dir.path().join(format);
            std::fs::create_dir(&out).unwrap();
            same_as_from_zero(&source, origin, (20.3, 21.9), None, preroll, &out, true);
        }
    }

    /// A testmedia clip (H.264 + AAC in MKV), 20.4–21.4 s: the same frames.
    /// Matroska times are whole milliseconds, so the resampler places the
    /// first sample after a seek up to half a millisecond from where a
    /// decode from 0 s puts it: the audio matches within that.
    #[test]
    #[ignore = "needs ffmpeg with libx264 and the testmedia clips"]
    fn a_raw_clip_of_a_real_file_seeks_to_the_same_frames() {
        let Some(source) = real::clip("[ryona]kof xiii Athena.mkv") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        same_as_from_zero(
            &source,
            0.,
            (20.4, 21.4),
            None,
            SEEK_PREROLL,
            dir.path(),
            false,
        );
    }

    /// Effects for [`same_as_from_zero`]: video and audio chains and the
    /// audio offset in ms.
    struct Effects {
        video: Vec<crate::playback::effects::VideoNode>,
        audio: Vec<crate::playback::effects::AudioNode>,
        offset_ms: i32,
    }

    /// Export `start..end` of `source` (video 0, audio 1) through the worker
    /// and compare it with decoding from 0 s: the reference runs the graph
    /// the worker builds without the input seek (one thread, so both
    /// encoders are deterministic). Frames (time and md5) and audio PCM must
    /// be the same; `preroll` is the expected `facts.preroll`.
    fn same_as_from_zero(
        source: &std::path::Path,
        origin: f64,
        (start, end): (f64, f64),
        effects: Option<Effects>,
        preroll: &str,
        dir: &std::path::Path,
        exact_audio: bool,
    ) {
        use crate::playback::effects::Resource;
        fn resource<N>(nodes: Vec<N>) -> Resource<N> {
            Resource {
                manifest_id: "test.plugin".into(),
                resource_id: "chain".into(),
                nodes,
            }
        }
        let (video_resources, audio_resources, offset_ms) = match &effects {
            Some(e) => (
                (!e.video.is_empty())
                    .then(|| resource(e.video.clone()))
                    .into_iter()
                    .collect(),
                (!e.audio.is_empty())
                    .then(|| resource(e.audio.clone()))
                    .into_iter()
                    .collect(),
                e.offset_ms,
            ),
            None => (Vec::new(), Vec::new(), 0),
        };
        let destination = dir.join("seek.mp4");
        let mut request = real::request(
            source,
            start,
            destination.clone(),
            Output::Mp4 {
                end,
                audio: Some(real::audio(1)),
                encoding: Encoding::default(),
            },
        );
        request.source_origin = origin;
        if effects.is_some() {
            request.processed = Some(Arc::new(crate::playback::media_jobs::FrozenProcessed {
                processing: crate::playback::processing::FrozenProcessing {
                    revision: 1,
                    video: video_resources.clone(),
                    audio: audio_resources.clone(),
                    audio_offset_ms: offset_ms,
                },
                subtitle: None,
                width: 320,
                height: 240,
                audio_delay_ms: f64::from(offset_ms),
            }));
        }
        let (control, progress) = real::run_job(request);
        assert!(
            matches!(control.snapshot().1, Some(Outcome::Completed { .. })),
            "{:?}",
            control.snapshot().1
        );
        assert_eq!(control.facts()["preroll"], preroll);
        assert!(
            control
                .progress()
                .is_some_and(|(done, total)| done == total)
        );
        assert!(progress.is_some_and(|(done, _)| done > 0.));
        // The reference: the graph the worker builds, decoded from 0 s.
        let filter = if effects.is_some() {
            crate::playback::offline_descriptor::video_range(
                &video_resources,
                None,
                start,
                Some(end),
            )
            .unwrap()
        } else {
            format!(
                "scale=w='max(1,round(iw*sar))':h=ih,setsar=1,settb=AVTB,trim=start={start}:end={end},setpts=PTS-STARTPTS"
            )
        };
        let video = format!("[0:0]{filter},pad=ceil(iw/2)*2:ceil(ih/2)*2:color=black[v]");
        let audio =
            crate::playback::offline_audio::audio(&audio_resources, offset_ms, start, end).unwrap();
        let reference = dir.join("reference.mp4");
        let made = Command::new(real::ffmpeg())
            .args([
                "-nostdin",
                "-v",
                "error",
                "-threads",
                "1",
                "-filter_threads",
                "1",
            ])
            .args(["-filter_complex_threads", "1", "-i"])
            .arg(source)
            .arg("-filter_complex")
            .arg(format!("{video};[0:1]{audio}[a]"))
            .args(["-map", "[v]", "-map", "[a]", "-c:a", "aac", "-ar", "48000"])
            .args(["-b:a", "192k", "-t", &(end - start).to_string()])
            .args(["-c:v", "libx264", "-preset", "veryfast", "-crf", "18"])
            .args([
                "-pix_fmt",
                "yuv420p",
                "-threads",
                "1",
                "-movflags",
                "+faststart",
            ])
            .args(["-f", "mp4", "-y"])
            .arg(&reference)
            .status()
            .unwrap();
        assert!(made.success());
        let frames = decoded(&destination, "0:v:0", "framemd5");
        assert!(!frames.is_empty());
        assert_eq!(
            frames,
            decoded(&reference, "0:v:0", "framemd5"),
            "video frames differ"
        );
        let pcm = decoded(&destination, "0:a:0", "pcm");
        assert!(pcm.len() > 100_000);
        let expected = decoded(&reference, "0:a:0", "pcm");
        if exact_audio {
            assert!(
                pcm == expected,
                "audio PCM differs: {:?}",
                alignment(&pcm, &expected)
            );
        } else {
            let (shift, correlation) = alignment(&pcm, &expected);
            assert!(
                shift.abs() <= 24 && correlation > 0.99,
                "audio off by {shift} samples (correlation {correlation})"
            );
        }
    }

    /// The best shift (samples, ±48) of `a` against `b` (16-bit stereo, left
    /// channel) and the normalized correlation there.
    fn alignment(a: &[u8], b: &[u8]) -> (i64, f64) {
        let left = |pcm: &[u8]| -> Vec<f64> {
            pcm.as_chunks::<4>()
                .0
                .iter()
                .map(|f| f64::from(i16::from_le_bytes([f[0], f[1]])))
                .collect()
        };
        let (a, b) = (left(a), left(b));
        assert_eq!(a.len(), b.len(), "audio lengths differ");
        let n = a.len() as i64;
        (-48..=48)
            .map(|shift: i64| {
                let (mut ab, mut aa, mut bb) = (0., 0., 0.);
                for i in 100..n - 100 {
                    let (x, y) = (a[i as usize], b[(i + shift) as usize]);
                    ab += x * y;
                    aa += x * x;
                    bb += y * y;
                }
                (shift, ab / (aa * bb).sqrt())
            })
            .fold((0, f64::MIN), |best, c| if c.1 > best.1 { c } else { best })
    }

    /// 30 s of noisy 25 fps video (so temporal filters have work) and pink
    /// noise, keyframes every 2 s, in MP4.
    fn noisy_source(dir: &std::path::Path) -> PathBuf {
        let source = dir.join("noisy.mp4");
        let made = Command::new(real::ffmpeg())
            .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i"])
            .arg("testsrc2=size=320x240:rate=25,noise=alls=20:allf=t")
            .args(["-f", "lavfi", "-i", "anoisesrc=d=30:c=pink:r=44100:seed=7"])
            .args([
                "-t",
                "30",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-g",
                "50",
            ])
            .args(["-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
            .arg(&source)
            .status()
            .unwrap();
        assert!(made.success());
        source
    }

    /// A chain of stateless nodes (eq, gblur, volume, channels) seeks to the
    /// start like a raw snapshot: the same frames as decoding from 0 s. The
    /// audio filters see the decoded audio in other chunks after a seek, so
    /// a sample can round one step differently, and the AAC encoder turns
    /// that into a different bitstream: the audio matches closely (aligned,
    /// correlation above 0.99), not exactly. An audio offset moves the seek
    /// earlier by itself.
    #[test]
    #[ignore = "needs ffmpeg with libx264"]
    fn a_stateless_chain_seeks_to_the_start() {
        use crate::playback::effects::{AudioNode, ChannelMode, VideoNode};
        let dir = tempfile::tempdir().unwrap();
        let source = noisy_source(dir.path());
        same_as_from_zero(
            &source,
            0.,
            (20.3, 21.9),
            Some(Effects {
                video: vec![
                    VideoNode::Eq {
                        brightness: 0.05,
                        contrast: 1.2,
                        saturation: 1.1,
                        gamma: 1.,
                    },
                    VideoNode::Gblur {
                        sigma: 1.5,
                        steps: 3,
                    },
                ],
                audio: vec![
                    AudioNode::Volume { gain_db: -3. },
                    AudioNode::Channels {
                        mode: ChannelMode::Mono,
                    },
                ],
                offset_ms: 400,
            }),
            SEEK_PREROLL,
            dir.path(),
            false,
        );
    }

    /// A chain with stateful nodes (temporal denoise, deinterlace, an
    /// equalizer, dynaudnorm) keeps 10 s of history before the start: the
    /// frames of the clip are those of decoding from 0 s; the audio filters
    /// keep a trace of older input (dynaudnorm's gain smoothing), so the
    /// samples match closely (aligned, correlation above 0.99), not exactly.
    #[test]
    #[ignore = "needs ffmpeg with libx264"]
    fn a_stateful_chain_keeps_a_bounded_history() {
        use crate::playback::effects::{AudioNode, DeinterlaceOutput, DeinterlaceWhen, VideoNode};
        let dir = tempfile::tempdir().unwrap();
        let source = noisy_source(dir.path());
        same_as_from_zero(
            &source,
            0.,
            (20.3, 21.9),
            Some(Effects {
                video: vec![
                    VideoNode::Denoise {
                        luma_spatial: 4.,
                        chroma_spatial: 3.,
                        luma_temporal: 6.,
                        chroma_temporal: 4.5,
                    },
                    VideoNode::Deinterlace {
                        when: DeinterlaceWhen::All,
                        output: DeinterlaceOutput::Frame,
                    },
                ],
                audio: vec![
                    AudioNode::Equalizer {
                        frequency: 1000.,
                        q: 1.,
                        gain_db: 6.,
                    },
                    AudioNode::Dynaudnorm {
                        enabled: true,
                        max_gain: 4.,
                        peak: 0.95,
                    },
                ],
                offset_ms: 0,
            }),
            BOUNDED_PREROLL,
            dir.path(),
            false,
        );
    }

    /// frameBlend's frame grid starts at the source's first frame, so its
    /// chain still decodes from 0 s.
    #[test]
    #[ignore = "needs ffmpeg with libx264"]
    fn a_frame_blend_chain_decodes_from_zero() {
        use crate::playback::effects::VideoNode;
        let dir = tempfile::tempdir().unwrap();
        let source = noisy_source(dir.path());
        same_as_from_zero(
            &source,
            0.,
            (20.3, 21.9),
            Some(Effects {
                video: vec![VideoNode::FrameBlend { fps: 60. }],
                audio: Vec::new(),
                offset_ms: 0,
            }),
            crate::playback::offline_audio::PREROLL,
            dir.path(),
            true,
        );
    }

    #[test]
    #[ignore = "needs ffmpeg (VESTRA_FFMPEG or PATH): run with --include-ignored (ci:local)"]
    fn a_far_seek_starts_the_audio_timeline_at_the_seek() {
        let seek = |at| {
            Some(Seek {
                at,
                preroll: SEEK_PREROLL,
            })
        };
        assert_eq!(audio_base(None, 0), 0.);
        // 30 min: the audio timeline starts there, not at 0 s.
        assert_eq!(audio_base(seek(1799.), 0), 1799.);
        // A positive offset plays audio later: the first sample is later
        // still, so the seek point is early enough. A negative one starts it
        // earlier, in whole seconds.
        assert_eq!(audio_base(seek(1799.), 400), 1799.);
        assert_eq!(audio_base(seek(1799.), -500), 1798.);
        assert_eq!(audio_base(seek(0.), -500), 0.);
        let graph =
            crate::playback::offline_audio::audio_from(&[], 0, 1800., 1801.5, 1799.).unwrap();
        assert!(
            graph.contains(
                "asetpts=PTS+(0-1799)/TB,aresample=48000:async=1:first_pts=0,apad,atrim=start=1:end=2.5,"
            ),
            "{graph}"
        );
        // From 0 s the graph is the one before seeking existed.
        assert_eq!(
            crate::playback::offline_audio::audio_from(&[], 400, 2., 3., 0.).unwrap(),
            "anull,asetpts=PTS+(0.4)/TB,aresample=48000:async=1:first_pts=0,apad,atrim=start=2:end=3,asetpts=PTS-STARTPTS"
        );
        // A base after the interval or between seconds is refused.
        assert!(crate::playback::offline_audio::audio_from(&[], 0, 1., 2., 2.).is_err());
        assert!(crate::playback::offline_audio::audio_from(&[], 0, 1., 2., 0.5).is_err());
        // Seek points are whole seconds.
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        std::fs::write(&source, b"bytes").unwrap();
        let request = real::request(
            &source,
            137.933,
            dir.path().join("o"),
            Output::Mp4 {
                end: 151.1,
                audio: Some(real::audio(1)),
                encoding: Encoding::default(),
            },
        );
        assert_eq!(seek_plan(&request, "mov,mp4"), seek(136.));
    }

    /// A clip 30 min into a file: the export seeks there and completes with
    /// its audio (the resampler starts at the seek, so it holds no 30 min of
    /// padding), and a job finished before FFmpeg's first progress report
    /// still ends at total/total.
    #[test]
    #[ignore = "needs ffmpeg with libx264"]
    fn a_far_seek_exports_with_audio() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("long.mp4");
        let made = Command::new(real::ffmpeg())
            .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i"])
            .arg("color=c=gray:size=64x64:rate=2")
            .args(["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000"])
            .args([
                "-t",
                "1830",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-g",
                "20",
            ])
            .args([
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-b:a",
                "32k",
                "-shortest",
            ])
            .arg(&source)
            .status()
            .unwrap();
        assert!(made.success());
        let destination = dir.path().join("far.mp4");
        let started = std::time::Instant::now();
        let (control, _) = real::run_job(real::request(
            &source,
            1800.,
            destination.clone(),
            Output::Mp4 {
                end: 1802.,
                audio: Some(real::audio(1)),
                encoding: Encoding::default(),
            },
        ));
        assert!(
            matches!(control.snapshot().1, Some(Outcome::Completed { .. })),
            "{:?}",
            control.snapshot().1
        );
        assert_eq!(control.facts()["preroll"], SEEK_PREROLL);
        assert!(started.elapsed() < Duration::from_secs(60));
        let (done, total) = control.progress().expect("progress after completion");
        assert_eq!(done, total);
        let pcm = decoded(&destination, "0:a:0", "pcm");
        // 2 s of 48 kHz stereo 16-bit, and not silence.
        assert!(pcm.len() >= 380_000, "{}", pcm.len());
        assert!(pcm.iter().any(|b| *b != 0));
    }

    #[test]
    #[ignore = "needs ffmpeg (VESTRA_FFMPEG or PATH): run with --include-ignored (ci:local)"]
    fn the_seek_follows_the_chain_s_history() {
        const MP4: &str = "mov,mp4,m4a,3gp,3g2,mj2";
        use crate::playback::effects::{AudioNode, Resource, VideoNode};
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        std::fs::write(&source, b"bytes").unwrap();
        let mp4 = || Output::Mp4 {
            end: 40.,
            audio: Some(real::audio(1)),
            encoding: Encoding::default(),
        };
        let with = |output, video: Vec<VideoNode>, audio: Vec<AudioNode>, offset_ms, time| {
            let mut request = real::request(&source, time, dir.path().join("out"), output);
            request.processed = Some(Arc::new(crate::playback::media_jobs::FrozenProcessed {
                processing: crate::playback::processing::FrozenProcessing {
                    revision: 1,
                    video: vec![Resource {
                        manifest_id: "p".into(),
                        resource_id: "v".into(),
                        nodes: video,
                    }],
                    audio: vec![Resource {
                        manifest_id: "p".into(),
                        resource_id: "a".into(),
                        nodes: audio,
                    }],
                    audio_offset_ms: offset_ms,
                },
                subtitle: None,
                width: 2,
                height: 2,
                audio_delay_ms: 0.,
            }));
            seek_plan(&request, MP4)
        };
        let eq = VideoNode::Eq {
            brightness: 0.,
            contrast: 1.,
            saturation: 1.,
            gamma: 1.,
        };
        let seek = |at, preroll| Some(Seek { at, preroll });
        // Raw: the start; a raw still and a copy seek elsewhere.
        assert_eq!(
            seek_plan(
                &real::request(&source, 30., dir.path().join("o"), mp4()),
                MP4
            ),
            seek(29., SEEK_PREROLL)
        );
        assert_eq!(
            seek_plan(
                &real::request(
                    &source,
                    30.,
                    dir.path().join("o"),
                    Output::Still(Still::Png)
                ),
                MP4
            ),
            None
        );
        // Stateless, also as a still; a positive audio offset seeks earlier.
        assert_eq!(
            with(mp4(), vec![eq.clone()], vec![], 0, 30.),
            seek(29., SEEK_PREROLL)
        );
        assert_eq!(
            with(Output::Still(Still::Png), vec![eq.clone()], vec![], 0, 30.),
            seek(30., SEEK_PREROLL)
        );
        assert_eq!(
            with(
                mp4(),
                vec![eq.clone()],
                vec![AudioNode::Volume { gain_db: 1. }],
                500,
                30.
            ),
            seek(28., SEEK_PREROLL)
        );
        assert_eq!(
            with(mp4(), vec![], vec![], -500, 30.),
            seek(29., SEEK_PREROLL)
        );
        // Spatial-only denoise is stateless, temporal denoise is not.
        let denoise = |temporal| VideoNode::Denoise {
            luma_spatial: 4.,
            chroma_spatial: 3.,
            luma_temporal: temporal,
            chroma_temporal: 0.,
        };
        assert_eq!(
            with(mp4(), vec![denoise(0.)], vec![], 0, 30.),
            seek(29., SEEK_PREROLL)
        );
        assert_eq!(
            with(mp4(), vec![eq.clone(), denoise(6.)], vec![], 0, 30.),
            seek(19., BOUNDED_PREROLL)
        );
        let dynaudnorm = AudioNode::Dynaudnorm {
            enabled: true,
            max_gain: 4.,
            peak: 0.95,
        };
        assert_eq!(
            with(mp4(), vec![], vec![dynaudnorm.clone()], 1000, 30.),
            seek(18., BOUNDED_PREROLL)
        );
        // History reaching 0 s, and frameBlend: from 0 s.
        assert_eq!(with(mp4(), vec![], vec![dynaudnorm], 0, 8.), None);
        assert_eq!(
            with(
                mp4(),
                vec![eq, VideoNode::FrameBlend { fps: 60. }],
                vec![],
                0,
                30.
            ),
            None
        );
        // MPEG-TS seeks by estimate: no seek. Matroska and WebM have cues.
        let raw = || real::request(&source, 30., dir.path().join("o"), mp4());
        assert_eq!(seek_plan(&raw(), "mpegts"), None);
        assert_eq!(seek_plan(&raw(), ""), None);
        assert_eq!(seek_plan(&raw(), "matroska,webm"), seek(29., SEEK_PREROLL));
    }

    #[test]
    fn the_source_bitrate_is_the_streams_or_the_containers_remainder() {
        let probed = |video: serde_json::Value| serde_json::json!({"format":{"bit_rate":"5000000"},"streams":[video,{"index":1,"codec_type":"audio","bit_rate":"192000"}]});
        assert_eq!(
            source_bitrate(
                &probed(serde_json::json!({"index":0,"bit_rate":"3000000"})),
                0
            ),
            Some(3_000_000)
        );
        // MKV streams carry no bit_rate: the container's minus the audio.
        assert_eq!(
            source_bitrate(&probed(serde_json::json!({"index":0})), 0),
            Some(4_808_000)
        );
        assert_eq!(
            source_bitrate(&probed(serde_json::json!({"index":0})), 3),
            None
        );
        assert_eq!(
            source_bitrate(&serde_json::json!({"format":{},"streams":[{"index":0}]}), 0),
            None
        );
    }

    /// "Same as source": the re-encode aims at the source video's bitrate.
    #[test]
    #[ignore = "needs ffmpeg with libx264 and the testmedia clips"]
    fn a_source_bitrate_encode_lands_near_the_source_bitrate() {
        let Some(source) = real::clip("bbb-480p-web.mp4") else {
            return;
        };
        let rate = |file: &std::path::Path| {
            real::probe(
                file,
                &["-select_streams", "v:0", "-show_entries", "stream=bit_rate"],
            )["streams"][0]["bit_rate"]
                .as_str()
                .unwrap()
                .parse::<f64>()
                .unwrap()
        };
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("same.mp4");
        let encoding: Encoding =
            serde_json::from_value(serde_json::json!({"videoBitrate":"source","threads":"auto"}))
                .unwrap();
        let (control, _) = real::run_job(real::request(
            &source,
            20.,
            destination.clone(),
            Output::Mp4 {
                end: 40.,
                audio: None,
                encoding,
            },
        ));
        assert!(
            matches!(control.snapshot().1, Some(Outcome::Completed { .. })),
            "{:?}",
            control.snapshot().1
        );
        let (out, src) = (rate(&destination), rate(&source));
        assert!((0.5..1.6).contains(&(out / src)), "{out} vs source {src}");
    }

    #[test]
    fn caps_apply_only_where_the_source_exceeds_them() {
        let encoding = |value| serde_json::from_value::<Encoding>(value).unwrap();
        assert_eq!(caps(&Encoding::default(), Some(60.)), "");
        assert_eq!(
            caps(&encoding(serde_json::json!({"fps":30})), Some(60.)),
            ",fps=30"
        );
        assert_eq!(
            caps(&encoding(serde_json::json!({"fps":30})), Some(29.97)),
            ""
        );
        assert_eq!(
            caps(&encoding(serde_json::json!({"fps":30})), Some(30.)),
            ""
        );
        assert_eq!(
            caps(&encoding(serde_json::json!({"fps":60})), None),
            ",fps=60"
        );
        assert_eq!(
            caps(&encoding(serde_json::json!({"maxHeight":720})), Some(24.)),
            ",scale=w='if(gt(ih,720),-2,iw)':h='min(ih,720)'"
        );
    }

    #[test]
    fn cancellation_and_publication_have_one_atomic_winner() {
        for _ in 0..64 {
            let control = Arc::new(Control::new());
            assert!(control.start());
            let barrier = Arc::new(Barrier::new(2));
            let other = control.clone();
            let gate = barrier.clone();
            let cancellation = thread::spawn(move || {
                gate.wait();
                other.cancel()
            });
            barrier.wait();
            let publishing = control.publish();
            let cancel = cancellation.join().unwrap();
            assert!(matches!(
                (publishing, cancel),
                (true, CancelResult::TooLate) | (false, CancelResult::Requested)
            ));
        }
    }

    #[test]
    fn queued_cancel_does_not_spawn_and_terminal_result_is_stable() {
        let control = Control::new();
        assert_eq!(control.cancel(), CancelResult::Requested);
        assert!(!control.start());
        control.finish(Outcome::Cancelled);
        assert_eq!(control.status(), Status::Cancelled);
        assert_eq!(control.snapshot().1, Some(Outcome::Cancelled));
        assert_eq!(control.cancel(), CancelResult::Terminal);
        control.finish(Outcome::Failed {
            code: "export-failed",
            message: "late".into(),
        });
        assert_eq!(control.snapshot().1, Some(Outcome::Cancelled));
        let publishing = Control::new();
        assert!(publishing.start());
        assert!(publishing.publish());
        assert_eq!(publishing.cancel(), CancelResult::TooLate);
        publishing.finish(Outcome::Completed {
            destination: "published.png".into(),
            range: None,
        });
        assert_eq!(publishing.cancel(), CancelResult::Terminal);
        let failed_publish = Control::new();
        assert!(failed_publish.start());
        assert!(failed_publish.publish());
        failed_publish.finish(Outcome::Failed {
            code: "export-failed",
            message: "destination already exists".into(),
        });
        let (status, outcome, publication_started) = failed_publish.snapshot();
        assert_eq!(status, Status::Failed);
        assert!(matches!(outcome, Some(Outcome::Failed { .. })));
        assert!(publication_started);
    }

    #[test]
    fn cancellation_interrupts_wait_for_shared_stdin_reader_without_starting_encoder() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        std::fs::write(&source, b"held bytes").unwrap();
        let lease = Arc::new(Lease::open(&source).unwrap());
        let reader = lease.lock_reader().unwrap();
        let control = Arc::new(Control::new());
        let worker_control = control.clone();
        let request = RenderRequest {
            lease: lease.clone(),
            time: 0.0,
            destination: dir.path().join("out.png"),
            ffmpeg: dir.path().join("must-not-run"),
            processed: None,
            output: Output::Still(Still::Png),
            video_stream: 0,
            source_origin: 0.,
        };
        let (done_tx, done_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            run(request, worker_control);
            done_tx.send(()).unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        while control.status() == Status::Queued && Instant::now() < deadline {
            thread::yield_now();
        }
        assert_eq!(control.status(), Status::Running);
        assert_eq!(control.cancel(), CancelResult::Requested);
        done_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(control.snapshot().1, Some(Outcome::Cancelled));
        assert!(!dir.path().join("out.png").exists());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        drop(reader);
        worker.join().unwrap();
    }

    #[test]
    fn active_cancellation_is_not_completed_by_a_duplicate_runner() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        std::fs::write(&source, b"bytes").unwrap();
        let lease = Arc::new(Lease::open(&source).unwrap());
        let control = Arc::new(Control::new());
        assert!(control.start());
        assert_eq!(control.cancel(), CancelResult::Requested);
        run(
            RenderRequest {
                lease,
                time: 0.0,
                destination: dir.path().join("out.png"),
                ffmpeg: dir.path().join("must-not-run"),
                processed: None,
                output: Output::Still(Still::Png),
                video_stream: 0,
                source_origin: 0.,
            },
            control.clone(),
        );
        assert_eq!(control.status(), Status::Cancelling);
        assert_eq!(control.snapshot().1, None);
        control.finish(Outcome::Cancelled);
        control.fail("late panic recovery".into());
        assert_eq!(control.snapshot().1, Some(Outcome::Cancelled));
        let failed = Control::new();
        assert!(failed.start());
        failed.fail("worker panic".into());
        assert_eq!(failed.status(), Status::Failed);
        assert_eq!(failed.cancel(), CancelResult::Terminal);
    }

    /// The held lease feeds FFmpeg as `HELD_FILE_INPUT` (Windows: `fd:`) and
    /// a still comes out. Needs FFmpeg/FFprobe with libx264: `VESTRA_FFMPEG`
    /// (`env.ps1 -Ffmpeg`), else `ffmpeg` on PATH with `ffprobe` beside it.
    #[test]
    #[ignore = "needs ffmpeg with libx264"]
    fn a_still_is_rendered_from_the_held_lease() {
        let ffmpeg = real::ffmpeg();
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("clip.mp4");
        let made = Command::new(&ffmpeg)
            .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i"])
            .arg("testsrc2=size=320x240:rate=30")
            .args(["-t", "2", "-c:v", "libx264", "-preset", "ultrafast"])
            .args(["-pix_fmt", "yuv420p"])
            .arg(&source)
            .status()
            .unwrap();
        assert!(made.success());
        let lease = Arc::new(Lease::open(&source).unwrap());
        let destination = dir.path().join("still.png");
        let control = Arc::new(Control::new());
        run(
            RenderRequest {
                lease,
                time: 1.0,
                destination: destination.clone(),
                ffmpeg,
                processed: None,
                output: Output::Still(Still::Png),
                video_stream: 0,
                source_origin: 0.,
            },
            control.clone(),
        );
        assert_eq!(
            control.snapshot().1,
            Some(Outcome::Completed {
                destination: destination.clone(),
                range: None,
            })
        );
        let still = image::open(&destination).unwrap();
        assert_eq!((still.width(), still.height()), (320, 240));
    }

    #[test]
    fn stderr_drain_keeps_only_a_bounded_suffix() {
        let mut input = vec![b'x'; STDERR_TAIL * 3];
        input.extend_from_slice(b"final diagnostic");
        let tail = drain_tail(io::Cursor::new(input)).unwrap();
        assert_eq!(tail.len(), STDERR_TAIL);
        assert!(tail.ends_with(b"final diagnostic"));
    }

    #[test]
    fn stills_convert_from_the_validated_png_bmp_pixel_for_pixel() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("frame.png");
        let pixels =
            image::RgbImage::from_fn(17, 9, |x, y| image::Rgb([x as u8 * 15, y as u8 * 28, 200]));
        pixels
            .save_with_format(&png, image::ImageFormat::Png)
            .unwrap();
        let bmp = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
        convert_still(&png, Still::Bmp, bmp.as_file()).unwrap();
        let back = image::load_from_memory_with_format(
            &std::fs::read(bmp.path()).unwrap(),
            image::ImageFormat::Bmp,
        )
        .unwrap();
        assert_eq!(back.to_rgb8(), pixels);
        let jpeg = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
        convert_still(&png, Still::Jpeg { quality: 90 }, jpeg.as_file()).unwrap();
        let bytes = std::fs::read(jpeg.path()).unwrap();
        assert!(bytes.starts_with(&[0xff, 0xd8, 0xff]));
        let decoded =
            image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (17, 9));
        let low = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
        convert_still(&png, Still::Jpeg { quality: 1 }, low.as_file()).unwrap();
        assert!(std::fs::metadata(low.path()).unwrap().len() < bytes.len() as u64);
        let broken = dir.path().join("broken.png");
        std::fs::write(&broken, b"not a png").unwrap();
        let out = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
        assert!(convert_still(&broken, Still::Bmp, out.as_file()).is_err());
        assert!(convert_still(&png, Still::Png, out.as_file()).is_err());
    }
}
