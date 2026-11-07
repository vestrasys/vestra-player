//! Stream-copied clips ("Fast" cut, host-internal; runs on the media worker).
//! The video stream and at most one audio stream are copied packet for packet
//! from the held source, without decoding or encoding. A copy can only begin on
//! a keyframe, so it starts at the video keyframe at or before the requested
//! start (the first keyframe when the video itself starts later) and is cut at
//! the requested end; the times actually written are
//! measured from the output and reported. No arbitrary FFmpeg strings: the
//! command is built from the probed stream indices and a closed container set.
#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::playback::{
    media_tracks::AudioTrack,
    media_worker::{
        Container, Control, Refused, capture_command, capture_input_command, decode_check, rate,
        run_tool, sibling,
    },
    source_lease::Reader,
};

/// Codecs a copy writes into MP4 under `auto`: the ones MP4 players commonly
/// play. Anything else goes into MKV, which holds nearly every codec.
const MP4_VIDEO: &[&str] = &["h264", "hevc", "av1", "mpeg4"];
const MP4_AUDIO: &[&str] = &["aac", "mp3", "ac3", "eac3", "alac"];
/// How far before the start the keyframe search looks, in turn; then from the
/// beginning of the file.
const KEYFRAME_WINDOWS: [f64; 3] = [10., 60., 300.];
/// A video that starts after the requested start: its first keyframe is
/// searched forward in 10 s steps, at most 5 minutes in.
const FIRST_KEYFRAME_STEP: f64 = 10.;
const FIRST_KEYFRAME_STEPS: u32 = 30;
/// Packets read at the head of the copy for its first presented frame: open
/// GOPs write leading frames (earlier presentation times) after the keyframe.
const HEAD_PACKETS: u32 = 64;
/// The copied file's first seconds are decoded before publication.
const DECODE_CHECK_SECONDS: f64 = 3.;
/// FFmpeg's refusals of a codec in a container.
const MUXER_REFUSALS: &[&str] = &[
    "not currently supported in container",
    "Could not find tag for codec",
    "not supported in this container",
];

pub struct Job<'a> {
    pub ffmpeg: &'a Path,
    pub video_stream: u32,
    /// Requested range, normalized source seconds (mpv's media time).
    pub start: f64,
    pub end: f64,
    pub audio: Option<&'a AudioTrack>,
    pub container: Container,
    /// The held file's container start time (normalized time 0).
    pub origin: f64,
    /// ffprobe's `streams` of the held file.
    pub streams: &'a Value,
    /// The asked destination (canonical directory); the copy is published
    /// under [`named`] for its container.
    pub destination: &'a Path,
}

#[derive(Debug)]
pub struct Copied {
    /// `Mp4` or `Mkv`, never `Auto`.
    pub container: Container,
    /// The first video frame (the keyframe) and the end of the last video
    /// frame, normalized source seconds.
    pub start: f64,
    pub end: f64,
}

/// Whether `auto` writes these codecs into MP4.
pub fn fits_mp4(video: &str, audio: Option<&str>) -> bool {
    MP4_VIDEO.contains(&video) && audio.is_none_or(|audio| MP4_AUDIO.contains(&audio))
}

/// The destination named for its container: a video extension (`.mp4`,
/// `.m4v`, `.mov`, `.mkv`, `.webm`, any case) is replaced, any other name
/// gets the container's extension appended.
pub fn named(destination: &Path, container: Container) -> PathBuf {
    let extension = match container {
        Container::Mkv => "mkv",
        Container::Mp4 | Container::Auto => "mp4",
    };
    let current = destination
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match current.as_deref() {
        Some(e) if e == extension => destination.to_path_buf(),
        Some("mp4" | "m4v" | "mov" | "mkv" | "webm") => destination.with_extension(extension),
        _ => {
            let mut name = destination.as_os_str().to_owned();
            name.push(".");
            name.push(extension);
            PathBuf::from(name)
        }
    }
}

fn codec(stream: &Value) -> Result<&str, Refused> {
    stream["codec_name"]
        .as_str()
        .filter(|c| !c.is_empty() && !matches!(*c, "none" | "unknown"))
        .ok_or_else(|| {
            Refused(
                "unsupported: a stream's codec is unknown, so it cannot be stream-copied".into(),
            )
        })
}

/// `time_base` `"n/d"` as positive integers.
fn time_base(stream: &Value) -> Option<(i128, i128)> {
    let (n, d) = stream["time_base"].as_str()?.split_once('/')?;
    let (n, d) = (n.parse::<i128>().ok()?, d.parse::<i128>().ok()?);
    (n > 0 && d > 0).then_some((n, d))
}

