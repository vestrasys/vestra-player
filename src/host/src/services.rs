// 实验公共资源服务；不代表最终 API。所有资源由调用插件拥有，不按插件名称分支。
mod assets;
mod effects;
use assets::ManagedAsset;
use std::sync::Arc;
mod jobs;
mod sources;
use crate::{backend::Backend, plugins::Manifest, ui};
use serde_json::{Value as Json, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

struct Job {
    owner: String,
    child: Option<Child>,
    state: String,
    error: Option<String>,
    output: PathBuf,
    working: PathBuf,
    diagnostic: PathBuf,
    progress: PathBuf,
    progress_seconds: f64,
    assets: Vec<Arc<ManagedAsset>>,
}
#[derive(Clone)]
struct Effect {
    owner: String,
    key: String,
    kind: String,
    filter: String,
    session: Option<String>,
    assets: Vec<Arc<ManagedAsset>>,
}
pub struct Source {
    pub owner: String,
    pub key: String,
    pub width: u32,
    pub height: u32,
    pub frame: Option<Vec<u8>>,
    pub frames: u64,
    pub paused: bool,
    audio_bytes: std::sync::Arc<std::sync::atomic::AtomicU64>,
    audio_sender: Option<std::sync::mpsc::SyncSender<Vec<u8>>>,
    audio_reader: Option<std::thread::JoinHandle<()>>,
    sample_rate: u32,
    audio: Option<Child>,
}
struct ProcessingSnapshot {
    owner: String,
    video: String,
    audio: String,
    assets: Vec<Arc<ManagedAsset>>,
}
pub struct Services {
    effects: Vec<Effect>,
    jobs: BTreeMap<String, Job>,
    next: u64,
    snapshots: BTreeMap<String, ProcessingSnapshot>,
    asset_refs: Vec<std::sync::Weak<ManagedAsset>>,
    pub session: u64,
    pub source: Option<Source>,
    subtitle_supported: bool,
    legacy_gaussian_owner: Option<String>,
}
fn text<'a>(p: &'a Json, k: &str) -> Result<&'a str, String> {
    p[k].as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("missing string {k}"))
}
fn finite(p: &Json, k: &str, default: f64) -> Result<f64, String> {
    let n = p[k].as_f64().unwrap_or(default);
    if n.is_finite() {
        Ok(n)
    } else {
        Err(format!("invalid {k}"))
    }
}
impl Services {
    pub fn new() -> Self {
        Self {
            effects: vec![],
            jobs: BTreeMap::new(),
            next: 0,
            snapshots: BTreeMap::new(),
            asset_refs: vec![],
            session: 0,
            source: None,
            legacy_gaussian_owner: None,
            subtitle_supported: Command::new("ffmpeg")
                .args(["-hide_banner", "-filters"])
                .output()
                .map(|o| {
                    String::from_utf8_lossy(&o.stdout)
                        .lines()
                        .any(|l| l.split_whitespace().nth(1) == Some("subtitles"))
                })
                .unwrap_or(false),
        }
    }
    pub fn session_id(&self) -> String {
        format!("session-{}", self.session)
    }
    pub fn diagnostics(&self) -> Json {
        json!({"effects":self.effects.len(),"jobs":self.jobs.len(),"snapshots":self.snapshots.len(),"source":self.source.is_some(),"managedAssets":self.asset_refs.iter().filter(|a|a.strong_count()>0).count()})
    }
    pub fn new_session(&mut self) {
        self.session += 1;
        self.effects.retain(|e| e.session.is_none());
    }
    pub fn file_loaded(&mut self, b: &mut Backend) -> Result<(), String> {
        self.close_source();
        self.new_session();
        self.apply(b)
    }
    fn apply(&self, b: &mut Backend) -> Result<(), String> {
        let chain = |kind: &str| {
            self.effects
                .iter()
                .filter(|e| e.kind == kind || (kind == "video" && e.kind == "subtitle"))
                .map(|e| e.filter.clone())
                .collect::<Vec<_>>()
                .join(",")
        };
        b.set_processing(chain("video"), chain("audio"))
    }
    pub fn cleanup(&mut self, owner: &str, b: &mut Backend) {
        if self.legacy_gaussian_owner.as_deref() == Some(owner) {
            self.legacy_gaussian_owner = None;
            let sigma = b.snapshot().sigma;
            let _ = b.set_gaussian(false, sigma);
        }
        self.effects.retain(|e| e.owner != owner);
        self.snapshots.retain(|_, s| s.owner != owner);
        let _ = self.apply(b);
        self.jobs.retain(|_, job| {
            if job.owner == owner {
                cancel(job);
                let _ = std::fs::remove_file(&job.progress);
                false
            } else {
                true
            }
        });
        if self.source.as_ref().is_some_and(|s| s.owner == owner) {
            self.close_source();
        }
    }
    fn close_source(&mut self) {
        if let Some(mut source) = self.source.take() {
            source.stop_audio();
        }
    }
    fn chains(&self, include_subtitles: bool, b: &Backend) -> (String, String) {
        let mut video = self
            .effects
            .iter()
            .filter(|e| e.kind == "video" || (include_subtitles && e.kind == "subtitle"))
            .map(|e| e.filter.clone())
            .collect::<Vec<_>>();
        let legacy = b.snapshot();
        if legacy.gaussian_enabled && legacy.sigma > 0. {
            video.insert(0, format!("gblur=sigma={}:steps=2", legacy.sigma));
        }
        let audio = self
            .effects
            .iter()
            .filter(|e| e.kind == "audio")
            .map(|e| e.filter.clone())
            .collect::<Vec<_>>()
            .join(",");
        (video.join(","), audio)
    }
    pub fn request(
        &mut self,
        owner: &str,
        method: &str,
        p: &Json,
        b: &mut Backend,
    ) -> Result<Option<Json>, String> {
        validate_params(method, p)?;
        if let Some(result) = self.request_effects(owner, method, p, b)? {
            return Ok(Some(result));
        }
        if let Some(result) = self.request_jobs(owner, method, p, b)? {
            return Ok(Some(result));
        }
        if let Some(result) = self.request_sources(owner, method, p, b)? {
            return Ok(Some(result));
        }
        let value = match method {
            "plugins.validate" => {
                let result = validate(Path::new(text(p, "manifest")?));
                match result {
                    Ok(_) => json!({"valid":true,"diagnostics":[]}),
                    Err(e) => json!({"valid":false,"diagnostics":[e]}),
                }
            }
            _ => return Ok(None),
        };
        Ok(Some(value))
    }
}
fn cancel(job: &mut Job) {
    if let Some(mut child) = job.child.take() {
        let _ = child.kill();
        let _ = child.wait();
        job.state = "cancelled".into();
        job.assets.clear();
        let _ = std::fs::remove_file(&job.diagnostic);
        let _ = std::fs::remove_file(&job.progress);
        let _ = std::fs::remove_file(&job.working);
    }
}
impl Drop for Services {
    fn drop(&mut self) {
        for job in self.jobs.values_mut() {
            cancel(job);
            let _ = std::fs::remove_file(&job.progress);
        }
        self.close_source();
    }
}
pub fn validate(path: &Path) -> Result<Manifest, String> {
    let manifest = Manifest::read(path)?;
    let tsc = Path::new(env!("CARGO_MANIFEST_DIR")).join("../node_modules/typescript/bin/tsc");
    let output = Command::new("node")
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join(".."))
        .arg(tsc)
        .args([
            "--ignoreConfig",
            "--types",
            "node",
            "--noEmit",
            "--strict",
            "--skipLibCheck",
            "--allowImportingTsExtensions",
            "--module",
            "nodenext",
            "--target",
            "es2023",
        ])
        .arg(&manifest.entry_path)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stdout).to_string()
            + &String::from_utf8_lossy(&output.stderr));
    }
    ui::compile(std::slice::from_ref(&manifest))?;
    Ok(manifest)
}

