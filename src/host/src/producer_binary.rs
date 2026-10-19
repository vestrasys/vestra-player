//! Experimental finite producer packet codec; final public API may differ.
//! No IO, buffering, epoch arbitration, scheduling or backend acceptance here.
//! sample_time is a 48kHz consumed-frame position: audio chunk's first frame,
//! or video's intended presentation position. Sequence zero is valid.
#![forbid(unsafe_code)]

const MAGIC: &[u8; 4] = b"VAV1";
const MAX_SAFE_INTEGER: u64 = (1_u64 << 53) - 1;
const MAX_VIDEO_BYTES: u32 = 4 * 1024 * 1024;
const MAX_AUDIO_BYTES: usize = 4800 * 2 * 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spec {
    pub width: u32,
    pub height: u32,
}

impl Spec {
    /// RGBA8 only. Width/height bounds and total byte bound both apply.
    pub fn validate(self) -> Result<(), Error> {
        self.video_bytes().map(|_| ())
    }

    fn video_bytes(self) -> Result<u32, Error> {
        if !(1..=1920).contains(&self.width) || !(1..=1080).contains(&self.height) {
            return Err(error(
                "video dimensions must be within 1..=1920 by 1..=1080",
            ));
        }
        let bytes = self
            .width
            .checked_mul(self.height)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| error("RGBA byte count overflow"))?;
        if bytes > MAX_VIDEO_BYTES {
            return Err(error("RGBA frame exceeds 4 MiB"));
        }
        Ok(bytes)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Audio,
    Video,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub kind: Kind,
    pub epoch: u64,
    pub sample_time: u64,
    pub sequence: u32,
    pub payload_bytes: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub message: String,
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}
impl std::error::Error for Error {}

fn error(message: &str) -> Error {
    Error {
        message: message.into(),
    }
}

fn audio_length(bytes: usize) -> Result<(), Error> {
    if bytes == 0 || bytes > MAX_AUDIO_BYTES || !bytes.is_multiple_of(8) {
        return Err(error(
            "audio payload must contain 1..=4800 stereo f32 frames",
        ));
    }
    Ok(())
}

fn validate(header: Header, spec: Spec) -> Result<(), Error> {
    let video_bytes = spec.video_bytes()?;
    if header.epoch == 0 || header.epoch > MAX_SAFE_INTEGER || header.sample_time > MAX_SAFE_INTEGER
    {
        return Err(error(
            "epoch must be positive and epoch/sample_time must be safe integers",
        ));
    }
    match header.kind {
        Kind::Audio => audio_length(header.payload_bytes as usize)?,
        Kind::Video if header.payload_bytes != video_bytes => {
            return Err(error("video payload must exactly match the RGBA spec"));
        }
        Kind::Video => {}
    }
    Ok(())
}

pub fn decode_header(bytes: &[u8; 32], spec: Spec) -> Result<Header, Error> {
    if &bytes[..4] != MAGIC {
        return Err(error("invalid producer packet magic"));
    }
    let kind = match bytes[4] {
        1 => Kind::Audio,
        2 => Kind::Video,
        _ => return Err(error("unknown producer packet kind")),
    };
    if bytes[5..8] != [0; 3] {
        return Err(error("producer packet reserved bytes must be zero"));
    }
    // Fixed array indexes, independent of payload or attacker-provided lengths.
    let header = Header {
        kind,
        epoch: u64::from_le_bytes(
            bytes[8..16]
                .try_into()
                .map_err(|_| error("invalid epoch field"))?,
        ),
        sample_time: u64::from_le_bytes(
            bytes[16..24]
                .try_into()
                .map_err(|_| error("invalid sample_time field"))?,
        ),
        payload_bytes: u32::from_le_bytes(
            bytes[24..28]
                .try_into()
                .map_err(|_| error("invalid length field"))?,
        ),
        sequence: u32::from_le_bytes(
            bytes[28..32]
                .try_into()
                .map_err(|_| error("invalid sequence field"))?,
        ),
    };
    validate(header, spec)?;
    Ok(header)
}

pub fn encode_header(header: Header, spec: Spec) -> Result<[u8; 32], Error> {
    validate(header, spec)?;
    let mut bytes = [0; 32];
    bytes[..4].copy_from_slice(MAGIC);
    bytes[4] = match header.kind {
        Kind::Audio => 1,
        Kind::Video => 2,
    };
    bytes[8..16].copy_from_slice(&header.epoch.to_le_bytes());
    bytes[16..24].copy_from_slice(&header.sample_time.to_le_bytes());
    bytes[24..28].copy_from_slice(&header.payload_bytes.to_le_bytes());
    bytes[28..32].copy_from_slice(&header.sequence.to_le_bytes());
    Ok(bytes)
}

/// Packed interleaved stereo f32le samples; reject the whole chunk on any bad
/// sample, without clipping, partial admission, or decoding an oversized input.
pub fn decode_audio(bytes: &[u8]) -> Result<Vec<f32>, Error> {
    audio_length(bytes.len())?;
    let mut samples = Vec::with_capacity(bytes.len() / 4);
    for sample in bytes.as_chunks::<4>().0 {
        let value = f32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]]);
        if !value.is_finite() || value.abs() > 1.0 {
            return Err(error("audio samples must be finite within -1..=1"));
        }
        samples.push(value);
    }
    Ok(samples)
}

