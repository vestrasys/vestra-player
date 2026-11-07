//! Finite owner-aware runtime candidate. Not a general filter graph API.
use crate::playback::processing_live::{self as live, Due, Live, ViewFacts};
use crate::playback::processing_operations::{self as operations, Admission, Key, Stage};
use crate::{
    host::RpcError,
    playback::effects::{
        self, AudioNode, Descriptors, PlayerGraph, Resource, RuntimeCommand, UpdatePlan, VideoNode,
    },
};
use rsmpv::Mpv;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
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
/// A plugin whose video chain needs frames in memory
/// (`Processing::copy_back_owners`): `UpscalingReason.plugins` in the API.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainOwner {
    /// The manifest id.
    pub id: String,
    /// The manifest's name, the id without one.
    pub name: String,
}
#[derive(Default)]
pub struct Processing {
    config_revision: u64,
    manifests: BTreeMap<u64, String>,
    /// The manifests' display names (`register`).
    names: BTreeMap<u64, String>,
    operations: BTreeMap<u64, operations::Book>,
    uncertain: BTreeMap<String, String>,
    source_generation: u64,
    source_phase: String,
    load: Option<(i64, crate::playback::session::IntentTicket)>,
    confirmations: BTreeMap<String, Value>,
    video: BTreeMap<(u64, String), (u64, Resource<VideoNode>)>,
    audio: BTreeMap<(u64, String), (u64, Resource<AudioNode>)>,
    audio_offsets: BTreeMap<(u64, String), i32>,
    /// `(original, copy)`: the `hwdec` mode before the owned video chain
    /// needed frames in memory, and the copy-back mode set for it.
    hwdec: Option<(String, String)>,
    /// The last `hwdec` switch `frames_in_memory` made, watched for mpv's
    /// software fallback (`hwdec_check`).
    hwdec_watch: Option<HwdecWatch>,
    /// Whether the host set mpv's global `deinterlace` to `auto`, because
    /// some owned video chain has a `deinterlace` `auto` node (`no` otherwise).
    /// The host is the option's only writer.
    deinterlace: bool,
    /// What stands in mpv's owned video graph (`effects::PlayerGraph`):
    /// the plugins' chains as slots, pass-through slots a drag left, the
    /// processing size. `video` is what the plugins asked for.
    installed: PlayerGraph,
    /// The live-change clock: paused preview, settling, teardown, size.
    live: Live,
    /// mpv's `framedrop` before an owned graph turned on decoder dropping.
    framedrop: Option<String>,
    /// When the processing size was last looked at (`tick`).
    sized: Option<Instant>,
    /// The user asked to play during a preview step (`pause_request`):
    /// applied once the step is over.
    resume: bool,
    /// Retries made, for the hardware tests.
    #[cfg(test)]
    hwdec_retries: u32,
}
impl Processing {
    /// `name`: the manifest's display name, for messages that name the
    /// plugin (`copy_back_owners`); the id stands in without one.
    pub fn register(&mut self, owner: u64, id: String, name: Option<String>) {
        self.manifests.insert(owner, id);
        match name.filter(|name| !name.trim().is_empty()) {
            Some(name) => self.names.insert(owner, name),
            None => self.names.remove(&owner),
        };
        self.operations.entry(owner).or_default();
    }
    /// The plugins whose video chain made the host switch decoding to
    /// copy-back (`frames_in_memory`): manifest id and name (the id without
    /// one), each once, in the order the plugins started (owner generation,
    /// the video map's key order). Empty when the host made no such switch: the
    /// decoding mode is then someone else's (software rendering's own
    /// copy-back mode, the user's).
    pub fn copy_back_owners(&self) -> Vec<ChainOwner> {
        if self.hwdec.is_none() {
            return Vec::new();
        }
        // The chains that act; while only pass-through slots are left (a
        // drag through 0, `TEARDOWN`), the chains that are there at all.
        let acting = |neutral_too: bool| -> Vec<u64> {
            self.video
                .iter()
                .filter(|(_, (_, resource))| {
                    if neutral_too {
                        effects::needs_frames_in_memory(std::iter::once(resource))
                    } else {
                        resource
                            .nodes
                            .iter()
                            .any(|n| !n.neutral() && n.in_player_graph())
                    }
                })
                .map(|((owner, _), _)| *owner)
                .collect()
        };
        let mut owners = acting(false);
        if owners.is_empty() {
            owners = acting(true);
        }
        owners.dedup();
        owners
            .into_iter()
            .map(|owner| {
                let id = self
                    .manifests
                    .get(&owner)
                    .cloned()
                    .unwrap_or_else(|| format!("plugin {owner}"));
                let name = self
                    .names
                    .get(&owner)
                    .cloned()
                    .unwrap_or_else(|| id.clone());
                ChainOwner { id, name }
            })
            .collect()
    }
    pub fn state(&self) -> Value {
        json!({"configurationRevision":self.config_revision,"audioOffsetMs":self.audio_offsets.values().sum::<i32>(),"source":{"generation":self.source_generation,"phase":self.source_phase,"load":self.load.map(|(entry,t)|json!({"entry":entry,"generation":t.stamp.generation,"cycle":t.cycle})),"stages":self.confirmations},"backendUncertain":self.uncertain,"videoScale":{"factor":self.installed.factor,"budgetSteps":self.live.budget.steps},"video":self.video.iter().map(|((owner,_),(revision,resource))|json!({"owner":owner,"revision":revision,"resource":resource})).collect::<Vec<_>>(),"audio":self.audio.iter().map(|((owner,_),(revision,resource))|json!({"owner":owner,"revision":revision,"resource":resource,"offsetMs":self.audio_offsets.get(&(*owner,resource.resource_id.clone())).copied().unwrap_or(0)})).collect::<Vec<_>>()})
    }
    pub fn freeze(
        &self,
        expected: u64,
        load: (i64, crate::playback::session::IntentTicket),
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
    pub fn note_source(&mut self, generation: u64, phase: crate::playback::session::Phase) {
        let name = match phase {
            crate::playback::session::Phase::Empty => "empty",
            crate::playback::session::Phase::Loading => "loading",
            crate::playback::session::Phase::Ready => "ready",
            crate::playback::session::Phase::Ended => "ended",
            crate::playback::session::Phase::Error => "error",
        };
        if generation != self.source_generation
            || (name == "loading" && self.source_phase != "loading")
            || matches!(
                phase,
                crate::playback::session::Phase::Empty | crate::playback::session::Phase::Error
            )
        {
            self.load = None;
            // Another picture: no preview position, full budget.
            self.live.forget_position();
            self.live.stepping = None;
            self.resume = false;
            if generation != self.source_generation {
                self.live.budget.reset();
            }
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
        ticket: crate::playback::session::IntentTicket,
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
            } else if empty && stage == "video" && !self.installed.is_empty() {
                // Pass-through slots left from the previous file: gone
                // before this one decodes (no decoder switch mid-file).
                self.install(mpv, PlayerGraph::default())
            } else if empty {
                Ok(())
            } else {
                // mpv's `deinterlace` is a global option and outlives the
                // file; only the FFmpeg graph is replayed, at the processing
                // size for this file (its container's size until it decodes).
                match &graph {
                    Descriptors::Video(resources) => {
                        self.live.forget_position();
                        PlayerGraph::build(resources, self.factor_for(mpv, resources))
                            .map_err(|e| RpcError::invalid(e.to_string()))
                            .and_then(|graph| self.install(mpv, graph))
                    }
                    Descriptors::Audio(_) => set_graph(mpv, &graph),
                }
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
                let after: Vec<Resource<VideoNode>> =
                    next.values().map(|(_, r)| r.clone()).collect();
                let wanted = effects::player_deinterlace(next.values().map(|(_, r)| r));
                // A generated stream never reaches mpv's graph: the change is
                // for the next decoded media, rebuilt at once (no pass-through).
                let applied = self.deinterlace_then(mpv, wanted, |this| {
                    if source == Source::Generated {
                        PlayerGraph::build(&after, 1.)
                            .map_err(|e| RpcError::invalid(e.to_string()))
                            .and_then(|graph| this.install(mpv, graph))
                    } else {
                        this.apply_video(mpv, &after)
                    }
                });
                if let Err(error) = applied {
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
    /// `owner` has a chain applied (lazy activation: it stays running).
    pub fn holds(&self, owner: u64) -> bool {
        self.video
            .keys()
            .chain(self.audio.keys())
            .chain(self.audio_offsets.keys())
            .any(|(id, _)| *id == owner)
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
        self.names.remove(&owner);
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
            // The player's deinterlacer stays only while another owner's
            // chain still asks for it. Switched before the graph, as in
            // `replace`: a paused picture is redrawn by the graph change.
            // Ownership is revoked whatever happens, so it is not put back.
            let deinterlace = if stage == "video" {
                let wanted = effects::player_deinterlace(self.video.values().map(|(_, r)| r));
                self.set_deinterlace(mpv, wanted)
            } else {
                Ok(())
            };
            let result = if stage == "video" {
                // Rebuilt at once, without pass-through slots: the
                // chain is no longer this owner's.
                if before != after {
                    self.rebuild_video(mpv)
                } else {
                    Ok(())
                }
            } else if before != after || old_offset != next_offset {
                apply_audio(mpv, &before, &after, old_offset, next_offset)
            } else {
                Ok(())
            }
            .and(deinterlace);
            if let Err(error) = result {
                let reason = format!("owner {owner} cleanup: {}", error.message);
                self.uncertain.insert(stage.into(), reason.clone());
                self.confirmations
                    .insert(stage.into(), json!({"status":"uncertain","error":reason}));
                failures.push(format!("{stage}: {reason}"));
            }
        }
        if self.installed.is_empty() {
            self.frames_in_memory(mpv, false);
            self.decoder_drops(mpv, false);
        }
        if !failures.is_empty() {
            return Err(RpcError::new("backend-uncertain", failures.join("; ")));
        }
        Ok(())
    }
    /// The renderer switched (GPU to software) and set its own `hwdec`:
    /// the mode saved before the owned chain is no longer the one to come
    /// back to, so it is forgotten.
    pub fn renderer_changed(&mut self) {
        self.hwdec = None;
        self.hwdec_watch = None;
        // Another renderer: a play asked for during a step is not carried
        // over (the renderer switch decides what plays).
        self.resume = false;
    }
    /// mpv's own deinterlacer (`deinterlace=auto`) for the `deinterlace`
    /// `auto` nodes, one switch shared by every owner: on while any owned
    /// chain has such a node, off (`no`) when none is left. mpv decides per
    /// frame from the decoder's interlaced flag and runs it before the `vf`
    /// chain: the decoder's hardware deinterlacer on GPU frames (`d3d11vpp`
    /// for zero-copy `d3d11va`), otherwise `bwdif` on frames in memory
    /// (downloaded first when they are on the GPU, as VideoToolbox frames
    /// are), one frame per field. It needs no copy-back decoding, so it does
    /// not count for `frames_in_memory`.
    fn set_deinterlace(&mut self, mpv: &Mpv, wanted: bool) -> Result<(), RpcError> {
        if wanted == self.deinterlace {
            return Ok(());
        }
        let value = if wanted { "auto" } else { "no" };
        mpv.set_property("deinterlace", value)
            .map_err(|e| RpcError::new("backend", format!("deinterlace={value}: {e}")))?;
        self.deinterlace = wanted;
        Ok(())
    }
    /// `set_deinterlace`, then the graph change `graph`: switched first so
    /// the redraw of a paused picture that the graph change makes already
    /// has the new deinterlacer. A graph change that fails puts the
    /// deinterlacer back; if that fails too the stage is uncertain.
    fn deinterlace_then(
        &mut self,
        mpv: &Mpv,
        wanted: bool,
        graph: impl FnOnce(&mut Self) -> Result<(), RpcError>,
    ) -> Result<(), RpcError> {
        let previous = self.deinterlace;
        self.set_deinterlace(mpv, wanted)?;
        let result = graph(self);
        if let Err(error) = &result
            && let Err(restore) = self.set_deinterlace(mpv, previous)
        {
            return Err(RpcError::new(
                "backend-uncertain",
                format!(
                    "{}; restoring deinterlace failed: {}",
                    error.message, restore.message
                ),
            ));
        }
        result
    }
    /// The video chains `after` into mpv's graph: live when the installed
    /// graph can reach them (`PlayerGraph::plan`: values and pass-through
    /// switches as `vf-command`s), otherwise a rebuild (`install`), which
    /// while paused waits for the changes to settle when one was made
    /// within `SETTLE` (`Live::defer`). Live updates while paused are shown
    /// by the preview (`tick`).
    fn apply_video(&mut self, mpv: &Mpv, after: &[Resource<VideoNode>]) -> Result<(), RpcError> {
        let now = Instant::now();
        // A preview step has mpv unpaused for a moment: still paused.
        let paused = paused(mpv) || self.live.stepping.is_some();
        let invalid = |e: effects::ValidationError| RpcError::invalid(e.to_string());
        if let Some((graph, commands)) = self.installed.plan(after).map_err(invalid)? {
            if let Err(error) = send(mpv, &commands) {
                // No filter instance to take them (no video decoded yet,
                // audio only): the graph text is all there is to change.
                diag!("processing: live update refused ({error}); rebuilding the graph");
                return self.install(mpv, graph);
            }
            self.installed = graph;
            if commands.is_empty() {
                // Reached without commands: nothing waits any more.
                self.live.pending = false;
            } else {
                self.live.updated(now, paused);
            }
            return Ok(());
        }
        let graph = PlayerGraph::build(after, self.factor_for(mpv, after)).map_err(invalid)?;
        if paused && self.live.defer(now) {
            return Ok(());
        }
        self.install(mpv, graph)
    }
    /// Rebuild mpv's owned graph as `graph` (`vf add` with the label, in
    /// place). A graph with filters needs frames in memory and decoder frame
    /// dropping, switched before; an empty one (`null`) switches both back
    /// after. A failure puts the previous graph back.
    fn install(&mut self, mpv: &Mpv, graph: PlayerGraph) -> Result<(), RpcError> {
        let filters = !graph.is_empty();
        if filters {
            self.frames_in_memory(mpv, true);
            self.decoder_drops(mpv, true);
        }
        let text = graph.render();
        let result = mpv.command(&["vf", "add", &format!("@vestra-owned:lavfi=[{text}]")]);
        self.live.rebuilt(Instant::now());
        if let Err(error) = result {
            let old = self.installed.render();
            let restored = mpv.command(&["vf", "add", &format!("@vestra-owned:lavfi=[{old}]")]);
            let keep = !self.installed.is_empty();
            self.frames_in_memory(mpv, keep);
            self.decoder_drops(mpv, keep);
            return Err(match restored {
                Ok(()) => RpcError::new("backend", format!("{error}; restored prior graph")),
                Err(restore) => RpcError::new(
                    "backend-uncertain",
                    format!("apply {error}; restoring prior graph failed: {restore}"),
                ),
            });
        }
        self.installed = graph;
        if !filters {
            self.frames_in_memory(mpv, false);
            self.decoder_drops(mpv, false);
        }
        Ok(())
    }
    /// Rebuild from the chains as they are (no pass-through slots left).
    fn rebuild_video(&mut self, mpv: &Mpv) -> Result<(), RpcError> {
        let resources = self.video_resources();
        let graph = PlayerGraph::build(&resources, self.factor_for(mpv, &resources))
            .map_err(|e| RpcError::invalid(e.to_string()))?;
        self.install(mpv, graph)
    }
    fn video_resources(&self) -> Vec<Resource<VideoNode>> {
        self.video.values().map(|(_, r)| r.clone()).collect()
    }
    /// mpv's `framedrop` while an owned graph is in place: `decoder+vo`,
    /// so a chain that cannot keep up makes the decoder skip frames before
    /// they are filtered, instead of filtering every frame and dropping
    /// most at the output (4K60 with a 20 ms chain: 8.6 fps shown before,
    /// 53 after). mpv drops in the decoder only while the video is late
    /// against the audio (`check_framedrop`), so a chain that keeps up
    /// plays every frame. Without an owned graph the option is mpv's
    /// default again (`vo`), as it was, unless someone else changed it.
    fn decoder_drops(&mut self, mpv: &Mpv, on: bool) {
        if on {
            if self.framedrop.is_some() {
                return;
            }
            let current = mpv
                .get_property::<String>("framedrop")
                .unwrap_or_else(|_| "vo".into());
            if current.contains("decoder") {
                return;
            }
            match mpv.set_property("framedrop", FRAMEDROP) {
                Ok(()) => self.framedrop = Some(current),
                Err(error) => diag!("processing: framedrop={FRAMEDROP} refused: {error}"),
            }
        } else if let Some(original) = self.framedrop.take()
            && mpv.get_property::<String>("framedrop").ok().as_deref() == Some(FRAMEDROP)
            && let Err(error) = mpv.set_property("framedrop", original.as_str())
        {
            diag!("processing: restoring framedrop={original} failed: {error}");
        }
    }
    /// The processing factor for `resources` now (`processing_live::factor`):
    /// 1 while the view or the source is unknown.
    fn factor_for(&self, mpv: &Mpv, resources: &[Resource<VideoNode>]) -> f64 {
        let (Some(view), Some(source)) = (self.live.view, source_display(mpv)) else {
            return 1.;
        };
        let output =
            effects::chain_display_size(resources, source.display).unwrap_or(source.display);
        live::factor(output, &view, self.live.budget.steps, source.anamorphic)
    }
    /// A paused preview step is in flight: mpv's `pause` reads `no` until
    /// it is over, which is not the user's (`StatePush` waits).
    pub fn stepping(&self) -> bool {
        self.live.stepping.is_some()
    }
    /// A pause control (`source.submit` `pause`) reached the worker. During
    /// a preview step mpv is unpaused already, so "play" changes nothing and
    /// the step's end would pause again: it is remembered and applied when
    /// the step is over (`tick`). "Pause" needs nothing (mpv pauses, which
    /// ends the step) and drops a remembered "play".
    pub fn pause_request(&mut self, paused: bool) {
        if self.live.stepping.is_some() {
            self.resume = !paused;
        }
    }
    /// How long the control worker may wait before `tick` has something to
    /// do (`None`: only a command or an event).
    pub fn due_in(&self, mpv: &Mpv, now: Instant) -> Option<Duration> {
        let installed = !self.installed.is_empty();
        if let Some(since) = self.live.stepping {
            // Polled until it is over (mpv's pause event also wakes).
            return Some(
                Duration::from_millis(10)
                    .min((since + live::STEP_TIMEOUT).saturating_duration_since(now)),
            );
        }
        // An uncertain video stage takes no more changes: nothing to wait for.
        if self.uncertain.contains_key("video")
            || (!installed && !self.live.pending && self.live.anchor.is_none())
        {
            return None;
        }
        let due = self
            .live
            .due_in(now, paused(mpv), installed, self.installed.acts());
        // The processing size is looked at every `SIZE_LOOK` while a graph acts.
        if installed && self.installed.acts() {
            return Some(due.map_or(SIZE_LOOK, |due| due.min(SIZE_LOOK)));
        }
        due
    }
    /// Each turn of the control worker (`processing_live`): the paused
    /// preview, settling, the pass-through teardown, the processing size
    /// and its budget fallback. `view`: the video area and display layer,
    /// `None` when unknown (the graph then runs at full size).
    pub fn tick(&mut self, mpv: &Mpv, now: Instant, view: Option<ViewFacts>) {
        self.live.view = view;
        // A step in flight ends whatever else holds (an uncertain backend
        // too): until then the state pushes wait (`stepping`).
        if self.live.stepping.is_some() {
            if !self.live.step_over(paused(mpv), now) {
                return;
            }
            // Where the step left the picture: a seek elsewhere since then
            // is someone else's, and the preview's way back no longer holds.
            self.live.stepped_to = mpv.get_property::<f64>("time-pos").ok();
            if std::mem::take(&mut self.resume) {
                // Play pressed during the step: mpv was already unpaused,
                // so it took no effect, and the step paused again.
                self.live.forget_position();
                if let Err(error) = mpv.set_property("pause", false) {
                    diag!("processing: resuming after a preview step failed: {error}");
                }
            }
        }
        let installed = !self.installed.is_empty();
        if self.uncertain.contains_key("video")
            || (!installed && !self.live.pending && self.live.anchor.is_none())
        {
            return;
        }
        let paused = paused(mpv);
        if !paused {
            // Playing: the picture moves on, nothing to step or go back to.
            self.live.forget_position();
        }
        match self.live.due(now, paused, installed, self.installed.acts()) {
            Due::Nothing => {}
            Due::Step => self.preview_step(mpv, now),
            Due::Settle { rebuild } => self.settle(mpv, rebuild, paused),
            Due::Teardown => {
                let result = self.install(mpv, PlayerGraph::default());
                self.tick_failed(result);
            }
        }
        if self.installed.acts()
            && self.live.stepping.is_none()
            && self
                .sized
                .is_none_or(|at| now.duration_since(at) >= SIZE_LOOK)
        {
            self.sized = Some(now);
            self.follow_size(mpv, now, paused);
        } else if !self.installed.acts() {
            self.live.budget.pause();
        }
    }
    /// Show the live updates while paused: two frames forward (the first
    /// was filtered before them; three on the first step after pausing),
    /// muted; the first step remembers where the picture was. At the end of the file mpv cannot step: the settle's
    /// rebuild redraws the picture instead.
    fn preview_step(&mut self, mpv: &Mpv, now: Instant) {
        if mpv.get_property::<bool>("eof-reached").unwrap_or(false) {
            self.live.unseen = false;
            return;
        }
        // Moved since the last step (the user seeked or stepped during the
        // drag): the preview starts over from here, and the settle comes
        // back here, not to where the drag began.
        if let (Some(_), Some(stepped_to), Some(here)) = (
            self.live.anchor,
            self.live.stepped_to,
            mpv.get_property::<f64>("time-pos").ok(),
        ) && (here - stepped_to).abs() >= PREVIEW_MOVED
        {
            self.live.anchor = None;
        }
        let frames = if self.live.anchor.is_none() {
            self.live.anchor = mpv.get_property::<f64>("time-pos").ok();
            self.live.stepped_to = self.live.anchor;
            PREVIEW_FIRST_FRAMES
        } else {
            PREVIEW_FRAMES
        };
        match mpv.command(&["frame-step", frames, "mute"]) {
            Ok(()) => self.live.step(now),
            Err(error) => {
                diag!("processing: preview frame-step refused: {error}");
                self.live.unseen = false;
            }
        }
    }
    /// The changes settled: paused after preview steps, back to where the
    /// picture was (an exact seek) unless someone moved it meanwhile; then
    /// the rebuild (a deferred one from the chains, or the installed graph
    /// with its live values written into its text). In that order: mpv
    /// makes no refresh seek of its own for a graph change while a seek is
    /// queued (`issue_refresh_seek`), so the picture goes back once; the
    /// other way round the refresh seek would take it to where it is.
    fn settle(&mut self, mpv: &Mpv, rebuild: bool, paused: bool) {
        let anchor = self.live.anchor;
        let stepped_to = self.live.stepped_to;
        self.live.forget_position();
        let here = mpv.get_property::<f64>("time-pos").ok();
        if paused
            && let (Some(anchor), Some(here), Some(stepped_to)) = (anchor, here, stepped_to)
            && (here - stepped_to).abs() < PREVIEW_MOVED
            && (here - anchor).abs() >= 0.0005
            && let Err(error) = mpv.command(&["seek", &anchor.to_string(), "absolute+exact"])
        {
            diag!("processing: preview seek back to {anchor} refused: {error}");
        }
        if rebuild {
            let result = if self.live.pending {
                self.rebuild_video(mpv)
            } else {
                self.install(mpv, self.installed.clone())
            };
            self.tick_failed(result);
        }
    }
    /// The processing size follows the view (debounced, `Retarget`) and the
    /// budget fallback (at once).
    fn follow_size(&mut self, mpv: &Mpv, now: Instant, paused: bool) {
        let (Some(view), Some(source)) = (self.live.view, source_display(mpv)) else {
            return;
        };
        let mut stepped = false;
        if paused {
            self.live.budget.pause();
        } else {
            let number = |name: &str| mpv.get_property::<f64>(name).ok();
            if let (Some(position), Some(fps)) = (
                number("time-pos"),
                number("container-fps").or_else(|| number("estimated-vf-fps")),
            ) {
                let dropped = number("decoder-frame-drop-count").unwrap_or(0.);
                let moved = self.live.budget.sample(now, dropped, position, fps);
                stepped = moved != live::BudgetMove::Stay;
                if stepped {
                    diag!(
                        "processing: the video chain {}; processing size one step {} ({} below the shown size)",
                        if moved == live::BudgetMove::Down {
                            "drops frames"
                        } else {
                            "keeps up"
                        },
                        if moved == live::BudgetMove::Down {
                            "smaller"
                        } else {
                            "larger"
                        },
                        self.live.budget.steps
                    );
                }
            }
        }
        let (width, height) = source.display;
        let inputs = live::Inputs {
            view,
            source: (width.round() as u32, height.round() as u32),
        };
        let settled = self.live.retarget.settled(inputs, now);
        let wanted = live::factor(
            self.installed.display_size(source.display),
            &view,
            self.live.budget.steps,
            source.anamorphic,
        );
        let busy = self.live.pending || (paused && self.live.anchor.is_some());
        if wanted != self.installed.factor && (settled || stepped) && !busy {
            let result = self.install(mpv, self.installed.at(wanted));
            self.tick_failed(result);
        }
    }
    /// A rebuild `tick` made failed: logged; an uncertain backend blocks
    /// further video updates as any other. `true` when it failed.
    fn tick_failed(&mut self, result: Result<(), RpcError>) -> bool {
        let Err(error) = result else {
            return false;
        };
        diag!("processing: video graph update failed: {}", error.message);
        if error.code == "backend-uncertain" {
            self.uncertain.insert("video".into(), error.message.clone());
            self.confirmations.insert(
                "video".into(),
                json!({"status":"uncertain","error":error.message}),
            );
        }
        true
    }
    /// The owned video nodes are FFmpeg software filters: they need decoded
    /// frames in memory. With zero-copy hardware decoding (GPU rendering's
    /// `hwdec=auto-safe`, i.e. VideoToolbox on macOS) mpv hands the graph GPU
    /// frames, the graph cannot be configured, and mpv drops the filter with
    /// only a log line ("Disabling filter vestra-owned because it has
    /// failed") while `vf add` has already succeeded. So while any owned video
    /// chain has such a node (the `deinterlace` `auto` node is not one: it is
    /// mpv's own deinterlacer, `set_deinterlace`), decoding uses the copy-back
    /// variant of the configured mode (still hardware decoding), switched
    /// before the graph is installed; the configured mode comes back once no
    /// such node is left, unless someone else changed `hwdec` in between or
    /// the renderer switched (`renderer_changed`). Software rendering already
    /// uses a copy-back mode (`renderer::hwdec`), so there nothing switches.
    ///
    /// Each switch reopens the decoder mid-stream; if that makes mpv give up
    /// on hardware decoding for the rest of the file, `hwdec_check` asks
    /// for it once more (`HwdecWatch`).
    fn frames_in_memory(&mut self, mpv: &Mpv, needed: bool) {
        let current = mpv.get_property::<String>("hwdec").unwrap_or_default();
        if needed {
            if self.hwdec.is_some() {
                return;
            }
            let Some(copy) = copy_back(&current) else {
                return;
            };
            let watch = HwdecWatch::before_switch(mpv, &copy);
            match mpv.set_property("hwdec", copy.as_str()) {
                Ok(()) => {
                    diag!(
                        "processing: hwdec {current} -> {copy} (a video chain needs frames in memory)"
                    );
                    self.hwdec = Some((current, copy));
                    self.hwdec_watch = watch;
                }
                Err(error) => diag!("processing: hwdec={copy} refused: {error}"),
            }
        } else if let Some((original, copy)) = self.hwdec.take()
            && current == copy
        {
            let watch = HwdecWatch::before_switch(mpv, &original);
            match mpv.set_property("hwdec", original.as_str()) {
                Ok(()) => {
                    diag!(
                        "processing: hwdec {copy} -> {original} (no video chain needs frames in memory)"
                    );
                    self.hwdec_watch = watch;
                }
                Err(error) => {
                    diag!("processing: restoring hwdec={original} failed: {error}");
                }
            }
        }
    }
    /// Follows mpv after a switch `frames_in_memory` made, with every mpv
    /// event (the backend observes `hwdec-current`, `observe_hwdec`) and
    /// with the backend's periodic state push while a watch is open, so the
    /// window also closes without an event. Reads `hwdec-current` itself:
    /// an observed value can be one from the middle of the decoder's
    /// reinitialization.
    ///
    /// A runtime switch reopens the decoder and seeks back to where it was
    /// from the last keyframe (`Seek`, then `PlaybackRestart`). When
    /// hardware decoding was active before the switch and mpv has fallen
    /// back to software by the time that restart is done, or within
    /// `HWDEC_WATCH_SECONDS` of playback after the switch, the switch broke
    /// it (`HWDEC_SOFTWARE_FALLBACK`), not the file: the intended mode is
    /// applied once more (copy-back while a chain exists, otherwise the
    /// configured mode: the mode the switch set), by way of `no` because
    /// mpv ignores setting an option to the value it has; that reopens the
    /// decoder with hardware decoding. Not before the restart: reopening the
    /// decoder again while mpv is still replaying packets for the switch
    /// ends the video ("video EOF reached"). One retry per switch: falling
    /// back again leaves this file on software decoding. Outside the window
    /// nothing is retried; mpv's own fallback stands. mpv tries hardware
    /// decoding again at the next file's decoder initialization anyway.
    pub fn hwdec_event(&mut self, mpv: &Mpv, event: &rsmpv::Event) {
        let Some(watch) = self.hwdec_watch.as_mut() else {
            return;
        };
        match event {
            // Another file is another decoder. (Not `Seek`: the switch
            // itself seeks; a user's seek away moves `time-pos` out of the
            // window.)
            rsmpv::Event::StartFile { .. } | rsmpv::Event::EndFile { .. } => {
                if watch.retried {
                    diag!(
                        "processing: hwdec retry outcome unknown (another file first; hwdec-current={})",
                        hwdec_current(mpv).as_deref().unwrap_or("unavailable"),
                    );
                }
                self.hwdec_watch = None;
            }
            rsmpv::Event::PlaybackRestart => {
                watch.restarted = true;
                self.hwdec_check(mpv);
            }
            _ => self.hwdec_check(mpv),
        }
    }
    /// See `hwdec_event`.
    pub fn hwdec_check(&mut self, mpv: &Mpv) {
        let Some(watch) = self.hwdec_watch.as_mut() else {
            return;
        };
        let position = mpv.get_property::<f64>("time-pos").ok();
        let current = hwdec_current(mpv);
        let current = current.as_deref();
        match watch.step(position, current) {
            HwdecCheck::Watch => {}
            HwdecCheck::Done => self.hwdec_watch = None,
            HwdecCheck::Retry => {
                let mode = watch.mode.clone();
                // Someone else changed `hwdec` meanwhile: theirs stands.
                if mpv.get_property::<String>("hwdec").ok().as_deref() != Some(mode.as_str()) {
                    self.hwdec_watch = None;
                    return;
                }
                diag!(
                    "processing: hwdec fell back to software after switching to {mode}; requesting hardware decoding once more"
                );
                // mpv ignores setting an option to the value it has: going
                // through `no` (the decoder already decodes in software)
                // makes the intended mode reopen the decoder.
                let applied = mpv
                    .set_property("hwdec", "no")
                    .and_then(|()| mpv.set_property("hwdec", mode.as_str()));
                match applied {
                    Ok(()) => {
                        #[cfg(test)]
                        {
                            self.hwdec_retries += 1;
                        }
                        watch.from = position.unwrap_or(watch.from);
                        watch.retried = true;
                        watch.restarted = false;
                    }
                    Err(error) => {
                        diag!("processing: hwdec retry ({mode}) refused: {error}");
                        self.hwdec_watch = None;
                    }
                }
            }
            HwdecCheck::Restored => {
                diag!(
                    "processing: hwdec retry restored hardware decoding ({})",
                    current.unwrap_or_default()
                );
                self.hwdec_watch = None;
            }
            HwdecCheck::StaysSoftware => {
                diag!(
                    "processing: hwdec retry did not restore hardware decoding (hwdec-current={}); this file stays on software decoding",
                    current.unwrap_or("unavailable")
                );
                self.hwdec_watch = None;
            }
        }
    }
    /// Whether a switch is being watched (the backend then checks with
    /// every state push).
    pub fn hwdec_watching(&self) -> bool {
        self.hwdec_watch.is_some()
    }
}
/// Notifies `hwdec-current` changes as `Event::PropertyChange` with this
/// userdata (`Processing::hwdec_event`).
pub const HWDEC_OBSERVATION: u64 = 1;
pub fn observe_hwdec(mpv: &Mpv) -> rsmpv::Result<()> {
    mpv.observe_property(HWDEC_OBSERVATION, "hwdec-current", rsmpv::Format::None)
}
fn hwdec_current(mpv: &Mpv) -> Option<String> {
    mpv.get_property::<String>("hwdec-current").ok()
}
/// How much playback (`time-pos`, seconds) after a switch still counts as
/// the switch's: mpv falls back once `HWDEC_SOFTWARE_FALLBACK` frames in a
/// row failed, usually while it seeks back for the switch (frames do not
/// advance meanwhile), at the latest a second or two later at common frame
/// rates. Pausing does not use up the window (no frames are decoded);
/// seeking away or another file ends it.
pub(crate) const HWDEC_WATCH_SECONDS: f64 = 3.;
/// A `hwdec` switch `frames_in_memory` made, watched for mpv's software
/// fallback (`Processing::hwdec_event`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HwdecWatch {
    /// The intended mode: the one the switch set, applied again to retry.
    pub mode: String,
    /// `time-pos` at the switch (at the retry, once retried).
    pub from: f64,
    /// `hwdec-current` was a hardware decoder right before the switch.
    pub hardware_before: bool,
    /// The one retry was made.
    pub retried: bool,
    /// mpv's `PlaybackRestart` since the switch (or the retry): the
    /// decoder is reopened and the seek back is done.
    pub restarted: bool,
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum HwdecCheck {
    /// Keep watching.
    Watch,
    /// Stop watching, nothing to do or report.
    Done,
    /// Fell back to software: apply the intended mode again (once).
    Retry,
    /// After the retry: hardware decoding again.
    Restored,
    /// After the retry: software decoding again (or still); stop.
    StaysSoftware,
}
impl HwdecWatch {
    /// Read right before a switch; `None` without a position (nothing is
    /// playing, so no frames can fail).
    fn before_switch(mpv: &Mpv, mode: &str) -> Option<Self> {
        let from = mpv.get_property::<f64>("time-pos").ok()?;
        Some(Self {
            mode: mode.into(),
            from,
            hardware_before: hwdec_current(mpv).is_some_and(|current| is_hardware(&current)),
            retried: false,
            restarted: false,
        })
    }
    /// The decision, from `time-pos` and `hwdec-current` now (`None`:
    /// unavailable, as while the decoder reopens).
    pub(crate) fn step(&self, position: Option<f64>, current: Option<&str>) -> HwdecCheck {
        let within = position
            .is_some_and(|position| (0. ..=HWDEC_WATCH_SECONDS).contains(&(position - self.from)));
        let hardware = current.is_some_and(is_hardware);
        let software = current == Some("no");
        if !self.hardware_before {
            HwdecCheck::Done
        } else if !self.retried {
            match () {
                _ if !within => HwdecCheck::Done,
                _ if software && self.restarted => HwdecCheck::Retry,
                _ => HwdecCheck::Watch,
            }
        } else {
            match () {
                _ if !within && hardware => HwdecCheck::Restored,
                _ if !within => HwdecCheck::StaysSoftware,
                // Before the restart the value can still be the old one.
                _ if !self.restarted => HwdecCheck::Watch,
                _ if hardware => HwdecCheck::Restored,
                _ if software => HwdecCheck::StaysSoftware,
                _ => HwdecCheck::Watch,
            }
        }
    }
}
fn is_hardware(current: &str) -> bool {
    !current.is_empty() && current != "no"
}
/// How many hardware decoding failures in a row mpv tolerates before it
/// falls back to the next method and finally to software
/// (`hwdec-software-fallback`, mpv's default 3); the player sets it at
/// start. A runtime `hwdec` switch (`frames_in_memory`) reopens the decoder
/// in place, usually mid-GOP, and the frames already in flight fail on the
/// new VideoToolbox session (`vt decoder cb: output image buffer is null:
/// -12909`, 14-17 frames measured) before it recovers on its own. With 3,
/// removing the last chain left the file on software decoding, more often
/// the busier the machine. 32 is the most that keeps mpv's packet replay
/// for a decoder that fails while probing (`software_fallback <= 32` in
/// vd_lavc.c), so a decoder that really cannot decode still loses no frames
/// at file start. More frames than that, or a switch under heavy load, can
/// still fall back; `Processing::hwdec_event` then asks for hardware
/// decoding once more.
pub(crate) const HWDEC_SOFTWARE_FALLBACK: &str = "32";
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
pub(crate) fn copy_back(mode: &str) -> Option<String> {
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
/// mpv's `framedrop` while an owned video graph is in place
/// (`Processing::decoder_drops`).
pub(crate) const FRAMEDROP: &str = "decoder+vo";
/// Frames one paused preview step moves (`Processing::preview_step`): mpv
/// has filtered one frame ahead of the picture before the live update ...
const PREVIEW_FRAMES: &str = "2";
/// ... and right after pausing up to two (measured with light chains at
/// 4K: two frames showed no change 4 times in 6, three every time).
const PREVIEW_FIRST_FRAMES: &str = "3";
/// The picture is still where the preview steps left it within this many
/// seconds (`time-pos` can lag the step by a frame); farther, someone else
/// moved it and the settle does not take it back.
const PREVIEW_MOVED: f64 = 0.25;
/// How often `tick` looks at the processing size while a graph acts.
const SIZE_LOOK: Duration = Duration::from_millis(200);
/// The runtime commands of a live change to the owned video graph, in
/// order; the first failure ends them.
fn send(mpv: &Mpv, commands: &[RuntimeCommand]) -> rsmpv::Result<()> {
    for command in commands {
        mpv.command(&[
            "vf-command",
            "vestra-owned",
            command.option,
            &command.value,
            &command.target,
        ])?;
    }
    Ok(())
}
fn paused(mpv: &Mpv) -> bool {
    mpv.get_property::<bool>("pause").unwrap_or(false)
}
/// The frames that reach the owned graph: their display size and how much
/// larger it is than the stored size (`processing_live::factor`).
#[derive(Clone, Copy, Debug, PartialEq)]
struct SourceSize {
    display: (f64, f64),
    /// max(dw / w, dh / h), at least 1: 4/3 for HDV's 1440x1080 shown
    /// 1920 wide.
    anamorphic: f64,
}
/// mpv's `video-params` (after the decoder, aspect applied; a quarter
/// `rotate` swapped, as the orientation stage turns the picture before the
/// graph), before the decoder runs the container's (`current-tracks/video`,
/// its pixel aspect `demux-par`).
fn source_display(mpv: &Mpv) -> Option<SourceSize> {
    let int = |name: &str| mpv.get_property::<i64>(name).ok().filter(|v| *v > 0);
    let (width, height, stored, rotate) = match (int("video-params/dw"), int("video-params/dh")) {
        (Some(w), Some(h)) => (
            w as f64,
            h as f64,
            int("video-params/w").zip(int("video-params/h")),
            mpv.get_property::<i64>("video-params/rotate").unwrap_or(0),
        ),
        _ => {
            let w = int("current-tracks/video/demux-w")?;
            let h = int("current-tracks/video/demux-h")?;
            let par = mpv
                .get_property::<f64>("current-tracks/video/demux-par")
                .ok()
                .filter(|p| p.is_finite() && *p > 0.)
                .unwrap_or(1.);
            let rotate = mpv
                .get_property::<i64>("current-tracks/video/demux-rotation")
                .unwrap_or(0);
            (w as f64 * par, h as f64, Some((w, h)), rotate)
        }
    };
    let anamorphic = stored.map_or(1., |(w, h)| {
        (width / w as f64).max(height / h as f64).max(1.)
    });
    let display = if rotate.rem_euclid(180) == 90 {
        (height, width)
    } else {
        (width, height)
    };
    Some(SourceSize {
        display,
        anamorphic,
    })
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
    let text = effects::render_player(graph).map_err(|e| RpcError::invalid(e.to_string()))?;
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
    let next = effects::render_player(after).map_err(|e| RpcError::invalid(e.to_string()))?;
    let old = effects::render_player(before).map_err(|e| RpcError::invalid(e.to_string()))?;
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
    use crate::playback::session::{IntentTicket, Phase, SourceStamp};

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
        processing.register(OWNER, "examples.blur".into(), None);
        processing.note_source(1, Phase::Ready);
        processing.load = Some((1, ticket(1)));
        processing
    }
    /// Every test here that builds an mpv holds this for the instance's
    /// lifetime (take it before `mpv()`, so it drops last). The hardware
    /// tests (`--include-ignored`) open VideoToolbox / copy-back decoders and
    /// a GPU renderer and wait on playback; one instance at a time keeps
    /// their timing independent of the rest of this module.
    fn one_mpv_at_a_time() -> std::sync::MutexGuard<'static, ()> {
        static MPV: std::sync::Mutex<()> = std::sync::Mutex::new(());
        MPV.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    fn mpv(hwdec: &str) -> Mpv {
        let mpv = Mpv::builder()
            .unwrap()
            .set_property("hwdec-software-fallback", HWDEC_SOFTWARE_FALLBACK)
            .unwrap()
            .set_property("vo", "null")
            .unwrap()
            .set_property("ao", "null")
            .unwrap()
            .set_property("vf", crate::playback::source_geometry::INITIAL_FILTERS)
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
    /// `tick` once the changes settled and a pass-through graph's
    /// teardown time passed.
    fn settled(processing: &mut Processing, mpv: &Mpv) {
        let later = Instant::now() + live::TEARDOWN + Duration::from_millis(10);
        processing.tick(mpv, later, None);
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

    fn watch(hardware_before: bool, restarted: bool) -> HwdecWatch {
        HwdecWatch {
            mode: "auto-safe".into(),
            from: 10.,
            hardware_before,
            retried: false,
            restarted,
        }
    }
    /// `HwdecWatch::step`: a fallback right after the player's own switch
    /// is retried once mpv is done seeking back for it; anything else is
    /// mpv's to decide.
    #[test]
    fn a_software_fallback_is_retried_only_right_after_the_switch() {
        // Software decoding before the switch: nothing to restore.
        assert_eq!(
            watch(false, true).step(Some(10.), Some("no")),
            HwdecCheck::Done
        );
        // Within the window, once restarted (also with no frame shown
        // since the switch).
        assert_eq!(
            watch(true, true).step(Some(10.), Some("no")),
            HwdecCheck::Retry
        );
        assert_eq!(
            watch(true, true).step(Some(12.9), Some("no")),
            HwdecCheck::Retry
        );
        // Not while mpv is still reopening the decoder for the switch.
        assert_eq!(
            watch(true, false).step(Some(10.), Some("no")),
            HwdecCheck::Watch
        );
        assert_eq!(watch(true, true).step(Some(10.4), None), HwdecCheck::Watch);
        assert_eq!(
            watch(true, true).step(Some(10.4), Some("videotoolbox")),
            HwdecCheck::Watch
        );
        // After the window, before the switch (seeked back), or no position:
        // a genuine failure, mpv's fallback stands.
        for position in [Some(13.5), Some(9.), None] {
            for current in ["no", "videotoolbox"] {
                assert_eq!(
                    watch(true, true).step(position, Some(current)),
                    HwdecCheck::Done,
                    "{position:?} {current}"
                );
            }
        }
    }
    #[test]
    fn the_retry_is_used_once_and_reports_its_outcome() {
        let retried = |restarted| HwdecWatch {
            retried: true,
            ..watch(true, restarted)
        };
        // Until the retry's restart the old value may still be read.
        assert_eq!(
            retried(false).step(Some(10.), Some("no")),
            HwdecCheck::Watch
        );
        assert_eq!(
            retried(true).step(Some(10.), Some("videotoolbox-copy")),
            HwdecCheck::Restored
        );
        assert_eq!(retried(true).step(Some(10.), None), HwdecCheck::Watch);
        // Fell back again: no second retry.
        assert_eq!(
            retried(true).step(Some(10.5), Some("no")),
            HwdecCheck::StaysSoftware
        );
        // The window ends: whatever it is now is the outcome.
        assert_eq!(
            retried(false).step(Some(14.), Some("no")),
            HwdecCheck::StaysSoftware
        );
        assert_eq!(retried(false).step(None, None), HwdecCheck::StaysSoftware);
        assert_eq!(
            retried(false).step(Some(14.), Some("videotoolbox")),
            HwdecCheck::Restored
        );
    }
    #[test]
    fn a_switch_without_playback_is_not_watched() {
        let _one = one_mpv_at_a_time();
        let mpv = mpv("auto-safe");
        let mut processing = ready();
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(1, gblur(4.0)))
            .unwrap();
        assert!(!processing.hwdec_watching());
    }
    /// The blur example's sequence: on, off (`[]`), on again. An empty chain
    /// removes the resource (it used to fail `invalid` for video, so "off"
    /// left the blur in place), and while a chain exists decoding copies
    /// frames back to memory so mpv's software filters can run.
    /// mpv's graph carries a `convolution` matrix and a `lut3d` path with a
    /// quote, a comma and a semicolon (player 0.5.20): frames pass through
    /// both filters without a filter or graph error, and the chain asks for
    /// frames in memory like every other node.
    #[test]
    fn mpv_runs_convolution_and_a_lut_from_an_awkward_directory() {
        use std::time::{Duration, Instant};
        let _one = one_mpv_at_a_time();
        let dir = tempfile::tempdir().unwrap();
        let looks = dir.path().join("O'Brien, looks; v2");
        std::fs::create_dir(&looks).unwrap();
        let cube = looks.join("invert.cube");
        let mut text = String::from("LUT_3D_SIZE 2\n");
        for _ in 0..8 {
            text.push_str("0.5 0.5 0.5\n");
        }
        std::fs::write(&cube, text).unwrap();
        let mpv = mpv("auto-safe");
        mpv.request_log_messages("warn").unwrap();
        let mut processing = ready();
        let nodes = json!([
            {"kind":"convolution","planes":[{"mode":"square","matrix":[0,-1,0,-1,5,-1,0,-1,0],"rdiv":1,"bias":0}]},
            {"kind":"lut3d","file":cube.to_str().unwrap(),"interpolation":"tetrahedral"},
        ]);
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(1, nodes))
            .unwrap();
        assert!(vf(&mpv).contains("convolution@n0"), "{}", vf(&mpv));
        assert!(vf(&mpv).contains("lut3d@n1"), "{}", vf(&mpv));
        assert_eq!(
            mpv.get_property::<String>("hwdec").unwrap(),
            "auto-copy-safe"
        );
        mpv.command(&["loadfile", "av://lavfi:testsrc2=s=64x64:d=30"])
            .unwrap();
        let (mut errors, start) = (Vec::new(), Instant::now());
        while start.elapsed() < Duration::from_secs(10)
            && mpv.get_property::<f64>("time-pos").unwrap_or(0.) < 0.5
        {
            while let Some(event) = mpv.poll_event() {
                if let rsmpv::Event::LogMessage(message) = event
                    && matches!(message.level.as_str(), "error" | "fatal")
                {
                    errors.push(format!("{}: {}", message.prefix, message.text));
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            mpv.get_property::<f64>("time-pos").unwrap_or(0.) >= 0.5,
            "played"
        );
        assert!(errors.is_empty(), "{errors:#?}");
    }

    /// Loads a generated 1920x1080 clip and waits until mpv decodes it.
    fn playing(mpv: &Mpv) {
        mpv.command(&["loadfile", "av://lavfi:testsrc2=s=1920x1080:r=30:d=60"])
            .unwrap();
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(10)
            && mpv.get_property::<f64>("time-pos").unwrap_or(0.) < 0.3
        {
            while mpv.poll_event().is_some() {}
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(mpv.get_property::<i64>("video-params/dw").unwrap_or(0) > 0);
    }
    fn view(width: u32, height: u32) -> ViewFacts {
        ViewFacts {
            area: (width, height),
            zoom: 1.,
            fit: crate::playback::video_view::Fit::Keep,
            aspect: None,
        }
    }
    fn eq(brightness: f64) -> Value {
        json!([{"kind":"eq","brightness":brightness,"contrast":1,"saturation":1,"gamma":1}])
    }

    /// A chain of neutral nodes builds no graph: no copy-back decoding, no
    /// decoder frame dropping. A chain that acts turns both on, and they
    /// go back with it.
    #[test]
    fn neutral_chains_cost_nothing_and_a_chain_drops_frames_in_the_decoder() {
        let _one = one_mpv_at_a_time();
        let mpv = mpv("auto-safe");
        let mut processing = ready();
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(1, eq(0.)))
            .unwrap();
        assert_eq!(processing.state()["video"].as_array().unwrap().len(), 1);
        assert!(!vf(&mpv).contains("eq@"), "{}", vf(&mpv));
        assert_eq!(hwdec(&mpv), "auto-safe");
        assert_eq!(mpv.get_property::<String>("framedrop").unwrap(), "vo");
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(2, eq(0.2)))
            .unwrap();
        assert!(vf(&mpv).contains("eq@n0=brightness=0.2"), "{}", vf(&mpv));
        assert_eq!(hwdec(&mpv), "auto-copy-safe");
        assert_eq!(mpv.get_property::<String>("framedrop").unwrap(), FRAMEDROP);
        processing.disable(&mpv, OWNER).unwrap();
        assert_eq!(hwdec(&mpv), "auto-safe");
        assert_eq!(mpv.get_property::<String>("framedrop").unwrap(), "vo");
    }

    /// The graph runs at the shown size: half of a 1080p source in a
    /// 960x540 area, with the blur's sigma halved; a larger area is
    /// followed once it held (`Retarget`).
    #[test]
    fn the_processing_size_follows_the_shown_size() {
        let _one = one_mpv_at_a_time();
        let mpv = mpv("no");
        let mut processing = ready();
        let t = Instant::now();
        processing.tick(&mpv, t, Some(view(960, 540)));
        playing(&mpv);
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(1, gblur(4.0)))
            .unwrap();
        let text = vf(&mpv);
        assert!(text.contains("scale@vs=w='2*trunc(iw*0.5/2)'"), "{text}");
        assert!(text.contains("gblur@n0=sigma=2:sigmaV=2"), "{text}");
        assert_eq!(processing.state()["videoScale"]["factor"], 0.5);
        // Snapshots and exports keep the plugins' chain at full size.
        let frozen = Descriptors::Video(processing.video_resources());
        assert_eq!(
            effects::render_lavfi(&frozen).unwrap(),
            "gblur@n0=sigma=4:sigmaV=4:steps=3"
        );
        // The same view: nothing changes.
        processing.tick(&mpv, t, Some(view(960, 540)));
        assert!(vf(&mpv).contains("iw*0.5"), "{}", vf(&mpv));
        // Maximized: after the jump's wait, full size.
        let at = |ms: u64| t + Duration::from_millis(ms);
        processing.tick(&mpv, at(300), Some(view(1920, 1080)));
        assert!(vf(&mpv).contains("iw*0.5"), "waits: {}", vf(&mpv));
        processing.tick(&mpv, at(600), Some(view(1920, 1080)));
        let text = vf(&mpv);
        assert!(!text.contains("scale@vs"), "{text}");
        assert!(text.contains("gblur@n0=sigma=4:sigmaV=4"), "{text}");
        assert_eq!(processing.state()["videoScale"]["factor"], 1.0);
    }

    /// A step in flight ends even when the video stage turned uncertain
    /// meanwhile: otherwise `stepping` would hold the state pushes and the
    /// worker would poll forever.
    #[test]
    fn a_preview_step_ends_although_the_video_stage_is_uncertain() {
        let _one = one_mpv_at_a_time();
        let mpv = mpv("no");
        let mut processing = ready();
        let t = Instant::now();
        processing.live.stepping = Some(t);
        processing
            .uncertain
            .insert("video".into(), "test: backend lost".into());
        assert_eq!(
            processing.due_in(&mpv, t),
            Some(Duration::from_millis(10)),
            "polled while it lasts"
        );
        // mpv never paused again (no file): the timeout ends it.
        processing.tick(&mpv, t + live::STEP_TIMEOUT, None);
        assert!(!processing.stepping());
        assert_eq!(processing.due_in(&mpv, t + live::STEP_TIMEOUT), None);
    }

    /// "Play" during a step is applied once the step is over; a new source
    /// or another renderer drops it, and "pause" cancels it.
    #[test]
    fn play_pressed_during_a_preview_step_is_applied_after_it() {
        let _one = one_mpv_at_a_time();
        let mpv = mpv("no");
        let mut processing = ready();
        let t = Instant::now();
        // Outside a step a request is mpv's alone.
        processing.pause_request(false);
        assert!(!processing.resume);
        mpv.set_property("pause", true).unwrap();
        processing.live.stepping = Some(t);
        processing.pause_request(false);
        processing.tick(&mpv, t, None);
        assert!(!processing.stepping());
        assert!(!mpv.get_property::<bool>("pause").unwrap(), "playing");
        // Pause after play: nothing to apply.
        mpv.set_property("pause", true).unwrap();
        processing.live.stepping = Some(t);
        processing.pause_request(false);
        processing.pause_request(true);
        processing.tick(&mpv, t, None);
        assert!(mpv.get_property::<bool>("pause").unwrap());
        // Another source or renderer meanwhile: stays paused.
        processing.live.stepping = Some(t);
        processing.pause_request(false);
        processing.renderer_changed();
        processing.tick(&mpv, t, None);
        assert!(mpv.get_property::<bool>("pause").unwrap());
        processing.live.stepping = Some(t);
        processing.pause_request(false);
        processing.note_source(2, Phase::Loading);
        assert!(!processing.stepping() && !processing.resume);
        processing.tick(&mpv, t, None);
        assert!(mpv.get_property::<bool>("pause").unwrap());
    }

    /// Paused, a live update is shown by stepping two frames; once the
    /// changes settle, the graph text is rewritten with the live value (a
    /// seek would otherwise rebuild the old one) and the picture goes back
    /// to where it was paused.
    #[test]
    fn a_paused_live_update_previews_then_settles_back_where_it_was() {
        let _one = one_mpv_at_a_time();
        let mpv = mpv("no");
        let mut processing = ready();
        playing(&mpv);
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(1, eq(0.1)))
            .unwrap();
        mpv.set_property("pause", true).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let paused_at = mpv.get_property::<f64>("time-pos").unwrap();
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(2, eq(0.3)))
            .unwrap();
        // Live: the graph text still has the old value.
        assert!(vf(&mpv).contains("brightness=0.1"), "{}", vf(&mpv));
        processing.tick(&mpv, Instant::now(), None);
        assert!(processing.stepping(), "a preview step");
        let start = Instant::now();
        while processing.stepping() && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
            processing.tick(&mpv, Instant::now(), None);
        }
        assert!(mpv.get_property::<bool>("pause").unwrap(), "paused again");
        let stepped = mpv.get_property::<f64>("time-pos").unwrap();
        assert!(stepped > paused_at, "{stepped} after {paused_at}");
        // Settled: rewritten and back.
        processing.tick(&mpv, Instant::now() + live::SETTLE, None);
        assert!(vf(&mpv).contains("brightness=0.3"), "{}", vf(&mpv));
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5)
            && (mpv.get_property::<f64>("time-pos").unwrap() - paused_at).abs() > 0.001
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            (mpv.get_property::<f64>("time-pos").unwrap() - paused_at).abs() <= 0.001,
            "back at {paused_at}"
        );
        assert!(mpv.get_property::<bool>("pause").unwrap());
        // The user moves the picture during a drag (a seek): the next step
        // starts over there, and the settle comes back there.
        let wait_steps = |processing: &mut Processing| {
            let start = Instant::now();
            processing.tick(&mpv, Instant::now(), None);
            while processing.stepping() && start.elapsed() < Duration::from_secs(5) {
                std::thread::sleep(Duration::from_millis(10));
                processing.tick(&mpv, Instant::now(), None);
            }
        };
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(3, eq(0.2)))
            .unwrap();
        wait_steps(&mut processing);
        mpv.command(&["seek", "5", "absolute+exact"]).unwrap();
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5)
            && (mpv.get_property::<f64>("time-pos").unwrap() - 5.).abs() > 0.001
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        std::thread::sleep(Duration::from_millis(200));
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(4, eq(0.4)))
            .unwrap();
        wait_steps(&mut processing);
        let anchor = processing.live.anchor.unwrap();
        assert!((anchor - 5.).abs() < 0.001, "re-anchored at {anchor}");
        processing.tick(&mpv, Instant::now() + live::SETTLE, None);
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5)
            && (mpv.get_property::<f64>("time-pos").unwrap() - 5.).abs() > 0.001
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            (mpv.get_property::<f64>("time-pos").unwrap() - 5.).abs() <= 0.001,
            "back at 5, not at {paused_at}"
        );
    }

    #[test]
    fn an_empty_video_chain_removes_the_resource_and_hwdec_follows_the_chain() {
        let _one = one_mpv_at_a_time();
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
        // The node passes frames through until the changes settle (a drag
        // through 0 neither rebuilds nor switches decoding) ...
        assert!(
            vf(&mpv).contains("gblur@n0=sigma=4:sigmaV=4:steps=3:enable=0"),
            "{}",
            vf(&mpv)
        );
        assert_eq!(hwdec(&mpv), "auto-copy-safe");
        // ... then it goes, and zero-copy decoding comes back.
        settled(&mut processing, &mpv);
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

    /// Software rendering (Windows) starts in a copy-back mode: a chain
    /// needs no switch and removing it leaves hardware decoding on.
    #[test]
    fn software_rendering_keeps_its_copy_back_mode_with_and_without_a_chain() {
        let _one = one_mpv_at_a_time();
        let software = crate::playback::renderer::hwdec(None, true);
        let mpv = mpv(&software);
        let mut processing = ready();
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(1, gblur(4.0)))
            .unwrap();
        assert_eq!(mpv.get_property::<String>("hwdec").unwrap(), software);
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(2, json!([])))
            .unwrap();
        assert_eq!(mpv.get_property::<String>("hwdec").unwrap(), software);
    }

    /// GPU rendering with a chain (`auto-copy-safe`), then the fallback to
    /// software rendering sets the same string: removing the chain must not
    /// bring back the GPU renderer's zero-copy `auto-safe`.
    #[test]
    fn a_renderer_switch_forgets_the_mode_to_restore() {
        let _one = one_mpv_at_a_time();
        let mpv = mpv(&crate::playback::renderer::hwdec(None, false));
        let mut processing = ready();
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(1, gblur(4.0)))
            .unwrap();
        processing.renderer_changed();
        mpv.set_property(
            "hwdec",
            crate::playback::renderer::hwdec(None, true).as_str(),
        )
        .unwrap();
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(2, json!([])))
            .unwrap();
        assert_eq!(
            mpv.get_property::<String>("hwdec").unwrap(),
            "auto-copy-safe"
        );
    }

    #[test]
    fn hwdec_changed_by_someone_else_is_left_alone() {
        let _one = one_mpv_at_a_time();
        let mpv = mpv("auto-safe");
        let mut processing = ready();
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(1, gblur(4.0)))
            .unwrap();
        // Someone (VESTRA_HWDEC-like user setting) turned it off meanwhile.
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

    fn auto() -> Value {
        json!({"kind":"deinterlace","when":"auto","output":"field"})
    }
    fn deinterlace(mpv: &Mpv) -> String {
        mpv.get_property::<String>("deinterlace").unwrap()
    }
    fn hwdec(mpv: &Mpv) -> String {
        mpv.get_property::<String>("hwdec").unwrap()
    }

    /// DEINT-AUTO-1: `when: "auto"` turns on mpv's own deinterlacer and
    /// keeps zero-copy decoding; other nodes still need copy-back.
    #[test]
    fn auto_deinterlace_keeps_zero_copy_decoding() {
        let _one = one_mpv_at_a_time();
        let mpv = mpv("auto-safe");
        let mut processing = ready();
        assert_eq!(deinterlace(&mpv), "no");
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(1, json!([auto()])))
            .unwrap();
        assert_eq!(deinterlace(&mpv), "auto");
        assert_eq!(hwdec(&mpv), "auto-safe");
        assert!(!vf(&mpv).contains("bwdif"), "{}", vf(&mpv));
        assert_eq!(processing.state()["video"].as_array().unwrap().len(), 1);

        // Another node needs frames in memory; the deinterlacer stays mpv's.
        let both = json!([auto(), {"kind":"gblur","sigma":4,"steps":3}]);
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(2, both))
            .unwrap();
        assert_eq!(hwdec(&mpv), "auto-copy-safe");
        assert_eq!(deinterlace(&mpv), "auto");
        assert!(vf(&mpv).contains("gblur@n0="), "{}", vf(&mpv));
        assert!(!vf(&mpv).contains("bwdif"), "{}", vf(&mpv));

        // Back to the deinterlacer alone: zero-copy again once settled.
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(3, json!([auto()])))
            .unwrap();
        settled(&mut processing, &mpv);
        assert_eq!(hwdec(&mpv), "auto-safe");
        assert_eq!(deinterlace(&mpv), "auto");
        assert!(!vf(&mpv).contains("gblur"), "{}", vf(&mpv));

        // An explicit mode replaces it: bwdif, copy-back, mpv's off.
        let explicit = json!([{"kind":"deinterlace","when":"interlaced","output":"frame"}]);
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(4, explicit))
            .unwrap();
        assert_eq!(deinterlace(&mpv), "no");
        assert_eq!(hwdec(&mpv), "auto-copy-safe");
        assert!(
            vf(&mpv).contains("bwdif@n0=mode=send_frame:deint=interlaced"),
            "{}",
            vf(&mpv)
        );

        // And back, then removed with an empty chain.
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(5, json!([auto()])))
            .unwrap();
        assert_eq!(deinterlace(&mpv), "auto");
        assert_eq!(hwdec(&mpv), "auto-safe");
        assert!(!vf(&mpv).contains("bwdif"), "{}", vf(&mpv));
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(6, json!([])))
            .unwrap();
        assert_eq!(deinterlace(&mpv), "no");
        assert_eq!(hwdec(&mpv), "auto-safe");

        // Refused before anything changes.
        for (revision, nodes) in [
            (
                7,
                json!([{"kind":"deinterlace","when":"auto","output":"frame"}]),
            ),
            (
                8,
                json!([auto(), {"kind":"deinterlace","when":"all","output":"field"}]),
            ),
        ] {
            let error = processing
                .replace(&mpv, OWNER, 1, Source::Decoded, &video(revision, nodes))
                .unwrap_err();
            assert_eq!(error.code, "invalid");
            assert_eq!(deinterlace(&mpv), "no");
        }
    }

    /// The upscaling `decoding` reason names whose chain switched decoding
    /// to copy-back: by manifest name (the id without one), only chains
    /// with nodes that need frames in memory, and nobody when the host did
    /// not switch (software rendering's or the user's own copy-back mode).
    #[test]
    fn copy_back_owners_name_the_chains_that_switched_decoding() {
        let names = |processing: &Processing| {
            processing
                .copy_back_owners()
                .into_iter()
                .map(|owner| owner.name)
                .collect::<Vec<_>>()
        };
        const OTHER: u64 = 8;
        const THIRD: u64 = 9;
        let _one = one_mpv_at_a_time();
        let mpv = mpv("auto-safe");
        let mut processing = Processing::default();
        processing.register(OWNER, "examples.blur".into(), Some("Vestra Blur".into()));
        processing.register(OTHER, "examples.other".into(), Some("Deinterlacer".into()));
        processing.register(THIRD, "examples.third".into(), Some("  ".into()));
        processing.note_source(1, Phase::Ready);
        processing.load = Some((1, ticket(1)));
        assert!(processing.copy_back_owners().is_empty());

        // Only the player's deinterlacer: zero-copy stays, nobody named.
        processing
            .replace(&mpv, OTHER, 1, Source::Decoded, &video(1, json!([auto()])))
            .unwrap();
        assert_eq!(hwdec(&mpv), "auto-safe");
        assert!(processing.copy_back_owners().is_empty());

        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(1, gblur(4.)))
            .unwrap();
        assert_eq!(hwdec(&mpv), "auto-copy-safe");
        assert_eq!(names(&processing), ["Vestra Blur"]);
        assert_eq!(processing.copy_back_owners()[0].id, "examples.blur");
        // A blank name: the manifest id.
        processing
            .replace(&mpv, THIRD, 1, Source::Decoded, &video(1, gblur(2.)))
            .unwrap();
        assert_eq!(names(&processing), ["Vestra Blur", "examples.third"]);

        processing.disable(&mpv, OWNER).unwrap();
        assert_eq!(names(&processing), ["examples.third"]);
        processing.disable(&mpv, THIRD).unwrap();
        assert_eq!(hwdec(&mpv), "auto-safe");
        assert!(processing.copy_back_owners().is_empty());

        // Already copy-back (software rendering's mode): the host switched
        // nothing, so the chain is not named as the cause.
        drop(mpv);
        let copy = self::mpv("auto-copy-safe");
        let mut processing = Processing::default();
        processing.register(OWNER, "examples.blur".into(), Some("Vestra Blur".into()));
        processing.note_source(1, Phase::Ready);
        processing.load = Some((1, ticket(1)));
        processing
            .replace(&copy, OWNER, 1, Source::Decoded, &video(1, gblur(4.)))
            .unwrap();
        assert!(processing.copy_back_owners().is_empty());
    }

    /// mpv's `deinterlace` is one option: on while any owner's chain asks
    /// for it, off once the last one is removed or its plugin stops.
    #[test]
    fn auto_deinterlace_is_shared_and_revoked_with_its_owners() {
        const OTHER: u64 = 8;
        let _one = one_mpv_at_a_time();
        let mpv = mpv("auto-safe");
        let mut processing = ready();
        processing.register(OTHER, "examples.other".into(), None);
        for owner in [OWNER, OTHER] {
            processing
                .replace(&mpv, owner, 1, Source::Decoded, &video(1, json!([auto()])))
                .unwrap();
        }
        assert_eq!(deinterlace(&mpv), "auto");
        processing.disable(&mpv, OWNER).unwrap();
        assert_eq!(deinterlace(&mpv), "auto", "the other owner still asks");
        assert_eq!(hwdec(&mpv), "auto-safe");
        processing.disable(&mpv, OTHER).unwrap();
        assert_eq!(deinterlace(&mpv), "no");

        // Stopping a plugin whose chain had other nodes restores both.
        processing.register(OWNER, "examples.blur".into(), None);
        let both = json!([auto(), {"kind":"gblur","sigma":4,"steps":3}]);
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(2, both))
            .unwrap();
        assert_eq!(hwdec(&mpv), "auto-copy-safe");
        processing.disable(&mpv, OWNER).unwrap();
        assert_eq!(deinterlace(&mpv), "no");
        assert_eq!(hwdec(&mpv), "auto-safe");
        assert!(!vf(&mpv).contains("gblur"), "{}", vf(&mpv));
    }

    /// A graph change that fails after the deinterlacer was switched puts
    /// the deinterlacer back (`deinterlace_then`, used by `replace`).
    #[test]
    fn a_failed_graph_change_restores_the_deinterlacer() {
        let _one = one_mpv_at_a_time();
        let mpv = mpv("auto-safe");
        let mut processing = ready();
        let seen = std::cell::Cell::new(String::new());
        let error = processing
            .deinterlace_then(&mpv, true, |_| {
                // The graph change runs with the new deinterlacer.
                seen.set(deinterlace(&mpv));
                Err(RpcError::new("backend", "vf refused"))
            })
            .unwrap_err();
        assert_eq!(seen.take(), "auto");
        assert_eq!(error.code, "backend");
        assert_eq!(deinterlace(&mpv), "no");
        assert!(!processing.deinterlace);
        // And the other way: on, then a failed change that would turn it off.
        processing.deinterlace_then(&mpv, true, |_| Ok(())).unwrap();
        assert_eq!(deinterlace(&mpv), "auto");
        processing
            .deinterlace_then(&mpv, false, |_| Err(RpcError::new("backend", "vf refused")))
            .unwrap_err();
        assert_eq!(deinterlace(&mpv), "auto");
        assert!(processing.deinterlace);
    }

    /// A new file replays the chains: a chain with only the player's
    /// deinterlacer needs no frames in memory, so decoding stays zero-copy;
    /// the global `deinterlace` option is left as it is.
    #[test]
    fn replaying_an_auto_only_chain_keeps_zero_copy_decoding() {
        let _one = one_mpv_at_a_time();
        let mpv = mpv("auto-safe");
        let mut processing = ready();
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(1, json!([auto()])))
            .unwrap();
        processing.note_source(2, Phase::Loading);
        processing.note_source(2, Phase::Ready);
        processing.replay_ready(&mpv, 2, ticket(2)).unwrap();
        assert_eq!(
            processing.state()["source"]["stages"]["video"]["status"],
            "applied"
        );
        assert_eq!(hwdec(&mpv), "auto-safe");
        assert_eq!(deinterlace(&mpv), "auto");
        assert!(!vf(&mpv).contains("bwdif"), "{}", vf(&mpv));
        // With another node the replay switches to copy-back first.
        let both = json!([auto(), {"kind":"gblur","sigma":4,"steps":3}]);
        let change = json!({"stage":"video","resourceId":"blur","revision":2,"expectedGeneration":2,"nodes":both});
        processing
            .replace(&mpv, OWNER, 2, Source::Decoded, &change)
            .unwrap();
        // As if the earlier file had ended with zero-copy decoding.
        processing.frames_in_memory(&mpv, false);
        assert_eq!(hwdec(&mpv), "auto-safe");
        processing.note_source(3, Phase::Loading);
        processing.note_source(3, Phase::Ready);
        processing.replay_ready(&mpv, 3, ticket(3)).unwrap();
        assert_eq!(hwdec(&mpv), "auto-copy-safe");
    }

    #[test]
    fn generated_streams_refuse_chains_but_accept_removal() {
        let _one = one_mpv_at_a_time();
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
        let _one = one_mpv_at_a_time();
        let fixture: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/processing-conformance.json"
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
    /// Software rendering (the Windows renderer) with the host's initial
    /// filters (`@vestra-orientation`): `renderer::hwdec(None, true)` decodes
    /// in hardware with copy-back (`videotoolbox-copy`, `d3d11va-copy`…),
    /// before and with an owned chain; up to 0.4.2 this was `hwdec=no`.
    /// Needs a hardware decoder and `ffmpeg`:
    /// `cargo test --locked -- --ignored software_rendering_decodes`.
    #[test]
    #[ignore = "needs a hardware decoder and ffmpeg"]
    fn software_rendering_decodes_in_hardware() {
        use std::time::{Duration, Instant};
        let _one = one_mpv_at_a_time();
        let dir = std::env::temp_dir().join(format!("vestra-swdec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("h264.mkv");
        let made = std::process::Command::new("ffmpeg")
            .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i"])
            .arg("testsrc2=size=1280x720:rate=25")
            .args(["-t", "6", "-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .arg(&file)
            .status()
            .unwrap();
        assert!(made.success());
        let mpv = std::sync::Arc::new(mpv(&crate::playback::renderer::hwdec(None, true)));
        mpv.set_property("keep-open", true).unwrap();
        mpv.set_property("vo", "libmpv").unwrap();
        mpv.request_log_messages("v").unwrap();
        let mut render = rsmpv::render::RenderContext::new_software(mpv.clone()).unwrap();
        mpv.command(&["loadfile", file.to_str().unwrap()]).unwrap();
        let failed = std::cell::Cell::new(false);
        let mut pixels = vec![0u8; 640 * 360 * 3];
        let mut pump = |seconds: u64| {
            let end = Instant::now() + Duration::from_secs(seconds);
            while Instant::now() < end {
                while let Some(event) = mpv.poll_event() {
                    if let rsmpv::Event::LogMessage(message) = event
                        && message.text.contains("because it has failed")
                    {
                        failed.set(true);
                    }
                }
                if render.update() {
                    render
                        .render_software(
                            640,
                            360,
                            rsmpv::render::SwPixelFormat::Rgb24,
                            640 * 3,
                            &mut pixels,
                        )
                        .unwrap();
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        pump(2);
        let current = || {
            mpv.get_property::<String>("hwdec-current")
                .unwrap_or_default()
        };
        assert!(current().ends_with("-copy"), "hwdec-current={}", current());
        let mut processing = ready();
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(1, gblur(4.0)))
            .unwrap();
        pump(2);
        assert!(!failed.get(), "mpv dropped a filter");
        assert!(vf(&mpv).contains("gblur"));
        assert!(current().ends_with("-copy"), "with a chain: {}", current());
        drop(render);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// DEINT-AUTO-1 on real decoding (software rendering, so frames are in
    /// memory and mpv's auto deinterlacer is `bwdif send_field`): with a
    /// `deinterlace` `auto` chain an interlaced H.264 clip (25i, flagged
    /// top field first) is deinterlaced at field rate, a progressive one is
    /// left alone, and no FFmpeg graph is added. Needs a hardware decoder
    /// and `ffmpeg`: `cargo test --locked -- --ignored auto_deinterlace_on_`.
    #[test]
    #[ignore = "needs a hardware decoder and ffmpeg"]
    fn auto_deinterlace_on_real_decoding() {
        use std::time::{Duration, Instant};
        let _one = one_mpv_at_a_time();
        let dir = std::env::temp_dir().join(format!("vestra-deint-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let make = |name: &str, source: &str, extra: &[&str]| {
            let file = dir.join(name);
            let made = std::process::Command::new("ffmpeg")
                .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i", source])
                .args(["-t", "6", "-c:v", "libx264", "-pix_fmt", "yuv420p"])
                .args(extra)
                .arg(&file)
                .status()
                .unwrap();
            assert!(made.success());
            file
        };
        let interlaced = make(
            "interlaced-tff.mkv",
            "testsrc2=size=1280x720:rate=50,interlace=scan=tff",
            &["-flags", "+ildct+ilme", "-x264-params", "tff=1"],
        );
        let progressive = make("progressive.mkv", "testsrc2=size=1280x720:rate=25", &[]);
        let mpv = std::sync::Arc::new(mpv(&crate::playback::renderer::hwdec(None, true)));
        mpv.set_property("keep-open", true).unwrap();
        mpv.set_property("vo", "libmpv").unwrap();
        mpv.request_log_messages("v").unwrap();
        let mut render = rsmpv::render::RenderContext::new_software(mpv.clone()).unwrap();
        let problems = std::cell::RefCell::new(Vec::new());
        let mut pixels = vec![0u8; 640 * 360 * 3];
        let mut pump = |seconds: u64| {
            let end = Instant::now() + Duration::from_secs(seconds);
            while Instant::now() < end {
                while let Some(event) = mpv.poll_event() {
                    if let rsmpv::Event::LogMessage(message) = event
                        && (message.text.contains("because it has failed")
                            || message.text.contains("no deinterlace filter available")
                            || message.text.contains("creating deinterlacer failed"))
                    {
                        problems.borrow_mut().push(message.text.to_owned());
                    }
                }
                if render.update() {
                    render
                        .render_software(
                            640,
                            360,
                            rsmpv::render::SwPixelFormat::Rgb24,
                            640 * 3,
                            &mut pixels,
                        )
                        .unwrap();
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            pixels.clone()
        };
        let active = || mpv.get_property::<bool>("deinterlace-active").unwrap();
        let fps = || {
            mpv.get_property::<f64>("estimated-vf-fps")
                .unwrap_or_default()
        };
        let current = || {
            mpv.get_property::<String>("hwdec-current")
                .unwrap_or_default()
        };
        let interlaced_flag = || {
            mpv.get_property::<bool>("video-frame-info/interlaced")
                .unwrap_or_default()
        };
        mpv.command(&["loadfile", interlaced.to_str().unwrap()])
            .unwrap();
        pump(2);
        assert!(interlaced_flag(), "the clip is flagged interlaced");
        assert!(!active());
        let mut processing = ready();
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(1, json!([auto()])))
            .unwrap();
        pump(3);
        assert!(problems.borrow().is_empty(), "{:?}", problems.borrow());
        assert!(active(), "mpv's deinterlacer runs on the interlaced clip");
        assert!((fps() - 50.).abs() < 3., "one frame per field: {}", fps());
        assert!(!vf(&mpv).contains("bwdif"), "{}", vf(&mpv));
        assert!(current().ends_with("-copy"), "hwdec-current={}", current());
        // Paused, stopping the last owner shows the combed frame at once:
        // mpv's deinterlacer is off before the graph change redraws.
        mpv.set_property("pause", true).unwrap();
        let deinterlaced = pump(1);
        assert_eq!(deinterlaced, pump(1), "paused: the picture stays");
        processing.disable(&mpv, OWNER).unwrap();
        assert_eq!(deinterlace(&mpv), "no");
        let combed = pump(1);
        assert_ne!(deinterlaced, combed, "the redraw is not deinterlaced");
        mpv.set_property("pause", false).unwrap();
        mpv.command(&["loadfile", progressive.to_str().unwrap()])
            .unwrap();
        pump(3);
        assert!(!interlaced_flag());
        assert!(!active(), "progressive frames are left alone");
        assert!((fps() - 25.).abs() < 3., "the source rate: {}", fps());
        assert!(problems.borrow().is_empty(), "{:?}", problems.borrow());
        drop(render);
        let _ = std::fs::remove_dir_all(dir);
    }
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
        let _one = one_mpv_at_a_time();
        let dir = std::env::temp_dir().join(format!("vestra-gblur-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("four3.mp4");
        // One GOP (libx264's keyint 250 > 150 frames): both switches below
        // land mid-GOP while playing, where the reopened VideoToolbox
        // decoder fails a few frames (`HWDEC_SOFTWARE_FALLBACK`).
        let made = std::process::Command::new("ffmpeg")
            .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i"])
            .arg("testsrc2=size=1440x1080:rate=25")
            .args(["-t", "6", "-c:v", "libx264", "-pix_fmt", "yuv420p"])
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
        mpv.set_property(
            "hwdec",
            crate::playback::renderer::hwdec(None, false).as_str(),
        )
        .unwrap();
        mpv.command(&["loadfile", file.to_str().unwrap()]).unwrap();
        let failed = std::cell::Cell::new(false);
        let format = || {
            mpv.get_property::<String>("video-out-params/pixelformat")
                .unwrap_or_default()
        };
        let current = || {
            mpv.get_property::<String>("hwdec-current")
                .unwrap_or_default()
        };
        let position = || mpv.get_property::<f64>("time-pos").unwrap_or(0.);
        // Pumps events until `done` holds or 10 s pass.
        let until = |done: &dyn Fn() -> bool| {
            let end = Instant::now() + Duration::from_secs(10);
            while Instant::now() < end && !done() {
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
        until(&|| position() >= 1.);
        assert_eq!(
            format(),
            "videotoolbox",
            "zero-copy decoding before any chain"
        );
        let mut processing = ready();
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(1, gblur(4.0)))
            .unwrap();
        until(&|| position() >= 2.5);
        assert!(!failed.get(), "mpv dropped the owned filter");
        // Still hardware decoding, copied back: not mpv's software fallback.
        assert_eq!(
            current(),
            "videotoolbox-copy",
            "frames reach the filter in memory"
        );
        assert!(vf(&mpv).contains("gblur"));
        processing
            .replace(&mpv, OWNER, 1, Source::Decoded, &video(2, json!([])))
            .unwrap();
        // Pass-through until the changes settle (`TEARDOWN`), then gone.
        settled(&mut processing, &mpv);
        until(&|| position() >= 4.);
        assert_eq!(format(), "videotoolbox", "zero-copy again without a chain");
        assert_eq!(current(), "videotoolbox");
        drop(renderer);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A single-GOP 1440x1080 25 fps H.264 clip (libx264's keyint 250)
    /// in its own temporary directory. Measured on an M-series Mac, a
    /// runtime switch between copy-back and zero-copy decoding here fails
    /// 11-15 frames on some switches, more often the busier the machine,
    /// and reopening the decoder from software decoding fails none.
    /// (50-60 fps clips fail 9-17 frames on every reopen, from software
    /// too, so a retry could not be told from the switch.)
    #[cfg(target_os = "macos")]
    struct Clip(std::path::PathBuf);
    #[cfg(target_os = "macos")]
    impl Clip {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("vestra-{name}-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let made = std::process::Command::new("ffmpeg")
                .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i"])
                .arg("testsrc2=size=1440x1080:rate=25")
                .args(["-t", "10", "-c:v", "libx264", "-pix_fmt", "yuv420p"])
                .arg(dir.join("clip.mp4"))
                .status()
                .unwrap();
            assert!(made.success());
            Self(dir)
        }
        fn file(&self) -> String {
            self.0.join("clip.mp4").to_string_lossy().into_owned()
        }
    }
    #[cfg(target_os = "macos")]
    impl Drop for Clip {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    /// GPU rendering (VideoToolbox, `hwdec=auto-safe`) with `hwdec-current`
    /// observed, as in the backend.
    #[cfg(target_os = "macos")]
    struct GpuPlayback {
        renderer: Option<crate::platform::GpuRenderer>,
        mpv: std::sync::Arc<Mpv>,
    }
    #[cfg(target_os = "macos")]
    impl GpuPlayback {
        fn start(fallback: &str) -> Self {
            let mpv = std::sync::Arc::new(mpv("no"));
            mpv.set_property("keep-open", true).unwrap();
            mpv.set_property("vo", "libmpv").unwrap();
            mpv.set_property("hwdec-software-fallback", fallback)
                .unwrap();
            observe_hwdec(&mpv).unwrap();
            let publish = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
            let renderer = crate::platform::gpu_start(mpv.clone(), (720, 540), publish).unwrap();
            mpv.set_property(
                "hwdec",
                crate::playback::renderer::hwdec(None, false).as_str(),
            )
            .unwrap();
            Self {
                renderer: Some(renderer),
                mpv,
            }
        }
        /// Plays `clip` from the start; `options` are mpv's per-file
        /// options (`loadfile`'s fourth argument).
        fn load(&self, clip: &Clip, options: &str) {
            self.mpv
                .command(&["loadfile", &clip.file(), "replace", "-1", options])
                .unwrap();
            let loaded = std::cell::Cell::new(false);
            self.pump(
                &mut |event| {
                    if matches!(event, rsmpv::Event::FileLoaded) {
                        loaded.set(true);
                    }
                },
                &|| loaded.get() && self.position() >= 0.5,
            );
            assert!(loaded.get(), "{} did not load", clip.file());
        }
        fn position(&self) -> f64 {
            self.mpv.get_property::<f64>("time-pos").unwrap_or(0.)
        }
        fn current(&self) -> String {
            hwdec_current(&self.mpv).unwrap_or_default()
        }
        /// Hands every event to `each` until `done` holds or 5 s pass.
        fn pump(&self, each: &mut dyn FnMut(&rsmpv::Event), done: &dyn Fn() -> bool) {
            let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while std::time::Instant::now() < end && !done() {
                while let Some(event) = self.mpv.poll_event() {
                    each(&event);
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        /// Plays `clip` with `hwdec-software-fallback=3` while every core is
        /// kept busy, and adds and removes a gblur chain until a switch
        /// makes mpv give up on hardware decoding for the file (usually
        /// within a few switches), with a new player after eight. Each
        /// event goes to `each`. The threshold stays: changing it while
        /// playing reopens the decoder itself (and so would restore
        /// hardware decoding on its own). Returns the player, its
        /// processing book and whether the chain is there.
        fn fallen_back(
            clip: &Clip,
            each: &mut dyn FnMut(&Self, &mut Processing, &rsmpv::Event),
        ) -> (Self, Processing, bool) {
            // A busy machine fails more frames (`HWDEC_SOFTWARE_FALLBACK`).
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let hogs: Vec<_> = (0..std::thread::available_parallelism().map_or(4, |n| n.get()))
                .map(|_| {
                    let stop = stop.clone();
                    std::thread::spawn(move || {
                        let mut x = 0u64;
                        while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                            x = std::hint::black_box(
                                x.wrapping_mul(6364136223846793005).wrapping_add(1),
                            );
                        }
                    })
                })
                .collect();
            let _hogs = Hogs(stop, hogs);
            for attempt in 1..=6 {
                let playback = Self::start("3");
                playback.load(clip, "");
                let mut processing = ready();
                let fell_back = std::cell::Cell::new(false);
                for revision in 1..=8 {
                    let chain = revision % 2 == 1;
                    let nodes = if chain { gblur(4.0) } else { json!([]) };
                    processing
                        .replace(
                            &playback.mpv,
                            OWNER,
                            1,
                            Source::Decoded,
                            &video(revision, nodes),
                        )
                        .unwrap();
                    if !chain {
                        // The pass-through graph goes once the changes
                        // settled: the switch back to zero-copy.
                        settled(&mut processing, &playback.mpv);
                    }
                    let end = playback.position() + 0.6;
                    playback.pump(
                        &mut |event| {
                            if playback.current() == "no" {
                                fell_back.set(true);
                            }
                            each(&playback, &mut processing, event);
                        },
                        &|| fell_back.get() || playback.position() >= end,
                    );
                    if fell_back.get() {
                        diag!("fallback forced: player {attempt}, switch {revision}");
                        return (playback, processing, chain);
                    }
                }
            }
            panic!("no switch made mpv fall back to software");
        }
    }
    /// Busy threads, stopped on drop.
    #[cfg(target_os = "macos")]
    struct Hogs(
        std::sync::Arc<std::sync::atomic::AtomicBool>,
        Vec<std::thread::JoinHandle<()>>,
    );
    #[cfg(target_os = "macos")]
    impl Drop for Hogs {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::Relaxed);
            for hog in self.1.drain(..) {
                let _ = hog.join();
            }
        }
    }
    #[cfg(target_os = "macos")]
    impl Drop for GpuPlayback {
        fn drop(&mut self) {
            drop(self.renderer.take());
        }
    }
    /// A runtime switch that makes mpv fall back to software is followed by
    /// one retry with the intended mode, and hardware decoding is back for
    /// the rest of the file (`Processing::hwdec_event`). Under the test's
    /// threshold the retry's own reopening can fail too now and then; then
    /// there is no second retry and the file stays on software, and the
    /// test tries again with a new player.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "needs a GPU, VideoToolbox and ffmpeg"]
    fn gpu_decoding_comes_back_after_a_fallback_caused_by_a_switch() {
        let _one = one_mpv_at_a_time();
        let clip = Clip::new("hwdec-retry");
        for _ in 0..4 {
            let (playback, mut processing, chain) =
                GpuPlayback::fallen_back(&clip, &mut |playback, processing, event| {
                    processing.hwdec_event(&playback.mpv, event);
                });
            let watching = std::cell::Cell::new(processing.hwdec_watching());
            playback.pump(
                &mut |event| {
                    processing.hwdec_event(&playback.mpv, event);
                    watching.set(processing.hwdec_watching());
                },
                &|| !watching.get(),
            );
            assert!(!processing.hwdec_watching(), "the retry has an outcome");
            assert_eq!(processing.hwdec_retries, 1);
            let outcome = playback.current();
            let end = playback.position() + 1.;
            playback.pump(
                &mut |event| processing.hwdec_event(&playback.mpv, event),
                &|| playback.position() >= end,
            );
            assert_eq!(playback.current(), outcome, "and it stays");
            assert_eq!(processing.hwdec_retries, 1, "one retry per switch");
            let expected = if chain {
                "videotoolbox-copy"
            } else {
                "videotoolbox"
            };
            if outcome == expected {
                return;
            }
            assert_eq!(outcome, "no");
        }
        panic!("the retry never restored hardware decoding");
    }
    /// After a software fallback the next file starts with hardware
    /// decoding again: mpv tries the configured mode at each decoder
    /// initialization. No retry here (events are not handed to
    /// `Processing`), so the fallback holds until the next file.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "needs a GPU, VideoToolbox and ffmpeg"]
    fn the_next_file_starts_with_hardware_decoding_after_a_fallback() {
        let _one = one_mpv_at_a_time();
        let clip = Clip::new("hwdec-next");
        let (playback, _processing, chain) = GpuPlayback::fallen_back(&clip, &mut |_, _, _| {});
        let end = playback.position() + 0.5;
        playback.pump(&mut |_| {}, &|| playback.position() >= end);
        assert_eq!(playback.current(), "no", "mpv keeps software for this file");
        // With the player's threshold: a fresh VideoToolbox session
        // right after failures can fail a few frames at the start too. Per
        // file, so the old file's decoder is not reopened by the change.
        playback.load(
            &clip,
            &format!("hwdec-software-fallback={HWDEC_SOFTWARE_FALLBACK}"),
        );
        let expected = if chain {
            "videotoolbox-copy"
        } else {
            "videotoolbox"
        };
        assert_eq!(playback.current(), expected, "next file");
    }
}
