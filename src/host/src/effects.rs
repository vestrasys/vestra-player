//! Experimental typed filter adapter. Final public API may differ substantially.
//! Rendering or planning commands is NOT backend acceptance, frame visibility,
//! atomic application, rollback, or evidence of runtime algorithm correctness.
//! The caller owns revisions, source CAS, applied confirmation and restoration.
#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum VideoNode {
    /// FFmpeg eq units: additive brightness, contrast/saturation multipliers,
    /// and gamma exponent control. These are not UI percentage values.
    Eq {
        brightness: f64,
        contrast: f64,
        saturation: f64,
        gamma: f64,
    },
    /// Isotropic sigma in pixels; the same sigma is written to sigmaV.
    Gblur { sigma: f64, steps: u8 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AudioNode {
    Volume {
        gain_db: f64,
    },
    /// Frequency in Hz, dimensionless Q, gain in dB.
    Equalizer {
        frequency: f64,
        q: f64,
        gain_db: f64,
    },
    /// Frequency in Hz, dimensionless shelf slope, gain in dB.
    Treble {
        frequency: f64,
        slope: f64,
        gain_db: f64,
    },
    /// max_gain is linear amplitude, peak is a full-scale amplitude fraction.
    /// Frame length is fixed at 100ms and Gaussian size at 5 frames.
    Dynaudnorm {
        enabled: bool,
        max_gain: f64,
        peak: f64,
    },
}

/// Stable public ordering key. Owner instance generations are lifecycle guards
/// held by the caller, not sorting keys. No plugin identity has special behavior.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Resource<N> {
    pub manifest_id: String,
    pub resource_id: String,
    pub nodes: Vec<N>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "stage", content = "resources", rename_all = "lowercase")]
pub enum Descriptors {
    Video(Vec<Resource<VideoNode>>),
    Audio(Vec<Resource<AudioNode>>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationError(pub String);

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ValidationError {}

type Result<T> = std::result::Result<T, ValidationError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeCommand {
    pub target: String,
    pub option: &'static str,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdatePlan {
    Commands(Vec<RuntimeCommand>),
    /// Includes owner/resource topology changes and options without local mpv
    /// runtime evidence. The caller must separately validate reconfiguration.
    NeedsReconfigure,
}

#[derive(Clone)]
struct Parameter {
    name: &'static str,
    value: String,
    runtime: bool,
}

fn parameter(name: &'static str, value: impl ToString, runtime: bool) -> Parameter {
    Parameter {
        name,
        value: value.to_string(),
        runtime,
    }
}

fn range(name: &str, value: f64, min: f64, max: f64) -> Result<()> {
    if !value.is_finite() || !(min..=max).contains(&value) {
        return Err(ValidationError(format!(
            "{name} must be finite in {min}..={max}"
        )));
    }
    Ok(())
}

/// A finite internal adapter; public callers cannot add filter/option strings.
trait Node {
    fn validate(&self) -> Result<()>;
    fn name(&self) -> &'static str;
    fn parameters(&self) -> Vec<Parameter>;
}

impl Node for VideoNode {
    fn validate(&self) -> Result<()> {
        match self {
            Self::Eq {
                brightness,
                contrast,
                saturation,
                gamma,
            } => {
                range("brightness", *brightness, -1.0, 1.0)?;
                range("contrast", *contrast, 0.0, 2.0)?;
                range("saturation", *saturation, 0.0, 2.0)?;
                range("gamma", *gamma, 0.5, 2.0)
            }
            Self::Gblur { sigma, steps } => {
                range("sigma", *sigma, 0.0, 10.0)?;
                if *steps != 3 {
                    return Err(ValidationError(
                        "steps must be 3 in this experimental adapter".into(),
                    ));
                }
                Ok(())
            }
        }
    }
    fn name(&self) -> &'static str {
        match self {
            Self::Eq { .. } => "eq",
            Self::Gblur { .. } => "gblur",
        }
    }
    fn parameters(&self) -> Vec<Parameter> {
        match self {
            Self::Eq {
                brightness,
                contrast,
                saturation,
                gamma,
            } => vec![
                parameter("brightness", brightness, true),
                parameter("contrast", contrast, true),
                parameter("saturation", saturation, true),
                parameter("gamma", gamma, true),
            ],
            Self::Gblur { sigma, steps } => vec![
                parameter("sigma", sigma, true),
                parameter("sigmaV", sigma, true),
                parameter("steps", steps, false),
            ],
        }
    }
}

impl Node for AudioNode {
    fn validate(&self) -> Result<()> {
        match self {
            Self::Volume { gain_db } => range("gain_db", *gain_db, -60.0, 0.0),
            Self::Equalizer {
                frequency,
                q,
                gain_db,
            } => {
                range("frequency", *frequency, 20.0, 20_000.0)?;
                range("q", *q, 1.0, 1.0)?;
                range("gain_db", *gain_db, -6.0, 6.0)
            }
            Self::Treble {
                frequency,
                slope,
                gain_db,
            } => {
                range("frequency", *frequency, 3000.0, 3000.0)?;
                range("slope", *slope, 0.5, 0.5)?;
                range("gain_db", *gain_db, 0.0, 6.0)
            }
            Self::Dynaudnorm { max_gain, peak, .. } => {
                range("max_gain", *max_gain, 1.0, 4.0)?;
                range("peak", *peak, 0.5, 0.95)
            }
        }
    }
    fn name(&self) -> &'static str {
        match self {
            Self::Volume { .. } => "volume",
            Self::Equalizer { .. } => "equalizer",
            Self::Treble { .. } => "treble",
            Self::Dynaudnorm { .. } => "dynaudnorm",
        }
    }
    fn parameters(&self) -> Vec<Parameter> {
        match self {
            Self::Volume { gain_db } => vec![parameter("volume", db_amplitude(*gain_db), true)],
            Self::Equalizer {
                frequency,
                q,
                gain_db,
            } => vec![
                parameter("frequency", frequency, false),
                parameter("width_type", "q", false),
                parameter("width", q, false),
                parameter("gain", gain_db, true),
            ],
            Self::Treble {
                frequency,
                slope,
                gain_db,
            } => vec![
                parameter("frequency", frequency, false),
                parameter("width_type", "s", false),
                parameter("width", slope, false),
                parameter("gain", gain_db, true),
            ],
            Self::Dynaudnorm {
                enabled,
                max_gain,
                peak,
            } => vec![
                parameter("framelen", 100, false),
                parameter("gausssize", 5, false),
                parameter("maxgain", max_gain, true),
                parameter("peak", peak, true),
                parameter("enable", u8::from(*enabled), true),
            ],
        }
    }
}

