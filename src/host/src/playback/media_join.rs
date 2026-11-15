//! Joined exports: one file from parts of several referenced local files
//! (`jobs.start { parts }`, host-internal; runs on the media worker like any
//! export).
//!
//! Each part is read from its own held file (a [`Lease`], FFmpeg's stdin, as
//! every export reads its source) and written to an intermediate file in a
//! temporary directory beside the destination; the concat demuxer then
//! copies the intermediates into one file, which is checked and published
//! no-clobber like a clip. One FFmpeg reads one held file, so the parts are
//! cut one after another rather than in one filter graph.
//!
//! - **copy**: each part is a stream copy (`media_copy.rs`), starting on the
//!   video keyframe at or before its start. Every part must have the same
//!   video codec, profile, codec extradata, size, frame rate and pixel
//!   format, and the same audio (codec, rate, channels) or none
//!   (`media_sources::check`); the files are probed again first.
//! - **encode**: each part is re-encoded to H.264 + AAC at the first part's
//!   square-pixel size and frame rate (under the encoding's ceilings), the
//!   picture scaled to fit and centred on black, the audio resampled to
//!   48 kHz stereo; a part without audio gets silence when another part has
//!   audio.
//!
//! The concat list names the intermediates by absolute path, quoted for the
//! concat demuxer's syntax ([`concat_line`]); no user path is ever written
//! into a filter graph.
#![forbid(unsafe_code)]

use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::playback::{
    media_sources::{self, Entry, Part, Resolved, Status},
    media_tracks::AudioTrack,
    media_worker::{
        AUDIO_WARMUP_SECONDS, Coded, Container, Control, Encoding, Outcome, Refused, Segment,
        Status as JobStatus, VideoBitrate, capture_command, decode_check, run_tool, run_with,
        seeks_exactly, sibling, source_bitrate,
    },
    source_lease::Lease,
};

/// The copy's start decode check, as for a copied clip (`media_copy.rs`).
const DECODE_CHECK_SECONDS: f64 = 0.5;

pub enum Mode {
    Copy { container: Container },
    Encode { encoding: Encoding },
}

pub struct JoinRequest {
    /// The parts in order, each with its reference as stored.
    pub parts: Vec<(Part, Entry)>,
    pub destination: PathBuf,
    pub ffmpeg: PathBuf,
    pub mode: Mode,
    /// Where the intermediate parts go ([`scratch_root`]); `None`: a
    /// hidden folder beside the destination.
    pub scratch: Option<PathBuf>,
}

/// The player's own folder for intermediate parts:
/// `<app cache>/export-parts`, out of the user's folders; the
/// `.vestra-join-*` folders a crashed player left there are removed at
/// start-up ([`clean_scratch`]), so a crash or a power cut mid-export
/// leaves nothing beside the user's videos. `None` in the host's unit
/// tests, which never touch the user's cache (a join then works beside its
/// destination).
pub fn scratch_root() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    crate::platform::app_cache_dir().map(|dir| dir.join("export-parts"))
}

/// The prefix of an export's folder of intermediate parts.
const WORK_PREFIX: &str = ".vestra-join-";

/// The file in an export's folder that names the process running the
/// export: its id and its start mark (`platform::process_start`), one per
/// line.
const OWNER_FILE: &str = "owner";

/// How long a folder without a readable owner stays (one from a player
/// before the owner file, or one whose owner the system does not report):
/// removed once nothing in it changed for this long.
const UNOWNED_KEPT: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Writes the owner file of an export's folder `work`: this process. A
/// player of the same user and cache that starts meanwhile (a second
/// player, a test run) leaves the folder alone while this process runs.
fn claim(work: &Path) -> Result<()> {
    let start = match crate::platform::process_start(std::process::id()) {
        crate::platform::ProcessStart::Running(start) => start,
        // Unknown: the age rule protects it.
        _ => String::new(),
    };
    std::fs::write(
        work.join(OWNER_FILE),
        format!("{}\n{start}\n", std::process::id()),
    )
    .context("the temporary folder's owner could not be written")
}

/// The owner a folder's owner file names: its process id and start mark;
/// `None` without a readable file or with an empty mark.
fn owner(work: &Path) -> Option<(u32, String)> {
    let text = std::fs::read_to_string(work.join(OWNER_FILE)).ok()?;
    let mut lines = text.lines();
    let pid = lines.next()?.trim().parse().ok()?;
    let start = lines.next()?.trim();
    (!start.is_empty()).then(|| (pid, start.to_owned()))
}

/// When anything in `work` last changed: the folder and its entries (not
/// followed), the latest; `None` when the system does not say.
fn last_change(work: &Path) -> Option<std::time::SystemTime> {
    let mut latest = std::fs::symlink_metadata(work).ok()?.modified().ok()?;
    for entry in std::fs::read_dir(work).ok()?.flatten() {
        if let Ok(changed) = entry.metadata().and_then(|meta| meta.modified()) {
            latest = latest.max(changed);
        }
    }
    Some(latest)
}

/// Removes the export folders (`.vestra-join-*`) in `root` whose export no
/// longer runs, at start-up: other players of the same user and cache may
/// be exporting into their own folders there (a second player, a test
/// run). A folder whose owner process still runs (its id with the same
/// start mark, so a reused id does not count) stays; one whose owner has
/// ended goes; one without a readable owner, or whose owner the system does
/// not report, goes only once nothing in it changed for a day. Only when
/// `root` itself is a real directory, not a link or junction to somewhere
/// else; a link among the entries is removed itself, never followed.
pub fn clean_scratch(root: &Path) {
    clean_scratch_with(
        root,
        crate::platform::process_start,
        std::time::SystemTime::now(),
    );
}

