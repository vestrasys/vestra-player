//! Each trusted plugin gets its own Node process and stdout protocol.
use crate::Event;
use anyhow::{Context, Result, bail};
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

pub const API: &str = "vestra/1";
pub use crate::manifest::Manifest;

/// Whose data directory and secrets a plugin process gets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DataKey {
    /// An installed instance: `plugin-data/<GUID>/`, secrets `vestra.plugin.<GUID>`.
    Instance(String),
    /// A development load (`--plugin`): `plugin-data/development/<id>/`.
    Development(String),
    /// A trial: `plugin-data/<id>.trial.<owner>/`, removed when it stops.
    Trial,
}
impl DataKey {
    /// The data directory under the data root.
    pub fn directory(&self, root: &Path, manifest_id: &str) -> PathBuf {
        match self {
            Self::Instance(guid) => root.join(guid),
            Self::Development(id) => root.join("development").join(id),
            Self::Trial => root.join(manifest_id),
        }
    }
    /// The secret store service name.
    pub fn secret_service(&self, manifest_id: &str) -> String {
        match self {
            Self::Instance(guid) => format!("vestra.plugin.{guid}"),
            Self::Development(id) => format!("vestra.dev.{id}"),
            Self::Trial => manifest_id.to_owned(),
        }
    }
}
/// Bounded per-plugin output queue. Source state alone emits ~10 events/s while playing,
/// so a small queue disables healthy plugins after a brief event-loop stall.
const OUTBOUND_CAPACITY: usize = 1024;
/// The SDK repeats this list (vestra-player-sdk: src/capabilities.ts, used by
/// `vestra-plugin check`); tests/sdk/host-agreement.test.ts checks the
/// installed SDK agrees.
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
    "ui.osd",
    "clipboard",
    "window.control",
    "bindings.manage",
    "timeline.marks",
    "ui.drop",
    "ui.settings",
    "ui.theme",
    "secrets",
];
/// `plugin.app` as JSON, set once at start-up and given to every plugin.
static APP_INFO: std::sync::OnceLock<String> = std::sync::OnceLock::new();
pub fn set_app_info(info: &Value) {
    let _ = APP_INFO.set(info.to_string());
}
pub struct Plugin {
    pub manifest: Manifest,
    pub directory: PathBuf,
    pub data_key: DataKey,
    /// `plugin.dataDirectory`; a trial's is temporary and removed when it stops.
    data_directory: Option<PathBuf>,
    temporary_data: bool,
    pub subscriptions: BTreeSet<String>,
    child: Child,
    outbound: mpsc::SyncSender<Value>,
}
impl Plugin {
    pub fn load(directory: &Path, generation: u64, events: mpsc::Sender<Event>) -> Result<Self> {
        let directory = dunce::canonicalize(directory).context("plugin directory")?;
        let manifest = crate::manifest::read_development(&directory)?;
        let key = DataKey::Development(manifest.id.clone());
        Self::load_manifest(&directory, generation, events, manifest, key)
    }
    pub fn load_manifest(
        directory: &Path,
        generation: u64,
        events: mpsc::Sender<Event>,
        manifest: Manifest,
        data_key: DataKey,
    ) -> Result<Self> {
        let trial = data_key == DataKey::Trial;
        let directory = dunce::canonicalize(directory)?;
        if manifest.api_version != API {
            bail!("unsupported apiVersion: {}", manifest.api_version);
        }
        if !crate::manifest::valid_id(&manifest.id) {
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
        // Per installed instance, development id or trial (see `DataKey`).
        // Without one the SDK reports `unavailable`; loading continues.
        let data_directory = data_root()
            .map(|root| data_key.directory(&root, &manifest.id))
            .filter(|dir| {
                std::fs::create_dir_all(dir)
                    .map_err(|error| eprintln!("plugin data {}: {error}", dir.display()))
                    .is_ok()
            });
        let mut command = Command::new("node");
        if let Some(dir) = &data_directory {
            command.env("VESTRA_PLUGIN_DATA_DIR", dir);
        }
        if let Some(info) = APP_INFO.get() {
            command.env("VESTRA_APP_INFO", info);
        }
        let mut child = command
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
            data_key,
            data_directory,
            temporary_data: trial,
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
    /// `plugin.dataDirectory`, when the player could create it.
    pub fn data_directory(&self) -> Option<&Path> {
        self.data_directory.as_deref()
    }
    pub fn allows(&self, capability: &str) -> bool {
        self.manifest.capabilities.iter().any(|c| c == capability)
    }
}
impl Drop for Plugin {
    fn drop(&mut self) {
        let _ = self.send(json!({"event":"runtime.stop","data":null}));
        let deadline = Instant::now() + Duration::from_millis(300);
        let mut exited = false;
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                exited = true;
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        if !exited {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        self.remove_temporary_data();
    }
}
impl Plugin {
    fn remove_temporary_data(&self) {
        if self.temporary_data
            && let Some(dir) = &self.data_directory
        {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}
/// `$VESTRA_PLUGIN_DATA`, else `plugin-data` in the application data
/// directory (`~/Library/Application Support/Vestra` on macOS,
/// `%APPDATA%\Vestra` on Windows).
pub fn data_root() -> Option<PathBuf> {
    std::env::var_os("VESTRA_PLUGIN_DATA")
        .map(PathBuf::from)
        .or_else(|| crate::platform::app_data_dir().map(|root| root.join("plugin-data")))
}
pub fn package_file(directory: &Path, relative: &str) -> Result<PathBuf> {
    if Path::new(relative).is_absolute() {
        bail!("package file must be relative");
    }
    let path = dunce::canonicalize(directory.join(relative))?;
    if !path.starts_with(directory) || !path.is_file() {
        bail!("package file must remain inside package directory");
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trial_data_is_removed_and_installed_data_kept() {
        let root = tempfile::tempdir().unwrap();
        let manifest = |id: &str| Manifest {
            id: id.into(),
            api_version: API.into(),
            entry: "main.mjs".into(),
            ..Default::default()
        };
        let package = tempfile::tempdir().unwrap();
        std::fs::write(package.path().join("main.mjs"), "export default () => {}").unwrap();
        for (id, trial) in [("a.plugin", false), ("a.plugin.trial.9", true)] {
            let dir = root.path().join(id);
            std::fs::create_dir_all(&dir).unwrap();
            let plugin = Plugin {
                manifest: manifest(id),
                directory: package.path().into(),
                data_key: if trial {
                    DataKey::Trial
                } else {
                    DataKey::Development(id.into())
                },
                data_directory: Some(dir.clone()),
                temporary_data: trial,
                subscriptions: BTreeSet::new(),
                // Any short-lived child; Node is there on every system.
                child: Command::new("node")
                    .arg("--version")
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .unwrap(),
                outbound: mpsc::sync_channel(1).0,
            };
            drop(plugin);
            assert_eq!(dir.exists(), !trial, "{id}");
        }
    }
}

#[cfg(test)]
mod data_key_tests {
    use super::*;

    #[test]
    fn data_keys_separate_instances_development_and_trials() {
        let root = Path::new("/data");
        let guid = "3f0c8a8e-5b7e-4c1f-9d0a-2b6f1c7e9a41";
        let instance = DataKey::Instance(guid.into());
        assert_eq!(instance.directory(root, "a.b"), root.join(guid));
        assert_eq!(
            instance.secret_service("a.b"),
            format!("vestra.plugin.{guid}")
        );
        let development = DataKey::Development("a.b".into());
        assert_eq!(
            development.directory(root, "a.b"),
            root.join("development").join("a.b")
        );
        assert_eq!(development.secret_service("a.b"), "vestra.dev.a.b");
        assert_eq!(
            DataKey::Trial.directory(root, "a.b.trial.7"),
            root.join("a.b.trial.7")
        );
        assert_eq!(DataKey::Trial.secret_service("a.b.trial.7"), "a.b.trial.7");
    }
}
