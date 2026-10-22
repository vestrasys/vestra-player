//! Worker-only native plain-SRT extraction (host-internal).
//! Caller freezes native subrip/srt + lavf absolute stream index + zero demuxer
//! start time, and holds its source Reader until this function has reaped the
//! child. No ASS/SSA, mov_text, bitmap, unverified timeline, markup conversion
//! or renderer equivalence is implied. No process work belongs on the UI thread.
#![forbid(unsafe_code)]

use std::{
    io::{self, Read, Write},
    path::Path,
    process::{Child, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};

use crate::{font_asset::FontAsset, subtitle_format};

const MAX_BYTES: usize = 2 * 1024 * 1024;
const STDERR_BYTES: usize = 64 * 1024;

pub struct Prepared {
    pub carrier: tempfile::NamedTempFile,
    pub canonical_digest: String,
    pub derived_digest: String,
    pub cue_count: usize,
}

/// The canonical digest describes the validated extracted SRT bytes, not the
/// original container packets. Font coverage is a cmap check, not a font fence.
pub fn prepare(bytes: Arc<[u8]>, font: &FontAsset) -> Result<Prepared> {
    let validated = subtitle_format::validate(bytes)?;
    let mut text = String::new();
    for cue in &validated.document.subtitles {
        for line in &cue.text {
            ensure!(
                !line.contains(['<', '>', '{', '}', '\\']),
                "unsupported: native subtitle markup or backslash semantics cannot be frozen as literal text"
            );
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(line);
        }
    }
    font.supports(&text)?;
    let webvtt = validated.webvtt();
    let derived_digest = format!("{:x}", Sha256::digest(webvtt.as_bytes()));
    let mut carrier = tempfile::Builder::new()
        .prefix("vestra-native-subtitle-")
        .suffix(".vtt")
        .tempfile()?;
    carrier
        .write_all(webvtt.as_bytes())
        .context("native subtitle carrier write failed")?;
    carrier
        .flush()
        .context("native subtitle carrier flush failed")?;
    Ok(Prepared {
        carrier,
        canonical_digest: validated.digest,
        derived_digest,
        cue_count: validated.document.subtitles.len(),
    })
}

fn stdout_bytes(mut reader: impl Read, overflow: &AtomicBool) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let count = match reader.read(&mut buffer) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if count == 0 {
            break;
        }
        let keep = count.min(MAX_BYTES - bytes.len());
        bytes.extend_from_slice(&buffer[..keep]);
        if keep < count {
            overflow.store(true, Ordering::Release);
        }
        // After overflow, drain/discard until the polling owner kills/reaps.
        // Never turn an overflowed prefix or suffix into accepted SRT.
    }
    Ok(bytes)
}

fn stderr_tail(mut reader: impl Read) -> io::Result<Vec<u8>> {
    let mut tail = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let count = match reader.read(&mut buffer) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if count == 0 {
            break;
        }
        let discard = (tail.len() + count).saturating_sub(STDERR_BYTES);
        tail.drain(..discard);
        tail.extend_from_slice(&buffer[..count]);
    }
    Ok(tail)
}

type Drain = JoinHandle<io::Result<Vec<u8>>>;

struct Process {
    child: Child,
    reaped: bool,
    stdout: Option<Drain>,
    stderr: Option<Drain>,
}

impl Process {
    fn stop(&mut self) -> Result<()> {
        if !self.reaped {
            // kill can race normal process exit; wait remains authoritative.
            let _ = self.child.kill();
            self.child
                .wait()
                .context("native subtitle child reap failed")?;
            self.reaped = true;
        }
        Ok(())
    }

    fn join(&mut self) -> Result<(Vec<u8>, Vec<u8>)> {
        // Join BOTH before propagating either error.
        let out = self.stdout.take().map(|worker| worker.join());
        let err = self.stderr.take().map(|worker| worker.join());
        let collect = |result: Option<thread::Result<io::Result<Vec<u8>>>>| -> Result<Vec<u8>> {
            result
                .context("native subtitle drain missing")?
                .map_err(|_| anyhow::anyhow!("native subtitle drain panicked"))?
                .context("native subtitle pipe read failed")
        };
        Ok((collect(out)?, collect(err)?))
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.stop();
        if let Some(worker) = self.stdout.take() {
            let _ = worker.join();
        }
        if let Some(worker) = self.stderr.take() {
            let _ = worker.join();
        }
    }
}