/// [`clean_scratch`] with the owner check and the clock given (tests).
fn clean_scratch_with(
    root: &Path,
    process_start: impl Fn(u32) -> crate::platform::ProcessStart,
    now: std::time::SystemTime,
) {
    use crate::platform::ProcessStart;
    let real_directory = std::fs::symlink_metadata(root)
        .is_ok_and(|meta| meta.is_dir() && !meta.file_type().is_symlink());
    if !real_directory {
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let old = |path: &Path| {
        last_change(path)
            .and_then(|changed| now.duration_since(changed).ok())
            .is_some_and(|age| age >= UNOWNED_KEPT)
    };
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().starts_with(WORK_PREFIX) {
            continue;
        }
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        let removed = if meta.file_type().is_symlink() {
            // A link (or a junction) goes as a link.
            std::fs::remove_file(&path).or_else(|_| std::fs::remove_dir(&path))
        } else if meta.is_dir() {
            let ended = match owner(&path) {
                Some((pid, start)) => match process_start(pid) {
                    ProcessStart::Running(running) => running != start,
                    ProcessStart::Gone => true,
                    ProcessStart::Unknown => old(&path),
                },
                None => old(&path),
            };
            if !ended {
                continue;
            }
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        if let Err(error) = removed {
            diag!("export parts: {} not removed: {error}", path.display());
        }
    }
}

/// Bytes a part of `length` seconds of `entry` takes once written: a copy
/// its share of the file, a re-encode the target's picture at about 0.12
/// bit per pixel (x264 CRF 18 at its fastest presets writes less on most
/// sources) or the source's rate (`videoBitrate: "source"`, at its 1.5×
/// peak), plus AAC at 256 kb/s. An estimate for the space check, on the
/// high side.
fn part_bytes(entry: &Entry, length: f64, mode: &Mode, target: media_sources::Target) -> f64 {
    let share = entry.len as f64 * length / entry.facts.duration.max(0.001);
    match mode {
        Mode::Copy { .. } => share,
        Mode::Encode { encoding } => {
            let video = match encoding.video_bitrate {
                Some(VideoBitrate::Source) => share * 1.5,
                None => {
                    f64::from(target.width) * f64::from(target.height) * f64::from(target.rate.0)
                        / f64::from(target.rate.1)
                        * 0.12
                        / 8.
                        * length
                }
            };
            video + 32_000. * length
        }
    }
}

/// Free space for the export: the intermediate parts in `scratch` and the
/// output beside `destination`, each about `bytes`, twice that when both
/// are on one volume, with 10 % and 64 MiB to spare. A volume the system
/// cannot tell about is not checked.
fn check_space(bytes: f64, scratch: &Path, destination: &Path) -> Result<()> {
    let need = bytes * 1.1 + 64. * 1024. * 1024.;
    let gib = |b: f64| b / (1024. * 1024. * 1024.);
    let short = |need: f64, free: u64, volume: &str| -> Result<()> {
        if (free as f64) < need {
            return Err(Coded(
                "no-space",
                format!(
                    "the export needs about {:.1} GB on {volume}, {:.1} GB is free",
                    gib(need),
                    gib(free as f64)
                ),
            )
            .into());
        }
        Ok(())
    };
    match (
        crate::platform::disk_space(scratch),
        crate::platform::disk_space(destination),
    ) {
        (Some((free, a)), Some((_, b))) if a == b => short(need * 2., free, &a),
        (scratch, destination) => {
            if let Some((free, volume)) = scratch {
                short(need, free, &volume)?;
            }
            if let Some((free, volume)) = destination {
                short(need, free, &volume)?;
            }
            Ok(())
        }
    }
}

/// Decodes `seconds` of `path` from `at` (the joins of a copy: each part's
/// start must decode); `Ok(None)`: cancelled.
fn decode_at(
    ffmpeg: &Path,
    path: &Path,
    at: f64,
    seconds: f64,
    control: &Control,
) -> Result<Option<()>> {
    let mut command = crate::platform::background_tool_command(ffmpeg);
    command
        .args(["-nostdin", "-v", "error", "-xerror", "-threads", "2"])
        .args(["-ss", &secs(at), "-t", &secs(seconds), "-i"])
        .arg(path)
        .args(["-map", "0:v:0", "-map", "0:a?", "-f", "null", "-"]);
    match run_tool(&mut command, Stdio::null(), control, None)? {
        None => Ok(None),
        Some((status, _)) if status.success() => Ok(Some(())),
        Some((status, stderr)) => bail!(
            "the join at {at:.3}s does not decode ({status}): {}",
            String::from_utf8_lossy(&stderr).trim()
        ),
    }
}

pub fn run(request: JoinRequest, control: &Control) {
    run_with(control, |control| join(&request, control));
}

/// One `file` line of a concat list: the path in single quotes, a quote
/// inside written `'\''` (FFmpeg's quoting: inside single quotes every
/// character but `'` is literal, so backslashes, spaces and any Unicode stay
/// as they are). Paths with a line break cannot be listed.
pub fn concat_line(path: &Path) -> Result<String> {
    let text = path
        .to_str()
        .context("an intermediate path is not valid Unicode")?;
    if text.contains(['\n', '\r', '\0']) {
        bail!("an intermediate path contains a line break");
    }
    Ok(format!("file '{}'\n", text.replace('\'', r"'\''")))
}

/// Seconds as FFmpeg reads them.
fn secs(value: f64) -> String {
    format!("{value:.6}")
}

fn part_name(entry: &Entry, index: usize) -> String {
    format!("part {} ({})", index + 1, entry.name)
}

/// Open each part's file as it was referenced.
fn open_parts(parts: &[(Part, Entry)]) -> Result<Vec<Lease>> {
    let mut leases = Vec::with_capacity(parts.len());
    for (index, (_, entry)) in parts.iter().enumerate() {
        let lease = match Lease::open(&entry.path) {
            Ok(lease) => lease,
            Err(_) if entry.status() == Status::Missing => {
                return Err(Coded(
                    "source-missing",
                    format!("{} is missing", part_name(entry, index)),
                )
                .into());
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("{} cannot be read", part_name(entry, index)));
            }
        };
        let facts = lease.facts();
        if facts.len != entry.len || media_sources::nanos(facts.modified) != entry.modified {
            return Err(Coded(
                "source-changed",
                format!("{} changed since it was added", part_name(entry, index)),
            )
            .into());
        }
        leases.push(lease);
    }
    Ok(leases)
}

