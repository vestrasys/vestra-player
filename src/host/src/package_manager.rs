//! Finite trusted-package lifecycle. This is not a sandbox or dependency solver.
#![forbid(unsafe_code)]
use super::*;
use package_stage::Candidate;
use package_store::{Instance, Source, Store, StoredPackage};
use plugins::DataKey;
use std::collections::VecDeque;

/// Validated candidates kept at once (each is a staged copy on disk).
const MAX_CANDIDATES: usize = 32;
pub struct Manager {
    store: Store,
    /// Secret names per plugin service (`secrets.*`), for deleting them.
    pub secrets: crate::secret_index::Index,
    /// Validated packages by token, at most `MAX_CANDIDATES`; `candidate_order`
    /// is oldest first, and a new validation drops the oldest token (`stale`).
    candidates: BTreeMap<String, Candidate>,
    candidate_order: VecDeque<String>,
    operations: BTreeMap<String, Operation>,
    next: u64,
    receipts: BTreeMap<String, Value>,
    booted: bool,
    /// Built-in plugins being seeded on the `builtin-seed` thread.
    seeding: Option<seed::Running>,
}
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
            if let Some(oldest) = self.candidate_order.pop_front() {
                self.candidates.remove(&oldest);
            }
        }
        self.candidate_order.push_back(token.clone());
        self.candidates.insert(token, candidate);
    }
    pub fn open() -> Result<Self> {
        let root = std::env::var_os("VESTRA_PACKAGE_STORE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                crate::platform::app_data_dir()
                    .unwrap_or_default()
                    .join("packages")
            });
        let secrets = crate::secret_index::Index::new(Some(root.join("secrets")));
        let store = Store::open(root)?;
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
            seeding: None,
        })
    }
}
impl Host {
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
        anyhow::ensure!(
            !self.plugins.values().any(|p| p.manifest.id == id),
            "plugin already running"
        );
        self.activations.begin(owner, self.now_ms() + 15_000)?;
        let plugin = Plugin::load_manifest(
            directory,
            owner,
            self.events.clone(),
            plugins::Manifest {
                id: id.clone(),
                ..manifest.clone()
            },
            key,
        )?;
        self.plugins.insert(owner, plugin);
        self.backend.commands.send(Command {
            reply: Reply { owner, id: 0 },
            method: "owner.registered".into(),
            params: json!({"manifestId":id}),
        })?;
        Ok(owner)
    }
    fn package_status(&mut self, owner: u64) -> String {
        self.activation(owner)
            .ok()
            .and_then(|v| v["activation"]["status"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "unknown".into())
    }
    pub(super) fn package_dispatch(
        &mut self,
        caller: u64,
        method: &str,
        params: &Value,
    ) -> std::result::Result<Value, RpcError> {
        match method {
            "packages.validate" | "packages.validateArchive" => {
                let candidate = if method == "packages.validate" {
                    Candidate::capture(Path::new(string(params, "directory")?))?
                } else {
                    let file = Path::new(string(params, "file")?);
                    package_archive::check_extension(file)
                        .map_err(|e| RpcError::invalid(e.to_string()))?;
                    let extracted = package_archive::extract(file)
                        .map_err(|e| RpcError::invalid(format!("{e:#}")))?;
                    Candidate::capture(extracted.path())?
                };
                check_reserved(&candidate.manifest().id, None)?;
                if params
                    .get("digest")
                    .is_some_and(|v| v.as_str() != Some(candidate.digest()))
                {
                    return Err(RpcError::new("staleDigest", "candidate digest changed"));
                }
                if params
                    .get("apiVersion")
                    .is_some_and(|v| v.as_str() != Some(plugins::API))
                {
                    return Err(RpcError::new("versionMismatch", "contract mismatch"));
                }
                let result = crate::runtime_paths::get()
                    .node_command()
                    .arg("--check")
                    .arg(candidate.directory().join(&candidate.manifest().entry))
                    .output()
                    .map_err(anyhow::Error::from)?;
                if !result.status.success() {
                    return Err(RpcError::new(
                        "javascript",
                        String::from_utf8_lossy(&result.stderr)
                            .chars()
                            .take(8192)
                            .collect::<String>(),
                    ));
                }
                fn slint_files(root: &Path, found: &mut Vec<PathBuf>) -> Result<()> {
                    for entry in std::fs::read_dir(root)? {
                        let p = entry?.path();
                        if p.is_dir() {
                            slint_files(&p, found)?;
                        } else if p.extension().is_some_and(|x| x == "slint") {
                            found.push(p);
                        }
                    }
                    Ok(())
                }
                let mut files = vec![];
                slint_files(candidate.directory(), &mut files)?;
                for path in files {
                    let compiler = crate::ui::compiler();
                    let result = pollster::block_on(compiler.build_from_path(path));
                    if result.has_errors() {
                        return Err(RpcError::new(
                            "slint",
                            format!("{:?}", result.diagnostics().collect::<Vec<_>>()),
                        ));
                    }
                }
                self.packages.next += 1;
                let token = format!("candidate-{}-{}", self.packages.next, candidate.digest());
                let report = json!({"token":token,"digest":candidate.digest(),"apiVersion":plugins::API,"manifest":candidate.manifest(),"checks":["manifest","digest","productionJavaScript","slintCompile"],"trustedExecution":true});
                self.packages.remember(token, candidate);
                Ok(report)
            }
            "packages.installed" => Ok(serde_json::to_value(
                self.packages.store.get(string(params, "id")?),
            )
            .map_err(anyhow::Error::from)?),
            "packages.install" | "packages.rollback" => {
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
                self.library_busy()?;
                if self.packages.operations.len() >= 64 {
                    return Err(RpcError::new("busy", "operation history full"));
                }
                let source = if method == "packages.install" {
                    Some(install_source(params)?)
                } else {
                    None
                };
                let replace = params.get("replace").is_some_and(|v| v == true);
                let target = if method == "packages.install" {
                    let token = string(params, "token")?;
                    let candidate = self
                        .packages
                        .candidates
                        .get(token)
                        .cloned()
                        .ok_or_else(|| RpcError::new("stale", "unknown validation token"))?;
                    check_reserved(&candidate.manifest().id, None)?;
                    self.packages.store.gc()?;
                    self.packages.store.stage(&candidate)?
                } else {
                    self.packages
                        .store
                        .get(string(params, "id")?)
                        .previous
                        .ok_or_else(|| RpcError::new("missing", "no previous successful version"))?
                };
                let id = target.manifest.id.clone();
                let expected = params["expectedRevision"]
                    .as_u64()
                    .ok_or_else(|| RpcError::invalid("expectedRevision required"))?;
                let before = self.packages.store.get(&id);
                // A rollback of a reserved id: only the built-in instance's.
                check_reserved(&id, before.instance.as_ref())?;
                if before.revision != expected {
                    return Err(RpcError::new(
                        "revisionConflict",
                        "installed revision changed",
                    ));
                }
                let (instance, fresh) = match source {
                    Some(source) => self.install_instance(&id, &before, source, replace)?,
                    None => (None, false),
                };
                self.start_operation(request, request_spec, target, before, instance, fresh)
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
            "packages.uninstall" => self.uninstall(caller, params),
            _ => Err(RpcError::new("unsupported", "unknown package method")),
        }
    }
    /// Which instance an install goes into (store proposal §1.3): the same
    /// source upgrades, another source needs `replace` and reconnects to a
    /// dormant instance of that source when there is one. Built-ins are only
    /// upgraded by seeding. `(instance, fresh)`: `fresh` when new data starts.
    fn install_instance(
        &self,
        id: &str,
        before: &package_store::Installed,
        source: Source,
        replace: bool,
    ) -> std::result::Result<(Option<Instance>, bool), RpcError> {
        install_instance(&self.packages.store, id, before, source, replace)
    }
    /// Journal the install (or rollback) and stop the running version; the
    /// tick activates the target and commits or restores.
    fn start_operation(
        &mut self,
        request: String,
        request_spec: Value,
        target: StoredPackage,
        before: package_store::Installed,
        instance: Option<Instance>,
        fresh: bool,
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
        self.packages.store.prepare_with_receipt(
            &id,
            before.revision,
            &target,
            &request,
            result.clone(),
            instance,
        )?;
        let waiting = self.running(&id);
        if let Some(owner) = waiting {
            self.disable(owner);
        }
        let fresh_data = fresh
            .then(|| plugins::data_root().map(|root| root.join(&target_guid)))
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
        plugins::data_root().map(|root| key.directory(&root))
    }
    /// Install or upgrade the built-in plugins the installation ships
    /// (`<resources>/plugins/builtin.json`, or `$VESTRA_BUILTIN_PLUGINS`). A
    /// built-in is upgraded only when the installation carries a newer
    /// version than it last seeded, so a user's rollback stays.
    ///
    /// The file work (hash, unpack, verify; seconds for a large package on
    /// Windows, where the antivirus scans each new file on first read) runs
    /// on a `builtin-seed` thread; the UI thread decides what to seed,
    /// commits each staged package to the catalog (one replacement, no
    /// reading of package files) and then starts it, one per tick
    /// (`seed_tick`). The other installed plugins start right away. While
    /// seeding runs, the thread is the library's only writer: install,
    /// rollback and uninstall answer `busy`, and a plugin being seeded cannot
    /// be enabled (`package_busy`).
    fn start_seeding(&mut self) {
        let Some(directory) = seed::directory() else {
            return;
        };
        let list = match seed::read(&directory) {
            Ok(list) => list,
            Err(error) => {
                eprintln!("built-in plugins: {error:#}");
                return;
            }
        };
        let mut jobs = Vec::new();
        for entry in list {
            match self.seed_decision(&entry) {
                Ok(Some(_)) => jobs.push(entry),
                Ok(None) => {}
                Err(error) => eprintln!("built-in plugin {}: {error:#}", entry.id),
            }
        }
        if jobs.is_empty() {
            return;
        }
        let awaiting = jobs.iter().map(|entry| entry.id.clone()).collect();
        let library = self.packages.store.library();
        let referenced = self.packages.store.referenced();
        let (sender, receiver) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("builtin-seed".into())
            .spawn(move || seed::stage_all(&directory, &library, &referenced, jobs, &sender));
        match spawned {
            Ok(_) => {
                self.packages.seeding = Some(seed::Running {
                    receiver,
                    awaiting,
                    ready: VecDeque::new(),
                    started: std::time::Instant::now(),
                    seeded: 0,
                })
            }
            // Nothing is seeded this run; the installed versions start.
            Err(error) => eprintln!("built-in plugins: {error}"),
        }
    }
    /// The instance seeding `entry` would install into, if it is to be seeded.
    fn seed_decision(&self, entry: &seed::Entry) -> Result<Option<Instance>> {
        if self.development.contains_key(&entry.id) || self.operation_pending(&entry.id) {
            return Ok(None);
        }
        seed::decide(entry, &self.packages.store.get(&entry.id))
    }
    /// Take the seeding thread's results: commit one staged package per tick
    /// and start it; when all are in, the start-up is complete.
    fn seed_tick(&mut self) {
        let Some(running) = self.packages.seeding.as_mut() else {
            return;
        };
        let mut finished = false;
        loop {
            match running.receiver.try_recv() {
                Ok(result) => running.ready.push_back(result),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    finished = true;
                    break;
                }
            }
        }
        if let Some((entry, staged)) = running.ready.pop_front() {
            running.awaiting.remove(&entry.id);
            match staged.and_then(|target| self.commit_seed(&entry, target)) {
                Ok(()) => {
                    if let Some(running) = self.packages.seeding.as_mut() {
                        running.seeded += 1;
                    }
                }
                Err(error) => eprintln!("built-in plugin {}: {error:#}", entry.id),
            }
            // Seeded or not, the installed version starts (a failure shows
            // as `failed` in the list).
            self.start_installed(Some(&entry.id));
            return;
        }
        if !finished {
            return;
        }
        let Some(running) = self.packages.seeding.take() else {
            return;
        };
        for id in &running.awaiting {
            eprintln!("built-in plugin {id}: seeding stopped");
            self.start_installed(Some(id));
        }
        eprintln!(
            "built-in plugins: {} seeded in {} ms",
            running.seeded,
            running.started.elapsed().as_millis()
        );
    }
    /// Record a package the seeding thread staged, if it is still to be
    /// seeded. Reads no package file (`Store::commit_seeded`).
    fn commit_seed(&mut self, entry: &seed::Entry, target: StoredPackage) -> Result<()> {
        let Some(instance) = self.seed_decision(entry)? else {
            return Ok(());
        };
        let before = self.packages.store.get(&entry.id);
        if before
            .current
            .as_ref()
            .is_some_and(|current| current.digest == target.digest)
        {
            self.packages.store.set_seeded(&entry.id, &entry.version)?;
            return Ok(());
        }
        // Seeding commits directly, without waiting for the activation; the
        // plugin then starts like any installed package.
        let request = format!("seed-{}", &entry.sha256[..32]);
        let spec = json!({"method":"seed","params":{"id":entry.id,"version":entry.version}});
        self.packages.store.commit_seeded(
            &entry.id,
            before.revision,
            &target,
            &request,
            (
                json!({"operationId":request,"packageId":entry.id,"status":"preparing","request":spec}),
                json!({"operationId":request,"packageId":entry.id,"status":"committed","request":spec}),
            ),
            instance,
        )?;
        self.packages.receipts = self.packages.store.operations();
        Ok(())
    }
    /// Start the installed packages (only `only`, when given) that are not
    /// disabled, not replaced by a development load, not in an operation and
    /// not being seeded.
    fn start_installed(&mut self, only: Option<&str>) {
        let pending = self
            .packages
            .operations
            .values()
            .map(|o| o.package.clone())
            .collect::<Vec<_>>();
        let installed = self.packages.store.installed();
        for (id, record) in installed {
            if only.is_some_and(|only| only != id)
                || self.running(&id).is_some()
                || self.seeding(&id)
            {
                continue;
            }
            // R-H10: a package the user disabled stays stopped; a
            // development load of the same id runs instead this run.
            if !pending.contains(&id)
                && !self.packages.store.disabled().contains(&id)
                && !self.development.contains_key(&id)
                && let Some(current) = record.current
                && let Some(instance) = record.instance
                && let Err(error) = check_reserved(&id, Some(&instance))
                    .map_err(|refusal| anyhow::anyhow!(refusal.message))
                    .and_then(|()| {
                        self.load_package(
                            &current.directory,
                            &current.manifest,
                            DataKey::Instance(instance.guid),
                        )
                    })
            {
                eprintln!("installed package {id} failed to start: {error}");
                self.failures
                    .insert(id.clone(), error.to_string().chars().take(512).collect());
            }
        }
    }
    /// Whether `id` waits for the seeding thread.
    fn seeding(&self, id: &str) -> bool {
        self.packages
            .seeding
            .as_ref()
            .is_some_and(|running| running.awaiting.contains(id))
    }
    /// An operation runs on `id`, or it is being seeded.
    fn package_busy(&self, id: &str) -> bool {
        self.operation_pending(id) || self.seeding(id)
    }
    /// While built-ins are seeded the seeding thread writes the package
    /// library; operations that stage, collect or remove packages wait.
    fn library_busy(&self) -> std::result::Result<(), RpcError> {
        if self.packages.seeding.is_some() {
            return Err(RpcError::new(
                "busy",
                "built-in plugins are being updated; try again shortly",
            ));
        }
        Ok(())
    }
    /// A running built-in plugin (shipped with the player, not a development load).
    pub(super) fn is_builtin(&self, owner: u64) -> bool {
        let Some(plugin) = self.plugins.get(&owner) else {
            return false;
        };
        let stored = self.packages.store.get(&plugin.manifest.id);
        stored
            .current
            .is_some_and(|current| current.directory == plugin.directory)
            && stored.instance.is_some_and(|instance| instance.builtin)
    }
    /// Until the installed plugins started and while a built-in activates,
    /// other plugins' mounts wait (slot_claims).
    pub(super) fn builtins_starting(&mut self) -> bool {
        if !self.packages.booted || self.packages.seeding.is_some() {
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
    fn running(&self, id: &str) -> Option<u64> {
        self.plugins
            .iter()
            .find(|(_, plugin)| plugin.manifest.id == id)
            .map(|(owner, _)| *owner)
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
    pub(super) fn package_list(&mut self) -> Value {
        let now = self.now_ms();
        let mut list = BTreeMap::<String, Value>::new();
        let owners = self.plugins.keys().copied().collect::<Vec<_>>();
        for owner in owners {
            let status = self.activations.status(owner, now).ok();
            let plugin = &self.plugins[&owner];
            let id = plugin.manifest.id.clone();
            let stored = self.packages.store.get(&id);
            let installed = stored
                .current
                .as_ref()
                .is_some_and(|current| current.directory == plugin.directory);
            let warnings = self.warnings_of(owner);
            let (state, error) = match status {
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
            let previous = installed
                .then(|| stored.previous.and_then(|p| p.manifest.version))
                .flatten();
            let instance = installed.then_some(stored.instance).flatten();
            list.insert(id.clone(), json!({"id":id,"version":plugin.manifest.version,
                "origin":origin,"state":state,"error":error,"capabilities":plugin.manifest.capabilities,
                "directory":plugin.directory,"previousVersion":previous,
                "builtin":instance.as_ref().is_some_and(|i| i.builtin),
                "source":instance.map(|i| i.source),
                "dataDirectory":plugin.data_directory(),"warnings":warnings,
                "display":display(&plugin.manifest, plugin.catalog.as_deref())}));
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
            let Some(current) = record.current.filter(|_| !list.contains_key(&id)) else {
                continue;
            };
            let (state, error) = if self.package_busy(&id) {
                ("starting", None)
            } else {
                stopped(
                    self.packages.store.disabled().contains(&id),
                    self.failures.get(&id),
                )
            };
            let data = record
                .instance
                .as_ref()
                .and_then(|i| self.data_directory_of(&DataKey::Instance(i.guid.clone())));
            list.insert(
                id.clone(),
                json!({"id":id,"version":current.manifest.version,
                "origin":"installed","state":state,"error":error,
                "capabilities":current.manifest.capabilities,"directory":current.directory,
                "previousVersion":record.previous.and_then(|p| p.manifest.version),
                "builtin":record.instance.as_ref().is_some_and(|i| i.builtin),
                "source":record.instance.map(|i| i.source),
                "dataDirectory":data,"warnings":[],
                "display":display(&current.manifest, crate::i18n::load(&current.directory).as_deref())}),
            );
        }
        for (id, (directory, manifest)) in &self.development {
            if list.contains_key(id) {
                continue;
            }
            let (state, error) = stopped(self.dev_disabled.contains(id), self.failures.get(id));
            let data = self.data_directory_of(&DataKey::Development(id.clone()));
            list.insert(
                id.clone(),
                json!({"id":id,"version":manifest.version,
                "origin":"development","state":state,"error":error,
                "capabilities":manifest.capabilities,"directory":directory,"previousVersion":null,
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
            return Err(RpcError::new("busy", "the package is being installed"));
        }
        let record = self.packages.store.get(id);
        let installed = record.current;
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
                self.disable(owner);
            }
            self.failures.remove(id);
            return Ok(());
        }
        if running.is_some() {
            return Ok(());
        }
        self.failures.remove(id);
        match (installed, development) {
            (_, Some((directory, _))) => self.load(&directory, 120_000)?,
            (Some(current), None) => {
                reserved?;
                // Only seeding and installs record a package, both with an instance.
                let key = key.ok_or_else(|| {
                    RpcError::new("unavailable", "the installed package has no instance")
                })?;
                self.load_package(&current.directory, &current.manifest, key)?;
            }
            (None, None) => unreachable!("checked above"),
        }
        Ok(())
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
        if installed.instance.as_ref().is_some_and(|i| i.builtin) {
            return Err(RpcError::new(
                "unsupported",
                "built-in plugins can be disabled but not uninstalled",
            ));
        }
        if installed.current.is_none() {
            return Err(
                if self.development.contains_key(&id) || self.running(&id).is_some() {
                    RpcError::new("unsupported", "only installed plugins can be uninstalled")
                } else {
                    RpcError::new("missing", format!("no plugin {id}"))
                },
            );
        }
        if params["expectedRevision"].as_u64() != Some(installed.revision) {
            return Err(RpcError::new(
                "revisionConflict",
                "installed revision changed",
            ));
        }
        self.library_busy()?;
        if self.package_busy(&id) {
            return Err(RpcError::new("busy", "the package is being installed"));
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
        if let Some(owner) = self.running(&id) {
            self.disable(owner);
            // Delete the data after the process is gone (it flushes on stop).
            if let Some(directory) = data.clone() {
                self.data_removals.insert(owner, directory);
            }
        } else if let Some(directory) = &data {
            let _ = std::fs::remove_dir_all(directory);
        }
        self.packages
            .store
            .uninstall(&id, &request, receipt.clone(), !remove_data)?;
        self.packages.receipts = self.packages.store.operations();
        self.failures.remove(&id);
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
            eprintln!("secrets purge: {error}");
        }
    }
    /// Retired instances dropped because the dormant list was full can never
    /// be reconnected: their secrets go too.
    fn purge_dropped_secrets(&mut self) {
        for (guid, _) in self.packages.store.take_dropped() {
            self.purge_secrets(&DataKey::Instance(guid));
        }
    }
    /// The installed packages were started (the first `package_tick`) and
    /// the built-ins seeded and started.
    pub(super) fn packages_started(&self) -> bool {
        self.packages.booted && self.packages.seeding.is_none()
    }
    pub(super) fn package_tick(&mut self) {
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
                    .and_then(|i| plugins::data_root().map(|root| root.join(&i.guid)));
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
            self.start_seeding();
            // Start committed packages independently of the authoring
            // plugin; the ones being seeded start as their seed lands.
            self.start_installed(None);
        }
        self.seed_tick();
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
                    eprintln!("operation receipt: {error}");
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
                self.disable(owner);
                op.waiting = Some(owner);
                op.owner = None;
                op.phase = "restoring".into();
                return Ok(());
            }
            if status == "ready" {
                let receipt = json!({"operationId":op.id,"packageId":op.package,"status":"committed","owner":owner,"request":op.request});
                let outcome = self
                    .packages
                    .store
                    .commit_with_receipt(&op.package, &op.id, receipt);
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
                return Ok(());
            }
        }
        if op.phase == "preparing" && op.cancel {
            op.phase = "restoring".into();
        }
        if op.phase == "restoring" {
            if let Some(old) = op.old.clone() {
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
            return Ok(());
        }
        op.result = json!({"operationId":op.id,"packageId":op.package,"status":op.phase,"owner":op.owner,"cancelRequested":op.cancel});
        Ok(())
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
        if text.len() > crate::manifest::URL_MAX {
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
fn display(manifest: &plugins::Manifest, catalog: Option<&crate::i18n::Catalog>) -> Value {
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

/// The 2026-10-24 ruling (ecosystem-plan.md): a reserved `vestra.` id
/// (`manifest::reserved_id`) is installed and started only as the
/// installation's built-in instance, which seeding alone creates. Store,
/// file, link and workshop installs of one are refused, and an
/// installed non-built-in one (from before the rule) is not started.
/// Development loads (`--plugin`) are not checked here (`Host::load`).
fn check_reserved(id: &str, instance: Option<&Instance>) -> std::result::Result<(), RpcError> {
    if crate::manifest::reserved_id(id) && !instance.is_some_and(|i| i.builtin) {
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

/// The installation's built-in plugin list (written by `scripts/stage-plugins.mjs`)
/// and the seeding thread's work (`Host::start_seeding`).
mod seed {
    use crate::package_store::{Installed, Instance, Library, Source, StoredPackage};
    use anyhow::{Context, Result, ensure};
    use serde::Deserialize;
    use std::collections::{BTreeSet, VecDeque};
    use std::path::{Path, PathBuf};
    use std::sync::mpsc::{Receiver, Sender};
    use std::time::Instant;

    /// A package the seeding thread staged in the library, or why not.
    pub type Staged = (Entry, Result<StoredPackage>);

    /// Seeding in progress (`Manager::seeding`).
    pub struct Running {
        pub receiver: Receiver<Staged>,
        /// Ids whose result has not been committed yet; they start then.
        pub awaiting: BTreeSet<String>,
        /// Results received, committed one per tick.
        pub ready: VecDeque<Staged>,
        pub started: Instant,
        pub seeded: usize,
    }

    /// The instance seeding `entry` installs into, or `None` when it is not
    /// seeded: installed from another source, or a version at least as new
    /// already seeded (a user's rollback stays) whose directory is still in
    /// the library.
    pub fn decide(entry: &Entry, before: &Installed) -> Result<Option<Instance>> {
        let version = crate::manifest::parse_version(&entry.version)
            .with_context(|| format!("invalid version {}", entry.version))?;
        Ok(match &before.instance {
            Some(current) if !current.builtin => {
                eprintln!(
                    "built-in plugin {}: installed from another source; not seeded",
                    entry.id
                );
                None
            }
            Some(current) => {
                let seeded = current
                    .seeded_version
                    .as_deref()
                    .and_then(crate::manifest::parse_version);
                // A current version whose directory is gone (collected by
                // another process sharing the library) is seeded again.
                let missing = before
                    .current
                    .as_ref()
                    .is_some_and(|package| !package.directory.is_dir());
                (missing || !seeded.is_some_and(|seeded| seeded >= version)).then(|| Instance {
                    seeded_version: Some(entry.version.clone()),
                    ..current.clone()
                })
            }
            None => Some(Instance {
                builtin: true,
                seeded_version: Some(entry.version.clone()),
                ..Instance::new(Source::Bundled)
            }),
        })
    }

    /// The seeding thread: collect the library's garbage (`referenced` as
    /// the catalog stood when seeding began; nothing else writes the
    /// library meanwhile), then stage each entry and send it.
    pub fn stage_all(
        directory: &Path,
        library: &Library,
        referenced: &BTreeSet<String>,
        jobs: Vec<Entry>,
        sender: &Sender<Staged>,
    ) {
        if let Err(error) = library.gc(referenced) {
            eprintln!("built-in plugins: package library: {error:#}");
        }
        for entry in jobs {
            let staged = stage(directory, library, &entry);
            if sender.send((entry, staged)).is_err() {
                return;
            }
        }
    }

    /// Check `entry`'s archive against its sha256 and stage it in the library.
    pub fn stage(directory: &Path, library: &Library, entry: &Entry) -> Result<StoredPackage> {
        let file = directory.join(&entry.file);
        ensure!(
            crate::package_archive::sha256(&file)? == entry.sha256,
            "{} does not match its sha256",
            entry.file
        );
        let target = library.stage_archive(&file)?;
        ensure!(
            target.manifest.id == entry.id
                && target.manifest.version.as_deref() == Some(entry.version.as_str()),
            "{} holds {} {:?}, not {} {}",
            entry.file,
            target.manifest.id,
            target.manifest.version,
            entry.id,
            entry.version
        );
        Ok(target)
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
        pub file: String,
        pub sha256: String,
    }

    /// `$VESTRA_BUILTIN_PLUGINS`, else the installation's built-in plugins
    /// (`runtime_paths.rs`); none when it has no `builtin.json` (a
    /// development run).
    pub fn directory() -> Option<PathBuf> {
        crate::runtime_paths::get().builtin_directory()
    }

    pub fn read(directory: &Path) -> Result<Vec<Entry>> {
        let bytes = std::fs::read(directory.join("builtin.json")).context("builtin.json")?;
        ensure!(bytes.len() <= 256 * 1024, "builtin.json is too large");
        let list: List = serde_json::from_slice(&bytes).context("builtin.json")?;
        ensure!(
            list.schema == 1,
            "builtin.json schema {} is unknown",
            list.schema
        );
        ensure!(
            list.plugins.len() <= 64,
            "builtin.json lists too many plugins"
        );
        for entry in &list.plugins {
            ensure!(
                crate::manifest::valid_id(&entry.id),
                "invalid id {}",
                entry.id
            );
            ensure!(
                entry.sha256.len() == 64
                    && entry
                        .sha256
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "{}: sha256 must be 64 lowercase hex digits",
                entry.id
            );
            ensure!(
                !entry.file.contains(['/', '\\']) && entry.file.ends_with(".vxp"),
                "{}: file must be a .vxp name in the same directory",
                entry.id
            );
        }
        Ok(list.plugins)
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
    use package_store::Installed;

    fn candidate(dir: &Path, id: &str, version: &str) -> Candidate {
        std::fs::write(
            dir.join("manifest.json"),
            serde_json::to_vec(&json!({"id":id,
            "apiVersion":plugins::API,"version":version,"entry":"main.js","capabilities":[]}))
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
        assert!(second.previous.is_some(), "rollback within the instance");
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
        // Replace: a new instance, the old one dormant, no rollback across.
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

    #[test]
    fn builtin_lists_are_strict() {
        let dir = tempfile::tempdir().unwrap();
        let write = |value: Value| {
            std::fs::write(dir.path().join("builtin.json"), value.to_string()).unwrap()
        };
        let sha = "a".repeat(64);
        write(
            json!({"schema":1,"plugins":[{"id":"vestra.video","version":"1.0.0","file":"vestra.video-1.0.0.vxp","sha256":sha}]}),
        );
        assert_eq!(seed::read(dir.path()).unwrap().len(), 1);
        for bad in [
            json!({"schema":2,"plugins":[]}),
            json!({"schema":1,"plugins":[{"id":"v","version":"1.0.0","file":"../v.vxp","sha256":sha}]}),
            json!({"schema":1,"plugins":[{"id":"v","version":"1.0.0","file":"v.zip","sha256":sha}]}),
            json!({"schema":1,"plugins":[{"id":"v","version":"1.0.0","file":"v.vxp","sha256":"x"}]}),
            json!({"schema":1,"plugins":[{"id":"v","version":"1.0.0","file":"v.vxp","sha256":sha,"extra":1}]}),
        ] {
            write(bad.clone());
            assert!(seed::read(dir.path()).is_err(), "{bad}");
        }
    }

    /// A `.vxp` of `id` `version` with `main.js` saying `body`, and its entry.
    fn archive(dir: &Path, id: &str, version: &str, body: &str) -> seed::Entry {
        use std::io::Write;
        let file = format!("{id}-{version}.vxp");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(dir.join(&file)).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("manifest.json", options).unwrap();
        writer
            .write_all(
                &serde_json::to_vec(&json!({"id":id,"apiVersion":plugins::API,
                "version":version,"entry":"main.js","capabilities":[]}))
                .unwrap(),
            )
            .unwrap();
        writer.start_file("ui/a.slint", options).unwrap();
        writer.write_all(b"export component A {}").unwrap();
        writer.start_file("main.js", options).unwrap();
        writer.write_all(body.as_bytes()).unwrap();
        writer.finish().unwrap();
        seed::Entry {
            id: id.into(),
            version: version.into(),
            sha256: package_archive::sha256(&dir.join(&file)).unwrap(),
            file,
        }
    }
    /// Seed `entry` the way the host does: decide, stage (the seeding
    /// thread's part), commit (the UI thread's part).
    fn seed_into(store: &mut Store, dir: &Path, entry: &seed::Entry) -> Option<Installed> {
        let before = store.get(&entry.id);
        let instance = seed::decide(entry, &before).unwrap()?;
        let target = seed::stage(dir, &store.library(), entry).unwrap();
        let receipt = json!({"operationId":"seed","status":"preparing"});
        Some(
            store
                .commit_seeded(
                    &entry.id,
                    before.revision,
                    &target,
                    &format!("seed-{}", &entry.sha256[..32]),
                    (receipt.clone(), receipt),
                    instance,
                )
                .unwrap(),
        )
    }

    #[test]
    fn seeding_installs_upgrades_only_to_newer_and_keeps_rollback_and_disabled() {
        let root = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(root.path().join("store")).unwrap();
        let v1 = archive(dir.path(), "vestra.demo", "1.0.0", "// 1");
        let first = seed_into(&mut store, dir.path(), &v1).unwrap();
        let instance = first.instance.clone().unwrap();
        assert!(instance.builtin);
        assert_eq!(instance.seeded_version.as_deref(), Some("1.0.0"));
        let current = first.current.clone().unwrap();
        // Staged in place under its digest, as a candidate captures it.
        let extracted = package_archive::extract(&dir.path().join(&v1.file)).unwrap();
        assert_eq!(
            current.digest,
            Candidate::capture(extracted.path()).unwrap().digest()
        );
        assert!(current.directory.join("ui/a.slint").is_file());
        // The same version again: nothing to do.
        assert!(seed_into(&mut store, dir.path(), &v1).is_none());
        // A newer version upgrades the same instance; the user's disabled
        // flag stays; the old version is the rollback target.
        store.set_disabled("vestra.demo", true).unwrap();
        let v2 = archive(dir.path(), "vestra.demo", "1.1.0", "// 2");
        let second = seed_into(&mut store, dir.path(), &v2).unwrap();
        assert_eq!(second.instance.as_ref().unwrap().guid, instance.guid);
        assert_eq!(second.previous.as_ref().unwrap().digest, current.digest);
        assert!(store.disabled().contains("vestra.demo"));
        assert!(store.pending().is_empty());
        // A rollback to 1.0.0 is not overwritten by the same installation.
        let rolled = store.get("vestra.demo");
        store
            .prepare("vestra.demo", rolled.revision, &current, "rollback-1")
            .unwrap();
        store.commit("vestra.demo", "rollback-1").unwrap();
        assert!(seed_into(&mut store, dir.path(), &v2).is_none());
        // An older installation does not downgrade either.
        let old = archive(dir.path(), "vestra.demo", "0.9.0", "// 0");
        assert!(
            seed::decide(&old, &store.get("vestra.demo"))
                .unwrap()
                .is_none()
        );
        // Garbage collection keeps every referenced version.
        store.library().gc(&store.referenced()).unwrap();
        let after = store.get("vestra.demo");
        assert!(after.current.unwrap().directory.is_dir());
        assert!(after.previous.unwrap().directory.is_dir());
    }

    #[test]
    fn seeding_refuses_a_wrong_hash_or_a_mislabelled_archive() {
        let root = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(root.path().join("store")).unwrap();
        let mut entry = archive(dir.path(), "vestra.demo", "1.0.0", "// 1");
        entry.sha256 = "0".repeat(64);
        let error = seed::stage(dir.path(), &store.library(), &entry).unwrap_err();
        assert!(error.to_string().contains("sha256"), "{error}");
        let mut entry = archive(dir.path(), "vestra.demo", "1.0.0", "// 1");
        entry.version = "1.0.1".into();
        let error = seed::stage(dir.path(), &store.library(), &entry).unwrap_err();
        assert!(
            error.to_string().contains("not vestra.demo 1.0.1"),
            "{error}"
        );
        // Nothing half-written is referenced: the next collection removes it.
        store.library().gc(&store.referenced()).unwrap();
        let left = std::fs::read_dir(root.path().join("store/packages"))
            .unwrap()
            .count();
        assert_eq!(left, 0);
    }

    /// Another process sharing the library (a development or headless run)
    /// may collect a directory staged but not committed yet, or a current one.
    #[test]
    fn a_package_collected_by_another_process_is_not_committed_and_heals() {
        let root = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(root.path().join("store")).unwrap();
        let v1 = archive(dir.path(), "vestra.demo", "1.0.0", "// 1");
        // Staged, then collected before the commit: no commit, no seededVersion.
        let target = seed::stage(dir.path(), &store.library(), &v1).unwrap();
        std::fs::remove_dir_all(&target.directory).unwrap();
        let instance = seed::decide(&v1, &store.get("vestra.demo"))
            .unwrap()
            .unwrap();
        let receipt = json!({"operationId":"seed","status":"preparing"});
        let error = store
            .commit_seeded(
                "vestra.demo",
                0,
                &target,
                "seed-1",
                (receipt.clone(), receipt),
                instance,
            )
            .unwrap_err();
        assert!(error.to_string().contains("gone"), "{error}");
        let after = store.get("vestra.demo");
        assert!(after.current.is_none() && after.instance.is_none());
        assert!(store.pending().is_empty());
        // Seeded, then its current directory collected: seeded again at the
        // same version, into the same instance.
        let first = seed_into(&mut store, dir.path(), &v1).unwrap();
        let current = first.current.unwrap();
        assert!(
            seed::decide(&v1, &store.get("vestra.demo"))
                .unwrap()
                .is_none()
        );
        std::fs::remove_dir_all(&current.directory).unwrap();
        let healed = seed::decide(&v1, &store.get("vestra.demo"))
            .unwrap()
            .unwrap();
        assert_eq!(healed.guid, first.instance.unwrap().guid);
        let target = seed::stage(dir.path(), &store.library(), &v1).unwrap();
        assert_eq!(target.digest, current.digest);
        assert!(current.directory.join("main.js").is_file());
    }

    #[test]
    fn another_source_is_not_seeded() {
        let dir = tempfile::tempdir().unwrap();
        let entry = archive(dir.path(), "vestra.demo", "1.0.0", "// 1");
        let before = Installed {
            instance: Some(Instance::new(Source::File)),
            ..Default::default()
        };
        assert!(seed::decide(&entry, &before).unwrap().is_none());
        let mut bad = archive(dir.path(), "vestra.demo", "1.0.0", "// 1");
        bad.version = "1.0".into();
        assert!(seed::decide(&bad, &Installed::default()).is_err());
    }

    /// Where seeding's time goes, on real packages (not run by default):
    /// `VESTRA_SEED_BENCH_OLD` / `VESTRA_SEED_BENCH_NEW` name two
    /// `resources/plugins` directories (an older and a newer installation).
    /// Prints, per package, the seeding thread's part (hash, unpack and
    /// verify) and the UI thread's part (the catalog commit).
    #[test]
    #[ignore]
    fn seed_bench() {
        let (Some(old), Some(new)) = (
            std::env::var_os("VESTRA_SEED_BENCH_OLD"),
            std::env::var_os("VESTRA_SEED_BENCH_NEW"),
        ) else {
            return;
        };
        let root = tempfile::tempdir().unwrap();
        let mut store = Store::open(root.path().join("packages")).unwrap();
        let mut run = |dir: &Path, label: &str| {
            let started = std::time::Instant::now();
            let mut longest_commit = 0;
            store.library().gc(&store.referenced()).unwrap();
            for entry in seed::read(dir).unwrap() {
                let before = store.get(&entry.id);
                let Some(instance) = seed::decide(&entry, &before).unwrap() else {
                    continue;
                };
                let lap = std::time::Instant::now();
                let target = seed::stage(dir, &store.library(), &entry).unwrap();
                let staged = lap.elapsed().as_millis();
                let lap = std::time::Instant::now();
                let receipt = json!({"operationId":"seed","status":"preparing"});
                store
                    .commit_seeded(
                        &entry.id,
                        before.revision,
                        &target,
                        &format!("seed-{}", &entry.sha256[..32]),
                        (receipt.clone(), receipt),
                        instance,
                    )
                    .unwrap();
                let committed = lap.elapsed().as_millis();
                longest_commit = longest_commit.max(committed);
                eprintln!(
                    "{label} {} {}: seeding thread {staged} ms, UI commit {committed} ms",
                    entry.id, entry.version
                );
            }
            eprintln!(
                "{label}: {} ms in all, longest UI commit {longest_commit} ms",
                started.elapsed().as_millis()
            );
        };
        run(Path::new(&old), "first");
        run(Path::new(&new), "upgrade");
        // The steps, for each package of the newer installation: unpacking
        // with and without flushing each file, reading new files (the
        // antivirus scans each on first read) and reading them again.
        let dir = Path::new(&new);
        for entry in seed::read(dir).unwrap() {
            let file = dir.join(&entry.file);
            let ms = |lap: std::time::Instant| lap.elapsed().as_millis();
            let lap = std::time::Instant::now();
            package_archive::sha256(&file).unwrap();
            let hash = ms(lap);
            let plain = tempfile::tempdir().unwrap();
            let lap = std::time::Instant::now();
            package_archive::extract_into(&file, plain.path(), false).unwrap();
            let unpack = ms(lap);
            let durable = tempfile::tempdir().unwrap();
            let lap = std::time::Instant::now();
            package_archive::extract_into(&file, durable.path(), true).unwrap();
            let unpack_durable = ms(lap);
            let lap = std::time::Instant::now();
            package_stage::inspect(plain.path()).unwrap();
            let first_read = ms(lap);
            let lap = std::time::Instant::now();
            package_stage::inspect(plain.path()).unwrap();
            let second_read = ms(lap);
            eprintln!(
                "steps {}: sha256 {hash} ms, unpack {unpack} ms, unpack+flush {unpack_durable} ms, \
                 first read+hash {first_read} ms, second read+hash {second_read} ms",
                entry.id
            );
        }
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
            eprintln!("shell (round {round}): {} ms", lap.elapsed().as_millis());
        }
        let dir = Path::new(&dir);
        let mut total = 0;
        for entry in seed::read(dir).unwrap() {
            let extracted = package_archive::extract(&dir.join(&entry.file)).unwrap();
            let mut files = Vec::new();
            let mut stack = vec![extracted.path().to_path_buf()];
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
                let _ = pollster::block_on(crate::ui::compiler().build_from_path(file));
            }
            let took = lap.elapsed().as_millis();
            total += took;
            eprintln!("slint {}: {} files, {took} ms", entry.id, files.len());
        }
        eprintln!("slint all built-ins: {total} ms");
    }
}
