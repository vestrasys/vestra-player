//! A plugin's host-side state and its runner: its own Node process
//! (`runner.ts`, one JSONL pipe each way), or a member of the shared runner
//! (`shared_runner.rs`) for the built-ins the host trusts
//! (`Host::trust`).
pub(crate) mod reaper;
pub(crate) mod runtime_paths;
pub(crate) mod shared_runner;
pub(crate) mod unanswered;

use crate::host::Event;
use crate::process::shared_runner::{Member, MemberStop, SharedRunner};
use crate::process::unanswered::{Admission, Unanswered};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

pub const API: &str = "vestra/1";
/// The longest protocol line a plugin may write, newline included (1 MiB);
/// a longer one stops the plugin. A property value may be larger than one
/// line (`ui_values`: 32 MiB of text): the SDK sends a large whole write as
/// its first rows and `insert` patches of the rest, each line within this
/// (player 0.6.0, SDK 1.5.46; doc/architecture/large-lists.md §4). The SDK
/// repeats it (`MESSAGE_LIMIT`); src/tests/sdk/host-agreement.test.ts checks
/// they agree.
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

/// The event for a plugin's line (`message`, parsed on its reader thread):
/// a property write's large array is converted here, off the UI thread
/// (`ui_values::prepare_request`).
pub(crate) fn request(owner: u64, mut message: Value) -> Event {
    match crate::ui::ui_values::prepare_request(&mut message) {
        Some(prepared) => Event::Prepared(owner, message, prepared),
        None => Event::Request(owner, message),
    }
}
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
/// Why the player stops a plugin: `runtime.stop`'s `data.reason` (player
/// 0.6.4, SDK 1.5.54 `StopReason`; doc/architecture/README.md "Stop reasons").
/// The runner adds `host-lost` itself when its pipe to the player closes
/// without a stop; the player never sends that one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StopReason {
    /// The player quits (a plugin dropped with the host).
    #[default]
    Quit,
    /// The user disabled it (`packages.setEnabled { enabled: false }`, or
    /// an install with `enabled: false` over the running version).
    Disable,
    /// The user uninstalls it (`packages.uninstall`), or cancels its first
    /// install while it activates.
    Uninstall,
    /// It starts again from the same package (`packages.reload`).
    Reload,
    /// Another version replaces it: any install over the running package
    /// (an update, a reinstall, a `replace` from another source), or the
    /// previous version coming back when the new one's install is cancelled.
    Upgrade,
    /// Lazy activation stops it after a while without use (a park).
    Idle,
    /// A fault: an unreadable, oversized or malformed line, a malformed
    /// request, a full input queue, its process exiting, a failed or timed
    /// out activation, an uncaught exception in the shared runner.
    Failed,
}
impl StopReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Quit => "quit",
            Self::Disable => "disable",
            Self::Uninstall => "uninstall",
            Self::Reload => "reload",
            Self::Upgrade => "upgrade",
            Self::Idle => "idle",
            Self::Failed => "failed",
        }
    }
    /// `{"reason":"<r>"}`: the data of `runtime.stop` and `runner.stop`.
    pub fn data(self) -> Value {
        json!({ "reason": self.as_str() })
    }
}
/// The `runtime.stop` line for a plugin stopped for `reason`.
pub fn stop_message(reason: StopReason) -> Value {
    json!({"event":"runtime.stop","data":reason.data()})
}
/// The V8 flag that caps a process's JavaScript heap at `mib` MiB (the
/// shared runner's is `shared_runner::MAX_OLD_SPACE`).
pub fn heap_flag(mib: u32) -> String {
    format!("--max-old-space-size={mib}")
}
/// Bounded per-plugin output queue. Source state alone emits ~10 events/s while playing,
/// so a small queue disables healthy plugins after a brief event-loop stall.
const OUTBOUND_CAPACITY: usize = 1024;
/// A line for an own process's writer, and the counter it decrements once
/// written (a refusal's, `Unanswered::refusals`).
type Outbound = (Value, Option<Arc<AtomicUsize>>);
/// `plugin.app`, set once at start-up and given to every plugin.
static APP_INFO: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
pub fn set_app_info(info: &Value) {
    let _ = APP_INFO.set(info.clone());
}
/// Where a plugin runs.
pub enum Placement<'a> {
    /// Its own Node process, its JavaScript heap capped at `heap_mib` MiB
    /// when set (`--max-old-space-size`, the supervisor's heap cap,
    /// supervisor.md §8).
    Own { heap_mib: Option<u32> },
    /// A member of this shared runner.
    Shared(&'a mut SharedRunner),
}
enum Runner {
    Own {
        child: Child,
        /// Its writer's queue; the reader holds it weakly (refusals), so
        /// the writer ends with the plugin's stop even while a child of
        /// the plugin keeps its stdout open.
        outbound: Arc<mpsc::SyncSender<Outbound>>,
        /// Its stderr was read to the end (`forward_stderr`).
        stderr_done: Arc<AtomicBool>,
    },
    Shared(Member),
    /// Its stop began (`Plugin::begin_stop`): the process or member is the
    /// returned [`Stop`]'s.
    Stopped,
}
/// What a plugin's stop leaves to wait for (`Plugin::begin_stop`): its own
/// process until it exits, killed once its grace is over; or its
/// shared-runner member until the runner reports it gone
/// ([`MemberStop`]). Polled by the plugin reaper (`reaper.rs`,
/// supervisor.md §1), or waited for where the plugin is dropped.
pub enum Stop {
    Own {
        child: Child,
        /// Kept until the process is gone: its stdin closes after the stop
        /// line, not before (a closed pipe ends it as `host-lost`).
        _outbound: Arc<mpsc::SyncSender<Outbound>>,
        deadline: Instant,
        /// Killed at its deadline; waited for without blocking after that.
        killed: bool,
        /// Its exit code once it is gone (`Event::Reaped`: the supervisor
        /// tells a heap abort by it, supervisor.md §7).
        code: Option<i32>,
        /// When it was seen gone.
        exited: Option<Instant>,
        /// Its stderr was read to the end: everything it wrote there was
        /// seen (V8's heap abort line, `Event::OutOfMemory`) before it is
        /// reported gone, or [`STDERR_WAIT`] after it went (a child of the
        /// plugin may hold the pipe open).
        stderr_done: Arc<AtomicBool>,
    },
    Member(MemberStop),
    Gone,
}
impl Stop {
    /// Without blocking: whether it is over at `now`. An own process past
    /// its grace is killed here once, then polled until it is gone (never
    /// waited for: one process stuck in the kernel must not hold up every
    /// other stop on the reaper's one thread).
    pub fn poll(&mut self, now: Instant) -> bool {
        match self {
            Stop::Own {
                child,
                deadline,
                killed,
                code,
                exited,
                stderr_done,
                ..
            } => {
                if exited.is_none() {
                    if let Some(status) = child.try_wait().ok().flatten() {
                        *code = exit_code(status);
                        *exited = Some(now);
                    } else if now >= *deadline && !*killed {
                        *killed = true;
                        let _ = child.kill();
                        if let Some(status) = child.try_wait().ok().flatten() {
                            *code = exit_code(status);
                            *exited = Some(now);
                        }
                    }
                }
                exited.is_some_and(|at| {
                    stderr_done.load(Ordering::Acquire)
                        || now.saturating_duration_since(at) >= STDERR_WAIT
                })
            }
            Stop::Member(member) => member.poll(now),
            Stop::Gone => true,
        }
    }
    /// The exit code of an own process that is gone (`None`: a member, a
    /// process not gone yet, or one the system gave no code for).
    pub fn code(&self) -> Option<i32> {
        match self {
            Stop::Own { code, .. } => *code,
            _ => None,
        }
    }
    /// Its deadline moves to `deadline` if that is sooner: a quit's one
    /// deadline, or a kill now (supervisor.md §9).
    pub fn hasten(&mut self, deadline: Instant) {
        if let Stop::Own { deadline: own, .. } = self
            && deadline < *own
        {
            *own = deadline;
        }
    }
    /// Blocking until it is over (`Drop for Plugin`: at quit, each plugin
    /// dropped with the host).
    pub fn wait(self) {
        match self {
            Stop::Own {
                mut child,
                deadline,
                _outbound,
                ..
            } => {
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
            Stop::Member(member) => member.wait(),
            Stop::Gone => {}
        }
    }
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
    /// What dropping it tells the plugin (`set_stop_reason`); `quit` for a
    /// plugin dropped with the host.
    stop_reason: StopReason,
    /// Its unanswered requests, which its reader counts and refuses past
    /// the limits (`unanswered.rs`); `answered` when the host answers.
    unanswered: Arc<Unanswered>,
    /// Its process ended by itself (`set_exited`): no `runtime.stop` is
    /// queued for it.
    exited: bool,
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
            Placement::Own { heap_mib: None },
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
        let unanswered = Unanswered::new(&manifest.id);
        let runner = match placement {
            Placement::Own { heap_mib } => own_process(
                paths,
                &directory,
                data_directory.as_deref(),
                heap_mib,
                generation,
                events,
                unanswered.clone(),
            )?,
            Placement::Shared(shared) => Runner::Shared(shared.start(
                generation,
                &manifest.id,
                &directory,
                data_directory.as_deref(),
                APP_INFO.get().unwrap_or(&Value::Null),
                unanswered.clone(),
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
            stop_reason: StopReason::Quit,
            exited: false,
            unanswered,
        })
    }
    /// The host answered its request `id` (`Host::respond`): it no longer
    /// counts against the limits.
    pub fn answered(&self, id: u64) {
        self.unanswered.answered(id);
    }
    pub fn send(&mut self, message: Value) -> Result<()> {
        match &self.runner {
            Runner::Own { outbound, .. } => match outbound.try_send((message, None)) {
                Ok(()) => Ok(()),
                // Its writer stopped: the pipe broke (the process is gone or
                // going), and the writer reported that (`RuntimeFault`), as
                // its reader reports the exit. Those drive the stop and the
                // class's restart rule (supervisor.md §7); an error here
                // would stop it for good before they arrive (as
                // `shared_runner::Member::send` does for a member).
                Err(mpsc::TrySendError::Disconnected(_)) => Ok(()),
                Err(mpsc::TrySendError::Full(_)) => {
                    bail!("plugin output backpressure: {OUTBOUND_CAPACITY} messages queued")
                }
            },
            Runner::Shared(member) => member.send(message),
            Runner::Stopped => bail!("plugin stopped"),
        }
    }
    /// The shared runner this plugin runs in, if it does.
    pub fn shared_runner(&self) -> Option<crate::process::shared_runner::RunnerId> {
        match &self.runner {
            Runner::Own { .. } | Runner::Stopped => None,
            Runner::Shared(member) => Some(member.runner()),
        }
    }
    /// Begin stopping it: its catalog goes and its `runtime.stop` (with
    /// `set_stop_reason`'s reason) is sent; what is left to wait for is
    /// the returned [`Stop`] (`set_stop_wait`'s grace for an own
    /// process). Dropping the plugin afterwards waits for nothing.
    pub fn begin_stop(&mut self) -> Stop {
        if let Some(catalog) = self.catalog.take() {
            crate::i18n::unregister(&self.directory, &catalog);
        }
        match std::mem::replace(&mut self.runner, Runner::Stopped) {
            // The runner lives on; past the same 300 ms the member counts as
            // leaked, and the wait goes on until the runner reports it gone
            // (at most 6 s, `MemberStop`).
            Runner::Shared(member) => Stop::Member(member.begin_stop(self.stop_reason)),
            Runner::Own {
                child,
                outbound,
                stderr_done,
            } => {
                // Not delivered when its queue is full or its pipe closed
                // (a `failed` stop, often): it is killed after the wait.
                if !self.exited {
                    let _ = outbound.try_send((stop_message(self.stop_reason), None));
                }
                Stop::Own {
                    child,
                    _outbound: outbound,
                    deadline: Instant::now() + self.stop_wait,
                    killed: false,
                    code: None,
                    exited: None,
                    stderr_done,
                }
            }
            Runner::Stopped => Stop::Gone,
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
    /// park waits longer, `host/supervisor/` `PARK_WAIT`).
    pub fn set_stop_wait(&mut self, wait: Duration) {
        self.stop_wait = wait;
    }
    /// Why it stops: the reason its `runtime.stop` carries when it is
    /// dropped (`Host::stop`).
    pub fn set_stop_reason(&mut self, reason: StopReason) {
        self.stop_reason = reason;
    }
    /// Its process ended by itself (its stdout closed): its stop sends no
    /// `runtime.stop` to the closed pipe; the reaper only waits for it.
    pub fn set_exited(&mut self) {
        self.exited = true;
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
/// An own process: `node [--max-old-space-size=<heap_mib>] runner.js
/// <directory>`, the plugin's data directory, `plugin.app` and the locale
/// in its environment. A heap past the cap ends the process with V8's heap
/// abort, which the supervisor reports as "ran out of memory" (supervisor.md §7).
fn own_process(
    paths: &crate::process::runtime_paths::RuntimePaths,
    directory: &Path,
    data_directory: Option<&Path>,
    heap_mib: Option<u32>,
    generation: u64,
    events: mpsc::Sender<Event>,
    unanswered: Arc<Unanswered>,
) -> Result<Runner> {
    let mut command = paths.node_command();
    if let Some(mib) = heap_mib {
        command.arg(heap_flag(mib));
    }
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
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| paths.node_failure())?;
    // Its stderr goes on to the player's, as when inherited, and to the
    // plugin log with `[<id>]` in front; read here so V8's heap abort is
    // seen (supervisor.md §7, §8).
    let stderr_done = Arc::new(AtomicBool::new(false));
    if let Some(stderr) = child.stderr.take() {
        let done = stderr_done.clone();
        let events = events.clone();
        let prefix = format!("[{}]", unanswered.plugin());
        thread::spawn(move || {
            forward_stderr(
                BufReader::new(stderr),
                Some(&prefix),
                Some((generation, &events)),
            );
            done.store(true, Ordering::Release);
        });
    } else {
        stderr_done.store(true, Ordering::Release);
    }
    let mut stdin = child.stdin.take().context("plugin stdin")?;
    let (outbound, output) = mpsc::sync_channel::<Outbound>(OUTBOUND_CAPACITY);
    let write_events = events.clone();
    thread::spawn(move || {
        for (message, written) in output {
            if let Some(written) = written {
                written.fetch_sub(1, Ordering::AcqRel);
            }
            // One write per line: `to_writer` on the unbuffered pipe made a
            // system call per JSON token (stress measurement 2026-11-11).
            let mut write = || -> Result<()> {
                let mut line = serde_json::to_vec(&message)?;
                line.push(b'\n');
                stdin.write_all(&line)?;
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
    let outbound = Arc::new(outbound);
    let refuse = Arc::downgrade(&outbound);
    thread::spawn(move || {
        read_plugin(
            BufReader::new(stdout),
            generation,
            &events,
            |message, bytes| match refuse.upgrade() {
                Some(outbound) => admit(&unanswered, &outbound, message, bytes),
                // Stopped: its lines are no longer the host's.
                None => true,
            },
        );
    });
    Ok(Runner::Own {
        child,
        outbound,
        stderr_done,
    })
}
/// What V8 writes to stderr when a heap allocation fails past the cap
/// (`FATAL ERROR: … JavaScript heap out of memory`), before it aborts.
pub const HEAP_ABORT_LINE: &str = "JavaScript heap out of memory";
/// How long a reap waits for the end of a gone process's stderr.
pub(crate) const STDERR_WAIT: Duration = Duration::from_millis(100);
/// A runner's stderr (an own process's, or the shared runner's), copied on
/// to the player's line by line and to the plugin log (`diag::plugin_line`,
/// with `prefix` in front of lines without it; the shared runner prefixes
/// its own). With `oom`, V8's heap abort line is reported once as
/// `Event::OutOfMemory` for that generation.
pub(crate) fn forward_stderr(
    mut reader: impl BufRead,
    prefix: Option<&str>,
    oom: Option<(u64, &mpsc::Sender<Event>)>,
) {
    let mut told = false;
    loop {
        let mut bytes = Vec::new();
        let read = Read::by_ref(&mut reader)
            .take(64 * 1024)
            .read_until(b'\n', &mut bytes);
        match read {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let _ = std::io::stderr().lock().write_all(&bytes);
                crate::diag::plugin_line(prefix, &bytes);
                if let Some((generation, events)) = oom
                    && !told
                    && String::from_utf8_lossy(&bytes).contains(HEAP_ABORT_LINE)
                {
                    told = true;
                    let _ = events.send(Event::OutOfMemory(generation));
                }
            }
        }
    }
}
/// A gone process's exit code; on Unix a signal is `128 + signal`, as a
/// shell reports it (V8's abort: 134 everywhere, measured on Windows with
/// Node 26).
fn exit_code(status: std::process::ExitStatus) -> Option<i32> {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        status
            .code()
            .or_else(|| status.signal().map(|signal| 128 + signal))
    }
    #[cfg(not(unix))]
    {
        status.code()
    }
}
/// Whether an own process's line goes on to the UI thread: a request past
/// the limits is answered on its writer's queue instead (`unanswered.rs`).
fn admit(
    unanswered: &Unanswered,
    outbound: &mpsc::SyncSender<Outbound>,
    message: &Value,
    bytes: usize,
) -> bool {
    match unanswered.admit(message, bytes) {
        Admission::Forward => true,
        Admission::Refuse(answer) => {
            let written = unanswered.refusals().clone();
            if outbound.try_send((answer, Some(written))).is_err() {
                unanswered.refusals().fetch_sub(1, Ordering::AcqRel);
            }
            false
        }
        Admission::Drop => false,
    }
}
/// An own process's stdout, line by line, as events for `generation`,
/// until it closes. After an oversized or unparsable line (the plugin is
/// stopped for it) or once the host no longer listens (quitting: its
/// receiver goes first), the rest is read and dropped rather than the pipe
/// closed: a closed pipe fails the plugin's next write, and its runner
/// would stop it as `host-lost` before the `runtime.stop` that follows
/// (`failed`, `quit`) arrives. `admit` (the line, its bytes) says whether a
/// line goes on: a request past the limits is answered at once instead.
fn read_plugin(
    mut reader: impl BufRead,
    generation: u64,
    events: &mpsc::Sender<Event>,
    mut admit: impl FnMut(&Value, usize) -> bool,
) {
    let mut forwarding = true;
    loop {
        // Protocol lines are capped at MAX_MESSAGE_BYTES.
        let mut bytes = Vec::new();
        let read = Read::by_ref(&mut reader)
            .take(MAX_MESSAGE_BYTES as u64 + 1)
            .read_until(b'\n', &mut bytes);
        match read {
            Ok(0) | Err(_) => break,
            Ok(_) if !forwarding => {}
            Ok(_) if bytes.len() > MAX_MESSAGE_BYTES => {
                let _ = events.send(Event::RuntimeFault(
                    generation,
                    format!("protocol message exceeds {} MiB", MAX_MESSAGE_BYTES >> 20),
                ));
                forwarding = false;
            }
            Ok(_) => match serde_json::from_slice(&bytes) {
                Ok(message) => {
                    if admit(&message, bytes.len()) {
                        forwarding = events.send(request(generation, message)).is_ok();
                    }
                }
                Err(e) => {
                    let _ = events.send(Event::RuntimeFault(
                        generation,
                        format!("invalid protocol: {e}"),
                    ));
                    forwarding = false;
                }
            },
        }
    }
    let _ = events.send(Event::Exited(generation));
}
impl Drop for Plugin {
    /// A plugin dropped where it is (at quit, with the host) is stopped
    /// and waited for here; one the host stops is handed to the plugin
    /// reaper instead (`Host::stop`), whose `begin_stop` leaves nothing.
    fn drop(&mut self) {
        self.begin_stop().wait();
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

#[cfg(test)]
mod stop_tests {
    use super::*;
    use std::io::Cursor;

    /// A development plugin that writes a request every 10 ms and, when it
    /// stops, writes its `DisposeInfo.reason` to `reason.txt` beside it.
    fn chatty(root: &Path) -> PathBuf {
        let directory = root.join("chatty");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("manifest.json"),
            json!({"id":"test.chatty","version":"1.0.0","apiVersion":API,
                "entry":"main.js","capabilities":["ui.osd"]})
            .to_string(),
        )
        .unwrap();
        std::fs::write(directory.join("package.json"), r#"{"type":"module"}"#).unwrap();
        std::fs::write(
            directory.join("main.js"),
            "import { writeFileSync } from \"node:fs\";\n\
             export default async function activate(plugin) {\n\
               setInterval(() => plugin.osd.show(\"x\").catch(() => {}), 10);\n\
               plugin.onDispose((info) =>\n\
                 writeFileSync(new URL(\"reason.txt\", import.meta.url), String(info?.reason)));\n\
             }\n",
        )
        .unwrap();
        directory
    }

    /// GUI quit (review of 0.6.4): the host's event receiver goes before the
    /// host drops its plugins (main.rs: the tick timer owns it). A reader
    /// thread that stopped reading then would let a plugin that writes get
    /// EPIPE and stop as `host-lost` before its `runtime.stop`; it keeps
    /// draining instead, so the plugin hears `quit`.
    #[test]
    #[ignore = "needs Node 26: run with --include-ignored (ci:local)"]
    fn a_plugin_still_hears_quit_after_the_host_stopped_listening() {
        let root = tempfile::tempdir().unwrap();
        let directory = dunce::canonicalize(chatty(root.path())).unwrap();
        let manifest = crate::packages::manifest::read_development(&directory).unwrap();
        let (events, receiver) = mpsc::channel();
        let runner = own_process(
            crate::process::runtime_paths::get(),
            &directory,
            None,
            None,
            1,
            events,
            Unanswered::new(&manifest.id),
        )
        .unwrap();
        let mut plugin = Plugin {
            manifest,
            directory: directory.clone(),
            catalog: None,
            data_key: DataKey::Development("test.chatty".into()),
            data_directory: None,
            subscriptions: BTreeMap::new(),
            listeners: BTreeMap::new(),
            runner,
            stop_wait: Duration::from_millis(1000),
            stop_reason: StopReason::Quit,
            exited: false,
            unanswered: Unanswered::new("test"),
        };
        // Answer like the host until it writes on its own.
        let mut shows = 0;
        let deadline = Instant::now() + Duration::from_secs(15);
        while shows < 5 {
            assert!(Instant::now() < deadline, "the plugin never wrote");
            let Ok(Event::Request(_, message)) = receiver.recv_timeout(Duration::from_secs(15))
            else {
                continue;
            };
            if message["method"] == "osd.show" {
                shows += 1;
            }
            if let Some(id) = message["id"].as_u64() {
                plugin.send(json!({"id":id,"result":null})).unwrap();
            }
        }
        // The host stops listening; the plugin goes on writing.
        drop(receiver);
        thread::sleep(Duration::from_millis(300));
        drop(plugin);
        let reason = std::fs::read_to_string(directory.join("reason.txt")).unwrap();
        assert_eq!(reason, "quit");
    }

    /// Review of the crash-restart change: a plugin that crashes while the
    /// host writes to it. Once its writer found the pipe closed (and
    /// reported it, `RuntimeFault`), `send` drops what follows instead of
    /// failing: a failed send stopped it for good (`transport_failed`)
    /// before its exit could restart it by its class's rule.
    #[test]
    #[ignore = "needs Node 26: run with --include-ignored (ci:local)"]
    fn a_send_to_a_plugin_that_crashed_is_dropped_not_a_failure() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("crashing");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("manifest.json"),
            json!({"id":"test.crashing","version":"1.0.0","apiVersion":API,
                "entry":"main.js","capabilities":[]})
            .to_string(),
        )
        .unwrap();
        std::fs::write(directory.join("package.json"), r#"{"type":"module"}"#).unwrap();
        std::fs::write(
            directory.join("main.js"),
            // At its import: its activation would wait for answers from
            // the host, which this test does not give.
            "setTimeout(() => process.exit(7), 50);\nexport default function activate() {}\n",
        )
        .unwrap();
        let directory = dunce::canonicalize(directory).unwrap();
        let manifest = crate::packages::manifest::read_development(&directory).unwrap();
        let (events, receiver) = mpsc::channel();
        let runner = own_process(
            crate::process::runtime_paths::get(),
            &directory,
            None,
            None,
            1,
            events,
            Unanswered::new(&manifest.id),
        )
        .unwrap();
        let mut plugin = Plugin {
            manifest,
            directory,
            catalog: None,
            data_key: DataKey::Development("test.crashing".into()),
            data_directory: None,
            subscriptions: BTreeMap::new(),
            listeners: BTreeMap::new(),
            runner,
            stop_wait: Duration::from_millis(300),
            stop_reason: StopReason::Quit,
            exited: false,
            unanswered: Unanswered::new("test"),
        };
        // The host goes on writing (events for it) through the crash.
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut writer_failed = false;
        let mut exited = false;
        while !(exited && writer_failed) && Instant::now() < deadline {
            plugin
                .send(json!({"event":"host.tick","data":{"pad":"x".repeat(4096)}}))
                .expect("a send to a crashed plugin is dropped, not an error");
            while let Ok(event) = receiver.try_recv() {
                match event {
                    Event::Exited(1) => exited = true,
                    Event::RuntimeFault(1, message) if message.contains("stdin") => {
                        writer_failed = true;
                    }
                    _ => {}
                }
            }
            thread::sleep(Duration::from_millis(2));
        }
        assert!(exited, "the plugin never exited");
        assert!(writer_failed, "the host never wrote to the closed pipe");
        // Its writer is gone now: what follows is dropped too.
        for _ in 0..100 {
            plugin.send(json!({"event":"host.tick"})).unwrap();
        }
    }

    #[test]
    fn the_reader_drains_to_the_end_after_a_fault_or_a_gone_host() {
        let big = format!("{}\n", "x".repeat(MAX_MESSAGE_BYTES + 10));
        for (input, fault) in [
            (format!("{{\"id\":1}}\n{big}{{\"id\":2}}\n"), Some("1 MiB")),
            (
                "{\"id\":1}\nnot json\n{\"id\":2}\n".to_owned(),
                Some("invalid"),
            ),
            ("{\"id\":1}\n{\"id\":2}\n".to_owned(), None),
        ] {
            let mut cursor = Cursor::new(input.into_bytes());
            let (events, received) = mpsc::channel();
            read_plugin(&mut cursor, 3, &events, |_, _| true);
            drop(events);
            assert_eq!(cursor.position() as usize, cursor.get_ref().len());
            let received: Vec<_> = received.into_iter().collect();
            assert!(matches!(&received[0], Event::Request(3, m) if m["id"] == 1));
            match fault {
                // The fault, nothing after it, then the exit at the end.
                Some(text) => {
                    assert!(matches!(&received[1], Event::RuntimeFault(3, m) if m.contains(text)));
                    assert_eq!(received.len(), 3);
                }
                None => {
                    assert!(matches!(&received[1], Event::Request(3, m) if m["id"] == 2));
                    assert_eq!(received.len(), 3);
                }
            }
            assert!(matches!(received.last(), Some(Event::Exited(3))));
        }
        // The backstop (`unanswered.rs`): past the limit a request is
        // answered on the writer's queue, not forwarded; notifications pass.
        let lines: String = (0..400)
            .map(|id| format!("{{\"id\":{id},\"method\":\"osd.show\"}}\n"))
            .chain(["{\"event\":\"runtime.ready\"}\n".to_owned()])
            .collect();
        let unanswered = Unanswered::new("test.flood");
        let (outbound, output) = mpsc::sync_channel::<Outbound>(OUTBOUND_CAPACITY);
        let (events, received) = mpsc::channel();
        read_plugin(
            Cursor::new(lines.into_bytes()),
            3,
            &events,
            |message, bytes| admit(&unanswered, &outbound, message, bytes),
        );
        drop(events);
        let received: Vec<_> = received.into_iter().collect();
        let limit = unanswered::MAX_UNANSWERED_REQUESTS;
        assert_eq!(received.len(), limit + 2, "requests, ready, exit");
        assert!(matches!(&received[limit], Event::Request(3, m) if m["event"] == "runtime.ready"));
        let refusals: Vec<_> = output.try_iter().collect();
        assert_eq!(refusals.len(), 400 - limit);
        assert_eq!(refusals[0].0["id"], limit);
        assert_eq!(refusals[0].0["error"]["code"], "limit");
        assert_eq!(unanswered.refusals().load(Ordering::Acquire), 400 - limit);
        for (_, written) in refusals {
            written.unwrap().fetch_sub(1, Ordering::AcqRel);
        }
        assert_eq!(unanswered.refusals().load(Ordering::Acquire), 0);
        // Nobody listens: it still reads everything (the plugin's writes
        // never fail).
        let mut cursor = Cursor::new(b"{\"id\":1}\n{\"id\":2}\n".to_vec());
        let (events, received) = mpsc::channel();
        drop(received);
        read_plugin(&mut cursor, 3, &events, |_, _| true);
        assert_eq!(cursor.position(), 18);
    }
}
