//! Finite offline audio mapper (host-internal).
//! This graph is a candidate for leading silence/negative-PTS handling, not
//! evidence of backend acceptance, DSP latency compensation, or audible parity.
#![forbid(unsafe_code)]

use anyhow::{Result, ensure};

use crate::effects::{self, AudioNode, Descriptors, Resource};

/// Decode/process from source time zero, including the entire prefix before
/// the requested interval. Caller must not input-seek away this history.
pub const PREROLL: &str = "source-zero-to-start";

/// Generate a single audio chain from validated typed nodes and finite numbers.
/// Caller owns source/stream selection and execution. No paths, stream maps or
/// arbitrary user filter expressions are accepted here. Effects precede the
/// timestamp shift and resampling so their state sees the original source prefix.
pub fn audio(
    resources: &[Resource<AudioNode>],
    offset_ms: i32,
    start: f64,
    end: f64,
) -> Result<String> {
    ensure!(
        (-10_000..=10_000).contains(&offset_ms),
        "audio offset must be within -10000..=10000 milliseconds"
    );
    ensure!(
        start.is_finite() && end.is_finite() && 0.0 <= start && start < end && end <= 86400.0,
        "audio interval must be finite with 0 <= start < end <= 86400 seconds"
    );
    let effects = effects::render_lavfi(&Descriptors::Audio(resources.to_vec()))?;
    let offset = f64::from(offset_ms) / 1000.0;
    Ok(format!(
        "{effects},asetpts=PTS+({offset})/TB,aresample=48000:async=1:first_pts=0,apad,atrim=start={start}:end={end},asetpts=PTS-STARTPTS"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resource(manifest: &str, node: AudioNode) -> Resource<AudioNode> {
        Resource {
            manifest_id: manifest.into(),
            resource_id: "audio".into(),
            nodes: vec![node],
        }
    }

    #[test]
    fn signed_shift_precedes_padding_and_trim_and_preserves_source_prefix() {
        assert_eq!(PREROLL, "source-zero-to-start");
        for (offset_ms, seconds) in [(-2000, "-2"), (-125, "-0.125"), (0, "0"), (2000, "2")] {
            let graph = audio(&[], offset_ms, 1.25, 4.5).unwrap();
            assert_eq!(
                graph,
                format!(
                    "anull,asetpts=PTS+({seconds})/TB,aresample=48000:async=1:first_pts=0,apad,atrim=start=1.25:end=4.5,asetpts=PTS-STARTPTS"
                )
            );
        }
    }

    #[test]
    fn typed_chain_is_stable_and_precedes_timeline_operations() {
        let earlier = resource("a", AudioNode::Volume { gain_db: -6.0 });
        let later = resource(
            "z",
            AudioNode::Equalizer {
                frequency: 1000.0,
                q: 1.0,
                gain_db: 3.0,
            },
        );
        let graph = audio(&[later.clone(), earlier.clone()], 500, 2.0, 3.0).unwrap();
        assert_eq!(graph, audio(&[earlier, later], 500, 2.0, 3.0).unwrap());
        assert!(graph.starts_with("volume@n0=volume="));
        assert!(graph.contains(
            ",equalizer@n1=frequency=1000:width_type=q:width=1:gain=3,asetpts=PTS+(0.5)/TB,"
        ));
        assert!(!graph.contains("manifest"));
    }

    #[test]
    fn invalid_intervals_offsets_and_typed_nodes_are_rejected() {
        for (start, end) in [
            (f64::NAN, 1.0),
            (0.0, f64::NAN),
            (f64::NEG_INFINITY, 1.0),
            (0.0, f64::INFINITY),
            (-0.1, 1.0),
            (1.0, 1.0),
            (2.0, 1.0),
            (0.0, 86400.1),
        ] {
            assert!(audio(&[], 0, start, end).is_err());
        }
        assert!(audio(&[], -10_010, 0.0, 1.0).is_err());
        assert!(audio(&[], 10_001, 0.0, 1.0).is_err());
        assert!(audio(&[], -10_000, 0.0, 1.0).is_ok());
        assert!(audio(&[], 0, 0.0, 86400.0).is_ok());
        for gain_db in [f64::NAN, 1.0, -60.1] {
            assert!(audio(&[resource("a", AudioNode::Volume { gain_db })], 0, 0.0, 1.0).is_err());
        }
    }
}
