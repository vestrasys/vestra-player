//! Finite subtitle resource adapter. Public libmpv commands, mature WebVTT/ASS rendering.
#![forbid(unsafe_code)]
use crate::{
    RpcError,
    processing_operations::{self as operations, Admission, Key, Stage},
    session::{IntentTicket, Phase},
    subtitle_format,
    subtitle_state::{self as model, Track},
    subtitle_style::{self as presentation, Layer, Style},
    subtitle_upload as upload,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use rsmpv::{Mpv, Node};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    sync::Arc,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum Target {
    Embedded {
        id: i64,
    },
    External {
        owner: u64,
        resource_id: String,
        content_revision: u64,
    },
}
impl Target {
    fn validate(&self) -> Result<(), RpcError> {
        match self {
            Self::Embedded { id } if *id > 0 && (*id as u64) <= operations::Book::MAX_REVISION => {
                Ok(())
            }
            Self::External {
                owner,
                resource_id,
                content_revision,
            } if *owner > 0
                && *owner <= operations::Book::MAX_REVISION
                && !resource_id.is_empty()
                && resource_id.len() <= 128
                && *content_revision > 0
                && *content_revision <= operations::Book::MAX_REVISION =>
            {
                Ok(())
            }
            _ => Err(RpcError::invalid(
                "bounded subtitle track identity required",
            )),
        }
    }
    fn track(&self) -> Track {
        match self {
            Self::Embedded { id } => Track::Embedded(*id),
            Self::External {
                owner,
                resource_id,
                content_revision,
            } => Track::External {
                owner: *owner,
                resource_id: resource_id.clone(),
                content_revision: *content_revision,
            },
        }
    }
}
fn target(track: &Track) -> Value {
    match track {
        Track::Embedded(id) => json!({"kind":"embedded","id":id}),
        Track::External {
            owner,
            resource_id,
            content_revision,
        } => {
            json!({"kind":"external","owner":owner,"resourceId":resource_id,"contentRevision":content_revision})
        }
    }
}
#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Stamp {
    generation: u64,
    revision: u64,
}
impl Stamp {
    fn model(self) -> model::Stamp {
        model::Stamp {
            generation: self.generation,
            revision: self.revision,
        }
    }
}
#[derive(Deserialize, Serialize)]
#[serde(tag = "method", rename_all_fields = "camelCase", deny_unknown_fields)]
enum Mutation {
    #[serde(rename = "subtitles.style")]
    Style {
        operation_id: u64,
        revision: u64,
        expected: Stamp,
        style: Option<Style>,
    },
    #[serde(rename = "subtitles.commit")]
    Commit {
        operation_id: u64,
        upload_id: u64,
        sha256: String,
        resource_id: String,
        revision: u64,
        expected: Stamp,
        input_origin: Option<Target>,
        #[serde(default)]
        format: Format,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        select: Selection,
    },
    #[serde(rename = "subtitles.select")]
    Select {
        operation_id: u64,
        target: Option<Target>,
        expected: Stamp,
        #[serde(default)]
        role: Role,
    },
    #[serde(rename = "subtitles.remove")]
    Remove {
        operation_id: u64,
        resource_id: String,
        expected: Stamp,
    },
}
/// Uploaded subtitle formats: SRT is carried to mpv as WebVTT, ASS as is.
#[derive(Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum Format {
    #[default]
    Srt,
    Ass,
}
impl Format {
    fn name(self) -> &'static str {
        match self {
            Self::Srt => "srt",
            Self::Ass => "ass",
        }
    }
}
/// Whether a commit also selects the new track.
#[derive(Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum Selection {
    #[default]
    Primary,
    Secondary,
    None,
}
/// Which subtitle `subtitles.select` sets (R-T3).
#[derive(Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum Role {
    #[default]
    Primary,
    Secondary,
}
#[derive(Clone)]
struct Asset {
    track: i64,
    binding: NativeLoad,
    file: Arc<tempfile::NamedTempFile>,
    digest: String,
    format: Format,
    title: Option<String>,
}
/// Codecs whose cues are text (searchable, styleable), as opposed to bitmaps.
fn text_codec(codec: Option<&str>) -> bool {
    matches!(
        codec,
        Some("subrip" | "ass" | "ssa" | "webvtt" | "mov_text" | "text")
    )
}
struct NativeTrack {
    id: i64,
    title: String,
    language: Option<String>,
    filename: Option<String>,
    style_supported: bool,
    codec: Option<String>,
    stream_index: Option<u32>,
    external: bool,
}
type NativeLoad = (i64, IntentTicket);
type Orphan = (u64, NativeLoad, Arc<tempfile::NamedTempFile>);
type AssetKey = (u64, String, u64);
#[derive(Clone)]
pub enum FrozenSubtitleInput {
    Carrier(Arc<tempfile::NamedTempFile>),
    Native { stream_index: u32, codec: String },
}
#[derive(Clone)]
pub struct FrozenSubtitle {
    pub input: FrozenSubtitleInput,
    pub fonts: crate::font_carrier::FontDirectory,
    pub style: Style,
    pub metadata: Value,
}
pub struct Subtitles {
    fonts: crate::font_carrier::FontDirectory,
    presentation_revision: u64,
    registry: model::Registry,
    uploads: upload::Registry,
    books: BTreeMap<u64, operations::Book>,
    assets: BTreeMap<AssetKey, Asset>,
    generation: u64,
    phase: Phase,
    loaded: Option<NativeLoad>,
    active_load: Option<NativeLoad>,
    orphans: Vec<Orphan>,
    uncertain: Option<String>,
    confirmed: bool,
    manifests: BTreeMap<u64, String>,
    styles: BTreeMap<u64, Layer>,
    style_revisions: BTreeMap<u64, u64>,
    style_baseline: Option<Vec<(&'static str, String)>>,
    style_confirmed: bool,
}
impl Subtitles {
    pub fn new(fonts: crate::font_carrier::FontDirectory) -> Self {
        Self {
            fonts,
            presentation_revision: 0,
            registry: model::Registry::new(),
            uploads: upload::Registry::new(),
            books: BTreeMap::new(),
            assets: BTreeMap::new(),
            generation: 0,
            phase: Phase::Empty,
            loaded: None,
            active_load: None,
            orphans: Vec::new(),
            uncertain: None,
            confirmed: false,
            manifests: BTreeMap::new(),
            styles: BTreeMap::new(),
            style_revisions: BTreeMap::new(),
            style_baseline: None,
            style_confirmed: false,
        }
    }
}
fn font_family(id: &str) -> Option<&'static str> {
    (id == "noto-cjk-sc-regular-2.004").then_some("Noto Sans CJK SC")
}
fn font_ids() -> BTreeSet<String> {
    BTreeSet::from(["noto-cjk-sc-regular-2.004".into()])
}
fn font_catalog() -> Value {
    json!([{"id":"noto-cjk-sc-regular-2.004","family":"Noto Sans CJK SC","resolution":"fixedFace","snapshotEligible":true,"digest":"2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b","faceIndex":0}])
}
fn failure(error: impl std::fmt::Display) -> RpcError {
    RpcError::new("backend", error.to_string())
}
fn model_error(error: model::Error) -> RpcError {
    RpcError::new(
        match error {
            model::Error::Invalid => "invalid",
            model::Error::Stale => "stale",
            model::Error::Conflict => "conflict",
            model::Error::Limit => "busy",
            model::Error::Missing => "missing",
        },
        error.to_string(),
    )
}
fn upload_error(error: upload::Error) -> RpcError {
    RpcError::new(
        match error {
            upload::Error::Invalid => "invalid",
            upload::Error::Stale => "stale",
            upload::Error::Conflict => "conflict",
            upload::Error::Limit => "busy",
            upload::Error::Missing => "missing",
            upload::Error::UnknownExpired => "unknown-expired",
        },
        error.to_string(),
    )
}
fn upload_json(status: upload::Status) -> Value {
    json!({"generation":status.generation,"expectedBytes":status.expected_bytes,"receivedBytes":status.received_bytes,"complete":status.complete})
}
fn id(params: &Value, key: &str) -> Result<u64, RpcError> {
    params[key]
        .as_u64()
        .filter(|v| *v > 0 && *v <= operations::Book::MAX_REVISION)
        .ok_or_else(|| RpcError::invalid(format!("positive safe {key} required")))
}
fn tracks(mpv: &Mpv) -> Result<Vec<NativeTrack>, RpcError> {
    let node = mpv.get_property::<Node>("track-list").map_err(failure)?;
    let Node::Array(items) = node else {
        return Err(failure("track-list is not an array"));
    };
    if items.len() > 1024 {
        return Err(failure("track-list exceeds finite limit"));
    }
    let mut output = Vec::new();
    for item in items {
        if let Node::Map(fields) = item {
            let field = |name: &str| fields.iter().find(|(key, _)| key == name).map(|(_, v)| v);
            if !matches!(field("type"),Some(Node::String(kind)) if kind=="sub") {
                continue;
            }
            if let Some(Node::Int64(id)) = field("id") {
                let text = |key: &str| match field(key) {
                    Some(Node::String(s)) => Some(s.clone()),
                    _ => None,
                };
                output.push(NativeTrack {
                    id: *id,
                    title: text("title")
                        .unwrap_or_else(|| format!("Subtitle {id}"))
                        .chars()
                        .take(512)
                        .collect(),
                    language: text("lang").map(|s| s.chars().take(128).collect()),
                    filename: text("external-filename"),
                    codec: text("codec"),
                    stream_index: match field("ff-index") {
                        Some(Node::Int64(index)) if (0..1024).contains(index) => {
                            Some(*index as u32)
                        }
                        _ => None,
                    },
                    external: matches!(field("external"), Some(Node::Flag(true))),
                    style_supported: matches!(
                        text("codec").as_deref(),
                        Some("ass" | "ssa" | "subrip" | "srt" | "webvtt" | "text" | "mov_text")
                    ),
                });
            }
        }
    }
    Ok(output)
}
fn set_target(
    mpv: &Mpv,
    load: Option<NativeLoad>,
    selection: &model::Snapshot,
    assets: &BTreeMap<AssetKey, Asset>,
) -> Result<(), RpcError> {
    let load = load
        .ok_or_else(|| RpcError::new("unavailable", "current subtitle load binding unavailable"))?;
    let native = |track: &Option<Track>| -> Result<Option<i64>, RpcError> {
        Ok(match track {
            None => None,
            Some(Track::Embedded(id)) => Some(*id),
            Some(Track::External {
                owner,
                resource_id,
                content_revision,
            }) => Some(
                assets
                    .get(&(*owner, resource_id.clone(), *content_revision))
                    .filter(|asset| asset.binding == load)
                    .ok_or_else(|| failure("subtitle asset load binding unavailable"))?
                    .track,
            ),
        })
    };
    // Primary first: mpv refuses a secondary-sid equal to sid.
    for (property, id) in [
        ("secondary-sid", None),
        ("sid", native(&selection.selected)?),
        ("secondary-sid", native(&selection.secondary)?),
    ] {
        match id {
            Some(id) => mpv.set_property(property, id),
            None => mpv.set_property(property, "no"),
        }
        .map_err(failure)?;
        let actual = mpv.get_property::<Node>(property).map_err(failure)?;
        let confirmed = match (id, actual) {
            (Some(expected), Node::Int64(actual)) => expected == actual,
            (None, Node::Flag(false)) => true,
            (None, Node::String(value)) => value == "no",
            _ => false,
        };
        if !confirmed {
            return Err(failure(format!("subtitle {property} was not confirmed")));
        }
    }
    Ok(())
}
impl Subtitles {
    fn add_file(
        &mut self,
        mpv: &Mpv,
        file: Arc<tempfile::NamedTempFile>,
        digest: String,
        owner: u64,
        (format, title): (Format, Option<String>),
    ) -> Result<Asset, RpcError> {
        let binding = self
            .active_load
            .ok_or_else(|| RpcError::new("unavailable", "subtitle current load unknown"))?;
        let before = tracks(mpv)?.into_iter().map(|t| t.id).collect::<Vec<_>>();
        let path = dunce::canonicalize(file.path()).map_err(failure)?;
        let url = path
            .to_str()
            .ok_or_else(|| failure("subtitle path is not UTF-8"))?;
        // `auto` adds without selecting; selection always goes through `sid`.
        let command = match &title {
            Some(title) => mpv.command(&["sub-add", url, "auto", title]),
            None => mpv.command(&["sub-add", url, "auto"]),
        }
        .map_err(failure);
        let after = tracks(mpv);
        let candidates = after.as_ref().ok().map(|tracks| {
            tracks
                .iter()
                .filter(|t| !before.contains(&t.id) && t.filename.as_deref() == Some(url))
                .map(|t| t.id)
                .collect::<Vec<_>>()
        });
        if command.is_ok() && candidates.as_ref().is_some_and(|ids| ids.len() == 1) {
            return Ok(Asset {
                track: candidates.as_ref().unwrap()[0],
                binding,
                file,
                digest,
                format,
                title,
            });
        }
        let command_ok = command.is_ok();
        let mut errors =
            vec![command.err().map(|e| e.message).unwrap_or_else(|| {
                "sub-add did not expose one owned filename/track identity".into()
            })];
        let unowned_new = after.as_ref().is_ok_and(|tracks| {
            tracks
                .iter()
                .any(|t| !before.contains(&t.id) && t.filename.as_deref() != Some(url))
        });
        let mut cleanup_failed = candidates.is_none()
            || unowned_new
            || (command_ok && candidates.as_ref().is_some_and(Vec::is_empty));
        if let Some(ids) = candidates {
            for id in ids {
                if let Err(error) = mpv.command(&["sub-remove", &id.to_string()]) {
                    cleanup_failed = true;
                    errors.push(error.to_string());
                }
            }
        }
        if cleanup_failed {
            self.orphans.push((owner, binding, file));
            let reason = errors.join("; ");
            self.uncertain = Some(reason.clone());
            return Err(RpcError::new("backend-uncertain", reason));
        }
        Err(failure(errors.join("; ")))
    }