fn join(request: &JoinRequest, control: &Control) -> Result<Outcome> {
    let parts = &request.parts;
    let leases = open_parts(parts)?;
    let Some((version, _)) = capture_command(
        crate::platform::tool_command(&request.ffmpeg).arg("-version"),
        control,
    )?
    else {
        return Ok(Outcome::Cancelled);
    };
    let version = String::from_utf8_lossy(&version);
    let line = |prefix: &str| {
        version
            .lines()
            .find(|l| l.trim_start().starts_with(prefix))
            .map(|l| l.trim().to_owned())
    };
    // Probe every part again: the facts the copy check and the commands use
    // are the files' as they are now.
    let mut probed = Vec::with_capacity(parts.len());
    let mut resolved = Vec::with_capacity(parts.len());
    for (lease, (_, entry)) in leases.iter().zip(parts) {
        let Some(json) = media_sources::probe(&request.ffmpeg, lease, control)? else {
            return Ok(Outcome::Cancelled);
        };
        let facts = media_sources::facts_from(&json)
            .map_err(|e| Refused(format!("unsupported: {}: {}", entry.name, e.message)))?;
        resolved.push(Resolved {
            entry: Some(Entry {
                facts,
                ..entry.clone()
            }),
            status: Status::Ok,
        });
        probed.push(json);
    }
    let encoding = match &request.mode {
        Mode::Encode { encoding } => *encoding,
        Mode::Copy { .. } => Encoding::default(),
    };
    let plain: Vec<Part> = parts.iter().map(|(part, _)| part.clone()).collect();
    let checked = media_sources::check(&plain, &resolved, &encoding);
    if let Some(problem) = checked["blocking"].as_array().and_then(|b| b.first()) {
        let index = problem["part"].as_u64().unwrap_or(0) as usize;
        bail!(Refused(format!(
            "unsupported: {} cannot be joined ({})",
            part_name(&parts[index].1, index),
            problem["problem"].as_str().unwrap_or("?")
        )));
    }
    let facts: Vec<&media_sources::Facts> = resolved
        .iter()
        .map(|r| &r.entry.as_ref().expect("resolved above").facts)
        .collect();
    let first_video = facts[0].video.as_ref().context("first part has no video")?;
    let any_audio = facts.iter().any(|f| f.audio.is_some());
    control.record_facts(json!({
        "descriptorVersion": "vestra-join/1",
        "ffmpegVersion": line("ffmpeg version ").unwrap_or_default(),
        "libavfilterVersion": line("libavfilter").unwrap_or_default(),
        "videoStreamIndex": first_video.index,
        "range": {"start": parts[0].0.start, "end": null, "interval": "join of parts; see join"},
        "preroll": null,
        "hasAudio": any_audio,
        "audioTiming": match &request.mode {
            Mode::Copy { .. } => "stream copy: each part's packets from its video start, unchanged",
            Mode::Encode { .. } => "each part resampled to 48 kHz stereo; silence for a part without audio",
        },
        "processed": false,
    }));

    // Where it goes: the copy's container decides the extension.
    let filename = request
        .destination
        .file_name()
        .context("destination needs a file name")?;
    let parent = request
        .destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = dunce::canonicalize(parent).context("destination directory is unavailable")?;
    let (container, destination) = match &request.mode {
        Mode::Copy { container } => {
            let fits = checked["copy"]["container"] == "mp4";
            if checked["copy"]["ok"] != true {
                bail!(Refused(format!(
                    "unsupported: the parts cannot be stream-copied together: {}",
                    reasons(&checked["copy"]["reasons"])
                )));
            }
            let container = match container {
                Container::Auto if fits => Container::Mp4,
                Container::Auto => Container::Mkv,
                Container::Mp4 if !fits => bail!(Refused(format!(
                    "unsupported: {} cannot be stream-copied into MP4; MKV can hold it",
                    first_video.codec
                ))),
                other => *other,
            };
            (
                container,
                crate::playback::media_copy::named(&parent.join(filename), container),
            )
        }
        Mode::Encode { .. } => (Container::Mp4, parent.join(filename)),
    };
    if destination.exists() {
        bail!("no-clobber: {} already exists", destination.display());
    }
    let total: f64 = parts.iter().map(|(p, _)| p.end - p.start).sum();
    let target = media_sources::target(first_video, &encoding);
    let scratch = match &request.scratch {
        Some(root) => {
            std::fs::create_dir_all(root).context("the export-parts folder could not be made")?;
            root.clone()
        }
        None => parent.clone(),
    };
    let bytes: f64 = parts
        .iter()
        .map(|(part, entry)| part_bytes(entry, part.end - part.start, &request.mode, target))
        .sum();
    check_space(bytes, &scratch, &parent)?;
    let work = tempfile::Builder::new()
        .prefix(WORK_PREFIX)
        .tempdir_in(&scratch)
        .context("the temporary folder could not be created")?;
    claim(work.path())?;
    // A copied part is Matroska whatever the output: it has no edit list,
    // so the frames FFmpeg copies past a part's end (it cuts in decode
    // order) are presented and counted in its duration, and the next part
    // starts after them. In MP4 an edit list hides them, the concat demuxer
    // starts the next part at the edit's end and their timestamps collide
    // ("non monotonically increasing dts" at every join, measured).
    let extension = match &request.mode {
        Mode::Copy { .. } => "mkv",
        Mode::Encode { .. } => "mp4",
    };
    let mut files = Vec::with_capacity(parts.len());
    let mut written = Vec::with_capacity(parts.len());
    let mut before = 0.;
    for (index, ((part, entry), lease)) in parts.iter().zip(&leases).enumerate() {
        let length = part.end - part.start;
        control.set_segment(Some(Segment {
            part: index + 1,
            parts: parts.len(),
            before,
            length,
            total,
        }));
        let file = work.path().join(format!("part-{index:03}.{extension}"));
        let part_facts = facts[index];
        let video = part_facts.video.as_ref().context("part without video")?;
        let mut reader = lease.lock_reader()?;
        match &request.mode {
            Mode::Copy { .. } => {
                let audio = part_facts.audio.as_ref().map(|a| AudioTrack {
                    track_id: 0,
                    stream_index: Some(a.index),
                    selected: true,
                    external: false,
                    codec: Some(a.codec.clone()),
                    title: None,
                    language: None,
                });
                let Some(copied) = crate::playback::media_copy::write(
                    &crate::playback::media_copy::Job {
                        ffmpeg: &request.ffmpeg,
                        video_stream: video.index,
                        start: part.start,
                        end: part.end,
                        audio: audio.as_ref(),
                        container: Container::Mkv,
                        origin: part_facts.origin,
                        streams: &probed[index]["streams"],
                        destination: &file,
                    },
                    &mut reader,
                    &file,
                    control,
                )
                .with_context(|| part_name(entry, index))?
                else {
                    return Ok(Outcome::Cancelled);
                };
                written.push(json!({"start":copied.start,"end":copied.end}));
            }
            Mode::Encode { encoding } => {
                let mut command = encode_command(
                    &request.ffmpeg,
                    part,
                    part_facts,
                    &probed[index],
                    target,
                    any_audio,
                    encoding,
                )?;
                command.arg(&file);
                match run_tool(&mut command, reader.stdin()?, control, Some(length))? {
                    None => return Ok(Outcome::Cancelled),
                    Some((status, stderr)) if !status.success() => bail!(
                        "{}: the encoder failed ({status}): {}",
                        part_name(entry, index),
                        String::from_utf8_lossy(&stderr).trim()
                    ),
                    Some(_) => {}
                }
                written.push(json!({"start":part.start,"end":part.end}));
            }
        }
        reader.validate().with_context(|| {
            format!(
                "source-unavailable: {} changed while it was read",
                part_name(entry, index)
            )
        })?;
        files.push(file);
        before += length;
    }
    if control.status() == JobStatus::Cancelling {
        return Ok(Outcome::Cancelled);
    }
    // The last part's copy facts (`media_copy::write`) are not the export's:
    // `facts.join` below has every part.
    let mut stale = control.facts();
    if let Some(facts) = stale.as_object_mut() {
        facts.remove("copy");
    }
    control.record_facts(stale);
    // All parts are written: what remains is quick (copying them into one).
    control.set_segment(Some(Segment {
        part: parts.len(),
        parts: parts.len(),
        before: total,
        length: 0.,
        total,
    }));
    let list = work.path().join("parts.txt");
    let mut text = String::from("ffconcat version 1.0\n");
    for file in &files {
        text.push_str(&concat_line(file)?);
    }
    std::fs::write(&list, text).context("the part list could not be written")?;
    // Beside the destination, for a no-clobber rename into place (as every
    // export); named so a leftover is recognisable.
    let temp = tempfile::Builder::new()
        .prefix(".vestra-export-")
        .suffix(".tmp")
        .tempfile_in(&parent)
        .context("owned temporary output could not be created")?;
    let mut command = crate::platform::background_tool_command(&request.ffmpeg);
    command
        .args([
            "-nostdin", "-v", "error", "-f", "concat", "-safe", "0", "-i",
        ])
        .arg(&list)
        .args([
            "-map",
            "0:v",
            "-map",
            "0:a?",
            "-c",
            "copy",
            "-map_chapters",
            "-1",
        ]);
    if container == Container::Mp4 {
        if matches!(request.mode, Mode::Copy { .. }) && first_video.codec == "hevc" {
            command.args(["-tag:v", "hvc1"]);
        }
        command.args(["-movflags", "+faststart", "-f", "mp4"]);
    } else {
        command.args(["-f", "matroska"]);
    }
    command.arg("-y").arg(temp.path());
    match run_tool(&mut command, Stdio::null(), control, None)? {
        None => return Ok(Outcome::Cancelled),
        Some((status, stderr)) if !status.success() => bail!(
            "joining the parts failed ({status}): {}",
            String::from_utf8_lossy(&stderr).trim()
        ),
        Some(_) => {}
    }
    let (threads, limit) = match &request.mode {
        Mode::Copy { .. } => (2, Some(DECODE_CHECK_SECONDS)),
        Mode::Encode { encoding } => (encoding.threads(), None),
    };
    if decode_check(&request.ffmpeg, temp.path(), threads, limit, control)?.is_none() {
        return Ok(Outcome::Cancelled);
    }
    // A copy: every join must decode too, not only the start (a re-encode
    // was decoded whole above). Each part starts where the ones before it
    // end in the output.
    if matches!(request.mode, Mode::Copy { .. }) {
        let mut at = 0.;
        for w in written.iter().take(written.len().saturating_sub(1)) {
            at += w["end"].as_f64().unwrap_or(0.) - w["start"].as_f64().unwrap_or(0.);
            let from = (at - 0.25).max(0.);
            if decode_at(
                &request.ffmpeg,
                temp.path(),
                from,
                DECODE_CHECK_SECONDS + 0.25,
                control,
            )?
            .is_none()
            {
                return Ok(Outcome::Cancelled);
            }
        }
    }
    let mut probe = crate::platform::tool_command(sibling(&request.ffmpeg, "ffprobe")?);
    probe
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=index,codec_name,codec_type,width,height,sample_rate,channels,duration,r_frame_rate,avg_frame_rate:format=duration,format_name",
            "-of",
            "json",
        ])
        .arg(temp.path());
    let Some((output, _)) = capture_command(&mut probe, control)? else {
        return Ok(Outcome::Cancelled);
    };
    let output: Value = serde_json::from_slice(&output).context("joined file probe invalid")?;
    let duration = output["format"]["duration"]
        .as_str()
        .and_then(|d| d.parse::<f64>().ok())
        .filter(|d| d.is_finite() && *d > 0.)
        .context("joined file duration unavailable")?;
    // Each part from where it was written to its requested end: a copy's
    // last packets past the end are cut by its own edit, so its "end" can
    // exceed what concat shows.
    let expected: f64 = written
        .iter()
        .zip(parts)
        .map(|(w, (part, _))| {
            w["end"].as_f64().unwrap_or(0.).min(part.end) - w["start"].as_f64().unwrap_or(0.)
        })
        .sum();
    let mut recorded = control.facts();
    recorded["output"] = output;
    recorded["join"] = json!({
        "mode": if matches!(request.mode, Mode::Copy { .. }) { "copy" } else { "mp4" },
        "container": if container == Container::Mkv { "mkv" } else { "mp4" },
        "parts": written,
        "target": matches!(request.mode, Mode::Encode { .. }).then(|| json!({
            "width": target.width,
            "height": target.height,
            "fps": f64::from(target.rate.0) / f64::from(target.rate.1),
        })),
    });
    control.record_facts(recorded);
    // One frame per part of slack, and 50 ms of rounding.
    let tolerance = parts.len() as f64 / first_video.fps() + 0.05;
    if duration + tolerance < expected {
        bail!(
            "the joined file is {duration}s long, shorter than the parts' {expected}s beyond {tolerance}s"
        );
    }
    for (lease, (_, entry)) in leases.iter().zip(parts) {
        lease
            .lock_reader()?
            .validate()
            .with_context(|| format!("source-unavailable: {} changed", entry.name))?;
    }
    if control.status() == JobStatus::Cancelling || !control.publish() {
        return Ok(Outcome::Cancelled);
    }
    temp.as_file()
        .sync_all()
        .context("temporary output could not be flushed")?;
    temp.persist_noclobber(&destination)
        .map_err(|error| anyhow::anyhow!("no-clobber publication failed: {}", error.error))?;
    control.set_segment(None);
    control.set_progress(total, total);
    Ok(Outcome::Completed {
        destination,
        range: None,
    })
}