// Applied identically to offline graph generation and live runtime updates.
fn db_amplitude(gain_db: f64) -> f64 {
    10.0_f64.powf(gain_db / 20.0)
}

fn ordered<N: Node>(resources: &[Resource<N>]) -> Result<Vec<&Resource<N>>> {
    if resources.len() > 32 || resources.iter().map(|r| r.nodes.len()).sum::<usize>() > 64 {
        return Err(ValidationError(
            "chain limit: 32 resources and 64 nodes".into(),
        ));
    }
    let mut ordered: Vec<_> = resources.iter().collect();
    ordered.sort_by(|a, b| (&a.manifest_id, &a.resource_id).cmp(&(&b.manifest_id, &b.resource_id)));
    let mut previous = None;
    for resource in &ordered {
        if resource.manifest_id.is_empty()
            || resource.resource_id.is_empty()
            || resource.manifest_id.len() > 128
            || resource.resource_id.len() > 128
        {
            return Err(ValidationError(
                "resource keys require 1..=128 bytes".into(),
            ));
        }
        let key = (&resource.manifest_id, &resource.resource_id);
        if previous == Some(key) {
            return Err(ValidationError("duplicate resource key".into()));
        }
        previous = Some(key);
        if resource.nodes.is_empty() {
            return Err(ValidationError(
                "resource must contain a node; remove the resource to clear it".into(),
            ));
        }
        for node in &resource.nodes {
            node.validate()?;
        }
    }
    Ok(ordered)
}

pub fn validate(descriptors: &Descriptors) -> Result<()> {
    match descriptors {
        Descriptors::Video(resources) => {
            ordered(resources)?;
        }
        Descriptors::Audio(resources) => {
            ordered(resources)?;
        }
    }
    Ok(())
}

