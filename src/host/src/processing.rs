//! Finite owner-aware runtime candidate. Not a general filter graph API.
use crate::processing_operations::{self as operations, Admission, Key, Stage};
use crate::{
    RpcError,
    effects::{self, AudioNode, Descriptors, Resource, UpdatePlan, VideoNode},
};
use rsmpv::Mpv;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
#[derive(Clone, Deserialize, Serialize)]
#[serde(
    tag = "stage",
    rename_all = "lowercase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum Change {
    Video {
        resource_id: String,
        revision: u64,
        expected_generation: u64,
        nodes: Vec<VideoNode>,
    },
    Audio {
        resource_id: String,
        revision: u64,
        expected_generation: u64,
        nodes: Vec<AudioNode>,
        #[serde(default)]
        offset_ms: i32,
    },
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrozenProcessing {
    pub revision: u64,
    pub video: Vec<Resource<VideoNode>>,
    pub audio: Vec<Resource<AudioNode>>,
    pub audio_offset_ms: i32,
}
#[derive(Default)]
pub struct Processing {
    config_revision: u64,
    manifests: BTreeMap<u64, String>,
    operations: BTreeMap<u64, operations::Book>,
    uncertain: BTreeMap<String, String>,
    source_generation: u64,
    source_phase: String,
    load: Option<(i64, crate::session::IntentTicket)>,
    confirmations: BTreeMap<String, Value>,
    video: BTreeMap<(u64, String), (u64, Resource<VideoNode>)>,
    audio: BTreeMap<(u64, String), (u64, Resource<AudioNode>)>,
    audio_offsets: BTreeMap<(u64, String), i32>,
    /// `(original, copy)`: the `hwdec` mode before the owned video chain
    /// needed frames in memory, and the copy-back mode set for it.
    hwdec: Option<(String, String)>,
}
impl Processing {
    pub fn register(&mut self, owner: u64, id: String) {
        self.manifests.insert(owner, id);
        self.operations.entry(owner).or_default();
    }
    pub fn state(&self) -> Value {
        json!({"configurationRevision":self.config_revision,"audioOffsetMs":self.audio_offsets.values().sum::<i32>(),"source":{"generation":self.source_generation,"phase":self.source_phase,"load":self.load.map(|(entry,t)|json!({"entry":entry,"generation":t.stamp.generation,"cycle":t.cycle})),"stages":self.confirmations},"backendUncertain":self.uncertain,"video":self.video.iter().map(|((owner,_),(revision,resource))|json!({"owner":owner,"revision":revision,"resource":resource})).collect::<Vec<_>>(),"audio":self.audio.iter().map(|((owner,_),(revision,resource))|json!({"owner":owner,"revision":revision,"resource":resource,"offsetMs":self.audio_offsets.get(&(*owner,resource.resource_id.clone())).copied().unwrap_or(0)})).collect::<Vec<_>>()})
    }
    pub fn freeze(
        &self,
        expected: u64,
        load: (i64, crate::session::IntentTicket),
        mpv: &Mpv,
    ) -> Result<FrozenProcessing, RpcError> {
        if expected != self.config_revision {
            return Err(RpcError::new("stale", "processing configuration changed"));
        }
        if self.load != Some(load)
            || !self.uncertain.is_empty()
            || ["video", "audio"]
                .iter()
                .any(|stage| self.confirmations[*stage]["status"] != "applied")
        {
            return Err(RpcError::new(
                "unavailable",
                "current source processing is not confirmed",
            ));
        }
        let audio_offset_ms = combined_offset(&self.audio_offsets)?;
        confirm_offset(mpv, audio_offset_ms)?;
        Ok(FrozenProcessing {
            audio_offset_ms,
            revision: self.config_revision,
            video: self.video.values().map(|(_, r)| r.clone()).collect(),
            audio: self
                .audio
                .values()
                .filter(|(_, r)| !r.nodes.is_empty())
                .map(|(_, r)| r.clone())
                .collect(),
        })
    }
    pub fn note_source(&mut self, generation: u64, phase: crate::session::Phase) {
        let name = match phase {
            crate::session::Phase::Empty => "empty",
            crate::session::Phase::Loading => "loading",
            crate::session::Phase::Ready => "ready",
            crate::session::Phase::Ended => "ended",
            crate::session::Phase::Error => "error",
        };
        if generation != self.source_generation
            || (name == "loading" && self.source_phase != "loading")
            || matches!(
                phase,
                crate::session::Phase::Empty | crate::session::Phase::Error
            )
        {
            self.load = None;
            for stage in ["video", "audio"] {
                self.confirmations
                    .insert(stage.into(), json!({"status":"waiting","error":null}));
            }
        }
        self.source_generation = generation;
        self.source_phase = name.into();
    }
    /// Invoked synchronously on the control worker after the actual current
    /// FileLoaded association, including restart's new entry at same generation.
    /// Full typed serialization recovers runtime options absent from mpv's
    /// original vf/af property strings. No visible/PTS fence is claimed.
    pub fn replay_ready(
        &mut self,
        mpv: &Mpv,
        entry: i64,
        ticket: crate::session::IntentTicket,
    ) -> Result<(), RpcError> {
        if ticket.stamp.generation != self.source_generation || self.source_phase != "ready" {
            return Err(RpcError::new(
                "stale",
                "processing replay load no longer current",
            ));
        }
        if self.load == Some((entry, ticket)) {
            return Ok(());
        }
        self.load = Some((entry, ticket));
        let mut failures = Vec::new();
        for (stage, graph) in [("video", self.video_graph()), ("audio", self.audio_graph())] {
            let empty = match &graph {
                Descriptors::Video(r) => r.is_empty(),
                Descriptors::Audio(r) => r.is_empty(),
            };
            let mut result = if let Some(reason) = self.uncertain.get(stage) {
                Err(RpcError::new("backend-uncertain", reason.clone()))
            } else if empty {
                Ok(())
            } else {
                if stage == "video" {
                    self.frames_in_memory(mpv, true);
                }
                set_graph(mpv, &graph)
            };
            if stage == "audio" && result.is_ok() {
                result = set_offset(mpv, combined_offset(&self.audio_offsets)?);
            }
            match result {
                Ok(()) => {
                    self.confirmations
                        .insert(stage.into(), json!({"status":"applied","error":null}));
                }
                Err(error) => {
                    self.uncertain.insert(
                        stage.into(),
                        format!("replaying load {entry}: {}", error.message),
                    );
                    self.confirmations.insert(
                        stage.into(),
                        json!({"status":"uncertain","error":error.message}),
                    );
                    failures.push(format!("{stage}: {}", error.message));
                }
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(RpcError::new("backend-uncertain", failures.join("; ")))
        }
    }
    fn video_graph(&self) -> Descriptors {
        Descriptors::Video(self.video.values().map(|(_, r)| r.clone()).collect())
    }
    fn audio_graph(&self) -> Descriptors {
        Descriptors::Audio(
            self.audio
                .values()
                .filter(|(_, r)| !r.nodes.is_empty())
                .map(|(_, r)| r.clone())
                .collect(),
        )
    }
    /// Token outcomes belong to an owner generation. Querying never reapplies.
    pub fn operation(&self, owner: u64, params: &Value) -> Result<Value, RpcError> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Query {
            stage: String,
            resource_id: String,
            revision: u64,
        }
        let query: Query =
            serde_json::from_value(params.clone()).map_err(|e| RpcError::invalid(e.to_string()))?;
        if query.resource_id.is_empty()
            || query.resource_id.len() > 128
            || query.revision == 0
            || query.revision > operations::Book::MAX_REVISION
        {
            return Err(RpcError::invalid(
                "resourceId and positive safe revision required",
            ));
        }
        let stage = match query.stage.as_str() {
            "video" => Stage::Video,
            "audio" => Stage::Audio,
            _ => return Err(RpcError::invalid("unknown processing stage")),
        };
        let book = self
            .operations
            .get(&owner)
            .ok_or_else(|| RpcError::new("disabled", "owner is not registered"))?;
        Ok(
            match book.query(
                &Key {
                    stage,
                    resource_id: query.resource_id,
                },
                query.revision,
            ) {
                operations::Query::Unknown => json!({"status":"unknown"}),
                operations::Query::UnknownExpired => json!({"status":"unknownExpired"}),
                operations::Query::Pending => json!({"status":"pending"}),
                operations::Query::Finished(Ok(result)) => {
                    json!({"status":"completed","result":result})
                }
                operations::Query::Finished(Err(error)) => {
                    json!({"status":"failed","error":{"code":error.code,"message":error.message}})
                }
            },
        )
    }
    pub fn replace(
        &mut self,
        mpv: &Mpv,
        owner: u64,
        generation: u64,
        source: Source,
        params: &Value,
    ) -> Result<Value, RpcError> {
        let change: Change =
            serde_json::from_value(params.clone()).map_err(|e| RpcError::invalid(e.to_string()))?;
        let manifest = self
            .manifests
            .get(&owner)
            .ok_or_else(|| RpcError::new("disabled", "owner is not registered"))?
            .clone();
        let (stage, resource_id, revision, graph) = match &change {
            Change::Video {
                resource_id,
                revision,
                nodes,
                ..
            } => (
                Stage::Video,
                resource_id,
                *revision,
                Descriptors::Video(vec![Resource {
                    manifest_id: manifest,
                    resource_id: resource_id.clone(),
                    nodes: nodes.clone(),
                }]),
            ),
            Change::Audio {
                resource_id,
                revision,
                nodes,
                ..
            } => (
                Stage::Audio,
                resource_id,
                *revision,
                Descriptors::Audio(vec![Resource {
                    manifest_id: manifest,
                    resource_id: resource_id.clone(),
                    nodes: nodes.clone(),
                }]),
            ),
        };
        if let Change::Audio { offset_ms, .. } = &change
            && (!(-10_000..=10_000).contains(offset_ms) || offset_ms % 10 != 0)
        {
            return Err(RpcError::invalid(
                "audio offsetMs must be -10000..10000 in 10ms increments",
            ));
        }
        // Bound typed data BEFORE retaining it. Unknown fields and arbitrary JSON
        // never enter the book. Normalize f64s through the typed serializer.
        if resource_id.is_empty()
            || resource_id.len() > 128
            || resource_id.chars().any(char::is_control)
        {
            return Err(RpcError::invalid("bounded resourceId required"));
        }
        // An empty chain removes the resource (both stages).
        let validation = match &graph {
            Descriptors::Audio(r) if r.len() == 1 && r[0].nodes.is_empty() => {
                Descriptors::Audio(vec![])
            }
            Descriptors::Video(r) if r.len() == 1 && r[0].nodes.is_empty() => {
                Descriptors::Video(vec![])
            }
            _ => graph,
        };
        effects::validate(&validation).map_err(|e| RpcError::invalid(e.to_string()))?;
        if revision == 0 || revision > operations::Book::MAX_REVISION {
            return Err(RpcError::invalid("positive safe revision required"));
        }
        let key = Key {
            stage,
            resource_id: resource_id.clone(),
        };
        let payload =
            serde_json::to_value(&change).map_err(|e| RpcError::invalid(e.to_string()))?;
        let book = self
            .operations
            .get_mut(&owner)
            .expect("registered owner book");
        match book.admit(key.clone(), revision, payload) {
            Admission::Execute => (),
            Admission::Replay(Ok(result)) => return Ok(result),
            Admission::Replay(Err(error)) => return Err(RpcError::new(error.code, error.message)),
            Admission::Pending => {
                return Err(RpcError::new(
                    "pending",
                    "operation is pending; query its token",
                ));
            }
            Admission::Conflict => {
                return Err(RpcError::new(
                    "conflict",
                    "processing token was used with different parameters",
                ));
            }
            Admission::UnknownExpired => {
                return Err(RpcError::new(
                    "unknown-expired",
                    "operation result expired; do not reexecute this token",
                ));
            }
            Admission::Full => {
                return Err(RpcError::new(
                    "busy",
                    "processing operation retention capacity reached",
                ));
            }
        }
        let result = self.apply_change(mpv, owner, generation, source, change);
        let outcome = result.as_ref().cloned().map_err(|e| operations::Failure {
            code: e.code.clone(),
            message: e.message.clone(),
        });
        self.operations
            .get_mut(&owner)
            .expect("registered owner book")
            .finish(&key, revision, outcome)
            .map_err(|e| {
                RpcError::new("internal", format!("operation retention: {}", e.message))
            })?;
        result
    }
    fn apply_change(
        &mut self,
        mpv: &Mpv,
        owner: u64,
        generation: u64,
        source: Source,
        change: Change,
    ) -> Result<Value, RpcError> {
        let manifest = self
            .manifests
            .get(&owner)
            .ok_or_else(|| RpcError::new("disabled", "owner is not registered"))?
            .clone();
        if source == Source::NotReady {
            return Err(RpcError::new(
                "unavailable",
                "source must be ready before applying processing",
            ));
        }
        let removal = match &change {
            Change::Video { nodes, .. } => nodes.is_empty(),
            Change::Audio {
                nodes, offset_ms, ..
            } => nodes.is_empty() && *offset_ms == 0,
        };
        // A generated stream's frames and samples go from the producer to the
        // renderer and the audio device without mpv, so no chain can act on
        // them. Removing a chain is still fine: it only changes what the next
        // decoded media gets.
        if source == Source::Generated && !removal {
            return Err(RpcError::new(
                "unsupported",
                "generated streams (openStream) do not pass through the player's filters; \
                 the chain applies to the next media the player decodes",
            ));
        }
        if self.config_revision >= operations::Book::MAX_REVISION {
            return Err(RpcError::new(
                "busy",
                "processing configuration revision exhausted",
            ));
        }
        let stage = match &change {
            Change::Video { .. } => "video",
            Change::Audio { .. } => "audio",
        };
        if let Some(reason) = self.uncertain.get(stage) {
            return Err(RpcError::new("backend-uncertain", reason.clone()));
        }
        if self.load.is_none() && source != Source::Generated {
            return Err(RpcError::new(
                "unavailable",
                "current load processing is not yet confirmed",
            ));
        }
        let result = match change {
            Change::Video {
                resource_id,
                revision,
                expected_generation,
                nodes,
            } => {
                check(&resource_id, revision, expected_generation, generation)?;
                let key = (owner, resource_id.clone());
                if self
                    .video
                    .get(&key)
                    .is_some_and(|(old, _)| revision <= *old)
                {
                    return Err(RpcError::new("stale", "resource revision must increase"));
                }
                let before = self.video_graph();
                let mut next = self.video.clone();
                if nodes.is_empty() {
                    next.remove(&key);
                } else {
                    next.insert(
                        key,
                        (
                            revision,
                            Resource {
                                manifest_id: manifest,
                                resource_id,
                                nodes,
                            },
                        ),
                    );
                }
                let after = Descriptors::Video(next.values().map(|(_, r)| r.clone()).collect());
                if !next.is_empty() {
                    self.frames_in_memory(mpv, true);
                }
                let applied = if before == after {
                    Ok(())
                } else {
                    apply(mpv, &before, &after)
                };
                if let Err(error) = applied {
                    let keep = !self.video.is_empty();
                    self.frames_in_memory(mpv, keep);
                    if error.code == "backend-uncertain" {
                        self.uncertain.insert(stage.into(), error.message.clone());
                        self.confirmations.insert(
                            stage.into(),
                            json!({"status":"uncertain","error":error.message}),
                        );
                    }
                    return Err(error);
                }
                self.video = next;
                if self.video.is_empty() {
                    self.frames_in_memory(mpv, false);
                }
                self.confirmations
                    .insert("video".into(), json!({"status":"applied","error":null}));
                json!({"stage":"video","revision":revision,"generation":generation,"status":"applied"})
            }
            Change::Audio {
                resource_id,
                revision,
                expected_generation,
                nodes,
                offset_ms,
            } => {
                check(&resource_id, revision, expected_generation, generation)?;
                let key = (owner, resource_id.clone());
                if self
                    .audio
                    .get(&key)
                    .is_some_and(|(old, _)| revision <= *old)
                {
                    return Err(RpcError::new("stale", "resource revision must increase"));
                }
                let before = self.audio_graph();
                let old_offset = combined_offset(&self.audio_offsets)?;
                let mut next_offsets = self.audio_offsets.clone();
                next_offsets.insert(key.clone(), offset_ms);
                let mut next = self.audio.clone();
                if nodes.is_empty() && offset_ms == 0 {
                    next_offsets.remove(&key);
                    next.remove(&key);
                } else {
                    next.insert(
                        key,
                        (
                            revision,
                            Resource {
                                manifest_id: manifest,
                                resource_id,
                                nodes,
                            },
                        ),
                    );
                }
                let next_offset = combined_offset(&next_offsets)?;
                let after = Descriptors::Audio(
                    next.values()
                        .filter(|(_, r)| !r.nodes.is_empty())
                        .map(|(_, r)| r.clone())
                        .collect(),
                );
                if let Err(error) = apply_audio(mpv, &before, &after, old_offset, next_offset) {
                    if error.code == "backend-uncertain" {
                        self.uncertain.insert(stage.into(), error.message.clone());
                        self.confirmations.insert(
                            stage.into(),
                            json!({"status":"uncertain","error":error.message}),
                        );
                    }
                    return Err(error);
                }
                self.audio = next;
                self.audio_offsets = next_offsets;
                self.confirmations
                    .insert("audio".into(), json!({"status":"applied","error":null}));
                json!({"stage":"audio","revision":revision,"generation":generation,"status":"applied"})
            }
        };
        self.config_revision += 1;
        Ok(result)
    }
    pub fn disable(&mut self, mpv: &Mpv, owner: u64) -> Result<(), RpcError> {
        let removed_configuration = self
            .video
            .keys()
            .chain(self.audio.keys())
            .any(|(id, _)| *id == owner);
        let before_video = self.video_graph();
        let before_audio = self.audio_graph();
        let old_offset = combined_offset(&self.audio_offsets)?;
        self.video.retain(|(id, _), _| *id != owner);
        self.audio.retain(|(id, _), _| *id != owner);
        self.audio_offsets.retain(|(id, _), _| *id != owner);
        let next_offset = combined_offset(&self.audio_offsets)?;
        self.manifests.remove(&owner);
        self.operations.remove(&owner);
        let after_video = self.video_graph();
        let after_audio = self.audio_graph();
        if removed_configuration {
            if self.config_revision >= operations::Book::MAX_REVISION {
                self.uncertain.insert(
                    "configuration".into(),
                    "configuration revision exhausted".into(),
                );
            } else {
                self.config_revision += 1;
            }
        }
        // Revoke logical ownership regardless of backend health. Attempt BOTH stages;
        // an unsuccessful cleanup must remain visible and blocks further updates.
        let mut failures = Vec::new();
        for (stage, before, after) in [
            ("video", before_video, after_video),
            ("audio", before_audio, after_audio),
        ] {
            let result = if stage == "audio" && (before != after || old_offset != next_offset) {
                apply_audio(mpv, &before, &after, old_offset, next_offset)
            } else if before != after {
                apply(mpv, &before, &after)
            } else {
                Ok(())
            };
            if let Err(error) = result {
                let reason = format!("owner {owner} cleanup: {}", error.message);
                self.uncertain.insert(stage.into(), reason.clone());
                self.confirmations
                    .insert(stage.into(), json!({"status":"uncertain","error":reason}));
                failures.push(format!("{stage}: {reason}"));
            }
        }
        if self.video.is_empty() {
            self.frames_in_memory(mpv, false);
        }
        if !failures.is_empty() {
            return Err(RpcError::new("backend-uncertain", failures.join("; ")));
        }
        Ok(())
    }
    /// The owned video nodes are FFmpeg software filters: they need decoded
    /// frames in memory. With zero-copy hardware decoding (GPU rendering's
    /// `hwdec=auto-safe`, i.e. VideoToolbox on macOS) mpv hands the graph GPU
    /// frames, the graph cannot be configured, and mpv drops the filter with
    /// only a log line ("Disabling filter vestra-owned because it has
    /// failed") while `vf add` has already succeeded. So while any owned video
    /// chain exists, decoding uses the copy-back variant of the configured
    /// mode (still hardware decoding), switched before the graph is
    /// installed; the configured mode comes back once no chain is left,
    /// unless someone else (the renderer's software fallback) changed `hwdec`
    /// in between.
    fn frames_in_memory(&mut self, mpv: &Mpv, needed: bool) {
        let current = mpv.get_property::<String>("hwdec").unwrap_or_default();
        if needed {
            if self.hwdec.is_some() {
                return;
            }
            let Some(copy) = copy_back(&current) else {
                return;
            };
            match mpv.set_property("hwdec", copy.as_str()) {
                Ok(()) => self.hwdec = Some((current, copy)),
                Err(error) => eprintln!("processing: hwdec={copy} refused: {error}"),
            }
        } else if let Some((original, copy)) = self.hwdec.take()
            && current == copy
            && let Err(error) = mpv.set_property("hwdec", original.as_str())
        {
            eprintln!("processing: restoring hwdec={original} failed: {error}");
        }
    }
}
/// What the current source is, for `processing.replace`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Media mpv decodes, ready.
    Decoded,
    /// A plugin's generated stream (`openStream`), ready.
    Generated,
    NotReady,
}
/// The copy-back variant of an mpv `hwdec` mode (a single mode or a
/// comma-separated list), or `None` when frames are already in memory.
fn copy_back(mode: &str) -> Option<String> {
    let items: Vec<String> = mode
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| match item {
            "no" => item.to_owned(),
            "auto-safe" => "auto-copy-safe".to_owned(),
            "auto" | "yes" | "auto-unsafe" => "auto-copy".to_owned(),
            _ if item.contains("copy") => item.to_owned(),
            _ => format!("{item}-copy"),
        })
        .collect();
    let copy = items.join(",");
    (!items.is_empty() && copy != mode && items.iter().any(|item| item != "no")).then_some(copy)
}
fn combined_offset(offsets: &BTreeMap<(u64, String), i32>) -> Result<i32, RpcError> {
    let positive: i64 = offsets
        .values()
        .filter(|v| **v > 0)
        .map(|v| i64::from(*v))
        .sum();
    let negative: i64 = offsets
        .values()
        .filter(|v| **v < 0)
        .map(|v| -i64::from(*v))
        .sum();
    if positive > 10_000 || negative > 10_000 {
        return Err(RpcError::invalid(
            "positive and negative audio offset contribution budgets must each be <=10000ms so owner removal remains representable",
        ));
    }
    Ok((positive - negative) as i32)
}
fn confirm_offset(mpv: &Mpv, offset: i32) -> Result<(), RpcError> {
    let actual = mpv
        .get_property::<f64>("audio-delay")
        .map_err(|e| RpcError::new("backend", e.to_string()))?;
    if !actual.is_finite() || (actual - f64::from(offset) / 1000.).abs() > 0.000001 {
        return Err(RpcError::new(
            "backend-uncertain",
            "actual audio-delay differs from owned offset",
        ));
    }
    Ok(())
}
fn set_offset(mpv: &Mpv, offset: i32) -> Result<(), RpcError> {
    mpv.set_property("audio-delay", f64::from(offset) / 1000.)
        .map_err(|e| RpcError::new("backend", e.to_string()))?;
    confirm_offset(mpv, offset)
}
fn apply_audio(
    mpv: &Mpv,
    before: &Descriptors,
    after: &Descriptors,
    old: i32,
    next: i32,
) -> Result<(), RpcError> {
    apply(mpv, before, after)?;
    if let Err(error) = set_offset(mpv, next) {
        let graph_restore = set_graph(mpv, before);
        let offset_restore = set_offset(mpv, old);
        if graph_restore.is_err() || offset_restore.is_err() {
            return Err(RpcError::new(
                "backend-uncertain",
                format!(
                    "audio update {}; rollback graph {:?}, offset {:?}",
                    error.message,
                    graph_restore.err().map(|e| e.message),
                    offset_restore.err().map(|e| e.message)
                ),
            ));
        }
        return Err(RpcError::new(
            "backend",
            format!(
                "{}; restored prior audio graph and offset (DSP history may reset)",
                error.message
            ),
        ));
    }
    Ok(())
}
fn check(resource: &str, revision: u64, expected: u64, current: u64) -> Result<(), RpcError> {
    if resource.is_empty() || revision == 0 {
        return Err(RpcError::invalid(
            "resourceId and positive revision required",
        ));
    }
    if expected != current {
        return Err(RpcError::new("stale", "source generation changed"));
    }
    Ok(())
}
fn set_graph(mpv: &Mpv, graph: &Descriptors) -> Result<(), RpcError> {
    let stage = if matches!(graph, Descriptors::Video(_)) {
        "vf"
    } else {
        "af"
    };
    let text = effects::render_lavfi(graph).map_err(|e| RpcError::invalid(e.to_string()))?;
    mpv.command(&[stage, "add", &format!("@vestra-owned:lavfi=[{text}]")])
        .map_err(|e| RpcError::new("backend", e.to_string()))
}
fn apply(mpv: &Mpv, before: &Descriptors, after: &Descriptors) -> Result<(), RpcError> {
    let plan =
        effects::runtime_commands(before, after).map_err(|e| RpcError::invalid(e.to_string()))?;
    let stage = if matches!(after, Descriptors::Video(_)) {
        "vf"
    } else {
        "af"
    };
    let next = effects::render_lavfi(after).map_err(|e| RpcError::invalid(e.to_string()))?;
    let old = effects::render_lavfi(before).map_err(|e| RpcError::invalid(e.to_string()))?;
    let set = |graph: &str| mpv.command(&[stage, "add", &format!("@vestra-owned:lavfi=[{graph}]")]);
    let result = match plan {
        UpdatePlan::NeedsReconfigure => set(&next),
        UpdatePlan::Commands(commands) => {
            let mut result = Ok(());
            for command in commands {
                if let Err(error) = mpv.command(&[
                    &format!("{stage}-command"),
                    "vestra-owned",
                    command.option,
                    &command.value,
                    &command.target,
                ]) {
                    result = Err(error);
                    break;
                }
            }
            if result.is_ok() && stage == "vf" && mpv.get_property::<bool>("pause").unwrap_or(false)
            {
                result = set(&next);
            }
            result
        }
    };
    if let Err(error) = result {
        return match set(&old) {
            Ok(()) => Err(RpcError::new(
                "backend",
                format!("{error}; restored prior graph"),
            )),
            Err(restore) => Err(RpcError::new(
                "backend-uncertain",
                format!("apply {error}; restoring prior graph failed: {restore}"),
            )),
        };
    }
    Ok(())
}

