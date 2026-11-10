//! `media.sources`: persistent references to local files the user opened
//! (host-internal store; runs on the playback worker and short threads).
//!
//! The host mints an opaque token for the current source when a plugin asks
//! (`media.sources.reference`); only a local regular file qualifies, never a
//! stream or a plugin's own picture. The token is the plugin's key into a
//! table the host keeps in the profile (`media-sources.json`, keyed by the
//! plugin's data key): the path, its size and modification time when it was
//! added (or located), its display name and the probed stream facts. A
//! plugin never sees the path or the file's content, only the name and the
//! facts; tokens of one plugin mean nothing to another. Tokens survive
//! restarts and are deleted when the plugin is uninstalled. A file that is no
//! longer at its path is `missing`; one whose size or modification time
//! differs is `changed` (same-length rewrites with a preserved time are not
//! seen, as for snapshot leases). `media.sources.locate` lets the host ask
//! the user for the file again and rebinds the token to the choice.
//!
//! `jobs.check` and `jobs.start { parts }` (media_join.rs) take these tokens.
#![forbid(unsafe_code)]

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::host::{Event, Reply, RpcError};
use crate::playback::media_worker::{Control, Encoding, capture_input_command, sibling};
use crate::playback::source_lease::Lease;

/// References one plugin may hold (Chrome-style per-extension bound).
pub const MAX_PER_PLUGIN: usize = 500;
/// Parts of one join.
pub const MAX_PARTS: usize = 100;
/// Requests served at once on short threads (probes, checks).
const MAX_IN_FLIGHT: usize = 16;
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
/// One plugin's share of them, so it cannot starve the others.
const MAX_IN_FLIGHT_PER_PLUGIN: usize = 4;
static IN_FLIGHT_BY_STORE: Mutex<BTreeMap<String, usize>> = Mutex::new(BTreeMap::new());
/// The parts' total length, as one clip's (24 hours).
pub const MAX_TOTAL_SECONDS: f64 = 86400.;

/// A video stream's facts, as probed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoFacts {
    pub index: u32,
    pub codec: String,
    #[serde(default)]
    pub profile: Option<String>,
    pub width: u32,
    pub height: u32,
    /// Sample aspect ratio `n:d` (`1:1` when unknown).
    pub sar: (u32, u32),
    /// `avg_frame_rate` (else `r_frame_rate`) as `n/d`.
    pub rate: (u32, u32),
    #[serde(default)]
    pub pixel_format: Option<String>,
    /// SHA-256 of the codec's extradata (SPS/PPS…), when it has any.
    #[serde(default)]
    pub extradata: Option<String>,
}
impl VideoFacts {
    pub fn fps(&self) -> f64 {
        f64::from(self.rate.0) / f64::from(self.rate.1)
    }
    /// The width of square pixels (`width × sar`), at least 1.
    pub fn display_width(&self) -> u32 {
        let (n, d) = self.sar;
        ((f64::from(self.width) * f64::from(n) / f64::from(d)).round() as u32).max(1)
    }
}

/// An audio stream's facts, as probed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioFacts {
    pub index: u32,
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u32,
}

/// What the host knows of a referenced file's streams.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Facts {
    pub duration: f64,
    /// The container's start time (normalized time 0).
    #[serde(default)]
    pub origin: f64,
    pub format: String,
    pub video: Option<VideoFacts>,
    pub audio: Option<AudioFacts>,
}

/// One reference.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub path: PathBuf,
    pub len: u64,
    /// Modification time, nanoseconds since the Unix epoch.
    pub modified: u64,
    pub name: String,
    pub facts: Facts,
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    version: u32,
    /// Data key (`instance:<guid>`, `development:<id>`) → token → entry.
    stores: BTreeMap<String, BTreeMap<String, Entry>>,
}

/// The references of every plugin, saved in the profile.
pub struct Store {
    saved: Saved,
    /// Writes the file on a thread of its own (`None`: kept in memory only:
    /// tests, no application data directory). A change only serializes the
    /// table under the lock; the write and its fsync happen elsewhere, the
    /// newest table winning when several wait.
    writer: Option<Writer>,
}

