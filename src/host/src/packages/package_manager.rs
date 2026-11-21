//! Finite trusted-package lifecycle. This is not a sandbox or dependency solver.
#![forbid(unsafe_code)]
use crate::host::revised::Revised;
use crate::host::supervisor::checkpoint;
use crate::host::*;
use crate::packages::package_stage::Candidate;
use crate::packages::package_store::{Instance, Source, Store, StoredPackage};
use crate::packages::{package_stage, package_store};
use crate::playback::backend::Command;
use crate::process::{Plugin, shared_runner};
use crate::supervisor::{
    Busy, Class, JobKind, JobMode, JobResult, ListState, Origin, Pid, Priority, STUCK_RELOAD, Then,
    UnitId, UnitSpec, activation,
};
use crate::{platform, process};
use anyhow::Result;
use process::DataKey;
use serde_json::{Value, json};
use std::collections::{BTreeSet, VecDeque};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// A guest window's library (doc/architecture/guest-windows.md §4): the
/// main one read-only, with the package versions it refers to leased
/// (`guest::lease`) before any of them runs. The catalog is read again
/// after each lease until it names nothing the lease does not: the main
/// player commits a new catalog before it reads the leases, so a version
/// it removes is one this guest does not run.
fn open_leased(root: PathBuf, local: PathBuf) -> Result<Store> {
    let mut leased = BTreeSet::new();
    let mut written = false;
    for _ in 0..8 {
        let store = Store::open_read_only(root.clone(), local.clone())?;
        let referenced = store.referenced();
        if written && referenced.is_subset(&leased) {
            return Ok(store);
        }
        leased.extend(referenced);
        crate::guest::lease(&leased)?;
        written = true;
    }
    diag!("package library: the catalog kept changing; the guest runs from its last reading");
    let store = Store::open_read_only(root, local)?;
    leased.extend(store.referenced());
    crate::guest::lease(&leased)?;
    Ok(store)
}

/// The refusal of a change to the plugins in a guest window.
pub(crate) const GUEST_PACKAGES: &str = "a temporary window cannot install, update, remove, enable or disable plugins; use the main window";