pub fn extract(
    ffmpeg: &Path,
    stdin: Stdio,
    stream: u32,
    font: &FontAsset,
    cancelled: impl Fn() -> bool,
) -> Result<Option<Prepared>> {
    ensure!(
        stream <= 1023,
        "native subtitle stream index exceeds limits"
    );
    if cancelled() {
        return Ok(None);
    }
    let child = crate::platform::tool_command(ffmpeg)
        .args([
            "-nostdin",
            "-v",
            "error",
            "-threads",
            "1",
            "-copyts",
            "-start_at_zero",
            "-i",
            crate::platform::HELD_FILE_INPUT,
            "-map",
        ])
        .arg(format!("0:{stream}"))
        .args(["-c:s", "copy", "-f", "srt", "-"])
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("native subtitle FFmpeg startup failed")?;
    let mut process = Process {
        child,
        reaped: false,
        stdout: None,
        stderr: None,
    };
    let stdout = process
        .child
        .stdout
        .take()
        .context("native subtitle stdout unavailable")?;
    let stderr = process
        .child
        .stderr
        .take()
        .context("native subtitle stderr unavailable")?;
    let overflow = Arc::new(AtomicBool::new(false));
    let flag = overflow.clone();
    process.stdout = Some(
        thread::Builder::new()
            .name("native-subtitle-stdout".into())
            .spawn(move || stdout_bytes(stdout, &flag))
            .context("native subtitle stdout worker startup failed")?,
    );
    process.stderr = Some(
        thread::Builder::new()
            .name("native-subtitle-stderr".into())
            .spawn(move || stderr_tail(stderr))
            .context("native subtitle stderr worker startup failed")?,
    );
    let status = loop {
        if cancelled() {
            process.stop()?;
            let _ = process.join();
            return Ok(None);
        }
        if overflow.load(Ordering::Acquire) {
            bail!("native subtitle extraction exceeds 2 MiB");
        }
        if let Some(status) = process
            .child
            .try_wait()
            .context("native subtitle child polling failed")?
        {
            process.reaped = true;
            break status;
        }
        thread::sleep(Duration::from_millis(20));
    };
    let (bytes, stderr) = process.join()?;
    if cancelled() {
        return Ok(None);
    }
    ensure!(
        !overflow.load(Ordering::Acquire),
        "native subtitle extraction exceeds 2 MiB"
    );
    ensure!(
        status.success(),
        "native subtitle FFmpeg failed: {}",
        String::from_utf8_lossy(&stderr)
    );
    let prepared = prepare(Arc::from(bytes), font)?;
    if cancelled() {
        return Ok(None);
    }
    Ok(Some(prepared))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn srt(text: &str) -> Arc<[u8]> {
        Arc::from(format!("1\n00:00:00,000 --> 00:00:02,000\n{text}\n\n").into_bytes())
    }

    #[test]
    fn chinese_plain_cues_keep_times_content_and_distinct_digests() {
        let font = FontAsset::bundled().unwrap();
        let bytes = srt("中文字幕\nHello & goodbye.");
        let prepared = prepare(bytes.clone(), &font).unwrap();
        assert_eq!(prepared.cue_count, 1);
        assert_eq!(
            prepared.canonical_digest,
            format!("{:x}", Sha256::digest(&bytes))
        );
        let carrier = std::fs::read(prepared.carrier.path()).unwrap();
        assert_eq!(
            prepared.derived_digest,
            format!("{:x}", Sha256::digest(&carrier))
        );
        let text = std::str::from_utf8(&carrier).unwrap();
        assert!(text.starts_with("WEBVTT\n\n"));
        assert!(text.contains("00:00:00.000 --> 00:00:02.000\n中文字幕\nHello &amp; goodbye."));
    }

    #[test]
    fn native_markup_missing_glyphs_and_bad_srt_are_explicitly_rejected() {
        let font = FontAsset::bundled().unwrap();
        for text in ["<b>B</b>", "a > b", "{\\b1}X", "\\N", "x}", "\u{10FFFF}"] {
            assert!(prepare(srt(text), &font).is_err());
        }
        assert!(prepare(Arc::from(&b"not SRT"[..]), &font).is_err());
        assert!(prepare(Arc::from(vec![b'x'; MAX_BYTES + 1]), &font).is_err());
    }

    #[test]
    fn stdout_boundary_keeps_bounded_prefix_and_marks_any_overflow() {
        let overflow = AtomicBool::new(false);
        let exact = vec![b'x'; MAX_BYTES];
        assert_eq!(
            stdout_bytes(io::Cursor::new(&exact), &overflow).unwrap(),
            exact
        );
        assert!(!overflow.load(Ordering::Acquire));
        let mut oversized = exact.clone();
        oversized.extend_from_slice(b"tail");
        assert_eq!(
            stdout_bytes(io::Cursor::new(oversized), &overflow).unwrap(),
            exact
        );
        assert!(overflow.load(Ordering::Acquire));
        let mut diagnostic = vec![b'a'; STDERR_BYTES];
        diagnostic.extend_from_slice(b"end");
        let tail = stderr_tail(io::Cursor::new(diagnostic)).unwrap();
        assert_eq!(tail.len(), STDERR_BYTES);
        assert!(tail.ends_with(b"end"));
    }
}