enum Persist {
    Write(Vec<u8>),
    /// Tests: answered once everything sent before is on disk.
    #[cfg_attr(not(test), allow(dead_code))]
    Flush(mpsc::Sender<()>),
}
struct Writer {
    tx: Option<mpsc::Sender<Persist>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Writer {
    fn start(file: PathBuf) -> Option<Self> {
        let (tx, rx) = mpsc::channel::<Persist>();
        let thread = std::thread::Builder::new()
            .name("media-sources-save".into())
            .spawn(move || {
                while let Ok(first) = rx.recv() {
                    let mut latest = None;
                    let mut acks = Vec::new();
                    let mut next = Some(first);
                    // Everything waiting now: only the newest table is written.
                    while let Some(message) = next {
                        match message {
                            Persist::Write(bytes) => latest = Some(bytes),
                            Persist::Flush(ack) => acks.push(ack),
                        }
                        next = rx.try_recv().ok();
                    }
                    if let Some(bytes) = latest {
                        write_file(&file, &bytes);
                    }
                    for ack in acks {
                        let _ = ack.send(());
                    }
                }
            })
            .map_err(|error| diag!("media sources: no writer thread: {error}"))
            .ok()?;
        Some(Self {
            tx: Some(tx),
            thread: Some(thread),
        })
    }
}
impl Drop for Writer {
    /// The pending table is written before the store goes (player exit).
    fn drop(&mut self) {
        drop(self.tx.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
/// The whole table, to a temporary file then renamed over the old one.
fn write_file(file: &Path, bytes: &[u8]) {
    let write = || -> Result<()> {
        use std::io::Write;
        let directory = file.parent().context("no directory")?;
        std::fs::create_dir_all(directory)?;
        let mut temp = tempfile::NamedTempFile::new_in(directory)?;
        temp.write_all(bytes)?;
        temp.as_file().sync_all()?;
        temp.persist(file)?;
        Ok(())
    };
    if let Err(error) = write() {
        diag!("media sources: the references could not be saved: {error:#}");
    }
}
impl Default for Store {
    fn default() -> Self {
        Self::open(None)
    }
}
pub type Shared = Arc<Mutex<Store>>;

/// `ok`, `missing` or `changed`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    Missing,
    Changed,
}
impl Status {
    pub fn name(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Missing => "missing",
            Self::Changed => "changed",
        }
    }
}

/// Nanoseconds since the Unix epoch (0 before it).
pub fn nanos(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
}

impl Entry {
    /// Whether the file is still at its path, as it was.
    pub fn status(&self) -> Status {
        match std::fs::metadata(&self.path) {
            Ok(meta) if meta.is_file() => {
                let modified = meta.modified().map(nanos).unwrap_or(0);
                if meta.len() == self.len && modified == self.modified {
                    Status::Ok
                } else {
                    Status::Changed
                }
            }
            _ => Status::Missing,
        }
    }
    /// What a plugin sees: never the path.
    pub fn public(&self, token: &str, status: Status) -> Value {
        json!({
            "token": token,
            "name": self.name,
            "duration": self.facts.duration,
            "status": status.name(),
            "video": self.facts.video.as_ref().map(|v| json!({
                "codec": v.codec,
                "width": v.width,
                "height": v.height,
                "fps": v.fps(),
                "pixelFormat": v.pixel_format,
            })),
            "audio": self.facts.audio.as_ref().map(|a| json!({
                "codec": a.codec,
                "sampleRate": a.sample_rate,
                "channels": a.channels,
            })),
        })
    }
}

impl Store {
    /// The profile's store (`<app data>/media-sources.json`), or an empty
    /// one in memory without an application data directory. A file that
    /// cannot be read starts empty (and is replaced at the next change).
    /// A development load's references end with the run that loaded it
    /// (`--plugin`): the next launch starts without them.
    pub fn open_default() -> Self {
        let mut store =
            Self::open(crate::platform::app_data_dir().map(|dir| dir.join("media-sources.json")));
        store.end_development_loads();
        store
    }
    /// Drops every `development:<id>` store.
    pub fn end_development_loads(&mut self) {
        let before = self.saved.stores.len();
        self.saved
            .stores
            .retain(|key, _| !key.starts_with("development:"));
        if self.saved.stores.len() != before {
            self.save();
        }
    }
    /// Waits until every change so far is on disk (tests; at shutdown the
    /// writer is joined instead).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn flush(&self) {
        if let Some(tx) = self.writer.as_ref().and_then(|w| w.tx.as_ref()) {
            let (ack, done) = mpsc::channel();
            if tx.send(Persist::Flush(ack)).is_ok() {
                let _ = done.recv();
            }
        }
    }
    pub fn open(file: Option<PathBuf>) -> Self {
        let saved = file
            .as_deref()
            .and_then(|file| std::fs::read(file).ok())
            .and_then(|bytes| match serde_json::from_slice::<Saved>(&bytes) {
                Ok(saved) => Some(saved),
                Err(error) => {
                    diag!("media sources: the saved references are unreadable: {error}");
                    None
                }
            })
            .unwrap_or_default();
        Self {
            saved,
            writer: file.and_then(Writer::start),
        }
    }
    /// Hands the table to the writer: no file system call under the lock.
    fn save(&self) {
        let Some(tx) = self.writer.as_ref().and_then(|w| w.tx.as_ref()) else {
            return;
        };
        match serde_json::to_vec(&self.saved) {
            Ok(bytes) => {
                let _ = tx.send(Persist::Write(bytes));
            }
            Err(error) => diag!("media sources: the references could not be written: {error}"),
        }
    }
    pub fn get(&self, key: &str, token: &str) -> Option<&Entry> {
        self.saved.stores.get(key)?.get(token)
    }
    pub fn list(&self, key: &str) -> Vec<(String, Entry)> {
        self.saved
            .stores
            .get(key)
            .map(|store| {
                store
                    .iter()
                    .map(|(token, entry)| (token.clone(), entry.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }
    /// A new reference, or the existing one for the same file as it is.
    pub fn insert(&mut self, key: &str, entry: Entry) -> Result<String, RpcError> {
        let store = self.saved.stores.entry(key.to_owned()).or_default();
        if let Some((token, existing)) = store.iter_mut().find(|(_, e)| {
            e.path == entry.path && e.len == entry.len && e.modified == entry.modified
        }) {
            *existing = entry;
            let token = token.clone();
            self.save();
            return Ok(token);
        }
        if store.len() >= MAX_PER_PLUGIN {
            return Err(RpcError::new(
                "limit",
                format!("a plugin holds at most {MAX_PER_PLUGIN} source references; forget one"),
            ));
        }
        let token = format!("src-{}", uuid::Uuid::new_v4().simple());
        store.insert(token.clone(), entry);
        self.save();
        Ok(token)
    }
    /// The token now names `entry` (the user located the file).
    pub fn rebind(&mut self, key: &str, token: &str, entry: Entry) -> Result<(), RpcError> {
        let slot = self
            .saved
            .stores
            .get_mut(key)
            .and_then(|store| store.get_mut(token))
            .ok_or_else(unknown)?;
        *slot = entry;
        self.save();
        Ok(())
    }
    pub fn forget(&mut self, key: &str, token: &str) {
        if let Some(store) = self.saved.stores.get_mut(key)
            && store.remove(token).is_some()
        {
            if store.is_empty() {
                self.saved.stores.remove(key);
            }
            self.save();
        }
    }
    /// Every reference of a plugin goes (it was uninstalled).
    pub fn revoke(&mut self, key: &str) {
        if self.saved.stores.remove(key).is_some() {
            self.save();
        }
    }
}

/// The store of a plugin: its data key (an installed instance or a
/// development load), as `media-sources.json` names it.
pub fn store_key(key: &crate::process::DataKey) -> String {
    match key {
        crate::process::DataKey::Instance(guid) => format!("instance:{guid}"),
        crate::process::DataKey::Development(id) => format!("development:{id}"),
    }
}

fn unknown() -> RpcError {
    RpcError::new("missing", "no such source reference")
}

/// The file's display name: its last component.
pub fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// `n/d` or `n:d` as positive integers.
fn ratio(value: &Value, separator: char) -> Option<(u32, u32)> {
    let (n, d) = value.as_str()?.split_once(separator)?;
    let (n, d) = (n.parse::<u32>().ok()?, d.parse::<u32>().ok()?);
    (n > 0 && d > 0).then_some((n, d))
}

/// The facts from ffprobe's JSON: the default (else first) video stream
/// that is not a cover image, and the default (else first) audio stream.
pub fn facts_from(probed: &Value) -> Result<Facts, RpcError> {
    let unsupported = |message: &str| RpcError::new("unsupported", message.to_owned());
    let streams = probed["streams"].as_array().cloned().unwrap_or_default();
    let pick = |kind: &str| -> Option<Value> {
        let of_kind: Vec<&Value> = streams
            .iter()
            .filter(|s| s["codec_type"] == kind)
            .filter(|s| s["disposition"]["attached_pic"].as_i64() != Some(1))
            .collect();
        of_kind
            .iter()
            .find(|s| s["disposition"]["default"].as_i64() == Some(1))
            .or(of_kind.first())
            .map(|s| (*s).clone())
    };
    let text = |v: &Value| {
        v.as_str()
            .filter(|s| !s.is_empty() && *s != "unknown")
            .map(str::to_owned)
    };
    let video = pick("video").and_then(|v| {
        let rate = ratio(&v["avg_frame_rate"], '/').or(ratio(&v["r_frame_rate"], '/'))?;
        Some(VideoFacts {
            index: u32::try_from(v["index"].as_u64()?).ok()?,
            codec: text(&v["codec_name"])?,
            profile: text(&v["profile"]),
            width: u32::try_from(v["width"].as_u64()?)
                .ok()
                .filter(|w| *w > 0)?,
            height: u32::try_from(v["height"].as_u64()?)
                .ok()
                .filter(|h| *h > 0)?,
            sar: ratio(&v["sample_aspect_ratio"], ':').unwrap_or((1, 1)),
            rate,
            pixel_format: text(&v["pix_fmt"]),
            extradata: text(&v["extradata_hash"]),
        })
    });
    let audio = pick("audio").and_then(|a| {
        Some(AudioFacts {
            index: u32::try_from(a["index"].as_u64()?).ok()?,
            codec: text(&a["codec_name"])?,
            sample_rate: a["sample_rate"].as_str()?.parse().ok()?,
            channels: u32::try_from(a["channels"].as_u64()?).ok()?,
        })
    });
    let number = |v: &Value| {
        v.as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|v| v.is_finite())
    };
    let duration = number(&probed["format"]["duration"])
        .filter(|d| *d > 0.)
        .ok_or_else(|| unsupported("the file's duration is unknown"))?;
    if video.is_none() && audio.is_none() {
        return Err(unsupported("the file has no audio or video stream"));
    }
    Ok(Facts {
        duration,
        origin: number(&probed["format"]["start_time"]).unwrap_or(0.),
        format: probed["format"]["format_name"]
            .as_str()
            .unwrap_or("")
            .to_owned(),
        video,
        audio,
    })
}

/// ffprobe's entries for [`facts_from`].
pub const PROBE_ENTRIES: &str = "format=duration,start_time,format_name,bit_rate:stream=index,codec_type,codec_name,profile,bit_rate,width,height,sample_aspect_ratio,pix_fmt,avg_frame_rate,r_frame_rate,time_base,sample_rate,channels,extradata_hash:stream_disposition=default,attached_pic";

/// Probe a held file (its stdin is the lease): the facts, or `None` when
/// cancelled.
pub fn probe(ffmpeg: &Path, lease: &Lease, control: &Control) -> Result<Option<Value>> {
    let mut reader = lease.lock_reader()?;
    let mut command = crate::platform::tool_command(sibling(ffmpeg, "ffprobe")?);
    command.args([
        "-v",
        "error",
        "-show_data_hash",
        "SHA256",
        "-show_entries",
        PROBE_ENTRIES,
        "-of",
        "json",
        crate::platform::HELD_FILE_INPUT,
    ]);
    let Some((out, _)) = capture_input_command(&mut command, reader.stdin()?, control)? else {
        return Ok(None);
    };
    reader.validate()?;
    Ok(Some(
        serde_json::from_slice(&out).context("source probe JSON invalid")?,
    ))
}

/// Open and probe `path` as a new entry.
fn entry_of(ffmpeg: &Path, path: &Path) -> Result<Entry, RpcError> {
    let lease = Lease::open(path).map_err(|e| {
        RpcError::new(
            "source-unavailable",
            format!("the file cannot be read: {e:#}"),
        )
    })?;
    let control = Control::new();
    let probed = probe(ffmpeg, &lease, &control)
        .map_err(|e| RpcError::new("unsupported", format!("the file cannot be probed: {e:#}")))?
        .ok_or_else(|| RpcError::new("internal", "probe cancelled"))?;
    let facts = facts_from(&probed)?;
    let accepted = lease.facts();
    Ok(Entry {
        path: path.to_path_buf(),
        len: accepted.len,
        modified: nanos(accepted.modified),
        name: display_name(path),
        facts,
    })
}

/// One part of a join, as a plugin sends it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Part {
    pub source: String,
    pub start: f64,
    pub end: f64,
}

/// The parts' size and kind (1..=100, finite times, 24 hours in all): the
/// shape only.
pub fn parts_valid(parts: &[Part]) -> bool {
    (1..=MAX_PARTS).contains(&parts.len())
        && parts.iter().map(|p| (p.end - p.start).max(0.)).sum::<f64>() <= MAX_TOTAL_SECONDS
        && parts.iter().all(|p| {
            !p.source.is_empty()
                && p.source.len() <= 64
                && p.start.is_finite()
                && p.end.is_finite()
                && p.start >= 0.
                && p.end <= 86400.
        })
}

/// What a re-encoded join writes: the first part's square-pixel size (even)
/// and rate, under the encoding's ceilings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub width: u32,
    pub height: u32,
    pub rate: (u32, u32),
}
fn even(value: u32) -> u32 {
    value.div_ceil(2) * 2
}
pub fn target(first: &VideoFacts, encoding: &Encoding) -> Target {
    let (mut width, mut height) = (first.display_width(), first.height);
    if let Some(max) = encoding.max_height
        && height > max
    {
        width = ((f64::from(width) * f64::from(max) / f64::from(height)).round() as u32).max(1);
        height = max;
    }
    let mut rate = first.rate;
    if let Some(cap) = encoding.fps
        && first.fps() > f64::from(cap) * 1.001
    {
        rate = (cap, 1);
    }
    Target {
        width: even(width),
        height: even(height),
        rate,
    }
}

/// A part resolved against the store: its entry and status (`None`: an
/// unknown token).
pub struct Resolved {
    pub entry: Option<Entry>,
    pub status: Status,
}

fn describe_audio(audio: Option<&AudioFacts>) -> String {
    audio.map_or_else(
        || "none".into(),
        |a| format!("{} {} Hz {} ch", a.codec, a.sample_rate, a.channels),
    )
}

/// `jobs.check`: what blocks an export of parts, whether a stream copy can join the
/// parts (and if not, why), and what a re-encode writes. Parts are numbered
/// from 0 in the order given.
pub fn check(parts: &[Part], resolved: &[Resolved], encoding: &Encoding) -> Value {
    let mut blocking = Vec::new();
    for (index, (part, resolved)) in parts.iter().zip(resolved).enumerate() {
        let Some(entry) = &resolved.entry else {
            blocking.push(json!({"part":index,"problem":"unknown"}));
            continue;
        };
        match resolved.status {
            Status::Missing => blocking.push(json!({"part":index,"problem":"missing"})),
            Status::Changed => blocking.push(json!({"part":index,"problem":"changed"})),
            Status::Ok => (),
        }
        let Some(video) = &entry.facts.video else {
            blocking.push(json!({"part":index,"problem":"noVideo"}));
            continue;
        };
        let frame = 1. / video.fps();
        if part.end <= part.start
            || part.end - part.start < frame - 1e-6
            || part.end > entry.facts.duration + 0.001
        {
            blocking.push(json!({"part":index,"problem":"range","duration":entry.facts.duration}));
        }
    }
    let videos: Vec<Option<(&VideoFacts, Option<&AudioFacts>)>> = resolved
        .iter()
        .map(|r| {
            r.entry
                .as_ref()
                .and_then(|e| Some((e.facts.video.as_ref()?, e.facts.audio.as_ref())))
        })
        .collect();
    let mut reasons = Vec::new();
    let first = videos.first().copied().flatten();
    if let Some((first, first_audio)) = first {
        for (index, other) in videos.iter().enumerate().skip(1) {
            let Some((video, audio)) = other else {
                continue;
            };
            let mut differs = |field: &str, value: Value, expected: Value| {
                reasons.push(json!({"part":index,"field":field,"value":value,"expected":expected}));
            };
            if video.codec != first.codec {
                differs("videoCodec", json!(video.codec), json!(first.codec));
            } else if video.profile != first.profile || video.extradata != first.extradata {
                differs("videoParams", json!(video.profile), json!(first.profile));
            }
            if (video.width, video.height) != (first.width, first.height) {
                differs(
                    "size",
                    json!(format!("{}×{}", video.width, video.height)),
                    json!(format!("{}×{}", first.width, first.height)),
                );
            }
            if (u64::from(video.rate.0) * u64::from(first.rate.1))
                != (u64::from(first.rate.0) * u64::from(video.rate.1))
            {
                differs("fps", json!(video.fps()), json!(first.fps()));
            }
            if video.pixel_format != first.pixel_format {
                differs(
                    "pixelFormat",
                    json!(video.pixel_format),
                    json!(first.pixel_format),
                );
            }
            let same_audio = match (audio, first_audio) {
                (None, None) => true,
                (Some(a), Some(b)) => {
                    (&a.codec, a.sample_rate, a.channels) == (&b.codec, b.sample_rate, b.channels)
                }
                _ => false,
            };
            if !same_audio {
                differs(
                    "audio",
                    json!(describe_audio(*audio)),
                    json!(describe_audio(first_audio)),
                );
            }
        }
    }
    let copy = if !reasons.is_empty() {
        json!({"ok":false,"reasons":reasons})
    } else if let Some((first, audio)) = first {
        let container =
            if crate::playback::media_copy::fits_mp4(&first.codec, audio.map(|a| a.codec.as_str()))
            {
                "mp4"
            } else {
                "mkv"
            };
        json!({"ok":true,"container":container})
    } else {
        json!({"ok":false,"reasons":[]})
    };
    let any_audio = videos.iter().flatten().any(|(_, audio)| audio.is_some());
    let encode = first.map(|(first, _)| {
        let target = target(first, encoding);
        json!({
            "width": target.width,
            "height": target.height,
            "fps": f64::from(target.rate.0) / f64::from(target.rate.1),
            "audio": any_audio.then(|| json!({"sampleRate":48000,"channels":2})),
        })
    });
    let duration: f64 = parts.iter().map(|p| (p.end - p.start).max(0.)).sum();
    json!({"blocking":blocking,"copy":copy,"encode":encode,"duration":duration})
}

/// Resolve parts against a plugin's store (statting each file).
pub fn resolve_parts(store: &Shared, key: &str, parts: &[Part]) -> Vec<Resolved> {
    let entries: Vec<Option<Entry>> = {
        let store = store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        parts
            .iter()
            .map(|p| store.get(key, &p.source).cloned())
            .collect()
    };
    entries
        .into_iter()
        .map(|entry| {
            let status = entry.as_ref().map_or(Status::Missing, Entry::status);
            Resolved { entry, status }
        })
        .collect()
}

/// The FFmpeg the jobs use (`$VESTRA_FFMPEG`, the bundled one, or `PATH`).
pub fn ffmpeg() -> Result<PathBuf, RpcError> {
    crate::process::runtime_paths::get()
        .ffmpeg
        .clone()
        .filter(|binary| binary.is_file())
        .ok_or_else(|| RpcError::new("unavailable", "validated FFmpeg binary unavailable"))
}

/// The store key the host put in a forwarded request.
fn key_of(params: &Value) -> Result<String, RpcError> {
    params["store"]
        .as_str()
        .filter(|k| !k.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| RpcError::new("internal", "source store unavailable"))
}
fn token_of(params: &Value) -> Result<String, RpcError> {
    params["token"]
        .as_str()
        .filter(|t| !t.is_empty() && t.len() <= 64)
        .map(str::to_owned)
        .ok_or_else(|| RpcError::invalid("token required"))
}

/// `media.sources.open { token, start?, operationId }`: the file a plugin's
/// token stands for, opened as any open (`source.submit` of an `open`
/// intent under the plugin's own operation id, so `source.operation`
/// follows it): its path (for the resume point that makes it start at
/// `start`), the start, and the submit's parameters. A missing or changed
/// file is refused (`source-missing`, `source-changed`) before anything
/// opens.
pub fn open_request(
    store: &Shared,
    params: &Value,
) -> Result<(String, Option<f64>, Value), RpcError> {
    let key = key_of(params)?;
    let token = token_of(params)?;
    let operation = params["operationId"]
        .as_u64()
        .filter(|id| *id > 0)
        .ok_or_else(|| RpcError::invalid("operationId requires positive integer"))?;
    let entry = store
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&key, &token)
        .cloned()
        .ok_or_else(unknown)?;
    let start = match &params["start"] {
        Value::Null => None,
        value => Some(
            value
                .as_f64()
                .filter(|s| s.is_finite() && *s >= 0. && *s < entry.facts.duration)
                .ok_or_else(|| {
                    RpcError::invalid("start must be seconds from 0 to before the file's end")
                })?,
        ),
    };
    match entry.status() {
        Status::Ok => (),
        Status::Missing => {
            return Err(RpcError::new(
                "source-missing",
                format!("{} is missing", entry.name),
            ));
        }
        Status::Changed => {
            return Err(RpcError::new(
                "source-changed",
                format!("{} changed since it was remembered", entry.name),
            ));
        }
    }
    let path = entry
        .path
        .to_str()
        .ok_or_else(|| RpcError::new("unsupported", "the file's path is not Unicode"))?
        .to_owned();
    let submit = json!({
        "operationId": operation,
        "intent": {"kind": "open", "source": path, "startPaused": false},
    });
    Ok((path, start.filter(|s| *s > 0.), submit))
}

/// The encoding of a check (`jobs.check`'s optional `encoding`).
pub fn check_encoding(params: &Value) -> Result<Encoding, RpcError> {
    match &params["encoding"] {
        Value::Null => Ok(Encoding::default()),
        value => match serde_json::from_value::<Encoding>(value.clone()) {
            Ok(encoding) if encoding.valid() => Ok(encoding),
            _ => Err(RpcError::invalid("invalid encoding")),
        },
    }
}

/// A snapshot's clip, as one part for [`check`].
pub struct SnapshotInput {
    pub lease: Arc<Lease>,
    pub start: f64,
    pub end: f64,
    /// A processed snapshot: its picture size (effects need a re-encode).
    pub presentation: Option<(u32, u32)>,
}

/// `jobs.check { snapshotId, end, encoding? }`: the check of a snapshot's
/// clip, as of one part: the held file is probed (on a short thread). A
/// processed snapshot cannot be copied (`effects`) and re-encodes at its
/// picture size.
pub fn check_snapshot(
    events: &mpsc::Sender<Event>,
    reply: Reply,
    store: String,
    input: SnapshotInput,
    encoding: Encoding,
) {
    let ffmpeg = match ffmpeg() {
        Ok(ffmpeg) => ffmpeg,
        Err(error) => {
            let _ = events.send(Event::Reply(reply, Err(error)));
            return;
        }
    };
    // The plugin's share of the request threads, as its parts checks.
    spawn(events, reply, store, move || {
        let probed = probe(&ffmpeg, &input.lease, &Control::new())
            .map_err(|e| RpcError::new("unsupported", format!("the file cannot be probed: {e:#}")))?
            .ok_or_else(|| RpcError::new("internal", "probe cancelled"))?;
        let facts = facts_from(&probed)?;
        let accepted = input.lease.facts();
        let entry = Entry {
            path: PathBuf::new(),
            len: accepted.len,
            modified: nanos(accepted.modified),
            name: String::new(),
            facts,
        };
        let part = Part {
            source: "snapshot".into(),
            start: input.start,
            end: input.end,
        };
        let mut checked = check(
            std::slice::from_ref(&part),
            &[Resolved {
                entry: Some(entry),
                status: Status::Ok,
            }],
            &encoding,
        );
        if let Some((width, height)) = input.presentation {
            checked["copy"] = json!({"ok":false,"reasons":[{"part":0,"field":"effects","value":null,"expected":null}]});
            if checked["encode"].is_object() {
                checked["encode"]["width"] = json!(width.div_ceil(2) * 2);
                checked["encode"]["height"] = json!(height.div_ceil(2) * 2);
            }
        }
        Ok(checked)
    });
}

/// Serve a request on a short thread and reply from it.
pub(super) fn spawn(
    events: &mpsc::Sender<Event>,
    reply: Reply,
    store: String,
    work: impl FnOnce() -> Result<Value, RpcError> + Send + 'static,
) {
    let busy = |events: &mpsc::Sender<Event>, reply: Reply| {
        let _ = events.send(Event::Reply(
            reply,
            Err(RpcError::new("busy", "too many source requests at once")),
        ));
    };
    {
        let mut by_store = IN_FLIGHT_BY_STORE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mine = by_store.entry(store.clone()).or_default();
        if *mine >= MAX_IN_FLIGHT_PER_PLUGIN {
            drop(by_store);
            return busy(events, reply);
        }
        if IN_FLIGHT.fetch_add(1, Ordering::AcqRel) >= MAX_IN_FLIGHT {
            IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
            drop(by_store);
            return busy(events, reply);
        }
        *mine += 1;
    }
    let release = move || {
        IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
        let mut by_store = IN_FLIGHT_BY_STORE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(mine) = by_store.get_mut(&store) {
            *mine = mine.saturating_sub(1);
            if *mine == 0 {
                by_store.remove(&store);
            }
        }
    };
    let release_failed = release.clone();
    let events = events.clone();
    let failed = events.clone();
    let failed_reply = reply.clone();
    let spawned = std::thread::Builder::new()
        .name("media-sources".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
                .unwrap_or_else(|_| Err(RpcError::new("internal", "source request panicked")));
            release();
            let _ = events.send(Event::Reply(reply, result));
        });
    if let Err(error) = spawned {
        release_failed();
        let _ = failed.send(Event::Reply(
            failed_reply,
            Err(RpcError::new("unavailable", error.to_string())),
        ));
    }
}