/// The copy check's reasons in a line (the outcome's message).
fn reasons(reasons: &Value) -> String {
    reasons
        .as_array()
        .map(|all| {
            all.iter()
                .map(|r| {
                    format!(
                        "part {} {} {} (part 1: {})",
                        r["part"].as_u64().unwrap_or(0) + 1,
                        r["field"].as_str().unwrap_or("?"),
                        r["value"],
                        r["expected"]
                    )
                })
                .collect::<Vec<_>>()
                .join("; ")
        })
        .unwrap_or_default()
}

/// The FFmpeg command that re-encodes one part to the join's format (the
/// output path is appended by the caller). Only numbers and stream indices
/// go into the filter graph.
fn encode_command(
    ffmpeg: &Path,
    part: &Part,
    facts: &media_sources::Facts,
    probed: &Value,
    target: media_sources::Target,
    with_audio: bool,
    encoding: &Encoding,
) -> Result<std::process::Command> {
    let video = facts.video.as_ref().context("part without video")?;
    let length = part.end - part.start;
    let threads = encoding.threads().to_string();
    let mut command = crate::platform::background_tool_command(ffmpeg);
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
    // Seek the held file (FFmpeg decodes from the keyframe before) in
    // containers that seek exactly, keeping the normalized timeline; the
    // audio decoder gets a second before the start to warm up.
    let at = (part.start
        - if facts.audio.is_some() {
            AUDIO_WARMUP_SECONDS
        } else {
            0.
        })
    .floor();
    let seek = seeks_exactly(&facts.format) && at > 0.;
    if seek {
        command.args(["-ss", &secs(at), "-copyts", "-start_at_zero"]);
    }
    command.args(["-i", crate::platform::HELD_FILE_INPUT]);
    let (width, height) = (target.width, target.height);
    let mut graph = format!(
        "[0:{index}]scale=w='max(1,round(iw*sar))':h=ih,setsar=1,settb=AVTB,trim=start={start}:end={end},setpts=PTS-STARTPTS,scale=w={width}:h={height}:force_original_aspect_ratio=decrease:force_divisible_by=2,pad=w={width}:h={height}:x=(ow-iw)/2:y=(oh-ih)/2:color=black,setsar=1,fps={n}/{d},format=yuv420p[v]",
        index = video.index,
        start = secs(part.start),
        end = secs(part.end),
        n = target.rate.0,
        d = target.rate.1,
    );
    if with_audio {
        match &facts.audio {
            Some(audio) => {
                let base = if seek { at } else { 0. };
                let chain =
                    crate::playback::offline_audio::audio_from(&[], 0, part.start, part.end, base)?;
                graph.push_str(&format!(
                    ";[0:{}]{chain},aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo[a]",
                    audio.index
                ));
            }
            None => graph.push_str(&format!(
                ";anullsrc=r=48000:cl=stereo,atrim=duration={}[a]",
                secs(length)
            )),
        }
    }
    command.args(["-filter_complex", &graph, "-map", "[v]"]);
    if with_audio {
        command.args([
            "-map",
            "[a]",
            "-c:a",
            "aac",
            "-ar",
            "48000",
            "-ac",
            "2",
            "-b:a",
            &format!("{}k", encoding.audio_bitrate()),
        ]);
    } else {
        command.arg("-an");
    }
    match encoding.video_bitrate {
        None => command.args(["-crf", &encoding.crf().to_string()]),
        Some(VideoBitrate::Source) => {
            let bits = source_bitrate(probed, video.index)
                .ok_or_else(|| Refused("unsupported: a part's video bitrate is unknown".into()))?;
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
        &secs(length),
        "-c:v",
        "libx264",
        "-preset",
        encoding.preset(),
        "-pix_fmt",
        "yuv420p",
        "-threads",
        &threads,
        "-progress",
        "pipe:1",
        "-stats_period",
        "0.25",
        "-f",
        "mp4",
        "-y",
    ]);
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playback::media_worker::{CancelResult, real};
    use std::{
        process::Command,
        sync::Arc,
        thread,
        time::{Duration, SystemTime},
    };

    #[test]
    fn concat_lines_quote_any_windows_or_unix_path() {
        for (path, line) in [
            (
                r"C:\Users\Zoë O'Brien\Videos\.vestra-join-x\part-000.mp4",
                r"file 'C:\Users\Zoë O'\''Brien\Videos\.vestra-join-x\part-000.mp4'",
            ),
            (
                r"D:\视频 [2026] #1; a,b=c\part-001.mkv",
                r"file 'D:\视频 [2026] #1; a,b=c\part-001.mkv'",
            ),
            (
                r"\\server\share\it's\p.mp4",
                r"file '\\server\share\it'\''s\p.mp4'",
            ),
            (
                "/home/u/My 'quoted' dir/p.mp4",
                r"file '/home/u/My '\''quoted'\'' dir/p.mp4'",
            ),
        ] {
            assert_eq!(concat_line(Path::new(path)).unwrap(), format!("{line}\n"));
        }
        assert!(concat_line(Path::new("/tmp/a\nfile '/etc/passwd'")).is_err());
    }

    // ---- real FFmpeg (ignored; ci:local runs them with --include-ignored)

    /// A test source made by FFmpeg: `seconds` of a counter pattern at
    /// `size` and `rate`, keyframes every second, with a tone when `audio`
    /// (`rate` Hz, `channels`).
    fn source(
        dir: &Path,
        name: &str,
        size: &str,
        rate: &str,
        audio: Option<(u32, u32)>,
        seconds: u32,
    ) -> PathBuf {
        let path = dir.join(name);
        let mut command = Command::new(real::ffmpeg());
        command
            .args(["-v", "error", "-f", "lavfi", "-i"])
            .arg(format!(
                "testsrc2=size={size}:rate={rate}:duration={seconds}"
            ));
        if let Some((rate, channels)) = audio {
            command.args(["-f", "lavfi", "-i"]).arg(format!(
                "sine=frequency=440:sample_rate={rate}:duration={seconds},aformat=channel_layouts={}",
                if channels == 1 { "mono" } else { "stereo" }
            ));
        }
        command.args(["-c:v", "libx264", "-g", rate, "-pix_fmt", "yuv420p"]);
        if audio.is_some() {
            command.args(["-c:a", "aac"]);
        }
        let out = command.arg("-y").arg(&path).output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        path
    }

    /// The reference the host would store for `path`.
    fn entry(path: &Path) -> Entry {
        let lease = Lease::open(path).unwrap();
        let probed = media_sources::probe(&real::ffmpeg(), &lease, &Control::new())
            .unwrap()
            .unwrap();
        let facts = lease.facts();
        Entry {
            path: path.to_path_buf(),
            len: facts.len,
            modified: media_sources::nanos(facts.modified),
            name: media_sources::display_name(path),
            facts: media_sources::facts_from(&probed).unwrap(),
        }
    }

    fn part(entry: &Entry, start: f64, end: f64) -> (Part, Entry) {
        (
            Part {
                source: "t".into(),
                start,
                end,
            },
            entry.clone(),
        )
    }

    /// Runs a join on a thread; the parts seen in progress come back too.
    fn run_join(request: JoinRequest) -> (Arc<Control>, Vec<(usize, usize)>) {
        let control = Arc::new(Control::new());
        let worker = control.clone();
        let thread = thread::spawn(move || run(request, &worker));
        let mut parts = Vec::new();
        while !thread.is_finished() {
            if let Some(part) = control.part()
                && parts.last() != Some(&part)
            {
                parts.push(part);
            }
            thread::sleep(Duration::from_millis(5));
        }
        thread.join().unwrap();
        (control, parts)
    }

    fn completed(control: &Control) -> PathBuf {
        match control.snapshot().1 {
            Some(Outcome::Completed { destination, .. }) => destination,
            other => panic!("{other:?}: {}", control.facts()),
        }
    }

    /// Two ranges of one file and one of a copy of it: a copy join, MP4,
    /// starting each part on its keyframe (every second), as long as the
    /// written ranges; progress went through each part.
    #[test]
    #[ignore = "needs ffmpeg"]
    fn a_copy_join_of_compatible_parts_writes_them_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let one = source(dir.path(), "one.mp4", "320x240", "30", Some((48000, 2)), 8);
        let two = dir.path().join("two it's ü.mp4");
        std::fs::copy(&one, &two).unwrap();
        let (a, b) = (entry(&one), entry(&two));
        let out = dir.path().join("out [x] 'q'.mkv");
        let (control, seen) = run_join(JoinRequest {
            parts: vec![part(&a, 2.5, 4.), part(&b, 1., 2.), part(&a, 6., 7.5)],
            destination: out.clone(),
            ffmpeg: real::ffmpeg(),
            scratch: None,
            mode: Mode::Copy {
                container: Container::Auto,
            },
        });
        let destination = completed(&control);
        // `auto`: H.264 + AAC fit MP4, so the extension follows.
        assert_eq!(
            destination,
            dunce::canonicalize(dir.path())
                .unwrap()
                .join("out [x] 'q'.mp4")
        );
        let facts = control.facts();
        let written = facts["join"]["parts"].as_array().unwrap();
        // Each part starts on the keyframe at or before its start.
        for (w, start) in written.iter().zip([2., 1., 6.]) {
            assert!((w["start"].as_f64().unwrap() - start).abs() < 0.05, "{w}");
        }
        let probed = real::probe(
            &destination,
            &["-show_entries", "format=duration:stream=codec_type"],
        );
        let duration: f64 = probed["format"]["duration"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        // 2 + 1 + 1.5 s written from keyframes, each part's last frames
        // and audio packets rounding up.
        assert!((4.45..4.9).contains(&duration), "{duration}");
        // Every frame and sample decodes, across the joins too.
        let decoded = Command::new(real::ffmpeg())
            .args(["-v", "error", "-xerror", "-i"])
            .arg(&destination)
            .args(["-f", "null", "-"])
            .output()
            .unwrap();
        assert!(
            decoded.status.success(),
            "{}",
            String::from_utf8_lossy(&decoded.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&decoded.stderr), "");
        assert_eq!(probed["streams"].as_array().unwrap().len(), 2);
        assert_eq!(seen.first(), Some(&(1, 3)));
        assert!(seen.contains(&(2, 3)) && seen.contains(&(3, 3)), "{seen:?}");
        assert_eq!(control.progress().map(|(d, t)| d == t), Some(true));
        // Only the export's own facts: no part's copy facts left over.
        assert!(facts.get("copy").is_none(), "{facts}");
        // The temporary folder is gone.
        let left: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left.len(), 3, "{left:?}");
    }

    /// Different sizes, rates and audio (one part without): copy is refused
    /// naming why; encode writes the first part's size and rate with
    /// 48 kHz stereo audio throughout.
    #[test]
    #[ignore = "needs ffmpeg"]
    fn an_encode_join_of_mismatched_parts_takes_the_first_parts_format() {
        let dir = tempfile::tempdir().unwrap();
        let wide = entry(&source(
            dir.path(),
            "wide.mp4",
            "640x360",
            "30",
            Some((44100, 1)),
            4,
        ));
        let tall = entry(&source(dir.path(), "tall.mp4", "240x320", "25", None, 4));
        let big = entry(&source(
            dir.path(),
            "big.mp4",
            "1280x720",
            "60",
            Some((48000, 2)),
            4,
        ));
        let parts = vec![
            part(&wide, 0.5, 2.),
            part(&tall, 1., 2.),
            part(&big, 3., 3.5),
        ];
        let (control, _) = run_join(JoinRequest {
            parts: parts.iter().map(|(p, e)| (p.clone(), e.clone())).collect(),
            destination: dir.path().join("copy.mp4"),
            ffmpeg: real::ffmpeg(),
            scratch: None,
            mode: Mode::Copy {
                container: Container::Auto,
            },
        });
        match control.snapshot().1 {
            Some(Outcome::Failed { code, message }) => {
                assert_eq!(code, "unsupported");
                assert!(message.contains("part 2 size"), "{message}");
            }
            other => panic!("{other:?}"),
        }
        assert!(!dir.path().join("copy.mp4").exists());
        let (control, _) = run_join(JoinRequest {
            parts,
            destination: dir.path().join("joined.mp4"),
            ffmpeg: real::ffmpeg(),
            scratch: None,
            mode: Mode::Encode {
                encoding: Encoding::default(),
            },
        });
        let destination = completed(&control);
        let probed = real::probe(
            &destination,
            &[
                "-show_entries",
                "format=duration:stream=codec_type,codec_name,width,height,avg_frame_rate,sample_rate,channels,duration",
            ],
        );
        let streams = probed["streams"].as_array().unwrap();
        let video = streams.iter().find(|s| s["codec_type"] == "video").unwrap();
        assert_eq!(
            (video["width"].as_u64(), video["height"].as_u64()),
            (Some(640), Some(360))
        );
        assert_eq!(video["codec_name"], "h264");
        assert_eq!(video["avg_frame_rate"], "30/1");
        let audio = streams.iter().find(|s| s["codec_type"] == "audio").unwrap();
        assert_eq!(audio["sample_rate"], "48000");
        assert_eq!(audio["channels"], 2);
        // The sound runs as long as the picture: each part was padded or
        // trimmed to its own length, the silent one filled.
        let seconds = |s: &Value| s["duration"].as_str().unwrap().parse::<f64>().unwrap();
        assert!((seconds(audio) - seconds(video)).abs() < 0.1, "{probed}");
        let duration: f64 = probed["format"]["duration"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!((duration - 3.).abs() < 0.15, "{duration}");
        assert_eq!(control.facts()["join"]["target"]["width"], 640);
        // Every frame and sample decodes, across the joins too.
        let decoded = Command::new(real::ffmpeg())
            .args(["-v", "error", "-xerror", "-i"])
            .arg(&destination)
            .args(["-f", "null", "-"])
            .output()
            .unwrap();
        assert!(decoded.status.success());
        assert_eq!(String::from_utf8_lossy(&decoded.stderr), "");
    }

    /// Cancelling a running join leaves nothing: no output, no temporary
    /// folder.
    #[test]
    #[ignore = "needs ffmpeg"]
    fn cancelling_a_join_leaves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let long = entry(&source(
            dir.path(),
            "long.mp4",
            "1280x720",
            "30",
            Some((48000, 2)),
            40,
        ));
        let control = Arc::new(Control::new());
        let worker = control.clone();
        let request = JoinRequest {
            parts: vec![part(&long, 0., 20.), part(&long, 20., 40.)],
            destination: dir.path().join("joined.mp4"),
            ffmpeg: real::ffmpeg(),
            scratch: None,
            mode: Mode::Encode {
                encoding: Encoding::default(),
            },
        };
        let thread = thread::spawn(move || run(request, &worker));
        while control.progress().is_none_or(|(done, _)| done <= 0.) && !thread.is_finished() {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(control.cancel(), CancelResult::Requested);
        thread.join().unwrap();
        assert_eq!(control.snapshot().1, Some(Outcome::Cancelled));
        let left: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left, [std::ffi::OsString::from("long.mp4")]);
    }

    /// The intermediate parts go to the player's own folder, which is
    /// empty again after the export and is emptied at start-up.
    #[test]
    #[ignore = "needs ffmpeg"]
    fn intermediates_live_in_the_scratch_folder_and_leave_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let videos = dir.path().join("Videos");
        let scratch = dir.path().join("cache").join("export-parts");
        std::fs::create_dir_all(&videos).unwrap();
        let a = entry(&source(
            &videos,
            "a.mp4",
            "320x240",
            "30",
            Some((48000, 2)),
            4,
        ));
        let (control, _) = run_join(JoinRequest {
            parts: vec![part(&a, 0., 1.5), part(&a, 2., 3.5)],
            destination: videos.join("out.mp4"),
            ffmpeg: real::ffmpeg(),
            scratch: Some(scratch.clone()),
            mode: Mode::Copy {
                container: Container::Auto,
            },
        });
        completed(&control);
        let names = |dir: &Path| -> Vec<String> {
            std::fs::read_dir(dir)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        };
        assert_eq!(names(&videos).len(), 2, "{:?}", names(&videos));
        assert!(names(&scratch).is_empty(), "{:?}", names(&scratch));
        // A crash's leftovers go at a later start; nothing else does.
        std::fs::create_dir_all(scratch.join(".vestra-join-old")).unwrap();
        std::fs::write(scratch.join(".vestra-join-old").join("part-000.mkv"), b"x").unwrap();
        std::fs::write(scratch.join("someone-else.txt"), b"keep").unwrap();
        clean_scratch_with(
            &scratch,
            crate::platform::process_start,
            std::time::SystemTime::now() + UNOWNED_KEPT,
        );
        assert_eq!(names(&scratch), ["someone-else.txt"]);
        // A root that is not a real directory is left alone.
        let file = dir.path().join("not-a-dir");
        std::fs::write(&file, b"x").unwrap();
        clean_scratch(&file);
        assert!(file.is_file());
    }

    /// Start-up removes only the export folders whose owner has ended: a
    /// running owner's folder stays (another player of the same cache is
    /// exporting into it), an ended or replaced owner's goes, and a folder
    /// without an owner (an older player's) goes only once it is a day old.
    #[test]
    fn start_up_removes_only_folders_whose_owner_has_ended() {
        use crate::platform::ProcessStart;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let folder = |name: &str, owner: Option<&str>| {
            let path = root.join(format!("{WORK_PREFIX}{name}"));
            std::fs::create_dir(&path).unwrap();
            std::fs::write(path.join("part-000.mp4"), b"x").unwrap();
            if let Some(owner) = owner {
                std::fs::write(path.join(OWNER_FILE), owner).unwrap();
            }
            path
        };
        let names = || -> Vec<String> {
            let mut names: Vec<_> = std::fs::read_dir(root)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        };
        // This process, through the real check: its folder stays, even a
        // day later.
        let mine = folder("mine", None);
        claim(&mine).unwrap();
        let (pid, start) = owner(&mine).unwrap();
        assert_eq!(pid, std::process::id());
        // Its id with another start: the id was reused, the owner ended.
        folder("reused", Some(&format!("{pid}\nnot-{start}\n")));
        clean_scratch_with(
            root,
            crate::platform::process_start,
            SystemTime::now() + UNOWNED_KEPT,
        );
        assert_eq!(names(), [".vestra-join-mine"]);

        // Through a given check and clock.
        let owners = |pid: u32| match pid {
            1 => ProcessStart::Running("a".into()),
            2 => ProcessStart::Gone,
            _ => ProcessStart::Unknown,
        };
        let reset = || {
            for name in names() {
                std::fs::remove_dir_all(root.join(name)).unwrap();
            }
            folder("alive", Some("1\na\n"));
            folder("restarted", Some("1\nb\n"));
            folder("dead", Some("2\na\n"));
            folder("unknown", Some("3\na\n"));
            folder("legacy", None);
            folder("unmarked", Some("1\n\n"));
        };
        // Now: the alive owner's folder stays, the ended ones go, the
        // folders without a known owner stay while they are new.
        reset();
        clean_scratch_with(root, owners, SystemTime::now());
        assert_eq!(
            names(),
            [
                ".vestra-join-alive",
                ".vestra-join-legacy",
                ".vestra-join-unknown",
                ".vestra-join-unmarked"
            ]
        );
        // A day on: only the alive owner's folder stays.
        reset();
        clean_scratch_with(root, owners, SystemTime::now() + UNOWNED_KEPT);
        assert_eq!(names(), [".vestra-join-alive"]);
    }

    /// A link (or junction) named like an export folder is removed as a
    /// link: what it points to is never entered.
    #[test]
    fn a_link_among_the_export_folders_is_removed_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("export-parts");
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("keep.mp4"), b"x").unwrap();
        // An ended owner, as a folder would be removed for.
        std::fs::write(elsewhere.join(OWNER_FILE), "2\na\n").unwrap();
        let link = root.join(format!("{WORK_PREFIX}link"));
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(&elsewhere, &link).is_ok()
            // No symbolic-link privilege: a junction (no privilege needed).
            || std::process::Command::new("cmd")
                .arg("/C")
                .arg("mklink")
                .arg("/J")
                .arg(&link)
                .arg(&elsewhere)
                .stdout(std::process::Stdio::null())
                .status()
                .is_ok_and(|status| status.success());
        #[cfg(not(windows))]
        let made = std::os::unix::fs::symlink(&elsewhere, &link).is_ok();
        assert!(made, "no link could be made");
        clean_scratch_with(
            &root,
            |_| crate::platform::ProcessStart::Gone,
            SystemTime::now() + UNOWNED_KEPT,
        );
        assert!(std::fs::symlink_metadata(&link).is_err(), "the link stays");
        assert!(elsewhere.join("keep.mp4").is_file());
        assert!(elsewhere.join(OWNER_FILE).is_file());
    }

    /// The check takes a snapshot's clip too: one part from its held file;
    /// a processed snapshot cannot be copied.
    #[test]
    #[ignore = "needs ffmpeg"]
    fn a_snapshot_clip_is_checked_as_one_part() {
        let dir = tempfile::tempdir().unwrap();
        let path = source(dir.path(), "a.mp4", "320x240", "30", Some((48000, 2)), 4);
        let ask = |presentation: Option<(u32, u32)>| {
            let (events, replies) = std::sync::mpsc::channel();
            media_sources::check_snapshot(
                &events,
                crate::host::Reply { owner: 9, id: 1 },
                "test:snapshot".into(),
                media_sources::SnapshotInput {
                    lease: Arc::new(Lease::open(&path).unwrap()),
                    start: 1.,
                    end: 3.,
                    presentation,
                },
                Encoding::default(),
            );
            match replies.recv_timeout(Duration::from_secs(60)).unwrap() {
                crate::host::Event::Reply(_, Ok(value)) => value,
                _ => panic!("no answer"),
            }
        };
        let raw = ask(None);
        assert_eq!(raw["blocking"], json!([]), "{raw}");
        assert_eq!(raw["copy"], json!({"ok":true,"container":"mp4"}));
        assert_eq!(raw["encode"]["width"], 320);
        assert_eq!(raw["duration"], 2.0);
        let processed = ask(Some((641, 360)));
        assert_eq!(processed["copy"]["reasons"][0]["field"], "effects");
        assert_eq!(processed["encode"]["width"], 642);
    }

    #[test]
    fn a_join_without_room_fails_no_space_before_writing() {
        let dir = tempfile::tempdir().unwrap();
        // More than any disk has: refused, naming the volume.
        let error = check_space(1e18, dir.path(), dir.path()).unwrap_err();
        let coded = error.downcast_ref::<Coded>().unwrap();
        assert_eq!(coded.0, "no-space");
        assert!(coded.1.contains("GB is free"), "{}", coded.1);
        assert!(check_space(1024., dir.path(), dir.path()).is_ok());
        // The estimate: a copy its share of the file, a re-encode its picture.
        let mut e = entry_facts(1_000_000, 100.);
        let copy = Mode::Copy {
            container: Container::Auto,
        };
        let t = media_sources::Target {
            width: 1920,
            height: 1080,
            rate: (30, 1),
        };
        assert_eq!(part_bytes(&e, 10., &copy, t), 100_000.);
        let encode = Mode::Encode {
            encoding: Encoding::default(),
        };
        let bytes = part_bytes(&e, 10., &encode, t);
        assert!((bytes - (1920. * 1080. * 30. * 0.12 / 8. * 10. + 320_000.)).abs() < 1.);
        e.facts.duration = 0.;
        assert!(part_bytes(&e, 1., &copy, t).is_finite());
    }

    fn entry_facts(len: u64, duration: f64) -> Entry {
        Entry {
            path: PathBuf::from("/x.mp4"),
            len,
            modified: 0,
            name: "x.mp4".into(),
            facts: media_sources::Facts {
                duration,
                origin: 0.,
                format: "mp4".into(),
                video: None,
                audio: None,
            },
        }
    }

    /// A part whose file was moved fails `source-missing`; one rewritten
    /// since it was added fails `source-changed`. Nothing is written.
    #[test]
    #[ignore = "needs ffmpeg"]
    fn a_missing_or_changed_source_fails_with_its_code() {
        let dir = tempfile::tempdir().unwrap();
        let a = entry(&source(dir.path(), "a.mp4", "320x240", "30", None, 3));
        let moved = entry(&source(dir.path(), "b.mp4", "320x240", "30", None, 3));
        std::fs::rename(&moved.path, dir.path().join("elsewhere.mp4")).unwrap();
        let attempt = |parts: Vec<(Part, Entry)>| {
            let (control, _) = run_join(JoinRequest {
                parts,
                destination: dir.path().join("joined.mp4"),
                ffmpeg: real::ffmpeg(),
                scratch: None,
                mode: Mode::Copy {
                    container: Container::Auto,
                },
            });
            match control.snapshot().1 {
                Some(Outcome::Failed { code, message }) => (code, message),
                other => panic!("{other:?}"),
            }
        };
        let (code, message) = attempt(vec![part(&a, 0., 1.), part(&moved, 0., 1.)]);
        assert_eq!(code, "source-missing");
        assert!(message.contains("part 2 (b.mp4)"), "{message}");
        let mut stale = a.clone();
        stale.len += 1;
        let (code, _) = attempt(vec![part(&stale, 0., 1.)]);
        assert_eq!(code, "source-changed");
        assert!(!dir.path().join("joined.mp4").exists());
    }
}