/// A keyframe ffprobe shows as presented: `K` first and no `D` (a packet an
/// MP4 edit list discards, such as the pre-roll of an `-ss … -c copy` cut).
fn presented_keyframe(flags: &str) -> bool {
    let mut flags = flags.chars();
    flags.next() == Some('K') && flags.next() != Some('D')
}

/// The last presented keyframe at or before `start` from ffprobe's
/// `pts,flags` lines (a bounded tail: a cut first line does not parse and is
/// skipped).
fn last_keyframe(lines: &str, time_base: (i128, i128), start_abs: f64) -> Option<i64> {
    lines
        .lines()
        .filter_map(|line| {
            let (pts, flags) = line.trim().split_once(',')?;
            let pts = pts.parse::<i64>().ok()?;
            (presented_keyframe(flags)
                && (pts as f64 * time_base.0 as f64 / time_base.1 as f64) <= start_abs + 1e-6)
                .then_some(pts)
        })
        .max()
}

/// The earliest presented keyframe among ffprobe's `pts,flags` lines.
fn first_keyframe(lines: &str) -> Option<i64> {
    lines
        .lines()
        .filter_map(|line| {
            let (pts, flags) = line.trim().split_once(',')?;
            presented_keyframe(flags).then(|| pts.parse::<i64>().ok())?
        })
        .min()
}

/// From the copy's first packets (decode order): the first packet's time,
/// which must be a presented keyframe, and the earliest presented time
/// (edit-list `D` packets are not presented).
fn head_times(lines: &str) -> Result<(f64, f64)> {
    let packets: Vec<(f64, &str)> = lines
        .lines()
        .filter_map(|line| {
            let (pts, flags) = line.trim().split_once(',')?;
            Some((pts.parse::<f64>().ok()?, flags))
        })
        .collect();
    let &(first, flags) = packets.first().context("copied clip has no video packet")?;
    if !presented_keyframe(flags) {
        bail!("copied clip does not start on a presented keyframe");
    }
    let earliest = packets
        .iter()
        .filter(|(_, flags)| flags.chars().nth(1) != Some('D'))
        .map(|(pts, _)| *pts)
        .fold(first, f64::min);
    Ok((first, earliest))
}

/// The copy's input seek in whole microseconds: the keyframe's time rounded
/// down, so FFmpeg's backward seek and its "drop packets before the start"
/// rule (`-copypriorss 0`) both keep the keyframe as the first packet.
fn seek_micros(pts: i64, time_base: (i128, i128), origin: f64) -> i64 {
    let key_abs = (i128::from(pts) * time_base.0 * 1_000_000).div_euclid(time_base.1);
    let origin = (origin * 1_000_000.).round() as i128;
    i64::try_from((key_abs - origin).max(0)).unwrap_or(i64::MAX)
}

fn seconds(micros: i64) -> String {
    format!("{}.{:06}", micros / 1_000_000, micros % 1_000_000)
}

