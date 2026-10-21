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
    Gblur {
        sigma: f64,
        steps: u8,
    },
    /// Hue rotation in degrees (FFmpeg `hue=h`), changeable while playing.
    Hue {
        degrees: f64,
    },
    /// `unsharp` luma amount with a 3x3 (`small`) or 7x7 (`large`) matrix.
    Sharpen {
        amount: f64,
        size: SharpenSize,
    },
    /// `hqdn3d` strengths.
    Denoise {
        luma_spatial: f64,
        chroma_spatial: f64,
        luma_temporal: f64,
        chroma_temporal: f64,
    },
    Deblock {
        strength: DeblockStrength,
    },
    /// `deband` threshold (all planes) and range in pixels.
    Deband {
        threshold: f64,
        range: f64,
    },
    /// Applied after the host's own source orientation.
    Transform {
        rotate: u16,
        flip_h: bool,
        flip_v: bool,
    },
    /// Always BWDIF.
    Deinterlace {
        when: DeinterlaceWhen,
        output: DeinterlaceOutput,
    },
    /// Crop to, or pad out to, a display aspect; `x`/`y` place the window
    /// (-1 left/top, 0 centre, 1 right/bottom).
    Reframe {
        mode: ReframeMode,
        aspect: f64,
        x: f64,
        y: f64,
    },
    /// Blended frame-rate conversion (FFmpeg `framerate=fps=N`), 24–120.
    /// At most one across all plugins; always placed after every chain.
    FrameBlend {
        fps: f64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SharpenSize {
    Small,
    Large,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeblockStrength {
    Weak,
    Strong,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeinterlaceWhen {
    Interlaced,
    All,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeinterlaceOutput {
    Frame,
    Field,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReframeMode {
    Crop,
    Pad,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChannelMode {
    Stereo,
    Left,
    Right,
    Mono,
    Swap,
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
    /// Channel routing on a stereo downmix: one side only, mono, swapped.
    Channels {
        mode: ChannelMode,
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

/// One FFmpeg filter of a node. Most nodes are one filter; transform, reframe
/// and channels compose several. An empty parameter name is positional.
struct Filter {
    name: &'static str,
    parameters: Vec<Parameter>,
}

fn filter(name: &'static str, parameters: Vec<Parameter>) -> Filter {
    Filter { name, parameters }
}

/// A finite internal adapter; public callers cannot add filter/option strings.
trait Node {
    fn validate(&self) -> Result<()>;
    fn name(&self) -> &'static str;
    fn parameters(&self) -> Vec<Parameter>;
    fn filters(&self) -> Vec<Filter> {
        vec![filter(self.name(), self.parameters())]
    }
    /// Placed after all chains (keeps its place among such nodes).
    fn last(&self) -> bool {
        false
    }
}

fn integer(name: &str, value: f64, min: f64, max: f64) -> Result<()> {
    range(name, value, min, max)?;
    if value.fract() != 0.0 {
        return Err(ValidationError(format!("{name} must be an integer")));
    }
    Ok(())
}

/// hqdn3d replaces a 0 strength with a derived default, so an explicit 0 is
/// sent as a strength too small to change any pixel.
fn strength(value: f64) -> f64 {
    if value == 0.0 { 0.001 } else { value }
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
            Self::Hue { degrees } => range("degrees", *degrees, -180.0, 180.0),
            Self::Sharpen { amount, .. } => range("amount", *amount, 0.0, 2.0),
            Self::Denoise {
                luma_spatial,
                chroma_spatial,
                luma_temporal,
                chroma_temporal,
            } => {
                range("lumaSpatial", *luma_spatial, 0.0, 20.0)?;
                range("chromaSpatial", *chroma_spatial, 0.0, 20.0)?;
                range("lumaTemporal", *luma_temporal, 0.0, 20.0)?;
                range("chromaTemporal", *chroma_temporal, 0.0, 20.0)
            }
            Self::Deblock { .. } | Self::Deinterlace { .. } => Ok(()),
            Self::Deband {
                threshold,
                range: pixels,
            } => {
                range("threshold", *threshold, 0.003, 0.5)?;
                integer("range", *pixels, 1.0, 64.0)
            }
            Self::Transform { rotate, .. } => {
                if ![0, 90, 180, 270].contains(rotate) {
                    return Err(ValidationError("rotate must be 0, 90, 180 or 270".into()));
                }
                Ok(())
            }
            Self::Reframe { aspect, x, y, .. } => {
                range("aspect", *aspect, 0.25, 4.0)?;
                range("x", *x, -1.0, 1.0)?;
                range("y", *y, -1.0, 1.0)
            }
            Self::FrameBlend { fps } => integer("fps", *fps, 24.0, 120.0),
        }
    }
    fn last(&self) -> bool {
        matches!(self, Self::FrameBlend { .. })
    }
    fn name(&self) -> &'static str {
        match self {
            Self::Eq { .. } => "eq",
            Self::Gblur { .. } => "gblur",
            Self::Hue { .. } => "hue",
            Self::Sharpen { .. } => "unsharp",
            Self::Denoise { .. } => "hqdn3d",
            Self::Deblock { .. } => "deblock",
            Self::Deband { .. } => "deband",
            Self::Transform { .. } => "transform",
            Self::Deinterlace { .. } => "bwdif",
            Self::Reframe { .. } => "reframe",
            Self::FrameBlend { .. } => "framerate",
        }
    }
    fn filters(&self) -> Vec<Filter> {
        match self {
            Self::Transform {
                rotate,
                flip_h,
                flip_v,
            } => {
                // Rotate, then mirror the rotated picture. 180 degrees is both
                // flips, so flips cancel in pairs.
                let (turn, half) = match rotate {
                    90 => (Some("clock"), false),
                    270 => (Some("cclock"), false),
                    180 => (None, true),
                    _ => (None, false),
                };
                let mut filters = Vec::new();
                if let Some(dir) = turn {
                    filters.push(filter("transpose", vec![parameter("dir", dir, false)]));
                }
                if *flip_h != half {
                    filters.push(filter("hflip", vec![]));
                }
                if *flip_v != half {
                    filters.push(filter("vflip", vec![]));
                }
                if filters.is_empty() {
                    filters.push(filter("null", vec![]));
                }
                filters
            }
            Self::Reframe { mode, aspect, x, y } => {
                // Display aspect is pixel width times sar. Even sizes keep
                // 4:2:0 chroma planes whole.
                let (name, pick, round, dx, dy) = match mode {
                    ReframeMode::Crop => ("crop", "min", "trunc", "(iw-ow)", "(ih-oh)"),
                    ReframeMode::Pad => ("pad", "max", "ceil", "(ow-iw)", "(oh-ih)"),
                };
                vec![filter(
                    name,
                    vec![
                        parameter(
                            "w",
                            format!("'2*{round}({pick}(iw,ih*{aspect}/sar)/2)'"),
                            false,
                        ),
                        parameter(
                            "h",
                            format!("'2*{round}({pick}(ih,iw*sar/{aspect})/2)'"),
                            false,
                        ),
                        parameter("x", format!("'{dx}*(1+({x}))/2'"), false),
                        parameter("y", format!("'{dy}*(1+({y}))/2'"), false),
                    ],
                )]
            }
            _ => vec![filter(self.name(), self.parameters())],
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
            Self::Hue { degrees } => vec![parameter("h", degrees, true)],
            Self::Sharpen { amount, size } => {
                let matrix = match size {
                    SharpenSize::Small => 3,
                    SharpenSize::Large => 7,
                };
                vec![
                    parameter("lx", matrix, false),
                    parameter("ly", matrix, false),
                    parameter("la", amount, false),
                ]
            }
            Self::Denoise {
                luma_spatial,
                chroma_spatial,
                luma_temporal,
                chroma_temporal,
            } => vec![
                parameter("luma_spatial", strength(*luma_spatial), false),
                parameter("chroma_spatial", strength(*chroma_spatial), false),
                parameter("luma_tmp", strength(*luma_temporal), false),
                parameter("chroma_tmp", strength(*chroma_temporal), false),
            ],
            Self::Deblock { strength } => vec![parameter(
                "filter",
                match strength {
                    DeblockStrength::Weak => "weak",
                    DeblockStrength::Strong => "strong",
                },
                false,
            )],
            Self::Deband { threshold, range } => vec![
                parameter("1thr", threshold, false),
                parameter("2thr", threshold, false),
                parameter("3thr", threshold, false),
                parameter("4thr", threshold, false),
                parameter("r", range, false),
            ],
            Self::Deinterlace { when, output } => vec![
                parameter(
                    "mode",
                    match output {
                        DeinterlaceOutput::Frame => "send_frame",
                        DeinterlaceOutput::Field => "send_field",
                    },
                    false,
                ),
                parameter(
                    "deint",
                    match when {
                        DeinterlaceWhen::Interlaced => "interlaced",
                        DeinterlaceWhen::All => "all",
                    },
                    false,
                ),
            ],
            Self::FrameBlend { fps } => vec![parameter("fps", fps, false)],
            Self::Transform { .. } | Self::Reframe { .. } => Vec::new(),
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
            Self::Channels { .. } => Ok(()),
        }
    }
    fn name(&self) -> &'static str {
        match self {
            Self::Volume { .. } => "volume",
            Self::Equalizer { .. } => "equalizer",
            Self::Treble { .. } => "treble",
            Self::Dynaudnorm { .. } => "dynaudnorm",
            Self::Channels { .. } => "channels",
        }
    }
    fn filters(&self) -> Vec<Filter> {
        let Self::Channels { mode } = self else {
            return vec![filter(self.name(), self.parameters())];
        };
        let routing = match mode {
            ChannelMode::Stereo => return vec![filter("anull", vec![])],
            ChannelMode::Left => "stereo|c0=c0|c1=c0",
            ChannelMode::Right => "stereo|c0=c1|c1=c1",
            ChannelMode::Mono => "stereo|c0=0.5*c0+0.5*c1|c1=0.5*c0+0.5*c1",
            ChannelMode::Swap => "stereo|c0=c1|c1=c0",
        };
        // Mix to stereo first (the graph inserts the resampler), so the
        // routing always sees exactly two channels.
        vec![
            filter(
                "aformat",
                vec![parameter("channel_layouts", "stereo", false)],
            ),
            filter("pan", vec![parameter("", routing, false)]),
        ]
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
            Self::Channels { .. } => Vec::new(),
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
    let last = ordered
        .iter()
        .flat_map(|r| &r.nodes)
        .filter(|n| n.last())
        .count();
    if last > 1 {
        return Err(ValidationError(
            "only one frameBlend node across all plugins (the frame rate changes once)".into(),
        ));
    }
    Ok(ordered)
}

/// The chain's nodes in graph order: resources in stable order, nodes in their
/// own order, then the nodes that belong after every chain (`frameBlend`), so
/// time-domain filters (denoise, deinterlace) work on the original frames.
fn sequence<'a, N: Node>(resources: &[&'a Resource<N>]) -> Vec<&'a N> {
    let nodes = resources.iter().flat_map(|r| &r.nodes);
    let (mut chain, last): (Vec<_>, Vec<_>) = nodes.partition(|n| !n.last());
    chain.extend(last);
    chain
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

/// `name@n3` for a node's first filter, `name@n3x1` for the following ones.
fn label(name: &str, prefix: &str, index: usize, part: usize) -> String {
    if part == 0 {
        format!("{name}@{prefix}{index}")
    } else {
        format!("{name}@{prefix}{index}x{part}")
    }
}

fn render<N: Node>(resources: &[Resource<N>], prefix: &str, empty: &str) -> Result<String> {
    let resources = ordered(resources)?;
    let filters: Vec<_> = sequence(&resources)
        .into_iter()
        .enumerate()
        .flat_map(|(index, node)| {
            node.filters()
                .into_iter()
                .enumerate()
                .map(move |(part, filter)| {
                    let parameters = filter
                        .parameters
                        .iter()
                        .map(|p| {
                            if p.name.is_empty() {
                                p.value.clone()
                            } else {
                                format!("{}={}", p.name, p.value)
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(":");
                    let label = label(filter.name, prefix, index, part);
                    if parameters.is_empty() {
                        label
                    } else {
                        format!("{label}={parameters}")
                    }
                })
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
    for (old, new) in old.iter().zip(&new) {
        if old.manifest_id != new.manifest_id
            || old.resource_id != new.resource_id
            || old.nodes.len() != new.nodes.len()
        {
            return Ok(UpdatePlan::NeedsReconfigure);
        }
    }
    let mut commands = Vec::new();
    for (index, (old, new)) in sequence(&old).into_iter().zip(sequence(&new)).enumerate() {
        let (old, new) = (old.filters(), new.filters());
        if old.len() != new.len() || old.iter().zip(&new).any(|(a, b)| a.name != b.name) {
            return Ok(UpdatePlan::NeedsReconfigure);
        }
        for (part, (old, new)) in old.into_iter().zip(new).enumerate() {
            for (old, parameter) in old.parameters.into_iter().zip(new.parameters) {
                if old.value != parameter.value {
                    if !parameter.runtime {
                        return Ok(UpdatePlan::NeedsReconfigure);
                    }
                    commands.push(RuntimeCommand {
                        target: label(new.name, prefix, index, part),
                        option: parameter.name,
                        value: parameter.value,
                    });
                }
            }
        }
    }
    Ok(UpdatePlan::Commands(commands))
}

/// Only the locally exercised command families are planned: eq controls,
/// gblur sigma/sigmaV, hue h, volume, EQ/treble gain, and norm
/// maxgain/peak/enable.
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

    fn video(nodes: Vec<VideoNode>) -> String {
        render_lavfi(&Descriptors::Video(vec![resource("a", "r", nodes)])).unwrap()
    }

    #[test]
    fn round_two_video_nodes_render_fixed_ffmpeg_filters() {
        let graph = video(vec![
            VideoNode::Hue { degrees: -30.0 },
            VideoNode::Sharpen {
                amount: 1.5,
                size: SharpenSize::Large,
            },
            VideoNode::Denoise {
                luma_spatial: 4.0,
                chroma_spatial: 3.0,
                luma_temporal: 6.0,
                chroma_temporal: 0.0,
            },
            VideoNode::Deblock {
                strength: DeblockStrength::Strong,
            },
            VideoNode::Deband {
                threshold: 0.02,
                range: 16.0,
            },
            VideoNode::Deinterlace {
                when: DeinterlaceWhen::Interlaced,
                output: DeinterlaceOutput::Field,
            },
        ]);
        assert_eq!(
            graph,
            "hue@n0=h=-30,unsharp@n1=lx=7:ly=7:la=1.5,\
             hqdn3d@n2=luma_spatial=4:chroma_spatial=3:luma_tmp=6:chroma_tmp=0.001,\
             deblock@n3=filter=strong,deband@n4=1thr=0.02:2thr=0.02:3thr=0.02:4thr=0.02:r=16,\
             bwdif@n5=mode=send_field:deint=interlaced"
        );
        let crop = video(vec![VideoNode::Reframe {
            mode: ReframeMode::Crop,
            aspect: 2.35,
            x: 0.0,
            y: -1.0,
        }]);
        assert_eq!(
            crop,
            "crop@n0=w='2*trunc(min(iw,ih*2.35/sar)/2)':h='2*trunc(min(ih,iw*sar/2.35)/2)':\
             x='(iw-ow)*(1+(0))/2':y='(ih-oh)*(1+(-1))/2'"
        );
        let pad = video(vec![VideoNode::Reframe {
            mode: ReframeMode::Pad,
            aspect: 1.0,
            x: 0.5,
            y: 0.0,
        }]);
        assert!(pad.starts_with("pad@n0=w='2*ceil(max(iw,ih*1/sar)/2)'"));
        assert!(pad.contains("x='(ow-iw)*(1+(0.5))/2'"));
    }

    #[test]
    fn transform_composes_rotation_then_mirroring_and_cancels_flip_pairs() {
        let transform = |rotate, flip_h, flip_v| {
            video(vec![VideoNode::Transform {
                rotate,
                flip_h,
                flip_v,
            }])
        };
        assert_eq!(transform(0, false, false), "null@n0");
        assert_eq!(transform(90, false, false), "transpose@n0=dir=clock");
        assert_eq!(
            transform(270, true, false),
            "transpose@n0=dir=cclock,hflip@n0x1"
        );
        assert_eq!(transform(180, false, false), "hflip@n0,vflip@n0x1");
        assert_eq!(transform(180, true, false), "vflip@n0");
        assert_eq!(transform(180, true, true), "null@n0");
        assert_eq!(transform(0, false, true), "vflip@n0");
        let rejected = Descriptors::Video(vec![resource(
            "a",
            "r",
            vec![VideoNode::Transform {
                rotate: 45,
                flip_h: false,
                flip_v: false,
            }],
        )]);
        assert!(validate(&rejected).is_err());
        // Later nodes keep their own index after a multi-filter node.
        let chain = video(vec![
            VideoNode::Transform {
                rotate: 180,
                flip_h: false,
                flip_v: false,
            },
            VideoNode::Hue { degrees: 10.0 },
        ]);
        assert_eq!(chain, "hflip@n0,vflip@n0x1,hue@n1=h=10");
    }

    /// The owned chain runs after the host's orientation filter, so a 90°
    /// transform on a container-rotated (90°) 1920x1080 source shows 1920x1080.
    #[test]
    fn transform_applies_after_source_orientation() {
        let size = |rotate: u16, (w, h): (u32, u32)| {
            if rotate % 180 == 90 { (h, w) } else { (w, h) }
        };
        let oriented = size(90, (1920, 1080));
        assert_eq!(oriented, (1080, 1920));
        assert_eq!(size(90, oriented), (1920, 1080));
        assert_eq!(size(180, oriented), (1080, 1920));
        assert!(
            crate::source_geometry::INITIAL_FILTERS.starts_with("@vestra-orientation:"),
            "the orientation stage is installed before any owned `vf add`"
        );
    }

    #[test]
    fn hue_changes_in_place_and_other_new_options_reconfigure() {
        let hue = |degrees| {
            Descriptors::Video(vec![resource("a", "r", vec![VideoNode::Hue { degrees }])])
        };
        let UpdatePlan::Commands(commands) = runtime_commands(&hue(0.0), &hue(45.0)).unwrap()
        else {
            panic!("hue is a runtime option")
        };
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].target, "hue@n0");
        assert_eq!(commands[0].option, "h");
        assert_eq!(commands[0].value, "45");
        for range in [-180.5, 180.5, f64::NAN] {
            assert!(validate(&hue(range)).is_err());
        }
        let sharpen = |amount| {
            Descriptors::Video(vec![resource(
                "a",
                "r",
                vec![VideoNode::Sharpen {
                    amount,
                    size: SharpenSize::Small,
                }],
            )])
        };
        assert_eq!(
            runtime_commands(&sharpen(0.5), &sharpen(1.0)).unwrap(),
            UpdatePlan::NeedsReconfigure
        );
        assert!(validate(&sharpen(2.1)).is_err());
        let deband = |threshold, range| {
            Descriptors::Video(vec![resource(
                "a",
                "r",
                vec![VideoNode::Deband { threshold, range }],
            )])
        };
        assert!(validate(&deband(0.003, 1.0)).is_ok());
        assert!(validate(&deband(0.5, 64.0)).is_ok());
        assert!(validate(&deband(0.002, 16.0)).is_err());
        assert!(validate(&deband(0.02, 16.5)).is_err());
        assert!(validate(&deband(0.02, 65.0)).is_err());
        let reframe = |aspect, x| {
            Descriptors::Video(vec![resource(
                "a",
                "r",
                vec![VideoNode::Reframe {
                    mode: ReframeMode::Crop,
                    aspect,
                    x,
                    y: 0.0,
                }],
            )])
        };
        assert!(validate(&reframe(0.25, -1.0)).is_ok());
        assert!(validate(&reframe(0.2, 0.0)).is_err());
        assert!(validate(&reframe(4.0, 1.1)).is_err());
        let wire: VideoNode = serde_json::from_str(
            r#"{"kind":"denoise","lumaSpatial":1,"chromaSpatial":2,"lumaTemporal":3,"chromaTemporal":4}"#,
        )
        .unwrap();
        assert!(
            matches!(wire, VideoNode::Denoise { chroma_temporal, .. } if chroma_temporal == 4.0)
        );
        assert!(
            serde_json::from_str::<VideoNode>(
                r#"{"kind":"transform","rotate":90,"flipH":false,"flipV":true}"#
            )
            .is_ok()
        );
        assert!(
            serde_json::from_str::<VideoNode>(
                r#"{"kind":"deinterlace","when":"bob","output":"frame"}"#
            )
            .is_err()
        );
    }

    #[test]
    fn frame_blend_goes_after_every_chain_and_only_once() {
        let blend = |fps| VideoNode::FrameBlend { fps };
        let denoise = VideoNode::Denoise {
            luma_spatial: 1.0,
            chroma_spatial: 1.0,
            luma_temporal: 1.0,
            chroma_temporal: 1.0,
        };
        // "a" sorts first, but its frameBlend still runs after "b"'s denoise.
        let graph = Descriptors::Video(vec![
            resource("a", "r", vec![blend(48.0), tone(0.1)]),
            resource("b", "r", vec![denoise]),
        ]);
        let text = render_lavfi(&graph).unwrap();
        assert!(text.starts_with("eq@n0="), "{text}");
        assert!(text.find("hqdn3d@n1").unwrap() < text.find("framerate@n2").unwrap());
        assert!(text.ends_with("framerate@n2=fps=48"), "{text}");
        // A second one anywhere is invalid.
        for second in [
            vec![resource("a", "r", vec![blend(48.0), blend(60.0)])],
            vec![
                resource("a", "r", vec![blend(48.0)]),
                resource("b", "r", vec![blend(60.0)]),
            ],
        ] {
            assert!(validate(&Descriptors::Video(second)).is_err());
        }
        for fps in [23.0, 121.0, 47.5, f64::NAN] {
            let one = Descriptors::Video(vec![resource("a", "r", vec![blend(fps)])]);
            assert!(validate(&one).is_err(), "{fps}");
        }
        // Changing fps rebuilds; runtime updates elsewhere keep their indices.
        let base = Descriptors::Video(vec![
            resource("a", "r", vec![blend(48.0), tone(0.1)]),
            resource("b", "r", vec![tone(0.0)]),
        ]);
        let faster = Descriptors::Video(vec![
            resource("a", "r", vec![blend(60.0), tone(0.1)]),
            resource("b", "r", vec![tone(0.0)]),
        ]);
        assert_eq!(
            runtime_commands(&base, &faster).unwrap(),
            UpdatePlan::NeedsReconfigure
        );
        let brighter = Descriptors::Video(vec![
            resource("a", "r", vec![blend(48.0), tone(0.1)]),
            resource("b", "r", vec![tone(0.3)]),
        ]);
        let UpdatePlan::Commands(commands) = runtime_commands(&base, &brighter).unwrap() else {
            panic!("eq changes in place");
        };
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].target, "eq@n1");
        assert!(serde_json::from_str::<VideoNode>(r#"{"kind":"frameBlend","fps":48}"#).is_ok());
    }

    #[test]
    fn channel_modes_route_a_stereo_mix() {
        let channels = |mode| {
            render_lavfi(&Descriptors::Audio(vec![resource(
                "a",
                "r",
                vec![
                    AudioNode::Volume { gain_db: 0.0 },
                    AudioNode::Channels { mode },
                ],
            )]))
            .unwrap()
        };
        assert_eq!(channels(ChannelMode::Stereo), "volume@n0=volume=1,anull@n1");
        assert_eq!(
            channels(ChannelMode::Left),
            "volume@n0=volume=1,aformat@n1=channel_layouts=stereo,pan@n1x1=stereo|c0=c0|c1=c0"
        );
        assert!(channels(ChannelMode::Swap).ends_with("pan@n1x1=stereo|c0=c1|c1=c0"));
        assert!(channels(ChannelMode::Mono).contains("c0=0.5*c0+0.5*c1"));
        assert!(channels(ChannelMode::Right).ends_with("stereo|c0=c1|c1=c1"));
        let audio =
            |mode| Descriptors::Audio(vec![resource("a", "r", vec![AudioNode::Channels { mode }])]);
        assert_eq!(
            runtime_commands(&audio(ChannelMode::Left), &audio(ChannelMode::Right)).unwrap(),
            UpdatePlan::NeedsReconfigure
        );
        assert_eq!(
            runtime_commands(&audio(ChannelMode::Stereo), &audio(ChannelMode::Left)).unwrap(),
            UpdatePlan::NeedsReconfigure
        );
        assert!(serde_json::from_str::<AudioNode>(r#"{"kind":"channels","mode":"swap"}"#).is_ok());
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
