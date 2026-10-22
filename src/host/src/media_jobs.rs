//! Finite raw snapshot/job candidate. No product behavior or arbitrary FFmpeg strings.
#![forbid(unsafe_code)]
use crate::{
    RpcError,
    media_worker::{self, Control, Status},
    processing_operations::{self as operations, Admission, Book, Key, Stage},
    session::SourceStamp,
    source_backend::Sources,
    source_lease::Lease,
};
use rsmpv::Mpv;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc, thread::JoinHandle};
pub struct CaptureContext<'a> {
    pub sources: &'a mut Sources,
    pub mpv: &'a Mpv,
    pub geometry: &'a crate::source_geometry::Geometry,
    pub processing: &'a crate::processing::Processing,
    pub subtitles: &'a crate::subtitle_backend::Subtitles,
}
pub struct FrozenProcessed {
    pub processing: crate::processing::FrozenProcessing,
    pub subtitle: Option<crate::subtitle_backend::FrozenSubtitle>,
    pub width: u32,
    pub height: u32,
    pub audio_delay_ms: f64,
}
struct Snapshot {
    owner: u64,
    lease: Arc<Lease>,
    time: f64,
    processed: Option<Arc<FrozenProcessed>>,
    audio: Option<crate::media_tracks::AudioTrack>,
    video_stream: u32,
    source_origin: f64,
    duration: f64,
    summary: Value,
}
struct Job {
    owner: u64,
    control: Arc<Control>,
}
#[derive(Default)]
pub struct Jobs {
    books: BTreeMap<u64, Book>,
    snapshots: BTreeMap<u64, Arc<Snapshot>>,
    jobs: BTreeMap<u64, Job>,
    workers: Vec<JoinHandle<()>>,
    next_snapshot: u64,
    next_job: u64,
}
#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Mode {
    #[default]
    Raw,
    Processed,
}
#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SubtitleCondition {
    selection: crate::subtitle_state::Stamp,
    presentation_revision: u64,
}
#[derive(Deserialize, Serialize)]
#[serde(tag = "method", rename_all_fields = "camelCase", deny_unknown_fields)]
enum Mutation {
    #[serde(rename = "snapshot.capture")]
    Capture {
        operation_id: u64,
        expected: SourceStamp,
        time: Option<f64>,
        #[serde(default)]
        mode: Mode,
        expected_processing_revision: Option<u64>,
        expected_subtitle: Option<SubtitleCondition>,
        #[serde(default = "include_subtitles_default")]
        include_subtitles: bool,
        #[serde(default)]
        audio: crate::media_tracks::Choice,
    },
    #[serde(rename = "snapshot.release")]
    Release { operation_id: u64, snapshot_id: u64 },
    #[serde(rename = "jobs.start")]
    Start {
        operation_id: u64,
        snapshot_id: u64,
        kind: String,
        destination: String,
        end: Option<f64>,
        sample_time: Option<f64>,
        /// JPEG quality 1–100 (`rawJpeg` / `processedJpeg` only; default 90).
        quality: Option<u8>,
    },
    #[serde(rename = "jobs.cancel")]
    Cancel { operation_id: u64, job_id: u64 },
}
/// The still format of a one-frame job kind (`rawPng`, `processedJpeg`, …).
fn still(kind: &str) -> Option<media_worker::Still> {
    match kind
        .strip_prefix("raw")
        .or(kind.strip_prefix("processed"))?
    {
        "Png" => Some(media_worker::Still::Png),
        "Jpeg" => Some(media_worker::Still::Jpeg { quality: 90 }),
        "Bmp" => Some(media_worker::Still::Bmp),
        _ => None,
    }
}
fn include_subtitles_default() -> bool {
    true
}
impl Mutation {
    fn id(&self) -> u64 {
        match self {
            Self::Capture { operation_id, .. }
            | Self::Release { operation_id, .. }
            | Self::Start { operation_id, .. }
            | Self::Cancel { operation_id, .. } => *operation_id,
        }
    }
}
fn safe(value: u64) -> bool {
    value > 0 && value <= Book::MAX_REVISION
}
fn id(params: &Value, key: &str) -> Result<u64, RpcError> {
    params[key]
        .as_u64()
        .filter(|n| safe(*n))
        .ok_or_else(|| RpcError::invalid("positive safe integer identity required"))
}
fn key() -> Key {
    Key {
        stage: Stage::Media,
        resource_id: "control".into(),
    }
}
fn status(status: Status) -> &'static str {
    match status {
        Status::Queued => "queued",
        Status::Running => "running",
        Status::Cancelling => "cancelling",
        Status::Publishing => "running",
        Status::Completed => "completed",
        Status::Failed => "failed",
        Status::Cancelled => "cancelled",
    }
}
fn terminal(status: Status) -> bool {
    matches!(
        status,
        Status::Completed | Status::Failed | Status::Cancelled
    )
}
impl Jobs {
    pub fn register(&mut self, owner: u64) {
        self.books.entry(owner).or_default();
    }
    pub fn poll(&mut self) {
        let mut pending = Vec::new();
        for worker in self.workers.drain(..) {
            if worker.is_finished() {
                if worker.join().is_err() {
                    eprintln!("media worker panicked");
                }
            } else {
                pending.push(worker);
            }
        }
        self.workers = pending;
    }
    pub fn disable(&mut self, owner: u64) {
        self.books.remove(&owner);
        self.snapshots.retain(|_, s| s.owner != owner);
        self.jobs.retain(|_, job| {
            if job.owner == owner {
                job.control.cancel();
                false
            } else {
                true
            }
        });
    }
    fn job_state(&self, owner: u64, id: u64) -> Result<Value, RpcError> {
        let job = self
            .jobs
            .get(&id)
            .filter(|j| j.owner == owner)
            .ok_or_else(|| {
                RpcError::new(
                    "unknown-expired",
                    "job is unavailable; do not resubmit an unknown operation",
                )
            })?;
        let (phase, outcome, publication_started) = job.control.snapshot();
        let outcome = match outcome {
            None => Value::Null,
            Some(media_worker::Outcome::Completed { destination }) => {
                json!({"destination":destination})
            }
            Some(media_worker::Outcome::Failed { message }) => {
                json!({"error":{"code":"export-failed","message":message}})
            }
            Some(media_worker::Outcome::Cancelled) => json!({"cancelled":true}),
        };
        Ok(
            json!({"jobId":id,"status":status(phase),"publicationStarted":publication_started,"outcome":outcome,"facts":job.control.facts()}),
        )
    }
    pub fn dispatch(
        &mut self,
        context: CaptureContext<'_>,
        owner: u64,
        method: &str,
        params: &Value,
    ) -> Result<Value, RpcError> {
        self.poll();
        if !self.books.contains_key(&owner) {
            return Err(RpcError::new("disabled", "media owner unavailable"));
        }
        if method == "jobs.state" {
            return self.job_state(owner, id(params, "jobId")?);
        }
        if method == "snapshot.state" {
            let id = id(params, "snapshotId")?;
            return self
                .snapshots
                .get(&id)
                .filter(|s| s.owner == owner)
                .map(|s| s.summary.clone())
                .ok_or_else(|| RpcError::new("unknown-expired", "snapshot unavailable"));
        }
        if method == "jobs.operation" {
            let operation = id(params, "operationId")?;
            return Ok(match self.books[&owner].query(&key(), operation) {
                operations::Query::Pending => json!({"status":"pending"}),
                operations::Query::Unknown => json!({"status":"unknown"}),
                operations::Query::UnknownExpired => json!({"status":"unknownExpired"}),
                operations::Query::Finished(Ok(result)) => {
                    json!({"status":"completed","result":result})
                }
                operations::Query::Finished(Err(error)) => {
                    json!({"status":"failed","error":{"code":error.code,"message":error.message}})
                }
            });
        }
        let mut params = params
            .as_object()
            .cloned()
            .ok_or_else(|| RpcError::invalid("object required"))?;
        params.insert("method".into(), json!(method));
        let mutation: Mutation = serde_json::from_value(Value::Object(params))
            .map_err(|e| RpcError::invalid(e.to_string()))?;
        let operation = mutation.id();
        if !safe(operation) {
            return Err(RpcError::invalid("positive safe operationId required"));
        }
        match &mutation {
            Mutation::Capture {
                expected,
                time,
                mode,
                expected_processing_revision,
                expected_subtitle,
                include_subtitles,
                ..
            } => {
                if !safe(expected.generation)
                    || !safe(expected.intent_version)
                    || time.is_some_and(|t| !t.is_finite() || t < 0.)
                {
                    return Err(RpcError::invalid(
                        "valid source stamp and finite nonnegative time required",
                    ));
                }
                if *mode == Mode::Processed
                    && (expected_processing_revision.is_none_or(|r| r > Book::MAX_REVISION)
                        || (*include_subtitles
                            && expected_subtitle.is_none_or(|c| {
                                !safe(c.selection.generation)
                                    || c.selection.revision > Book::MAX_REVISION
                                    || c.presentation_revision > Book::MAX_REVISION
                            })))
                {
                    return Err(RpcError::invalid(
                        "processed snapshot requires processing and subtitle conditions",
                    ));
                }
            }
            Mutation::Release { snapshot_id, .. } if !safe(*snapshot_id) => {
                return Err(RpcError::invalid("snapshotId required"));
            }
            Mutation::Start {
                snapshot_id,
                kind,
                destination,
                end,
                sample_time,
                quality,
                ..
            } => {
                if !safe(*snapshot_id)
                    || (still(kind).is_none()
                        && !matches!(kind.as_str(), "rawMp4" | "processedMp4"))
                    || (quality.is_some() && !kind.ends_with("Jpeg"))
                    || quality.is_some_and(|quality| !(1..=100).contains(&quality))
                    || (kind.ends_with("Mp4")
                        && end.is_none_or(|v| !v.is_finite() || v <= 0. || v > 86400.))
                    || (!kind.ends_with("Mp4") && end.is_some())
                    || (kind.ends_with("Mp4") && sample_time.is_some())
                    || sample_time
                        .is_some_and(|time| !time.is_finite() || time < 0. || time > 86400.)
                    || destination.is_empty()
                    || destination.len() > 4096
                    || destination.contains('\0')
                    || !PathBuf::from(destination).is_absolute()
                {
                    return Err(RpcError::invalid(
                        "frame or MP4 kind, snapshotId, bounded absolute destination and JPEG quality 1–100 required",
                    ));
                }
            }
            Mutation::Cancel { job_id, .. } if !safe(*job_id) => {
                return Err(RpcError::invalid("jobId required"));
            }
            _ => (),
        }
        let payload =
            serde_json::to_value(&mutation).map_err(|e| RpcError::invalid(e.to_string()))?;
        match self
            .books
            .get_mut(&owner)
            .unwrap()
            .admit(key(), operation, payload)
        {
            Admission::Execute => (),
            Admission::Replay(result) => {
                return result.map_err(|e| RpcError::new(e.code, e.message));
            }
            Admission::Pending => return Err(RpcError::new("pending", "query jobs.operation")),
            Admission::Conflict => {
                return Err(RpcError::new("conflict", "media operation token conflict"));
            }
            Admission::UnknownExpired => {
                return Err(RpcError::new("unknown-expired", "media operation expired"));
            }
            Admission::Full => {
                return Err(RpcError::new("busy", "media operation capacity reached"));
            }
        }
        let result = self.mutate(context, owner, mutation);
        let outcome = result.as_ref().cloned().map_err(|e| operations::Failure {
            code: e.code.clone(),
            message: e.message.clone(),
        });
        self.books
            .get_mut(&owner)
            .unwrap()
            .finish(&key(), operation, outcome)
            .map_err(|e| RpcError::new("internal", e.message))?;
        result
    }
    fn mutate(
        &mut self,
        context: CaptureContext<'_>,
        owner: u64,
        mutation: Mutation,
    ) -> Result<Value, RpcError> {
        let CaptureContext {
            sources,
            mpv,
            geometry,
            processing,
            subtitles,
        } = context;
        match mutation {
            Mutation::Capture {
                expected,
                time,
                mode,
                expected_processing_revision,
                expected_subtitle,
                include_subtitles,
                audio,
                ..
            } => {
                let source = sources.snapshot(mpv);
                if source["stamp"] != json!(expected) {
                    return Err(RpcError::new("stale", "source intent changed"));
                }
                let lease = sources.lease(expected.generation)?;
                let time = time
                    .or_else(|| source["position"].as_f64())
                    .ok_or_else(|| {
                        RpcError::new("source-unavailable", "source time unavailable")
                    })?;
                if source["duration"]
                    .as_f64()
                    .is_some_and(|duration| time >= duration)
                {
                    return Err(RpcError::invalid("snapshot time must be before source end"));
                }
                if self.snapshots.len() >= 16
                    || self.snapshots.values().filter(|s| s.owner == owner).count() >= 4
                {
                    return Err(RpcError::new(
                        "busy",
                        "snapshot capacity reached; release a handle",
                    ));
                }
                if self.next_snapshot >= Book::MAX_REVISION {
                    return Err(RpcError::new(
                        "busy",
                        "snapshot identity capacity exhausted",
                    ));
                }
                let frozen = if mode == Mode::Processed {
                    let load = sources.presentation_load().ok_or_else(|| {
                        RpcError::new("unavailable", "actual source load unavailable")
                    })?;
                    if !geometry.ready_for(load) {
                        return Err(RpcError::new(
                            "unavailable",
                            "source presentation geometry is preparing",
                        ));
                    }
                    let graph = processing.freeze(
                        expected_processing_revision
                            .ok_or_else(|| RpcError::invalid("processing revision required"))?,
                        load,
                        mpv,
                    )?;
                    let subtitle = if include_subtitles {
                        let condition = expected_subtitle
                            .ok_or_else(|| RpcError::invalid("subtitle condition required"))?;
                        subtitles.freeze(
                            condition.selection,
                            condition.presentation_revision,
                            load,
                            mpv,
                        )?
                    } else {
                        None
                    };
                    let width = mpv
                        .get_property::<i64>("video-out-params/w")
                        .map_err(|e| RpcError::new("unsupported", e.to_string()))?;
                    let height = mpv
                        .get_property::<i64>("video-out-params/h")
                        .map_err(|e| RpcError::new("unsupported", e.to_string()))?;
                    let par = mpv
                        .get_property::<f64>("video-out-params/par")
                        .map_err(|e| RpcError::new("unsupported", e.to_string()))?;
                    if width <= 0 || height <= 0 || !par.is_finite() || par <= 0. {
                        return Err(RpcError::new(
                            "unavailable",
                            "invalid confirmed presentation dimensions",
                        ));
                    }
                    let display_width = (width as f64 * par).round();
                    if !(1.0..=16384.).contains(&display_width)
                        || height > 16384
                        || display_width * height as f64 > 64_000_000.
                    {
                        return Err(RpcError::new(
                            "unsupported",
                            "processed image size exceeds limits",
                        ));
                    }
                    let audio_delay_ms = f64::from(graph.audio_offset_ms);
                    Some(Arc::new(FrozenProcessed {
                        processing: graph,
                        subtitle,
                        width: display_width as u32,
                        height: height as u32,
                        audio_delay_ms,
                    }))
                } else {
                    None
                };
                let video_stream = crate::media_tracks::video_stream(mpv)
                    .map_err(|e| RpcError::new("unsupported", e.to_string()))?;
                let audio_tracks = crate::media_tracks::audio_tracks(mpv)
                    .map_err(|e| RpcError::new("unsupported", e.to_string()))?;
                let demuxer = mpv
                    .get_property::<String>("current-demuxer")
                    .map_err(|e| RpcError::new("unsupported", e.to_string()))?;
                let audio = crate::media_tracks::resolve(&audio_tracks, &demuxer, &audio)
                    .map_err(|e| RpcError::new("unsupported", e.to_string()))?;
                let duration = source["duration"]
                    .as_f64()
                    .filter(|d| d.is_finite() && *d > 0.)
                    .ok_or_else(|| {
                        RpcError::new("unsupported", "finite source duration required")
                    })?;
                let source_origin = mpv
                    .get_property::<f64>("demuxer-start-time")
                    .map_err(|e| RpcError::new("unsupported", e.to_string()))?;
                if !source_origin.is_finite() {
                    return Err(RpcError::new("unsupported", "source origin unavailable"));
                }
                self.next_snapshot += 1;
                let id = self.next_snapshot;
                let summary = json!({"snapshotId":id,"kind":mode,"sourceStamp":expected,"time":time,"source":source["source"],"container":source["container"],"sizeBytes":lease.facts().len,"timeline":{"domain":"mpv-media","validation":"pending-worker-origin-match","containerStartTime":source_origin},"duration":duration,"videoStreamIndex":video_stream,"audio":audio,"includeSubtitles":mode==Mode::Processed && include_subtitles,"preroll":if mode==Mode::Processed{Some(crate::offline_audio::PREROLL)}else{None},"processing":frozen.as_ref().map(|f|&f.processing),"subtitle":frozen.as_ref().map(|f|f.subtitle.as_ref().map(|s|&s.metadata)),"presentation":frozen.as_ref().map(|f|json!({"width":f.width,"height":f.height})),"audioOffsetMs":frozen.as_ref().map(|f|f.audio_delay_ms)});
                self.snapshots.insert(
                    id,
                    Arc::new(Snapshot {
                        owner,
                        lease,
                        time,
                        processed: frozen,
                        audio,
                        video_stream,
                        source_origin,
                        duration,
                        summary: summary.clone(),
                    }),
                );
                Ok(summary)
            }
            Mutation::Release { snapshot_id, .. } => {
                if self
                    .snapshots
                    .get(&snapshot_id)
                    .is_some_and(|s| s.owner == owner)
                {
                    self.snapshots.remove(&snapshot_id);
                }
                Ok(Value::Null)
            }
            Mutation::Start {
                snapshot_id,
                destination,
                kind,
                end,
                sample_time,
                quality,
                ..
            } => {
                let snapshot = self
                    .snapshots
                    .get(&snapshot_id)
                    .filter(|s| s.owner == owner)
                    .cloned()
                    .ok_or_else(|| RpcError::new("unknown-expired", "snapshot unavailable"))?;
                if kind.starts_with("processed") != snapshot.processed.is_some() {
                    return Err(RpcError::invalid(
                        "job kind must match frozen snapshot mode",
                    ));
                }
                let sample_time = sample_time.unwrap_or(snapshot.time);
                if sample_time >= snapshot.duration {
                    return Err(RpcError::invalid(
                        "sampleTime must be before frozen source end",
                    ));
                }
                if let Some(end) = end
                    && (end <= snapshot.time || end > snapshot.duration)
                {
                    return Err(RpcError::invalid(
                        "MP4 end must be after frozen start and within frozen duration",
                    ));
                }
                let output = if kind.ends_with("Mp4") {
                    media_worker::Output::Mp4 {
                        end: end.ok_or_else(|| RpcError::invalid("MP4 end required"))?,
                        audio: snapshot.audio.clone(),
                    }
                } else {
                    let format = still(&kind).ok_or_else(|| RpcError::invalid("unknown kind"))?;
                    media_worker::Output::Still(match format {
                        media_worker::Still::Jpeg { .. } => media_worker::Still::Jpeg {
                            quality: quality.unwrap_or(90),
                        },
                        other => other,
                    })
                };
                if self.workers.len() >= 8
                    || self
                        .jobs
                        .values()
                        .filter(|j| j.owner == owner && !terminal(j.control.status()))
                        .count()
                        >= 2
                {
                    return Err(RpcError::new("busy", "worker capacity reached"));
                }
                while self.jobs.len() >= 128 {
                    let old = self
                        .jobs
                        .iter()
                        .find(|(_, j)| terminal(j.control.status()))
                        .map(|(id, _)| *id);
                    if let Some(id) = old {
                        self.jobs.remove(&id);
                    } else {
                        return Err(RpcError::new("busy", "job records full"));
                    }
                }
                if self.next_job >= Book::MAX_REVISION {
                    return Err(RpcError::new("busy", "job identity capacity exhausted"));
                }
                // `$VESTRA_FFMPEG`, the bundled one, else `PATH` in a
                // development build (`runtime_paths.rs`).
                let Some(binary) = crate::runtime_paths::get()
                    .ffmpeg
                    .clone()
                    .filter(|binary| binary.is_file())
                else {
                    return Err(RpcError::new(
                        "unavailable",
                        "validated FFmpeg binary unavailable",
                    ));
                };
                self.next_job += 1;
                let id = self.next_job;
                let control = Arc::new(Control::new());
                let worker_control = control.clone();
                let worker = std::thread::Builder::new()
                    .name(format!("media-job-{id}"))
                    .spawn(move || {
                        media_worker::run(
                            media_worker::RenderRequest {
                                lease: snapshot.lease.clone(),
                                time: sample_time,
                                destination: PathBuf::from(destination),
                                ffmpeg: binary,
                                processed: snapshot.processed.clone(),
                                output,
                                video_stream: snapshot.video_stream,
                                source_origin: snapshot.source_origin,
                            },
                            worker_control,
                        )
                    })
                    .map_err(|error| {
                        RpcError::new(
                            "unavailable",
                            format!("media worker could not start: {error}"),
                        )
                    })?;
                self.jobs.insert(
                    id,
                    Job {
                        owner,
                        control: control.clone(),
                    },
                );
                self.workers.push(worker);
                Ok(json!({"jobId":id,"status":"queued","snapshotId":snapshot_id}))
            }
            Mutation::Cancel { job_id, .. } => {
                let job = self
                    .jobs
                    .get(&job_id)
                    .filter(|j| j.owner == owner)
                    .ok_or_else(|| RpcError::new("unknown-expired", "job unavailable"))?;
                let result = match job.control.cancel() {
                    media_worker::CancelResult::Requested => "requested",
                    media_worker::CancelResult::TooLate => "tooLate",
                    media_worker::CancelResult::Terminal => "terminal",
                };
                Ok(json!({"jobId":job_id,"cancel":result}))
            }
        }
    }
}
impl Drop for Jobs {
    fn drop(&mut self) {
        for job in self.jobs.values() {
            job.control.cancel();
        }
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_kinds_name_their_still_format() {
        assert_eq!(still("rawPng"), Some(media_worker::Still::Png));
        assert_eq!(still("processedBmp"), Some(media_worker::Still::Bmp));
        assert_eq!(
            still("rawJpeg"),
            Some(media_worker::Still::Jpeg { quality: 90 })
        );
        for other in ["rawMp4", "rawGif", "Png", "processedpng"] {
            assert_eq!(still(other), None, "{other}");
        }
    }
}
