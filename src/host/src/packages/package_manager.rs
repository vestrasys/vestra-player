//! Finite trusted-package lifecycle. This is not a sandbox or dependency solver.
#![forbid(unsafe_code)]
use crate::host::*;
use crate::packages::package_stage::Candidate;
use crate::packages::package_store::{Instance, Source, Store, StoredPackage};
use crate::packages::{package_stage, package_store};
use crate::playback::backend::Command;
use crate::process::{Plugin, StopReason, shared_runner};
use crate::{i18n, platform, process};
use anyhow::Result;
use process::DataKey;
use serde_json::{Value, json};
use std::collections::{BTreeSet, VecDeque};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// Validated candidates kept at once (each is a staged copy on disk).
const MAX_CANDIDATES: usize = 32;
/// At start-up, how long one tick may spend starting installed plugins
/// (`start_queued`).
const START_BUDGET: std::time::Duration = std::time::Duration::from_millis(25);
pub struct Manager {
    store: Store,
    /// Secret names per plugin service (`secrets.*`), for deleting them.
    pub secrets: crate::packages::secret_index::Index,
    /// Validated packages by token, at most `MAX_CANDIDATES`; `candidate_order`
    /// is oldest first, and a new validation drops the oldest token (`stale`).
    candidates: BTreeMap<String, Candidate>,
    candidate_order: VecDeque<String>,
    operations: BTreeMap<String, Operation>,
    next: u64,
    receipts: BTreeMap<String, Value>,
    booted: bool,
    /// Installed plugins still to start at start-up (`start_queued`).
    to_start: StartQueue,
    /// The installation's built-in plugins (`builtin.json`) by id; empty
    /// when it has none (a development run).
    builtins: BTreeMap<String, builtin::Builtin>,
    /// The `library-cleanup` thread, while it removes package directories.
    cleanup: Option<cleanup::Running>,
    /// Package directories an operation freed while the thread ran; they
    /// go on its next run.
    cleanup_queue: BTreeSet<String>,
    /// `packages.reload`: plugins stopped to start again, by id: the stopped
    /// process and when; started once that process is gone (`reload_tick`).
    reloads: BTreeMap<String, (u64, u64)>,
    /// Built-in digest checks for the shared runner (`verify`), by id.
    verified: BTreeMap<String, Verified>,
    verify_serial: u64,
    verify_results: (
        std::sync::mpsc::Sender<VerifyResult>,
        std::sync::mpsc::Receiver<VerifyResult>,
    ),
    /// `packages.validate*` on their threads (package_validate.rs).
    validations: crate::packages::package_validate::Validations,
}
/// Installed plugins still to start, in start order, and how many of them
/// are built-ins (`builtin_first_run_pending` asks once per plugin started).
#[derive(Default)]
struct StartQueue {
    ids: VecDeque<(String, bool)>,
    builtins: usize,
}
impl StartQueue {
    /// Built-ins first, then the others, each in id order: the built-ins
    /// (playback, video, subtitles) do not wait behind third-party plugins.
    fn new(ids: impl IntoIterator<Item = String>, builtin: impl Fn(&str) -> bool) -> Self {
        let (builtins, others): (Vec<_>, Vec<_>) = ids
            .into_iter()
            .map(|id| {
                let is_builtin = builtin(&id);
                (id, is_builtin)
            })
            .partition(|(_, is_builtin)| *is_builtin);
        Self {
            builtins: builtins.len(),
            ids: builtins.into_iter().chain(others).collect(),
        }
    }
    /// Queues `id` again (a shared runner restart): a built-in after the
    /// queued built-ins, before every other plugin, as `new` orders them;
    /// another plugin at the end.
    fn push(&mut self, id: String, builtin: bool) {
        if builtin {
            self.ids.insert(self.builtins, (id, builtin));
            self.builtins += 1;
        } else {
            self.ids.push_back((id, builtin));
        }
    }
    /// The id at `at` (0: the next to start).
    fn get(&self, at: usize) -> Option<&str> {
        self.ids.get(at).map(|(id, _)| id.as_str())
    }
    /// Takes the id at `at` out of the queue.
    fn remove(&mut self, at: usize) -> Option<String> {
        let (id, builtin) = self.ids.remove(at)?;
        self.builtins -= usize::from(builtin);
        Some(id)
    }
    fn contains(&self, id: &str) -> bool {
        self.ids.iter().any(|(queued, _)| queued == id)
    }
    fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
    fn len(&self) -> usize {
        self.ids.len()
    }
    /// A built-in is still to start.
    fn builtin_pending(&self) -> bool {
        self.builtins > 0
    }
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
/// Where `load_package` runs a plugin (`Host::trust`).
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
/// How long a reload waits for the stopped process to go, as installs do.
const RELOAD_WAIT_MS: u64 = 20_000;
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
    waiting: Option<u64>,
    cancel: bool,
    failure: Option<String>,
    started: u64,
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
    pub fn open() -> Result<Self> {
        // Staged copies an earlier run left behind (it quit or crashed
        // while validating).
        crate::packages::package_validate::sweep_stale();
        let root = std::env::var_os("VESTRA_PACKAGE_STORE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                crate::platform::app_data_dir()
                    .unwrap_or_default()
                    .join("packages")
            });
        let secrets = crate::packages::secret_index::Index::new(Some(root.join("secrets")));
        let mut store = Store::open(root)?;
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
            operations: BTreeMap::new(),
            next: 0,
            receipts,
            booted: false,
            to_start: StartQueue::default(),
            builtins,
            cleanup: None,
            cleanup_queue: BTreeSet::new(),
            reloads: BTreeMap::new(),
            verified: BTreeMap::new(),
            verify_serial: 0,
            verify_results: std::sync::mpsc::channel(),
            validations: Default::default(),
        })
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
    /// Take the finished digest checks.
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
impl Host {
    /// Where plugin `id`, about to run from `directory`, runs (doc/process/
    /// rounds/2026-10-shared-runner.md §3): in the shared runner only a
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
    /// The shared runner at a host tick: the heartbeat (a hang restarts
    /// every member), and an idle runner let go.
    fn shared_tick(&mut self) {
        let now = self.now_ms();
        let Some(shared) = self.shared.as_mut() else {
            return;
        };
        if shared.idle(now) {
            if let Some(runner) = self.shared.take() {
                diag!("shared runner {}: no plugin left; stopped", runner.link.id);
                runner.retire(true);
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
        runner.retire(false);
        let failed_open = self.shared_policy.restart(self.now_ms());
        let members = self
            .plugins
            .iter()
            .filter(|(_, plugin)| plugin.shared_runner() == Some(id))
            .map(|(owner, plugin)| (*owner, plugin.manifest.id.clone()))
            .collect::<Vec<_>>();
        // The runner is killed already: no member gets its `runtime.stop`
        // (their cleanups do not run; nothing is left to tell).
        for (owner, _) in &members {
            self.disable(*owner, StopReason::Failed);
        }
        for (_, plugin) in members {
            if !failed_open {
                self.packages.verify(&plugin);
            }
            if !self.packages.to_start.contains(&plugin) {
                let builtin = self.packages.builtins.contains_key(&plugin);
                self.packages.to_start.push(plugin, builtin);
            }
        }
        let text = if failed_open {
            diag!(
                "shared runner: {} restarts within {} s; every plugin runs in its own process until the player quits",
                shared_runner::RESTART_LIMIT,
                shared_runner::RESTART_WINDOW_MS / 1000
            );
            i18n::tr("Built-in plugins run separately for this session after repeated faults")
        } else {
            i18n::tr("Built-in plugins restarted after a fault")
        };
        self.show_osd(&text);
    }
    fn cleanup_done(&self, owner: u64) -> bool {
        self.cleanup.get(&owner) == Some(&(true, true))
    }
    fn load_package(
        &mut self,
        directory: &Path,
        manifest: &package_stage::PackageManifest,
        key: DataKey,
    ) -> Result<u64> {
        self.next_generation += 1;
        let owner = self.next_generation;
        let id = manifest.id.clone();
        anyhow::ensure!(self.running(&id).is_none(), "plugin already running");
        self.activations.begin(owner, self.now_ms() + 15_000)?;
        let manifest = process::Manifest {
            id: id.clone(),
            ..manifest.clone()
        };
        let plugin = match self.trust(&id, directory) {
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
                Plugin::load_manifest(
                    directory,
                    owner,
                    self.events.clone(),
                    manifest,
                    key,
                    process::Placement::Own,
                )?
            }
        };
        let name = plugin.manifest.name.clone();
        self.add_plugin(owner, plugin);
        self.backend.commands.send(Command {
            reply: Reply { owner, id: 0 },
            method: "owner.registered".into(),
            params: json!({"manifestId":id,"manifestName":name}),
        })?;
        Ok(owner)
    }
    fn package_status(&mut self, owner: u64) -> String {
        self.activation(owner)
            .ok()
            .and_then(|v| v["activation"]["status"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "unknown".into())
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
                let (instance, fresh) = self.install_instance(&id, &before, source, replace)?;
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
            lazy::remove(&self.activation_dir(), &instance.guid);
        }
        self.packages.store.prepare_with_receipt(
            &id,
            before.revision,
            &target,
            &request,
            result.clone(),
            instance,
        )?;
        // A parked version goes like a running one (an upgrade has a new
        // digest: its recording is of no use).
        self.forget_parked(&id, true);
        let running = self.running(&id);
        // `upgrade` for every install over a running version that runs on:
        // an update, a reinstall of the same version, and a `replace` from
        // another source (`fresh` data) alike. An install that leaves it
        // off (`enabled: false`; a running package is never disabled, so
        // only the installer's option does this) switches the plugin off:
        // `disable`, as `packages.setEnabled(false)` (its session values go
        // too, at the commit).
        if let Some(owner) = running {
            let reason = if enabled {
                StopReason::Upgrade
            } else {
                StopReason::Disable
            };
            self.disable(owner, reason);
        }
        // Parked just now: its process may still be finishing; the new
        // version starts once it is gone, as after a running one.
        let waiting = running.or_else(|| self.parked_process(&id));
        let fresh_data = fresh
            .then(|| process::data_root().map(|root| root.join(&target_guid)))
            .flatten();
        self.packages.operations.insert(
            request.clone(),
            Operation {
                id: request,
                package: id,
                target,
                old: before.current,
                target_key: DataKey::Instance(target_guid),
                old_key,
                fresh_data,
                enabled,
                phase: "preparing".into(),
                owner: None,
                waiting,
                cancel: false,
                failure: None,
                started: self.now_ms(),
                result: result.clone(),
                request: request_spec,
            },
        );
        Ok(result)
    }
    /// The data directory of an installed instance, development load or
    /// running plugin (`PluginInfo.dataDirectory`).
    fn data_directory_of(&self, key: &DataKey) -> Option<PathBuf> {
        process::data_root().map(|root| key.directory(&root))
    }
    /// Start the installed packages (only `only`, when given) that are not
    /// disabled, not replaced by a development load and not in an
    /// operation. Built-ins run in place from the installation
    /// (`runs_from`); nothing is copied.
    pub(crate) fn start_installed(&mut self, only: Option<&str>) {
        // One record (the start-up queue starts them one by one), not a
        // copy of the whole catalog per plugin.
        let ids = match only {
            Some(only) => self
                .packages
                .store
                .record(only)
                .map(|_| only.to_owned())
                .into_iter()
                .collect(),
            None => self.packages.store.installed_ids(),
        };
        for id in ids {
            if self.running(&id).is_some() || self.lazy.parked_id(&id).is_some() {
                continue;
            }
            let Some(record) = self.packages.store.record(&id).cloned() else {
                continue;
            };
            // R-H10: a package the user disabled stays stopped; a
            // development load of the same id runs instead this run.
            if self.packages.operations.values().any(|o| o.package == id)
                || self.packages.store.disabled().contains(&id)
                || self.development.contains_key(&id)
            {
                continue;
            }
            // Its library copy could not be read at start: kept as it is,
            // not started (`runs_from`); the next start tries again.
            if let Some(error) = self.packages.store.deferred().get(&id) {
                self.failures.insert(id.clone(), deferred_failure(error));
                continue;
            }
            let (Some(package), Some(instance)) =
                (self.packages.runs_from(&id, &record), record.instance)
            else {
                continue;
            };
            // Lazy activation: a valid recording of a plugin that was lazy
            // at its ready is replayed instead; the plugin starts on its
            // first use (host/lazy.rs). One that was resident starts now.
            // Without a recording (its first run) it waits for a place.
            let mut first_run = false;
            if self.lazy.enabled
                && let Ok((directory, manifest)) = &package
                && check_reserved(&id, Some(&instance)).is_ok()
                && let Some(key) = self.lazy_key(&id)
            {
                match lazy::read(&self.packages.store.activation_dir(), &key) {
                    Some(recording) if recording.resident_at_ready => {
                        diag!("lazy: {id} starts: resident at its last ready");
                    }
                    Some(recording) => {
                        match self.lazy_replay(&id, directory, manifest, &key, recording) {
                            Ok(()) => continue,
                            Err(error) => diag!("lazy: {id} replay failed, starting it: {error}"),
                        }
                    }
                    // Built-ins first: another plugin's first run waits
                    // while a built-in is still to start or queued, since
                    // what it claims at activation waits for them.
                    None if !self.lazy.first_run_free()
                        || !self.packages.builtins.contains_key(&id)
                            && self.builtin_first_run_pending() =>
                    {
                        let builtins = &self.packages.builtins;
                        self.lazy
                            .queue_first_run(id.clone(), |id| builtins.contains_key(id));
                        continue;
                    }
                    None => first_run = true,
                }
            }
            let started = package
                .map_err(anyhow::Error::msg)
                .and_then(|(directory, manifest)| {
                    check_reserved(&id, Some(&instance))
                        .map_err(|refusal| anyhow::anyhow!(refusal.message))?;
                    self.load_package(&directory, &manifest, DataKey::Instance(instance.guid))
                });
            match started {
                Ok(owner) if first_run => {
                    diag!("lazy: {id} starts for its first run");
                    self.lazy.first_run(owner);
                }
                Ok(_) => (),
                Err(error) => {
                    diag!("installed package {id} failed to start: {error}");
                    self.failures
                        .insert(id.clone(), error.to_string().chars().take(512).collect());
                }
            }
        }
    }
    /// Lazy activation: start the parked installed plugin `id` (its first
    /// use woke it), as `start_installed` would.
    pub(crate) fn start_package(&mut self, id: &str) -> Result<u64> {
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
        self.load_package(&directory, &manifest, DataKey::Instance(instance.guid))
    }
    /// What a recording of installed plugin `id` is valid for: its instance,
    /// its package digest (a library copy's, or the one `builtin.json`
    /// lists) and version. `None`: it is not recorded (lazy activation
    /// leaves it running), e.g. a built-in listed without a digest.
    pub(crate) fn lazy_key(&self, id: &str) -> Option<lazy::Key> {
        let record = self.packages.store.get(id);
        let instance = record.instance.as_ref()?;
        let (digest, version) = if instance.builtin {
            let builtin = self.packages.builtins.get(id)?;
            (builtin.digest.clone()?, Some(builtin.version.clone()))
        } else {
            let current = record.current.as_ref()?;
            (current.digest.clone(), current.manifest.version.clone())
        };
        Some(lazy::Key {
            guid: instance.guid.clone(),
            digest,
            version,
        })
    }
    /// Removes installed plugin `id`'s recording, if it has one.
    pub(crate) fn drop_recording(&self, id: &str) {
        if let Some(instance) = self.packages.store.get(id).instance {
            lazy::remove(&self.activation_dir(), &instance.guid);
        }
    }
    /// The directory recordings are kept in.
    pub(crate) fn activation_dir(&self) -> PathBuf {
        self.packages.store.activation_dir()
    }
    /// An installed (not development) plugin `owner` runs as installed:
    /// its recording key.
    pub(crate) fn lazy_key_of(&self, owner: u64) -> Option<lazy::Key> {
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
        self.lazy_key(id).filter(|key| runs && key.guid == *guid)
    }
    /// An operation or a reload runs on `id`.
    pub(crate) fn package_busy(&self, id: &str) -> bool {
        self.operation_pending(id) || self.packages.reloads.contains_key(id)
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
        let library = self.packages.store.library();
        let referenced = self.packages.store.referenced();
        match cleanup::spawn(library, cleanup::Work::Unreferenced(referenced)) {
            Ok(running) => self.packages.cleanup = Some(running),
            // Left for the next run; the directories are harmless.
            Err(error) => diag!("package library: {error}"),
        }
    }
    /// An operation freed `digest` (the version an upgrade replaced, an
    /// uninstalled package, a failed install's target): remove it on the
    /// cleanup thread, now or after its current run.
    fn discard(&mut self, digest: &str) {
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
        if digests.is_empty() {
            return;
        }
        match cleanup::spawn(self.packages.store.library(), cleanup::Work::Only(digests)) {
            Ok(running) => self.packages.cleanup = Some(running),
            Err(error) => diag!("package library: {error}"),
        }
    }
    /// A running built-in plugin: shipped with the player and running from
    /// the installation (not a development load of the same id).
    pub(crate) fn is_builtin(&self, owner: u64) -> bool {
        let Some(plugin) = self.plugins.get(&owner) else {
            return false;
        };
        let id = &plugin.manifest.id;
        self.packages
            .builtins
            .get(id)
            .is_some_and(|builtin| builtin.directory == plugin.directory)
            && self
                .packages
                .store
                .get(id)
                .instance
                .is_some_and(|instance| instance.builtin)
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
                self.activations.status(owner, now),
                Ok(package_activation::Status::Pending { .. })
            )
        })
    }
    /// A built-in is still to start at start-up (`to_start`) or waits for
    /// its first run: other plugins' first runs wait for it.
    fn builtin_first_run_pending(&self) -> bool {
        self.packages.to_start.builtin_pending() || self.builtin_first_run_queued()
    }
    /// A built-in waits for its first run (lazy activation, §2.4).
    pub(crate) fn builtin_first_run_queued(&self) -> bool {
        self.lazy.builtin_queued()
    }
    /// A built-in still activating that may mount a panel (it declares
    /// `ui.mount`) and has not asked yet: at start-up the shell waits a
    /// little for it, so that the built-ins' panels come in one compile
    /// (`prepare_mount_tick`).
    pub(crate) fn builtin_mount_expected(&mut self) -> bool {
        if !self.packages_started() {
            return true;
        }
        let now = self.now_ms();
        let owners = self
            .plugins
            .iter()
            .filter(|(owner, plugin)| {
                plugin.allows("ui.mount")
                    && !self.mounts.contains_key(owner)
                    && !self.mount_queue.iter().any(|c| c.reply.owner == **owner)
                    && !self
                        .preparing
                        .as_ref()
                        .is_some_and(|(_, c)| c.reply.owner == **owner)
            })
            .map(|(owner, _)| *owner)
            .collect::<Vec<_>>();
        owners.into_iter().any(|owner| {
            self.is_builtin(owner)
                && matches!(
                    self.activations.status(owner, now),
                    Ok(package_activation::Status::Pending { .. })
                )
        })
    }
    /// The running owner of plugin `id` (`Host::plugin_ids`).
    pub(crate) fn running(&self, id: &str) -> Option<u64> {
        let owner = self.plugin_ids.get(id).copied();
        // Tests only: a scan per lookup made a debug start-up quadratic.
        #[cfg(test)]
        assert_eq!(
            owner,
            self.plugins
                .iter()
                .find(|(_, plugin)| plugin.manifest.id == id)
                .map(|(owner, _)| *owner),
            "plugin_ids follows plugins"
        );
        owner
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
    /// `packages.list`: running plugins, installed packages and this run's
    /// development loads, by id.
    pub(crate) fn package_list(&mut self) -> Value {
        let now = self.now_ms();
        let mut list = BTreeMap::<String, Value>::new();
        let owners = self.plugins.keys().copied().collect::<Vec<_>>();
        for owner in owners {
            let status = self.activations.status(owner, now).ok();
            let plugin = &self.plugins[&owner];
            let id = plugin.manifest.id.clone();
            let stored = self.packages.store.get(&id);
            let installed = self
                .packages
                .runs_from(&id, &stored)
                .is_some_and(|package| package.is_ok_and(|(dir, _)| dir == plugin.directory));
            let warnings = self.warnings_of(owner);
            let (state, error) = match status {
                // Woken from idle (lazy activation): it was running all along.
                Some(package_activation::Status::Pending { .. })
                    if self.lazy.waking.contains_key(&owner) =>
                {
                    ("running", None)
                }
                Some(package_activation::Status::Pending { .. }) => ("starting", None),
                Some(package_activation::Status::Failed { message }) => ("failed", Some(message)),
                Some(package_activation::Status::TimedOut) => {
                    ("failed", Some("activation timed out".to_owned()))
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
            let instance = installed.then_some(stored.instance).flatten();
            // `process` (SDK 1.5.38): it has a process (`idle` below).
            let process = (state == "running").then_some("running");
            list.insert(
                id.clone(),
                json!({"id":id,"version":plugin.manifest.version,
                "origin":origin,"state":state,"error":error,"process":process,
                "capabilities":plugin.manifest.capabilities,
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
        for (id, record) in self.packages.store.installed() {
            if list.contains_key(&id) {
                continue;
            }
            let Some(package) = self.packages.runs_from(&id, &record) else {
                continue;
            };
            // Waiting for its first run (lazy activation) counts as starting.
            let disabled = self.packages.store.disabled().contains(&id);
            let parked = self
                .lazy
                .parked_id(&id)
                .and_then(|owner| self.lazy.parked(owner));
            let (state, error) =
                if self.package_busy(&id) || !disabled && self.lazy.first_queue.contains(&id) {
                    ("starting", None)
                } else if let Some(parked) = parked {
                    // Parked (lazy activation): it runs again on its first use;
                    // `state` stays `running`, `process` says `idle`, and what
                    // it was refused while it ran stays listed.
                    ("running", parked.warning.clone())
                } else {
                    stopped(disabled, self.failures.get(&id))
                };
            let warnings = parked.map_or_else(Vec::new, |parked| parked.warnings.clone());
            let data = record
                .instance
                .as_ref()
                .and_then(|i| self.data_directory_of(&DataKey::Instance(i.guid.clone())));
            let builtin = record.instance.as_ref().is_some_and(|i| i.builtin);
            let source = record.instance.map(|i| i.source);
            let process =
                (self.lazy.parked_id(&id).is_some() && state == "running").then_some("idle");
            let entry = match package {
                Ok((directory, manifest)) => json!({"id":id,"version":manifest.version,
                    "origin":"installed","state":state,"error":error,"process":process,
                    "capabilities":manifest.capabilities,"directory":directory,
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
                        "capabilities":[],"directory":directory.unwrap_or_default(),
                        "builtin":builtin,"source":source,
                        "dataDirectory":data,"warnings":[],
                        "display":{"name":null,"punchline":null,"description":null,"tags":[]}})
                }
            };
            list.insert(id.clone(), entry);
        }
        // Installed records the library set aside at start (`Quarantined`):
        // failed, with the reason, until installed again.
        for record in self.packages.store.quarantined() {
            if record.section != "installed" || list.contains_key(&record.id) {
                continue;
            }
            let error = format!(
                "the plugin library's record of this plugin was set aside: {}",
                record.reason
            );
            list.insert(
                record.id.clone(),
                json!({"id":record.id,"version":null,
                    "origin":"installed","state":"failed","error":error,
                    "capabilities":[],"directory":"",
                    "builtin":false,"source":null,
                    "dataDirectory":null,"warnings":[],
                    "display":{"name":null,"punchline":null,"description":null,"tags":[]}}),
            );
        }
        for (id, (directory, manifest)) in &self.development {
            if list.contains_key(id) {
                continue;
            }
            let (state, error) = if self.packages.reloads.contains_key(id) {
                ("starting", None)
            } else {
                stopped(self.dev_disabled.contains(id), self.failures.get(id))
            };
            let data = self.data_directory_of(&DataKey::Development(id.clone()));
            list.insert(
                id.clone(),
                json!({"id":id,"version":manifest.version,
                "origin":"development","state":state,"error":error,
                "capabilities":manifest.capabilities,"directory":directory,
                "builtin":false,"source":null,"dataDirectory":data,"warnings":[],
                "display":display(manifest, crate::i18n::load(directory).as_deref())}),
            );
        }
        Value::Array(list.into_values().collect())
    }
    /// `packages.setEnabled`: disabling stops it (and keeps it stopped across
    /// launches when installed); enabling starts it again.
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
        if self.package_busy(id) {
            return Err(RpcError::new(
                "busy",
                "the package is being installed or reloaded",
            ));
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
            self.dev_disabled.remove(id);
        } else {
            self.dev_disabled.insert(id.into());
        }
        if !enabled {
            if let Some(owner) = running {
                self.disable(owner, StopReason::Disable);
            }
            // Parked: its recorded entries go; the recording stays (not
            // replayed while it is disabled).
            self.forget_parked(id, false);
            self.failures.remove(id);
            // The user switched it off: its session values go (a reload, an
            // upgrade or a crash keeps them; session_storage.rs).
            let stopped = match &development {
                Some(_) => Some(DataKey::Development(id.into())),
                None => key,
            };
            if let Some(stopped) = stopped {
                self.session_storage.clear(&stopped);
            }
            return Ok(());
        }
        if running.is_some() || self.lazy.parked_id(id).is_some() {
            return Ok(());
        }
        // Disabled right after a park: it starts once that process is gone
        // (`reload_tick`), never beside it on the same data.
        if let Some(owner) = self.parked_process(id) {
            let now = self.now_ms();
            self.packages.reloads.insert(id.into(), (owner, now));
            return Ok(());
        }
        self.start_stopped(id, installed, reserved, key, development)
    }
    /// The process installed plugin `id` was parked from (lazy activation),
    /// while it is still finishing: what a start of `id` waits for, as for
    /// a running one it stops.
    fn parked_process(&self, id: &str) -> Option<u64> {
        self.lazy
            .stopping(id)
            .filter(|owner| !self.process_gone(*owner))
    }
    /// Start a stopped plugin: its development load, else its installed
    /// package (`set_enabled`, `reload`). The disabled flag is not touched.
    fn start_stopped(
        &mut self,
        id: &str,
        installed: Option<std::result::Result<(PathBuf, package_stage::PackageManifest), String>>,
        reserved: std::result::Result<(), RpcError>,
        key: Option<DataKey>,
        development: Option<(PathBuf, process::Manifest)>,
    ) -> std::result::Result<(), RpcError> {
        self.failures.remove(id);
        match (installed, development) {
            (_, Some((directory, _))) => self.load(&directory, 120_000)?,
            (Some(package), None) => {
                reserved?;
                // Nothing runnable (a deferred record, a broken built-in):
                // it stays listed as failed, with the same reason.
                let (directory, manifest) = package.map_err(|failure| {
                    self.failures.insert(id.into(), failure.clone());
                    RpcError::new("unavailable", failure)
                })?;
                // Built-in records and installs both have an instance.
                let key = key.ok_or_else(|| {
                    RpcError::new("unavailable", "the installed package has no instance")
                })?;
                self.load_package(&directory, &manifest, key)?;
            }
            (None, None) => return Err(RpcError::new("missing", format!("no plugin {id}"))),
        }
        Ok(())
    }
    /// `packages.reload { id }` (SDK 1.5.19, STORE-RELOAD-BUILTIN): stop the
    /// running plugin and start it again from the same package and data, once
    /// its process is gone (`reload_tick`); a stopped or failed one starts
    /// now. Built-ins included; the disabled flag is never written, so a
    /// reload never leaves a plugin disabled. Same caller rules as
    /// `setEnabled`: `plugins.manage`, never itself.
    fn reload(&mut self, caller: u64, id: &str) -> std::result::Result<(), RpcError> {
        if self.plugins[&caller].manifest.id == id {
            return Err(RpcError::invalid("a plugin cannot reload itself"));
        }
        if self.package_busy(id) {
            return Err(RpcError::new(
                "busy",
                "the package is being installed or reloaded",
            ));
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
        // Parked: it has no process, so it starts now, observed fresh (its
        // recording goes); once the process it was parked from is gone.
        self.forget_parked(id, true);
        let running = self.running(id);
        if let Some(owner) = running {
            self.disable(owner, StopReason::Reload);
        }
        if let Some(owner) = running.or_else(|| self.parked_process(id)) {
            let now = self.now_ms();
            self.packages.reloads.insert(id.into(), (owner, now));
            // A built-in's files are checked again before it shares the
            // runner (`reload_tick` waits for the check).
            if shared_runner::enabled() {
                self.packages.verify(id);
            }
            return Ok(());
        }
        let reserved = check_reserved(id, record.instance.as_ref());
        let key = record.instance.map(|i| DataKey::Instance(i.guid));
        self.start_stopped(id, installed, reserved, key, development)
    }
    /// Start the reloaded plugins whose stopped process is gone; one that
    /// does not go within `RELOAD_WAIT_MS` is listed as failed instead.
    fn reload_tick(&mut self) {
        if self.packages.reloads.is_empty() {
            return;
        }
        let now = self.now_ms();
        let due = self
            .packages
            .reloads
            .iter()
            .filter(|(id, (owner, since))| {
                (self.cleanup_done(*owner) && !self.packages.verifying(id))
                    || now.saturating_sub(*since) >= RELOAD_WAIT_MS
            })
            .map(|(id, (owner, _))| (id.clone(), *owner))
            .collect::<Vec<_>>();
        for (id, owner) in due {
            self.packages.reloads.remove(&id);
            if !self.cleanup_done(owner) {
                self.failures.insert(
                    id,
                    "reload: the previous process did not stop within 20 s".into(),
                );
                continue;
            }
            if self.running(&id).is_some() {
                continue;
            }
            let record = self.packages.store.get(&id);
            let installed = self.packages.runs_from(&id, &record);
            let reserved = check_reserved(&id, record.instance.as_ref());
            let key = record.instance.map(|i| DataKey::Instance(i.guid));
            let development = self.development.get(&id).cloned();
            if let Err(error) = self.start_stopped(&id, installed, reserved, key, development) {
                diag!("packages.reload {id}: {}", error.message);
                self.failures.entry(id).or_insert(error.message);
            }
        }
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
        uninstallable(
            &id,
            &installed,
            self.development.contains_key(&id) || self.running(&id).is_some(),
            params["expectedRevision"].as_u64(),
        )?;
        self.library_busy(None)?;
        if self.package_busy(&id) {
            return Err(RpcError::new(
                "busy",
                "the package is being installed or reloaded",
            ));
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
        self.forget_parked(&id, true);
        // Parked just now: its process may still be writing its data.
        let parked = self
            .parked_process(&id)
            .filter(|owner| self.cleanup.get(owner).is_some_and(|(reaped, _)| !reaped));
        if let Some(owner) = self.running(&id) {
            self.disable(owner, StopReason::Uninstall);
            // Delete the data after the process is gone (it flushes on stop).
            if let Some(directory) = data.clone() {
                self.data_removals.insert(owner, directory);
            }
        } else if let Some(owner) = parked
            && let Some(directory) = data.clone()
        {
            self.data_removals.insert(owner, directory);
        } else if let Some(directory) = &data {
            let _ = std::fs::remove_dir_all(directory);
        }
        self.packages
            .store
            .uninstall(&id, &request, receipt.clone(), !remove_data)?;
        if let Some(current) = &installed.current {
            self.discard(&current.digest);
        }
        self.packages.receipts = self.packages.store.operations();
        self.failures.remove(&id);
        // Session values go with every uninstall, whether or not the files do.
        if let Some(instance) = &installed.instance {
            self.session_storage
                .clear(&DataKey::Instance(instance.guid.clone()));
        }
        if remove_data && let Some(instance) = &installed.instance {
            self.purge_secrets(&DataKey::Instance(instance.guid.clone()));
        }
        self.purge_dropped_secrets();
        Ok(receipt)
    }
    /// Deletes the secret store entries recorded for `key` (on a worker
    /// thread: keychain calls block and may prompt).
    fn purge_secrets(&self, key: &DataKey) {
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
    /// `package_tick`).
    pub(crate) fn packages_started(&self) -> bool {
        self.packages.booted && self.packages.to_start.is_empty()
    }
    /// Start the next installed plugins of the start-up queue, in their
    /// order, until `START_BUDGET` of this tick is spent (at least one).
    /// Starting a plugin spawns its Node process, several milliseconds each
    /// on Windows; all of them in one tick held the UI thread for 150 ms.
    fn start_queued(&mut self) {
        if self.packages.to_start.is_empty() {
            return;
        }
        let began = std::time::Instant::now();
        let mut started = 0;
        // A built-in whose digest is still being checked stays queued, in
        // its place (it starts once the check is done, shared or alone, and
        // other plugins' first runs still wait for it); the others do not
        // wait for it: a large package's first read can take seconds.
        let mut at = 0;
        while let Some(id) = self.packages.to_start.get(at).map(str::to_owned) {
            if self.packages.verifying(&id) {
                at += 1;
                continue;
            }
            self.packages.to_start.remove(at);
            self.start_installed(Some(&id));
            started += 1;
            if began.elapsed() >= START_BUDGET {
                break;
            }
        }
        if self.frame_log.is_some() {
            diag!(
                "vestra packages: {started} started in {:.1} ms, {} to go",
                began.elapsed().as_secs_f64() * 1000.,
                self.packages.to_start.len()
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
                self.packages.operations.insert(
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
                        waiting: None,
                        cancel: false,
                        failure: Some(
                            "application stopped before commit; restoring last committed version"
                                .into(),
                        ),
                        started: self.now_ms(),
                        result,
                        request,
                    },
                );
            }
            // The previous versions and built-in copies players up to 0.5
            // kept, and any leftovers, go on the cleanup thread.
            self.collect_garbage();
            // Start committed packages and the built-ins independently of
            // the authoring plugin: queued, a few per tick (`START_BUDGET`).
            // Built-ins first (`StartQueue::new`).
            let builtins = &self.packages.builtins;
            self.packages.to_start = StartQueue::new(self.packages.store.installed_ids(), |id| {
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
        self.answer_validations();
        self.shared_tick();
        self.start_queued();
        self.reload_tick();
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
            let mut op = self.packages.operations.remove(&key).unwrap();
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
            } else {
                self.packages.operations.insert(key, op);
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
        if let Some(waiting) = op.waiting {
            if !self.cleanup_done(waiting) {
                anyhow::ensure!(
                    self.now_ms() - op.started < 20_000,
                    "cleanup acknowledgment timed out; recovery remains pending"
                );
                return Ok(());
            }
            op.waiting = None;
        }
        if op.phase == "preparing" && !op.cancel && !op.enabled {
            // It stays (or becomes) disabled: its files are installed and
            // nothing runs it, not even to check its activation.
            let was_enabled = !self.packages.store.disabled().contains(&op.package);
            self.commit_operation(op, None)?;
            // Switched off by this install: its session values go, as for
            // `packages.setEnabled(false)`.
            if was_enabled && let Some(key) = &op.old_key {
                self.session_storage.clear(key);
            }
            return Ok(());
        }
        if op.phase == "preparing" && !op.cancel {
            match self.load_package(
                &op.target.directory,
                &op.target.manifest,
                op.target_key.clone(),
            ) {
                Ok(owner) => {
                    op.owner = Some(owner);
                    op.phase = "activating".into();
                }
                Err(error) => {
                    op.failure = Some(error.to_string());
                    op.phase = "restoring".into();
                }
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
                // `upgrade` in reverse), or, a first install, it goes
                // (`uninstall`); else its activation failed.
                let reason = match (op.cancel, op.old.is_some()) {
                    (true, true) => StopReason::Upgrade,
                    (true, false) => StopReason::Uninstall,
                    (false, _) => StopReason::Failed,
                };
                self.disable(owner, reason);
                op.waiting = Some(owner);
                op.owner = None;
                op.phase = "restoring".into();
                return Ok(());
            }
            if status == "ready" {
                return self.commit_operation(op, Some(owner));
            }
        }
        if op.phase == "preparing" && op.cancel {
            op.phase = "restoring".into();
        }
        if op.phase == "restoring" {
            // The disabled flag is still the one from before the install: a
            // package that was disabled is restored without running.
            let stopped = self.packages.store.disabled().contains(&op.package);
            if let Some(old) = op.old.clone().filter(|_| !stopped) {
                if op.owner.is_none() {
                    let key = op.old_key.clone().unwrap_or_else(|| op.target_key.clone());
                    op.owner = Some(self.load_package(&old.directory, &old.manifest, key)?);
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
        // The version this replaced is not kept.
        if let Some(old) = &op.old {
            self.discard(&old.digest);
        }
        Ok(())
    }
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
/// `Host::trust` as a table: `builtin` is `id`'s `builtin.json` entry,
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
/// or partial installation, not against a program running as the user
/// (doc/process/rounds/2026-10-shared-runner.md §3.3).
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
        /// (`Host::trust`). `None` from a schema-2 list: its own process.
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
    use crate::packages::package_store::Library;
    use std::collections::BTreeSet;
    use std::thread::JoinHandle;

    pub const THREAD: &str = "library-cleanup";

    pub enum Work {
        /// Everything the catalog does not refer to (`referenced`), and
        /// leftovers: at start, while nothing stages.
        Unreferenced(BTreeSet<String>),
        /// Only these directories (by digest).
        Only(BTreeSet<String>),
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

    pub fn spawn(library: Library, work: Work) -> std::io::Result<Running> {
        let only = match &work {
            Work::Unreferenced(_) => None,
            Work::Only(digests) => Some(digests.clone()),
        };
        let handle = std::thread::Builder::new()
            .name(THREAD.into())
            .spawn(move || {
                let started = std::time::Instant::now();
                let result = match &work {
                    Work::Unreferenced(referenced) => library.collect(referenced),
                    Work::Only(digests) => library.remove(digests),
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

    #[test]
    fn start_queue_starts_builtins_first_and_counts_them() {
        let builtin = |id: &str| id.starts_with("vestra.");
        // `installed_ids` gives them in id order.
        let ids = ["a.one", "b.two", "vestra.audio", "vestra.video"].map(String::from);
        let mut queue = StartQueue::new(ids, builtin);
        assert_eq!(queue.get(0), Some("vestra.audio"));
        // `start_queued` skips a built-in still being checked: it stays
        // queued, and counted, while the others start.
        assert_eq!(queue.remove(1).as_deref(), Some("vestra.video"));
        assert_eq!(queue.remove(1).as_deref(), Some("a.one"));
        assert!(queue.builtin_pending());
        assert_eq!(queue.remove(0).as_deref(), Some("vestra.audio"));
        assert!(!queue.builtin_pending());
        assert_eq!(queue.remove(0).as_deref(), Some("b.two"));
        assert_eq!(queue.remove(0), None);
        assert!(!queue.builtin_pending() && queue.is_empty());
        queue.push("vestra.subtitles".into(), true);
        assert!(queue.builtin_pending() && queue.contains("vestra.subtitles"));
    }

    /// A shared runner restart mid start-up queues its members again: the
    /// built-ins go after the built-ins still queued, before every
    /// third-party plugin, and the others at the end.
    #[test]
    fn a_restart_mid_start_up_keeps_builtins_at_the_front() {
        let builtin = |id: &str| id.starts_with("vestra.");
        let ids = ["a.one", "b.two", "c.three", "vestra.audio", "vestra.video"].map(String::from);
        let mut queue = StartQueue::new(ids, builtin);
        // vestra.audio and vestra.video started; vestra.audio is still being
        // checked (it stays at the front), a.one started.
        assert_eq!(queue.remove(1).as_deref(), Some("vestra.video"));
        assert_eq!(queue.remove(1).as_deref(), Some("a.one"));
        queue.push("vestra.video".into(), true);
        queue.push("vestra.subtitles".into(), true);
        queue.push("z.member".into(), false);
        let mut order = Vec::new();
        while let Some(id) = queue.remove(0) {
            order.push(id);
        }
        assert_eq!(
            order,
            [
                "vestra.audio",
                "vestra.video",
                "vestra.subtitles",
                "b.two",
                "c.three",
                "z.member"
            ]
        );
        assert!(!queue.builtin_pending());
    }
    use crate::ui::composition;

    fn candidate(dir: &Path, id: &str, version: &str) -> Candidate {
        std::fs::write(
            dir.join("manifest.json"),
            serde_json::to_vec(&json!({"id":id,
            "apiVersion":process::API,"version":version,"entry":"main.js","capabilities":[]}))
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
        store.uninstall("k", "rm", json!({}), true).unwrap();
        let again = install(&mut store, "k", "1.0.0", Source::File, false).unwrap();
        assert_eq!(again.instance.unwrap().guid, guid);
        store.uninstall("k", "rm2", json!({}), false).unwrap();
        assert!(store.dormant().is_empty());
        let fresh = install(&mut store, "k", "1.0.0", Source::File, false).unwrap();
        assert_ne!(fresh.instance.unwrap().guid, guid);
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
                "version":version,"entry":"main.js","capabilities":[]}))
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
            operations: BTreeMap::new(),
            next: 0,
            receipts: BTreeMap::new(),
            booted: false,
            to_start: StartQueue::default(),
            builtins,
            cleanup: None,
            cleanup_queue: BTreeSet::new(),
            reloads: BTreeMap::new(),
            verified: BTreeMap::new(),
            verify_serial: 0,
            verify_results: std::sync::mpsc::channel(),
            validations: Default::default(),
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
        )
        .unwrap();
        assert!(
            running.blocks(None),
            "the start-up scan blocks every change"
        );
        running.join();
        assert!(!package.directory.exists());
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
    /// the main window's shell with no plugin mounted (each mount recompiles
    /// the shell with every mounted component, `prepare_mount_tick`) and
    /// each built-in's .slint files compiled on their own.
    #[test]
    #[ignore]
    fn slint_compile_bench() {
        let Some(dir) = std::env::var_os("VESTRA_SLINT_BENCH") else {
            return;
        };
        let (events, _receiver) = std::sync::mpsc::sync_channel(16);
        for round in 0..3 {
            let lap = std::time::Instant::now();
            composition::compile_json(&[], events.clone(), 1).unwrap();
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

    /// The shell compiles at start-up (not run by default;
    /// `VESTRA_SLINT_BENCH` names a `resources/plugins` directory): the
    /// main window with nothing mounted, then the built-ins' three panels
    /// (playback bottom bar, playlist sidebar, visualizer backdrop) mounted
    /// one compile each, as they arrive, against all three in one compile.
    #[test]
    #[ignore]
    fn startup_mount_bench() {
        let Some(dir) = std::env::var_os("VESTRA_SLINT_BENCH") else {
            return;
        };
        let dir = Path::new(&dir);
        let spec = |owner, plugin: &str, file: &str, component: &str, global: &str, slot| {
            composition::CompileSpec {
                owner,
                file: dunce::canonicalize(dir.join(plugin).join(file)).unwrap(),
                component: component.into(),
                global: Some(global.into()),
                slot,
            }
        };
        let specs = [
            spec(
                2,
                "vestra.playback",
                "ui/bar.slint",
                "PlaybackBar",
                "PlaybackBridge",
                composition::Slot::Bottom,
            ),
            spec(
                3,
                "vestra.playlist",
                "ui/sidebar.slint",
                "PlaylistSidebar",
                "PlaylistBridge",
                composition::Slot::Sidebar,
            ),
            spec(
                4,
                "vestra.visualizer",
                "ui/backdrop.slint",
                "Backdrop",
                "Art",
                composition::Slot::Backdrop,
            ),
        ];
        let (events, _receiver) = std::sync::mpsc::sync_channel(16);
        let compile = |specs: &[composition::CompileSpec]| {
            let lap = std::time::Instant::now();
            composition::compile_json(specs, events.clone(), 1).unwrap();
            lap.elapsed().as_millis()
        };
        diag!("main window, nothing mounted: {} ms", compile(&[]));
        let one_each = (1..=specs.len())
            .map(|n| compile(&specs[..n]))
            .collect::<Vec<_>>();
        diag!(
            "one compile per mount: {one_each:?} ms, longest {} ms, total {} ms",
            one_each.iter().max().unwrap(),
            one_each.iter().sum::<u128>()
        );
        diag!("all three in one compile: {} ms", compile(&specs));
        for round in 0..3 {
            let lap = std::time::Instant::now();
            let source = composition::shell_source(&specs).unwrap();
            let sourced = lap.elapsed().as_millis();
            let definition = composition::build(source, "HostWindow").unwrap();
            let built = lap.elapsed().as_millis();
            let _instance = definition.create().unwrap();
            let created = lap.elapsed().as_millis();
            diag!(
                "round {round}: source {sourced} ms, compile {} ms, create {} ms",
                built - sourced,
                created - built
            );
        }
    }

    /// B0 (supervisor.md §10, §11): start-up and wait tunables as they are
    /// today (the 15 s activation deadline and the install wait of 20 s are
    /// literals in `load_package` and `advance_package`).
    #[test]
    fn b0_start_up_and_wait_tunables() {
        assert_eq!(START_BUDGET, std::time::Duration::from_millis(25));
        assert_eq!(VERIFY_WAIT, std::time::Duration::from_secs(3));
        assert_eq!(RELOAD_WAIT_MS, 20_000);
    }

    /// B0: the `StartQueue` semantics `start_queued` relies on (it needs a
    /// `Host`, so its walk is written out here): `get(at)` and
    /// `remove(at)` let the plugins behind a held built-in start while it
    /// keeps its place at the front and its count (`builtin_pending`, which
    /// keeps third-party first runs waiting).
    #[test]
    fn b0_start_queue_lets_others_pass_a_held_builtin() {
        let builtin = |id: &str| id.starts_with("vestra.");
        let ids = ["a.one", "vestra.slow", "vestra.fast"].map(String::from);
        let mut queue = StartQueue::new(ids, builtin);
        let verifying = |id: &str| id == "vestra.slow";
        let mut started = Vec::new();
        let mut at = 0;
        while let Some(id) = queue.get(at).map(str::to_owned) {
            if verifying(&id) {
                at += 1;
                continue;
            }
            started.push(queue.remove(at).unwrap());
            assert!(queue.builtin_pending(), "vestra.slow still counts");
        }
        assert_eq!(started, ["vestra.fast", "a.one"]);
        assert_eq!((queue.len(), queue.get(0)), (1, Some("vestra.slow")));
    }
}
