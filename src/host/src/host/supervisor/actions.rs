//! The core's actions, executed (doc/architecture/supervisor.md §2.2, §5):
//! each executor is the body of today's code that made the same decision.
//! Since B2d every start (`Spawn`, after `Verify` or `Replay`) and every
//! stop (`Terminate`) runs here; the Host announces list changes through
//! `notify_tick` (`ListChanged` is not run), and the reaper times every
//! stop's grace and kills at it (A3: `Kill`), until B4's `shutdown` and
//! runner restart (`KillRunner`).
use super::*;
use crate::supervisor::{AnswerReason, Cause, JobKind, JobResult, Notice, STUCK_RELOAD};

impl Host {
    /// Runs what the core decided, in order.
    pub(crate) fn supervisor_execute(&mut self, actions: Vec<Action<Value>>) {
        for action in actions {
            match action {
                Action::JobFinished {
                    unit, kind, result, ..
                } => self.job_finished(unit, &kind, result),
                Action::RemoveData { unit } => self.remove_unit_data(unit),
                Action::QueryResources { pid, .. } => self.query_resources(pid.0),
                Action::Checkpoint {
                    unit,
                    pid,
                    resident,
                } => self.checkpoint(unit, pid.0, resident),
                Action::DropCheckpoint { unit } => self.drop_checkpoint(unit),
                Action::Terminate {
                    pid,
                    reason,
                    grace,
                    keep_registrations: true,
                    ..
                } => self.park(pid.0, reason, grace),
                Action::Terminate {
                    pid, reason, grace, ..
                } => self.terminate(pid.0, reason, grace),
                Action::Rekey { from, to, .. } => self.on_demand.rekey = Some((from.0, to.0)),
                Action::Spawn {
                    unit, pid, image, ..
                } => self.spawn_unit(unit, pid.0, image),
                Action::Verify { unit } => self.verify_unit(unit),
                Action::Replay { unit, holder } => self.replay_unit(unit, holder.0),
                Action::Revoke { pid, .. } => self.revoke_held(pid.0),
                Action::Notify {
                    unit: Some(unit),
                    notice,
                } => self.notice(unit, notice),
                Action::Answer {
                    unit,
                    reason,
                    payload,
                } => self.held_unanswered(unit, reason, &payload),
                // A woken run's ready runs these itself, in today's order
                // (`unit_ready`).
                Action::Prune { pid, .. } => capabilities::prune(self, pid.0),
                Action::Deliver { pid, payload, .. } => self.send(pid.0, payload),
                Action::Kill { .. }
                | Action::KillRunner { .. }
                | Action::Notify { unit: None, .. }
                | Action::ListChanged { .. } => {}
            }
        }
    }
    /// A job ended. A wake that failed (its spawn, or its stopped process
    /// that did not go within 20 s) is today's `wake_failed`; an enable or
    /// reload whose previous process did not go is today's `reload_tick`
    /// failure. A failed spawn of any other start was recorded by its
    /// executor (`spawn_unit`); a package operation reads its job's result.
    fn job_finished(&mut self, unit: UnitId, kind: &JobKind, result: JobResult) {
        let id = self.unit_id(unit);
        if id.is_empty() {
            return;
        }
        if matches!(
            kind,
            JobKind::Stop {
                then: crate::supervisor::Then::Removed,
                ..
            }
        ) {
            self.remove_if_gone(unit);
        }
        match (kind, result) {
            (
                JobKind::Start {
                    cause: Cause::Trigger(_),
                },
                JobResult::Failed(error),
            ) => {
                diag!("lazy: {id} could not start: {error}");
                self.failures.insert(id, error.chars().take(512).collect());
            }
            (
                JobKind::Start {
                    cause: Cause::Enable,
                }
                | JobKind::Restart { image: None, .. },
                JobResult::Failed(text),
            ) if text == STUCK_RELOAD => {
                self.failures.insert(id, format!("reload: {text}"));
            }
            _ => {}
        }
    }
    /// The core decided a checkpoint (`Action::Checkpoint`). A standby's
    /// was written ahead (`write_ahead`); a resident class is written when
    /// the file does not say so (today's `lazy_classified`).
    fn checkpoint(&mut self, unit: UnitId, owner: u64, resident: bool) {
        if self
            .on_demand
            .written
            .is_some_and(|(written, _)| written == owner)
        {
            return;
        }
        let Some(key) = self.checkpoint_key_of(owner) else {
            return;
        };
        if !resident {
            // Unreachable before B4's `shutdown`: a lazy checkpoint is a
            // standby's, written ahead.
            diag!(
                "supervisor: unit {}: lazy checkpoint not written ahead",
                self.unit_id(unit)
            );
            return;
        }
        let directory = self.activation_dir();
        let recorded = checkpoint::read(&directory, &key).is_some_and(|r| r.resident_at_ready);
        if !recorded && let Err(error) = checkpoint::write(&directory, &key, Vec::new(), true) {
            diag!("lazy: {}: its recording: {error}", self.plugin_id(owner));
            // The core follows the file as it is now.
            let left = checkpoint::read(&directory, &key).map(|r| r.resident_at_ready);
            self.supervisor.checkpoint_failed(unit, left);
        }
    }
    /// The unit's recording is of no use any more (an uninstall at its
    /// request, a reload, an install or upgrade: `Restart{fresh}`): the
    /// installed instance's recording goes (today's `forget_parked(id,
    /// true)`; the package RPCs remove it at the request too).
    fn drop_checkpoint(&mut self, unit: UnitId) {
        let id = self.unit_id(unit);
        if let Some(instance) = self.installed_instance(&id) {
            checkpoint::remove(&self.activation_dir(), &instance);
        }
    }
    /// The core's `Terminate` keeping the run's registrations: an idle run
    /// (T12, T14; today's `park` after its recording), or from B3 a start
    /// that timed out (T8/T11, `start_failure_keeps_registrations`; its
    /// recording is the one it was woken from): stopped, held.
    fn park(&mut self, owner: u64, reason: crate::supervisor::StopReason, grace: u64) {
        let (Some(_), Some(plugin)) = (self.checkpoint_key_of(owner), self.plugins.get(&owner))
        else {
            // A failed start with nothing to hold: stopped as any other
            // stop (an idle park of such a run never comes).
            if reason != crate::supervisor::StopReason::Idle {
                self.terminate(owner, reason, grace);
            }
            return;
        };
        if reason != crate::supervisor::StopReason::Idle {
            let id = plugin.manifest.id.clone();
            let now = self.now_ms();
            if let Ok(activation::Status::TimedOut) = self.activation_status(owner, now) {
                self.record_failure(owner, "activation timed out");
            }
            let plugin = &self.plugins[&owner];
            let origin = Origin {
                catalog: i18n::load(&plugin.directory),
                ..plugin.origin()
            };
            let name = crate::packages::package_manager::display_name(
                &plugin.manifest,
                origin.catalog.as_deref(),
            );
            let (warnings, warning) = (self.warnings_of(owner), self.warning_text(owner));
            self.stop_process(owner, wire_reason(reason), Duration::from_millis(grace));
            diag!("lazy: {id} could not start; its entry points stay for the next use");
            self.hold(
                owner,
                Held {
                    warnings,
                    warning,
                    ..Held::new(id, origin, name)
                },
            );
            return;
        }
        let id = plugin.manifest.id.clone();
        // Its own catalog: the stopping plugin unregisters its `Arc` on
        // the thread that drops it, which must not take the held one's.
        let origin = Origin {
            catalog: i18n::load(&plugin.directory),
            ..plugin.origin()
        };
        let name = crate::packages::package_manager::display_name(
            &plugin.manifest,
            origin.catalog.as_deref(),
        );
        let count = self
            .on_demand
            .written
            .filter(|(written, _)| *written == owner)
            .map_or(0, |(_, count)| count);
        let (warnings, warning) = (self.warnings_of(owner), self.warning_text(owner));
        self.probe_registry(owner, "parking");
        self.stop_process(owner, StopReason::Idle, Duration::from_millis(grace));
        diag!("lazy: {id} parked ({count} recorded calls)");
        // `VESTRA_TEST_EXECUTE` entries timed from this park.
        let now = self.now_ms();
        self.probe_parked(&id, now);
        // Its settings pages still show in its language.
        self.hold(
            owner,
            Held {
                warnings,
                warning,
                ..Held::new(id, origin, name)
            },
        );
        self.probe_registry(owner, "parked");
    }
    /// The core stops run `owner` (T24, T10/T25 under `legacy()`, a
    /// package RPC's stop half): today's `lifecycle.rs` `stop`. A failed
    /// activation the core timed or was told of is recorded as
    /// `activation_tick` records it.
    fn terminate(&mut self, owner: u64, reason: crate::supervisor::StopReason, grace: u64) {
        if !self.plugins.contains_key(&owner) {
            return;
        }
        let reason = wire_reason(reason);
        if reason == StopReason::Failed {
            let now = self.now_ms();
            match self.activation_status(owner, now) {
                Ok(activation::Status::Failed { message }) => self.record_failure(owner, &message),
                Ok(activation::Status::TimedOut) => {
                    self.record_failure(owner, "activation timed out")
                }
                _ => {}
            }
        }
        self.stop_process(owner, reason, Duration::from_millis(grace));
    }
    /// The core starts `unit` as process `to` (T6; today's
    /// `start_installed`, `start_stopped`, `load`, an operation's
    /// `load_package`, and a wake's `wake_now`): from standby the held
    /// entries let go of their catalog, the process starts under the pid
    /// the core allocated, then the entries move to it (`Rekey`). A spawn
    /// that fails is recorded as the start that asked for it did.
    fn spawn_unit(&mut self, unit: UnitId, to: u64, image: Option<String>) {
        let id = self.unit_id(unit);
        let from = self
            .on_demand
            .rekey
            .take()
            .filter(|(_, rekey_to)| *rekey_to == to)
            .map(|(from, _)| from);
        if let Some(from) = from {
            self.unhold(from);
        }
        let first = self
            .supervisor
            .unit(unit)
            .and_then(|unit| {
                unit.backlog
                    .iter()
                    .find(|trigger| !trigger.payload.is_null())
            })
            .and_then(|trigger| event(&trigger.payload).map(str::to_owned));
        let job = self.supervisor.job(unit).cloned();
        let now = self.now_ms();
        let deadline = job
            .as_ref()
            .and_then(|job| job.deadline)
            .unwrap_or(now + 15_000);
        let development = self
            .supervisor
            .unit(unit)
            .is_some_and(|unit| unit.class == crate::supervisor::Class::Development);
        match self.start_unit_process(&id, to, image.as_deref(), development, deadline) {
            Ok(()) => {
                if let Some(from) = from {
                    diag!(
                        "lazy: {id} woken by {} (owner {from} → {to})",
                        first.as_deref().unwrap_or("its settings page")
                    );
                    capabilities::rekey(self, from, to);
                }
                if job
                    .as_ref()
                    .is_some_and(|job| job.cause() == Some(Cause::FirstRun))
                {
                    diag!("lazy: {id} starts for its first run");
                }
            }
            Err(error) => {
                if let Some(from) = from {
                    self.on_demand.unrekeyed.insert(to, from);
                }
                self.start_failed(&id, job.as_ref().map(|job| &job.kind), &error);
                self.supervisor
                    .process_spawned(Pid(to), Err(error.to_string()));
            }
        }
    }
    /// A start's process could not be created: where today's start put
    /// the error. A package RPC answering now (`setEnabled`, `reload`, a
    /// development load) returns it; a boot start or first run is listed
    /// failed (`start_installed`); an enable or reload that waited is
    /// listed failed (today's `restart_finished`); a wake is today's `wake_failed`
    /// (`job_finished`); an operation reads its job's result.
    fn start_failed(&mut self, id: &str, kind: Option<&JobKind>, error: &anyhow::Error) {
        if let Some(slot) = self.start_error.as_mut()
            && slot.0 == id
            && slot.1.is_none()
        {
            slot.1 = Some(RpcError::from(anyhow::anyhow!("{error:#}")));
            return;
        }
        match kind {
            Some(JobKind::Start {
                cause: Cause::Boot | Cause::FirstRun,
            }) => {
                diag!("installed package {id} failed to start: {error}");
                self.failures
                    .insert(id.into(), error.to_string().chars().take(512).collect());
            }
            Some(
                JobKind::Start {
                    cause: Cause::Enable | Cause::AutoRestart,
                }
                | JobKind::Restart { image: None, .. },
            ) => {
                let message = RpcError::from(anyhow::anyhow!("{error:#}")).message;
                diag!("packages.reload {id}: {message}");
                self.failures.entry(id.into()).or_insert(message);
            }
            _ => {}
        }
    }
    /// A built-in's digest check before its start (T4): the check the
    /// start-up (or a reload, or a runner restart) began on its thread;
    /// done now if it is over, else when `poll_verified` sees it end or
    /// its `VERIFY_WAIT` runs out (`verify_tick`).
    fn verify_unit(&mut self, unit: UnitId) {
        let id = self.unit_id(unit);
        self.packages_verify_begin(&id, unit);
        self.verify_tick();
    }
    /// The core revokes the registrations under `owner` (a failed wake, a
    /// standby left for good): today's `forget_entries`, and the held data
    /// goes.
    fn revoke_held(&mut self, owner: u64) {
        let owner = self.on_demand.unrekeyed.remove(&owner).unwrap_or(owner);
        self.unhold(owner);
        self.forget_entries(owner);
    }
    /// An uninstall's data may go now (no process may write it any more):
    /// every directory the uninstall requests of `unit` resolved goes, then
    /// the unit, while it is still removed and tracks no process (a
    /// reinstall may own it by now: review notes B2-3, B2-5).
    fn remove_unit_data(&mut self, unit: UnitId) {
        for directory in self.data_removals.remove(&unit).unwrap_or_default() {
            remove_data(&directory);
        }
        self.remove_if_gone(unit);
    }
    /// `remove_unit` of a unit that is still removed (a reinstall did not
    /// take it back), has no job, tracks no process and owes no data
    /// removal (review note B2-3): at its `RemoveData`, at the end of its
    /// uninstall's stop, or when a stuck stop's process went late.
    pub(super) fn remove_if_gone(&mut self, unit: UnitId) {
        if self.supervisor.removable(unit) {
            self.supervisor.remove_unit(unit);
        }
    }
    /// The name `unit` is shown by in notices (as held).
    fn unit_name(&self, unit: UnitId) -> String {
        let id = self.unit_id(unit);
        self.on_demand.names.get(&id).cloned().unwrap_or(id)
    }
    /// An OSD for `unit` (i18n texts here, §5).
    fn notice(&mut self, unit: UnitId, notice: Notice) {
        let name = self.unit_name(unit);
        let text = match notice {
            Notice::Starting => i18n::tr_args("Starting {}…", &json!([name])),
            Notice::CouldNotStart => i18n::tr_args("{} could not start", &json!([name])),
            // B4 (crash notices, start limits, runner restarts).
            other => {
                diag!("supervisor: unit {name}: notice {other:?} not shown before B4");
                return;
            }
        };
        self.supervisor_osd(unit, text);
    }
    /// A supervisor OSD for `unit`, shown once for the same text within a
    /// second: a start that could not start and the answers to its held
    /// clicks, or a storm of clicks answered busy, show one.
    fn supervisor_osd(&mut self, unit: UnitId, text: String) {
        let now = self.now_ms();
        let repeated = self
            .on_demand
            .last_osd
            .as_ref()
            .is_some_and(|(u, shown, at)| {
                *u == unit && *shown == text && now.saturating_sub(*at) < 1_000
            });
        if !repeated {
            self.show_osd(&text);
            self.on_demand.last_osd = Some((unit, text, now));
        }
    }
    /// A held trigger that cannot be delivered (§4.6, from B3:
    /// `answer_held`, and a trigger refused by the bounds): a command shows
    /// why in an OSD ("`<name>` is busy" / "could not start" / "is
    /// disabled"); a resolution request fails at once with that text
    /// instead of at its 30 s deadline; a drop or a UI callback is dropped
    /// with a diagnostic line; a wake event never comes here (dropped
    /// silently by the core).
    fn held_unanswered(&mut self, unit: UnitId, reason: AnswerReason, payload: &Value) {
        let name = self.unit_name(unit);
        let what = event(payload).unwrap_or("a shown settings page");
        diag!(
            "supervisor: unit {}: {what} not delivered: {reason:?}",
            self.unit_id(unit)
        );
        let text = match reason {
            AnswerReason::Busy => i18n::tr_args("{} is busy", &json!([name])),
            AnswerReason::Disabled => i18n::tr_args("{} is disabled", &json!([name])),
            AnswerReason::CouldNotStart | AnswerReason::Held { .. } => {
                i18n::tr_args("{} could not start", &json!([name]))
            }
            // Uninstalled, or its start canceled by another stop (a reload,
            // quit): nothing the click can still do.
            AnswerReason::Removed | AnswerReason::Stopped => {
                i18n::tr_args("{} is no longer available", &json!([name]))
            }
        };
        match what {
            "commands.execute" => self.supervisor_osd(unit, text),
            "source.resolveRequest" => {
                if let Some(id) = payload["data"]["requestId"].as_u64() {
                    let _ = self.backend.commands.send(Command {
                        reply: Reply { owner: 0, id: 0 },
                        method: "owner.refuseResolution".into(),
                        params: json!({"requestId": id, "message": text}),
                    });
                }
            }
            _ => {}
        }
    }
}

/// The wire's stop reason (`runtime.stop`) of the core's (§7.3, A14).
fn wire_reason(reason: crate::supervisor::StopReason) -> StopReason {
    use crate::supervisor::StopReason as Core;
    match reason {
        Core::Quit => StopReason::Quit,
        Core::Disable => StopReason::Disable,
        Core::Uninstall => StopReason::Uninstall,
        Core::Reload => StopReason::Reload,
        Core::Upgrade => StopReason::Upgrade,
        Core::Idle => StopReason::Idle,
        Core::Failed => StopReason::Failed,
    }
}