/// Validated candidates kept at once (each is a staged copy on disk).
const MAX_CANDIDATES: usize = 32;
/// At start-up, how long one tick may spend loading installed plugins into
/// the supervisor and starting them (`load_queued`).
const START_BUDGET: std::time::Duration = std::time::Duration::from_millis(25);
pub struct Manager {
    store: Store,
    /// Secret names per plugin service (`secrets.*`), for deleting them.
    pub secrets: crate::packages::secret_index::Index,
    /// Validated packages by token, at most `MAX_CANDIDATES`; `candidate_order`
    /// is oldest first, and a new validation drops the oldest token (`stale`).
    candidates: BTreeMap<String, Candidate>,
    candidate_order: VecDeque<String>,
    /// Package operations by id; a source of `packages.list` (`list_revision`).
    operations: Revised<BTreeMap<String, Operation>>,
    next: u64,
    receipts: BTreeMap<String, Value>,
    booted: bool,
    /// Installed plugins still to load into the supervisor at start-up, in
    /// start order (built-ins first, then the others, each in id order;
    /// `start_order`), a few per tick (`load_queued`); their starts wait in
    /// the supervisor's lanes (supervisor.md §5).
    to_load: VecDeque<String>,
    /// Starts that wait for a built-in's digest check (`Action::Verify`),
    /// by id (`verify_tick`).
    awaiting: BTreeMap<String, UnitId>,
    /// Recordings read for the supervisor's `Replay` (`load_installed`).
    replays: BTreeMap<String, checkpoint::Recording>,
    /// What a pending package operation on an id may start (its target and
    /// the version it restores), by digest: the supervisor's `Spawn` of a
    /// `Restart{image}` (`start_unit_process`), also while `package_tick`
    /// holds the operation itself.
    images: BTreeMap<String, Vec<Image>>,
    /// The installation's built-in plugins (`builtin.json`) by id; empty
    /// when it has none (a development run).
    builtins: BTreeMap<String, builtin::Builtin>,
    /// The `library-cleanup` thread, while it removes package directories.
    cleanup: Option<cleanup::Running>,
    /// Package directories an operation freed while the thread ran; they
    /// go on its next run.
    cleanup_queue: BTreeSet<String>,
    /// The holding folder has package directories to delete
    /// (`Library::hold`): on the cleanup thread's next run.
    purge_held: bool,
    /// Built-in digest checks for the shared runner (`verify`), by id.
    verified: BTreeMap<String, Verified>,
    verify_serial: u64,
    verify_results: (
        std::sync::mpsc::Sender<VerifyResult>,
        std::sync::mpsc::Receiver<VerifyResult>,
    ),
    /// `packages.validate*` on their threads (package_validate.rs).
    validations: crate::packages::package_validate::Validations,
    /// The guests directory whose leases the library cleanup keeps
    /// (`guest::leased`, guest-windows.md §4); `None` in the host's unit
    /// tests, which never read the user's.
    guests: Option<PathBuf>,
}
/// The start-up order of installed plugins `ids` (today's `StartQueue`):
/// built-ins first, then the others, each in id order: the built-ins
/// (playback, video, subtitles) do not wait behind third-party plugins.
fn start_order(
    ids: impl IntoIterator<Item = String>,
    builtin: impl Fn(&str) -> bool,
) -> VecDeque<String> {
    let (builtins, others): (Vec<_>, Vec<_>) = ids.into_iter().partition(|id| builtin(id));
    builtins.into_iter().chain(others).collect()
}
/// A package an operation may start: digest, directory, manifest, data.
type Image = (String, PathBuf, package_stage::PackageManifest, DataKey);
/// The images of `op` (`Manager::images`).
fn images_of(op: &Operation) -> Vec<Image> {
    let mut images = vec![(
        op.target.digest.clone(),
        op.target.directory.clone(),
        op.target.manifest.clone(),
        op.target_key.clone(),
    )];
    if let Some(old) = &op.old {
        images.push((
            old.digest.clone(),
            old.directory.clone(),
            old.manifest.clone(),
            op.old_key.clone().unwrap_or_else(|| op.target_key.clone()),
        ));
    }
    images
}
/// How `packages.list` lists a plugin running from `directory`: installed
/// (with its instance) or a development load (`false`). It is installed
/// when it runs its record's package (`from_record`), or the target of the
/// install that is activating (`pending`): the record names that target
/// only once the install commits, and a store install is not a development
/// load meanwhile.
fn listed_instance(
    directory: &Path,
    from_record: bool,
    record: package_store::Installed,
    pending: Option<&package_store::Recovery>,
) -> (bool, Option<Instance>) {
    if let Some(pending) = pending.filter(|p| p.target.directory == directory) {
        let instance = pending
            .instance
            .clone()
            .or_else(|| pending.before.instance.clone());
        return (true, instance);
    }
    (from_record, record.instance.filter(|_| from_record))
}
/// A finished digest check: id, serial, outcome.
type VerifyResult = (String, u64, std::result::Result<(), String>);
/// A built-in's digest check (`Manager::verify`).
enum Verified {
    /// On a `builtin-verify` thread (its serial, since when).
    Running(u64, std::time::Instant),
    /// `Err`: why it runs in its own process.
    Done(std::result::Result<(), String>),
}
/// Where `load_package` runs a plugin (`Core::trust`).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Trust {
    /// In the shared runner.
    Shared,
    /// In its own process; for a built-in, why (logged).
    Own(Option<String>),
    /// A built-in whose digest is being checked: start it once known.
    Pending,
}
/// How long a built-in's start waits for its digest check (a large
/// package read for the first time, scanned by an antivirus, can take
/// seconds); then it starts in its own process.
const VERIFY_WAIT: std::time::Duration = std::time::Duration::from_secs(3);
struct Operation {
    id: String,
    package: String,
    target: StoredPackage,
    old: Option<StoredPackage>,
    /// Whose data the target and the old version run with.
    target_key: DataKey,
    old_key: Option<DataKey>,
    /// A data directory this operation's new instance created; removed when
    /// the operation fails or is cancelled.
    fresh_data: Option<PathBuf>,
    /// Whether the target runs once committed
    /// (`package_store::enables_after_install`). When it does not, nothing
    /// activates it: the commit installs its files only.
    enabled: bool,
    phase: String,
    owner: Option<u64>,
    /// The supervisor's job the operation waits on: its target's
    /// start (`Restart{fresh, image}`, which stops the running or parked
    /// version first), the old version's (`restoring`), or its target's
    /// stop when nothing is restored.
    job: Option<crate::supervisor::JobId>,
    /// Its target's start was asked (`job` is its `Restart{image}`).
    started: bool,
    cancel: bool,
    failure: Option<String>,
    result: Value,
    request: Value,
}
impl Manager {
    /// Keep a validated candidate; past `MAX_CANDIDATES` the oldest token
    /// goes (an install that already staged it keeps its own copy).
    fn remember(&mut self, token: String, candidate: Candidate) {
        while self.candidate_order.len() >= MAX_CANDIDATES {
            if let Some(oldest) = self.candidate_order.pop_front()
                && let Some(gone) = self.candidates.remove(&oldest)
            {
                // Its staged copy is removed off the UI thread.
                crate::packages::package_validate::drop_later(gone);
            }
        }
        self.candidate_order.push_back(token.clone());
        self.candidates.insert(token, candidate);
    }
    /// Tests: a manager on the store at `root`, no built-ins (the core's
    /// unit tests, plan step S2f(3)).
    #[cfg(test)]
    pub(crate) fn at(root: PathBuf) -> Result<Self> {
        Self::on(Store::open(root)?)
    }
    /// Tests: a guest window's manager, the store at `root` read-only.
    #[cfg(test)]
    pub(crate) fn at_read_only(root: PathBuf, local: PathBuf) -> Result<Self> {
        Self::on(Store::open_read_only(root, local)?)
    }
    #[cfg(test)]
    fn on(store: Store) -> Result<Self> {
        let receipts = store.operations();
        Ok(Self {
            store,
            secrets: crate::packages::secret_index::Index::new(None),
            candidates: BTreeMap::new(),
            candidate_order: VecDeque::new(),
            operations: Revised::default(),
            next: 0,
            receipts,
            booted: false,
            to_load: VecDeque::new(),
            awaiting: BTreeMap::new(),
            replays: BTreeMap::new(),
            images: BTreeMap::new(),
            builtins: BTreeMap::new(),
            cleanup: None,
            cleanup_queue: BTreeSet::new(),
            purge_held: false,
            verified: BTreeMap::new(),
            verify_serial: 0,
            verify_results: std::sync::mpsc::channel(),
            validations: Default::default(),
            guests: None,
        })
    }
    pub fn open() -> Result<Self> {
        let guest = crate::guest::active();
        // Staged copies an earlier run left behind (it quit or crashed
        // while validating). Not by a guest window: the main player does it.
        if !guest {
            crate::packages::package_validate::sweep_stale();
        }
        // The main profile's library, also in a guest window, which reads it
        // (doc/architecture/guest-windows.md §4).
        let root = std::env::var_os("VESTRA_PACKAGE_STORE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                crate::platform::main_app_data_dir()
                    .unwrap_or_default()
                    .join("packages")
            });
        // A guest records no secret names (it writes no secrets).
        let secrets =
            crate::packages::secret_index::Index::new((!guest).then(|| root.join("secrets")));
        let mut store = if guest {
            open_leased(
                root,
                crate::platform::app_data_dir()
                    .unwrap_or_default()
                    .join("packages"),
            )?
        } else {
            Store::open(root)?
        };
        // Reads one small manifest per built-in; copies and hashes nothing.
        let builtins = match builtin::directory().map(|directory| builtin::load(&directory)) {
            Some(Ok(builtins)) => {
                let ids = builtins.keys().cloned().collect();
                if let Err(error) = store.sync_builtins(&ids) {
                    diag!("built-in plugins: {error:#}");
                }
                builtins
            }
            Some(Err(error)) => {
                diag!("built-in plugins: {error:#}");
                BTreeMap::new()
            }
            None => BTreeMap::new(),
        };
        let receipts = store.operations();
        Ok(Self {
            store,
            secrets,
            candidates: BTreeMap::new(),
            candidate_order: VecDeque::new(),
            operations: Revised::default(),
            next: 0,
            receipts,
            booted: false,
            to_load: VecDeque::new(),
            awaiting: BTreeMap::new(),
            replays: BTreeMap::new(),
            images: BTreeMap::new(),
            builtins,
            cleanup: None,
            cleanup_queue: BTreeSet::new(),
            purge_held: false,
            verified: BTreeMap::new(),
            verify_serial: 0,
            verify_results: std::sync::mpsc::channel(),
            validations: Default::default(),
            guests: crate::guest::root(),
        })
    }
    /// Tests: the guests directory whose leases the cleanup keeps.
    #[cfg(test)]
    pub(crate) fn set_guests(&mut self, guests: Option<PathBuf>) {
        self.guests = guests;
    }
    /// A guest window's view of the main library: read-only
    /// (`Store::open_read_only`, its own state under `local`).
    pub(crate) fn read_only(&self) -> bool {
        self.store.read_only()
    }
    /// The package side's sources of `packages.list`: the library's
    /// catalog (`Store::revision`) and this run's operations
    /// (`binding_manager.rs` `packages_key`). The built-ins' list
    /// (`builtins`) does not change after the start.
    pub(crate) fn list_revision(&self) -> (u64, u64) {
        (self.store.revision(), self.operations.revision())
    }
    /// Where an installed record's plugin runs from: a built-in from the
    /// installation, any other from its package in the library. `None`
    /// when there is nothing to run (a built-in this installation does
    /// not list); `Err` when the installation's copy failed its checks.
    fn runs_from(
        &self,
        id: &str,
        record: &package_store::Installed,
    ) -> Option<std::result::Result<(PathBuf, package_stage::PackageManifest), String>> {
        if record.instance.as_ref().is_some_and(|i| i.builtin) {
            let builtin = self.builtins.get(id)?;
            return Some(
                builtin
                    .manifest
                    .clone()
                    .map(|manifest| (builtin.directory.clone(), manifest)),
            );
        }
        let current = record.current.as_ref()?;
        // Its copy in this library could not be read at start: the record
        // still names another library's directory, which is never run
        // (packages run only from this library, verified). The next start
        // tries again.
        if let Some(error) = self.store.deferred().get(id) {
            return Some(Err(deferred_failure(error)));
        }
        Some(Ok((current.directory.clone(), current.manifest.clone())))
    }
    /// Check built-in `id`'s directory against its `builtin.json` digest on
    /// a `builtin-verify` thread (`package_stage::inspect`: the package
    /// limits, the manifest and the digest, as for a library copy); the
    /// result arrives in `poll_verified`. A new check replaces a running one.
    /// Nothing for a built-in without a digest or whose checks failed.
    fn verify(&mut self, id: &str) {
        let Some(builtin) = self.builtins.get(id) else {
            return;
        };
        let (Some(expected), Ok(_)) = (builtin.digest.clone(), &builtin.manifest) else {
            return;
        };
        self.verify_serial += 1;
        let serial = self.verify_serial;
        let directory = builtin.directory.clone();
        let results = self.verify_results.0.clone();
        let name = id.to_owned();
        let spawned = std::thread::Builder::new()
            .name("builtin-verify".into())
            .spawn(move || {
                let result = match package_stage::inspect(&directory) {
                    Ok((digest, _)) if digest == expected => Ok(()),
                    Ok((digest, _)) => Err(format!(
                        "its files have digest {digest}, builtin.json records {expected}"
                    )),
                    Err(error) => Err(format!("{error:#}")),
                };
                let _ = results.send((name, serial, result));
            });
        let state = match spawned {
            Ok(_) => Verified::Running(serial, std::time::Instant::now()),
            Err(error) => Verified::Done(Err(format!("digest check: {error}"))),
        };
        self.verified.insert(id.to_owned(), state);
    }
    /// Take the finished digest checks (a start waiting for one goes on,
    /// `Core::verify_tick`, in start order as today's queue).
    fn poll_verified(&mut self) {
        while let Ok((id, serial, result)) = self.verify_results.1.try_recv() {
            if !matches!(self.verified.get(&id), Some(Verified::Running(s, _)) if *s == serial) {
                continue;
            }
            if let Err(error) = &result {
                diag!("built-in plugin {id}: {error}");
            }
            self.verified.insert(id, Verified::Done(result));
        }
    }
    /// A digest check of `id` is running and its start may still wait for
    /// it ([`VERIFY_WAIT`]); past that it starts in its own process.
    fn verifying(&self, id: &str) -> bool {
        matches!(self.verified.get(id), Some(Verified::Running(_, since)) if since.elapsed() < VERIFY_WAIT)
    }
}
impl Core {
    /// The shared runner at a host tick: the heartbeat (a hang restarts
    /// every member), and an idle runner let go (after the runner idle
    /// exit, at least 30 s; supervisor.md §6).
    fn shared_tick(&mut self) {
        let now = self.now_ms();
        let after = crate::host::supervisor::runner_idle_exit(&self.supervisor);
        let Some(shared) = self.shared.as_mut() else {
            return;
        };
        if shared.idle(now, after) {
            if let Some(runner) = self.shared.take() {
                diag!("shared runner {}: no plugin left; stopped", runner.link.id);
                runner.retire(true, &self.reaper);
            }
            return;
        }
        if let Err(reason) = shared.tick(now, shared_runner::ping_ms()) {
            self.restart_shared(&reason);
        }
    }
    /// A control line from shared runner `runner` (`Event::Runner`).
    pub(crate) fn runner_event(&mut self, runner: shared_runner::RunnerId, message: &Value) {
        let Some(shared) = self.shared.as_mut().filter(|s| s.link.id == runner) else {
            return;
        };
        let Some((g, text)) = shared.control(message) else {
            return;
        };
        if g == 0 {
            // No plugin owns it; the runner exits on its own.
            self.restart_shared(&format!("unattributed fault: {text}"));
            return;
        }
        let Some(id) = self
            .plugins
            .get(&g)
            .filter(|plugin| plugin.shared_runner() == Some(runner))
            .map(|plugin| plugin.manifest.id.clone())
        else {
            return;
        };
        // That plugin alone stops, as if its own process had crashed, and
        // runs in its own process if it starts again this run.
        diag!("shared runner {runner}: {id} faulted; its own process from now on");
        self.shared_policy.quarantined.insert(id);
        self.handle(Event::RuntimeFault(
            g,
            format!("uncaught exception: {text}"),
        ));
    }
    /// The shared runner exited, faulted without attribution or hung:
    /// every member stops (its registrations revoked as for any exit) and
    /// starts again in a new runner, its digest checked again. The third
    /// restart within a minute
    /// fails open: own processes for all of them until the player quits.
    /// Session storage is keyed by the plugin's data key, not by process,
    /// so the restarted plugins find their values (`session_storage.rs`).
    pub(crate) fn restart_shared(&mut self, reason: &str) {
        let Some(runner) = self.shared.take() else {
            return;
        };
        let id = runner.link.id;
        diag!(
            "shared runner {id}: {reason}; restarting its plugins \
             (members whose cleanup timed out: {}; last stats: {})",
            runner.link.leaked(),
            runner.stats
        );
        runner.retire(false, &self.reaper);
        // The Host's trust check counts the restart too: fail-open after
        // the third in a minute (`Policy::restart`), as the core does.
        let now = self.now_ms();
        self.shared_policy.restart(now);
        // The core's runner group restart (supervisor.md §7): every member
        // stops at once (no member gets its `runtime.stop`: the runner is
        // killed), goes back to standby by replay when it has a standby
        // recording, else starts again in the built-in lane: the first
        // restart at once, a later one in the window after the restart delay
        // (3 s × 2ⁿ), once the playback worker cleaned up after it.
        self.supervisor.set_now(now);
        let runner = self.supervisor.runner();
        self.supervisor
            .runner_failed(runner, crate::supervisor::ExitCause::Transport);
        self.supervisor_run();
    }
    /// Starts `manifest` from `directory` as process `owner` (the pid the
    /// supervisor allocated with its `Spawn`), its activation due by
    /// `deadline` (the start's the start timeout, `Job::deadline`).
    fn load_package_as(
        &mut self,
        owner: u64,
        directory: &Path,
        manifest: &package_stage::PackageManifest,
        key: DataKey,
        deadline: u64,
    ) -> Result<bool> {
        let id = manifest.id.clone();
        anyhow::ensure!(
            self.running(&id).is_none_or(|running| running == owner),
            "plugin already running"
        );
        self.activation_begin(&id, owner, deadline)?;
        let manifest = process::Manifest {
            id: id.clone(),
            ..manifest.clone()
        };
        // A spawn that fails gives its activation back (before, the place
        // stayed taken for the rest of the run).
        let plugin = match self.spawn_package(owner, &id, directory, manifest, key) {
            Ok(Some(plugin)) => plugin,
            // On the spawn thread (`host/supervisor/worker.rs`).
            Ok(None) => return Ok(false),
            Err(error) => {
                self.supervisor.activation_end(Pid(owner));
                return Err(error);
            }
        };
        let name = plugin.manifest.name.clone();
        self.add_plugin(owner, plugin);
        self.backend.commands.send(Command {
            reply: Reply { owner, id: 0 },
            method: "owner.registered".into(),
            params: json!({"manifestId":id,"manifestName":name}),
        })?;
        Ok(true)
    }
    /// `load_package`'s process: in the shared runner (started if none
    /// runs) when trusted, else its own, its heap capped by its class
    /// (the heap cap, 512 MB; supervisor.md §8). `None`: its own process is created
    /// on the spawn thread (`Event::Spawned`).
    fn spawn_package(
        &mut self,
        owner: u64,
        id: &str,
        directory: &Path,
        manifest: process::Manifest,
        key: DataKey,
    ) -> Result<Option<Plugin>> {
        Ok(Some(match self.trust(id, directory) {
            Trust::Shared => {
                if self.shared.is_none() {
                    let runner = self.shared_policy.next_id();
                    let now = self.now_ms();
                    self.shared = Some(shared_runner::SharedRunner::spawn(
                        runner,
                        self.events.clone(),
                        now,
                    )?);
                }
                let shared = self
                    .shared
                    .as_mut()
                    .ok_or_else(|| anyhow::anyhow!("shared runner"))?;
                diag!("shared runner {}: {id} admitted", shared.link.id);
                Plugin::load_manifest(
                    directory,
                    owner,
                    self.events.clone(),
                    manifest,
                    key,
                    process::Placement::Shared(shared),
                )?
            }
            trust => {
                match trust {
                    Trust::Own(Some(reason)) => diag!("shared runner: {id} runs alone: {reason}"),
                    Trust::Pending => {
                        diag!("shared runner: {id} runs alone: its digest is still being checked")
                    }
                    _ => {}
                }
                let heap_mib = self
                    .supervisor
                    .by_id(id)
                    .and_then(|unit| self.supervisor.unit(unit))
                    .and_then(|unit| self.supervisor.tunables().policy(unit.class).memory_max_mib);
                if self.spawn_off_thread() {
                    let job = crate::host::supervisor::worker::SpawnJob {
                        owner,
                        directory: directory.to_owned(),
                        manifest,
                        key,
                        heap_mib,
                    };
                    match self.spawn_queued(job) {
                        Ok(()) => return Ok(None),
                        Err(job) => {
                            let job = *job;
                            Plugin::load_manifest(
                                &job.directory,
                                owner,
                                self.events.clone(),
                                job.manifest,
                                job.key,
                                process::Placement::Own { heap_mib },
                            )?
                        }
                    }
                } else {
                    Plugin::load_manifest(
                        directory,
                        owner,
                        self.events.clone(),
                        manifest,
                        key,
                        process::Placement::Own { heap_mib },
                    )?
                }
            }
        }))
    }
    /// `packages.validate` and `packages.validateArchive`: the request is
    /// read here, the package checked on a thread (package_validate.rs), and
    /// the answer sent from `package_tick` (`answer_validations`).
    pub(crate) fn validate_package(
        &mut self,
        reply: &Reply,
        method: &str,
        params: &Value,
    ) -> std::result::Result<(), RpcError> {
        let request = crate::packages::package_validate::Request::read(method, params)?;
        for (reply, error) in self.packages.validations.start(reply.clone(), request) {
            self.respond(reply, Err(error));
        }
        Ok(())
    }
    /// Answer finished validations: a passing package gets its token and
    /// joins the recent candidates (`Manager::remember`).
    fn answer_validations(&mut self) {
        let running = &self.plugins;
        let done = self
            .packages
            .validations
            .poll(|owner| running.contains_key(&owner));
        for (reply, result) in done {
            let result = result.map(|candidate| {
                self.packages.next += 1;
                let token = format!("candidate-{}-{}", self.packages.next, candidate.digest());
                let report = json!({"token":token,"digest":candidate.digest(),"apiVersion":process::API,"manifest":candidate.manifest(),"checks":["manifest","digest","productionJavaScript","slintCompile"],"trustedExecution":true});
                self.packages.remember(token, candidate);
                report
            });
            self.respond(reply, result);
        }
    }
    pub(crate) fn package_dispatch(
        &mut self,
        caller: u64,
        method: &str,
        params: &Value,
    ) -> std::result::Result<Value, RpcError> {
        // A guest window runs the main profile's plugins as they are
        // (doc/architecture/guest-windows.md §4).
        if self.packages.read_only()
            && matches!(
                method,
                "packages.install" | "packages.uninstall" | "packages.setEnabled"
            )
        {
            return Err(RpcError::new("unavailable", GUEST_PACKAGES));
        }
        match method {
            "packages.installed" => Ok(serde_json::to_value(
                self.packages.store.get(string(params, "id")?),
            )
            .map_err(anyhow::Error::from)?),
            "packages.install" => {
                let request = string(params, "requestId")?.to_owned();
                if request.is_empty() || request.len() > 128 {
                    return Err(RpcError::invalid("requestId must contain 1..128 bytes"));
                }
                let request_spec = json!({"method":method,"params":params});
                if let Some(receipt) = self.packages.receipts.get(&request) {
                    if receipt["request"] != request_spec {
                        return Err(RpcError::new(
                            "conflict",
                            "requestId was used with different arguments",
                        ));
                    }
                    return Ok(receipt.clone());
                }
                if let Some(operation) = self.packages.operations.get(&request) {
                    if operation.request != request_spec {
                        return Err(RpcError::new(
                            "conflict",
                            "requestId was used with different arguments",
                        ));
                    }
                    return Ok(operation.result.clone());
                }
                self.library_busy(None)?;
                if self.packages.operations.len() >= 64 {
                    return Err(RpcError::new("busy", "operation history full"));
                }
                let source = install_source(params)?;
                let replace = params.get("replace").is_some_and(|v| v == true);
                let requested = install_enabled(params)?;
                let token = string(params, "token")?;
                let candidate = self
                    .packages
                    .candidates
                    .get(token)
                    .cloned()
                    .ok_or_else(|| RpcError::new("stale", "unknown validation token"))?;
                check_reserved(&candidate.manifest().id, None)?;
                let id = candidate.manifest().id.clone();
                let expected = params["expectedRevision"]
                    .as_u64()
                    .ok_or_else(|| RpcError::invalid("expectedRevision required"))?;
                let before = self.packages.store.get(&id);
                if before.revision != expected {
                    return Err(RpcError::new(
                        "revisionConflict",
                        "installed revision changed",
                    ));
                }
                let new_source = source_changes(&before, &source);
                let (instance, fresh) = self.install_instance(&id, &before, source, replace)?;
                // A replace from another source is another plugin to the
                // user: "Don't ask again" for the old one does not carry over
                // (an update from the same source keeps it).
                if new_source {
                    crate::capabilities::forget_command_trust(self, &id);
                }
                // Garbage is collected off the UI thread, not here. Staging
                // reuses an identical directory still in the library, so it
                // waits while the cleanup thread removes that one.
                self.library_busy(Some(candidate.digest()))?;
                let target = self.packages.store.stage(&candidate)?;
                // An update of a package the user disabled stays disabled
                // and is not activated, unless the installer asks.
                let enabled = package_store::enables_after_install(
                    &before,
                    self.packages.store.disabled().contains(&id),
                    requested,
                );
                self.start_operation(
                    request,
                    request_spec,
                    target,
                    before,
                    (instance, fresh),
                    enabled,
                )
            }
            "packages.operation" | "packages.cancel" => {
                let id = string(params, "operationId")?;
                if let Some(receipt) = self.packages.receipts.get(id) {
                    return Ok(
                        if method == "packages.cancel" && receipt["status"] == "committed" {
                            json!({"operationId":id,"status":"tooLate","result":receipt})
                        } else {
                            receipt.clone()
                        },
                    );
                }
                let operation = self
                    .packages
                    .operations
                    .edit()
                    .get_mut(id)
                    .ok_or_else(|| RpcError::new("missing", "operation unknown"))?;
                if method == "packages.cancel" {
                    if operation.phase == "committed" {
                        return Ok(
                            json!({"operationId":id,"status":"tooLate","result":operation.result}),
                        );
                    }
                    if matches!(operation.phase.as_str(), "preparing" | "activating") {
                        operation.cancel = true;
                    }
                }
                Ok(operation.result.clone())
            }
            "packages.list" => Ok(self.package_list()),
            "packages.setEnabled" => {
                let enabled = params["enabled"]
                    .as_bool()
                    .ok_or_else(|| RpcError::invalid("enabled requires a boolean"))?;
                self.set_enabled(caller, string(params, "id")?, enabled)?;
                Ok(Value::Null)
            }
            "packages.reload" => {
                self.reload(caller, string(params, "id")?)?;
                Ok(Value::Null)
            }
            "packages.uninstall" => self.uninstall(caller, params),
            _ => Err(RpcError::new("unsupported", "unknown package method")),
        }
    }
    /// Journal the install and stop the running version; the tick activates
    /// the target and commits or restores. A target that is not `enabled`
    /// is committed without activating it (files only).
    fn start_operation(
        &mut self,
        request: String,
        request_spec: Value,
        target: StoredPackage,
        before: package_store::Installed,
        (instance, fresh): (Option<Instance>, bool),
        enabled: bool,
    ) -> std::result::Result<Value, RpcError> {
        let id = target.manifest.id.clone();
        let result = json!({"operationId":request,"packageId":id,"status":"preparing","request":request_spec,
            "fingerprint":{"action":request_spec["method"],"packageId":id,"digest":target.digest,"expectedRevision":before.revision}});
        let target_guid = instance
            .as_ref()
            .or(before.instance.as_ref())
            .map(|i| i.guid.clone())
            .unwrap_or_else(package_store::new_guid);
        let old_key = before
            .instance
            .as_ref()
            .map(|i| DataKey::Instance(i.guid.clone()));
        // The replaced version's recording (keyed by its instance).
        if let Some(instance) = &before.instance {
            self.recordings_written();
            checkpoint::remove(&self.activation_dir(), &instance.guid);
        }
        self.packages.store.prepare_with_receipt(
            &id,
            before.revision,
            &target,
            &request,
            result.clone(),
            instance,
        )?;
        // Its recording goes (an upgrade has a new digest: of no use).
        self.drop_recording(&id);
        let running = self.running(&id).is_some() || self.parked_id(&id).is_some();
        let image = target.digest.clone();
        let fresh_data = fresh
            .then(|| process::data_root().map(|root| root.join(&target_guid)))
            .flatten();
        let operation = request.clone();
        // In the table before its start: the supervisor's spawn of the
        // target reads it (`start_unit_process`).
        self.packages.operations.edit().insert(
            request.clone(),
            Operation {
                id: request,
                package: id.clone(),
                target,
                old: before.current,
                target_key: DataKey::Instance(target_guid),
                old_key,
                fresh_data,
                enabled,
                phase: "preparing".into(),
                owner: None,
                job: None,
                started: false,
                cancel: false,
                failure: None,
                result: result.clone(),
                request: request_spec,
            },
        );
        if let Some(op) = self.packages.operations.get(&operation) {
            let images = images_of(op);
            self.packages.images.insert(id.clone(), images);
        }
        let unit = self.operation_unit(&id);
        // The target is not committed while it activates: no recording of
        // it, no classification (today's `lazy_key_of` is `None` for it).
        self.supervisor.set_recordable(unit, false);
        // `upgrade` for every install over a running (or parked) version
        // that runs on: an update, a reinstall of the same version, and a
        // `replace` from another source (`fresh` data) alike; the target
        // starts once that version's process is gone (any process of it
        // still going: never two processes on its data). An install that leaves it off
        // (`enabled: false`; a running package is never disabled, so only the
        // installer's option does this) switches the plugin off: a
        // `Disable` stop, as `packages.setEnabled(false)` (its session values
        // go too, at the commit). The operation is the busy package RPC, so
        // its job merges with what is installed (`JobMode::Replace`), as
        // today's install goes on beside a waiting reload.
        // A target with nothing to stop first starts at the next tick
        // (`advance_package`), as today's `load_package` there; a parked
        // version only lets go of its entries now.
        let process = self.running(&id).is_some();
        if enabled {
            self.supervisor.set_enabled(unit, true);
        }
        let kind = if enabled && process {
            Some(JobKind::Restart {
                fresh: true,
                image: Some(image),
            })
        } else if running {
            Some(JobKind::Stop {
                reason: if enabled {
                    crate::supervisor::StopReason::Upgrade
                } else {
                    crate::supervisor::StopReason::Disable
                },
                then: Then::Inactive,
            })
        } else {
            None
        };
        let started = enabled && process;
        if let Some(kind) = kind {
            let job = self.supervisor.enqueue_job(
                unit,
                kind,
                JobMode::Replace,
                Priority::Interactive,
                Origin::PackageRpc,
            );
            if let Some(op) = self.packages.operations.edit().get_mut(&operation) {
                op.job = job.ok();
                op.started = started;
            }
            self.supervisor_run();
        }
        Ok(result)
    }
    /// The data directory of an installed instance, development load or
    /// running plugin (`PluginInfo.dataDirectory`).
    fn data_directory_of(&self, key: &DataKey) -> Option<PathBuf> {
        process::data_root().map(|root| key.directory(&root))
    }
    /// Loads installed package `id` into the supervisor (`add_unit`: at
    /// start-up, and a shared runner member queued again), unless it runs
    /// already, is in an operation, or a development load of the same id
    /// runs instead (R-H10). A disabled one is loaded disabled (it stays
    /// stopped). Built-ins run in place from the installation
    /// (`runs_from`); nothing is copied. The supervisor replays a valid
    /// recording of a plugin that was lazy at its ready (on-demand start:
    /// it starts on its first use), starts one that was resident, and
    /// queues a first run without one (supervisor.md §4).
    pub(crate) fn load_installed(&mut self, id: &str) {
        if self.running(id).is_some() || self.parked_id(id).is_some() {
            return;
        }
        let Some(record) = self.packages.store.record(id).cloned() else {
            return;
        };
        if self.packages.operations.values().any(|o| o.package == id)
            || self.development.contains_key(id)
        {
            return;
        }
        // Its library copy could not be read at start: kept as it is, not
        // started (`runs_from`); the next start tries again.
        if let Some(error) = self.packages.store.deferred().get(id) {
            self.failures
                .edit()
                .insert(id.to_owned(), deferred_failure(error));
            return;
        }
        let (Some(package), Some(instance)) =
            (self.packages.runs_from(id, &record), record.instance)
        else {
            return;
        };
        let disabled = self.packages.store.disabled().contains(id);
        let startable = package.is_ok() && check_reserved(id, Some(&instance)).is_ok();
        let mut spec = self.unit_spec(id);
        if disabled {
            spec = spec.disabled();
        }
        if !startable {
            // Its start fails (recorded as today's); never a first run.
            spec = spec.unrecordable();
        }
        if !disabled
            && startable
            && let Some(key) = self.checkpoint_key(id)
        {
            self.recordings_written();
            match checkpoint::read(&self.packages.store.activation_dir(), &key) {
                Some(recording) if recording.resident_at_ready => {
                    diag!("lazy: {id} starts: resident at its last ready");
                    spec = spec.checkpoint(true);
                }
                Some(recording) => {
                    spec = spec.checkpoint(false);
                    self.packages.replays.insert(id.to_owned(), recording);
                }
                None => {}
            }
        }
        self.supervisor.add_unit(spec);
        // Its kept trigger conditions (crate::triggers): evaluated when the
        // start-up mute ends.
        if !disabled {
            self.triggers_load(id);
        }
        self.supervisor_run();
        self.packages.replays.remove(id);
    }
    /// The supervisor's spawn of plugin `id` as process `owner`
    /// (`Action::Spawn`): a development load, a package operation's target
    /// or the version it restores (`Restart{image}`), or the installed
    /// package (today's `start_installed`, `start_stopped`, a wake).
    /// `false`: its own process is created on the spawn thread
    /// (`Event::Spawned`).
    pub(crate) fn start_unit_process(
        &mut self,
        id: &str,
        owner: u64,
        image: Option<&str>,
        development: bool,
        deadline: u64,
    ) -> Result<bool> {
        if development {
            let directory = self
                .dev_loading
                .clone()
                .or_else(|| {
                    self.development
                        .get(id)
                        .map(|(directory, _)| directory.clone())
                })
                .ok_or_else(|| anyhow::anyhow!("no development load of {id}"))?;
            return self
                .load_development_as(&directory, owner, deadline)
                .map(|()| true);
        }
        if let Some(image) = image
            && let Some(images) = self.packages.images.get(id)
        {
            let (_, directory, manifest, key) = images
                .iter()
                .find(|(digest, ..)| digest == image)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("the package {image} is not this operation's"))?;
            return self.load_package_as(owner, &directory, &manifest, key, deadline);
        }
        self.start_package_as(id, owner, deadline)
    }
    /// Starts installed plugin `id` as process `owner`, as `start_installed`
    /// would.
    fn start_package_as(&mut self, id: &str, owner: u64, deadline: u64) -> Result<bool> {
        let record = self.packages.store.get(id);
        let instance = record
            .instance
            .clone()
            .ok_or_else(|| anyhow::anyhow!("the installed package has no instance"))?;
        let (directory, manifest) = self
            .packages
            .runs_from(id, &record)
            .ok_or_else(|| anyhow::anyhow!("not installed"))?
            .map_err(anyhow::Error::msg)?;
        check_reserved(id, Some(&instance)).map_err(|refusal| anyhow::anyhow!(refusal.message))?;
        self.load_package_as(
            owner,
            &directory,
            &manifest,
            DataKey::Instance(instance.guid),
            deadline,
        )
    }
    /// Removes installed plugin `id`'s recording, if it has one.
    pub(crate) fn drop_recording(&mut self, id: &str) {
        if let Some(instance) = self.packages.store.get(id).instance {
            self.recordings_written();
            checkpoint::remove(&self.activation_dir(), &instance.guid);
        }
    }
    /// `packages.list`: running plugins, installed packages and this run's
    /// development loads, by id.
    pub(crate) fn package_list(&mut self) -> Value {
        let now = self.now_ms();
        self.package_list_at(now)
    }
    /// [`Core::package_list`] as at `now` (the supervisor's clock): the
    /// same `now` as [`Core::packages_key`]'s when they are compared.
    pub(crate) fn package_list_at(&mut self, now: u64) -> Value {
        let mut list = BTreeMap::<String, Value>::new();
        self.supervisor.set_now(now);
        let owners = self.plugins.keys().copied().collect::<Vec<_>>();
        for owner in owners {
            let plugin = &self.plugins[&owner];
            let id = plugin.manifest.id.clone();
            let stored = self.packages.store.get(&id);
            let from_record = self
                .packages
                .runs_from(&id, &stored)
                .is_some_and(|package| package.is_ok_and(|(dir, _)| dir == plugin.directory));
            let (installed, instance) = listed_instance(
                &plugin.directory,
                from_record,
                stored,
                self.packages.store.pending_of(&id),
            );
            let warnings = self.warnings_of(owner);
            // The supervisor's state of its unit, its activation read (supervisor.md §10).
            let listed = self
                .supervisor
                .by_id(&id)
                .and_then(|unit| self.supervisor.list_state(unit));
            let (state, error) = match listed {
                Some(ListState::Starting) => ("starting", None),
                Some(ListState::Failed(message)) => ("failed", Some(message)),
                // Woken from idle (lazy activation): it was running all along.
                Some(ListState::Running { .. }) if self.supervisor.waking(Pid(owner)).is_some() => {
                    ("running", None)
                }
                // Until the SDK passes `warnings` on (it keeps only the known
                // fields), a running plugin's `error` carries them as text.
                _ => ("running", self.warning_text(owner)),
            };
            let origin = if installed {
                "installed"
            } else {
                "development"
            };
            // `process` (SDK 1.5.38): it has a process (`idle` below).
            let process = (state == "running").then_some("running");
            list.insert(
                id.clone(),
                json!({"id":id,"version":plugin.manifest.version,
                "origin":origin,"state":state,"error":error,"process":process,
                "directory":plugin.directory,
                "builtin":instance.as_ref().is_some_and(|i| i.builtin),
                "source":instance.map(|i| i.source),
                "dataDirectory":plugin.data_directory(),"warnings":warnings,
                "display":display(&plugin.manifest, plugin.catalog.as_deref())}),
            );
        }
        let stopped = |disabled: bool, failure: Option<&String>| {
            if disabled {
                ("disabled", None)
            } else if let Some(failure) = failure {
                ("failed", Some(failure.clone()))
            } else {
                ("stopped", None)
            }
        };
        // The catalog's records as they are (not a copy of it: `notify_tick`
        // builds this list while plugins come and go, supervisor.md).
        for (id, record) in self.packages.store.installed_records() {
            if list.contains_key(id) {
                continue;
            }
            let id = id.clone();
            let Some(package) = self.packages.runs_from(&id, record) else {
                continue;
            };
            // The supervisor's state of its unit (supervisor.md §10): a start it waits for
            // (a first run waiting for a place, a reload or an enable
            // waiting for the previous process) and a package operation
            // count as starting; one waiting at start-up lists `stopped`.
            let disabled = self.packages.store.disabled().contains(&id);
            let parked = self
                .parked_id(&id)
                .and_then(|owner| self.on_demand.held(owner));
            let listed = self
                .supervisor
                .by_id(&id)
                .and_then(|unit| self.supervisor.list_state(unit));
            let (state, error) =
                if self.operation_pending(&id) || listed == Some(ListState::Starting) {
                    ("starting", None)
                } else if let Some(parked) = parked {
                    // Parked (lazy activation): it runs again on its first use;
                    // `state` stays `running`, `process` says `idle`, and what
                    // it was refused while it ran stays listed.
                    ("running", parked.warning.clone())
                } else if matches!(listed, Some(ListState::Running { .. })) {
                    // Waiting to start again by itself after a crash
                    // (`Activating{AutoRestart}`): running, no process.
                    ("running", None)
                } else {
                    stopped(disabled, self.failures.get(&id))
                };
            let warnings = parked.map_or_else(Vec::new, |parked| parked.warnings.clone());
            let data = record
                .instance
                .as_ref()
                .and_then(|i| self.data_directory_of(&DataKey::Instance(i.guid.clone())));
            let builtin = record.instance.as_ref().is_some_and(|i| i.builtin);
            let source = record.instance.as_ref().map(|i| i.source.clone());
            let process = ((self.parked_id(&id).is_some()
                || matches!(listed, Some(ListState::Running { .. })))
                && state == "running")
                .then_some("idle");
            let entry = match package {
                Ok((directory, manifest)) => json!({"id":id,"version":manifest.version,
                    "origin":"installed","state":state,"error":error,"process":process,
                    "directory":directory,
                    "builtin":builtin,"source":source,
                    "dataDirectory":data,"warnings":warnings,
                    "display":display(&manifest, crate::i18n::load(&directory).as_deref())}),
                // A built-in whose copy in the installation failed its
                // checks, or a package whose library copy could not be read
                // at start (`Store::deferred`: the directory it names is
                // another library's, not shown).
                Err(failure) => {
                    let (version, directory) = match self.packages.builtins.get(&id) {
                        Some(entry) => (Some(entry.version.clone()), Some(entry.directory.clone())),
                        None => (
                            record
                                .current
                                .as_ref()
                                .and_then(|c| c.manifest.version.clone()),
                            None,
                        ),
                    };
                    let state = if state == "disabled" { state } else { "failed" };
                    json!({"id":id,"version":version,
                        "origin":"installed","state":state,"error":failure,
                        "directory":directory.unwrap_or_default(),
                        "builtin":builtin,"source":source,
                        "dataDirectory":data,"warnings":[],
                        "display":{"name":null,"punchline":null,"description":null,"tags":[]}})
                }
            };
            list.insert(id.clone(), entry);
        }
        // Installed records the library set aside at start, this one or an
        // earlier one (`Store::set_aside_installed`): failed, with the
        // reason, named, versioned and sourced from the record as read,
        // until installed again or uninstalled (`uninstall_set_aside`).
        let dormant = self.packages.store.dormant();
        let aside = self.packages.store.set_aside_installed();
        for (id, (entry, reason)) in aside {
            if list.contains_key(&id) {
                continue;
            }
            let error =
                format!("the plugin library's record of this plugin was set aside: {reason}");
            let (version, source, display) = set_aside_fields(entry);
            // Its instance was retired with its data (`Store::screen`).
            let data = entry["instance"]["guid"]
                .as_str()
                .filter(|guid| dormant.get(*guid).is_some_and(|d| d.id == id))
                .and_then(|guid| self.data_directory_of(&DataKey::Instance(guid.to_owned())));
            list.insert(
                id.clone(),
                json!({"id":id,"version":version,
                    "origin":"installed","state":"failed","error":error,
                    "directory":"",
                    "builtin":false,"source":source,
                    "dataDirectory":data,"warnings":[],
                    "display":display}),
            );
        }
        for (id, (directory, manifest)) in self.development.iter() {
            if list.contains_key(id) {
                continue;
            }
            let (state, error) = if self.package_job_busy(id) {
                ("starting", None)
            } else {
                stopped(self.dev_disabled.contains(id), self.failures.get(id))
            };
            let data = self.data_directory_of(&DataKey::Development(id.clone()));
            list.insert(
                id.clone(),
                json!({"id":id,"version":manifest.version,
                "origin":"development","state":state,"error":error,
                "directory":directory,
                "builtin":false,"source":null,"dataDirectory":data,"warnings":[],
                "display":display(manifest, crate::i18n::load(directory).as_deref())}),
            );
        }
        Value::Array(list.into_values().collect())
    }
    /// `packages.setEnabled`: disabling stops it (and keeps it stopped across
    /// launches when installed); enabling starts it again. Both are the
    /// supervisor's jobs (`disable`, `enable`: `Stop{Disable}`,
    /// `Start{Enable}`, which waits for the plugin's previous process).
    fn set_enabled(
        &mut self,
        caller: u64,
        id: &str,
        enabled: bool,
    ) -> std::result::Result<(), RpcError> {
        if !enabled && self.plugins[&caller].manifest.id == id {
            return Err(RpcError::invalid("a plugin cannot disable itself"));
        }
        let running = self.running(id);
        if self.package_rpc_busy(id) {
            return Err(busy());
        }
        let record = self.packages.store.get(id);
        // User ruling 2026-10-26: users manage third-party plugins only;
        // a built-in always runs (a development load of its id aside).
        if !enabled
            && record.instance.as_ref().is_some_and(|i| i.builtin)
            && !self.development.contains_key(id)
        {
            return Err(RpcError::new(
                "unsupported",
                format!("{id} is a built-in plugin: built-in plugins cannot be disabled"),
            ));
        }
        let installed = self.packages.runs_from(id, &record);
        let reserved = check_reserved(id, record.instance.as_ref());
        let key = record.instance.map(|i| DataKey::Instance(i.guid));
        let development = self.development.get(id).cloned();
        if installed.is_none() && development.is_none() {
            return Err(RpcError::new("missing", format!("no plugin {id}")));
        }
        // A development load of the same id wins over the installed copy.
        if installed.is_some() && development.is_none() {
            self.packages.store.set_disabled(id, !enabled)?;
        } else if enabled {
            self.dev_disabled.edit().remove(id);
        } else {
            self.dev_disabled.edit().insert(id.into());
        }
        let unit = self.ensure_unit(id);
        let now = self.now_ms();
        self.supervisor.set_now(now);
        if !enabled {
            // Its process stops; in standby its recorded entries go and its
            // recording stays (not replayed while it is disabled).
            if self.supervisor.disable(unit).is_err() {
                return Err(busy());
            }
            self.supervisor_run();
            self.failures.edit().remove(id);
            // The user switched it off: its session values go (a reload, an
            // upgrade or a crash keeps them; session_storage.rs).
            let stopped = match &development {
                Some(_) => Some(DataKey::Development(id.into())),
                None => key,
            };
            if let Some(stopped) = stopped {
                self.session_storage.clear(&stopped);
            }
            // And its trigger conditions (events-and-triggers.md §4.10).
            self.triggers_clear(id);
            return Ok(());
        }
        if running.is_some() || self.parked_id(id).is_some() {
            self.supervisor.set_enabled(unit, true);
            return Ok(());
        }
        // Its previous process (parked, disabled or failed just now) still
        // going: it starts once that is gone, never beside it on the same
        // data, and is `busy` meanwhile; checked when it starts.
        let waits = self
            .supervisor
            .unit(unit)
            .is_some_and(|u| u.stopping.is_some() || u.job.is_some());
        if !waits {
            self.startable(id, installed, reserved, key, development.is_some())?;
        }
        if let Err(Busy) = self.supervisor.enable(unit) {
            return Err(busy());
        }
        self.start_now(id)
    }
    /// `packages.reload { id }` (SDK 1.5.19, STORE-RELOAD-BUILTIN): stop the
    /// running plugin and start it again from the same package and data, once
    /// its process is gone (the supervisor's `Restart{fresh}`); a stopped or
    /// failed one starts now. Built-ins included; the disabled flag is never
    /// written, so a reload never leaves a plugin disabled. Same caller rules
    /// as `setEnabled`: `plugins.manage`, never itself.
    fn reload(&mut self, caller: u64, id: &str) -> std::result::Result<(), RpcError> {
        if self.plugins[&caller].manifest.id == id {
            return Err(RpcError::invalid("a plugin cannot reload itself"));
        }
        if self.package_rpc_busy(id) {
            return Err(busy());
        }
        let record = self.packages.store.get(id);
        let installed = self.packages.runs_from(id, &record);
        let development = self.development.get(id).cloned();
        if installed.is_none() && development.is_none() {
            return Err(RpcError::new("missing", format!("no plugin {id}")));
        }
        let disabled = if development.is_some() {
            self.dev_disabled.contains(id)
        } else {
            self.packages.store.disabled().contains(id)
        };
        if disabled {
            return Err(RpcError::invalid(format!(
                "{id} is disabled: enable it instead of reloading it"
            )));
        }
        // Observed fresh: its recording goes, in standby or not.
        self.drop_recording(id);
        let unit = self.ensure_unit(id);
        let now = self.now_ms();
        self.supervisor.set_now(now);
        let going = self.running(id).is_some()
            || self
                .supervisor
                .unit(unit)
                .is_some_and(|u| u.stopping.is_some_and(|s| !s.done()));
        if going {
            // A built-in's files are checked again before it shares the
            // runner (its start waits for the check too: `Activating{Verify}`).
            if shared_runner::enabled() {
                self.packages.verify(id);
            }
        } else {
            let reserved = check_reserved(id, record.instance.as_ref());
            let key = record.instance.map(|i| DataKey::Instance(i.guid));
            self.startable(id, installed, reserved, key, development.is_some())?;
        }
        if let Err(Busy) = self.supervisor.restart(unit, true, None) {
            return Err(busy());
        }
        self.start_now(id)
    }
    /// `packages.uninstall { id, removeData, expectedRevision, requestId }`:
    /// stop it, remove it from the library (one catalog replacement), and
    /// delete its data directory once its process is gone when asked.
    fn uninstall(&mut self, caller: u64, params: &Value) -> std::result::Result<Value, RpcError> {
        let request = string(params, "requestId")?.to_owned();
        if request.is_empty() || request.len() > 128 {
            return Err(RpcError::invalid("requestId must contain 1..128 bytes"));
        }
        let spec = json!({"method":"packages.uninstall","params":params});
        if let Some(receipt) = self.packages.receipts.get(&request) {
            if receipt["request"] != spec {
                return Err(RpcError::new(
                    "conflict",
                    "requestId was used with different arguments",
                ));
            }
            return Ok(receipt.clone());
        }
        let id = string(params, "id")?.to_owned();
        if self.plugins[&caller].manifest.id == id {
            return Err(RpcError::invalid("a plugin cannot uninstall itself"));
        }
        let installed = self.packages.store.get(&id);
        if installed.current.is_none()
            && !self.development.contains_key(&id)
            && self.running(&id).is_none()
            && self.packages.store.set_aside_installed().contains_key(&id)
        {
            return self.uninstall_set_aside(&id, &request, spec, params, installed.revision);
        }
        uninstallable(
            &id,
            &installed,
            self.development.contains_key(&id) || self.running(&id).is_some(),
            params["expectedRevision"].as_u64(),
        )?;
        self.library_busy(None)?;
        if self.package_rpc_busy(&id) {
            return Err(busy());
        }
        let remove_data = params["removeData"] == true;
        let data = remove_data
            .then(|| {
                installed
                    .instance
                    .as_ref()
                    .and_then(|i| self.data_directory_of(&DataKey::Instance(i.guid.clone())))
            })
            .flatten();
        let receipt =
            json!({"operationId":request,"packageId":id,"status":"committed","request":spec});
        // Its recording goes at the request, not when the stop lands; so do
        // its trigger conditions (events-and-triggers.md §4.10).
        self.drop_recording(&id);
        self.triggers_clear(&id);
        // The supervisor stops it (`Stop{Uninstall, then: Removed}`); its
        // data directory, resolved now, is deleted once no process of it
        // may write it (`RemoveData`): one running, parked just now, or
        // stopped otherwise and still going; at once without one.
        match self.supervisor.by_id(&id) {
            Some(unit) => {
                if let Some(directory) = data {
                    self.data_removals.entry(unit).or_default().push(directory);
                }
                let now = self.now_ms();
                self.supervisor.set_now(now);
                let _ = self.supervisor.enqueue_job(
                    unit,
                    JobKind::Stop {
                        reason: crate::supervisor::StopReason::Uninstall,
                        then: Then::Removed,
                    },
                    JobMode::Replace,
                    Priority::Background,
                    Origin::PackageRpc,
                );
                self.supervisor_run();
            }
            None => {
                if let Some(directory) = data {
                    let _ = std::fs::remove_dir_all(directory);
                }
            }
        }
        self.packages
            .store
            .uninstall(&id, &request, receipt.clone(), !remove_data)?;
        if let Some(current) = &installed.current {
            self.discard(&current.digest);
        }
        self.packages.receipts = self.packages.store.operations();
        self.failures.edit().remove(&id);
        // "Don't ask again" for its commands goes too (Ask Vestra UX §4.7):
        // a later install of the same id asks again.
        crate::capabilities::forget_command_trust(self, &id);
        // Session values go with every uninstall, whether or not the files do.
        if let Some(instance) = &installed.instance {
            self.session_storage
                .clear(&DataKey::Instance(instance.guid.clone()));
            // Its source references go too, whether or not its files do
            // (`media.sources`): another install starts without them.
            let _ = self
                .backend
                .commands
                .send(crate::playback::backend::Command {
                    reply: Reply { owner: 0, id: 0 },
                    method: "owner.sourcesRevoked".into(),
                    params: json!({"store": crate::playback::media_sources::store_key(
                        &DataKey::Instance(instance.guid.clone())
                    )}),
                });
        }
        if remove_data && let Some(instance) = &installed.instance {
            self.purge_secrets(&DataKey::Instance(instance.guid.clone()));
        }
        self.purge_dropped_secrets();
        Ok(receipt)
    }
    /// `packages.uninstall` of an installed record the library set aside
    /// at start, this one or an earlier one (a package an older player
    /// installed that this one refuses, such as a manifest with
    /// `capabilities`). Forced (user ruling 2026-11-15): nothing of it runs,
    /// so every record of it goes from `quarantine.json` and the id is free
    /// at once; the package directories only those records referred to are
    /// moved out of the library into its holding folder and deleted there
    /// on the cleanup thread (`Library::hold`, `purge_held`). With
    /// `removeData` its retired instance goes too: data directory, dormant
    /// record, secrets and session values. What cannot be moved or deleted
    /// is logged with its path and left, never in the way of an install.
    fn uninstall_set_aside(
        &mut self,
        id: &str,
        request: &str,
        spec: Value,
        params: &Value,
        revision: u64,
    ) -> std::result::Result<Value, RpcError> {
        if params["expectedRevision"].as_u64() != Some(revision) {
            return Err(RpcError::new(
                "revisionConflict",
                "installed revision changed",
            ));
        }
        self.library_busy(None)?;
        if self.package_rpc_busy(id) {
            return Err(busy());
        }
        let guid = self
            .packages
            .store
            .set_aside_installed()
            .get(id)
            .and_then(|(entry, _)| entry["instance"]["guid"].as_str())
            .filter(|guid| {
                self.packages
                    .store
                    .dormant()
                    .get(*guid)
                    .is_some_and(|dormant| dormant.id == id)
            })
            .map(str::to_owned);
        let receipt =
            json!({"operationId":request,"packageId":id,"status":"committed","request":spec});
        let freed = self
            .packages
            .store
            .forget_set_aside(id, request, receipt.clone())?;
        self.packages.receipts = self.packages.store.operations();
        self.hold_freed(&freed);
        self.failures.edit().remove(id);
        crate::capabilities::forget_command_trust(self, id);
        if let Some(guid) = guid {
            let key = DataKey::Instance(guid.clone());
            // As every uninstall: its session values and source references.
            self.session_storage.clear(&key);
            let _ = self
                .backend
                .commands
                .send(crate::playback::backend::Command {
                    reply: Reply { owner: 0, id: 0 },
                    method: "owner.sourcesRevoked".into(),
                    params: json!({"store": crate::playback::media_sources::store_key(&key)}),
                });
            if params["removeData"] == true {
                if let (Some(root), Some(directory)) =
                    (process::data_root(), self.data_directory_of(&key))
                {
                    match crate::packages::safe_remove::remove_within(&root, &directory) {
                        Ok(left) if left.is_empty() => {}
                        Ok(left) => {
                            diag!("uninstall {id}: data left in place: {}", left.join("; "))
                        }
                        Err(error) => diag!("uninstall {id}: data not removed: {error:#}"),
                    }
                }
                self.packages.store.forget_dormant(&guid)?;
                self.purge_secrets(&key);
            }
        }
        self.purge_dropped_secrets();
        Ok(receipt)
    }
    /// Loads the next installed plugins of the start order into the
    /// supervisor, which starts them, until `START_BUDGET` of this tick is
    /// spent (at least one). Starting a plugin spawns its Node process,
    /// several milliseconds each on Windows; all of them in one tick held
    /// the UI thread for 150 ms.
    fn load_queued(&mut self) {
        if self.packages.to_load.is_empty() {
            return;
        }
        let began = std::time::Instant::now();
        let mut started = 0;
        // A built-in whose digest is still being checked stays in its place
        // (loaded once the check is done, shared or alone, and third-party
        // first runs still wait for it: `set_builtins_to_load`); the others
        // pass it: a large package's first read can take seconds.
        let mut at = 0;
        while let Some(id) = self.packages.to_load.get(at).cloned() {
            if self.packages.verifying(&id) {
                at += 1;
                continue;
            }
            self.packages.to_load.remove(at);
            let builtins = self.builtins_to_load();
            self.supervisor.set_builtins_to_load(builtins);
            self.load_installed(&id);
            started += 1;
            if began.elapsed() >= START_BUDGET {
                break;
            }
        }
        let builtins = self.builtins_to_load();
        self.supervisor.set_builtins_to_load(builtins);
        // The core reads the switch itself (plan step S2f(3)).
        if crate::playback::render_target::FrameLog::enabled() {
            diag!(
                "vestra packages: {started} started in {:.1} ms, {} to go",
                began.elapsed().as_secs_f64() * 1000.,
                self.packages.to_load.len()
            );
        }
    }
    pub(crate) fn package_tick(&mut self) {
        self.purge_dropped_secrets();
        if !self.packages.booted {
            self.packages.booted = true;
            for recovery in self.packages.store.pending() {
                let request = self
                    .packages
                    .receipts
                    .get(&recovery.operation_id)
                    .map(|v| v["request"].clone())
                    .unwrap_or(Value::Null);
                let result = json!({"operationId":recovery.operation_id,"packageId":recovery.id,"status":"restoring","request":request});
                self.packages.receipts.remove(&recovery.operation_id);
                let old_key = recovery
                    .before
                    .instance
                    .as_ref()
                    .map(|i| DataKey::Instance(i.guid.clone()));
                let target_guid = recovery
                    .instance
                    .as_ref()
                    .or(recovery.before.instance.as_ref())
                    .map(|i| i.guid.clone())
                    .unwrap_or_else(package_store::new_guid);
                let fresh_data = recovery
                    .instance
                    .as_ref()
                    .filter(|i| {
                        old_key != Some(DataKey::Instance(i.guid.clone()))
                            && !self.packages.store.dormant().contains_key(&i.guid)
                    })
                    .and_then(|i| process::data_root().map(|root| root.join(&i.guid)));
                self.packages.operations.edit().insert(
                    recovery.operation_id.clone(),
                    Operation {
                        id: recovery.operation_id,
                        package: recovery.id,
                        target: recovery.target,
                        old: recovery.before.current,
                        target_key: DataKey::Instance(target_guid),
                        old_key,
                        fresh_data,
                        // Restored, never committed.
                        enabled: false,
                        phase: "restoring".into(),
                        owner: None,
                        job: None,
                        started: false,
                        cancel: false,
                        failure: Some(
                            "application stopped before commit; restoring last committed version"
                                .into(),
                        ),
                        result,
                        request,
                    },
                );
            }
            let recovering = self
                .packages
                .operations
                .values()
                .map(|op| (op.package.clone(), images_of(op)))
                .collect::<Vec<_>>();
            self.packages.images.extend(recovering);
            // The previous versions and built-in copies players up to 0.5
            // kept, and any leftovers, go on the cleanup thread.
            self.collect_garbage();
            // Load committed packages and the built-ins independently of
            // the authoring plugin: a few per tick (`START_BUDGET`), built-ins
            // first (`start_order`); the supervisor starts them.
            let builtins = &self.packages.builtins;
            self.packages.to_load = start_order(self.packages.store.installed_ids(), |id| {
                builtins.contains_key(id)
            });
            // The shared runner admits a built-in once its digest checked
            // out, off this thread (`verify`).
            if shared_runner::enabled() {
                let ids = self.packages.builtins.keys().cloned().collect::<Vec<_>>();
                for id in &ids {
                    self.packages.verify(id);
                }
                if self.packages.builtins.values().any(|b| b.digest.is_none()) {
                    diag!("built-in plugins without digests run in their own processes");
                }
            }
        }
        self.packages.poll_verified();
        self.verify_tick();
        self.answer_validations();
        self.shared_tick();
        self.load_queued();
        // The supervisor's jobs: starts whose previous process went, first
        // runs whose place freed, due idle checks, classifications and
        // "Starting…" timers.
        self.supervisor_tick();
        self.cleanup_tick();
        let keys = self
            .packages
            .operations
            .iter()
            .filter(|(_, op)| {
                !matches!(
                    op.phase.as_str(),
                    "committed" | "failed" | "cancelled" | "failedDegraded"
                )
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in keys {
            let mut op = self.packages.operations.edit().remove(&key).unwrap();
            let result = self.advance_package(&mut op);
            if let Err(error) = result {
                op.phase = "failedDegraded".into();
                op.result = json!({"operationId":op.id,"packageId":op.package,"status":"failedDegraded","message":error.to_string()});
            }
            op.result["request"] = op.request.clone();
            if matches!(
                op.phase.as_str(),
                "committed" | "failed" | "cancelled" | "failedDegraded"
            ) {
                if let Err(error) = self.packages.store.save_operation(&op.id, &op.result) {
                    diag!("operation receipt: {error}");
                }
                self.packages
                    .receipts
                    .insert(key.clone(), op.result.clone());
            }
            if matches!(
                op.phase.as_str(),
                "committed" | "failed" | "cancelled" | "failedDegraded"
            ) {
                self.packages.receipts = self.packages.store.operations();
                self.packages.images.remove(&op.package);
            } else {
                self.packages.operations.edit().insert(key, op);
            }
        }
    }
    fn advance_package(&mut self, op: &mut Operation) -> Result<()> {
        if matches!(
            op.phase.as_str(),
            "committed" | "failed" | "cancelled" | "failedDegraded"
        ) {
            return Ok(());
        }
        let stuck =
            || anyhow::anyhow!("cleanup acknowledgment timed out; recovery remains pending");
        let ended = self.operation_job(op);
        if op.phase == "preparing" {
            // Its previous process did not go within 20 s of the wait.
            if matches!(&ended, Some(JobResult::Failed(text)) if text == STUCK_RELOAD) {
                return Err(stuck());
            }
            if !op.cancel && !op.enabled {
                // It stays (or becomes) disabled: its files are installed and
                // nothing runs it, not even to check its activation; once the
                // version it switched off is gone.
                if ended.is_none() {
                    return Ok(());
                }
                let was_enabled = !self.packages.store.disabled().contains(&op.package);
                self.commit_operation(op, None)?;
                // Switched off by this install: its session values go, as for
                // `packages.setEnabled(false)`.
                if was_enabled && let Some(key) = &op.old_key {
                    self.session_storage.clear(key);
                }
                if was_enabled {
                    let id = op.package.clone();
                    self.triggers_clear(&id);
                }
                return Ok(());
            }
            // Its target starts once what it replaces is stopped (today's
            // `load_package` in this tick), as the supervisor's
            // `Restart{fresh, image}`.
            if !op.cancel && !op.started {
                if ended.is_none() {
                    return Ok(());
                }
                let unit = self.ensure_unit(&op.package);
                op.started = true;
                op.job = self.operation_enqueue(
                    unit,
                    JobKind::Restart {
                        fresh: true,
                        image: Some(op.target.digest.clone()),
                    },
                );
            }
            let ended = self.operation_job(op);
            // The supervisor started its target: its activation is watched.
            if let Some(owner) = self.running(&op.package) {
                op.owner = Some(owner);
                op.phase = "activating".into();
            } else if op.cancel {
                op.phase = "restoring".into();
                op.job = None;
            } else {
                match ended.clone() {
                    None => return Ok(()),
                    // Its process could not be created.
                    Some(JobResult::Failed(text)) => op.failure = Some(text),
                    Some(other) => op.failure = Some(format!("activation {other:?}")),
                }
                op.phase = "restoring".into();
                op.job = None;
            }
        }
        if op.phase == "activating" {
            let owner = op.owner.unwrap();
            let status = self.package_status(owner);
            if op.cancel || !matches!(status.as_str(), "ready" | "pending") {
                op.failure = Some(if op.cancel {
                    "cancelled before commit".into()
                } else {
                    format!("activation {status}: {}", self.activation(owner)?)
                });
                // Cancelled: the version it replaced comes back (an
                // `upgrade` in reverse), or, a first install, it goes (an
                // uninstall), which stops it (`restoring`); else its
                // activation failed and the supervisor stops it already.
                op.owner = None;
                op.phase = "restoring".into();
                op.job = None;
                return Ok(());
            }
            if status == "ready" {
                return self.commit_operation(op, Some(owner));
            }
        }
        if op.phase == "restoring" {
            // The disabled flag is still the one from before the install: a
            // package that was disabled is restored without running.
            let stopped = self.packages.store.disabled().contains(&op.package);
            let unit = self.ensure_unit(&op.package);
            if let Some(old) = op.old.clone().filter(|_| !stopped) {
                if op.job.is_none() && op.owner.is_none() {
                    // The old version starts once its target's process is gone
                    // (it stops it first when it still runs).
                    let recordable = self.checkpoint_key(&op.package).is_some();
                    self.supervisor.set_recordable(unit, recordable);
                    self.supervisor.set_enabled(unit, true);
                    op.job = self.operation_enqueue(
                        unit,
                        JobKind::Restart {
                            fresh: false,
                            image: Some(old.digest.clone()),
                        },
                    );
                    return Ok(());
                }
                if op.owner.is_none() {
                    match self.running(&op.package) {
                        Some(owner) => op.owner = Some(owner),
                        None => match self.operation_job(op) {
                            None => return Ok(()),
                            Some(JobResult::Failed(text)) if text == STUCK_RELOAD => {
                                return Err(stuck());
                            }
                            Some(JobResult::Failed(text)) => anyhow::bail!("{text}"),
                            Some(other) => anyhow::bail!(
                                "old version recovery failed: {other:?}; recovery remains pending"
                            ),
                        },
                    }
                    return Ok(());
                }
                let status = self.package_status(op.owner.unwrap());
                if status == "pending" {
                    return Ok(());
                }
                anyhow::ensure!(
                    status == "ready",
                    "old version recovery failed: {status}; recovery remains pending"
                );
            } else {
                // Nothing to restore: the target's process goes first (a
                // first install's unit goes with it, as its record does).
                if op.job.is_none() {
                    let reason = match (op.cancel, op.old.is_some()) {
                        (true, true) => crate::supervisor::StopReason::Upgrade,
                        (true, false) => crate::supervisor::StopReason::Uninstall,
                        (false, _) => crate::supervisor::StopReason::Failed,
                    };
                    let then = if op.old.is_some() {
                        Then::Inactive
                    } else {
                        Then::Removed
                    };
                    op.job = self.operation_enqueue(unit, JobKind::Stop { reason, then });
                }
                match self.operation_job(op) {
                    None => return Ok(()),
                    Some(JobResult::Failed(text)) if text == STUCK_RELOAD => return Err(stuck()),
                    Some(_) => {}
                }
                if op.old.is_some() {
                    let recordable = self.checkpoint_key(&op.package).is_some();
                    self.supervisor.set_recordable(unit, recordable);
                }
            }
            op.phase = if op.cancel { "cancelled" } else { "failed" }.into();
            // The new instance's data (it ran during activation) goes too.
            if let Some(directory) = op.fresh_data.take() {
                let _ = std::fs::remove_dir_all(directory);
            }
            op.result = json!({"operationId":op.id,"packageId":op.package,"status":op.phase,"message":op.failure,"restored":op.old.is_some(),"owner":op.owner,"request":op.request});
            self.packages
                .store
                .abort_with_receipt(&op.package, &op.id, op.result.clone())?;
            // The target staged for it is not kept.
            let target = op.target.digest.clone();
            self.discard(&target);
            return Ok(());
        }
        op.result = json!({"operationId":op.id,"packageId":op.package,"status":op.phase,"owner":op.owner,"cancelRequested":op.cancel});
        Ok(())
    }
    /// A package operation's job on `unit`: it merges with what is
    /// installed (`JobMode::Replace`; the operation is the busy RPC already),
    /// and runs now.
    fn operation_enqueue(
        &mut self,
        unit: UnitId,
        kind: JobKind,
    ) -> Option<crate::supervisor::JobId> {
        let now = self.now_ms();
        self.supervisor.set_now(now);
        let job = self
            .supervisor
            .enqueue_job(
                unit,
                kind,
                JobMode::Replace,
                Priority::Interactive,
                Origin::PackageRpc,
            )
            .ok();
        self.supervisor_run();
        job
    }
}
impl Core {
    /// Where plugin `id`, about to run from `directory`, runs (doc/
    /// architecture/README.md §2): in the shared runner only a
    /// built-in by provenance — listed in `builtin.json`, running from the
    /// installation's own directory, recorded as built-in in the catalog —
    /// whose files still have the digest packaging recorded, unless
    /// `VESTRA_SHARED=0`, the runner failed repeatedly this run, or a fault
    /// was attributed to it. Never from a manifest field; development loads
    /// (`Host::load`) never ask.
    pub(crate) fn trust(&self, id: &str, directory: &Path) -> Trust {
        trust_of(
            id,
            self.packages.builtins.get(id),
            directory,
            self.packages
                .store
                .get(id)
                .instance
                .is_some_and(|instance| instance.builtin),
            shared_runner::enabled(),
            &self.shared_policy,
            self.packages.verified.get(id),
        )
    }
    fn package_status(&mut self, owner: u64) -> String {
        self.activation(owner)
            .ok()
            .and_then(|v| v["activation"]["status"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "unknown".into())
    }
    /// Which instance an install goes into (store proposal §1.3): the same
    /// source upgrades, another source needs `replace` and reconnects to a
    /// dormant instance of that source when there is one. Built-ins are
    /// updated with the player only. `(instance, fresh)`: `fresh` when new
    /// data starts.
    fn install_instance(
        &self,
        id: &str,
        before: &package_store::Installed,
        source: Source,
        replace: bool,
    ) -> std::result::Result<(Option<Instance>, bool), RpcError> {
        install_instance(&self.packages.store, id, before, source, replace)
    }
    /// The supervisor's spec of plugin `id` (`add_unit`), enabled, without a
    /// checkpoint: a development load of the id runs instead of the
    /// installed copy; a built-in is checked (its digest) before it joins
    /// the shared runner; one without a recording key is never recorded.
    fn unit_spec(&self, id: &str) -> UnitSpec {
        if self.development.contains_key(id) {
            return UnitSpec::new(id, Class::Development).start_timeout(120_000);
        }
        let builtin = self.packages.builtins.get(id);
        let class = if builtin.is_some() {
            Class::Builtin
        } else {
            Class::Normal
        };
        let mut spec = UnitSpec::new(id, class);
        if self.checkpoint_key(id).is_none() {
            spec = spec.unrecordable();
        }
        if shared_runner::enabled()
            && builtin.is_some_and(|b| b.digest.is_some() && b.manifest.is_ok())
        {
            spec = spec.shared().verify();
        }
        spec
    }
    /// The unit of plugin `id`; loaded disabled (nothing starts) when the
    /// supervisor has none yet (a deferred record, a package RPC on an id
    /// start-up did not load).
    fn ensure_unit(&mut self, id: &str) -> UnitId {
        if let Some(unit) = self.supervisor.by_id(id) {
            return unit;
        }
        let spec = self.unit_spec(id).disabled();
        self.supervisor.add_unit(spec)
    }
    /// The unit a package operation on `id` starts and stops: loaded (a
    /// first install), or brought back by `add_unit` when it is being
    /// removed (a reinstall: it keeps waiting for the
    /// uninstalled process); loaded disabled, the operation enables it when
    /// it starts its target.
    fn operation_unit(&mut self, id: &str) -> UnitId {
        let spec = self.unit_spec(id).disabled();
        self.supervisor.add_unit(spec)
    }
    /// What the supervisor's `Replay` of `id` registers: the recording read
    /// at `load_installed`, the package and its recording key.
    pub(crate) fn packages_replay(
        &mut self,
        id: &str,
    ) -> Option<(
        checkpoint::Recording,
        PathBuf,
        package_stage::PackageManifest,
        checkpoint::Key,
    )> {
        let recording = self.packages.replays.remove(id)?;
        let record = self.packages.store.get(id);
        let (directory, manifest) = self.packages.runs_from(id, &record)?.ok()?;
        let key = self.checkpoint_key(id)?;
        Some((recording, directory, manifest, key))
    }
    /// The instance installed plugin `id` runs with (its recording's key).
    pub(crate) fn installed_instance(&self, id: &str) -> Option<String> {
        self.packages.store.get(id).instance.map(|i| i.guid)
    }
    /// A start of built-in `id` waits for its digest check (`Action::Verify`):
    /// the one start-up, a reload or a runner restart began, or one now.
    pub(crate) fn packages_verify_begin(&mut self, id: &str, unit: UnitId) {
        if !self.packages.verified.contains_key(id) {
            self.packages.verify(id);
        }
        self.packages.awaiting.insert(id.to_owned(), unit);
    }
    /// The digest checks a start waits for that are over, or past
    /// `VERIFY_WAIT` (then it runs in its own process, as today's
    /// `verifying` lets it start): the supervisor's `verify_done`.
    pub(crate) fn verify_tick(&mut self) {
        let over = self
            .packages
            .awaiting
            .iter()
            .filter(|(id, _)| !self.packages.verifying(id))
            .map(|(id, unit)| (id.clone(), *unit))
            .collect::<Vec<_>>();
        for (id, unit) in over {
            self.packages.awaiting.remove(&id);
            let ok = matches!(
                self.packages.verified.get(&id),
                Some(Verified::Done(Ok(())))
            );
            self.supervisor.verify_done(unit, ok);
        }
    }
    /// What a recording of installed plugin `id` is valid for: its instance,
    /// its package digest (a library copy's, or the one `builtin.json`
    /// lists) and version. `None`: it is not recorded (on-demand start
    /// leaves it running), e.g. a built-in listed without a digest.
    pub(crate) fn checkpoint_key(&self, id: &str) -> Option<checkpoint::Key> {
        let record = self.packages.store.get(id);
        let instance = record.instance.as_ref()?;
        let (digest, version) = if instance.builtin {
            let builtin = self.packages.builtins.get(id)?;
            (builtin.digest.clone()?, Some(builtin.version.clone()))
        } else {
            let current = record.current.as_ref()?;
            (current.digest.clone(), current.manifest.version.clone())
        };
        Some(checkpoint::Key {
            guid: instance.guid.clone(),
            digest,
            version,
        })
    }
    /// The directory recordings are kept in.
    pub(crate) fn activation_dir(&self) -> PathBuf {
        self.packages.store.activation_dir()
    }
    /// The directory kept trigger conditions are in (`crate::triggers`).
    pub(crate) fn triggers_dir(&self) -> PathBuf {
        self.packages.store.triggers_dir()
    }
    /// An installed (not development) plugin `owner` runs as installed:
    /// its recording key.
    pub(crate) fn checkpoint_key_of(&self, owner: u64) -> Option<checkpoint::Key> {
        let plugin = self.plugins.get(&owner)?;
        let DataKey::Instance(guid) = &plugin.data_key else {
            return None;
        };
        let id = &plugin.manifest.id;
        if self.development.contains_key(id) {
            return None;
        }
        let record = self.packages.store.get(id);
        let runs = self
            .packages
            .runs_from(id, &record)
            .is_some_and(|package| package.is_ok_and(|(dir, _)| dir == plugin.directory));
        self.checkpoint_key(id)
            .filter(|key| runs && key.guid == *guid)
    }
    /// A package RPC on `id` is answered `busy`: an operation runs on it
    /// (its phases are the package transaction's), or the supervisor
    /// has a package-RPC job of it installed (`JobMode::Fail`: a reload, an
    /// enable or an operation waiting for the previous process; today's
    /// `packages.reloads` entry).
    pub(crate) fn package_rpc_busy(&self, id: &str) -> bool {
        self.operation_pending(id) || self.package_job_busy(id)
    }
    /// While the `library-cleanup` thread scans the whole library (once, at
    /// start), installs and uninstalls wait; later runs remove only the
    /// directories they were given, and only an install of one of those
    /// (`digest`) waits.
    fn library_busy(&self, digest: Option<&str>) -> std::result::Result<(), RpcError> {
        if self
            .packages
            .cleanup
            .as_ref()
            .is_some_and(|running| running.blocks(digest))
        {
            return Err(RpcError::new(
                "busy",
                "the plugin library is being cleaned up; try again shortly",
            ));
        }
        Ok(())
    }
    /// At start: remove every package directory the catalog does not refer
    /// to (the built-in copies and previous versions that players up to 0.5
    /// kept, uninstalled packages, leftovers) on the `library-cleanup`
    /// thread, never on the UI thread: on Windows a large package takes
    /// seconds to delete.
    fn collect_garbage(&mut self) {
        // Never in a guest window: the library is the main player's.
        if self.packages.read_only() {
            return;
        }
        let library = self.packages.store.library();
        let referenced = self.packages.store.referenced();
        match cleanup::spawn(
            library,
            cleanup::Work::Unreferenced(referenced),
            self.packages.guests.clone(),
        ) {
            Ok(running) => self.packages.cleanup = Some(running),
            // Left for the next run; the directories are harmless.
            Err(error) => diag!("package library: {error}"),
        }
    }
    /// An operation freed `digest` (the version an upgrade replaced, an
    /// uninstalled package, a failed install's target): remove it on the
    /// cleanup thread, now or after its current run.
    fn discard(&mut self, digest: &str) {
        if self.packages.read_only() {
            return;
        }
        self.packages.cleanup_queue.insert(digest.to_owned());
        self.cleanup_tick();
    }
    /// Start the queued removals when the thread is free; the ones the
    /// catalog refers to again (reinstalled meanwhile) stay.
    fn cleanup_tick(&mut self) {
        if let Some(running) = &self.packages.cleanup {
            if !running.finished() {
                return;
            }
            self.packages.cleanup = None;
        }
        let referenced = self.packages.store.referenced();
        let digests = std::mem::take(&mut self.packages.cleanup_queue)
            .into_iter()
            .filter(|digest| !referenced.contains(digest))
            .collect::<BTreeSet<_>>();
        let work = if !digests.is_empty() {
            cleanup::Work::Only(digests)
        } else if std::mem::take(&mut self.packages.purge_held) {
            cleanup::Work::Held
        } else {
            return;
        };
        match cleanup::spawn(
            self.packages.store.library(),
            work,
            self.packages.guests.clone(),
        ) {
            Ok(running) => self.packages.cleanup = Some(running),
            Err(error) => diag!("package library: {error}"),
        }
    }
    /// Whether `id` may still be trusted when the approval sheet is
    /// answered: installed and not a built-in, or loaded for development.
    /// One uninstalled while the sheet was open is not trusted again.
    pub(crate) fn trustable_plugin(&self, id: &str) -> bool {
        trustable(
            &self.packages.store.get(id),
            self.development.contains_key(id),
        )
    }
    /// A running built-in plugin: shipped with the player and running from
    /// the installation (not a development load of the same id).
    pub(crate) fn is_builtin(&self, owner: u64) -> bool {
        let Some(plugin) = self.plugins.get(&owner) else {
            return false;
        };
        let installed = self.packages.store.get(&plugin.manifest.id);
        builtin_provenance(
            self.packages
                .builtins
                .get(&plugin.manifest.id)
                .map(|b| b.directory.as_path()),
            installed.instance.as_ref(),
            &plugin.directory,
            |guid| matches!(&plugin.data_key, process::DataKey::Instance(key) if key == guid),
        )
    }
    /// The package `origin` is a built-in plugin's (also for a plugin in
    /// standby, which has no process: its replayed sidebar tab, the
    /// built-in-only check of a replay): its directory and its data
    /// directory are the built-in instance's.
    pub(crate) fn is_builtin_package(&self, origin: &crate::host::Origin) -> bool {
        let installed = self.packages.store.get(&origin.id);
        builtin_provenance(
            self.packages
                .builtins
                .get(&origin.id)
                .map(|b| b.directory.as_path()),
            installed.instance.as_ref(),
            &origin.directory,
            |guid| {
                origin.data_directory
                    == process::data_root()
                        .map(|root| process::DataKey::Instance(guid.to_owned()).directory(&root))
            },
        )
    }
    /// Where a running plugin's shaders go in the shader channel
    /// (`playback/video_shaders.rs`): `(0, 0)` built-in, `(1, first
    /// install in Unix ms)` an installed instance, `(2, 0)` a development
    /// load.
    pub(crate) fn install_rank(&self, owner: u64) -> (u8, u64) {
        if self.is_builtin(owner) {
            return (0, 0);
        }
        let Some(plugin) = self.plugins.get(&owner) else {
            return (2, 0);
        };
        let process::DataKey::Instance(guid) = &plugin.data_key else {
            return (2, 0);
        };
        self.packages
            .store
            .get(&plugin.manifest.id)
            .instance
            .filter(|instance| &instance.guid == guid)
            .map_or((2, 0), |instance| (1, instance.installed_at))
    }
    /// Until the installed plugins started and while a built-in activates,
    /// other plugins' mounts wait (slot_claims).
    pub(crate) fn builtins_starting(&mut self) -> bool {
        // A built-in waiting for its first run (lazy activation) has not
        // claimed its slots yet.
        if !self.packages_started() || self.builtin_first_run_queued() {
            return true;
        }
        let now = self.now_ms();
        let builtins = self
            .plugins
            .keys()
            .copied()
            .filter(|owner| self.is_builtin(*owner))
            .collect::<Vec<_>>();
        builtins.into_iter().any(|owner| {
            matches!(
                self.activation_status(owner, now),
                Ok(activation::Status::Pending { .. })
            )
        })
    }
    /// A built-in waits for its first run (lazy activation, §2.4; the
    /// supervisor's first-run lane).
    pub(crate) fn builtin_first_run_queued(&self) -> bool {
        self.supervisor.first_runs_queued().1
    }
    fn operation_pending(&self, id: &str) -> bool {
        self.packages.operations.values().any(|op| {
            op.package == id
                && !matches!(
                    op.phase.as_str(),
                    "committed" | "failed" | "cancelled" | "failedDegraded"
                )
        })
    }
    /// What today's `start_stopped` checked before it started a stopped
    /// plugin (its development load, else its installed package), with the
    /// same replies; the supervisor's job starts it. The disabled flag is
    /// not touched.
    fn startable(
        &mut self,
        id: &str,
        installed: Option<std::result::Result<(PathBuf, package_stage::PackageManifest), String>>,
        reserved: std::result::Result<(), RpcError>,
        key: Option<DataKey>,
        development: bool,
    ) -> std::result::Result<(), RpcError> {
        self.failures.edit().remove(id);
        match (installed, development) {
            (_, true) => Ok(()),
            (Some(package), false) => {
                reserved?;
                // Nothing runnable (a deferred record, a broken built-in):
                // it stays listed as failed, with the same reason.
                package.map_err(|failure| {
                    self.failures.edit().insert(id.into(), failure.clone());
                    RpcError::new("unavailable", failure)
                })?;
                // Built-in records and installs both have an instance.
                key.ok_or_else(|| {
                    RpcError::new("unavailable", "the installed package has no instance")
                })?;
                Ok(())
            }
            (None, false) => Err(RpcError::new("missing", format!("no plugin {id}"))),
        }
    }
    /// Package directories a record set aside no longer keeps (`freed`):
    /// out of the library now (`Library::hold`), deleted on the cleanup
    /// thread. One that cannot be moved stays whole under its digest (the
    /// next start's `collect` takes it), logged.
    pub(crate) fn hold_freed(&mut self, freed: &BTreeSet<String>) {
        if freed.is_empty() || self.packages.read_only() {
            return;
        }
        let library = self.packages.store.library();
        // A version a running guest window uses stays (guest-windows.md §4);
        // the next start's `collect` takes it.
        let leased = crate::guest::leased(self.packages.guests.as_deref());
        for digest in freed {
            if leased.keeps(digest) {
                diag!("package library: {digest} kept while a temporary window uses it");
                continue;
            }
            if let Err(error) = library.hold(digest) {
                diag!("package library: {digest} left in place for the next start: {error:#}");
            }
        }
        self.packages.purge_held = true;
        self.cleanup_tick();
    }
    /// Deletes the secret store entries recorded for `key` (on a worker
    /// thread: keychain calls block and may prompt).
    fn purge_secrets(&self, key: &DataKey) {
        // A guest window deletes no keychain entries (they are the main
        // profile's).
        if self.packages.read_only() {
            return;
        }
        let index = self.packages.secrets.clone();
        let service = key.secret_service();
        let spawned = std::thread::Builder::new()
            .name("secrets-purge".into())
            .spawn(move || {
                index.purge(&service, platform::secret_delete);
            });
        if let Err(error) = spawned {
            diag!("secrets purge: {error}");
        }
    }
    /// Retired instances dropped because the dormant list was full can never
    /// be reconnected: their secrets and session values go too.
    fn purge_dropped_secrets(&mut self) {
        for (guid, _) in self.packages.store.take_dropped() {
            let key = DataKey::Instance(guid);
            self.session_storage.clear(&key);
            self.purge_secrets(&key);
        }
    }
    /// The installed packages, built-ins included, were started (the first
    /// `package_tick`): every one loaded into the supervisor, and none of
    /// their start-up starts still waiting for its turn or its digest check.
    pub(crate) fn packages_started(&self) -> bool {
        self.packages.booted && self.packages.to_load.is_empty() && !self.supervisor.boot_pending()
    }
    /// Built-ins still to load (`to_load` keeps them first, so they are its
    /// front).
    fn builtins_to_load(&self) -> usize {
        let builtins = &self.packages.builtins;
        self.packages
            .to_load
            .iter()
            .take_while(|id| builtins.contains_key(id.as_str()))
            .count()
    }
    /// The operation's job (`Operation.job`): `None` while it runs, else
    /// how it ended (`Done` when there is none: nothing to wait for).
    fn operation_job(&self, op: &Operation) -> Option<JobResult> {
        let (Some(unit), Some(job)) = (self.supervisor.by_id(&op.package), op.job) else {
            return Some(JobResult::Done);
        };
        if self
            .supervisor
            .job(unit)
            .is_some_and(|installed| installed.id == job)
        {
            return None;
        }
        // Only a unit's last 8 results are kept: a lost one is not taken for
        // a process that went.
        Some(
            self.supervisor
                .job_result(unit, job)
                .cloned()
                .unwrap_or_else(|| {
                    JobResult::Failed("the wait's result is no longer known".into())
                }),
        )
    }
    /// Commit `op` (its target activated as `owner`, or not activated at
    /// all when it stays disabled) with the disabled flag it decided.
    fn commit_operation(&mut self, op: &mut Operation, owner: Option<u64>) -> Result<()> {
        let receipt = json!({"operationId":op.id,"packageId":op.package,"status":"committed","owner":owner,"request":op.request});
        let outcome =
            self.packages
                .store
                .commit_with_receipt(&op.package, &op.id, receipt, Some(op.enabled));
        let installed = match outcome {
            Ok(installed) => installed,
            Err(error) => {
                let installed = self.packages.store.get(&op.package);
                if installed
                    .current
                    .as_ref()
                    .is_some_and(|p| p.digest == op.target.digest)
                    && self
                        .packages
                        .store
                        .operations()
                        .get(&op.id)
                        .is_some_and(|v| v["status"] == "committed")
                {
                    installed
                } else {
                    return Err(error);
                }
            }
        };
        op.phase = "committed".into();
        op.result = json!({"operationId":op.id,"packageId":op.package,"status":"committed","installed":installed,"owner":owner,"request":op.request});
        // The unit runs the committed package now: recorded from its next
        // run, and enabled as the commit decided.
        if let Some(unit) = self.supervisor.by_id(&op.package) {
            let recordable = self.checkpoint_key(&op.package).is_some();
            self.supervisor.set_recordable(unit, recordable);
            self.supervisor.set_enabled(unit, op.enabled);
        }
        // The version this replaced is not kept.
        if let Some(old) = &op.old {
            self.discard(&old.digest);
        }
        // A record of this id the library set aside is superseded: it goes
        // from `quarantine.json`, and the package directories only it kept.
        match self.packages.store.release_set_aside(&op.package) {
            Ok(freed) => self.hold_freed(&freed),
            Err(error) => diag!(
                "package library: {}: set-aside record kept: {error:#}",
                op.package
            ),
        }
        Ok(())
    }
}

/// Today's `busy` reply of a package RPC while another runs on the same
/// plugin (`Core::package_rpc_busy`; the supervisor's `Busy`).
fn busy() -> RpcError {
    RpcError::new("busy", crate::supervisor::Busy.to_string())
}

/// `packages.install`'s optional `enabled` (after host 0.6.3): whether the package
/// runs once installed. Absent (or `null`), a first install runs and an
/// update or reinstall keeps the installed package's state
/// (`package_store::enables_after_install`).
fn install_enabled(params: &Value) -> std::result::Result<Option<bool>, RpcError> {
    match params.get("enabled") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(enabled)) => Ok(Some(*enabled)),
        Some(_) => Err(RpcError::invalid("enabled must be a boolean")),
    }
}

