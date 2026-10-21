//! Finite trusted-package lifecycle. This is not a sandbox or dependency solver.
#![forbid(unsafe_code)]
use super::*;
use package_stage::Candidate;
use package_store::{Instance, Source, Store, StoredPackage};
use plugins::DataKey;

pub struct Manager {
    store: Store,
    /// Secret names per plugin service (`secrets.*`), for deleting them.
    pub secrets: crate::secret_index::Index,
    candidates: BTreeMap<String, Candidate>,
    trials: BTreeMap<u64, Trial>,
    operations: BTreeMap<String, Operation>,
    next: u64,
    receipts: BTreeMap<String, Value>,
    booted: bool,
}
struct Trial {
    owner: u64,
    request_id: Option<String>,
    candidate: Candidate,
    stopping: bool,
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
            trials: BTreeMap::new(),
            operations: BTreeMap::new(),
            next: 0,
            receipts,
            booted: false,
        })
    }
    pub fn is_trial(&self, owner: u64) -> bool {
        self.trials.contains_key(&owner)
    }
    pub fn owner_gone(&mut self, owner: u64) {
        for trial in self.trials.values_mut() {
            if trial.owner == owner {
                trial.stopping = true;
            }
        }
    }
}
impl Host {
    fn trial_state(&mut self, id: u64) -> std::result::Result<Value, RpcError> {
        let trial = &self.packages.trials[&id];
        let state = if self.cleanup_done(id) {
            "stopped".to_owned()
        } else if trial.stopping {
            "stopping".into()
        } else {
            self.package_status(id)
        };
        Ok(json!({"trialId":id,"activationOwner":id,"status":state,
            "cleanupConfirmed":self.cleanup_done(id),"activation":self.activation(id)?}))
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
        let trial = key == DataKey::Trial;
        self.next_generation += 1;
        let owner = self.next_generation;
        let id = if trial {
            format!("{}.trial.{owner}", manifest.id)
        } else {
            manifest.id.clone()
        };
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
                if self.packages.candidates.len() >= 32 {
                    return Err(RpcError::new("busy", "candidate limit reached"));
                }
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
                let result = std::process::Command::new("node")
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
                self.packages.candidates.insert(token, candidate);
                Ok(report)
            }
            "packages.installed" => Ok(serde_json::to_value(
                self.packages.store.get(string(params, "id")?),
            )
            .map_err(anyhow::Error::from)?),
            "packages.trial" => {
                let token = string(params, "token")?;
                let request_id = match params.get("requestId") {
                    None => None,
                    Some(value) => {
                        let id = value
                            .as_str()
                            .filter(|id| !id.is_empty() && id.len() <= 128)
                            .ok_or_else(|| {
                                RpcError::invalid("requestId must contain 1..128 bytes")
                            })?;
                        Some(id.to_owned())
                    }
                };
                let candidate = self
                    .packages
                    .candidates
                    .get(token)
                    .cloned()
                    .ok_or_else(|| RpcError::new("stale", "unknown validation token"))?;
                if let Some(request) = &request_id
                    && let Some((&id, trial)) = self.packages.trials.iter().find(|(_, trial)| {
                        trial.owner == caller && trial.request_id.as_ref() == Some(request)
                    })
                {
                    if trial.candidate.digest() != candidate.digest() {
                        return Err(RpcError::new(
                            "conflict",
                            "trial requestId was used for another candidate",
                        ));
                    }
                    return self.trial_state(id);
                }
                if candidate
                    .manifest()
                    .capabilities
                    .iter()
                    .any(|c| !matches!(c.as_str(), "commands" | "ui.window" | "events"))
                {
                    return Err(RpcError::new(
                        "unsupported",
                        "finite trial supports only commands, windows and events",
                    ));
                }
                if self.packages.trials.len() >= 16 {
                    return Err(RpcError::new("busy", "trial limit"));
                }
                let owner =
                    self.load_package(candidate.directory(), candidate.manifest(), DataKey::Trial)?;
                self.packages.trials.insert(
                    owner,
                    Trial {
                        owner: caller,
                        request_id,
                        candidate,
                        stopping: false,
                    },
                );
                Ok(json!({"trialId":owner,"activationOwner":owner,"status":"pending"}))
            }
            "packages.stopTrial" | "packages.trialState" | "packages.trialStateByRequest" => {
                let id = if method == "packages.trialStateByRequest" {
                    let request = string(params, "requestId")?;
                    self.packages
                        .trials
                        .iter()
                        .find(|(_, trial)| {
                            trial.owner == caller && trial.request_id.as_deref() == Some(request)
                        })
                        .map(|(&id, _)| id)
                        .ok_or_else(|| {
                            RpcError::new("missing", "trial request unknown in this owner session")
                        })?
                } else {
                    params["trialId"]
                        .as_u64()
                        .ok_or_else(|| RpcError::invalid("trialId required"))?
                };
                let trial = self
                    .packages
                    .trials
                    .get_mut(&id)
                    .ok_or_else(|| RpcError::new("missing", "trial unknown"))?;
                if trial.owner != caller {
                    return Err(RpcError::new("owner", "trial belongs to another instance"));
                }
                if method == "packages.stopTrial" {
                    trial.stopping = true;
                    self.disable(id);
                }
                self.trial_state(id)
            }
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
                    if self.packages.trials.iter().any(|(id, t)| {
                        t.candidate.digest() == candidate.digest() && !self.cleanup_done(*id)
                    }) {
                        return Err(RpcError::new(
                            "busy",
                            "stop matching trial and await cleanup before install",
                        ));
                    }
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
    fn data_directory_of(&self, key: &DataKey, id: &str) -> Option<PathBuf> {
        plugins::data_root().map(|root| key.directory(&root, id))
    }
    /// Install or upgrade the built-in plugins the installation ships
    /// (`Resources/plugins/builtin.json`, or `$VESTRA_BUILTIN_PLUGINS`). A
    /// built-in is upgraded only when the installation carries a newer
    /// version than it last seeded, so a user's rollback stays.
    fn seed_builtins(&mut self) {
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
        for entry in list {
            if let Err(error) = self.seed_one(&directory, &entry) {
                eprintln!("built-in plugin {}: {error:#}", entry.id);
            }
        }
    }
    fn seed_one(&mut self, directory: &Path, entry: &seed::Entry) -> Result<()> {
        if self.development.contains_key(&entry.id) || self.operation_pending(&entry.id) {
            return Ok(());
        }
        let before = self.packages.store.get(&entry.id);
        let version = crate::manifest::parse_version(&entry.version)
            .ok_or_else(|| anyhow::anyhow!("invalid version {}", entry.version))?;
        let instance = match &before.instance {
            Some(current) if !current.builtin => {
                eprintln!(
                    "built-in plugin {}: installed from another source; not seeded",
                    entry.id
                );
                return Ok(());
            }
            Some(current) => {
                let seeded = current
                    .seeded_version
                    .as_deref()
                    .and_then(crate::manifest::parse_version);
                if seeded.is_some_and(|seeded| seeded >= version) {
                    return Ok(());
                }
                Instance {
                    seeded_version: Some(entry.version.clone()),
                    ..current.clone()
                }
            }
            None => Instance {
                builtin: true,
                seeded_version: Some(entry.version.clone()),
                ..Instance::new(Source::Bundled)
            },
        };
        let file = directory.join(&entry.file);
        anyhow::ensure!(
            package_archive::sha256(&file)? == entry.sha256,
            "{} does not match its sha256",
            entry.file
        );
        let extracted = package_archive::extract(&file)?;
        let candidate = Candidate::capture(extracted.path())?;
        anyhow::ensure!(
            candidate.manifest().id == entry.id
                && candidate.manifest().version.as_deref() == Some(entry.version.as_str()),
            "{} holds {} {:?}, not {} {}",
            entry.file,
            candidate.manifest().id,
            candidate.manifest().version,
            entry.id,
            entry.version
        );
        if before
            .current
            .as_ref()
            .is_some_and(|current| current.digest == candidate.digest())
        {
            self.packages.store.set_seeded(&entry.id, &entry.version)?;
            return Ok(());
        }
        self.packages.store.gc()?;
        let target = self.packages.store.stage(&candidate)?;
        // Seeding runs at start-up, before any installed plugin starts: commit
        // directly; the start-up loop then starts it like any installed
        // package (a failure shows as `failed` in the list).
        let request = format!("seed-{}", &entry.sha256[..32]);
        let spec = json!({"method":"seed","params":{"id":entry.id,"version":entry.version}});
        let disabled = self.packages.store.disabled().contains(&entry.id);
        let store = &mut self.packages.store;
        store.prepare_with_receipt(
            &entry.id,
            before.revision,
            &target,
            &request,
            json!({"operationId":request,"packageId":entry.id,"status":"preparing","request":spec}),
            Some(instance),
        )?;
        store.commit_with_receipt(
            &entry.id,
            &request,
            json!({"operationId":request,"packageId":entry.id,"status":"committed","request":spec}),
        )?;
        if disabled {
            store.set_disabled(&entry.id, true)?;
        }
        self.packages.receipts = self.packages.store.operations();
        Ok(())
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
            let trial = self.packages.is_trial(owner);
            let status = self.activations.status(owner, now).ok();
            let plugin = &self.plugins[&owner];
            let id = plugin.manifest.id.clone();
            let stored = self.packages.store.get(&id);
            let installed = !trial
                && stored
                    .current
                    .as_ref()
                    .is_some_and(|current| current.directory == plugin.directory);
            let (state, error) = match status {
                Some(package_activation::Status::Pending { .. }) => ("starting", None),
                Some(package_activation::Status::Failed { message }) => ("failed", Some(message)),
                Some(package_activation::Status::TimedOut) => {
                    ("failed", Some("activation timed out".to_owned()))
                }
                _ => ("running", None),
            };
            let origin = match (trial, installed) {
                (true, _) => "trial",
                (_, true) => "installed",
                _ => "development",
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
                "dataDirectory":plugin.data_directory()}));
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
            let (state, error) = if self.operation_pending(&id) {
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
                .and_then(|i| self.data_directory_of(&DataKey::Instance(i.guid.clone()), &id));
            list.insert(
                id.clone(),
                json!({"id":id,"version":current.manifest.version,
                "origin":"installed","state":state,"error":error,
                "capabilities":current.manifest.capabilities,"directory":current.directory,
                "previousVersion":record.previous.and_then(|p| p.manifest.version),
                "builtin":record.instance.as_ref().is_some_and(|i| i.builtin),
                "source":record.instance.map(|i| i.source),
                "dataDirectory":data}),
            );
        }
        for (id, (directory, manifest)) in &self.development {
            if list.contains_key(id) {
                continue;
            }
            let (state, error) = stopped(self.dev_disabled.contains(id), self.failures.get(id));
            let data = self.data_directory_of(&DataKey::Development(id.clone()), id);
            list.insert(
                id.clone(),
                json!({"id":id,"version":manifest.version,
                "origin":"development","state":state,"error":error,
                "capabilities":manifest.capabilities,"directory":directory,"previousVersion":null,
                "builtin":false,"source":null,"dataDirectory":data}),
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
        if running.is_some_and(|owner| self.packages.is_trial(owner)) {
            return Err(RpcError::new(
                "unsupported",
                "a trial is stopped with its trial handle",
            ));
        }
        if self.operation_pending(id) {
            return Err(RpcError::new("busy", "the package is being installed"));
        }
        let record = self.packages.store.get(id);
        let installed = record.current;
        let key = record
            .instance
            .map(|i| DataKey::Instance(i.guid))
            .unwrap_or(DataKey::Trial);
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
        if self.operation_pending(&id) {
            return Err(RpcError::new("busy", "the package is being installed"));
        }
        let remove_data = params["removeData"] == true;
        let data = remove_data
            .then(|| {
                installed
                    .instance
                    .as_ref()
                    .and_then(|i| self.data_directory_of(&DataKey::Instance(i.guid.clone()), &id))
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
            self.purge_secrets(&DataKey::Instance(instance.guid.clone()), &id);
        }
        self.purge_dropped_secrets();
        Ok(receipt)
    }
    /// Deletes the secret store entries recorded for `key` (on a worker
    /// thread: keychain calls block and may prompt).
    fn purge_secrets(&self, key: &DataKey, id: &str) {
        let index = self.packages.secrets.clone();
        let service = key.secret_service(id);
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
        for (guid, id) in self.packages.store.take_dropped() {
            self.purge_secrets(&DataKey::Instance(guid), &id);
        }
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
            self.seed_builtins();
            // Start committed packages independently of the authoring plugin.
            let pending = self
                .packages
                .operations
                .values()
                .map(|o| o.package.clone())
                .collect::<Vec<_>>();
            let installed = self.packages.store.installed();
            for (id, record) in installed {
                // R-H10: a package the user disabled stays stopped; a
                // development load of the same id runs instead this run.
                if !pending.contains(&id)
                    && !self.packages.store.disabled().contains(&id)
                    && !self.development.contains_key(&id)
                    && let Some(current) = record.current
                    && let Some(instance) = record.instance
                    && let Err(error) = self.load_package(
                        &current.directory,
                        &current.manifest,
                        DataKey::Instance(instance.guid),
                    )
                {
                    eprintln!("installed package {id} failed to start: {error}");
                    self.failures
                        .insert(id.clone(), error.to_string().chars().take(512).collect());
                }
            }
        }
        let dead = self
            .packages
            .trials
            .iter()
            .filter(|(_, t)| t.stopping || !self.plugins.contains_key(&t.owner))
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        for id in dead {
            self.disable(id);
        }
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

/// The installation's built-in plugin list (written by `scripts/stage-plugins.mjs`).
mod seed {
    use anyhow::{Context, Result, ensure};
    use serde::Deserialize;
    use std::path::{Path, PathBuf};

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

    /// `$VESTRA_BUILTIN_PLUGINS`, else `Resources/plugins` beside the packaged
    /// runtime; none when it has no `builtin.json` (a development run).
    pub fn directory() -> Option<PathBuf> {
        std::env::var_os("VESTRA_BUILTIN_PLUGINS")
            .map(PathBuf::from)
            .or_else(|| {
                Some(
                    std::env::current_exe()
                        .ok()?
                        .parent()?
                        .parent()?
                        .join("Resources/plugins"),
                )
            })
            .filter(|dir| dir.join("builtin.json").is_file())
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
}
