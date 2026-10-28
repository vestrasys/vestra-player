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

/// A bounded ASS/SSA script, kept byte for byte (libass renders it).
pub struct ValidatedAss {
    pub source: Arc<[u8]>,
    pub digest: String,
    pub dialogues: u32,
}

/// UTF-8, at most 2 MiB, `[Script Info]` and `[Events]` sections, and
/// 1..=20000 `Dialogue:` lines. SSA scripts pass the same check.
pub fn validate_ass(source: Arc<[u8]>) -> Result<ValidatedAss> {
    if source.is_empty() || source.len() > 2 * 1024 * 1024 {
        bail!("ASS must contain 1..2097152 bytes");
    }
    let text = std::str::from_utf8(&source)?;
    if text.contains('\0') {
        bail!("ASS must not contain NUL bytes");
    }
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let (mut info, mut events, mut dialogues) = (false, false, 0_u32);
    for line in text.lines() {
        let line = line.trim();
        if line.eq_ignore_ascii_case("[Script Info]") {
            info = true;
        } else if line.eq_ignore_ascii_case("[Events]") {
            events = true;
        } else if events
            && line
                .get(..9)
                .is_some_and(|head| head.eq_ignore_ascii_case("Dialogue:"))
        {
            dialogues += 1;
        }
    }
    if !info || !events {
        bail!("ASS requires [Script Info] and [Events] sections");
    }
    if !(1..=20_000).contains(&dialogues) {
        bail!("ASS must contain 1..20000 Dialogue lines");
    }
    let digest = format!("{:x}", Sha256::digest(&source));
    Ok(ValidatedAss {
        source,
        digest,
        dialogues,
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
    fn ass_scripts_are_bounded_by_sections_and_dialogue_count() {
        let script = "\u{feff}[Script Info]\r\nScriptType: v4.00+\r\n\r\n[V4+ Styles]\r\nFormat: Name\r\n\r\n[Events]\r\nFormat: Layer, Start, End, Style, Text\r\nDialogue: 0,0:00:00.00,0:00:01.00,Default,{\\pos(10,10)}Hi\r\ndialogue: 0,0:00:01.00,0:00:02.00,Default,Two\r\n";
        let checked = validate_ass(Arc::from(script.as_bytes())).unwrap();
        assert_eq!(checked.dialogues, 2);
        assert_eq!(&*checked.source, script.as_bytes());
        assert_eq!(checked.digest.len(), 64);
        let ssa = "[Script Info]\nScriptType: v4.00\n[Events]\nDialogue: Marked=0,0:00:00.00,0:00:01.00,Default,,0,0,0,,X\n";
        assert!(validate_ass(Arc::from(ssa.as_bytes())).is_ok());
        for bad in [
            "[Events]\nDialogue: 0,0:00:00.00,0:00:01.00,Default,X\n",
            "[Script Info]\nDialogue: 0,0:00:00.00,0:00:01.00,Default,X\n",
            "[Script Info]\n[Events]\nFormat: Text\n",
            "1\n00:00:00,000 --> 00:00:01,000\nX\n",
        ] {
            assert!(validate_ass(Arc::from(bad.as_bytes())).is_err(), "{bad:?}");
        }
        assert!(validate_ass(Arc::from(&b"[Script Info]\n[Events]\n\xff"[..])).is_err());
        let many = format!(
            "[Script Info]\n[Events]\n{}",
            "Dialogue: 0,0:00:00.00,0:00:01.00,Default,X\n".repeat(20_001)
        );
        assert!(validate_ass(Arc::from(many.as_bytes())).is_err());
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