/// `packages.install`'s `source` (default: a local file). `bundled` is the
/// player's own; repository and link addresses are http(s), normalized.
fn install_source(params: &Value) -> std::result::Result<Source, RpcError> {
    let Some(value) = params.get("source").filter(|v| !v.is_null()) else {
        return Ok(Source::File);
    };
    let source: Source = serde_json::from_value(value.clone())
        .map_err(|error| RpcError::invalid(format!("source: {error}")))?;
    let normalize = |url: &str| {
        let parsed = url::Url::parse(url)
            .ok()
            .filter(|u| matches!(u.scheme(), "http" | "https"))
            .ok_or_else(|| RpcError::invalid("source url must be an http(s) address"))?;
        let text = parsed.to_string();
        if text.len() > crate::packages::manifest::URL_MAX {
            return Err(RpcError::invalid("source url exceeds 2048 bytes"));
        }
        Ok(text)
    };
    Ok(match source {
        Source::Bundled => {
            return Err(RpcError::invalid(
                "source bundled is reserved for the player",
            ));
        }
        Source::Repository { url } => Source::Repository {
            url: normalize(&url)?,
        },
        Source::Link { url } => Source::Link {
            url: normalize(&url)?,
        },
        other => other,
    })
}

/// `PluginInfo.display`: the manifest's display texts in the interface
/// language, translated with the package's own catalog (the store and the
/// plugin manager show these; `null` fields are absent from the manifest).
fn display(manifest: &process::Manifest, catalog: Option<&crate::i18n::Catalog>) -> Value {
    let text = |value: &Option<String>| {
        value
            .as_deref()
            .map(|text| catalog.map_or(text, |c| c.text(text)).to_owned())
    };
    let tags: Vec<String> = manifest
        .tags
        .iter()
        .flatten()
        .map(|tag| catalog.map_or(tag.as_str(), |c| c.text(tag)).to_owned())
        .collect();
    json!({"name":text(&manifest.name),"punchline":text(&manifest.punchline),
        "description":text(&manifest.description),"tags":tags})
}

