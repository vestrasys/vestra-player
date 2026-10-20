//! Finite trusted-package lifecycle. This is not a sandbox or dependency solver.
#![forbid(unsafe_code)]
use super::*;
use package_stage::Candidate;
use package_store::{Store, StoredPackage};

pub struct Manager {
    store: Store,
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
                PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                    .join("Library/Application Support/Vestra/packages")
            });
        let store = Store::open(root)?;
        let receipts = store.operations();
        Ok(Self {
            store,
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
        trial: bool,
    ) -> Result<u64> {
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
                api_version: manifest.api_version.clone(),
                entry: manifest.entry.clone(),
                capabilities: manifest.capabilities.clone(),
                version: Some(manifest.version.clone()),
            },
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
            "packages.validate" => {
                if self.packages.candidates.len() >= 32 {
                    return Err(RpcError::new("busy", "candidate limit reached"));
                }
                let candidate = Candidate::capture(Path::new(string(params, "directory")?))?;
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
                let owner = self.load_package(candidate.directory(), candidate.manifest(), true)?;
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
                let result = json!({"operationId":request,"packageId":id,"status":"preparing","request":request_spec,"fingerprint":{"action":method,"packageId":id,"digest":target.digest,"expectedRevision":expected}});
                self.packages.store.prepare_with_receipt(
                    &id,
                    expected,
                    &target,
                    &request,
                    result.clone(),
                )?;
                let waiting = self
                    .plugins
                    .iter()
                    .find(|(_, p)| p.manifest.id == id)
                    .map(|(o, _)| *o);
                if let Some(owner) = waiting {
                    self.disable(owner);
                }
                self.packages.operations.insert(
                    request.clone(),
                    Operation {
                        id: request,
                        package: id,
                        target,
                        old: before.current,
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
            _ => Err(RpcError::new("unsupported", "unknown package method")),
        }
    }
    pub(super) fn package_tick(&mut self) {
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
                self.packages.operations.insert(
                    recovery.operation_id.clone(),
                    Operation {
                        id: recovery.operation_id,
                        package: recovery.id,
                        target: recovery.target,
                        old: recovery.before.current,
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
            // Start committed packages independently of the authoring plugin.
            let pending = self
                .packages
                .operations
                .values()
                .map(|o| o.package.clone())
                .collect::<Vec<_>>();
            let installed = self.packages.store.installed();
            for (id, record) in installed {
                if !pending.contains(&id)
                    && let Some(current) = record.current
                    && let Err(error) =
                        self.load_package(&current.directory, &current.manifest, false)
                {
                    eprintln!("installed package {id} failed to start: {error}");
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
            match self.load_package(&op.target.directory, &op.target.manifest, false) {
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
                    op.owner = Some(self.load_package(&old.directory, &old.manifest, false)?);
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