pub fn write(
    job: &Job<'_>,
    reader: &mut Reader<'_>,
    temp: &Path,
    control: &Control,
) -> Result<Option<Copied>> {
    let streams = job
        .streams
        .as_array()
        .context("source stream facts unavailable")?;
    let find = |index: u32, kind: &str| {
        streams
            .iter()
            .find(|s| s["index"].as_u64() == Some(u64::from(index)) && s["codec_type"] == kind)
    };
    let video = find(job.video_stream, "video")
        .ok_or_else(|| Refused("unsupported: the source has no video stream to copy".into()))?;
    if video["disposition"]["attached_pic"].as_i64() == Some(1) {
        return Err(Refused(
            "unsupported: the picture is a still cover image, not a video stream to copy".into(),
        )
        .into());
    }
    let video_codec = codec(video)?;
    let audio = match job.audio {
        None => None,
        Some(track) => {
            let index = track.stream_index.ok_or_else(|| {
                Refused("unsupported: an external audio track cannot be stream-copied".into())
            })?;
            let stream = find(index, "audio").ok_or_else(|| {
                Refused("unsupported: the frozen audio stream is not in the source".into())
            })?;
            Some((index, codec(stream)?))
        }
    };
    let fits = fits_mp4(video_codec, audio.map(|(_, codec)| codec));
    let container = match job.container {
        Container::Auto if fits => Container::Mp4,
        Container::Auto => Container::Mkv,
        Container::Mp4 if !fits => {
            let codecs = match audio {
                Some((_, audio)) => format!("{video_codec} + {audio}"),
                None => video_codec.into(),
            };
            return Err(Refused(format!(
                "unsupported: {codecs} cannot be stream-copied into MP4; MKV can hold it"
            ))
            .into());
        }
        other => other,
    };
    // Fail fast when the container's name is taken (publication is still
    // no-clobber; this only saves copying a file that cannot be published).
    let destination = named(job.destination, container);
    if destination.exists() {
        bail!("no-clobber: {} already exists", destination.display());
    }
    let time_base = time_base(video).context("source video time base unavailable")?;

    // The keyframe at or before the start: packet flags only, nothing decoded.
    let start_abs = job.start + job.origin;
    let probe = sibling(job.ffmpeg, "ffprobe")?;
    // A video that starts after the requested start (a first frame at 0.03 s,
    // say) begins on its first keyframe: nothing earlier exists to copy.
    let mut key = None;
    let mut searches: Vec<(String, bool)> = KEYFRAME_WINDOWS
        .iter()
        .map(|w| start_abs - w)
        .take_while(|from| *from > 0.)
        .map(|from| (format!("{from}%{}", start_abs + 0.001), false))
        .collect();
    searches.push((format!("%{}", start_abs + 0.001), false));
    // Then forward from the file's start, in steps short enough that every
    // packet line fits the bounded output (a TS cut mid-GOP starts on
    // non-keyframes; its first keyframe can be seconds in).
    for step in 0..FIRST_KEYFRAME_STEPS {
        let from = job.origin + f64::from(step) * FIRST_KEYFRAME_STEP;
        let interval = if step == 0 {
            // From the file's start without a seek.
            format!("%{}", from + FIRST_KEYFRAME_STEP)
        } else {
            format!("{from}%{}", from + FIRST_KEYFRAME_STEP)
        };
        searches.push((interval, true));
    }
    for (interval, first) in searches {
        let mut command = crate::platform::tool_command(&probe);
        command.args([
            "-v",
            "error",
            "-select_streams",
            &job.video_stream.to_string(),
            "-read_intervals",
            &interval,
            "-show_entries",
            "packet=pts,flags",
            "-of",
            "csv=p=0",
            crate::platform::HELD_FILE_INPUT,
        ]);
        let Some((lines, _)) = capture_input_command(&mut command, reader.stdin()?, control)?
        else {
            return Ok(None);
        };
        let lines = String::from_utf8_lossy(&lines);
        key = if first {
            first_keyframe(&lines)
        } else {
            last_keyframe(&lines, time_base, start_abs)
        };
        if key.is_some() {
            break;
        }
    }
    let key = key.ok_or_else(|| Refused("unsupported: the source video has no keyframe".into()))?;
    reader
        .validate()
        .context("source-unavailable after the keyframe search")?;
    let keyframe = key as f64 * time_base.0 as f64 / time_base.1 as f64 - job.origin;
    let seek = seek_micros(key, time_base, job.origin);
    let length = job.end - seek as f64 / 1_000_000.;
    if !length.is_finite() || length <= 0. {
        bail!("invalid stream-copy range");
    }

    let mut command = crate::platform::background_tool_command(job.ffmpeg);
    command.args([
        "-nostdin",
        "-v",
        "error",
        "-ss",
        &seconds(seek),
        "-i",
        crate::platform::HELD_FILE_INPUT,
        "-t",
        &length.to_string(),
        "-map",
        &format!("0:{}", job.video_stream),
    ]);
    if let Some((index, _)) = audio {
        command.args(["-map", &format!("0:{index}")]);
    }
    command.args(["-c", "copy", "-copypriorss", "0", "-map_chapters", "-1"]);
    if container == Container::Mp4 {
        if video_codec == "hevc" {
            // The tag Apple's players need; FFmpeg's default is hev1.
            command.args(["-tag:v", "hvc1"]);
        }
        command.args(["-movflags", "+faststart"]);
    }
    command.args(["-progress", "pipe:1", "-stats_period", "0.25", "-f"]);
    command.arg(match container {
        Container::Mkv => "matroska",
        Container::Mp4 | Container::Auto => "mp4",
    });
    command.arg("-y").arg(temp);
    match run_tool(
        &mut command,
        reader.stdin()?,
        control,
        Some(job.end - keyframe),
    )? {
        None => return Ok(None),
        Some((status, stderr)) if !status.success() => {
            let stderr = String::from_utf8_lossy(&stderr);
            if MUXER_REFUSALS
                .iter()
                .any(|refusal| stderr.contains(refusal))
            {
                return Err(Refused(format!(
                    "unsupported: the streams cannot be stream-copied into {}: {}",
                    if container == Container::Mkv {
                        "MKV"
                    } else {
                        "MP4"
                    },
                    stderr.trim()
                ))
                .into());
            }
            bail!("stream copy failed ({status}): {}", stderr.trim());
        }
        Some(_) => {}
    }
    reader
        .validate()
        .context("source-unavailable after the stream copy")?;
    if decode_check(job.ffmpeg, temp, 1, Some(DECODE_CHECK_SECONDS), control)?.is_none() {
        return Ok(None);
    }

    // What was written: the output's streams, its first video packet (the
    // keyframe) and the end of its last video frame.
    let probe_output = |args: &[&str]| -> Result<Option<String>> {
        let mut command = crate::platform::tool_command(&probe);
        command.args(["-v", "error"]).args(args).arg(temp);
        Ok(capture_command(&mut command, control)?
            .map(|(out, _)| String::from_utf8_lossy(&out).into_owned()))
    };
    let Some(output) = probe_output(&[
        "-show_entries",
        "stream=index,codec_name,codec_type,width,height,sample_rate,channels,channel_layout,duration,r_frame_rate,avg_frame_rate:format=duration,format_name",
        "-of",
        "json",
    ])?
    else {
        return Ok(None);
    };
    let output: Value = serde_json::from_str(&output).context("copied clip probe invalid")?;
    let mut facts = control.facts();
    facts["output"] = output.clone();
    control.record_facts(facts);
    let duration = output["format"]["duration"]
        .as_str()
        .and_then(|d| d.parse::<f64>().ok())
        .filter(|d| d.is_finite() && *d > 0.)
        .context("copied clip duration unavailable")?;
    let frame = output["streams"]
        .as_array()
        .and_then(|s| s.iter().find(|s| s["codec_type"] == "video"))
        .and_then(|v| rate(&v["avg_frame_rate"]).or(rate(&v["r_frame_rate"])))
        .map_or(0., |rate| 1. / rate);
    let Some(head) = probe_output(&[
        "-select_streams",
        "v:0",
        "-read_intervals",
        &format!("%+#{HEAD_PACKETS}"),
        "-show_entries",
        "packet=pts_time,flags",
        "-of",
        "csv=p=0",
    ])?
    else {
        return Ok(None);
    };
    let (first, earliest) = head_times(&head)?;
    let Some(tail) = probe_output(&[
        // Every packet, read in order without a seek (a seek into the copy
        // can land after its end and print nothing); the bounded output
        // keeps the last lines, which hold the last frames.
        "-select_streams",
        "v:0",
        "-show_entries",
        "packet=pts_time,duration_time",
        "-of",
        "csv=p=0",
    ])?
    else {
        return Ok(None);
    };
    let last = tail
        .lines()
        .filter_map(|line| {
            let (pts, length) = line.trim().split_once(',')?;
            let pts = pts.parse::<f64>().ok()?;
            Some(pts + length.parse::<f64>().unwrap_or(frame))
        })
        .fold(None, |max: Option<f64>, end| {
            Some(max.map_or(end, |m| m.max(end)))
        })
        .with_context(|| {
            format!(
                "copied clip has no final video packet (duration {duration}s, {} tail lines)",
                tail.lines().count()
            )
        })?;
    // The output's first packet is the source keyframe (`-copypriorss 0`):
    // it maps output times to source times. Leading frames of an open GOP
    // present before it, so the start is the earliest presented frame.
    let start = keyframe + (earliest - first);
    let end = keyframe + (last - first);
    let mut facts = control.facts();
    facts["copy"] = json!({"container":if container == Container::Mkv {"mkv"} else {"mp4"},"videoCodec":video_codec,"audioCodec":audio.map(|(_, codec)| codec),"keyframe":keyframe,"seek":seek as f64 / 1_000_000.,"written":{"start":start,"end":end}});
    control.record_facts(facts);
    let tolerance = frame + 0.0001;
    if end + tolerance < job.end {
        bail!(
            "source-unavailable: the copied video ends at {end}s, before the requested {}s beyond one frame ({tolerance}s; output {duration}s, first packet {first}s, last frame end {last}s)",
            job.end
        );
    }
    Ok(Some(Copied {
        container,
        start,
        end,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playback::media_worker::{CancelResult, Outcome, Output, real, run};
    use std::sync::Arc;

    /// The keyframe at or before `time` in the file, read independently.
    fn keyframe_before(file: &Path, time: f64) -> f64 {
        let packets = real::probe(
            file,
            &[
                "-select_streams",
                "v:0",
                "-show_entries",
                "packet=pts_time,flags",
            ],
        );
        packets["packets"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["flags"].as_str().unwrap().starts_with('K'))
            .filter_map(|p| p["pts_time"].as_str()?.parse::<f64>().ok())
            .filter(|t| *t <= time)
            .fold(f64::MIN, f64::max)
    }

    fn completed(control: &Control) -> (PathBuf, f64, f64) {
        match control.snapshot().1 {
            Some(Outcome::Completed {
                destination,
                range: Some((start, end)),
            }) => (destination, start, end),
            other => panic!("{other:?}"),
        }
    }

    /// Video only (bbb 240p, keyframes at 5.07 s and 8.2 s): 6–10 s snaps
    /// back to 5.07 s, the cut ends at about 10 s, the reported range is the
    /// file's, `auto` names the file `.mp4` although `.mkv` was asked, and
    /// progress counted the clip's seconds.
    #[test]
    #[ignore = "needs ffmpeg and the testmedia clips"]
    fn a_copy_starts_on_the_keyframe_and_reports_what_it_wrote() {
        let Some(source) = real::clip("bbb-240p-web.mp4") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let (control, progress) = real::run_job(real::request(
            &source,
            6.0,
            dir.path().join("clip.mkv"),
            Output::Copy {
                end: 10.0,
                audio: None,
                container: Container::Auto,
            },
        ));
        let (destination, start, end) = completed(&control);
        assert_eq!(
            destination,
            dunce::canonicalize(dir.path()).unwrap().join("clip.mp4")
        );
        assert!(!dir.path().join("clip.mkv").exists());
        let key = keyframe_before(&source, 6.0);
        assert!((key - 5.066667).abs() < 0.001, "{key}");
        assert!(
            (start - key).abs() < 0.000_01,
            "start {start} vs keyframe {key}"
        );
        assert!((10.0 - 1. / 30. ..10.5).contains(&end), "end {end}");
        let written = real::probe(&destination, &["-show_entries", "format=duration"]);
        let duration: f64 = written["format"]["duration"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!(
            (duration - (end - start)).abs() < 0.05,
            "{duration} vs {}",
            end - start
        );
        // The first frame of the copy is the source's keyframe frame.
        let (done, total) = progress.expect("progress while copying");
        assert!((total - (10.0 - key)).abs() < 0.001);
        assert!(done > 0.);
        let facts = control.facts();
        assert_eq!(facts["copy"]["container"], "mp4");
        assert_eq!(facts["copy"]["videoCodec"], "h264");
    }

    /// Video and audio from an MKV (H.264 + AAC): `auto` writes MP4 with both
    /// streams copied, `mkv` keeps MKV; the start is the keyframe either way.
    #[test]
    #[ignore = "needs ffmpeg and the testmedia clips"]
    fn video_and_audio_are_copied_into_the_chosen_container() {
        let Some(source) = real::clip("[ryona]kof xiii Athena.mkv") else {
            return;
        };
        let key = keyframe_before(&source, 30.0);
        for (container, name, ext) in [
            (Container::Auto, "auto.mp4", "mp4"),
            (Container::Mkv, "kept.mp4", "mkv"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let (control, _) = real::run_job(real::request(
                &source,
                30.0,
                dir.path().join(name),
                Output::Copy {
                    end: 40.0,
                    audio: Some(real::audio(1)),
                    container,
                },
            ));
            let (destination, start, end) = completed(&control);
            assert_eq!(
                destination.extension().unwrap().to_str().unwrap(),
                ext,
                "{container:?}"
            );
            assert!((start - key).abs() < 0.000_01, "{start} vs {key}");
            assert!((40.0 - 0.034..40.5).contains(&end), "end {end}");
            let streams = real::probe(&destination, &["-show_entries", "stream=codec_name"]);
            let codecs: Vec<_> = streams["streams"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s["codec_name"].as_str().unwrap().to_owned())
                .collect();
            assert_eq!(codecs, ["h264", "aac"]);
        }
    }

    /// PCM audio does not go into MP4: `auto` falls back to MKV, and asking
    /// for MP4 fails with the typed `unsupported`, writing nothing.
    #[test]
    #[ignore = "needs ffmpeg with libx264"]
    fn streams_mp4_cannot_hold_go_to_mkv_or_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("pcm.mkv");
        let made = std::process::Command::new(real::ffmpeg())
            .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i"])
            .arg("testsrc2=size=320x240:rate=25")
            .args(["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000"])
            .args([
                "-t",
                "6",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-g",
                "50",
            ])
            .args(["-pix_fmt", "yuv420p", "-c:a", "pcm_s16le", "-shortest"])
            .arg(&source)
            .status()
            .unwrap();
        assert!(made.success());
        let out = dir.path().join("out");
        std::fs::create_dir(&out).unwrap();
        let copy = |container, name: &str| {
            real::run_job(real::request(
                &source,
                3.0,
                out.join(name),
                Output::Copy {
                    end: 5.0,
                    audio: Some(real::audio(1)),
                    container,
                },
            ))
            .0
        };
        let refused = copy(Container::Mp4, "refused.mp4");
        match refused.snapshot().1 {
            Some(Outcome::Failed { code, message }) => {
                assert_eq!(code, "unsupported");
                assert!(message.contains("pcm_s16le"), "{message}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(std::fs::read_dir(&out).unwrap().count(), 0);
        let auto = copy(Container::Auto, "auto.mp4");
        let (destination, start, _) = completed(&auto);
        assert_eq!(destination.file_name().unwrap(), "auto.mkv");
        // Keyframes every 2 s: 3 s snaps back to 2 s.
        assert!((start - 2.0).abs() < 0.000_01, "{start}");
    }

    /// The 4K60 clip's video begins at 0.033 s (its audio at 0): a copy from 0
    /// starts on that first keyframe and says so.
    #[test]
    #[ignore = "needs ffmpeg and the testmedia clips"]
    fn a_video_that_starts_late_is_copied_from_its_first_keyframe() {
        let Some(source) = real::clip("bbb_sunflower_2160p_60fps_normal.mp4") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let (control, _) = real::run_job(real::request(
            &source,
            0.,
            dir.path().join("first.mp4"),
            Output::Copy {
                end: 2.,
                audio: Some(real::audio(1)),
                container: Container::Auto,
            },
        ));
        let (_, start, end) = completed(&control);
        assert!((start - 2000. / 60000.).abs() < 0.000_01, "{start}");
        assert!((2.0 - 1. / 60. ..2.5).contains(&end), "{end}");
        assert_eq!(control.facts()["copy"]["audioCodec"], "mp3");
    }

    /// `ffmpeg -y <args>`; panics on failure.
    fn make(args: &[&str]) {
        let made = std::process::Command::new(real::ffmpeg())
            .args(["-loglevel", "error", "-y"])
            .args(args)
            .status()
            .unwrap();
        assert!(made.success(), "{args:?}");
    }

    /// 8 s of 25 fps H.264 with B-frames, a keyframe every 2 s (extra x264
    /// parameters, e.g. an open GOP), into `name`.
    fn source(dir: &Path, name: &str, x264: &str) -> PathBuf {
        let path = dir.join(name);
        let params = format!("keyint=50:min-keyint=50:scenecut=0{x264}");
        make(&[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x240:rate=25",
            "-t",
            "8",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-bf",
            "3",
            "-x264-params",
            &params,
            "-pix_fmt",
            "yuv420p",
            path.to_str().unwrap(),
        ]);
        path
    }

    fn copy(source: &Path, start: f64, end: f64, destination: PathBuf) -> Arc<Control> {
        real::run_job(real::request(
            source,
            start,
            destination,
            Output::Copy {
                end,
                audio: None,
                container: Container::Auto,
            },
        ))
        .0
    }

    /// The video packets of a file, decode order: time and flags.
    fn packets(file: &Path) -> Vec<(f64, String)> {
        real::probe(
            file,
            &[
                "-select_streams",
                "v:0",
                "-show_entries",
                "packet=pts_time,flags",
            ],
        )["packets"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|p| {
                Some((
                    p["pts_time"].as_str()?.parse().ok()?,
                    p["flags"].as_str()?.to_owned(),
                ))
            })
            .collect()
    }

    /// The earliest decoded frame time of a file.
    fn decoded_first(file: &Path) -> f64 {
        real::probe(
            file,
            &["-select_streams", "v:0", "-show_entries", "frame=pts_time"],
        )["frames"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|f| f["pts_time"].as_str()?.parse::<f64>().ok())
            .fold(f64::MAX, f64::min)
    }

    /// An edit-list MP4 (any `ffmpeg -ss … -c copy` output) begins with a
    /// discarded pre-roll keyframe (`D`, negative time), which is never a
    /// start: 0–3 s starts on the first presented keyframe, 0.48 s here, and
    /// the job completes.
    #[test]
    #[ignore = "needs ffmpeg with libx264"]
    fn an_edit_list_pre_roll_is_not_a_start() {
        let dir = tempfile::tempdir().unwrap();
        let closed = source(dir.path(), "closed.mp4", "");
        let edited = dir.path().join("edited.mp4");
        make(&[
            "-ss",
            "1.52",
            "-i",
            closed.to_str().unwrap(),
            "-c",
            "copy",
            edited.to_str().unwrap(),
        ]);
        let edited_packets = packets(&edited);
        assert_eq!(edited_packets[0].1, "KD_", "the sample has an edit list");
        let presented = edited_packets
            .iter()
            .find(|(_, f)| f == "K__")
            .map(|(t, _)| *t)
            .unwrap();
        assert!((presented - 0.48).abs() < 0.001, "{presented}");
        let out = dir.path().join("out");
        std::fs::create_dir(&out).unwrap();
        let control = copy(&edited, 0., 3., out.join("cut.mp4"));
        let (destination, start, end) = completed(&control);
        assert!((start - presented).abs() < 0.001, "start {start}");
        assert!((3.0 - 0.04..3.5).contains(&end), "end {end}");
        // The output decodes from its first frame.
        assert!(decoded_first(&destination).abs() < 0.001);
    }

    /// An open GOP copied into MKV with its audio writes leading frames
    /// that present before the keyframe (with timestamps, at 2 s minus a
    /// frame or two): the reported start is the earliest written frame, as
    /// the whole output's packets show, not the keyframe.
    #[test]
    #[ignore = "needs ffmpeg with libx264"]
    fn an_open_gop_start_counts_its_leading_frames() {
        let dir = tempfile::tempdir().unwrap();
        let open = dir.path().join("open.mkv");
        make(&[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x240:rate=24",
            "-f",
            "lavfi",
            "-i",
            "sine=sample_rate=48000",
            "-t",
            "8",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-bf",
            "3",
            "-x264-params",
            "keyint=48:min-keyint=25:scenecut=0:open-gop=1",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
            open.to_str().unwrap(),
        ]);
        let out = dir.path().join("out");
        std::fs::create_dir(&out).unwrap();
        let (control, _) = real::run_job(real::request(
            &open,
            3.,
            out.join("cut.mkv"),
            Output::Copy {
                end: 5.,
                audio: Some(real::audio(1)),
                container: Container::Mkv,
            },
        ));
        let (destination, start, _) = completed(&control);
        let keyframe = control.facts()["copy"]["keyframe"].as_f64().unwrap();
        assert!((keyframe - 2.0).abs() < 0.001, "{keyframe}");
        // Independently: every video packet of the output, mapped through
        // its first one (the keyframe).
        let written = packets(&destination);
        let first = written[0].0;
        let earliest = written.iter().map(|(t, _)| *t).fold(first, f64::min);
        assert!(earliest < first, "the copy has leading frames");
        let expected = keyframe + (earliest - first);
        assert!(
            (start - expected).abs() < 0.001,
            "start {start} vs {expected}"
        );
        assert!(start < keyframe);
    }

    /// A TS cut mid-GOP starts on non-keyframes (its first keyframe is
    /// about 1 s in): a copy from its start begins on that first keyframe.
    #[test]
    #[ignore = "needs ffmpeg with libx264"]
    fn a_ts_cut_mid_gop_starts_on_its_first_keyframe() {
        let dir = tempfile::tempdir().unwrap();
        let closed = source(dir.path(), "closed.mp4", "");
        let mid = dir.path().join("mid.ts");
        make(&[
            "-i",
            closed.to_str().unwrap(),
            "-ss",
            "1",
            "-c",
            "copy",
            "-copyinkf",
            "-f",
            "mpegts",
            mid.to_str().unwrap(),
        ]);
        let origin: f64 =
            real::probe(&mid, &["-show_entries", "format=start_time"])["format"]["start_time"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap();
        let mid_packets = packets(&mid);
        assert!(
            !mid_packets[0].1.starts_with('K'),
            "the sample starts mid-GOP"
        );
        let first = mid_packets
            .iter()
            .find(|(_, f)| f.starts_with('K'))
            .map(|(t, _)| *t - origin)
            .unwrap();
        assert!(first > 0.5, "{first}");
        let mut request = real::request(
            &mid,
            0.,
            dir.path().join("cut.mp4"),
            Output::Copy {
                end: 3.,
                audio: None,
                container: Container::Auto,
            },
        );
        request.source_origin = origin;
        let (control, _) = real::run_job(request);
        let (_, start, _) = completed(&control);
        assert!((start - first).abs() < 0.001, "start {start} vs {first}");
    }

    /// When `auto` renames `clip.mkv` to `clip.mp4` and that name is taken,
    /// the job fails before copying; the existing file is untouched and
    /// nothing else is left.
    #[test]
    #[ignore = "needs ffmpeg with libx264"]
    fn a_taken_container_name_fails_before_copying() {
        let dir = tempfile::tempdir().unwrap();
        let closed = source(dir.path(), "closed.mp4", "");
        let out = dir.path().join("out");
        std::fs::create_dir(&out).unwrap();
        std::fs::write(out.join("clip.mp4"), b"mine").unwrap();
        let control = copy(&closed, 1., 3., out.join("clip.mkv"));
        match control.snapshot().1 {
            Some(Outcome::Failed { code, message }) => {
                assert_eq!(code, "export-failed");
                assert!(message.contains("already exists"), "{message}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(control.progress(), None, "nothing was copied");
        assert_eq!(std::fs::read(out.join("clip.mp4")).unwrap(), b"mine");
        assert_eq!(std::fs::read_dir(&out).unwrap().count(), 1);
    }

    /// Cancelling a copy of the 4K60 clip (about 640 MB from 0 to 10:00)
    /// right after it starts ends it cancelled, with no file left.
    #[test]
    #[ignore = "needs ffmpeg and the testmedia clips"]
    fn cancelling_a_copy_leaves_nothing() {
        let Some(source) = real::clip("bbb_sunflower_2160p_60fps_normal.mp4") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let control = Arc::new(Control::new());
        let worker = control.clone();
        let request = real::request(
            &source,
            0.,
            dir.path().join("long.mp4"),
            Output::Copy {
                end: 600.,
                audio: Some(real::audio(1)),
                container: Container::Auto,
            },
        );
        let thread = std::thread::spawn(move || run(request, worker));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while control.progress().is_none() {
            assert!(
                std::time::Instant::now() < deadline,
                "the copy did not start"
            );
            assert!(!thread.is_finished(), "{:?}", control.snapshot());
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(control.cancel(), CancelResult::Requested);
        thread.join().unwrap();
        assert_eq!(control.snapshot().1, Some(Outcome::Cancelled));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn auto_container_follows_the_codecs() {
        assert!(fits_mp4("h264", Some("aac")));
        assert!(fits_mp4("hevc", None));
        assert!(!fits_mp4("vp9", Some("opus")));
        assert!(!fits_mp4("h264", Some("pcm_s16le")));
        assert!(!fits_mp4("mpeg2video", None));
    }

    #[test]
    fn the_destination_is_named_for_the_container() {
        let mkv = |p: &str| named(Path::new(p), Container::Mkv);
        let mp4 = |p: &str| named(Path::new(p), Container::Mp4);
        assert_eq!(mkv("/c/clip.mp4"), PathBuf::from("/c/clip.mkv"));
        assert_eq!(mkv("/c/clip.MKV"), PathBuf::from("/c/clip.MKV"));
        assert_eq!(mp4("/c/clip.mkv"), PathBuf::from("/c/clip.mp4"));
        assert_eq!(mp4("/c/clip.Mp4"), PathBuf::from("/c/clip.Mp4"));
        assert_eq!(mkv("/c/clip"), PathBuf::from("/c/clip.mkv"));
        assert_eq!(mkv("/c/a.b_04.02"), PathBuf::from("/c/a.b_04.02.mkv"));
    }

    #[test]
    fn the_keyframe_is_the_last_at_or_before_the_start() {
        // A cut first line (bounded tail) and N/A timestamps are skipped.
        let lines = "K__\n77824,K__\n78336,___\nN/A,K__\n99000,K__\n";
        let tb = (1, 15360);
        assert_eq!(last_keyframe(lines, tb, 6.0), Some(77824));
        assert_eq!(last_keyframe(lines, tb, 77824. / 15360.), Some(77824));
        assert_eq!(last_keyframe(lines, tb, 7.0), Some(99000));
        assert_eq!(last_keyframe(lines, tb, 5.0), None);
        assert_eq!(last_keyframe("", tb, 5.0), None);
        // A video that starts late: its first keyframe.
        assert_eq!(first_keyframe("2000,K__\n3000,___\n"), Some(2000));
        assert_eq!(first_keyframe("3000,___\n"), None);
    }

    #[test]
    fn edit_list_discards_are_never_keyframes_to_start_on() {
        // An `-ss … -c copy` MP4: the pre-roll keyframe is flagged D and has
        // a negative time; the first presented keyframe is at 0.48 s.
        let lines = "-19456,KD_\n-17920,_D_\n6144,K__\n31744,K__\n";
        let tb = (1, 12800);
        assert_eq!(last_keyframe(lines, tb, 0.0), None);
        assert_eq!(last_keyframe(lines, tb, 1.0), Some(6144));
        assert_eq!(first_keyframe(lines), Some(6144));
        // TS lines end with a comma.
        assert_eq!(first_keyframe("129600,___,\n216000,K__,\n"), Some(216000));
        assert!(presented_keyframe("K__") && presented_keyframe("K__,"));
        assert!(!presented_keyframe("KD_") && !presented_keyframe("_D_"));
    }

    #[test]
    fn the_start_is_the_earliest_presented_frame_of_the_head() {
        // Open GOP: leading frames after the keyframe present before it.
        let head = "2.000000,K__\n2.120000,___\n1.880000,___\n1.920000,___\n";
        assert_eq!(head_times(head).unwrap(), (2.0, 1.88));
        // Discarded packets do not count.
        assert_eq!(
            head_times("0.000000,K__\n-0.040000,_D_\n").unwrap(),
            (0.0, 0.0)
        );
        assert!(head_times("0.040000,___\n").is_err());
        assert!(head_times("-1.0,KD_\n").is_err());
        assert!(head_times("").is_err());
    }

    #[test]
    fn the_seek_rounds_the_keyframe_down_to_a_microsecond() {
        // 90091/90000 s = 1.00101111… s
        assert_eq!(seek_micros(90091, (1, 90000), 0.), 1_001_011);
        assert_eq!(seconds(seek_micros(90091, (1, 90000), 0.)), "1.001011");
        assert_eq!(seek_micros(26667, (1, 1000), 0.), 26_667_000);
        // The container's start time is normalized away, never below 0.
        assert_eq!(seek_micros(1_400_000, (1, 90000), 10.), 5_555_555);
        assert_eq!(seek_micros(10, (1, 1000), 1.), 0);
        assert_eq!(seconds(0), "0.000000");
    }
}