/// What `packages.list` shows of an installed record set aside at start,
/// from its JSON as read (`Store::set_aside_installed`): its version, its
/// source and its manifest's display fields (untranslated). Only what reads
/// as the expected type within the manifest limits; null otherwise.
fn set_aside_fields(entry: &Value) -> (Value, Value, Value) {
    let version = entry["current"]["manifest"]["version"]
        .as_str()
        .filter(|version| !version.is_empty() && version.len() <= 64)
        .map_or(Value::Null, |version| Value::String(version.into()));
    let source =
        serde_json::from_value::<package_store::Source>(entry["instance"]["source"].clone())
            .ok()
            .and_then(|source| serde_json::to_value(source).ok())
            .unwrap_or(Value::Null);
    (
        version,
        source,
        set_aside_display(&entry["current"]["manifest"]),
    )
}

fn set_aside_display(manifest: &Value) -> Value {
    use crate::packages::manifest::{DESCRIPTION_MAX, NAME_MAX, PUNCHLINE_MAX, TAG_MAX, TAGS_MAX};
    let text = |field: &str, max: usize| {
        manifest[field]
            .as_str()
            .filter(|text| !text.is_empty() && text.chars().count() <= max)
    };
    let tags: Vec<&str> = manifest["tags"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|tag| !tag.is_empty() && tag.chars().count() <= TAG_MAX)
        .take(TAGS_MAX)
        .collect();
    json!({"name":text("name", NAME_MAX),"punchline":text("punchline", PUNCHLINE_MAX),
        "description":text("description", DESCRIPTION_MAX),"tags":tags})
}

