//! The core's actions, executed (doc/architecture/supervisor.md §2.2, §5):
//! each executor is the body of today's code that made the same decision
//! (`lazy.rs` until B2c). B2c runs the actions of standby, wake and
//! classification; the Host still starts and stops package RPCs' runs
//! itself and announces through `notify_tick` (B2d), so those actions are
//! not run here.
use super::*;
use crate::supervisor::{AnswerReason, Cause, JobKind, Notice};

impl Host {
    /// Runs what the core decided, in order.
    pub(crate) fn supervisor_execute(&mut self, actions: Vec<Action<Value>>) {
        for action in actions {
            match action {
                Action::JobFinished {
                    unit, kind, result, ..
                } => self.job_finished(unit, &kind, result),
                Action::RemoveData {
                    data: Some(data), ..
                } => remove_data(&data),
                Action::QueryResources { pid, .. } => self.query_resources(pid.0),
                Action::Checkpoint {
                    unit,
                    pid,
                    resident,
                } => self.checkpoint(unit, pid.0, resident),
                Action::Terminate {
                    pid,
                    reason: crate::supervisor::StopReason::Idle,
                    keep_registrations: true,
                    ..
                } => self.park(pid.0),
                Action::Rekey { from, to, .. } => self.on_demand.rekey = Some((from.0, to.0)),
                Action::Spawn { unit, pid, .. } => self.spawn_woken(unit, pid.0),
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
                // The reaper times every stop's grace and kills at it (A3),
                // the Host removes recordings and data at its own choke
                // points, and nothing announces through the core before
                // B2d's `list_state`; the Host's own stops (`stop`) are not
                // the core's `Terminate` until B2d, nor its verifies.
                Action::RemoveData { data: None, .. }
                | Action::Terminate { .. }
                | Action::Kill { .. }
                | Action::KillRunner { .. }
                | Action::Verify { .. }
                | Action::Replay { .. }
                | Action::DropCheckpoint { .. }
                | Action::Notify { unit: None, .. }
                | Action::ListChanged { .. } => {}
            }
        }
    }
    /// A job ended. B2b–B2d: the wait of a start the Host runs itself (an
    /// enable, a reload; a package operation reads its result,
    /// `package_wait`). B2c: a wake that failed (its spawn, or its stopped
    /// process that did not go within 20 s) is today's `wake_failed`.
    fn job_finished(&mut self, unit: UnitId, kind: &JobKind, result: JobResult) {
        let id = self.unit_id(unit);
        if id.is_empty() {
            return;
        }
        match kind {
            JobKind::Start {
                cause: Cause::Trigger(_),
            } => {
                if let JobResult::Failed(error) = result {
                    diag!("lazy: {id} could not start: {error}");
                    self.failures.insert(id, error.chars().take(512).collect());
                }
            }
            JobKind::Start {
                cause: Cause::Enable,
            }
            | JobKind::Restart { image: None, .. } => self.restart_finished(&id, result),
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
            self.supervisor.checkpoint_known(unit, left);
        }
    }
    /// The core's `Terminate` of an idle run, keeping its registrations
    /// (T12, T14; today's `park` after its recording): stopped, held.
    fn park(&mut self, owner: u64) {
        let (Some(key), Some(plugin)) = (self.checkpoint_key_of(owner), self.plugins.get(&owner))
        else {
            return;
        };
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
        self.stop(owner, StopReason::Idle);
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
                ..Held::new(id, key.guid, origin, name)
            },
        );
        self.probe_registry(owner, "parked");
    }
    /// The core's start of a unit in standby (T6; today's `wake_now`): the
    /// held entries let go of their catalog, the process starts under the
    /// pid the core allocated, then the entries move to it (`Rekey`).
    fn spawn_woken(&mut self, unit: UnitId, to: u64) {
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
        match self.start_package_as(&id, to) {
            Ok(()) => {
                if let Some(from) = from {
                    diag!(
                        "lazy: {id} woken by {} (owner {from} → {to})",
                        first.as_deref().unwrap_or("its settings page")
                    );
                    capabilities::rekey(self, from, to);
                }
                self.supervisor.process_spawned(Pid(to), Ok(()));
            }
            Err(error) => {
                if let Some(from) = from {
                    self.on_demand.unrekeyed.insert(to, from);
                }
                self.supervisor
                    .process_spawned(Pid(to), Err(error.to_string()));
            }
        }
        self.supervisor_actions();
    }
    /// The core revokes the registrations under `owner` (a failed wake, a
    /// standby left for good): today's `forget_entries`, and the held data
    /// goes.
    fn revoke_held(&mut self, owner: u64) {
        let owner = self.on_demand.unrekeyed.remove(&owner).unwrap_or(owner);
        self.unhold(owner);
        self.forget_entries(owner);
    }
    /// An OSD for `unit` (i18n texts here, §5).
    fn notice(&mut self, unit: UnitId, notice: Notice) {
        let id = self.unit_id(unit);
        let name = self.on_demand.names.get(&id).cloned().unwrap_or(id);
        let text = match notice {
            Notice::Starting => i18n::tr_args("Starting {}…", &json!([name])),
            Notice::CouldNotStart => i18n::tr_args("{} could not start", &json!([name])),
            // B4 (crash notices, start limits, runner restarts) and B3.
            other => {
                diag!("supervisor: unit {name}: notice {other:?} not shown before B3/B4");
                return;
            }
        };
        self.show_osd(&text);
    }
    /// A held trigger that cannot be delivered (§4.6): from B3
    /// (`answer_held`); `legacy()` answers nothing.
    fn held_unanswered(&mut self, unit: UnitId, reason: AnswerReason, payload: &Value) {
        diag!(
            "supervisor: unit {}: {} not delivered: {reason:?}",
            self.unit_id(unit),
            event(payload).unwrap_or("a trigger")
        );
    }
}