#[cfg(test)]
mod tests {
    use super::*;
    const SPEC: Spec = Spec {
        width: 256,
        height: 240,
    };
    fn header(kind: Kind) -> Header {
        Header {
            kind,
            epoch: 7,
            sample_time: 48_000,
            sequence: 0,
            payload_bytes: match kind {
                Kind::Audio => 8,
                Kind::Video => 256 * 240 * 4,
            },
        }
    }

    #[test]
    fn wire_layout_and_both_kinds_roundtrip_without_losing_sequence_zero() {
        for kind in [Kind::Audio, Kind::Video] {
            let h = header(kind);
            let bytes = encode_header(h, SPEC).unwrap();
            assert_eq!(&bytes[..4], b"VAV1");
            assert_eq!(bytes[4], if kind == Kind::Audio { 1 } else { 2 });
            assert_eq!(&bytes[5..8], &[0, 0, 0]);
            assert_eq!(&bytes[8..16], &7_u64.to_le_bytes());
            assert_eq!(&bytes[16..24], &48_000_u64.to_le_bytes());
            assert_eq!(&bytes[24..28], &h.payload_bytes.to_le_bytes());
            assert_eq!(decode_header(&bytes, SPEC).unwrap(), h);
        }
    }

    #[test]
    fn bad_magic_kind_reserved_and_unsafe_times_are_rejected() {
        let valid = encode_header(header(Kind::Audio), SPEC).unwrap();
        for (index, value) in [(0, b'X'), (4, 0), (4, 3), (5, 1), (6, 1), (7, 1)] {
            let mut bad = valid;
            bad[index] = value;
            assert!(decode_header(&bad, SPEC).is_err());
        }
        for (epoch, sample_time) in [(0, 0), (MAX_SAFE_INTEGER + 1, 0), (1, MAX_SAFE_INTEGER + 1)] {
            let h = Header {
                epoch,
                sample_time,
                ..header(Kind::Audio)
            };
            assert!(encode_header(h, SPEC).is_err());
            let mut bad = valid;
            bad[8..16].copy_from_slice(&epoch.to_le_bytes());
            bad[16..24].copy_from_slice(&sample_time.to_le_bytes());
            assert!(decode_header(&bad, SPEC).is_err());
        }
        let h = Header {
            epoch: MAX_SAFE_INTEGER,
            sample_time: MAX_SAFE_INTEGER,
            sequence: u32::MAX,
            ..header(Kind::Audio)
        };
        assert_eq!(
            decode_header(&encode_header(h, SPEC).unwrap(), SPEC).unwrap(),
            h
        );
    }

    #[test]
    fn dimensions_and_exact_payload_limits_are_independent() {
        for spec in [
            Spec {
                width: 0,
                height: 1,
            },
            Spec {
                width: 1,
                height: 0,
            },
            Spec {
                width: 1921,
                height: 1,
            },
            Spec {
                width: 1,
                height: 1081,
            },
            Spec {
                width: 1920,
                height: 1080,
            },
        ] {
            assert!(spec.validate().is_err());
        }
        assert!(
            Spec {
                width: 1024,
                height: 1024
            }
            .validate()
            .is_ok()
        );
        assert!(
            Spec {
                width: 1920,
                height: 1
            }
            .validate()
            .is_ok()
        );
        for size in [0, 4, 256 * 240 * 4 - 1, 256 * 240 * 4 + 1] {
            assert!(
                encode_header(
                    Header {
                        payload_bytes: size,
                        ..header(Kind::Video)
                    },
                    SPEC
                )
                .is_err()
            );
        }
        for size in [0, 4, MAX_AUDIO_BYTES as u32 + 8] {
            assert!(
                encode_header(
                    Header {
                        payload_bytes: size,
                        ..header(Kind::Audio)
                    },
                    SPEC
                )
                .is_err()
            );
        }
        assert!(
            encode_header(
                Header {
                    payload_bytes: MAX_AUDIO_BYTES as u32,
                    ..header(Kind::Audio)
                },
                SPEC
            )
            .is_ok()
        );
    }

    #[test]
    fn audio_is_little_endian_stereo_and_never_clips_invalid_samples() {
        let bytes: Vec<_> = [-1.0_f32, 1.0, 0.25, -0.5]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect();
        assert_eq!(decode_audio(&bytes).unwrap(), vec![-1.0, 1.0, 0.25, -0.5]);
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1.0001, -1.0001] {
            let bytes: Vec<_> = [0.0_f32, value]
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .collect();
            assert!(decode_audio(&bytes).is_err());
        }
        assert!(decode_audio(&[]).is_err());
        assert!(decode_audio(&[0; 4]).is_err());
        assert!(decode_audio(&vec![0; MAX_AUDIO_BYTES]).is_ok());
        assert!(decode_audio(&vec![0; MAX_AUDIO_BYTES + 8]).is_err());
    }
}