/// A plugin's name as shown, else its id (lazy activation's "Starting…").
pub(crate) fn display_name(
    manifest: &process::Manifest,
    catalog: Option<&crate::i18n::Catalog>,
) -> String {
    display(manifest, catalog)["name"]
        .as_str()
        .map_or_else(|| manifest.id.clone(), str::to_owned)
}

/// The 2026-10-24 ruling (ecosystem-plan.md): a reserved `vestra.` id
/// (`manifest::reserved_id`) is installed and started only as the
/// installation's built-in instance, which `builtin.json` alone creates
/// (`builtin::read` accepts only reserved ids). Store, file, link and
/// workshop installs of one are refused, and an installed non-built-in one
/// (from before the rule) is not started. Development loads (`--plugin`)
/// are not checked here (`Host::load`).
/// The failure shown for an installed package whose library copy could not
/// be read at start (`Store::deferred`).
fn deferred_failure(error: &str) -> String {
    format!("the plugin's files could not be read ({error}); try restarting the player")
        .chars()
        .take(512)
        .collect()
}
/// `Core::trust` as a table: `builtin` is `id`'s `builtin.json` entry,
/// `catalog_builtin` whether the catalog records its instance as built-in,
/// `enabled` whether sharing is on (`VESTRA_SHARED`), `verified` its digest
/// check.
fn trust_of(
    id: &str,
    builtin: Option<&builtin::Builtin>,
    directory: &Path,
    catalog_builtin: bool,
    enabled: bool,
    policy: &shared_runner::Policy,
    verified: Option<&Verified>,
) -> Trust {
    let Some(builtin) = builtin else {
        return Trust::Own(None);
    };
    let own = |reason: &str| Trust::Own(Some(reason.into()));
    if builtin.directory != directory || !catalog_builtin {
        return own("not the installation's built-in");
    }
    if !enabled {
        return own("VESTRA_SHARED=0");
    }
    if policy.failed_open {
        return own("the shared runner failed repeatedly in this run");
    }
    if policy.quarantined.contains(id) {
        return own("a fault was attributed to it in the shared runner");
    }
    if builtin.digest.is_none() {
        return own("builtin.json records no digest");
    }
    match verified {
        Some(Verified::Running(..)) => Trust::Pending,
        Some(Verified::Done(Ok(()))) => Trust::Shared,
        Some(Verified::Done(Err(error))) => Trust::Own(Some(error.clone())),
        None => own("its digest was not checked"),
    }
}
/// Built-in by provenance: `builtin.json` lists the id with this directory
/// (`listed`), the store records its instance built-in, and the run uses
/// that instance's data key (`runs_as(guid)`). A development load of the
/// same id, even of the installation's directory, runs under a
/// development key and is not built-in.
pub(crate) fn builtin_provenance(
    listed: Option<&std::path::Path>,
    instance: Option<&package_store::Instance>,
    directory: &std::path::Path,
    runs_as: impl Fn(&str) -> bool,
) -> bool {
    listed == Some(directory)
        && instance.is_some_and(|instance| instance.builtin && runs_as(&instance.guid))
}

pub(crate) fn check_reserved(
    id: &str,
    instance: Option<&Instance>,
) -> std::result::Result<(), RpcError> {
    if crate::packages::manifest::reserved_id(id) && !instance.is_some_and(|i| i.builtin) {
        return Err(RpcError::new(
            "reserved",
            crate::i18n::tr_args(
                "“{}”: ids starting with vestra. are reserved for the player's built-in plugins and cannot be installed",
                &serde_json::json!([id]),
            ),
        ));
    }
    Ok(())
}

/// Whether `packages.uninstall` may remove `installed` (`id`): a built-in
/// cannot be uninstalled, only disabled; a plugin that is only loaded for
/// development or running without a package is not installed; and the
/// caller must have read the current revision.
fn uninstallable(
    id: &str,
    installed: &package_store::Installed,
    loaded: bool,
    expected_revision: Option<u64>,
) -> std::result::Result<(), RpcError> {
    if installed.instance.as_ref().is_some_and(|i| i.builtin) {
        return Err(RpcError::new(
            "unsupported",
            "built-in plugins can be disabled but not uninstalled",
        ));
    }
    if installed.current.is_none() {
        return Err(if loaded {
            RpcError::new("unsupported", "only installed plugins can be uninstalled")
        } else {
            RpcError::new("missing", format!("no plugin {id}"))
        });
    }
    if expected_revision != Some(installed.revision) {
        return Err(RpcError::new(
            "revisionConflict",
            "installed revision changed",
        ));
    }
    Ok(())
}