fn render<N: Node>(resources: &[Resource<N>], prefix: &str, empty: &str) -> Result<String> {
    let resources = ordered(resources)?;
    let filters: Vec<_> = resources
        .into_iter()
        .flat_map(|r| &r.nodes)
        .enumerate()
        .map(|(index, node)| {
            let parameters = node
                .parameters()
                .into_iter()
                .map(|p| format!("{}={}", p.name, p.value))
                .collect::<Vec<_>>()
                .join(":");
            format!("{}@{prefix}{index}={parameters}", node.name())
        })
        .collect();
    Ok(if filters.is_empty() {
        empty.into()
    } else {
        filters.join(",")
    })
}

/// Returns a numeric-only single-chain FFmpeg graph, usable within mpv's lavfi
/// wrapper and offline FFmpeg. Resource IDs never enter the graph expression.
/// Empty pipelines render null/anull. Sample-rate/Nyquist support must also be
/// checked against the actual selected source before backend application.
pub fn render_lavfi(descriptors: &Descriptors) -> Result<String> {
    match descriptors {
        Descriptors::Video(resources) => render(resources, "n", "null"),
        Descriptors::Audio(resources) => render(resources, "n", "anull"),
    }
}

fn updates<N: Node>(old: &[Resource<N>], new: &[Resource<N>], prefix: &str) -> Result<UpdatePlan> {
    let old = ordered(old)?;
    let new = ordered(new)?;
    if old.len() != new.len() {
        return Ok(UpdatePlan::NeedsReconfigure);
    }
    let mut commands = Vec::new();
    let mut index = 0;
    for (old, new) in old.into_iter().zip(new) {
        if old.manifest_id != new.manifest_id
            || old.resource_id != new.resource_id
            || old.nodes.len() != new.nodes.len()
        {
            return Ok(UpdatePlan::NeedsReconfigure);
        }
        for (old, new) in old.nodes.iter().zip(&new.nodes) {
            if old.name() != new.name() {
                return Ok(UpdatePlan::NeedsReconfigure);
            }
            let filter_name = new.name();
            for (old, new) in old.parameters().into_iter().zip(new.parameters()) {
                if old.value != new.value {
                    if !new.runtime {
                        return Ok(UpdatePlan::NeedsReconfigure);
                    }
                    commands.push(RuntimeCommand {
                        target: format!("{}@{prefix}{index}", filter_name),
                        option: new.name,
                        value: new.value,
                    });
                }
            }
            index += 1;
        }
    }
    Ok(UpdatePlan::Commands(commands))
}

