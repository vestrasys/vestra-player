//! Bounded SRT validation around the mature public subtp parser.
//! Text is preserved literally here: this is NOT an ASS/HTML escaping renderer.
#![forbid(unsafe_code)]

use anyhow::{Result, bail};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use subtp::srt::{SrtTimestamp, SubRip};

pub struct Validated {
    pub source: Arc<[u8]>,
    pub digest: String,
    pub document: SubRip,
}

fn milliseconds(time: &SrtTimestamp) -> Result<u64> {
    if time.minutes >= 60 || time.seconds >= 60 || time.milliseconds >= 1000 {
        bail!("subtitle timestamp has out-of-range fields");
    }
    let value = ((u64::from(time.hours) * 60 + u64::from(time.minutes)) * 60
        + u64::from(time.seconds))
        * 1000
        + u64::from(time.milliseconds);
    if value > 86_400_000 {
        bail!("subtitle timestamp exceeds 24 hours");
    }
    Ok(value)
}

pub fn validate(source: Arc<[u8]>) -> Result<Validated> {
    if source.is_empty() || source.len() > 2 * 1024 * 1024 {
        bail!("SRT must contain 1..2097152 bytes");
    }
    let text = std::str::from_utf8(&source)?;
    let text = text
        .strip_prefix('\u{feff}')
        .unwrap_or(text)
        .replace("\r\n", "\n");
    if text.contains(['\r', '\0']) {
        bail!("SRT requires LF/CRLF and no NUL bytes");
    }
    // A final line terminator is optional in user files.
    let text = if text.ends_with('\n') {
        text
    } else {
        text + "\n"
    };
    let document = SubRip::parse(&text).map_err(|error| anyhow::anyhow!("SRT parse: {error:?}"))?;
    if document.subtitles.is_empty() || document.subtitles.len() > 10_000 {
        bail!("SRT must contain 1..10000 cues");
    }
    let (mut sequence, mut last_start) = (0, 0);
    for cue in &document.subtitles {
        let start = milliseconds(&cue.start)?;
        let end = milliseconds(&cue.end)?;
        if cue.sequence <= sequence || start >= end || start < last_start {
            bail!(
                "SRT cue {} requires increasing positive sequence, ordered starts and start < end",
                cue.sequence
            );
        }
        let text = cue.text.join("\n");
        if text.trim().is_empty() || text.chars().count() > 4096 || cue.line_position.is_some() {
            bail!(
                "SRT cue {} has empty/oversized text or unsupported position metadata",
                cue.sequence
            );
        }
        sequence = cue.sequence;
        last_start = start;
    }
    let digest = format!("{:x}", Sha256::digest(&source));
    Ok(Validated {
        source,
        digest,
        document,
    })
}

/// Standard WebVTT adapter. FFmpeg's mature WebVTT decoder protects ASS braces
/// and backslashes; only WebVTT's three text entities are encoded here.
impl Validated {
    pub fn webvtt(&self) -> String {
        let timestamp = |t: &SrtTimestamp| {
            format!(
                "{:02}:{:02}:{:02}.{:03}",
                t.hours, t.minutes, t.seconds, t.milliseconds
            )
        };
        let mut output = String::from("WEBVTT\n\n");
        for cue in &self.document.subtitles {
            output.push_str(&format!(
                "{}\n{} --> {}\n",
                cue.sequence,
                timestamp(&cue.start),
                timestamp(&cue.end)
            ));
            for line in &cue.text {
                output.push_str(
                    &line
                        .replace('&', "&amp;")
                        .replace('<', "&lt;")
                        .replace('>', "&gt;"),
                );
                output.push('\n');
            }
            output.push('\n');
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(text: &str) -> Result<Validated> {
        validate(Arc::from(text.as_bytes()))
    }
    #[test]
    fn bom_crlf_and_overlap_preserve_source_and_literal_text() {
        let raw = "\u{feff}1\r\n00:00:00,000 --> 00:00:02,000\r\n<b>B</b> {\\b1}X \\N\r\n\r\n3\r\n00:00:01,000 --> 00:00:03,000\r\nOverlap";
        let result = parse(raw).unwrap();
        assert_eq!(&*result.source, raw.as_bytes());
        assert_eq!(result.document.subtitles.len(), 2);
        assert_eq!(result.document.subtitles[0].text[0], "<b>B</b> {\\b1}X \\N");
        assert_eq!(result.digest.len(), 64);
    }
    #[test]
    fn invalid_ranges_order_and_encoding_are_rejected() {
        for bad in [
            "0\n00:00:00,000 --> 00:00:01,000\nX\n",
            "1\n00:60:00,000 --> 00:60:01,000\nX\n",
            "1\n00:00:02,000 --> 00:00:01,000\nX\n",
            "1\n24:00:00,000 --> 24:00:01,000\nX\n",
            "1\n00:00:02,000 --> 00:00:03,000\nX\n\n2\n00:00:01,000 --> 00:00:02,000\nY\n",
        ] {
            assert!(parse(bad).is_err(), "accepted {bad:?}");
        }
        assert!(validate(Arc::from(&b"\xff"[..])).is_err());
        assert!(
            parse(&format!(
                "1\n00:00:00,000 --> 00:00:01,000\n{}\n",
                "x".repeat(4097)
            ))
            .is_err()
        );
    }
}