/// The installation's built-in plugins (`builtin.json`, written by
/// `scripts/stage-plugins.mjs`). Each runs in place from its directory
/// under `<resources>/plugins`: nothing is copied or written there (each
/// directory is hashed once per shared-runner incarnation, `Manager::verify`).
/// The files are as safe as the rest of `<resources>` (the host runs
/// `runtime/runner.js` and `ui/host.slint` from there too): the `.app`
/// signature on macOS; on Windows a per-user install (`PrivilegesRequired=
/// lowest`) is writable by the user, so the digest guards against a damaged
/// or partial installation, not against a program running as the user.
mod builtin {
    use crate::packages::package_stage::PackageManifest;
    use anyhow::{Context, Result, ensure};
    use serde::Deserialize;
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::{Path, PathBuf};

    /// A built-in plugin of this installation.
    pub struct Builtin {
        /// The version `builtin.json` lists.
        pub version: String,
        /// Its directory (canonical), where it runs from.
        pub directory: PathBuf,
        /// Its manifest, or why the directory failed the checks (the
        /// plugin is then listed as `failed`).
        pub manifest: std::result::Result<PackageManifest, String>,
        /// Its package digest as packaging recorded it (schema 3); the
        /// shared runner admits it only while its directory still has it
        /// (`Core::trust`). `None` from a schema-2 list: its own process.
        pub digest: Option<String>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct List {
        schema: u32,
        plugins: Vec<Entry>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub struct Entry {
        pub id: String,
        pub version: String,
        /// The plugin's directory next to `builtin.json`: its id.
        pub directory: String,
        /// Schema 3: the package digest of that directory
        /// (`package_stage::inspect`), 64 lowercase hex digits.
        #[serde(default)]
        pub digest: Option<String>,
    }

    /// `$VESTRA_BUILTIN_PLUGINS`, else the installation's built-in plugins
    /// (`runtime_paths.rs`); none when it has no `builtin.json` (a
    /// development run).
    pub fn directory() -> Option<PathBuf> {
        crate::process::runtime_paths::get().builtin_directory()
    }

    /// `builtin.json` in `directory`, strictly: schema 2 or 3, at most 64
    /// plugins, each a reserved id (`vestra.`) in a directory of the same
    /// name, with a version, and in schema 3 with its package digest (only
    /// there).
    pub fn read(directory: &Path) -> Result<Vec<Entry>> {
        let bytes = std::fs::read(directory.join("builtin.json")).context("builtin.json")?;
        ensure!(bytes.len() <= 256 * 1024, "builtin.json is too large");
        let list: List = serde_json::from_slice(&bytes).context("builtin.json")?;
        ensure!(
            matches!(list.schema, 2 | 3),
            "builtin.json schema {} is unknown",
            list.schema
        );
        ensure!(
            list.plugins.len() <= 64,
            "builtin.json lists too many plugins"
        );
        let mut seen = BTreeSet::new();
        for entry in &list.plugins {
            ensure!(
                crate::packages::manifest::valid_id(&entry.id),
                "invalid id {}",
                entry.id
            );
            ensure!(
                crate::packages::manifest::reserved_id(&entry.id),
                "{}: built-in plugins have vestra. ids",
                entry.id
            );
            ensure!(seen.insert(&entry.id), "{} is listed twice", entry.id);
            ensure!(
                entry.directory == entry.id,
                "{}: its directory must be named after its id",
                entry.id
            );
            ensure!(
                crate::packages::manifest::parse_version(&entry.version).is_some(),
                "{}: invalid version {}",
                entry.id,
                entry.version
            );
            ensure!(
                match (list.schema, &entry.digest) {
                    (3, Some(digest)) => {
                        digest.len() == 64
                            && digest
                                .bytes()
                                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
                    }
                    (2, None) => true,
                    _ => false,
                },
                "{}: schema {} needs {} digest",
                entry.id,
                list.schema,
                if list.schema == 3 {
                    "a 64-digit lowercase hex"
                } else {
                    "no"
                }
            );
        }
        Ok(list.plugins)
    }

    /// Read `builtin.json` and check each plugin's directory: one small
    /// manifest read each, no hashing or copying (milliseconds for all of
    /// them). A plugin that fails is kept with the reason.
    pub fn load(directory: &Path) -> Result<BTreeMap<String, Builtin>> {
        let entries = read(directory)?;
        let root = dunce::canonicalize(directory).context("built-in plugins directory")?;
        Ok(entries
            .into_iter()
            .map(|entry| {
                let path = root.join(&entry.directory);
                let manifest = check(&root, &path, &entry).map_err(|error| format!("{error:#}"));
                if let Err(error) = &manifest {
                    diag!("built-in plugin {}: {error}", entry.id);
                }
                let directory = dunce::canonicalize(&path).unwrap_or(path);
                (
                    entry.id,
                    Builtin {
                        version: entry.version,
                        directory,
                        manifest,
                        digest: entry.digest,
                    },
                )
            })
            .collect())
    }

    /// `directory` is a real directory (not a link) inside `root`, and its
    /// manifest passes the package rules and names `entry`'s id and version.
    fn check(root: &Path, directory: &Path, entry: &Entry) -> Result<PackageManifest> {
        let metadata = std::fs::symlink_metadata(directory)
            .with_context(|| format!("{} is missing", entry.directory))?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "{} must be a directory",
            entry.directory
        );
        let canonical = dunce::canonicalize(directory)?;
        ensure!(
            canonical.parent() == Some(root),
            "{} is outside the built-in plugins directory",
            entry.directory
        );
        let manifest = crate::packages::manifest::parse(
            &std::fs::read(canonical.join("manifest.json")).context("manifest.json")?,
            crate::packages::manifest::player_version(),
        )?;
        crate::packages::manifest::check(&manifest, &canonical, true)?;
        ensure!(
            manifest.id == entry.id && manifest.version.as_deref() == Some(entry.version.as_str()),
            "its manifest is {} {:?}, not {} {}",
            manifest.id,
            manifest.version,
            entry.id,
            entry.version
        );
        Ok(manifest)
    }
}

/// The `library-cleanup` thread: removes package directories off the UI
/// thread (`Library::collect` / `Library::remove`).
mod cleanup {
    use crate::guest::Leases;
    use crate::packages::package_store::Library;
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use std::thread::JoinHandle;

    pub const THREAD: &str = "library-cleanup";

    pub enum Work {
        /// Everything the catalog does not refer to (`referenced`), and
        /// leftovers: at start, while nothing stages.
        Unreferenced(BTreeSet<String>),
        /// Only these directories (by digest).
        Only(BTreeSet<String>),
        /// Only what the holding folder holds (`Library::purge_held`).
        Held,
    }

    pub struct Running {
        handle: JoinHandle<()>,
        /// `None` while it scans the whole library.
        only: Option<BTreeSet<String>>,
    }
    impl Running {
        pub fn finished(&self) -> bool {
            self.handle.is_finished()
        }
        /// Whether staging `digest` (`None`: any change to the library)
        /// must wait for this run.
        pub fn blocks(&self, digest: Option<&str>) -> bool {
            match (&self.only, digest) {
                (None, _) => true,
                (Some(only), Some(digest)) => only.contains(digest),
                (Some(_), None) => false,
            }
        }
        #[cfg(test)]
        pub fn join(self) -> std::thread::Thread {
            let thread = self.handle.thread().clone();
            self.handle.join().unwrap();
            thread
        }
    }

    /// `guests`: the guests directory whose leases stay (`guest::leased`).
    pub fn spawn(
        library: Library,
        work: Work,
        guests: Option<PathBuf>,
    ) -> std::io::Result<Running> {
        let only = match &work {
            Work::Unreferenced(_) => None,
            Work::Only(digests) => Some(digests.clone()),
            // Never a digest name: no install waits for it.
            Work::Held => Some(BTreeSet::new()),
        };
        let handle = std::thread::Builder::new()
            .name(THREAD.into())
            .spawn(move || {
                let started = std::time::Instant::now();
                // At start, and after a forced uninstall: what the holding
                // folder holds (left there by an earlier run, say).
                if matches!(work, Work::Unreferenced(_) | Work::Held) {
                    let left = library.purge_held();
                    if !left.is_empty() {
                        diag!(
                            "package library: left in the holding folder for the next start: {}",
                            left.join("; ")
                        );
                    }
                }
                // The versions running guest windows use stay, read just
                // before deleting (guest-windows.md §4); the next start's
                // `collect` takes them once those guests are gone.
                // A guest whose lease cannot be read keeps everything
                // this time (`Leases::All`).
                let leased = crate::guest::leased(guests.as_deref());
                let result = match (&work, &leased) {
                    (Work::Unreferenced(_) | Work::Only(_), Leases::All) => {
                        diag!("package library: a temporary window's lease cannot be read; nothing removed this time");
                        Ok(0)
                    }
                    (Work::Unreferenced(referenced), Leases::These(leased)) => {
                        library.collect(&referenced.union(leased).cloned().collect())
                    }
                    (Work::Only(digests), Leases::These(leased)) => {
                        library.remove(&digests.difference(leased).cloned().collect())
                    }
                    (Work::Held, _) => Ok(0),
                };
                let ms = started.elapsed().as_millis();
                match result {
                    Ok(0) => {}
                    Ok(removed) => {
                        diag!("package library: {removed} unused package(s) removed in {ms} ms")
                    }
                    Err(error) => diag!("package library: {error:#}"),
                }
            })?;
        Ok(Running { handle, only })
    }
}

/// Whether the plugin of the catalog record `installed` may still be
/// trusted ("Don't ask again", Ask Vestra UX step 4): installed and not a
/// built-in, or a development load (`development`).
fn trustable(installed: &package_store::Installed, development: bool) -> bool {
    development
        || (installed.current.is_some()
            && !installed
                .instance
                .as_ref()
                .is_some_and(|instance| instance.builtin))
}

/// Whether installing from `source` replaces an installed instance of
/// another source (`replace`), not an update from the same one.
fn source_changes(before: &package_store::Installed, source: &Source) -> bool {
    before
        .instance
        .as_ref()
        .is_some_and(|current| current.source.identity() != source.identity())
}

/// Which instance an install goes into (store proposal §1.3).
fn install_instance(
    store: &Store,
    id: &str,
    before: &package_store::Installed,
    source: Source,
    replace: bool,
) -> std::result::Result<(Option<Instance>, bool), RpcError> {
    if let Some(current) = &before.instance {
        if current.builtin {
            return Err(RpcError::new(
                "unsupported",
                "built-in plugins are updated with the player",
            ));
        }
        if current.source.identity() == source.identity() {
            return Ok((
                Some(Instance {
                    source,
                    ..current.clone()
                }),
                false,
            ));
        }
        if !replace {
            return Err(RpcError::new(
                "conflict",
                format!(
                    "{id} is installed from another source; install with replace to replace it"
                ),
            ));
        }
    }
    Ok(match store.dormant_match(id, &source.identity()) {
        Some((guid, dormant)) => (
            Some(Instance {
                guid,
                source,
                builtin: false,
                installed_at: dormant.installed_at,
                seeded_version: None,
            }),
            false,
        ),
        None => (Some(Instance::new(source)), true),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packages::package_store::Installed;

    /// Built-in by provenance only: a development load of a built-in's id
    /// (even of the installation's own directory) runs under a development
    /// key and is not built-in (built-in-only capabilities, the shader rank).
    #[test]
    fn a_development_load_of_a_built_in_is_not_built_in() {
        let directory = std::path::Path::new("/app/plugins/vestra.workshop");
        let mut instance = package_store::Instance::new(package_store::Source::File);
        instance.builtin = true;
        let guid = instance.guid.clone();
        fn installed(key: &process::DataKey) -> impl Fn(&str) -> bool + use<'_> {
            move |g: &str| matches!(key, process::DataKey::Instance(k) if k == g)
        }
        let ours = process::DataKey::Instance(guid.clone());
        let development = process::DataKey::Development("vestra.workshop".into());
        assert!(builtin_provenance(
            Some(directory),
            Some(&instance),
            directory,
            installed(&ours)
        ));
        // The same directory, loaded with --plugin.
        assert!(!builtin_provenance(
            Some(directory),
            Some(&instance),
            directory,
            installed(&development)
        ));
        // Another directory, another instance, an instance not built-in.
        let elsewhere = std::path::Path::new("/src/plugins/workshop");
        assert!(!builtin_provenance(
            Some(directory),
            Some(&instance),
            elsewhere,
            installed(&ours)
        ));
        assert!(!builtin_provenance(
            None,
            Some(&instance),
            directory,
            installed(&ours)
        ));
        let other = process::DataKey::Instance("another".into());
        assert!(!builtin_provenance(
            Some(directory),
            Some(&instance),
            directory,
            installed(&other)
        ));
        instance.builtin = false;
        assert!(!builtin_provenance(
            Some(directory),
            Some(&instance),
            directory,
            installed(&ours)
        ));
        assert!(!builtin_provenance(
            Some(directory),
            None,
            directory,
            installed(&ours)
        ));
    }

    /// The start order (formerly `StartQueue::new`): built-ins first, then
    /// the others, each in id order. A built-in still being
    /// checked does not hold up the others: it is loaded in its turn and
    /// waits in `Activating{Verify}` while the others start, counted as
    /// pending (`supervisor/queue.rs` tests).
    #[test]
    fn start_order_puts_builtins_first() {
        let builtin = |id: &str| id.starts_with("vestra.");
        // `installed_ids` gives them in id order.
        let ids = ["a.one", "b.two", "vestra.audio", "vestra.video"].map(String::from);
        let order = start_order(ids, builtin);
        assert_eq!(order, ["vestra.audio", "vestra.video", "a.one", "b.two"]);
    }

    use crate::ui::composition;

    fn candidate(dir: &Path, id: &str, version: &str) -> Candidate {
        std::fs::write(
            dir.join("manifest.json"),
            serde_json::to_vec(&json!({"id":id,
            "apiVersion":process::API,"version":version,"entry":"main.js"}))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(dir.join("main.js"), format!("// {version}")).unwrap();
        Candidate::capture(dir).unwrap()
    }
    /// Install `version` of `id` into `store` the way the manager does.
    fn install(
        store: &mut Store,
        id: &str,
        version: &str,
        source: Source,
        replace: bool,
    ) -> std::result::Result<Installed, RpcError> {
        let dir = tempfile::tempdir().unwrap();
        let target = store.stage(&candidate(dir.path(), id, version)).unwrap();
        let before = store.get(id);
        let (instance, _) = install_instance(store, id, &before, source, replace)?;
        let op = format!("op-{}", package_store::new_guid());
        store
            .prepare_with_receipt(id, before.revision, &target, &op, json!({}), instance)
            .unwrap();
        Ok(store.commit(id, &op).unwrap())
    }
    fn repo(url: &str) -> Source {
        Source::Repository { url: url.into() }
    }

    /// N1: a record left as it is because this library's copy could not be
    /// read still names the other library's directory. Nothing runs it:
    /// `runs_from` (which `start_installed`, `set_enabled` and
    /// `package_list` all go through) answers the failure, never the
    /// directory.
    #[test]
    fn a_deferred_record_never_runs_from_another_library() {
        use crate::packages::package_store::tests::{hold_unreadable, moved_copy, two_installed};
        let first = tempfile::tempdir().unwrap();
        let first_path = dunce::canonicalize(first.path()).unwrap();
        let (counter, other) = two_installed(&first_path);
        let (_moved, moved_path) = moved_copy(&first_path);
        let copy = moved_path.join("packages").join(&counter.digest);
        let Some(_held) = hold_unreadable(&copy.join("main.js")) else {
            return;
        };
        let store = Store::open(moved_path.clone()).unwrap();
        assert!(store.deferred().contains_key("counter"));
        let manager = manager(store, BTreeMap::new());
        let record = manager.store.get("counter");
        assert_eq!(
            record.current.as_ref().unwrap().directory,
            counter.directory,
            "the record is left as it was"
        );
        let failure = manager.runs_from("counter", &record).unwrap().unwrap_err();
        assert!(failure.contains("could not be read"), "{failure}");
        assert!(
            !failure.contains(&*counter.directory.to_string_lossy()),
            "{failure}"
        );
        // The other package verified and runs from this library.
        let (directory, _) = manager
            .runs_from("other", &manager.store.get("other"))
            .unwrap()
            .unwrap();
        assert_eq!(directory, moved_path.join("packages").join(&other.digest));
    }

    /// A store install's target runs (activates) before its record names
    /// it: `packages.list` lists it installed, with the repository it came
    /// from, never as a development load (the store showed "Development
    /// load" for ~1 s after installing VestraTube). A plugin from any other
    /// directory still lists as one.
    #[test]
    fn an_activating_install_is_listed_installed_not_a_development_load() {
        let root = tempfile::tempdir().unwrap();
        let mut store = Store::open(root.path().into()).unwrap();
        let id = "a.tube";
        let dir = tempfile::tempdir().unwrap();
        let target = store.stage(&candidate(dir.path(), id, "1.0.0")).unwrap();
        let before = store.get(id);
        let source = repo("https://example.org/index.json");
        let (instance, _) = install_instance(&store, id, &before, source.clone(), false).unwrap();
        store
            .prepare_with_receipt(id, before.revision, &target, "op-1", json!({}), instance)
            .unwrap();
        let manager = manager(store, BTreeMap::new());
        let list = |directory: &Path| {
            let record = manager.store.get(id);
            let from_record = manager
                .runs_from(id, &record)
                .is_some_and(|p| p.is_ok_and(|(dir, _)| dir == directory));
            listed_instance(directory, from_record, record, manager.store.pending_of(id))
        };
        // Activating: the record does not run it yet (a first install has
        // none), the pending install does.
        assert!(manager.runs_from(id, &manager.store.get(id)).is_none());
        let (installed, instance) = list(&target.directory);
        assert!(
            installed,
            "an activating store install is not a development load"
        );
        assert_eq!(instance.map(|i| i.source), Some(source.clone()));
        // A development load of the same id runs from elsewhere.
        let elsewhere = tempfile::tempdir().unwrap();
        assert_eq!(list(elsewhere.path()), (false, None));
        // Committed: the record runs it.
        let mut manager = manager;
        manager.store.commit(id, "op-1").unwrap();
        assert!(manager.store.pending_of(id).is_none());
        let record = manager.store.get(id);
        let (installed, instance) = listed_instance(&target.directory, true, record, None);
        assert!(installed);
        assert_eq!(instance.map(|i| i.source), Some(source));
    }

    #[test]
    fn same_source_upgrades_and_another_source_needs_replace() {
        let root = tempfile::tempdir().unwrap();
        let mut store = Store::open(root.path().into()).unwrap();
        let first = install(
            &mut store,
            "a.b",
            "1.0.0",
            repo("https://one/index.json"),
            false,
        )
        .unwrap();
        let guid = first.instance.unwrap().guid;
        // "Don't ask again" goes with another source only (Ask Vestra UX
        // step 4): not with an update, nor a first install.
        assert!(!source_changes(
            &store.get("a.b"),
            &repo("https://one/index.json")
        ));
        assert!(source_changes(
            &store.get("a.b"),
            &repo("https://two/index.json")
        ));
        assert!(!source_changes(
            &store.get("c.d"),
            &repo("https://two/index.json")
        ));
        let second = install(
            &mut store,
            "a.b",
            "1.1.0",
            repo("https://one/index.json"),
            false,
        )
        .unwrap();
        assert_eq!(
            second.instance.as_ref().unwrap().guid,
            guid,
            "upgrade keeps the instance"
        );
        assert!(second.previous.is_none(), "no previous version is kept");
        let error = install(
            &mut store,
            "a.b",
            "2.0.0",
            repo("https://two/index.json"),
            false,
        )
        .unwrap_err();
        assert_eq!(error.code, "conflict");
        assert_eq!(
            store
                .get("a.b")
                .current
                .unwrap()
                .manifest
                .version
                .as_deref(),
            Some("1.1.0")
        );
        // Replace: a new instance, the old one dormant, nothing kept across.
        let replaced = install(
            &mut store,
            "a.b",
            "2.0.0",
            repo("https://two/index.json"),
            true,
        )
        .unwrap();
        let new_guid = replaced.instance.unwrap().guid;
        assert_ne!(new_guid, guid);
        assert!(replaced.previous.is_none());
        assert_eq!(store.dormant()[&guid].version.as_deref(), Some("1.1.0"));
        // Switching back reconnects to the dormant instance's data.
        let back = install(
            &mut store,
            "a.b",
            "1.2.0",
            repo("https://one/index.json"),
            true,
        )
        .unwrap();
        assert_eq!(back.instance.unwrap().guid, guid);
        assert!(!store.dormant().contains_key(&guid));
        assert!(store.dormant().contains_key(&new_guid));
        drop(store);
        let store = Store::open(root.path().into()).unwrap();
        assert_eq!(store.get("a.b").instance.unwrap().guid, guid, "persisted");
    }

    #[test]
    fn links_and_files_are_one_manual_source_and_workshop_is_another() {
        let root = tempfile::tempdir().unwrap();
        let mut store = Store::open(root.path().into()).unwrap();
        let first = install(
            &mut store,
            "x",
            "1.0.0",
            Source::Link {
                url: "https://a/x.vxp".into(),
            },
            false,
        )
        .unwrap();
        let next = install(&mut store, "x", "1.1.0", Source::File, false).unwrap();
        assert_eq!(
            first.instance.unwrap().guid,
            next.instance.as_ref().unwrap().guid
        );
        assert_eq!(
            next.instance.unwrap().source,
            Source::File,
            "latest source recorded"
        );
        assert_eq!(
            install(&mut store, "x", "1.2.0", Source::Workshop, false)
                .unwrap_err()
                .code,
            "conflict"
        );
    }

    #[test]
    fn built_ins_refuse_every_install() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path().into()).unwrap();
        let before = Installed {
            instance: Some(Instance {
                builtin: true,
                ..Instance::new(Source::Bundled)
            }),
            ..Default::default()
        };
        for (source, replace) in [
            (Source::File, false),
            (Source::File, true),
            (Source::Bundled, false),
        ] {
            let error =
                install_instance(&store, "vestra.video", &before, source, replace).unwrap_err();
            assert_eq!(error.code, "unsupported");
        }
    }

    #[test]
    fn reserved_ids_run_only_as_built_ins() {
        let builtin = Instance {
            builtin: true,
            ..Instance::new(Source::Bundled)
        };
        let installed = Instance::new(Source::Repository {
            url: "https://example.com/index.json".into(),
        });
        for id in ["vestra.youtube", "Vestra.video", "vestra"] {
            for instance in [None, Some(&installed)] {
                let error = check_reserved(id, instance).unwrap_err();
                assert_eq!(error.code, "reserved", "{id}");
                assert!(error.message.contains(id) && error.message.contains("built-in plugins"));
            }
            assert!(check_reserved(id, Some(&builtin)).is_ok(), "{id}");
        }
        for id in ["zysilm.youtube", "workshop.counter", "vestrafan.x"] {
            assert!(check_reserved(id, None).is_ok());
            assert!(check_reserved(id, Some(&installed)).is_ok());
        }
    }

    #[test]
    fn uninstall_keeping_data_lets_the_same_source_reconnect() {
        let root = tempfile::tempdir().unwrap();
        let mut store = Store::open(root.path().into()).unwrap();
        let guid = install(&mut store, "k", "1.0.0", Source::File, false)
            .unwrap()
            .instance
            .unwrap()
            .guid;
        // "Don't ask again" (Ask Vestra UX step 4): an installed plugin may
        // be trusted, one uninstalled while the sheet was open may not
        // (unless a development load of its id runs).
        assert!(trustable(&store.get("k"), false));
        store.uninstall("k", "rm", json!({}), true).unwrap();
        assert!(!trustable(&store.get("k"), false));
        assert!(trustable(&store.get("k"), true));
        let again = install(&mut store, "k", "1.0.0", Source::File, false).unwrap();
        assert_eq!(again.instance.unwrap().guid, guid);
        store.uninstall("k", "rm2", json!({}), false).unwrap();
        assert!(store.dormant().is_empty());
        let fresh = install(&mut store, "k", "1.0.0", Source::File, false).unwrap();
        assert_ne!(fresh.instance.unwrap().guid, guid);
    }

    /// After the forced uninstall of a record set aside, the same id
    /// installs as new: without `removeData` it reattaches the retired
    /// instance's data (as any uninstall that keeps data); with it (the
    /// dormant record forgotten) it starts with new data.
    #[test]
    fn a_set_aside_id_installs_again_after_its_uninstall() {
        for remove_data in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let root_path = dunce::canonicalize(root.path()).unwrap();
            let mut store = Store::open(root_path.clone()).unwrap();
            let guid = install(&mut store, "k", "1.0.0", Source::File, false)
                .unwrap()
                .instance
                .unwrap()
                .guid;
            drop(store);
            let path = root_path.join("catalog.json");
            let mut catalog: Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            catalog["installed"]["k"]["current"]["manifest"]["capabilities"] = json!(["events"]);
            std::fs::write(&path, serde_json::to_vec(&catalog).unwrap()).unwrap();
            let mut store = Store::open(root_path.clone()).unwrap();
            assert!(store.set_aside_installed().contains_key("k"));
            let freed = store.forget_set_aside("k", "rm", json!({})).unwrap();
            assert_eq!(freed.len(), 1);
            if remove_data {
                store.forget_dormant(&guid).unwrap();
            }
            let again = install(&mut store, "k", "1.0.0", Source::File, false).unwrap();
            assert_eq!(
                again.instance.unwrap().guid == guid,
                !remove_data,
                "removeData {remove_data}"
            );
            assert!(store.release_set_aside("k").unwrap().is_empty());
        }
    }

