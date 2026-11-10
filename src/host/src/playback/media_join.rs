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
    let work = tempfile::Builder::new()
        .prefix(".vestra-join-")
        .tempdir_in(&parent)
        .context("the temporary folder could not be created")?;
    let total: f64 = parts.iter().map(|(p, _)| p.end - p.start).sum();
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
    let target = media_sources::target(first_video, &encoding);
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
    let temp = tempfile::NamedTempFile::new_in(&parent)
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
    use std::{process::Command, sync::Arc, thread, time::Duration};

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
