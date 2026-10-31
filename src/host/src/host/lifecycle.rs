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
            let status = self.activations.status(owner, now)?;
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
        let _ = self.activations.runtime_failed(owner, message.into(), now);
        if let Ok(mut receipt) = self.activation(owner) {
            receipt["running"] = false.into();
            self.activation_receipts
                .retain(|receipt| receipt["owner"] != owner);
            self.activation_receipts.push_back(receipt);
            if self.activation_receipts.len() > 128 {
                self.activation_receipts.pop_front();
            }
        }
        self.activations.remove(owner);
        self.activation_ui
            .retain(|(candidate, _)| *candidate != owner);
    }
    pub(crate) fn activation_tick(&mut self) {
        let owners = self.plugins.keys().copied().collect::<Vec<_>>();
        let now = self.now_ms();
        for owner in owners {
            let status = match self.activations.status(owner, now) {
                Ok(status) => status,
                Err(error) => {
                    diag!("activation status: {error}");
                    continue;
                }
            };
            let terminal_failure = matches!(
                status,
                package_activation::Status::Failed { .. } | package_activation::Status::TimedOut
            );
            if terminal_failure {
                let message = match status {
                    package_activation::Status::Failed { message } => message,
                    _ => "activation timed out".into(),
                };
                self.record_failure(owner, &message);
                self.disable(owner);
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
    /// Load a development package (`--plugin DIR`).
    pub(crate) fn load(&mut self, directory: &Path, timeout_ms: u64) -> Result<()> {
        if !(1_000..=120_000).contains(&timeout_ms) {
            bail!("activation timeout must be 1000..120000ms");
        }
        // Validate before spawning, preventing duplicate plugin activation side effects.
        let candidate = manifest::read_development(directory)?;
        if self.plugins.values().any(|p| p.manifest.id == candidate.id) {
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
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .filter(|owner| *owner <= 9_007_199_254_740_991)
            .ok_or_else(|| anyhow::anyhow!("plugin owner identities exhausted"))?;
        self.activations.begin(
            self.next_generation,
            self.now_ms()
                .checked_add(timeout_ms)
                .ok_or_else(|| anyhow::anyhow!("activation deadline overflow"))?,
        )?;
        let plugin = match Plugin::load(directory, self.next_generation, self.events.clone()) {
            Ok(plugin) => plugin,
            Err(error) => {
                self.activations.remove(self.next_generation);
                return Err(error);
            }
        };
        let manifest_id = plugin.manifest.id.clone();
        // `packages.list` keeps this run's development loads after they stop.
        self.development.insert(
            manifest_id.clone(),
            (plugin.directory.clone(), plugin.manifest.clone()),
        );
        self.failures.remove(&manifest_id);
        self.plugins.insert(self.next_generation, plugin);
        self.backend.commands.send(Command {
            reply: Reply {
                owner: self.next_generation,
                id: 0,
            },
            method: "owner.registered".into(),
            params: json!({"manifestId":manifest_id}),
        })?;
        Ok(())
    }
    pub(crate) fn disable(&mut self, owner: u64) {
        self.stop(owner, false);
    }
    /// Stop a running plugin and revoke what it holds. `park` (lazy
    /// activation, `lazy.rs`): its recordable registrations stay, marked
    /// recorded, and its process gets longer to finish.
    pub(crate) fn stop(&mut self, owner: u64, park: bool) {
        // Process exit commonly follows an explicit disable. The first revoke
        // owns cleanup: a duplicate notification must not cancel its id=0
        // rebuild, clear another instance, or repeat backend/resource cleanup.
        if !self.plugins.contains_key(&owner) {
            return;
        }
        self.archive_activation(owner, "plugin disabled before activation completed");
        self.lazy.forget_running(owner);
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
        self.cleanup.insert(owner, (false, false));
        // Waiting for graceful disposal/killing belongs off the Slint event loop.
        if let Some(mut plugin) = self.plugins.remove(&owner) {
            if park {
                // A routine stop, not a fault: its `onDispose` gets time.
                plugin.set_stop_wait(lazy::PARK_WAIT);
            }
            let events = self.events.clone();
            std::thread::spawn(move || {
                drop(plugin);
                let _ = events.send(Event::Reaped(owner));
            });
        }
    }
    pub(crate) fn send(&mut self, owner: u64, message: Value) {
        // Lazy activation: a parked owner has no process; a trigger wakes
        // it, and is held (as are triggers for a plugin still waking) until
        // its `runtime.ready`.
        if !self.lazy_deliver(owner, &message) {
            return;
        }
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
            self.disable(owner);
        }
    }
    pub(crate) fn broadcast(&mut self, event: &str, data: Value) {
        if event == "host.error" {
            diag!("host.error {data}");
        }
        let mut failed = Vec::new();
        for (owner, plugin) in self
            .plugins
            .iter_mut()
            .filter(|(_, p)| p.subscriptions.contains_key(event))
        {
            if let Err(error) = plugin.send(json!({"event":event,"data":data})) {
                failed.push((*owner, error));
            }
        }
        for (owner, error) in failed {
            self.transport_failed(owner, &error);
        }
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
