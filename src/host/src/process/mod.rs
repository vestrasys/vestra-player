//! A plugin's host-side state and its runner: its own Node process
//! (`runner.ts`, one JSONL pipe each way), or a member of the shared runner
//! (`shared_runner.rs`) for the built-ins the host trusts
//! (`Host::trust`).
pub(crate) mod runtime_paths;
pub(crate) mod shared_runner;

use crate::host::Event;
use crate::process::shared_runner::{Member, SharedRunner};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

pub const API: &str = "vestra/1";
/// The longest protocol line a plugin may write, newline included (1 MiB);
/// a longer one stops the plugin. The SDK repeats it (`MESSAGE_LIMIT`);
/// src/tests/sdk/host-agreement.test.ts checks they agree.
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
pub use crate::packages::manifest::Manifest;

/// Whose data directory and secrets a plugin process gets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DataKey {
    /// An installed instance: `plugin-data/<GUID>/`, secrets `vestra.plugin.<GUID>`.
    Instance(String),
    /// A development load (`--plugin`): `plugin-data/development/<id>/`.
    Development(String),
}
impl DataKey {
    /// The data directory under the data root.
    pub fn directory(&self, root: &Path) -> PathBuf {
        match self {
            Self::Instance(guid) => root.join(guid),
            Self::Development(id) => root.join("development").join(id),
        }
    }
    /// The secret store service name.
    pub fn secret_service(&self) -> String {
        match self {
            Self::Instance(guid) => format!("vestra.plugin.{guid}"),
            Self::Development(id) => format!("vestra.dev.{id}"),
        }
    }
}
/// Bounded per-plugin output queue. Source state alone emits ~10 events/s while playing,
/// so a small queue disables healthy plugins after a brief event-loop stall.
const OUTBOUND_CAPACITY: usize = 1024;
/// `plugin.app`, set once at start-up and given to every plugin.
static APP_INFO: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
pub fn set_app_info(info: &Value) {
    let _ = APP_INFO.set(info.clone());
}
/// Where a plugin runs.
pub enum Placement<'a> {
    /// Its own Node process.
    Own,
    /// A member of this shared runner.
    Shared(&'a mut SharedRunner),
}
enum Runner {
    Own {
        child: Child,
        outbound: mpsc::SyncSender<Value>,
    },
    Shared(Member),
}
pub struct Plugin {
    pub manifest: Manifest,
    pub directory: PathBuf,
    /// This package's translations for the interface language (`None`:
    /// English, or the package has none). Its `.slint` files and the
    /// strings it hands the player (menus, OSD) are translated with it.
    pub catalog: Option<std::sync::Arc<crate::i18n::Catalog>>,
    pub data_key: DataKey,
    /// `plugin.dataDirectory`.
    data_directory: Option<PathBuf>,
    /// Its events: `true` for a listener of its own, `false` for a
    /// mirror (the SDK's copy of the state, `events.subscribe { mirror }`,
    /// SDK 1.5.38), which holds nothing for lazy activation.
    pub subscriptions: BTreeMap<String, bool>,
    /// What listens, as the SDK describes it (`events.subscribe { why }`,
    /// SDK 1.5.40: the listener and where the plugin added it), by event:
    /// for the log line that says why a plugin keeps running.
    pub listeners: BTreeMap<String, Vec<String>>,
    runner: Runner,
    /// How long dropping it waits for its own process (`set_stop_wait`).
    stop_wait: Duration,
}
impl Plugin {
    /// A development load (`--plugin`): always its own process.
    pub fn load(directory: &Path, generation: u64, events: mpsc::Sender<Event>) -> Result<Self> {
        let directory = dunce::canonicalize(directory).context("plugin directory")?;
        let manifest = crate::packages::manifest::read_development(&directory)?;
        let key = DataKey::Development(manifest.id.clone());
        Self::load_manifest(
            &directory,
            generation,
            events,
            manifest,
            key,
            Placement::Own,
        )
    }
    pub fn load_manifest(
        directory: &Path,
        generation: u64,
        events: mpsc::Sender<Event>,
        manifest: Manifest,
        data_key: DataKey,
        placement: Placement<'_>,
    ) -> Result<Self> {
        let directory = dunce::canonicalize(directory)?;
        if manifest.api_version != API {
            bail!("unsupported apiVersion: {}", manifest.api_version);
        }
        if !crate::packages::manifest::valid_id(&manifest.id) {
            bail!("invalid plugin id");
        }
        if manifest
            .capabilities
            .iter()
            .any(|c| !crate::capabilities::known(c))
        {
            bail!("unknown required capability");
        }
        package_file(&directory, &manifest.entry)?;
        let paths = crate::process::runtime_paths::get();
        // Per installed instance or development id (see `DataKey`).
        // Without one the SDK reports `unavailable`; loading continues.
        let data_directory = data_root()
            .map(|root| data_key.directory(&root))
            .filter(|dir| {
                std::fs::create_dir_all(dir)
                    .map_err(|error| diag!("plugin data {}: {error}", dir.display()))
                    .is_ok()
            });
        let runner = match placement {
            Placement::Own => own_process(
                paths,
                &directory,
                data_directory.as_deref(),
                generation,
                events,
            )?,
            Placement::Shared(shared) => Runner::Shared(shared.start(
                generation,
                &manifest.id,
                &directory,
                data_directory.as_deref(),
                APP_INFO.get().unwrap_or(&Value::Null),
            )?),
        };
        let catalog = crate::i18n::load(&directory);
        if let Some(catalog) = &catalog {
            crate::i18n::register(&directory, catalog.clone());
        }
        Ok(Self {
            manifest,
            directory,
            catalog,
            data_key,
            data_directory,
            subscriptions: BTreeMap::new(),
            listeners: BTreeMap::new(),
            runner,
            stop_wait: Duration::from_millis(300),
        })
    }
    pub fn send(&mut self, message: Value) -> Result<()> {
        match &self.runner {
            Runner::Own { outbound, .. } => outbound.try_send(message).map_err(|error| {
                anyhow::anyhow!("plugin output backpressure/disconnected: {error}")
            }),
            Runner::Shared(member) => member.send(message),
        }
    }
    /// The shared runner this plugin runs in, if it does.
    pub fn shared_runner(&self) -> Option<crate::process::shared_runner::RunnerId> {
        match &self.runner {
            Runner::Own { .. } => None,
            Runner::Shared(member) => Some(member.runner()),
        }
    }
    /// `plugin.dataDirectory`, when the player could create it.
    pub fn data_directory(&self) -> Option<&Path> {
        self.data_directory.as_deref()
    }
    pub fn allows(&self, capability: &str) -> bool {
        self.manifest.capabilities.iter().any(|c| c == capability)
    }
    /// A string this plugin declared (menu title, page or tab title), in the
    /// interface language.
    pub fn text(&self, source: &str) -> String {
        self.catalog
            .as_ref()
            .map_or(source, |catalog| catalog.text(source))
            .to_owned()
    }
    /// Its package, for what a running plugin and a parked one (lazy
    /// activation) register alike.
    pub fn origin(&self) -> crate::host::Origin {
        crate::host::Origin {
            id: self.manifest.id.clone(),
            directory: self.directory.clone(),
            catalog: self.catalog.clone(),
            data_directory: self.data_directory.clone(),
        }
    }
    /// How long a stop waits for its own process before killing it (a
    /// park waits longer, `host/lazy.rs`).
    pub fn set_stop_wait(&mut self, wait: Duration) {
        self.stop_wait = wait;
    }
    /// Text values in the interface language, with this plugin's catalog
    /// (`i18n::text`).
    pub fn texts(&self) -> crate::i18n::text::Texts<'_> {
        crate::i18n::text::Texts {
            catalog: self.catalog.as_deref(),
            locale: crate::i18n::locale(),
        }
    }
}
/// An own process: `node runner.js <directory>`, the plugin's data
/// directory, `plugin.app` and the locale in its environment.
fn own_process(
    paths: &crate::process::runtime_paths::RuntimePaths,
    directory: &Path,
    data_directory: Option<&Path>,
    generation: u64,
    events: mpsc::Sender<Event>,
) -> Result<Runner> {
    let mut command = paths.node_command();
    if let Some(dir) = data_directory {
        command.env("VESTRA_PLUGIN_DATA_DIR", dir);
    }
    if let Some(info) = APP_INFO.get() {
        command.env("VESTRA_APP_INFO", info.to_string());
    }
    // `plugin.i18n.locale`: fixed for the run.
    command.env("VESTRA_LOCALE", crate::i18n::locale());
    let mut child = command
        .arg(&paths.runner)
        .arg(directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| paths.node_failure())?;
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
                .take(MAX_MESSAGE_BYTES as u64 + 1)
                .read_until(b'\n', &mut bytes);
            match read {
                Ok(0) | Err(_) => break,
                Ok(_) if bytes.len() > MAX_MESSAGE_BYTES => {
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
    Ok(Runner::Own { child, outbound })
}
impl Drop for Plugin {
    fn drop(&mut self) {
        if let Some(catalog) = &self.catalog {
            crate::i18n::unregister(&self.directory, catalog);
        }
        match &mut self.runner {
            // The runner lives on; past the same 300 ms the member counts as
            // leaked, and the wait goes on until the runner reports it gone
            // (at most 6 s, `Member::stop`).
            Runner::Shared(member) => member.stop(),
            Runner::Own { child, outbound } => {
                let _ = outbound.try_send(json!({"event":"runtime.stop","data":null}));
                let deadline = Instant::now() + self.stop_wait;
                let mut exited = false;
                while Instant::now() < deadline {
                    if child.try_wait().ok().flatten().is_some() {
                        exited = true;
                        break;
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                if !exited {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
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
mod data_key_tests {
    use super::*;

    #[test]
    fn data_keys_separate_instances_and_development_loads() {
        let root = Path::new("/data");
        let guid = "3f0c8a8e-5b7e-4c1f-9d0a-2b6f1c7e9a41";
        let instance = DataKey::Instance(guid.into());
        assert_eq!(instance.directory(root), root.join(guid));
        assert_eq!(instance.secret_service(), format!("vestra.plugin.{guid}"));
        let development = DataKey::Development("a.b".into());
        assert_eq!(
            development.directory(root),
            root.join("development").join("a.b")
        );
        assert_eq!(development.secret_service(), "vestra.dev.a.b");
    }
}