#[cfg(test)]
mod offset_tests {
    use super::*;
    #[test]
    fn any_owner_subset_remains_representable() {
        let mut offsets = BTreeMap::new();
        offsets.insert((1, "a".into()), 10_000);
        offsets.insert((2, "b".into()), -10_000);
        assert_eq!(combined_offset(&offsets).unwrap(), 0);
        offsets.insert((3, "c".into()), 10);
        assert!(combined_offset(&offsets).is_err());
        offsets.remove(&(3, "c".into()));
        offsets.remove(&(2, "b".into()));
        assert_eq!(combined_offset(&offsets).unwrap(), 10_000);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{IntentTicket, Phase, SourceStamp};

    const OWNER: u64 = 7;

    fn ticket(generation: u64) -> IntentTicket {
        IntentTicket {
            stamp: SourceStamp {
                generation,
                intent_version: 1,
            },
            cycle: 1,
            owner: 1,
        }
    }
    /// A processing book whose current source is ready (generation 1).
    fn ready() -> Processing {
        let mut processing = Processing::default();
        processing.register(OWNER, "examples.blur".into());
        processing.note_source(1, Phase::Ready);
        processing.load = Some((1, ticket(1)));
        processing
    }
    fn mpv(hwdec: &str) -> Mpv {
        let mpv = Mpv::builder()
            .unwrap()
            .set_property("vo", "null")
            .unwrap()
            .set_property("ao", "null")
            .unwrap()
            .set_property("vf", crate::source_geometry::INITIAL_FILTERS)
            .unwrap()
            .build()
            .unwrap();
        mpv.set_property("hwdec", hwdec).unwrap();
        mpv
    }
    fn video(revision: u64, nodes: Value) -> Value {
        json!({"stage":"video","resourceId":"blur","revision":revision,"expectedGeneration":1,"nodes":nodes})
    }
    fn gblur(sigma: f64) -> Value {
        json!([{"kind":"gblur","sigma":sigma,"steps":3}])
    }
    fn vf(mpv: &Mpv) -> String {
        mpv.get_property::<String>("vf").unwrap()
    }

    #[test]
    fn copy_back_modes_keep_hardware_decoding() {
        assert_eq!(copy_back("auto-safe").as_deref(), Some("auto-copy-safe"));
        assert_eq!(copy_back("auto").as_deref(), Some("auto-copy"));
        assert_eq!(
            copy_back("videotoolbox").as_deref(),
            Some("videotoolbox-copy")
        );
        assert_eq!(
            copy_back("videotoolbox,d3d11va").as_deref(),
            Some("videotoolbox-copy,d3d11va-copy")
        );
        for already in ["no", "", "auto-copy-safe", "auto-copy", "videotoolbox-copy"] {
            assert_eq!(copy_back(already), None, "{already}");
        }
    }

    /// The blur example's sequence: on, off (`[]`), on again. An empty chain
    /// removes the resource (it used to fail `invalid` for video, so "off"
    /// left the blur in place), and while a chain exists decoding copies
    /// frames back to memory so mpv's software filters can run.
    #[test]
    fn an_empty_video_chain_removes_the_resource_and_hwdec_follows_the_chain() {
        let mpv = mpv("auto-safe");
        let mut processing = ready();
        let source = Source::Decoded;
        processing
            .replace(&mpv, OWNER, 1, source, &video(1, gblur(4.0)))
            .unwrap();
        assert!(
            vf(&mpv).contains("gblur@n0=sigma=4:sigmaV=4:steps=3"),
            "{}",
            vf(&mpv)
        );
        assert_eq!(
            mpv.get_property::<String>("hwdec").unwrap(),
            "auto-copy-safe"
        );
        assert_eq!(processing.state()["video"].as_array().unwrap().len(), 1);

        processing
            .replace(&mpv, OWNER, 1, source, &video(3, json!([])))
            .unwrap();
        assert!(processing.state()["video"].as_array().unwrap().is_empty());
        assert!(!vf(&mpv).contains("gblur"), "{}", vf(&mpv));
        assert_eq!(mpv.get_property::<String>("hwdec").unwrap(), "auto-safe");
        // Removing what is not there is a no-op, not an error.
        processing
            .replace(&mpv, OWNER, 1, source, &video(4, json!([])))
            .unwrap();

        processing
            .replace(&mpv, OWNER, 1, source, &video(5, gblur(4.0)))
            .unwrap();
        assert!(vf(&mpv).contains("gblur@n0=sigma=4"), "{}", vf(&mpv));
        assert_eq!(
            mpv.get_property::<String>("hwdec").unwrap(),
            "auto-copy-safe"
        );
        // Stopping the owner removes its chain and restores the mode.
        processing.disable(&mpv, OWNER).unwrap();
        assert_eq!(mpv.get_property::<String>("hwdec").unwrap(), "auto-safe");
    }

    #[test]
    fn hwdec_changed_by_someone_else_is_left_alone() {
        let mpv = mpv("auto-safe");
        let mut processing = ready();
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(1, gblur(4.0)))
            .unwrap();
        // The renderer fell back to software rendering meanwhile.
        mpv.set_property("hwdec", "no").unwrap();
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(2, json!([])))
            .unwrap();
        assert_eq!(mpv.get_property::<String>("hwdec").unwrap(), "no");
        // Software decoding needs no switch at all.
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(3, gblur(4.0)))
            .unwrap();
        assert_eq!(mpv.get_property::<String>("hwdec").unwrap(), "no");
    }

    #[test]
    fn generated_streams_refuse_chains_but_accept_removal() {
        let mpv = mpv("no");
        let mut processing = ready();
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(1, gblur(4.0)))
            .unwrap();
        // openStream: a new source whose frames never reach mpv.
        processing.note_source(2, Phase::Ready);
        let on_stream = |revision, nodes| json!({"stage":"video","resourceId":"blur","revision":revision,"expectedGeneration":2,"nodes":nodes});
        let error = processing
            .replace(&mpv, OWNER, 2, Source::Generated, &on_stream(2, gblur(5.0)))
            .unwrap_err();
        assert_eq!(error.code, "unsupported");
        let audio = json!({"stage":"audio","resourceId":"tone","revision":1,"expectedGeneration":2,
            "nodes":[{"kind":"channels","mode":"mono"}]});
        let error = processing
            .replace(&mpv, OWNER, 2, Source::Generated, &audio)
            .unwrap_err();
        assert_eq!(error.code, "unsupported");
        // Turning the effect off works and reaches the next decoded media.
        processing
            .replace(&mpv, OWNER, 2, Source::Generated, &on_stream(3, json!([])))
            .unwrap();
        assert!(processing.state()["video"].as_array().unwrap().is_empty());
        assert!(!vf(&mpv).contains("gblur"));
        let error = processing
            .replace(&mpv, OWNER, 2, Source::NotReady, &on_stream(4, gblur(1.0)))
            .unwrap_err();
        assert_eq!(error.code, "unavailable");
    }

    /// The cases the SDK's TestHost must answer alike
    /// (tests/sdk/host-agreement.test.ts runs them against it).
    #[test]
    fn conformance_cases_shared_with_the_sdk_test_host() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/processing-conformance.json"
        ))
        .unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let mpv = mpv("no");
            let mut processing = ready();
            for (index, step) in case["steps"].as_array().unwrap().iter().enumerate() {
                let source = match step["source"].as_str().unwrap() {
                    "decoded" => Source::Decoded,
                    "generated" => Source::Generated,
                    "loading" => Source::NotReady,
                    other => panic!("{name}: source {other}"),
                };
                let stage = step["stage"].as_str().unwrap();
                let mut change = json!({"stage":stage,"resourceId":"r","revision":index + 1,
                    "expectedGeneration":1,"nodes":step["nodes"]});
                if let Some(offset) = step.get("offsetMs") {
                    change["offsetMs"] = offset.clone();
                }
                let outcome = match processing.replace(&mpv, OWNER, 1, source, &change) {
                    Ok(_) => "applied".to_owned(),
                    Err(error) => error.code,
                };
                assert_eq!(
                    outcome,
                    step["expect"].as_str().unwrap(),
                    "{name}, step {index}"
                );
                let held = processing.state()[stage].as_array().unwrap().len();
                assert_eq!(
                    held as u64,
                    step["resources"].as_u64().unwrap(),
                    "{name}, step {index}"
                );
            }
        }
    }

    /// Real decoding under GPU rendering, as the packaged player plays: a
    /// 1440x1080 H.264 file, VideoToolbox (`hwdec=auto-safe`), an owned
    /// gblur chain. Before the copy-back switch the frames reaching the
    /// renderer stayed `videotoolbox` and mpv logged "Disabling filter
    /// vestra-owned because it has failed": the blur was reported applied but
    /// never drawn. Needs a GPU and `ffmpeg` (to make the file), so it runs
    /// on request: `cargo test --locked -- --ignored gpu_decoding`.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "needs a GPU, VideoToolbox and ffmpeg"]
    fn gpu_decoding_runs_the_owned_chain() {
        use std::time::{Duration, Instant};
        let dir = std::env::temp_dir().join(format!("vestra-gblur-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("four3.mp4");
        let made = std::process::Command::new("ffmpeg")
            .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i"])
            .arg("testsrc2=size=1440x1080:rate=25")
            .args(["-t", "4", "-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .arg(&file)
            .status()
            .unwrap();
        assert!(made.success());
        let mpv = std::sync::Arc::new(mpv("no"));
        mpv.set_property("keep-open", true).unwrap();
        mpv.set_property("vo", "libmpv").unwrap();
        mpv.request_log_messages("v").unwrap();
        let publish = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let renderer = crate::platform::gpu_start(mpv.clone(), (720, 540), publish).unwrap();
        mpv.set_property("hwdec", crate::renderer::hwdec(None).as_str())
            .unwrap();
        mpv.command(&["loadfile", file.to_str().unwrap()]).unwrap();
        let failed = std::cell::Cell::new(false);
        let pump = |seconds: u64| {
            let end = Instant::now() + Duration::from_secs(seconds);
            while Instant::now() < end {
                while let Some(event) = mpv.poll_event() {
                    if let rsmpv::Event::LogMessage(message) = event
                        && message.text.contains("because it has failed")
                    {
                        failed.set(true);
                    }
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        pump(2);
        let format = || {
            mpv.get_property::<String>("video-out-params/pixelformat")
                .unwrap_or_default()
        };
        assert_eq!(
            format(),
            "videotoolbox",
            "zero-copy decoding before any chain"
        );
        let mut processing = ready();
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(1, gblur(4.0)))
            .unwrap();
        pump(2);
        assert!(!failed.get(), "mpv dropped the owned filter");
        assert_ne!(
            format(),
            "videotoolbox",
            "frames reach the filter in memory"
        );
        assert!(vf(&mpv).contains("gblur"));
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(2, json!([])))
            .unwrap();
        pump(2);
        assert_eq!(format(), "videotoolbox", "zero-copy again without a chain");
        drop(renderer);
        let _ = std::fs::remove_dir_all(dir);
    }
}