fn validate_filter(kind: &str, filter: &str) -> Result<(), String> {
    let mut command = Command::new("ffmpeg");
    command.args(["-v", "error", "-nostdin", "-f", "lavfi", "-i"]);
    if kind == "audio" {
        command
            .arg("anullsrc=r=48000:cl=stereo")
            .args(["-t", "0.01", "-af", filter]);
    } else {
        command
            .arg("color=c=black:s=640x360:r=1")
            .args(["-frames:v", "1", "-vf", filter]);
    }
    let output = command
        .args(["-f", "null", "-"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "filter rejected: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

pub fn validate_params(method: &str, p: &Json) -> Result<(), String> {
    let allowed: &[&str] = match method {
        "capabilities" | "effects.list" | "player.snapshot" | "player.togglePause"
        | "dialogs.openMedia" | "host.inspect" => &[],
        "effects.set" => &["key", "kind", "filter", "sessionId", "files"],
        "effects.remove" | "sources.close" => &["key"],
        "sources.setPaused" => &["key", "paused"],
        "effects.snapshot" => &["includeSubtitles"],
        "effects.releaseSnapshot" | "media.status" | "media.cancel" | "plugins.remove" => &["id"],
        "media.start" => &[
            "input",
            "output",
            "format",
            "videoFilter",
            "audioFilter",
            "start",
            "duration",
            "processing",
            "processingSnapshot",
            "includeSubtitles",
        ],
        "sources.open" => &["key", "width", "height", "sampleRate"],
        "sources.present" => &["key", "sessionId", "rgb", "pcm"],
        "plugins.validate" | "plugins.install" | "host.load" => &["manifest"],
        "player.load" => &["path"],
        "player.seek" => &["seconds"],
        "player.filters.setGaussian" => &["enabled", "sigma"],
        "ui.patch" => &["properties"],
        "dialogs.openFile" => &["title", "extensions"],
        "dialogs.saveFile" => &["title", "extensions", "suggestedName"],
        "host.dialogs" => &["paths"],
        "host.action" => &["plugin", "name", "value"],
        "host.input" => &["plugin", "key", "pressed"],
        "host.unload" => &["plugin"],
        _ => return Ok(()),
    };
    let object = p.as_object().ok_or("params must be an object")?;
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(format!("unsupported field {key} for {method}"));
        }
    }
    for key in [
        "key",
        "kind",
        "filter",
        "sessionId",
        "input",
        "output",
        "format",
        "videoFilter",
        "audioFilter",
        "processing",
        "processingSnapshot",
        "rgb",
        "pcm",
        "manifest",
        "path",
        "title",
        "suggestedName",
        "id",
        "plugin",
        "name",
    ] {
        if object.contains_key(key) && !object[key].is_string() {
            return Err(format!("{key} must be string"));
        }
    }
    if let Some(extensions) = object.get("extensions") {
        let extensions = extensions.as_array().ok_or("extensions must be array")?;
        if extensions.is_empty()
            || extensions.iter().any(|v| {
                !v.as_str()
                    .is_some_and(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric()))
            })
        {
            return Err("extensions must contain file extension strings".into());
        }
    }
    if let Some(paths) = object.get("paths") {
        let paths = paths.as_array().ok_or("paths must be array")?;
        if paths.iter().any(|v| !v.is_null() && !v.is_string()) {
            return Err("paths must contain strings or null".into());
        }
    }
    if let Some(files) = object.get("files") {
        let files = files.as_object().ok_or("files must be object")?;
        if files.len() > 16
            || files.iter().any(|(key, path)| {
                key.is_empty()
                    || !key
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                    || !path.as_str().is_some_and(|p| Path::new(p).is_absolute())
            })
        {
            return Err("files must map identifiers to absolute file paths".into());
        }
    }
    for key in [
        "start",
        "duration",
        "sampleRate",
        "width",
        "height",
        "seconds",
        "sigma",
        "value",
    ] {
        if let Some(value) = object.get(key)
            && !value.as_f64().is_some_and(f64::is_finite)
        {
            return Err(format!("{key} must be finite number"));
        }
    }
    for key in ["sampleRate", "width", "height"] {
        if object.contains_key(key) && object[key].as_u64().is_none() {
            return Err(format!("{key} must be a nonnegative integer"));
        }
    }
    for key in ["includeSubtitles", "enabled", "pressed", "paused"] {
        if object.contains_key(key) && !object[key].is_boolean() {
            return Err(format!("{key} must be boolean"));
        }
    }
    if object.contains_key("processing") && object["processing"] != "current" {
        return Err("unsupported processing mode".into());
    }
    Ok(())
}

impl Source {
    fn stop_audio(&mut self) {
        self.audio_sender.take();
        if let Some(mut child) = self.audio.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(reader) = self.audio_reader.take() {
            let _ = reader.join();
        }
    }

    pub fn diagnostics(&self) -> Json {
        let hash = self.frame.as_ref().map(|frame| {
            let hash = frame.iter().fold(0xcbf29ce484222325u64, |hash, byte| {
                (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
            });
            format!("{hash:016x}")
        });
        json!({"width":self.width,"height":self.height,"frames":self.frames,"audioBytes":self.audio_bytes.load(std::sync::atomic::Ordering::Relaxed),"frameBytes":self.frame.as_ref().map(Vec::len),"frameHash":hash,"paused":self.paused})
    }
}