/// Only the locally exercised command families are planned: eq controls,
/// gblur sigma/sigmaV, volume, EQ/treble gain, and norm maxgain/peak/enable.
/// Other option changes conservatively require reconfiguration even when a
/// particular FFmpeg build advertises additional runtime commands.
pub fn runtime_commands(old: &Descriptors, new: &Descriptors) -> Result<UpdatePlan> {
    match (old, new) {
        (Descriptors::Video(old), Descriptors::Video(new)) => updates(old, new, "n"),
        (Descriptors::Audio(old), Descriptors::Audio(new)) => updates(old, new, "n"),
        _ => {
            validate(old)?;
            validate(new)?;
            Ok(UpdatePlan::NeedsReconfigure)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resource<N>(manifest: &str, id: &str, nodes: Vec<N>) -> Resource<N> {
        Resource {
            manifest_id: manifest.into(),
            resource_id: id.into(),
            nodes,
        }
    }
    fn tone(brightness: f64) -> VideoNode {
        VideoNode::Eq {
            brightness,
            contrast: 1.0,
            saturation: 1.0,
            gamma: 1.0,
        }
    }
    fn blur(sigma: f64) -> VideoNode {
        VideoNode::Gblur { sigma, steps: 3 }
    }

    #[test]
    fn invalid_numbers_nodes_and_duplicate_keys_are_rejected() {
        for node in [
            tone(f64::NAN),
            tone(f64::INFINITY),
            blur(-1.0),
            VideoNode::Gblur {
                sigma: 1.0,
                steps: 2,
            },
        ] {
            let graph = Descriptors::Video(vec![resource("owner", "r", vec![node])]);
            assert!(render_lavfi(&graph).is_err());
            assert!(runtime_commands(&graph, &graph).is_err());
        }
        let duplicate = resource("owner", "same", vec![tone(0.0)]);
        assert!(validate(&Descriptors::Video(vec![duplicate.clone(), duplicate])).is_err());
        assert!(
            serde_json::from_str::<VideoNode>(
                r#"{"kind":"arbitraryFilter","expression":"movie=file"}"#
            )
            .is_err()
        );
        assert!(
            validate(&Descriptors::Audio(vec![resource(
                "a",
                "r",
                vec![AudioNode::Volume { gain_db: 1.0 }]
            )]))
            .is_err()
        );
    }

    #[test]
    fn stable_owner_then_resource_order_preserves_internal_node_order() {
        let a = resource("a", "z", vec![tone(0.0), blur(1.0)]);
        let b = resource("b", "a", vec![tone(0.2)]);
        let first = Descriptors::Video(vec![b.clone(), a.clone()]);
        let reordered = Descriptors::Video(vec![a, b]);
        let graph = render_lavfi(&first).unwrap();
        assert_eq!(graph, render_lavfi(&reordered).unwrap());
        assert!(graph.find("eq@n0").unwrap() < graph.find("gblur@n1").unwrap());
        assert!(graph.find("gblur@n1").unwrap() < graph.find("eq@n2").unwrap());
        assert_eq!(
            runtime_commands(&first, &reordered).unwrap(),
            UpdatePlan::Commands(vec![])
        );
    }

    #[test]
    fn updates_target_sorted_node_indices_without_rebuilding_the_chain() {
        let old = Descriptors::Video(vec![
            resource("z", "r", vec![blur(0.0)]),
            resource("a", "r", vec![tone(0.0)]),
        ]);
        let new = Descriptors::Video(vec![
            resource("a", "r", vec![tone(-0.2)]),
            resource("z", "r", vec![blur(2.0)]),
        ]);
        let UpdatePlan::Commands(commands) = runtime_commands(&old, &new).unwrap() else {
            panic!("compatible parameters should update in place")
        };
        assert_eq!(commands.len(), 3);
        assert_eq!(commands[0].target, "eq@n0");
        assert_eq!(commands[0].option, "brightness");
        assert_eq!(commands[0].value.parse::<f64>().unwrap(), -0.2);
        assert!(
            commands[1..]
                .iter()
                .all(|command| command.target == "gblur@n1")
        );
        assert_eq!(commands[1].option, "sigma");
        assert_eq!(commands[2].option, "sigmaV");
    }

    #[test]
    fn topology_or_unmeasured_options_require_separate_reconfiguration() {
        let old = Descriptors::Video(vec![resource("a", "r", vec![tone(0.0)])]);
        let changed = Descriptors::Video(vec![resource("a", "r", vec![blur(1.0)])]);
        let removed = Descriptors::Video(vec![]);
        assert_eq!(
            runtime_commands(&old, &changed).unwrap(),
            UpdatePlan::NeedsReconfigure
        );
        assert_eq!(
            runtime_commands(&old, &removed).unwrap(),
            UpdatePlan::NeedsReconfigure
        );
        let eq = |frequency| {
            Descriptors::Audio(vec![resource(
                "a",
                "r",
                vec![AudioNode::Equalizer {
                    frequency,
                    q: 1.0,
                    gain_db: 2.0,
                }],
            )])
        };
        assert_eq!(
            runtime_commands(&eq(1000.0), &eq(2000.0)).unwrap(),
            UpdatePlan::NeedsReconfigure
        );
    }

    #[test]
    fn live_and_offline_volume_share_db_conversion_and_norm_stays_present() {
        let chain = |gain_db, enabled| {
            Descriptors::Audio(vec![resource(
                "a",
                "r",
                vec![
                    AudioNode::Volume { gain_db },
                    AudioNode::Dynaudnorm {
                        enabled,
                        max_gain: 2.0,
                        peak: 0.9,
                    },
                ],
            )])
        };
        let old = chain(0.0, true);
        let new = chain(-6.0, false);
        let graph = render_lavfi(&new).unwrap();
        let UpdatePlan::Commands(commands) = runtime_commands(&old, &new).unwrap() else {
            panic!("gain/bypass changes keep topology")
        };
        let volume = commands
            .iter()
            .find(|command| command.target == "volume@n0")
            .unwrap();
        let linear = volume.value.parse::<f64>().unwrap();
        assert!((linear - 0.5011872336272722).abs() < 1e-12);
        assert!(graph.contains(&format!("volume@n0=volume={}", volume.value)));
        assert!(graph.contains("dynaudnorm@n1=framelen=100:gausssize=5"));
        let bypass = commands
            .iter()
            .find(|command| command.option == "enable")
            .unwrap();
        assert_eq!(bypass.target, "dynaudnorm@n1");
        assert_eq!(bypass.value, "0");
    }
}