    /// An installed record set aside is listed with its version, source
    /// and display fields read from its JSON; what does not read within
    /// the manifest limits is null, never the record of another.
    #[test]
    fn a_set_aside_record_is_listed_with_its_own_fields() {
        let entry = json!({"current":{"manifest":{"id":"z.amd","version":"0.1.0",
            "name":"AMD Motion Smoothing","punchline":"Smoother","tags":["video", 3, "x".repeat(25)],
            "capabilities":["events"]}},
            "instance":{"guid":"g","source":{"kind":"repository","url":"https://example.com/index.json"},
            "installedAt":1}});
        let (version, source, display) = set_aside_fields(&entry);
        assert_eq!(version, "0.1.0");
        assert_eq!(
            source,
            json!({"kind":"repository","url":"https://example.com/index.json"})
        );
        assert_eq!(
            display,
            json!({"name":"AMD Motion Smoothing","punchline":"Smoother","description":null,"tags":["video"]})
        );
        let (version, source, display) = set_aside_fields(&json!({"truncated": 70000}));
        assert_eq!((version, source), (Value::Null, Value::Null));
        assert_eq!(
            display,
            json!({"name":null,"punchline":null,"description":null,"tags":[]})
        );
        let long = json!({"current":{"manifest":{"name":"n".repeat(65),"version":7}},
            "instance":{"source":{"kind":"unknown"}}});
        let (version, source, display) = set_aside_fields(&long);
        assert_eq!((version, source), (Value::Null, Value::Null));
        assert_eq!(display["name"], Value::Null);
    }

    /// `packages.install`'s optional `enabled`: absent for SDKs that do not
    /// send it, a boolean otherwise.
    #[test]
    fn install_enabled_is_optional_and_boolean() {
        assert_eq!(install_enabled(&json!({})).unwrap(), None);
        assert_eq!(install_enabled(&json!({"enabled":null})).unwrap(), None);
        assert_eq!(
            install_enabled(&json!({"enabled":true})).unwrap(),
            Some(true)
        );
        assert_eq!(
            install_enabled(&json!({"enabled":false})).unwrap(),
            Some(false)
        );
        assert_eq!(
            install_enabled(&json!({"enabled":"yes"})).unwrap_err().code,
            "invalid"
        );
    }

    #[test]
    fn install_sources_are_checked_and_normalized() {
        assert_eq!(install_source(&json!({})).unwrap(), Source::File);
        assert_eq!(
            install_source(
                &json!({"source":{"kind":"repository","url":"HTTPS://Example.com/a/../index.json"}})
            )
            .unwrap(),
            Source::Repository {
                url: "https://example.com/index.json".into()
            }
        );
        for bad in [
            json!({"kind":"bundled"}),
            json!({"kind":"link","url":"file:///x.vxp"}),
            json!({"kind":"link"}),
            json!({"kind":"other"}),
        ] {
            assert_eq!(
                install_source(&json!({ "source": bad })).unwrap_err().code,
                "invalid",
                "{bad}"
            );
        }
    }

    /// A built-in plugin directory `<root>/<id>` with `version`, as
    /// `stage-plugins.mjs` unpacks it.
    fn builtin_dir(root: &Path, id: &str, version: &str) {
        let dir = root.join(id);
        std::fs::create_dir_all(dir.join("ui")).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            serde_json::to_vec(&json!({"id":id,"apiVersion":process::API,
                "version":version,"entry":"main.js"}))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(dir.join("main.js"), "// built in").unwrap();
        std::fs::write(dir.join("ui/a.slint"), "export component A {}").unwrap();
    }
    fn builtin_json(root: &Path, plugins: &[(&str, &str)]) {
        let plugins = plugins
            .iter()
            .map(|(id, version)| json!({"id":id,"version":version,"directory":id}))
            .collect::<Vec<_>>();
        std::fs::write(
            root.join("builtin.json"),
            json!({"schema":2,"plugins":plugins}).to_string(),
        )
        .unwrap();
    }
    /// Every file under `root` with its size and modification time.
    fn snapshot(root: &Path) -> Vec<(PathBuf, u64, std::time::SystemTime)> {
        let mut files = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(next) = stack.pop() {
            for item in std::fs::read_dir(next).unwrap() {
                let path = item.unwrap().path();
                let metadata = std::fs::metadata(&path).unwrap();
                if metadata.is_dir() {
                    stack.push(path.clone());
                }
                files.push((path, metadata.len(), metadata.modified().unwrap()));
            }
        }
        files.sort();
        files
    }
    fn set_read_only(root: &Path, read_only: bool) {
        for (path, ..) in snapshot(root).into_iter().rev() {
            let mut permissions = std::fs::metadata(&path).unwrap().permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(read_only);
            std::fs::set_permissions(&path, permissions).unwrap();
        }
    }

    #[test]
    fn builtin_lists_are_strict() {
        let dir = tempfile::tempdir().unwrap();
        let write = |value: Value| {
            std::fs::write(dir.path().join("builtin.json"), value.to_string()).unwrap()
        };
        write(
            json!({"schema":2,"plugins":[{"id":"vestra.video","version":"1.0.0","directory":"vestra.video"}]}),
        );
        assert_eq!(builtin::read(dir.path()).unwrap().len(), 1);
        let entry = |id: &str, version: &str, directory: &str| json!({"id":id,"version":version,"directory":directory});
        for bad in [
            // Schema 1 listed .vxp archives (players up to 0.5).
            json!({"schema":1,"plugins":[]}),
            json!({"schema":2,"plugins":[{"id":"vestra.v","version":"1.0.0","file":"v.vxp","sha256":"a"}]}),
            json!({"schema":2,"plugins":[entry("vestra.v","1.0.0","../vestra.v")]}),
            json!({"schema":2,"plugins":[entry("vestra.v","1.0.0","other")]}),
            json!({"schema":2,"plugins":[entry("vestra.v","1.0","vestra.v")]}),
            // Only reserved ids are built in.
            json!({"schema":2,"plugins":[entry("zysilm.youtube","1.0.0","zysilm.youtube")]}),
            json!({"schema":2,"plugins":[entry("vestra.v","1.0.0","vestra.v"),entry("vestra.v","1.0.1","vestra.v")]}),
            json!({"schema":2,"plugins":[{"id":"vestra.v","version":"1.0.0","directory":"vestra.v","extra":1}]}),
            // Schema 3 records each package digest (stage-plugins.mjs), and only it.
            json!({"schema":4,"plugins":[]}),
            json!({"schema":3,"plugins":[entry("vestra.v","1.0.0","vestra.v")]}),
            json!({"schema":3,"plugins":[{"id":"vestra.v","version":"1.0.0","directory":"vestra.v","digest":"AB"}]}),
            json!({"schema":3,"plugins":[{"id":"vestra.v","version":"1.0.0","directory":"vestra.v","digest":"A".repeat(64)}]}),
            json!({"schema":2,"plugins":[{"id":"vestra.v","version":"1.0.0","directory":"vestra.v","digest":"a".repeat(64)}]}),
        ] {
            write(bad.clone());
            assert!(builtin::read(dir.path()).is_err(), "{bad}");
        }
        write(
            json!({"schema":3,"plugins":[{"id":"vestra.v","version":"1.0.0","directory":"vestra.v","digest":"0a".repeat(32)}]}),
        );
        let entries = builtin::read(dir.path()).unwrap();
        assert_eq!(entries[0].digest.as_deref(), Some("0a".repeat(32).as_str()));
    }

    fn manager(store: Store, builtins: BTreeMap<String, builtin::Builtin>) -> Manager {
        Manager {
            store,
            secrets: crate::packages::secret_index::Index::new(None),
            candidates: BTreeMap::new(),
            candidate_order: VecDeque::new(),
            operations: Revised::default(),
            next: 0,
            receipts: BTreeMap::new(),
            booted: false,
            to_load: VecDeque::new(),
            awaiting: BTreeMap::new(),
            replays: BTreeMap::new(),
            images: BTreeMap::new(),
            builtins,
            cleanup: None,
            cleanup_queue: BTreeSet::new(),
            purge_held: false,
            verified: BTreeMap::new(),
            verify_serial: 0,
            verify_results: std::sync::mpsc::channel(),
            validations: Default::default(),
            guests: None,
        }
    }

    /// The digest check runs off the caller's thread and tells a match, a
    /// changed file and a newer check apart.
    #[test]
    fn builtin_digests_are_checked_off_thread() {
        let root = tempfile::tempdir().unwrap();
        builtin_dir(root.path(), "vestra.good", "1.0.0");
        builtin_dir(root.path(), "vestra.patched", "1.0.0");
        let digest = |id: &str| package_stage::inspect(&root.path().join(id)).unwrap().0;
        std::fs::write(
            root.path().join("builtin.json"),
            json!({"schema":3,"plugins":[
                {"id":"vestra.good","version":"1.0.0","directory":"vestra.good","digest":digest("vestra.good")},
                {"id":"vestra.patched","version":"1.0.0","directory":"vestra.patched","digest":digest("vestra.patched")}]})
            .to_string(),
        )
        .unwrap();
        std::fs::write(root.path().join("vestra.patched/main.js"), "// patched").unwrap();
        let store_root = tempfile::tempdir().unwrap();
        let mut manager = manager(
            Store::open(store_root.path().to_path_buf()).unwrap(),
            builtin::load(root.path()).unwrap(),
        );
        manager.verify("vestra.good");
        manager.verify("vestra.patched");
        manager.verify("vestra.missing");
        assert!(!manager.verifying("vestra.missing"));
        let began = std::time::Instant::now();
        let running = |manager: &Manager, id: &str| {
            matches!(manager.verified.get(id), Some(Verified::Running(..)))
        };
        while running(&manager, "vestra.good") || running(&manager, "vestra.patched") {
            assert!(began.elapsed() < std::time::Duration::from_secs(10));
            std::thread::sleep(std::time::Duration::from_millis(5));
            manager.poll_verified();
        }
        assert!(matches!(
            manager.verified["vestra.good"],
            Verified::Done(Ok(()))
        ));
        assert!(
            matches!(&manager.verified["vestra.patched"], Verified::Done(Err(e)) if e.contains("builtin.json records"))
        );
        // A check started again supersedes the finished one until it reports;
        // a start waits for a check only so long.
        manager.verify("vestra.good");
        assert!(manager.verifying("vestra.good"));
        manager.verified.insert(
            "vestra.good".into(),
            Verified::Running(0, std::time::Instant::now() - VERIFY_WAIT),
        );
        assert!(!manager.verifying("vestra.good"));
    }

    /// Schema 3: the digest the list records is what `package_stage::inspect`
    /// finds in the directory as long as nothing in it changed.
    #[test]
    fn a_builtin_carries_its_recorded_digest_and_inspect_tells_a_changed_file() {
        let root = tempfile::tempdir().unwrap();
        builtin_dir(root.path(), "vestra.demo", "1.2.0");
        let (digest, _) = package_stage::inspect(&root.path().join("vestra.demo")).unwrap();
        std::fs::write(
            root.path().join("builtin.json"),
            json!({"schema":3,"plugins":[{"id":"vestra.demo","version":"1.2.0",
                "directory":"vestra.demo","digest":digest}]})
            .to_string(),
        )
        .unwrap();
        let builtins = builtin::load(root.path()).unwrap();
        assert_eq!(
            builtins["vestra.demo"].digest.as_deref(),
            Some(digest.as_str())
        );
        std::fs::write(root.path().join("vestra.demo/main.js"), "// patched").unwrap();
        let (patched, _) = package_stage::inspect(&root.path().join("vestra.demo")).unwrap();
        assert_ne!(patched, digest);
    }

    /// Who runs in the shared runner: built-ins by provenance whose digest
    /// checked out, unless sharing is off, failed open or the plugin faulted.
    #[test]
    fn only_verified_builtins_from_the_installation_share_the_runner() {
        let root = tempfile::tempdir().unwrap();
        builtin_dir(root.path(), "vestra.demo", "1.2.0");
        builtin_json(root.path(), &[("vestra.demo", "1.2.0")]);
        let mut builtins = builtin::load(root.path()).unwrap();
        let directory = builtins["vestra.demo"].directory.clone();
        let ok = Verified::Done(Ok(()));
        let policy = shared_runner::Policy::default();
        let trust = |builtin: Option<&builtin::Builtin>,
                     directory: &Path,
                     catalog: bool,
                     enabled: bool,
                     policy: &shared_runner::Policy,
                     verified: Option<&Verified>| {
            trust_of(
                "vestra.demo",
                builtin,
                directory,
                catalog,
                enabled,
                policy,
                verified,
            )
        };
        let reason = |trust: Trust| match trust {
            Trust::Own(Some(reason)) => reason,
            other => panic!("{other:?}"),
        };
        // Schema 2: no digest, no sharing.
        let demo = builtins.get("vestra.demo");
        assert!(
            reason(trust(demo, &directory, true, true, &policy, Some(&ok))).contains("no digest")
        );
        builtins.get_mut("vestra.demo").unwrap().digest = Some("0".repeat(64));
        let demo = builtins.get("vestra.demo");
        assert_eq!(
            trust(demo, &directory, true, true, &policy, Some(&ok)),
            Trust::Shared
        );
        // Not a built-in at all: its own process, nothing to log.
        assert_eq!(
            trust(None, &directory, false, true, &policy, None),
            Trust::Own(None)
        );
        // Another directory (a development copy), or not built-in in the catalog.
        assert!(
            reason(trust(demo, root.path(), true, true, &policy, Some(&ok)))
                .contains("not the installation")
        );
        assert!(
            reason(trust(demo, &directory, false, true, &policy, Some(&ok)))
                .contains("not the installation")
        );
        // The digest check: running, failed, never run.
        assert_eq!(
            trust(
                demo,
                &directory,
                true,
                true,
                &policy,
                Some(&Verified::Running(1, std::time::Instant::now()))
            ),
            Trust::Pending
        );
        let mismatch = Verified::Done(Err("its files have digest x".into()));
        assert!(
            reason(trust(
                demo,
                &directory,
                true,
                true,
                &policy,
                Some(&mismatch)
            ))
            .contains("digest x")
        );
        assert!(reason(trust(demo, &directory, true, true, &policy, None)).contains("not checked"));
        // VESTRA_SHARED=0, fail-open, quarantine.
        assert!(
            reason(trust(demo, &directory, true, false, &policy, Some(&ok)))
                .contains("VESTRA_SHARED=0")
        );
        let mut failed = shared_runner::Policy::default();
        failed.failed_open = true;
        assert!(
            reason(trust(demo, &directory, true, true, &failed, Some(&ok)))
                .contains("failed repeatedly")
        );
        let mut quarantined = shared_runner::Policy::default();
        quarantined.quarantined.insert("vestra.demo".into());
        assert!(
            reason(trust(demo, &directory, true, true, &quarantined, Some(&ok))).contains("fault")
        );
    }