/// The current source, when it is a local regular file (`reference`).
pub struct Current {
    pub path: Option<PathBuf>,
    /// The source's stamp (`source.state`'s `stamp`).
    pub stamp: Value,
}

/// Serves `media.sources.*`, `jobs.check` and the host's own
/// `owner.sourceLocated` / `owner.sourcesRevoked`. Replies come from short
/// threads (`spawn`) or at once; `locate` asks the UI thread for the dialog.
pub fn request(
    store: &Shared,
    events: &mpsc::Sender<Event>,
    reply: Reply,
    method: &str,
    params: &Value,
    current: impl FnOnce() -> Current,
) {
    let answer = |result: Result<Value, RpcError>| {
        let _ = events.send(Event::Reply(reply.clone(), result));
    };
    if method == "owner.sourcesRevoked" {
        if let Ok(key) = key_of(params) {
            store
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .revoke(&key);
        }
        return;
    }
    let key = match key_of(params) {
        Ok(key) => key,
        Err(error) => return answer(Err(error)),
    };
    let store = store.clone();
    match method {
        "media.sources.reference" => {
            let current = current();
            if !params["expected"].is_null() && params["expected"] != current.stamp {
                return answer(Err(RpcError::new("stale", "source intent changed")));
            }
            let Some(path) = current.path else {
                return answer(Err(RpcError::new(
                    "unsupported",
                    "only a local file can be referenced",
                )));
            };
            let ffmpeg = match ffmpeg() {
                Ok(ffmpeg) => ffmpeg,
                Err(error) => return answer(Err(error)),
            };
            spawn(events, reply, key.clone(), move || {
                let entry = entry_of(&ffmpeg, &path)?;
                let mut store = store
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let token = store.insert(&key, entry.clone())?;
                Ok(entry.public(&token, Status::Ok))
            });
        }
        "media.sources.resolve" => {
            let token = match token_of(params) {
                Ok(token) => token,
                Err(error) => return answer(Err(error)),
            };
            spawn(events, reply, key.clone(), move || {
                let entry = store
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .get(&key, &token)
                    .cloned()
                    .ok_or_else(unknown)?;
                Ok(entry.public(&token, entry.status()))
            });
        }
        "media.sources.list" => spawn(events, reply, key.clone(), move || {
            let entries = store
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .list(&key);
            Ok(Value::Array(
                entries
                    .iter()
                    .map(|(token, entry)| entry.public(token, entry.status()))
                    .collect(),
            ))
        }),
        "media.sources.forget" => {
            let token = match token_of(params) {
                Ok(token) => token,
                Err(error) => return answer(Err(error)),
            };
            store
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .forget(&key, &token);
            answer(Ok(Value::Null));
        }
        "media.sources.locate" => {
            let token = match token_of(params) {
                Ok(token) => token,
                Err(error) => return answer(Err(error)),
            };
            let found = store
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&key, &token)
                .map(|entry| {
                    (
                        entry.name.clone(),
                        entry.path.parent().map(Path::to_path_buf),
                    )
                });
            match found {
                None => answer(Err(unknown())),
                // The UI thread shows the dialog (`Event::SourceLocate`),
                // opened where the file was, and sends the choice back as
                // `owner.sourceLocated`. The plugin sees neither path.
                Some((name, directory)) => {
                    let _ = events.send(Event::SourceLocate(
                        reply,
                        json!({"store":key,"token":token,"name":name,"directory":directory}),
                    ));
                }
            }
        }
        "owner.sourceLocated" => {
            let token = match token_of(params) {
                Ok(token) => token,
                Err(error) => return answer(Err(error)),
            };
            let Some(path) = params["path"].as_str().map(PathBuf::from) else {
                return answer(Err(RpcError::invalid("path required")));
            };
            let ffmpeg = match ffmpeg() {
                Ok(ffmpeg) => ffmpeg,
                Err(error) => return answer(Err(error)),
            };
            spawn(events, reply, key.clone(), move || {
                let entry = entry_of(&ffmpeg, &path)?;
                store
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .rebind(&key, &token, entry.clone())?;
                Ok(entry.public(&token, Status::Ok))
            });
        }
        "jobs.check" => {
            let parts: Vec<Part> = match serde_json::from_value(params["parts"].clone()) {
                Ok(parts) => parts,
                Err(error) => return answer(Err(RpcError::invalid(error.to_string()))),
            };
            let encoding = match check_encoding(params) {
                Ok(encoding) => encoding,
                Err(error) => return answer(Err(error)),
            };
            if !parts_valid(&parts) {
                return answer(Err(RpcError::invalid(format!(
                    "1–{MAX_PARTS} parts with a token and finite times from 0 to 86400 s (24 hours in all) required"
                ))));
            }
            spawn(events, reply, key.clone(), move || {
                let resolved = resolve_parts(&store, &key, &parts);
                Ok(check(&parts, &resolved, &encoding))
            });
        }
        _ => answer(Err(RpcError::new("unsupported", "unknown method"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(codec: &str, width: u32, height: u32, rate: (u32, u32)) -> VideoFacts {
        VideoFacts {
            index: 0,
            codec: codec.into(),
            profile: Some("High".into()),
            width,
            height,
            sar: (1, 1),
            rate,
            pixel_format: Some("yuv420p".into()),
            extradata: Some("SHA256:aa".into()),
        }
    }
    fn audio(rate: u32, channels: u32) -> AudioFacts {
        AudioFacts {
            index: 1,
            codec: "aac".into(),
            sample_rate: rate,
            channels,
        }
    }
    fn entry(path: &str, video: Option<VideoFacts>, audio: Option<AudioFacts>) -> Entry {
        Entry {
            path: PathBuf::from(path),
            len: 1,
            modified: 2,
            name: display_name(Path::new(path)),
            facts: Facts {
                duration: 60.,
                origin: 0.,
                format: "mov,mp4,m4a,3gp,3g2,mj2".into(),
                video,
                audio,
            },
        }
    }
    fn ok(entry: Entry) -> Resolved {
        Resolved {
            entry: Some(entry),
            status: Status::Ok,
        }
    }
    fn part(start: f64, end: f64) -> Part {
        Part {
            source: "t".into(),
            start,
            end,
        }
    }

    #[test]
    fn matching_parts_copy_into_the_container_that_fits() {
        let a = entry(
            "/a.mp4",
            Some(video("h264", 1920, 1080, (30, 1))),
            Some(audio(48000, 2)),
        );
        let checked = check(
            &[part(0., 5.), part(10., 12.)],
            &[ok(a.clone()), ok(a)],
            &Encoding::default(),
        );
        assert_eq!(checked["blocking"], json!([]));
        assert_eq!(checked["copy"], json!({"ok":true,"container":"mp4"}));
        assert_eq!(
            checked["encode"],
            json!({"width":1920,"height":1080,"fps":30.0,"audio":{"sampleRate":48000,"channels":2}})
        );
        assert_eq!(checked["duration"], json!(7.0));
        let vp9 = entry("/a.webm", Some(video("vp9", 640, 360, (25, 1))), None);
        let checked = check(&[part(0., 1.)], &[ok(vp9)], &Encoding::default());
        assert_eq!(checked["copy"], json!({"ok":true,"container":"mkv"}));
        assert_eq!(checked["encode"]["audio"], Value::Null);
    }

    #[test]
    fn each_difference_from_the_first_part_is_a_reason() {
        let first = entry(
            "/a.mp4",
            Some(video("h264", 3840, 1714, (30, 1))),
            Some(audio(48000, 2)),
        );
        let mut other = video("h264", 1920, 818, (30000, 1001));
        other.extradata = Some("SHA256:bb".into());
        other.pixel_format = Some("yuv420p10le".into());
        let second = entry("/b.mp4", Some(other), Some(audio(44100, 1)));
        let third = entry("/c.mkv", Some(video("hevc", 3840, 1714, (60, 2))), None);
        let checked = check(
            &[part(0., 5.), part(0., 5.), part(0., 5.)],
            &[ok(first), ok(second), ok(third)],
            &Encoding::default(),
        );
        let fields: Vec<(u64, &str)> = checked["copy"]["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| (r["part"].as_u64().unwrap(), r["field"].as_str().unwrap()))
            .collect();
        assert_eq!(
            fields,
            [
                (1, "videoParams"),
                (1, "size"),
                (1, "fps"),
                (1, "pixelFormat"),
                (1, "audio"),
                (2, "videoCodec"),
                (2, "audio"),
            ]
        );
        assert_eq!(checked["copy"]["ok"], false);
        let size = &checked["copy"]["reasons"][1];
        assert_eq!(size["value"], "1920×818");
        assert_eq!(size["expected"], "3840×1714");
        assert_eq!(checked["copy"]["reasons"][4]["value"], "aac 44100 Hz 1 ch");
        assert_eq!(checked["copy"]["reasons"][6]["value"], "none");
        // 60/2 is 30/1: the same rate.
        assert!(!fields.contains(&(2, "fps")));
        // The re-encode takes the first part's size and rate, with audio.
        assert_eq!(
            checked["encode"],
            json!({"width":3840,"height":1714,"fps":30.0,"audio":{"sampleRate":48000,"channels":2}})
        );
    }

    #[test]
    fn missing_changed_unknown_and_bad_ranges_block_both_modes() {
        let a = entry("/a.mp4", Some(video("h264", 640, 360, (25, 1))), None);
        let checked = check(
            &[
                part(0., 5.),
                part(0., 5.),
                part(0., 5.),
                part(50., 61.),
                part(3., 3.01),
            ],
            &[
                Resolved {
                    entry: Some(a.clone()),
                    status: Status::Missing,
                },
                Resolved {
                    entry: Some(a.clone()),
                    status: Status::Changed,
                },
                Resolved {
                    entry: None,
                    status: Status::Missing,
                },
                ok(a.clone()),
                ok(a),
            ],
            &Encoding::default(),
        );
        assert_eq!(
            checked["blocking"],
            json!([
                {"part":0,"problem":"missing"},
                {"part":1,"problem":"changed"},
                {"part":2,"problem":"unknown"},
                {"part":3,"problem":"range","duration":60.0},
                {"part":4,"problem":"range","duration":60.0},
            ])
        );
        let audio_only = entry("/a.m4a", None, Some(audio(48000, 2)));
        let checked = check(&[part(0., 5.)], &[ok(audio_only)], &Encoding::default());
        assert_eq!(checked["blocking"], json!([{"part":0,"problem":"noVideo"}]));
        assert_eq!(checked["encode"], Value::Null);
    }

    #[test]
    fn the_encode_target_is_even_square_pixels_under_the_ceilings() {
        let mut anamorphic = video("mpeg2video", 720, 576, (25, 1));
        anamorphic.sar = (16, 15);
        assert_eq!(
            target(&anamorphic, &Encoding::default()),
            Target {
                width: 768,
                height: 576,
                rate: (25, 1)
            }
        );
        let big = video("h264", 3840, 1714, (60000, 1001));
        let capped = Encoding {
            max_height: Some(720),
            fps: Some(30),
            ..Encoding::default()
        };
        assert_eq!(
            target(&big, &capped),
            Target {
                width: 1614,
                height: 720,
                rate: (30, 1)
            }
        );
        let odd = video("h264", 427, 241, (30, 1));
        assert_eq!(target(&odd, &Encoding::default()).width, 428);
        assert_eq!(target(&odd, &Encoding::default()).height, 242);
    }

    #[test]
    fn probe_json_gives_the_default_streams_and_skips_cover_images() {
        let probed = json!({
            "format": {"duration":"12.5","start_time":"1.400000","format_name":"matroska,webm"},
            "streams": [
                {"index":0,"codec_type":"video","codec_name":"mjpeg","width":300,"height":300,"avg_frame_rate":"0/0","r_frame_rate":"90000/1","disposition":{"default":0,"attached_pic":1}},
                {"index":1,"codec_type":"audio","codec_name":"opus","sample_rate":"48000","channels":2,"disposition":{"default":0}},
                {"index":2,"codec_type":"video","codec_name":"vp9","profile":"Profile 0","width":1280,"height":720,"sample_aspect_ratio":"1:1","pix_fmt":"yuv420p","avg_frame_rate":"30000/1001","disposition":{"default":1}},
                {"index":3,"codec_type":"audio","codec_name":"aac","sample_rate":"44100","channels":6,"disposition":{"default":1}},
            ]
        });
        let facts = facts_from(&probed).unwrap();
        assert_eq!(facts.duration, 12.5);
        assert_eq!(facts.origin, 1.4);
        let video = facts.video.unwrap();
        assert_eq!(
            (video.index, video.codec.as_str(), video.rate),
            (2, "vp9", (30000, 1001))
        );
        let audio = facts.audio.unwrap();
        assert_eq!(
            (audio.index, audio.sample_rate, audio.channels),
            (3, 44100, 6)
        );
        assert_eq!(
            facts_from(&json!({"format":{},"streams":[]}))
                .unwrap_err()
                .code,
            "unsupported"
        );
    }

    #[test]
    fn the_store_dedupes_bounds_forgets_revokes_and_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("media-sources.json");
        let mut store = Store::open(Some(file.clone()));
        let a = entry("/a.mp4", Some(video("h264", 640, 360, (25, 1))), None);
        let token = store.insert("instance:1", a.clone()).unwrap();
        assert!(token.starts_with("src-"));
        // The same file as it is: the same token.
        assert_eq!(store.insert("instance:1", a.clone()).unwrap(), token);
        // Another plugin's store does not know it.
        assert!(store.get("instance:2", &token).is_none());
        let other = store.insert("instance:2", a.clone()).unwrap();
        assert_ne!(other, token);
        // A restart reads the same references.
        store.flush();
        let reopened = Store::open(Some(file.clone()));
        assert_eq!(reopened.get("instance:1", &token), Some(&a));
        assert_eq!(reopened.list("instance:2").len(), 1);
        // The plugin's public view never has the path.
        let public = a.public(&token, Status::Missing);
        assert_eq!(public["name"], "a.mp4");
        assert_eq!(public["status"], "missing");
        assert!(!public.to_string().contains("/a.mp4"));
        // Rebind, forget, revoke.
        let mut b = a.clone();
        b.path = PathBuf::from("/b.mp4");
        store.rebind("instance:1", &token, b.clone()).unwrap();
        assert_eq!(store.get("instance:1", &token), Some(&b));
        assert_eq!(
            store.rebind("instance:1", "src-x", b).unwrap_err().code,
            "missing"
        );
        store.forget("instance:1", &token);
        assert!(store.get("instance:1", &token).is_none());
        store.revoke("instance:2");
        store.flush();
        assert!(
            Store::open(Some(file.clone()))
                .list("instance:2")
                .is_empty()
        );
        // A development load's references end with its run.
        store.insert("development:a.b", a.clone()).unwrap();
        store.end_development_loads();
        assert!(store.list("development:a.b").is_empty());
        assert_eq!(store.list("instance:1").len(), 0);
        drop(store);
        // Dropping the store writes what was pending.
        assert!(Store::open(Some(file)).list("development:a.b").is_empty());
        // The bound.
        let mut store = Store::open(None);
        for n in 0..MAX_PER_PLUGIN {
            let mut e = a.clone();
            e.path = PathBuf::from(format!("/{n}.mp4"));
            store.insert("k", e).unwrap();
        }
        assert_eq!(
            store
                .insert("k", entry("/x.mp4", None, None))
                .unwrap_err()
                .code,
            "limit"
        );
    }

    #[test]
    fn status_follows_the_file_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clip.mp4");
        std::fs::write(&path, b"1234").unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        let mut e = entry("/x", None, None);
        e.path = path.clone();
        e.len = meta.len();
        e.modified = nanos(meta.modified().unwrap());
        assert_eq!(e.status(), Status::Ok);
        std::fs::write(&path, b"12345").unwrap();
        assert_eq!(e.status(), Status::Changed);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(e.status(), Status::Missing);
    }

    #[test]
    fn open_names_the_file_only_to_the_open_and_refuses_a_missing_or_changed_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clip it's.mp4");
        std::fs::write(&path, b"1234").unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        let mut e = entry("/x", None, None);
        e.path = path.clone();
        e.len = meta.len();
        e.modified = nanos(meta.modified().unwrap());
        let shared: Shared = Arc::new(Mutex::new(Store::default()));
        let token = shared.lock().unwrap().insert("k", e).unwrap();
        let request = |fields: Value| {
            let mut params = json!({"store":"k","token":token,"operationId":3});
            params
                .as_object_mut()
                .unwrap()
                .extend(fields.as_object().unwrap().clone());
            open_request(&shared, &params)
        };
        let (opened, start, submit) = request(json!({"start": 12.5})).unwrap();
        assert_eq!(opened, path.to_str().unwrap());
        assert_eq!(start, Some(12.5));
        assert_eq!(
            submit,
            json!({"operationId":3,"intent":{"kind":"open","source":opened,"startPaused":false}})
        );
        // From the beginning: no resume point.
        assert_eq!(request(json!({"start": 0})).unwrap().1, None);
        assert_eq!(request(json!({})).unwrap().1, None);
        for bad in [
            json!({"start": -1}),
            json!({"start": 60}),
            json!({"start": "1"}),
        ] {
            assert_eq!(request(bad).unwrap_err().code, "invalid");
        }
        // Another plugin's store; no operation id.
        let mut other = json!({"store":"j","token":token,"operationId":3});
        assert_eq!(open_request(&shared, &other).unwrap_err().code, "missing");
        other["store"] = json!("k");
        other["operationId"] = json!(0);
        assert_eq!(open_request(&shared, &other).unwrap_err().code, "invalid");
        std::fs::write(&path, b"12345").unwrap();
        assert_eq!(request(json!({})).unwrap_err().code, "source-changed");
        std::fs::remove_file(&path).unwrap();
        assert_eq!(request(json!({})).unwrap_err().code, "source-missing");
    }

    #[test]
    fn one_plugin_cannot_take_every_request_slot() {
        let (events, replies) = mpsc::channel();
        let (go, wait) = std::sync::mpsc::channel::<()>();
        let wait = Arc::new(Mutex::new(wait));
        for _ in 0..MAX_IN_FLIGHT_PER_PLUGIN + 1 {
            let wait = wait.clone();
            spawn(
                &events,
                Reply { owner: 1, id: 1 },
                "test:greedy".into(),
                move || {
                    let _ = wait.lock().unwrap().recv();
                    Ok(Value::Null)
                },
            );
        }
        // The fifth of one plugin is refused at once; another plugin's runs.
        let refused = replies.recv().unwrap();
        assert!(
            matches!(&refused, Event::Reply(_, Err(e)) if e.code == "busy"),
            "refused"
        );
        spawn(
            &events,
            Reply { owner: 2, id: 1 },
            "test:other".into(),
            || Ok(json!(2)),
        );
        assert!(matches!(replies.recv().unwrap(), Event::Reply(r, Ok(_)) if r.owner == 2));
        for _ in 0..MAX_IN_FLIGHT_PER_PLUGIN {
            go.send(()).unwrap();
        }
        for _ in 0..MAX_IN_FLIGHT_PER_PLUGIN {
            assert!(matches!(replies.recv().unwrap(), Event::Reply(_, Ok(_))));
        }
    }

    #[test]
    fn parts_are_bounded() {
        // 24 hours in all, as one clip.
        assert!(parts_valid(&[part(0., 43200.), part(0., 43200.)]));
        assert!(!parts_valid(&[part(0., 43200.), part(0., 43201.)]));
        assert!(parts_valid(&[part(0., 1.)]));
        assert!(!parts_valid(&[]));
        assert!(!parts_valid(&vec![part(0., 1.); MAX_PARTS + 1]));
        assert!(!parts_valid(&[part(-1., 1.)]));
        assert!(!parts_valid(&[part(0., f64::NAN)]));
        assert!(!parts_valid(&[Part {
            source: String::new(),
            start: 0.,
            end: 1.
        }]));
    }
}
