//! A plugin's life in the host: activation, loading, disabling, sending
//! and broadcasting.
use super::*;

/// The window policy's slot (the backdrop has no preference or auto-hide).
/// What an initial UI request means for activation. A slot another plugin
/// holds does not fail it: the plugin runs without its panel, with a warning
/// (`packages.list`).
pub(crate) fn activation_outcome(
    result: &std::result::Result<Value, RpcError>,
) -> std::result::Result<(), String> {
    match result {
        Err(error) if !error.is_taken() => Err(format!("{}: {}", error.code, error.message)
            .chars()
            .take(512)
            .collect()),
        _ => Ok(()),
    }
}

impl Host {
    pub(crate) fn activation(&mut self, owner: u64) -> Result<Value> {
        if let Some(plugin) = self.plugins.get(&owner) {
            let id = plugin.manifest.id.clone();
            let now = self.now_ms();
            let status = self.activation_status(owner, now)?;
            return Ok(json!({"owner":owner,"id":id,"running":true,"activation":status}));
        }
        Ok(self
            .activation_receipts
            .iter()
            .find(|receipt| receipt["owner"] == owner)
            .cloned()
            .unwrap_or_else(
                || json!({"owner":owner,"running":false,"activation":{"status":"unknown"}}),
            ))
    }
    pub(crate) fn archive_activation(&mut self, owner: u64, message: &str) {
        let now = self.now_ms();
        let _ = self
            .supervisor
            .activation_failed(Pid(owner), message.into(), now);
        if let Ok(mut receipt) = self.activation(owner) {
            receipt["running"] = false.into();
            self.activation_receipts
                .retain(|receipt| receipt["owner"] != owner);
            self.activation_receipts.push_back(receipt);
            if self.activation_receipts.len() > 128 {
                self.activation_receipts.pop_front();
            }
        }
        // With the UI requests it waited on (the old `activation_ui`).
        self.supervisor.activation_end(Pid(owner));
    }
    pub(crate) fn activation_tick(&mut self) {
        let owners = self.plugins.keys().copied().collect::<Vec<_>>();
        let now = self.now_ms();
        for owner in owners {
            let status = match self.activation_status(owner, now) {
                Ok(status) => status,
                Err(error) => {
                    diag!("activation status: {error}");
                    continue;
                }
            };
            let terminal_failure = matches!(
                status,
                activation::Status::Failed { .. } | activation::Status::TimedOut
            );
            if terminal_failure {
                let message = match status {
                    activation::Status::Failed { message } => message,
                    _ => "activation timed out".into(),
                };
                self.record_failure(owner, &message);
                // The core stops it (T10; T11 when it timed out, under
                // `legacy()` without a retry): `Terminate`.
                self.supervisor.set_now(now);
                self.supervisor.process_failed(Pid(owner), message);
                self.supervisor_run();
            }
        }
    }
    #[cfg(feature = "producer-audio")]
    pub(crate) fn physical_input(&self, params: Value) {
        if self
            .backend
            .commands
            .send(Command {
                reply: Reply { owner: 0, id: 0 },
                method: "host.physicalInput".into(),
                params,
            })
            .is_err()
        {
            diag!("host.error physical-input backend unavailable");
        }
    }
    #[cfg(feature = "producer-audio")]
    pub(crate) fn physical_revoke(&self, reason: producer_input::RevokeReason) {
        self.physical_input(json!({"kind":"revoke","reason":reason}));
    }
    /// `plugin` runs as `owner` (`plugins`, and its unit in the
    /// supervisor's unit table: `unit_started`).
    pub(crate) fn add_plugin(&mut self, owner: u64, plugin: Plugin) {
        self.unit_started(&plugin.manifest.id, owner);
        self.plugins.insert(owner, plugin);
    }
    /// Load a development package (`--plugin DIR`): checked here, then a
    /// start job of its `Development` unit (outside the lanes, with its own
    /// start timeout); its spawn is `load_development_as`.
    pub(crate) fn load(&mut self, directory: &Path, timeout_ms: u64) -> Result<()> {
        if !(1_000..=120_000).contains(&timeout_ms) {
            bail!("activation timeout must be 1000..120000ms");
        }
        // Validate before spawning, preventing duplicate plugin activation side effects.
        let candidate = manifest::read_development(directory)?;
        if self.running(&candidate.id).is_some() {
            bail!("plugin already running: {}", candidate.id);
        }
        // Built-in plugins are developed with `--plugin`, so a reserved id
        // loads here; a development load is never installed (data under
        // plugin-data/development/), so it cannot become a built-in.
        if manifest::reserved_id(&candidate.id) {
            diag!(
                "host.warning development load {}: vestra. ids are reserved for built-in plugins; \
                 this load is for built-in development only and cannot be installed",
                candidate.id
            );
        }
        if self.owners_exhausted() {
            bail!("plugin owner identities exhausted");
        }
        self.now_ms()
            .checked_add(timeout_ms)
            .ok_or_else(|| anyhow::anyhow!("activation deadline overflow"))?;
        self.dev_loading = Some(directory.to_owned());
        let spec =
            crate::supervisor::UnitSpec::new(&candidate.id, crate::supervisor::Class::Development)
                .start_timeout(timeout_ms);
        let unit = self.supervisor.add_unit(spec);
        if self.supervisor.unit(unit).is_some_and(|u| u.job.is_none()) {
            // A unit of this id still stopping or busy keeps it: as an enable.
            self.supervisor.set_enabled(unit, true);
            let _ = self
                .supervisor
                .start(unit, crate::supervisor::Cause::Enable);
        }
        let started = self.start_now(&candidate.id);
        self.dev_loading = None;
        started.map_err(|error| anyhow::anyhow!(error.message))
    }
    /// The core's spawn of a development unit (`Action::Spawn`): today's
    /// `load` after its checks, as process `owner` with the activation
    /// deadline the core gave its start.
    pub(crate) fn load_development_as(
        &mut self,
        directory: &Path,
        owner: u64,
        deadline: u64,
    ) -> Result<()> {
        let candidate = manifest::read_development(directory)?;
        self.activation_begin(&candidate.id, owner, deadline)?;
        let plugin = match Plugin::load(directory, owner, self.events.clone()) {
            Ok(plugin) => plugin,
            Err(error) => {
                self.supervisor.activation_end(Pid(owner));
                return Err(error);
            }
        };
        // `Plugin::load` reads the manifest again: one changed in between
        // would run under an id other than the one its activation and the
        // `running` check were for.
        if plugin.manifest.id != candidate.id {
            self.supervisor.activation_end(Pid(owner));
            let error = anyhow::anyhow!(
                "plugin manifest changed while loading: {} became {}",
                candidate.id,
                plugin.manifest.id
            );
            // Its stop waits for the process: off the event loop, as `stop`.
            self.reaper.discard(plugin);
            return Err(error);
        }
        let manifest_id = plugin.manifest.id.clone();
        let plugin_name = plugin.manifest.name.clone();
        // `packages.list` keeps this run's development loads after they stop.
        self.development.insert(
            manifest_id.clone(),
            (plugin.directory.clone(), plugin.manifest.clone()),
        );
        self.failures.remove(&manifest_id);
        self.add_plugin(owner, plugin);
        self.backend.commands.send(Command {
            reply: Reply { owner, id: 0 },
            method: "owner.registered".into(),
            params: json!({"manifestId":manifest_id,"manifestName":plugin_name}),
        })?;
        Ok(())
    }
    /// Runs the core now for a package RPC that starts plugin `id` and
    /// answers with its start's error (`setEnabled`, `reload`, a
    /// development load): the error of a spawn that failed meanwhile, as
    /// today's synchronous start returned it.
    pub(crate) fn start_now(&mut self, id: &str) -> std::result::Result<(), RpcError> {
        let outer = self.start_error.replace((id.to_owned(), None));
        self.supervisor_run();
        let error = std::mem::replace(&mut self.start_error, outer).and_then(|(_, error)| error);
        error.map_or(Ok(()), Err)
    }
    /// A running plugin stops after a failure the Host found (a malformed
    /// request, a full or closed pipe, its runner killed): the core's stop
    /// (`Stop{Failed, then: Inactive}`: listed stopped, as today's
    /// `disable(Failed)` without a recorded failure).
    pub(crate) fn stop_failed(&mut self, owner: u64) {
        if let Some(unit) = self.supervisor.by_pid(Pid(owner))
            && self.supervisor.process_of(&self.unit_id(unit)) == Some(Pid(owner))
        {
            let now = self.now_ms();
            self.supervisor.set_now(now);
            let _ = self.supervisor.stop(
                unit,
                crate::supervisor::StopReason::Failed,
                crate::supervisor::Then::Inactive,
            );
            self.supervisor_run();
        }
        self.stopped_anyway(owner);
    }
    /// A run ended or failed by itself (`Event::Exited`, a runtime fault):
    /// the core's `process_exited` (T25; under `legacy()` every class lands
    /// failed with `text`, today's `disable(Failed)`).
    pub(crate) fn process_ended(
        &mut self,
        owner: u64,
        cause: crate::supervisor::ExitCause,
        text: Option<String>,
    ) {
        let now = self.now_ms();
        self.supervisor.set_now(now);
        self.supervisor.process_exited(Pid(owner), cause, text);
        self.supervisor_run();
        self.stopped_anyway(owner);
    }
    /// Every stop of a running plugin is the core's: one it did not stop is
    /// a bug (its unit does not run it), reported, and stopped here as
    /// before (a `debug_assert!` stops a debug build).
    fn stopped_anyway(&mut self, owner: u64) {
        if self.plugins.contains_key(&owner) {
            debug_assert!(false, "running plugin {owner} not stopped by its unit");
            diag!("supervisor: running plugin {owner} not stopped by its unit");
            self.stop_process(owner, StopReason::Failed, Duration::from_millis(300));
        }
    }
    /// Stop a running plugin and revoke what it holds; `reason` goes with
    /// its `runtime.stop`, and the reaper gives it `grace` before it kills
    /// it (the `Terminate` executor's body). `Idle` parks it (the core's
    /// standby, the adapter's `park`): its recordable registrations stay,
    /// marked recorded.
    pub(crate) fn stop_process(&mut self, owner: u64, reason: StopReason, grace: Duration) {
        let park = reason == StopReason::Idle;
        // Process exit commonly follows an explicit disable. The first revoke
        // owns cleanup: a duplicate notification must not cancel its id=0
        // rebuild, clear another instance, or repeat backend/resource cleanup.
        if !self.plugins.contains_key(&owner) {
            return;
        }
        self.archive_activation(owner, "plugin disabled before activation completed");
        self.on_demand_forget(owner);
        self.held_mounts.forget(owner);
        self.warnings.remove(&owner);
        capabilities::revoke(self, owner, park);
        {
            let mut input = self.input.borrow_mut();
            if input.bottom == Some(owner) {
                input.bottom = None;
            }
            if input.sidebar == Some(owner) {
                input.sidebar = None;
            }
        }
        let tabs_before = self.policy.sidebar_panels();
        if self
            .sequence
            .is_some_and(|(provider, ..)| provider == owner)
        {
            self.sequence = None;
            self.rebroadcast_source();
        }
        if self.info_panels.remove(&owner).is_some() {
            self.show_info_panels();
        }
        // The floating window opens again without this panel (`sync_floating`).
        if self
            .floating
            .as_ref()
            .is_some_and(|f| f.owners.contains(&owner))
        {
            self.close_floating();
        }
        if matches!(&self.menu_mode, MenuMode::Panel(menu_owner, _) if *menu_owner == owner) {
            self.close_menu();
        }
        self.policy.remove_owner(owner);
        self.power.remove_owner(owner);
        self.timeline.remove_owner(owner);
        self.images.release_owner(owner);
        self.on_top.remove(owner);
        self.cursor_playing.remove(owner);
        self.pip.remove_owner(owner);
        let step = self.step_seconds();
        if self.step_seek.remove(&owner).is_some() {
            self.step_seek_changed(step);
        }
        self.mount_queue.retain(|c| c.reply.owner != owner);
        if self
            .preparing
            .as_ref()
            .is_some_and(|(_, c)| c.reply.owner == owner)
        {
            self.preparing = None;
        }
        if let Some(mount) = self.mounts.remove(&owner) {
            // Another sidebar tab stays shown until the shell is rebuilt.
            let last_tab = tabs_before.iter().all(|&o| o == owner);
            if let Some(shell) = &self.shell
                && let Some(property) = mount.spec.slot.visible_property()
                && (mount.spec.slot != composition::Slot::Sidebar || last_tab)
            {
                let _ = shell.set_property(property, false.into());
            }
            self.mount_queue.push_front(MountChange {
                reply: Reply { owner, id: 0 },
                contribution: None,
                announced: false,
            });
        }
        // The worker drops its shader lists too (`diagnostics.rs`).
        self.plugin_shaders_gone(owner);
        // Parked: the worker keeps its resolvers (recorded).
        let _ = self.backend.commands.send(Command {
            reply: Reply { owner, id: 0 },
            method: if park {
                "owner.parked"
            } else {
                "owner.disabled"
            }
            .into(),
            params: Value::Null,
        });
        let keys: Vec<_> = self
            .views
            .keys()
            .filter(|(key, _)| *key == owner)
            .cloned()
            .collect();
        for key in keys {
            if let Some(view) = self.views.remove(&key) {
                let _ = view.instance.hide();
            }
        }
        // Waiting for graceful disposal/killing belongs off the Slint event
        // loop: the reaper thread's, which reports `Event::Reaped`; until
        // then (and the playback worker's `BackendCleaned`) the unit is
        // deactivating, and every start of it waits (supervisor.md §17.9).
        if let Some(mut plugin) = self.plugins.remove(&owner) {
            self.unit_stopped(&plugin.manifest.id, owner);
            plugin.set_stop_reason(reason);
            plugin.set_stop_wait(grace);
            self.reaper.reap(owner, plugin);
        }
    }
    pub(crate) fn send(&mut self, owner: u64, message: Value) {
        // On-demand start: an owner in standby has no process; a trigger
        // wakes it, and is held (as are triggers for a plugin still waking)
        // until its `runtime.ready` (the core's `wake`).
        let Some(owner) = self.deliver_to(owner, &message) else {
            return;
        };
        let failed = self
            .plugins
            .get_mut(&owner)
            .and_then(|plugin| plugin.send(message).err());
        if let Some(error) = failed {
            self.transport_failed(owner, &error);
        }
    }
    /// A message for `owner` could not be queued. An own process: its queue
    /// is full or its pipe closed, and it stops, as before. A member of the
    /// shared runner: its queue fills only while the runner reads none of
    /// its input (one reader for every member), so the runner is stuck;
    /// every member restarts with it (`restart_shared`), this one included,
    /// instead of this one stopping for good.
    pub(crate) fn transport_failed(&mut self, owner: u64, error: &anyhow::Error) {
        let Some(plugin) = self.plugins.get(&owner) else {
            return;
        };
        diag!("plugin {owner} ({}) transport: {error}", plugin.manifest.id);
        let runner = plugin.shared_runner();
        if runner.is_some() && self.shared.as_ref().map(|s| s.link.id) == runner {
            self.restart_shared(&format!(
                "{}'s queue is full: the runner reads no input",
                plugin.manifest.id
            ));
        } else {
            // Its queue is full or its pipe closed: the stop line cannot be
            // queued either, and the process is killed after the wait.
            self.stop_failed(owner);
        }
    }
    pub(crate) fn broadcast(&mut self, event: &str, data: Value) {
        if event == "host.error" {
            diag!("host.error {data}");
            // The host log: the code and the message up to any path or link.
            crate::diag::log_only(&super::diagnostics::host_error_line(&data));
        }
        // Commands whose state is a condition on this state (`plugin.when`)
        // follow it, whether their plugin runs or not.
        if matches!(
            event,
            "source.state" | "window.state" | "appearance.changed"
        ) {
            capabilities::refresh_conditions(self);
        }
        let wake = capabilities::WAKE_EVENTS.contains(&event);
        let now = self.now_ms();
        self.supervisor.set_now(now);
        let mut failed = Vec::new();
        for (owner, plugin) in self
            .plugins
            .iter_mut()
            .filter(|(_, p)| p.subscriptions.contains_key(event))
        {
            // A plugin woken for a wake event receives the held one at its
            // ready: a newer one waits behind it, never before (§6.9; the
            // core holds it while its start is waking).
            if wake && self.supervisor.waking(Pid(*owner)).is_some() {
                let trigger = Trigger::event(event, json!({"event":event,"data":data}));
                self.supervisor.wake(Pid(*owner), &trigger);
                continue;
            }
            if let Err(error) = plugin.send(json!({"event":event,"data":data})) {
                failed.push((*owner, error));
            }
        }
        for (owner, error) in failed {
            self.transport_failed(owner, &error);
        }
        // A plugin in standby that listens to a low-frequency event is
        // woken to receive it (lazy activation §6.9).
        self.wake_listeners(event, &data);
    }
    /// Why a plugin stopped on its own (`packages.list` state `failed`).
    pub(crate) fn record_failure(&mut self, owner: u64, message: &str) {
        if let Some(plugin) = self.plugins.get(&owner) {
            self.failures.insert(
                plugin.manifest.id.clone(),
                message.chars().take(512).collect(),
            );
        }
    }
    pub(crate) fn now_ms(&self) -> u64 {
        self.clock.elapsed().as_millis().min(u64::MAX as u128) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_slot_refusal_leaves_activation_to_succeed() {
        let refusal = RpcError::slot_occupied(composition::Slot::Bottom, "a.b");
        assert_eq!(activation_outcome(&Err(refusal)), Ok(()));
        let sequence = RpcError::sequence_taken("a.b");
        assert_eq!(activation_outcome(&Err(sequence)), Ok(()));
        // A file type or site another plugin resolves (source_resolvers).
        let resolver = RpcError::taken("resolver-taken", "taken".into(), json!({"holder":1}));
        assert_eq!(activation_outcome(&Err(resolver)), Ok(()));
        assert_eq!(activation_outcome(&Ok(Value::Null)), Ok(()));
        assert_eq!(
            activation_outcome(&Err(RpcError::new("ui-prepare", "syntax error"))),
            Err("ui-prepare: syntax error".into())
        );
        assert_eq!(
            activation_outcome(&Err(RpcError::new("conflict", "owner already mounted"))),
            Err("conflict: owner already mounted".into())
        );
    }

    #[test]
    fn a_full_sidebar_is_refused_like_a_taken_slot() {
        let full = RpcError::sidebar_full();
        assert_eq!(
            serde_json::to_value(&full).unwrap(),
            json!({"code":"conflict","message":"The sidebar holds at most 8 tabs","reason":"slot-occupied","slot":"sidebar"})
        );
        assert_eq!(activation_outcome(&Err(full)), Ok(()));
    }
}