    /// Built-ins run in place: the registry reads each manifest from the
    /// installation, which may be read-only, and writes nothing there.
    #[test]
    fn builtins_load_in_place_from_a_read_only_installation() {
        let resources = tempfile::tempdir().unwrap();
        let plugins_dir = resources.path().join("plugins");
        std::fs::create_dir(&plugins_dir).unwrap();
        builtin_dir(&plugins_dir, "vestra.demo", "1.2.0");
        builtin_json(&plugins_dir, &[("vestra.demo", "1.2.0")]);
        set_read_only(&plugins_dir, true);
        let before = snapshot(&plugins_dir);
        let builtins = builtin::load(&plugins_dir).unwrap();
        let demo = &builtins["vestra.demo"];
        assert_eq!(
            demo.directory,
            dunce::canonicalize(plugins_dir.join("vestra.demo")).unwrap(),
            "runs from the installation"
        );
        let manifest = demo.manifest.as_ref().unwrap();
        assert_eq!(manifest.version.as_deref(), Some("1.2.0"));
        // Its entry resolves inside its directory, as `Plugin::load_manifest` checks.
        process::package_file(&demo.directory, &manifest.entry).unwrap();
        assert_eq!(snapshot(&plugins_dir), before, "nothing written");
        set_read_only(&plugins_dir, false);
    }

    #[test]
    fn a_builtin_that_fails_its_checks_is_kept_with_the_reason() {
        let plugins_dir = tempfile::tempdir().unwrap();
        let root = plugins_dir.path();
        builtin_dir(root, "vestra.good", "1.0.0");
        builtin_dir(root, "vestra.mislabelled", "2.0.0");
        builtin_json(
            root,
            &[
                ("vestra.good", "1.0.0"),
                ("vestra.mislabelled", "2.0.1"),
                ("vestra.missing", "1.0.0"),
            ],
        );
        let builtins = builtin::load(root).unwrap();
        assert!(builtins["vestra.good"].manifest.is_ok());
        let error = builtins["vestra.mislabelled"]
            .manifest
            .as_ref()
            .unwrap_err();
        assert!(error.contains("not vestra.mislabelled 2.0.1"), "{error}");
        let error = builtins["vestra.missing"].manifest.as_ref().unwrap_err();
        assert!(error.contains("missing"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn a_builtin_directory_that_is_a_link_is_refused() {
        let plugins_dir = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        builtin_dir(elsewhere.path(), "vestra.demo", "1.0.0");
        std::os::unix::fs::symlink(
            elsewhere.path().join("vestra.demo"),
            plugins_dir.path().join("vestra.demo"),
        )
        .unwrap();
        builtin_json(plugins_dir.path(), &[("vestra.demo", "1.0.0")]);
        let builtins = builtin::load(plugins_dir.path()).unwrap();
        let error = builtins["vestra.demo"].manifest.as_ref().unwrap_err();
        assert!(error.contains("must be a directory"), "{error}");
    }

    #[test]
    fn builtins_are_never_disabled_nor_uninstalled_and_third_party_plugins_are() {
        let root = tempfile::tempdir().unwrap();
        let mut store = Store::open(root.path().into()).unwrap();
        store
            .sync_builtins(&["vestra.audio".to_owned()].into())
            .unwrap();
        let builtin = store.get("vestra.audio");
        let error = uninstallable("vestra.audio", &builtin, true, Some(builtin.revision));
        assert_eq!(error.unwrap_err().code, "unsupported");
        // User ruling 2026-10-26: not disabled either.
        assert!(store.set_disabled("vestra.audio", true).is_err());
        // A built-in an earlier player let the user disable runs again; the
        // catalog is written once.
        drop(store);
        let path = dunce::canonicalize(root.path())
            .unwrap()
            .join("catalog.json");
        let mut catalog: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        catalog["disabled"] = json!(["vestra.audio"]);
        std::fs::write(&path, serde_json::to_vec(&catalog).unwrap()).unwrap();
        let store = Store::open(root.path().into()).unwrap();
        assert!(!store.disabled().contains("vestra.audio"));
        drop(store);
        let bytes = std::fs::read(&path).unwrap();
        let mut store = Store::open(root.path().into()).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "written once");
        // A third-party plugin can be disabled.
        let other = install(&mut store, "c.d", "1.0.0", Source::File, false).unwrap();
        assert!(other.instance.is_some_and(|i| !i.builtin));
        store.set_disabled("c.d", true).unwrap();
        assert!(store.disabled().contains("c.d"));
        // A third-party plugin: refused only with a stale revision.
        let installed = install(&mut store, "a.b", "1.0.0", Source::File, false).unwrap();
        assert_eq!(
            uninstallable("a.b", &installed, true, Some(installed.revision + 1))
                .unwrap_err()
                .code,
            "revisionConflict"
        );
        assert!(uninstallable("a.b", &installed, true, Some(installed.revision)).is_ok());
        let none = Installed::default();
        assert_eq!(
            uninstallable("x", &none, false, Some(0)).unwrap_err().code,
            "missing"
        );
        assert_eq!(
            uninstallable("x", &none, true, Some(0)).unwrap_err().code,
            "unsupported"
        );
    }

    #[test]
    fn third_party_uninstall_keeps_or_drops_its_data_and_its_files_go_off_thread() {
        let root = tempfile::tempdir().unwrap();
        let mut store = Store::open(root.path().into()).unwrap();
        let first = install(&mut store, "k", "1.0.0", Source::File, false).unwrap();
        let guid = first.instance.unwrap().guid;
        let package = first.current.unwrap();
        // Keeping data: the instance goes dormant, the package is garbage.
        store.uninstall("k", "rm", json!({}), true).unwrap();
        assert!(store.dormant().contains_key(&guid));
        assert!(package.directory.is_dir(), "not removed on this thread");
        let running = cleanup::spawn(
            store.library(),
            cleanup::Work::Only([package.digest.clone()].into()),
            None,
        )
        .unwrap();
        assert!(running.blocks(Some(&package.digest)));
        assert!(!running.blocks(Some(&"0".repeat(64))) && !running.blocks(None));
        assert_eq!(running.join().name(), Some(cleanup::THREAD));
        assert!(!package.directory.exists());
        // Reinstalling reconnects; uninstalling with the data forgets it.
        let again = install(&mut store, "k", "1.0.0", Source::File, false).unwrap();
        assert_eq!(again.instance.unwrap().guid, guid);
        store.uninstall("k", "rm2", json!({}), false).unwrap();
        assert!(store.dormant().is_empty());
        let running = cleanup::spawn(
            store.library(),
            cleanup::Work::Unreferenced(store.referenced()),
            None,
        )
        .unwrap();
        assert!(
            running.blocks(None),
            "the start-up scan blocks every change"
        );
        running.join();
        assert!(!package.directory.exists());
    }

    /// A guests directory with one running guest (its lock held by this
    /// test) that leases `digests`; the lock is held while the value lives.
    fn running_guest(digests: &[&str]) -> (tempfile::TempDir, std::fs::File) {
        let guests = tempfile::tempdir().unwrap();
        let directory = guests.path().join("guest");
        std::fs::create_dir(&directory).unwrap();
        let lock = std::fs::File::create(directory.join("guest.lock")).unwrap();
        lock.try_lock().unwrap();
        std::fs::write(
            directory.join("packages.json"),
            serde_json::to_vec(&digests).unwrap(),
        )
        .unwrap();
        (guests, lock)
    }

    /// Guest windows (guest-windows.md §4): the cleanup thread leaves the
    /// versions a running guest leases, for a removal of named versions
    /// (`Work::Only`, after an update or uninstall) and for the start-up
    /// collection (`Work::Unreferenced`); once that guest is gone they go.
    /// A running guest whose lease cannot be read keeps everything.
    #[test]
    fn the_cleanup_keeps_the_versions_a_running_guest_leases() {
        let root = tempfile::tempdir().unwrap();
        let mut store = Store::open(root.path().into()).unwrap();
        let kept = install(&mut store, "k", "1.0.0", Source::File, false)
            .unwrap()
            .current
            .unwrap();
        let gone = install(&mut store, "j", "1.0.0", Source::File, false)
            .unwrap()
            .current
            .unwrap();
        store.uninstall("k", "rm-k", json!({}), false).unwrap();
        store.uninstall("j", "rm-j", json!({}), false).unwrap();
        let (guests, lock) = running_guest(&[&kept.digest]);
        let only = [kept.digest.clone(), gone.digest.clone()].into();
        cleanup::spawn(
            store.library(),
            cleanup::Work::Only(only),
            Some(guests.path().into()),
        )
        .unwrap()
        .join();
        assert!(kept.directory.is_dir(), "leased: kept");
        assert!(!gone.directory.exists(), "not leased: removed");
        cleanup::spawn(
            store.library(),
            cleanup::Work::Unreferenced(store.referenced()),
            Some(guests.path().into()),
        )
        .unwrap()
        .join();
        assert!(
            kept.directory.is_dir(),
            "the start-up collection keeps it too"
        );
        // A lease that cannot be read: nothing goes this time.
        std::fs::write(guests.path().join("guest/packages.json"), b"broken").unwrap();
        drop(lock);
        let lock = std::fs::OpenOptions::new()
            .write(true)
            .open(guests.path().join("guest/guest.lock"))
            .unwrap();
        lock.try_lock().unwrap();
        cleanup::spawn(
            store.library(),
            cleanup::Work::Unreferenced(store.referenced()),
            Some(guests.path().into()),
        )
        .unwrap()
        .join();
        assert!(kept.directory.is_dir(), "fail-safe");
        // The guest is gone: the next collection removes it.
        drop(lock);
        cleanup::spawn(
            store.library(),
            cleanup::Work::Unreferenced(store.referenced()),
            Some(guests.path().into()),
        )
        .unwrap()
        .join();
        assert!(!kept.directory.exists());
    }

    /// The library of a player up to 0.5: each built-in seeded (current,
    /// previous, `seededVersion`), a third-party plugin upgraded once
    /// (previous kept) and an interrupted seed. Opening it with this player
    /// drops the packages from built-in records and every previous version
    /// in one catalog write, keeps instances and data, clears a built-in's
    /// disabled flag (built-ins cannot be disabled), and leaves the files to
    /// the cleanup thread.
    #[test]
    fn a_library_from_0_5_migrates_to_builtins_in_place_without_previous_versions() {
        let root = tempfile::tempdir().unwrap();
        let mut store = Store::open(root.path().into()).unwrap();
        let old = install(&mut store, "vestra.audio", "1.0.0", Source::File, false).unwrap();
        let new = install(&mut store, "vestra.audio", "1.1.0", Source::File, false).unwrap();
        let youtube_old =
            install(&mut store, "zysilm.youtube", "1.0.0", Source::File, false).unwrap();
        let youtube = install(&mut store, "zysilm.youtube", "1.1.0", Source::File, false).unwrap();
        let fresh_seed = {
            let dir = tempfile::tempdir().unwrap();
            store
                .stage(&candidate(dir.path(), "vestra.info", "1.0.0"))
                .unwrap()
        };
        store
            .prepare_with_receipt(
                "vestra.info",
                0,
                &fresh_seed,
                "seed-info",
                json!({"status":"preparing"}),
                Some(Instance {
                    builtin: true,
                    ..Instance::new(Source::Bundled)
                }),
            )
            .unwrap();
        store.set_disabled("vestra.audio", true).unwrap();
        drop(store);
        // Make it what 0.5 wrote: a seeded built-in with a previous version
        // (the `install` helper drops `previous`, as this player does).
        let path = dunce::canonicalize(root.path())
            .unwrap()
            .join("catalog.json");
        let mut catalog: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let audio = &mut catalog["installed"]["vestra.audio"];
        audio["previous"] = serde_json::to_value(old.current.as_ref().unwrap()).unwrap();
        audio["instance"]["builtin"] = true.into();
        audio["instance"]["source"] = json!({"kind":"bundled"});
        audio["instance"]["seededVersion"] = "1.1.0".into();
        catalog["installed"]["zysilm.youtube"]["previous"] =
            serde_json::to_value(youtube_old.current.as_ref().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_vec(&catalog).unwrap()).unwrap();
        let guid = new.instance.unwrap().guid;

        let store = Store::open(root.path().into()).unwrap();
        let audio = store.get("vestra.audio");
        let instance = audio.instance.clone().unwrap();
        assert!(audio.current.is_none() && audio.previous.is_none());
        assert_eq!(instance.guid, guid, "data and secrets stay with the GUID");
        assert!(instance.builtin && instance.seeded_version.is_none());
        // Built-ins cannot be disabled (user ruling 2026-10-26): it runs again.
        assert!(!store.disabled().contains("vestra.audio"));
        let youtube_now = store.get("zysilm.youtube");
        assert!(youtube_now.previous.is_none());
        assert_eq!(
            youtube_now.current.as_ref().unwrap().digest,
            youtube.current.as_ref().unwrap().digest
        );
        assert!(
            store.pending().is_empty(),
            "the interrupted seed is dropped"
        );
        assert_eq!(store.operations()["seed-info"]["status"], "cancelled");
        // The catalog still has only the fields 0.5 knows (it refuses unknown
        // ones), `previous` included, so an older player still opens it.
        let written: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let keys = |value: &Value| {
            let mut keys = value
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>();
            keys.sort();
            keys
        };
        assert_eq!(
            keys(&written["installed"]["vestra.audio"]),
            ["current", "instance", "previous", "revision"]
        );
        assert_eq!(
            keys(&written["installed"]["vestra.audio"]["instance"]),
            ["builtin", "guid", "installedAt", "source"]
        );
        assert!(written["installed"]["zysilm.youtube"]["previous"].is_null());
        // Idempotent: opening again writes nothing.
        let bytes = std::fs::read(&path).unwrap();
        drop(store);
        let mut store = Store::open(root.path().into()).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        // The built-in copies, previous versions and the seed's staging go;
        // the third-party plugin's current version stays.
        store
            .sync_builtins(&["vestra.audio".to_owned(), "vestra.info".to_owned()].into())
            .unwrap();
        let removed = store.library().collect(&store.referenced()).unwrap();
        assert_eq!(removed, 4);
        assert!(!old.current.unwrap().directory.exists());
        assert!(!new.current.unwrap().directory.exists());
        assert!(!youtube_old.current.unwrap().directory.exists());
        assert!(!fresh_seed.directory.exists());
        assert!(youtube.current.unwrap().directory.is_dir());
        let left = std::fs::read_dir(root.path().join("packages"))
            .unwrap()
            .count();
        assert_eq!(left, 1);
    }

    #[test]
    fn builtin_records_follow_builtin_json() {
        let root = tempfile::tempdir().unwrap();
        let mut store = Store::open(root.path().into()).unwrap();
        let ids = |list: &[&str]| {
            list.iter()
                .map(|id| id.to_string())
                .collect::<BTreeSet<_>>()
        };
        // A fresh profile: one record per built-in, no package, in one write.
        store
            .sync_builtins(&ids(&["vestra.a", "vestra.b"]))
            .unwrap();
        let a = store.get("vestra.a").instance.unwrap();
        assert!(a.builtin && a.source == Source::Bundled);
        assert!(store.get("vestra.a").current.is_none());
        assert!(
            std::fs::read_dir(root.path().join("packages"))
                .unwrap()
                .next()
                .is_none()
        );
        let path = dunce::canonicalize(root.path())
            .unwrap()
            .join("catalog.json");
        let bytes = std::fs::read(&path).unwrap();
        store
            .sync_builtins(&ids(&["vestra.a", "vestra.b"]))
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "nothing to change");
        // A built-in the installation dropped is retired with its data...
        store.sync_builtins(&ids(&["vestra.b"])).unwrap();
        assert!(store.get("vestra.a").instance.is_none());
        assert_eq!(store.dormant()[&a.guid].id, "vestra.a");
        assert!(!store.disabled().contains("vestra.a"));
        // ...and reconnects to it when it comes back.
        store
            .sync_builtins(&ids(&["vestra.a", "vestra.b"]))
            .unwrap();
        assert_eq!(store.get("vestra.a").instance.unwrap().guid, a.guid);
        assert!(store.dormant().is_empty());
        // A reserved id installed from a file before the rule is left alone.
        let legacy_root = tempfile::tempdir().unwrap();
        let mut legacy = Store::open(legacy_root.path().into()).unwrap();
        let file = install(&mut legacy, "vestra.c", "1.0.0", Source::File, false).unwrap();
        legacy.sync_builtins(&ids(&["vestra.c"])).unwrap();
        assert_eq!(legacy.get("vestra.c").instance, file.instance);
        assert!(check_reserved("vestra.c", file.instance.as_ref()).is_err());
    }

    /// What compiling Slint costs on the UI thread (not run by default):
    /// `VESTRA_SLINT_BENCH` names a `resources/plugins` directory. Prints
    /// the main window's shell (compiled once a run; every slot a container,
    /// `ui/panels.rs`) and each built-in's .slint files compiled on their
    /// own.
    #[test]
    #[ignore]
    fn slint_compile_bench() {
        let Some(dir) = std::env::var_os("VESTRA_SLINT_BENCH") else {
            return;
        };
        for round in 0..3 {
            let lap = std::time::Instant::now();
            composition::build_shell(composition::shell_source().unwrap(), "HostWindow").unwrap();
            diag!("shell (round {round}): {} ms", lap.elapsed().as_millis());
        }
        let dir = Path::new(&dir);
        let mut total = 0;
        for entry in builtin::read(dir).unwrap() {
            let mut files = Vec::new();
            let mut stack = vec![dir.join(&entry.directory)];
            while let Some(next) = stack.pop() {
                for item in std::fs::read_dir(next).unwrap() {
                    let path = item.unwrap().path();
                    if path.is_dir() {
                        stack.push(path);
                    } else if path.extension().is_some_and(|x| x == "slint") {
                        files.push(path);
                    }
                }
            }
            let lap = std::time::Instant::now();
            for file in &files {
                let _ = pollster::block_on(crate::ui::view::compiler().build_from_path(file));
            }
            let took = lap.elapsed().as_millis();
            total += took;
            diag!("slint {}: {} files, {took} ms", entry.id, files.len());
        }
        diag!("slint all built-ins: {total} ms");
    }

    /// Characterization test (supervisor.md §6): start-up and wait tunables
    /// as they were pinned before the supervisor (the 15 s activation
    /// deadline and the install wait of 20 s were literals in
    /// `load_package` and `advance_package`).
    #[test]
    fn b0_start_up_and_wait_tunables() {
        assert_eq!(START_BUDGET, std::time::Duration::from_millis(25));
        assert_eq!(VERIFY_WAIT, std::time::Duration::from_secs(3));
        // The reload's and the install's wait (formerly `RELOAD_WAIT_MS` and
        // a literal): the supervisor's `STOP_WAIT`.
        assert_eq!(
            crate::supervisor::Tunables::new(8, 16_384).stop_wait,
            20_000
        );
    }

    /// Characterization test: the plugins behind a held built-in start while
    /// it keeps its place: the supervisor's lanes (`supervisor/queue.rs`
    /// `b0_others_pass_a_builtin_being_checked`); the start order itself is
    /// `start_order` (`start_order_puts_builtins_first`).
    #[test]
    fn b0_start_order_is_builtins_first_then_id_order() {
        let builtin = |id: &str| id.starts_with("vestra.");
        let ids = ["a.one", "vestra.slow", "vestra.fast"].map(String::from);
        let order = start_order(ids, builtin);
        assert_eq!(order, ["vestra.slow", "vestra.fast", "a.one"]);
    }
}
