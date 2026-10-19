//! Each trusted plugin gets its own Node process and stdout protocol.
use crate::Event;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

pub const API: &str = "vestra.experimental/0.1";
#[derive(Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub api_version: String,
    pub entry: String,
    pub capabilities: Vec<String>,
    /// Production packages carry a version; direct debug loading ignores it.
    #[serde(default, skip_serializing)]
    pub version: Option<String>,
}
/// Bounded per-plugin output queue. Source state alone emits ~10 events/s while playing,
/// so a small queue disables healthy plugins after a brief event-loop stall.
const OUTBOUND_CAPACITY: usize = 1024;
pub const CAPABILITIES: &[&str] = &[
    "producer.av",
    "playback.read",
    "playback.control",
    "source.resolve",
    "events",
    "commands",
    "ui.window",
    "ui.mount",
    "ui.dialog",
    "plugins.manage",
    "processing.video",
    "processing.audio",
    "subtitles.read",
    "subtitles.control",
    "media.read",
    "media.control",
];
pub struct Plugin {
    pub manifest: Manifest,
    pub directory: PathBuf,
    pub subscriptions: BTreeSet<String>,
    child: Child,
    outbound: mpsc::SyncSender<Value>,
}
impl Plugin {
    pub fn load(directory: &Path, generation: u64, events: mpsc::Sender<Event>) -> Result<Self> {
        let directory = directory.canonicalize().context("plugin directory")?;
        let manifest: Manifest =
            serde_json::from_slice(&std::fs::read(directory.join("manifest.json"))?)?;
        Self::load_manifest(&directory, generation, events, manifest)
    }
    pub fn load_manifest(
        directory: &Path,
        generation: u64,
        events: mpsc::Sender<Event>,
        manifest: Manifest,
    ) -> Result<Self> {
        let directory = directory.canonicalize()?;
        if manifest.api_version != API {
            bail!("unsupported apiVersion: {}", manifest.api_version);
        }
        if manifest.id.is_empty()
            || !manifest
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
        {
            bail!("invalid plugin id");
        }
        if manifest
            .capabilities
            .iter()
            .any(|c| !CAPABILITIES.contains(&c.as_str()))
        {
            bail!("unknown required capability");
        }
        package_file(&directory, &manifest.entry)?;
        let packaged_runner = std::env::current_exe()?
            .parent()
            .and_then(Path::parent)
            .map(|p| p.join("Resources/runtime/runner.ts"));
        let runner = packaged_runner
            .filter(|p| p.is_file())
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../runtime/runner.ts"));
        let mut child = Command::new("node")
            .arg(runner)
            .arg(&directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("start Node plugin runtime (Node >=22.18)")?;
        let mut stdin = child.stdin.take().context("plugin stdin")?;
        let (outbound, output) = mpsc::sync_channel::<Value>(OUTBOUND_CAPACITY);
        let write_events = events.clone();
        thread::spawn(move || {
            for message in output {
                let mut write = || -> Result<()> {
                    serde_json::to_writer(&mut stdin, &message)?;
                    stdin.write_all(b"\n")?;
                    stdin.flush()?;
                    Ok(())
                };
                if let Err(error) = write() {
                    let _ = write_events.send(Event::RuntimeFault(
                        generation,
                        format!("plugin stdin: {error}"),
                    ));
                    break;
                }
            }
        });
        let stdout = child.stdout.take().context("plugin stdout")?;
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                // Protocol lines are capped at 1 MiB. Oversize/malformed output terminates the instance.
                let mut bytes = Vec::new();
                let read = std::io::Read::by_ref(&mut reader)
                    .take(1_048_577)
                    .read_until(b'\n', &mut bytes);
                match read {
                    Ok(0) | Err(_) => break,
                    Ok(_) if bytes.len() > 1_048_576 => {
                        let _ = events.send(Event::RuntimeFault(
                            generation,
                            "protocol message exceeds 1 MiB".into(),
                        ));
                        break;
                    }
                    Ok(_) => match serde_json::from_slice(&bytes) {
                        Ok(message) => {
                            if events.send(Event::Request(generation, message)).is_err() {
                                return;
                            }
                        }
                        Err(e) => {
                            let _ = events.send(Event::RuntimeFault(
                                generation,
                                format!("invalid protocol: {e}"),
                            ));
                            break;
                        }
                    },
                }
            }
            let _ = events.send(Event::Exited(generation));
        });
        Ok(Self {
            manifest,
            directory,
            subscriptions: BTreeSet::new(),
            child,
            outbound,
        })
    }
    pub fn send(&mut self, message: Value) -> Result<()> {
        self.outbound
            .try_send(message)
            .map_err(|error| anyhow::anyhow!("plugin output backpressure/disconnected: {error}"))?;
        Ok(())
    }
    pub fn allows(&self, capability: &str) -> bool {
        self.manifest.capabilities.iter().any(|c| c == capability)
    }
}
impl Drop for Plugin {
    fn drop(&mut self) {
        let _ = self.send(json!({"event":"runtime.stop","data":null}));
        let deadline = Instant::now() + Duration::from_millis(300);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
pub fn package_file(directory: &Path, relative: &str) -> Result<PathBuf> {
    if Path::new(relative).is_absolute() {
        bail!("package file must be relative");
    }
    let path = directory.join(relative).canonicalize()?;
    if !path.starts_with(directory) || !path.is_file() {
        bail!("package file must remain inside package directory");
    }
    Ok(path)
}
