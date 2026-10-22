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

use crate::source_lease::{Lease, Reader};
use anyhow::{Context, Result, bail};

// Distinguish queued cancellation from an active worker cancelling, so a
// duplicate runner cannot report terminal cancellation before the owner reaps.
const CANCELLED_BEFORE_START: u8 = 7;
const POLL: Duration = Duration::from_millis(20);
const STDERR_TAIL: usize = 64 * 1024;
const PNG_BYTES: usize = 128 * 1024 * 1024;

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
    Mp4 {
        end: f64,
        audio: Option<crate::media_tracks::AudioTrack>,
    },
}
pub struct RenderRequest {
    pub lease: Arc<Lease>,
    pub time: f64,
    pub destination: PathBuf,
    pub ffmpeg: PathBuf,
    pub processed: Option<Arc<crate::media_jobs::FrozenProcessed>>,
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Completed { destination: PathBuf },
    Failed { message: String },
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
        }
    }
    pub fn facts(&self) -> serde_json::Value {
        self.facts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
    fn record_facts(&self, facts: serde_json::Value) {
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
        self.finish(Outcome::Failed { message });
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
    fn publish(&self) -> bool {
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
    if !control.start() {
        if control.state.load(Ordering::Acquire) == CANCELLED_BEFORE_START {
            control.finish(Outcome::Cancelled);
        }
        return;
    }
    match std::panic::catch_unwind(|| encode(&request, &control)) {
        Ok(Ok(outcome)) => control.finish(outcome),
        Ok(Err(error)) => control.finish(Outcome::Failed {
            message: format!("{error:#}"),
        }),
        Err(_) => control.finish(Outcome::Failed {
            message: "offline PNG worker panicked".into(),
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

struct ReapedChild {
    child: Child,
    reaped: bool,
}
impl ReapedChild {
    fn wait(&mut self) -> io::Result<ExitStatus> {
        let status = self.child.wait()?;
        self.reaped = true;
        Ok(status)
    }
    fn kill_wait(&mut self) -> io::Result<ExitStatus> {
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
            eprintln!("media worker could not reap child: {error}");
        }
    }
}

enum ChildResult {
    Exited(ExitStatus),
    Cancelled,
}
fn poll_child(child: &mut ReapedChild, control: &Control) -> Result<ChildResult> {
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

fn drain_tail(mut reader: impl Read) -> io::Result<Vec<u8>> {
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

fn capture_command(command: &mut Command, control: &Control) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
    capture_input_command(command, Stdio::null(), control)
}
fn capture_input_command(
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

fn validate_mp4(
    request: &RenderRequest,
    temp: &tempfile::NamedTempFile,
    control: &Control,
) -> Result<bool> {
    // Decode every encoded stream to completion before publication, on this
    // cancellable background worker. Encoder success/header bytes alone do not
    // establish a readable MP4. ffprobe/independent consumers verify codec/time.
    let child = crate::platform::tool_command(&request.ffmpeg)
        .args(["-nostdin", "-v", "error", "-threads", "1", "-i"])
        .arg(temp.path())
        .args([
            "-map", "0:v:0", "-map", "0:a?", "-threads", "1", "-f", "null", "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("MP4 validation decoder could not start")?;
    let mut child = ReapedChild {
        child,
        reaped: false,
    };
    let stderr = child
        .child
        .stderr
        .take()
        .context("MP4 validation stderr unavailable")?;
    let drain = thread::Builder::new()
        .name("mp4-validation-stderr".into())
        .spawn(move || drain_tail(stderr));
    let drain = match drain {
        Ok(value) => value,
        Err(error) => {
            child.kill_wait()?;
            return Err(error).context("MP4 validation drain could not start");
        }
    };
    let result = poll_child(&mut child, control);
    if !child.reaped {
        child.kill_wait()?;
    }
    let stderr = drain
        .join()
        .map_err(|_| anyhow::anyhow!("MP4 validation drain panicked"))??;
    match result? {
        ChildResult::Cancelled => Ok(false),
        ChildResult::Exited(status) if status.success() => {
            let probe = sibling(&request.ffmpeg, "ffprobe")?;
            let mut command = crate::platform::tool_command(probe);
            command.args(["-v","error","-show_entries","stream=index,codec_name,codec_type,width,height,sample_rate,channels,channel_layout,duration,r_frame_rate:format=duration","-of","json"]).arg(temp.path());
            let Some((stdout, _)) = capture_command(&mut command, control)? else {
                return Ok(false);
            };
            let output: serde_json::Value =
                serde_json::from_slice(&stdout).context("encoded MP4 probe metadata invalid")?;
            // Retain actual video/audio track facts even when coverage rejects publication.
            let mut facts = control.facts();
            facts["output"] = output.clone();
            control.record_facts(facts);
            if let Output::Mp4 { end, .. } = &request.output {
                let video = output["streams"]
                    .as_array()
                    .and_then(|streams| streams.iter().find(|s| s["codec_type"] == "video"))
                    .context("encoded MP4 video stream missing")?;
                let actual = video["duration"]
                    .as_str()
                    .and_then(|s| s.parse::<f64>().ok())
                    .filter(|v| v.is_finite() && *v > 0.)
                    .context("encoded MP4 video duration unavailable")?;
                let rate = video["r_frame_rate"]
                    .as_str()
                    .and_then(|r| r.split_once('/'))
                    .and_then(|(n, d)| Some(n.parse::<f64>().ok()? / d.parse::<f64>().ok()?))
                    .filter(|r| r.is_finite() && *r > 0.)
                    .context("encoded MP4 video frame interval unavailable")?;
                let tolerance = 1. / rate + 0.0001;
                if actual + tolerance < end - request.time {
                    bail!(
                        "source-unavailable: encoded video covers {actual}s, shorter than requested {}s beyond one frame ({tolerance}s)",
                        end - request.time
                    );
                }
            }
            Ok(true)
        }
        ChildResult::Exited(status) => bail!(
            "MP4 validation decode failed ({status}): {}",
            String::from_utf8_lossy(&stderr)
        ),
    }
}

/// FFmpeg's sibling tool (`ffprobe`, with `.exe` on Windows).
fn sibling(ffmpeg: &std::path::Path, tool: &str) -> Result<std::path::PathBuf> {
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
    control.record_facts(serde_json::json!({"descriptorVersion":"vestra-finite/1","ffmpegVersion":ffmpeg_version,"libavfilterVersion":libavfilter.trim(),"videoStreamIndex":request.video_stream,"range":{"start":request.time,"end":match &request.output{Output::Still(_)=>None,Output::Mp4{end,..}=>Some(*end)},"interval":"half-open; first video PTS >= start"},"preroll":if request.processed.is_some() || matches!(&request.output,Output::Mp4{..}) {Some(crate::offline_audio::PREROLL)}else{None},"hasAudio":matches!(&request.output,Output::Mp4{audio:Some(_),..}),"audioTiming":if matches!(&request.output,Output::Mp4{audio:Some(_),..}) {"DSP source0 -> signed PTS shift -> 48kHz resample first_pts=0 -> zero pad -> range trim; device gain/mute excluded"}else{"not applicable: no audio output"},"processed":request.processed.is_some()}));
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
        "format=start_time",
        "-of",
        "json",
        crate::platform::HELD_FILE_INPUT,
    ]);
    let Some((origin, _)) = capture_input_command(&mut origin_command, reader.stdin()?, control)?
    else {
        return Ok(Outcome::Cancelled);
    };
    let origin: serde_json::Value =
        serde_json::from_slice(&origin).context("source origin probe JSON invalid")?;
    let origin = origin["format"]["start_time"]
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
    // Native extraction and video decoding share this OFFLINE reader only in
    // sequence. Each stdin rewind occurs after the earlier child is reaped.
    let native = if let Some(frozen) = &request.processed
        && let Some(subtitle) = &frozen.subtitle
        && let crate::subtitle_backend::FrozenSubtitleInput::Native {
            stream_index,
            codec,
        } = &subtitle.input
    {
        let stdin = reader
            .stdin()
            .context("native subtitle source stdin could not be prepared")?;
        let Some(native) = crate::native_subtitle::extract(
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
        let subtitle =
            frozen
                .subtitle
                .as_ref()
                .map(|subtitle| crate::offline_descriptor::Subtitle {
                    carrier: match &subtitle.input {
                        crate::subtitle_backend::FrozenSubtitleInput::Carrier(carrier) => {
                            carrier.path()
                        }
                        crate::subtitle_backend::FrozenSubtitleInput::Native { .. } => native
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
                });
        match &request.output {
            Output::Still(_) => {
                crate::offline_descriptor::video(&frozen.processing.video, subtitle, request.time)?
            }
            Output::Mp4 { end, .. } => crate::offline_descriptor::video_range(
                &frozen.processing.video,
                subtitle,
                request.time,
                Some(*end),
            )?,
        }
    } else {
        "scale=w='max(1,round(iw*sar))':h=ih,setsar=1,settb=AVTB".into()
    };
    let mut command = crate::platform::tool_command(&request.ffmpeg);
    command.args([
        "-nostdin",
        "-v",
        "error",
        "-threads",
        "1",
        "-filter_threads",
        "1",
        "-filter_complex_threads",
        "1",
    ]);
    if request.processed.is_none() && matches!(&request.output, Output::Still(_)) {
        command.args(["-ss", &request.time.to_string()]);
    }
    command.args(["-i", crate::platform::HELD_FILE_INPUT]);
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
        Output::Mp4 { end, audio } => {
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
                let af = crate::offline_audio::audio(resources, offset, request.time, *end)?;
                let stream = audio
                    .stream_index
                    .context("frozen audio stream mapping unavailable")?;
                graph.push_str(&format!(";[0:{stream}]{af}[a]"));
            }
            command.args(["-filter_complex", &graph, "-map", "[v]"]);
            if audio.is_some() {
                command.args(["-map", "[a]", "-c:a", "aac", "-ar", "48000", "-b:a", "192k"]);
            } else {
                command.arg("-an");
            }
            command.args([
                "-t",
                &duration.to_string(),
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-crf",
                "18",
                "-pix_fmt",
                "yuv420p",
                "-threads",
                "1",
                "-movflags",
                "+faststart",
                "-f",
                "mp4",
                "-y",
            ]);
        }
    }
    let child = command
        .arg(temp.path())
        .stdin(stdin)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("offline PNG encoder could not be started")?;
    let mut child = ReapedChild {
        child,
        reaped: false,
    };
    let stderr = child
        .child
        .stderr
        .take()
        .context("encoder stderr pipe is unavailable")?;
    let drain = match thread::Builder::new()
        .name("png-stderr".into())
        .spawn(move || drain_tail(stderr))
    {
        Ok(drain) => drain,
        Err(error) => {
            child
                .kill_wait()
                .context("encoder cleanup after stderr worker failure failed")?;
            return Err(error).context("encoder stderr drain could not be started");
        }
    };
    let waited = poll_child(&mut child, control);
    // Keep both child guard and source Reader until reaping and stderr drain
    // complete, including cancellation/error paths. FFmpeg is the trusted binary
    // and must not delegate stderr ownership to an unbounded-lived grandchild.
    if !child.reaped {
        child
            .kill_wait()
            .context("encoder cleanup could not reap child")?;
    }
    let stderr = drain
        .join()
        .map_err(|_| anyhow::anyhow!("encoder stderr drain panicked"))?
        .context("encoder stderr drain failed")?;
    match waited? {
        ChildResult::Cancelled => return Ok(Outcome::Cancelled),
        ChildResult::Exited(status) if !status.success() => bail!(
            "PNG encoder failed ({status}): {}",
            String::from_utf8_lossy(&stderr)
        ),
        ChildResult::Exited(_) => {}
    }
    if control.status() == Status::Cancelling {
        return Ok(Outcome::Cancelled);
    }
    let temp = match &request.output {
        Output::Still(still) => {
            validate_png(&temp)?;
            if *still == Still::Png {
                temp
            } else {
                // The converted file replaces the PNG (removed when dropped).
                let converted = tempfile::NamedTempFile::new_in(&parent)
                    .context("owned temporary output could not be created")?;
                convert_still(temp.path(), *still, converted.as_file())?;
                converted
            }
        }
        Output::Mp4 { .. } => {
            if !validate_mp4(request, &temp, control)? {
                return Ok(Outcome::Cancelled);
            }
            temp
        }
    };
    reader
        .validate()
        .context("source-unavailable after encoding")?;
    // This CAS is the publication/cancellation boundary. No filesystem publish
    // is attempted if a cancellation request has already won.
    if !control.publish() {
        return Ok(Outcome::Cancelled);
    }
    temp.as_file()
        .sync_all()
        .context("temporary output could not be flushed")?;
    temp.persist_noclobber(&destination)
        .map_err(|error| anyhow::anyhow!("no-clobber publication failed: {}", error.error))?;
    Ok(Outcome::Completed { destination })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{Barrier, mpsc},
        time::Instant,
    };

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
            message: "late".into(),
        });
        assert_eq!(control.snapshot().1, Some(Outcome::Cancelled));
        let publishing = Control::new();
        assert!(publishing.start());
        assert!(publishing.publish());
        assert_eq!(publishing.cancel(), CancelResult::TooLate);
        publishing.finish(Outcome::Completed {
            destination: "published.png".into(),
        });
        assert_eq!(publishing.cancel(), CancelResult::Terminal);
        let failed_publish = Control::new();
        assert!(failed_publish.start());
        assert!(failed_publish.publish());
        failed_publish.finish(Outcome::Failed {
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