    pub fn failed(&mut self, reason: String) {
        self.confirmed = false;
        self.uncertain = Some(reason);
    }
    pub fn register(&mut self, owner: u64, manifest: String) {
        self.books.entry(owner).or_default();
        self.manifests.insert(owner, manifest);
    }
    pub fn note_source(&mut self, generation: u64, phase: Phase) {
        if generation != self.generation
            || (phase == Phase::Loading && self.phase != Phase::Loading)
            || matches!(phase, Phase::Empty | Phase::Error)
        {
            self.confirmed = false;
            self.style_confirmed = false;
            self.loaded = None;
            self.active_load = None;
            self.uploads.retain_generation(generation);
        }
        self.generation = generation;
        self.phase = phase;
    }
    pub fn ready(&mut self, mpv: &Mpv, entry: i64, ticket: IntentTicket) -> Result<(), RpcError> {
        if self.phase != Phase::Ready || ticket.stamp.generation != self.generation {
            return Err(RpcError::new("stale", "subtitle load no longer current"));
        }
        if self.loaded == Some((entry, ticket)) {
            return Ok(());
        }
        self.active_load = Some((entry, ticket));
        // Actual FileLoaded has replaced the old native track namespace.
        self.orphans.clear();
        let native = tracks(mpv)?;
        let ids = native.iter().map(|t| t.id).collect::<Vec<_>>();
        if self.registry.state().stamp.generation != self.generation
            || self.loaded.is_none() && self.registry.state().stamp.generation == 0
        {
            let sid = mpv
                .get_property::<i64>("sid")
                .ok()
                .filter(|id| ids.contains(id));
            self.registry
                .reset_source(self.generation, ids, sid)
                .map_err(model_error)?;
            // `secondary-sid` would otherwise carry over to the new file.
            mpv.set_property("secondary-sid", "no").map_err(failure)?;
            self.assets.clear();
            self.uncertain = None;
        } else {
            self.registry
                .reload_embedded(self.generation, ids)
                .map_err(model_error)?;
            mpv.set_property("sid", "no").map_err(failure)?;
            for (key, asset) in self.assets.clone() {
                let current = self.add_file(
                    mpv,
                    asset.file.clone(),
                    asset.digest.clone(),
                    key.0,
                    (asset.format, asset.title.clone()),
                )?;
                self.assets.insert(key, current);
            }
            set_target(mpv, self.active_load, &self.registry.state(), &self.assets)?;
        }
        let layers = self.styles.values().cloned().collect::<Vec<_>>();
        self.apply_style(mpv, &layers)?;
        self.loaded = Some((entry, ticket));
        self.confirmed = true;
        Ok(())
    }
    pub fn state(&self, mpv: &Mpv) -> Value {
        let snapshot = self.registry.state();
        let native = tracks(mpv).unwrap_or_default();
        // Container stream numbers are trusted only where ff-index is: the
        // leased local loads that force the lavf demuxer (media_tracks.rs).
        let indexed = mpv
            .get_property::<String>("current-demuxer")
            .ok()
            .as_deref()
            == Some("lavf")
            && mpv
                .get_property::<String>("path")
                .is_ok_and(|path| path.starts_with("fd://"));
        let tracks=native.into_iter().map(|native| {
   let managed=self.assets.iter().find(|(_,a)|a.binding==self.active_load.unwrap_or(a.binding)&&self.active_load.is_some()&&a.track==native.id).map(|((owner,id,revision),_)|Track::External{owner:*owner,resource_id:id.clone(),content_revision:*revision});
   let stream_index = if managed.is_none() && !native.external && indexed { native.stream_index } else { None };
   json!({"target":target(&managed.unwrap_or(Track::Embedded(native.id))),"title":native.title,"language":native.language,"styleSupported":native.style_supported,"styleReason":if native.style_supported{None}else{Some("bitmap or unsupported subtitle codec")},
     "codec":native.codec,"text":text_codec(native.codec.as_deref()),"streamIndex":stream_index})
  }).collect::<Vec<_>>();
        let resources=snapshot.resources.iter().map(|r|{let asset=self.assets.get(&(r.owner,r.resource_id.clone(),r.revision));let format=asset.map_or(Format::Srt,|a|a.format);json!({"owner":r.owner,"resourceId":r.resource_id,"revision":r.revision,"format":format.name(),"digest":r.content.digest,"sizeBytes":r.content.bytes.len(),"cues":r.content.cues,"derived":{"format":if format==Format::Ass{"ass"}else{"webvtt"},"digest":asset.map(|a|a.digest.clone())}})}).collect::<Vec<_>>();
        json!({"source":{"generation":self.generation,"phase":match self.phase{Phase::Empty=>"empty",Phase::Loading=>"loading",Phase::Ready=>"ready",Phase::Ended=>"ended",Phase::Error=>"error"}},"confirmation":if self.uncertain.is_some(){"uncertain"}else if self.confirmed{"applied"}else{"waiting"},"error":self.uncertain,"selection":{"stamp":{"generation":snapshot.stamp.generation,"revision":snapshot.stamp.revision},"target":snapshot.selected.as_ref().map(target),"secondary":snapshot.secondary.as_ref().map(target)},"tracks":tracks,"resources":resources,"presentation":self.presentation_state()})
    }
    fn restore(
        &mut self,
        mpv: &Mpv,
        selection: &model::Snapshot,
        assets: &BTreeMap<AssetKey, Asset>,
        original: RpcError,
    ) -> RpcError {
        match set_target(mpv, self.active_load, selection, assets) {
            Ok(()) => original,
            Err(error) => {
                let reason = format!("{}; restore failed: {}", original.message, error.message);
                self.uncertain = Some(reason.clone());
                RpcError::new("backend-uncertain", reason)
            }
        }
    }
    pub fn disable(&mut self, mpv: &Mpv, owner: u64) -> Result<(), RpcError> {
        self.books.remove(&owner);
        self.manifests.remove(&owner);
        self.style_revisions.remove(&owner);
        let removed_style = self.styles.remove(&owner).is_some();
        if removed_style {
            self.presentation_revision = self.presentation_revision.saturating_add(1);
        }
        let style_failure = if removed_style && self.active_load.is_some() {
            let layers = self.styles.values().cloned().collect::<Vec<_>>();
            self.apply_style(mpv, &layers).err()
        } else {
            None
        };
        self.uploads.remove_owner(owner);
        let contributed = self.orphans.iter().any(|(id, _, _)| *id == owner)
            || self.assets.keys().any(|(id, _, _)| *id == owner)
            || self
                .registry
                .state()
                .resources
                .iter()
                .any(|r| r.owner == owner);
        if !contributed {
            return match style_failure {
                Some(error) => {
                    self.style_confirmed = false;
                    self.uncertain = Some(error.message.clone());
                    Err(RpcError::new("backend-uncertain", error.message))
                }
                None => Ok(()),
            };
        }
        let snapshot = self.registry.remove_owner(owner).map_err(model_error)?;
        let mut failures = style_failure
            .into_iter()
            .map(|e| e.message)
            .collect::<Vec<_>>();
        if self.active_load.is_some()
            && let Err(error) = set_target(mpv, self.active_load, &snapshot, &self.assets)
        {
            failures.push(error.message);
        }
        let keys = self
            .assets
            .keys()
            .filter(|(id, _, _)| *id == owner)
            .cloned()
            .collect::<Vec<_>>();
        for key in keys {
            if let Some(asset) = self.assets.remove(&key) {
                if Some(asset.binding) != self.active_load {
                    continue;
                }
                if let Err(error) = mpv.command(&["sub-remove", &asset.track.to_string()]) {
                    failures.push(error.to_string());
                }
            }
        }
        let mut remaining = Vec::new();
        for (id, binding, file) in std::mem::take(&mut self.orphans) {
            if id != owner {
                remaining.push((id, binding, file));
                continue;
            }
            if Some(binding) != self.active_load {
                continue;
            }
            let cleanup = (|| -> Result<(), RpcError> {
                let path = dunce::canonicalize(file.path()).map_err(failure)?;
                let url = path
                    .to_str()
                    .ok_or_else(|| failure("orphan subtitle path unavailable"))?;
                for track in tracks(mpv)?
                    .into_iter()
                    .filter(|t| t.filename.as_deref() == Some(url))
                {
                    mpv.command(&["sub-remove", &track.id.to_string()])
                        .map_err(failure)?;
                }
                Ok(())
            })();
            if let Err(error) = cleanup {
                failures.push(error.message);
                remaining.push((id, binding, file));
            }
        }
        self.orphans = remaining;
        if !failures.is_empty() {
            let reason = failures.join("; ");
            self.uncertain = Some(reason.clone());
            return Err(RpcError::new("backend-uncertain", reason));
        }
        Ok(())
    }
    pub fn freeze(
        &self,
        expected: model::Stamp,
        presentation_revision: u64,
        load: NativeLoad,
        mpv: &Mpv,
    ) -> Result<Option<FrozenSubtitle>, RpcError> {
        let snapshot = self.registry.state();
        if expected != snapshot.stamp || presentation_revision != self.presentation_revision {
            return Err(RpcError::new(
                "stale",
                "subtitle selection or presentation changed",
            ));
        }
        if self.active_load != Some(load)
            || !self.confirmed
            || !self.style_confirmed
            || self.uncertain.is_some()
        {
            return Err(RpcError::new(
                "unavailable",
                "subtitle presentation is not confirmed for this load",
            ));
        }
        let Some(selected) = snapshot.selected.as_ref() else {
            return Ok(None);
        };
        let layers = self.styles.values().cloned().collect::<Vec<_>>();
        let style = presentation::effective(&layers)
            .map_err(failure)?
            .map(|layer| layer.style.clone())
            .unwrap_or_else(|| {
                presentation::default_for(self.fonts.asset().metadata().font_id.into())
            });
        if style.font_id != self.fonts.asset().metadata().font_id {
            return Err(RpcError::new(
                "unsupported",
                "subtitle font cannot be frozen exactly",
            ));
        }
        let actual_delay = mpv.get_property::<f64>("sub-delay").map_err(failure)?;
        if !actual_delay.is_finite()
            || (actual_delay - f64::from(style.offset_ms) / 1000.).abs() > 0.000001
        {
            return Err(RpcError::new(
                "unsupported",
                "mpv sub-delay differs from the frozen typed subtitle style",
            ));
        }
        if let Track::Embedded(id) = selected {
            let demuxer = mpv
                .get_property::<String>("current-demuxer")
                .map_err(failure)?;
            let start = mpv
                .get_property::<f64>("demuxer-start-time")
                .map_err(failure)?;
            if demuxer != "lavf" || !start.is_finite() {
                return Err(RpcError::new(
                    "unsupported",
                    "native subtitle snapshot requires source-bound lavf and a compatible mpv timeline; file origin must still be verified in worker",
                ));
            }
            let native = tracks(mpv)?
                .into_iter()
                .find(|track| track.id == *id)
                .ok_or_else(|| RpcError::new("unavailable", "native subtitle binding missing"))?;
            let codec=native.codec.filter(|codec|matches!(codec.as_str(),"subrip"|"srt")).ok_or_else(||RpcError::new("unsupported","native subtitle snapshot currently supports only plain subrip/srt; ASS, bitmap and other codecs are not flattened"))?;
            if native.external {
                return Err(RpcError::new(
                    "unsupported",
                    "unmanaged external subtitle file cannot be leased",
                ));
            }
            let stream_index = native
                .stream_index
                .ok_or_else(|| RpcError::new("unsupported", "native subtitle ff-index unknown"))?;
            let metadata = json!({"kind":"native","selectionStamp":expected,"presentationRevision":presentation_revision,"track":target(selected),"native":{"trackId":id,"streamIndex":stream_index,"codec":codec,"timeline":"pending-file-origin-validation"},"style":style,"font":self.fonts.asset().metadata(),"coverageValidated":false});
            return Ok(Some(FrozenSubtitle {
                input: FrozenSubtitleInput::Native {
                    stream_index,
                    codec,
                },
                fonts: self.fonts.clone(),
                style,
                metadata,
            }));
        }
        let Track::External {
            owner,
            resource_id,
            content_revision,
        } = selected
        else {
            return Err(RpcError::new(
                "unsupported",
                "unsupported subtitle snapshot selection",
            ));
        };
        let resource = snapshot
            .resources
            .iter()
            .find(|r| {
                r.owner == *owner
                    && r.resource_id == *resource_id
                    && r.revision == *content_revision
            })
            .ok_or_else(|| {
                RpcError::new(
                    "unavailable",
                    "selected immutable subtitle resource unavailable",
                )
            })?;
        let asset = self
            .assets
            .get(&(*owner, resource_id.clone(), *content_revision))
            .filter(|a| a.binding == load)
            .ok_or_else(|| {
                RpcError::new("unavailable", "selected carrier is not bound to this load")
            })?;
        if asset.format == Format::Ass {
            return Err(RpcError::new(
                "unsupported",
                "ASS subtitles are not flattened into processed snapshots",
            ));
        }
        let validated =
            subtitle_format::validate(resource.content.bytes.clone()).map_err(failure)?;
        let text = validated
            .document
            .subtitles
            .iter()
            .map(|cue| cue.text.join("\n"))
            .collect::<Vec<_>>()
            .join("\n");
        self.fonts
            .asset()
            .supports(&text)
            .map_err(|e| RpcError::new("unsupported", e.to_string()))?;
        let metadata = json!({"kind":"external","selectionStamp":expected,"presentationRevision":presentation_revision,"track":target(selected),"canonical":{"format":"srt","digest":resource.content.digest},"derived":{"format":"webvtt","digest":asset.digest},"style":style,"font":self.fonts.asset().metadata(),"coverageValidated":true});
        Ok(Some(FrozenSubtitle {
            input: FrozenSubtitleInput::Carrier(asset.file.clone()),
            fonts: self.fonts.clone(),
            style,
            metadata,
        }))
    }
    fn presentation_state(&self) -> Value {
        let layers = self.styles.values().cloned().collect::<Vec<_>>();
        let effective = presentation::effective(&layers).ok().flatten();
        json!({"revision":self.presentation_revision,"referenceHeight":1080,"layers":layers,"effective":effective,
            "confirmation":if self.uncertain.is_some(){"uncertain"}else if self.active_load.is_some() && self.style_confirmed{"applied"}else{"waiting"},
            "fonts":font_catalog(),"actualFont":self.fonts.asset().metadata(),"snapshotEligible":self.active_load.is_some() && self.style_confirmed,"coverageCheckedOnCapture":true})
    }
    fn apply_style(&mut self, mpv: &Mpv, layers: &[Layer]) -> Result<(), RpcError> {
        let effective =
            presentation::effective(layers).map_err(|e| RpcError::invalid(e.to_string()))?;
        let mut properties = if let Some(layer) = effective {
            let family = font_family(&layer.style.font_id)
                .ok_or_else(|| RpcError::new("unsupported", "subtitle font unavailable"))?;
            presentation::mpv_properties(&layer.style, family)
                .map_err(|e| RpcError::invalid(e.to_string()))?
        } else if let Some(baseline) = &self.style_baseline {
            baseline.clone()
        } else {
            self.style_confirmed = true;
            return Ok(());
        };
        // Commit the decoder override mode after all style fields are ready.
        properties.sort_by_key(|(name, _)| *name == "sub-ass-override");
        let before = properties
            .iter()
            .map(|(name, _)| {
                mpv.get_property::<String>(name)
                    .map(|value| (*name, value))
                    .map_err(failure)
            })
            .collect::<Result<Vec<_>, _>>()?;
        if self.style_baseline.is_none() {
            self.style_baseline = Some(before.clone());
        }
        if std::env::var_os("VESTRA_STYLE_DIAGNOSTICS").is_some() {
            eprintln!("SUBTITLE_STYLE_APPLY desired={properties:?} before={before:?}");
        }
        for (name, value) in &properties {
            if let Err(error) = mpv.set_property(name, value.as_str()) {
                let mut errors = vec![error.to_string()];
                for (old_name, old_value) in &before {
                    if let Err(error) = mpv.set_property(old_name, old_value.as_str()) {
                        errors.push(error.to_string());
                    }
                }
                if errors.len() > 1 {
                    let reason = errors.join("; rollback: ");
                    self.style_confirmed = false;
                    self.uncertain = Some(reason.clone());
                    return Err(RpcError::new("backend-uncertain", reason));
                }
                return Err(failure(errors.join("; ")));
            }
        }
        // mpv can discard the paused lavc subtitle event when override mode
        // changes. Reload only our selected external subtitle, never the media.
        let mode_changed = properties
            .iter()
            .find(|(name, _)| *name == "sub-ass-override")
            .zip(before.iter().find(|(name, _)| *name == "sub-ass-override"))
            .is_some_and(|(new, old)| new.1 != old.1);
        if mode_changed
            && let Some(Track::External {
                owner,
                resource_id,
                content_revision,
            }) = self.registry.state().selected
        {
            let key = (owner, resource_id, content_revision);
            if let Some(asset) = self
                .assets
                .get(&key)
                .cloned()
                .filter(|asset| Some(asset.binding) == self.active_load)
            {
                let refreshed = (|| -> Result<i64, RpcError> {
                    mpv.command(&["sub-reload", &asset.track.to_string()])
                        .map_err(failure)?;
                    let url = dunce::canonicalize(asset.file.path()).map_err(failure)?;
                    let matches = tracks(mpv)?
                        .into_iter()
                        .filter(|track| track.filename.as_deref() == url.to_str())
                        .collect::<Vec<_>>();
                    if matches.len() != 1 {
                        return Err(failure("reloaded subtitle identity ambiguous"));
                    }
                    Ok(matches[0].id)
                })();
                match refreshed {
                    Ok(id) => {
                        self.assets.get_mut(&key).unwrap().track = id;
                        set_target(mpv, self.active_load, &self.registry.state(), &self.assets)?;
                    }
                    Err(error) => {
                        self.style_confirmed = false;
                        self.uncertain = Some(error.message.clone());
                        return Err(RpcError::new("backend-uncertain", error.message));
                    }
                }
            }
        }
        self.style_confirmed = true;
        Ok(())
    }
    fn replace_style(
        &mut self,
        mpv: &Mpv,
        owner: u64,
        revision: u64,
        expected: Stamp,
        style: Option<Style>,
    ) -> Result<Value, RpcError> {
        if expected.model() != self.registry.state().stamp {
            return Err(RpcError::new("stale", "subtitle selection changed"));
        }
        if self.presentation_revision >= operations::Book::MAX_REVISION {
            return Err(RpcError::new(
                "busy",
                "subtitle presentation revision exhausted",
            ));
        }
        if self
            .style_revisions
            .get(&owner)
            .is_some_and(|old| revision <= *old)
        {
            return Err(RpcError::new(
                "stale",
                "subtitle style revision must increase",
            ));
        }
        // A bitmap primary takes no style; a text secondary still does
        // (its position and look), so the style is accepted then.
        if style.is_some()
            && self.registry.state().selected.is_some()
            && self.registry.state().secondary.is_none()
        {
            let selected_id = match &self.registry.state().selected {
                Some(Track::Embedded(id)) => Some(*id),
                Some(Track::External {
                    owner,
                    resource_id,
                    content_revision,
                }) => self
                    .assets
                    .get(&(*owner, resource_id.clone(), *content_revision))
                    .filter(|a| Some(a.binding) == self.active_load)
                    .map(|a| a.track),
                None => None,
            };
            if let Some(id) = selected_id
                && tracks(mpv)?
                    .iter()
                    .any(|track| track.id == id && !track.style_supported)
            {
                return Err(RpcError::new(
                    "unsupported",
                    "selected subtitle codec does not support style",
                ));
            }
        }
        let mut next = self.styles.clone();
        if let Some(style) = style {
            let manifest_id = self
                .manifests
                .get(&owner)
                .cloned()
                .ok_or_else(|| RpcError::new("disabled", "subtitle owner unavailable"))?;
            next.insert(
                owner,
                Layer {
                    owner,
                    manifest_id,
                    revision,
                    style,
                },
            );
        } else {
            next.remove(&owner);
        }
        self.apply_style(mpv, &next.values().cloned().collect::<Vec<_>>())?;
        self.styles = next;
        self.presentation_revision += 1;
        self.style_revisions.insert(owner, revision);
        Ok(self.state(mpv))
    }
    pub fn dispatch(
        &mut self,
        mpv: &Mpv,
        owner: u64,
        method: &str,
        params: &Value,
    ) -> Result<Value, RpcError> {
        if !self.books.contains_key(&owner) {
            return Err(RpcError::new("disabled", "subtitle owner unavailable"));
        }
        if method == "subtitles.state" {
            return Ok(self.state(mpv));
        }
        if method == "subtitles.operation" {
            let operation = id(params, "operationId")?;
            return Ok(
                match self.books[&owner].query(
                    &Key {
                        stage: Stage::Subtitles,
                        resource_id: "control".into(),
                    },
                    operation,
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
            );
        }
        if method.starts_with("subtitles.upload.") {
            let upload = id(params, "uploadId")?;
            return match method {
                "subtitles.upload.begin" => {
                    let generation = params["generation"]
                        .as_u64()
                        .ok_or_else(|| RpcError::invalid("generation required"))?;
                    if generation != self.generation {
                        return Err(RpcError::new("stale", "source generation changed"));
                    }
                    let size = params["expectedBytes"]
                        .as_u64()
                        .and_then(|n| usize::try_from(n).ok())
                        .ok_or_else(|| RpcError::invalid("expectedBytes required"))?;
                    self.uploads
                        .begin(owner, upload, generation, size)
                        .map(upload_json)
                        .map_err(upload_error)
                }
                "subtitles.upload.append" => {
                    let offset = params["offset"]
                        .as_u64()
                        .and_then(|n| usize::try_from(n).ok())
                        .ok_or_else(|| RpcError::invalid("offset required"))?;
                    let encoded = params["base64"]
                        .as_str()
                        .filter(|s| s.len() <= 87384)
                        .ok_or_else(|| RpcError::invalid("bounded base64 required"))?;
                    let bytes = STANDARD
                        .decode(encoded)
                        .map_err(|e| RpcError::invalid(e.to_string()))?;
                    self.uploads
                        .append(owner, upload, self.generation, offset, &bytes)
                        .map(upload_json)
                        .map_err(upload_error)
                }
                "subtitles.upload.discard" => {
                    self.uploads.discard(owner, upload).map_err(upload_error)?;
                    Ok(Value::Null)
                }
                _ => Err(RpcError::invalid("unknown subtitle upload method")),
            };
        }
        let mut object = params
            .as_object()
            .cloned()
            .ok_or_else(|| RpcError::invalid("object required"))?;
        object.insert("method".into(), json!(method));
        let mutation: Mutation = serde_json::from_value(Value::Object(object))
            .map_err(|e| RpcError::invalid(e.to_string()))?;
        let operation = match &mutation {
            Mutation::Commit {
                operation_id,
                resource_id,
                sha256,
                ..
            } => {
                if resource_id.is_empty()
                    || resource_id.len() > 128
                    || sha256.len() != 64
                    || !sha256
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                {
                    return Err(RpcError::invalid(
                        "bounded resourceId/lowercase sha256 required",
                    ));
                }
                *operation_id
            }
            Mutation::Select { operation_id, .. }
            | Mutation::Remove { operation_id, .. }
            | Mutation::Style { operation_id, .. } => *operation_id,
        };
        if operation == 0 || operation > operations::Book::MAX_REVISION {
            return Err(RpcError::invalid("positive safe operationId required"));
        }
        match &mutation {
            Mutation::Style {
                revision, style, ..
            } => {
                if *revision == 0 || *revision > operations::Book::MAX_REVISION {
                    return Err(RpcError::invalid("safe style revision required"));
                }
                if let Some(style) = style {
                    presentation::validate(style, &font_ids()).map_err(|e| {
                        RpcError::new(
                            match e {
                                presentation::Error::UnsupportedFont => "unsupported",
                                presentation::Error::Invalid => "invalid",
                            },
                            e.to_string(),
                        )
                    })?;
                }
            }
            Mutation::Commit {
                upload_id,
                revision,
                input_origin,
                title,
                ..
            } => {
                if *upload_id == 0
                    || *upload_id > operations::Book::MAX_REVISION
                    || *revision == 0
                    || *revision > operations::Book::MAX_REVISION
                {
                    return Err(RpcError::invalid("safe uploadId and revision required"));
                }
                if title.as_ref().is_some_and(|title| {
                    !(1..=200).contains(&title.chars().count())
                        || title.chars().any(char::is_control)
                }) {
                    return Err(RpcError::invalid(
                        "subtitle title needs 1..200 characters without control characters",
                    ));
                }
                if let Some(origin) = input_origin {
                    origin.validate()?;
                }
            }
            Mutation::Select { target, .. } => {
                if let Some(target) = target {
                    target.validate()?;
                }
            }
            Mutation::Remove { resource_id, .. } => {
                if resource_id.is_empty() || resource_id.len() > 128 {
                    return Err(RpcError::invalid("bounded resourceId required"));
                }
            }
        }
        let key = Key {
            stage: Stage::Subtitles,
            resource_id: "control".into(),
        };
        let payload = serde_json::to_value(&mutation).map_err(failure)?;
        match self
            .books
            .get_mut(&owner)
            .unwrap()
            .admit(key.clone(), operation, payload)
        {
            Admission::Execute => (),
            Admission::Replay(result) => {
                return result.map_err(|e| RpcError::new(e.code, e.message));
            }
            Admission::Pending => return Err(RpcError::new("pending", "query subtitle operation")),
            Admission::Conflict => {
                return Err(RpcError::new(
                    "conflict",
                    "subtitle operation token conflict",
                ));
            }
            Admission::UnknownExpired => {
                return Err(RpcError::new(
                    "unknown-expired",
                    "subtitle operation result expired",
                ));
            }
            Admission::Full => {
                return Err(RpcError::new("busy", "subtitle operation capacity reached"));
            }
        }
        let result = self.mutate(mpv, owner, mutation);
        let outcome = result
            .as_ref()
            .map(Clone::clone)
            .map_err(|e| operations::Failure {
                code: e.code.clone(),
                message: e.message.clone(),
            });
        self.books
            .get_mut(&owner)
            .unwrap()
            .finish(&key, operation, outcome)
            .map_err(|e| RpcError::new("internal", e.message))?;
        result
    }
    fn mutate(&mut self, mpv: &Mpv, owner: u64, mutation: Mutation) -> Result<Value, RpcError> {
        let expected = match &mutation {
            Mutation::Commit { expected, .. }
            | Mutation::Select { expected, .. }
            | Mutation::Remove { expected, .. }
            | Mutation::Style { expected, .. } => *expected,
        };
        if expected.generation != self.generation {
            return Err(RpcError::new("stale", "subtitle source generation changed"));
        }
        if let Some(reason) = &self.uncertain {
            return Err(RpcError::new("backend-uncertain", reason));
        }
        if self.phase != Phase::Ready || !self.confirmed {
            return Err(RpcError::new(
                "unavailable",
                "subtitle source must be ready",
            ));
        }
        if let Mutation::Style {
            revision,
            expected,
            style,
            ..
        } = mutation
        {
            return self.replace_style(mpv, owner, revision, expected, style);
        }
        let before = self.registry.state();
        let mut next = self.registry.clone();
        let mut assets = self.assets.clone();
        let mut consumed_upload = None;
        match mutation {
            Mutation::Style { .. } => unreachable!("style handled above"),
            Mutation::Commit {
                upload_id,
                sha256,
                resource_id,
                revision,
                expected,
                input_origin,
                format,
                title,
                select,
                ..
            } => {
                if expected.generation != self.generation || expected.model() != before.stamp {
                    return Err(RpcError::new("stale", "subtitle selection changed"));
                }
                if let Some(origin) = input_origin {
                    let track = origin.track();
                    let exists = match &track {
                        Track::Embedded(id) => tracks(mpv)?.iter().any(|t| t.id == *id),
                        Track::External {
                            owner,
                            resource_id,
                            content_revision,
                        } => before.resources.iter().any(|r| {
                            r.owner == *owner
                                && r.resource_id == *resource_id
                                && r.revision == *content_revision
                        }),
                    };
                    if !exists {
                        return Err(RpcError::new(
                            "stale",
                            "subtitle input content revision changed",
                        ));
                    }
                }
                let bytes = self
                    .uploads
                    .complete(owner, upload_id, self.generation)
                    .map_err(upload_error)?;
                // SRT goes to mpv as WebVTT; ASS/SSA is handed over byte for byte.
                let (content, carrier, suffix) = match format {
                    Format::Srt => {
                        let validated = subtitle_format::validate(bytes)
                            .map_err(|e| RpcError::invalid(e.to_string()))?;
                        let vtt = validated.webvtt();
                        (
                            model::Content {
                                cues: validated.document.subtitles.len() as u32,
                                bytes: validated.source,
                                digest: validated.digest,
                            },
                            vtt.into_bytes(),
                            ".vtt",
                        )
                    }
                    Format::Ass => {
                        let validated = subtitle_format::validate_ass(bytes)
                            .map_err(|e| RpcError::invalid(e.to_string()))?;
                        (
                            model::Content {
                                cues: validated.dialogues,
                                bytes: validated.source.clone(),
                                digest: validated.digest,
                            },
                            validated.source.to_vec(),
                            ".ass",
                        )
                    }
                };
                if content.digest != sha256 {
                    return Err(RpcError::invalid("subtitle sha256 mismatch"));
                }
                let digest = format!("{:x}", Sha256::digest(&carrier));
                match select {
                    Selection::Primary => next.register_and_select(
                        owner,
                        resource_id.clone(),
                        revision,
                        content,
                        expected.model(),
                    ),
                    Selection::Secondary => next.register_as_secondary(
                        owner,
                        resource_id.clone(),
                        revision,
                        content,
                        expected.model(),
                    ),
                    Selection::None => next.register(
                        owner,
                        resource_id.clone(),
                        revision,
                        content,
                        expected.generation,
                    ),
                }
                .map_err(model_error)?;
                let mut file = tempfile::Builder::new()
                    .prefix("vestra-subtitle-")
                    .suffix(suffix)
                    .tempfile()
                    .map_err(failure)?;
                file.write_all(&carrier).map_err(failure)?;
                file.flush().map_err(failure)?;
                set_target(mpv, self.active_load, &before, &self.assets)?;
                let asset = match self.add_file(mpv, Arc::new(file), digest, owner, (format, title))
                {
                    Ok(asset) => asset,
                    Err(error) => {
                        return Err(self.restore(mpv, &before, &self.assets.clone(), error));
                    }
                };
                assets.insert((owner, resource_id, revision), asset);
                consumed_upload = Some(upload_id);
            }
            Mutation::Select {
                target,
                expected,
                role,
                ..
            } => {
                let target = target.map(|t| t.track());
                if role == Role::Secondary {
                    // Bitmap subtitles cannot be the secondary one.
                    if let Some(Track::Embedded(id)) = &target
                        && tracks(mpv)?
                            .iter()
                            .any(|t| t.id == *id && !text_codec(t.codec.as_deref()))
                    {
                        return Err(RpcError::invalid("a bitmap subtitle cannot be secondary"));
                    }
                    next.select_secondary(target, expected.model())
                } else {
                    next.select(owner, target, expected.model())
                }
                .map_err(model_error)?;
            }
            Mutation::Remove {
                resource_id,
                expected,
                ..
            } => {
                next.remove_resource(owner, &resource_id, expected.model())
                    .map_err(model_error)?;
            }
        }
        let after = next.state();
        if let Err(error) = set_target(mpv, self.active_load, &after, &assets) {
            let added = assets
                .iter()
                .filter(|(k, _)| !self.assets.contains_key(*k))
                .map(|(k, a)| (k.clone(), a.clone()))
                .collect::<Vec<_>>();
            let mut cleanup = Vec::new();
            for (key, asset) in added {
                if let Err(failure) = mpv.command(&["sub-remove", &asset.track.to_string()]) {
                    cleanup.push(failure.to_string());
                    self.assets.insert(key, asset);
                }
            }
            let restored = self.restore(mpv, &before, &self.assets.clone(), error);
            if !cleanup.is_empty() {
                let reason = format!(
                    "{}; new track cleanup failed: {}",
                    restored.message,
                    cleanup.join("; ")
                );
                self.uncertain = Some(reason.clone());
                return Err(RpcError::new("backend-uncertain", reason));
            }
            return Err(restored);
        }
        let keep = after
            .resources
            .iter()
            .map(|r| (r.owner, r.resource_id.clone(), r.revision))
            .collect::<Vec<_>>();
        let old = assets
            .keys()
            .filter(|k| !keep.contains(k))
            .cloned()
            .collect::<Vec<_>>();
        for key in old {
            if let Some(asset) = assets.get(&key)
                && let Err(error) = mpv.command(&["sub-remove", &asset.track.to_string()])
            {
                let restored = self.restore(mpv, &before, &self.assets.clone(), failure(error));
                let added = assets
                    .iter()
                    .filter(|(k, _)| !self.assets.contains_key(*k))
                    .map(|(k, a)| (k.clone(), a.clone()))
                    .collect::<Vec<_>>();
                let mut reasons = vec![restored.message];
                for (key, asset) in added {
                    if let Err(error) = mpv.command(&["sub-remove", &asset.track.to_string()]) {
                        reasons.push(error.to_string());
                        self.assets.insert(key, asset);
                    }
                }
                let reason = reasons.join("; ");
                self.uncertain = Some(reason.clone());
                return Err(RpcError::new("backend-uncertain", reason));
            }
            assets.remove(&key);
        }
        self.registry = next;
        self.assets = assets;
        if let Some(upload) = consumed_upload {
            self.uploads.discard(owner, upload).map_err(upload_error)?;
        }
        Ok(self.state(mpv))
    }
}
