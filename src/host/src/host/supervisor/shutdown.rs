//! Quit and a device-loss restart (doc/architecture/supervisor.md §9): the
//! core's `shutdown` with one deadline for every plugin process, 1 s at quit
//! and 300 ms after a GPU device loss. Every checkpoint is written before the
//! first stop (coordinator ruling); then every process gets its `runtime.stop`
//! at once (the shared runner one `runner.stop` for all its members), and what
//! is still running at the deadline is killed. A process left from a stop that
//! landed stuck is killed at once. Before the supervisor each plugin was
//! dropped with the host, one after another, up to 300 ms each.
use super::*;

/// The quit's deadline for every plugin process.
pub(crate) const QUIT_BUDGET: Duration = Duration::from_secs(1);
/// After a GPU device loss: the teardown has `EXIT_AFTER_LOOP` in all.
pub(crate) const DEVICE_LOSS_BUDGET: Duration = Duration::from_millis(300);

impl Host {
    /// Stops every plugin process within `budget` and waits for them (at most
    /// `budget` and the reaper's last poll).
    pub(crate) fn shutdown_plugins(&mut self, budget: Duration) {
        let began = Instant::now();
        let deadline = began + budget;
        self.quit_source_ended();
        self.probe_quit();
        // Nothing more is created; a process the spawn thread created and
        // the Host did not take yet (its `Event::Spawned` still queued)
        // stops with the rest, within the same deadline.
        let created = self
            .on_demand
            .spawner
            .as_ref()
            .map(worker::Spawner::quit)
            .unwrap_or_default();
        for (owner, mut plugin) in created {
            diag!("supervisor: plugin {owner} created as the player quits; stopped");
            plugin.set_stop_reason(StopReason::Quit);
            plugin.set_stop_wait(budget);
            self.reaper.reap(owner, plugin);
        }
        let now = self.now_ms();
        let budget_ms = u64::try_from(budget.as_millis()).unwrap_or(u64::MAX);
        let actions = self.supervisor.shutdown(now, budget_ms);
        let running = self.plugins.len();
        // Checkpoints first, from the registrations as they are now.
        let (checkpoints, rest): (Vec<_>, Vec<_>) = actions
            .into_iter()
            .partition(|action| matches!(action, Action::Checkpoint { .. }));
        let mut runs = checkpoints
            .into_iter()
            .filter_map(|action| match action {
                Action::Checkpoint { pid, resident, .. } => Some((pid.0, resident)),
                _ => None,
            })
            .collect::<Vec<_>>();
        runs.sort_unstable();
        // The recordings the recorder thread was given land first (every
        // checkpoint before the first stop), within the deadline.
        self.recordings_written_by(deadline.saturating_duration_since(Instant::now()));
        for (owner, resident) in runs {
            self.quit_checkpoint(owner, resident);
        }
        // Stops from here on are the quit's: no UI cleanup, one deadline.
        self.on_demand.quitting = true;
        // The shared runner stops all its members with one `runner.stop`.
        if let Some(runner) = self.shared.take() {
            runner.quit(budget, &self.reaper);
        }
        self.supervisor_execute(rest);
        // Anything the core did not stop (none expected) stops all the same.
        let left = self.plugins.keys().copied().collect::<Vec<_>>();
        for owner in left {
            diag!("supervisor: plugin {owner} stopped at quit without its unit");
            self.quit_stop(owner, budget_ms);
        }
        let done = self.reaper.finish_by(deadline);
        // The reaper polls every 5 ms and a gone process's stderr is read
        // up to 100 ms (`process::STDERR_WAIT`).
        let waited = done.recv_timeout(budget + Duration::from_millis(200));
        diag!(
            "supervisor: {running} plugin processes stopped in {} ms{}",
            began.elapsed().as_millis(),
            if waited.is_ok() {
                ""
            } else {
                " (some still going)"
            }
        );
    }
    /// The player quits with a source in playback: each running plugin that
    /// listens to `source.ended` gets it with `reason: "quit"` and the
    /// source's last state, ahead of its stop on the same pipe, so within
    /// the quit's deadline (events-and-triggers.md §4.6). A plugin in
    /// standby gets nothing: it is neither woken nor is the event kept
    /// (§6, M5).
    fn quit_source_ended(&mut self) {
        let Some(data) = crate::source_events::at_quit(&self.last_source) else {
            return;
        };
        let event = crate::source_events::ENDED;
        let message = serde_json::json!({"event":event,"data":data});
        let mut failed = Vec::new();
        for (owner, plugin) in self
            .plugins
            .iter_mut()
            .filter(|(_, plugin)| plugin.subscriptions.contains_key(event))
        {
            if let Err(error) = plugin.send(message.clone()) {
                failed.push((*owner, error));
            }
        }
        // A full queue: its stop follows anyway (the deadline kills it).
        for (owner, error) in failed {
            diag!("plugin {owner} quit: source.ended not sent: {error}");
        }
    }
    /// A quit's stop of `owner` (the `Terminate` executor while quitting):
    /// its `runtime.stop` (\`quit\`), killed after `grace`; nothing else is
    /// cleaned up, the player goes. A shared-runner member stops with its
    /// runner (`runner.stop`).
    pub(super) fn quit_stop(&mut self, owner: u64, grace: u64) {
        let Some(mut plugin) = self.plugins.remove(&owner) else {
            return;
        };
        self.plugins_revision += 1;
        plugin.set_stop_reason(StopReason::Quit);
        plugin.set_stop_wait(Duration::from_millis(grace));
        self.reaper.reap(owner, plugin);
    }
    /// A run's checkpoint at quit (today's `lazy_quit`): resident as such,
    /// a lazy one with its registrations as they are now.
    fn quit_checkpoint(&mut self, owner: u64, resident: bool) {
        let Some(key) = self.checkpoint_key_of(owner) else {
            return;
        };
        let directory = self.activation_dir();
        let id = self.plugin_id(owner);
        let (calls, resident) = if resident {
            if checkpoint::read(&directory, &key).is_some_and(|r| r.resident_at_ready) {
                return;
            }
            (Vec::new(), true)
        } else {
            let backend = self
                .on_demand
                .answers
                .get(&owner)
                .cloned()
                .unwrap_or(Value::Null);
            // Something held in the playback worker (`held_in_worker`: work
            // in flight there is not), or a request there whose answer has
            // not come (`worker_requested`): the safe side, as a
            // classification without an answer. The next start launches it,
            // and its ready classifies it again.
            let live = held_in_worker(&backend).is_some();
            let unanswered =
                self.on_demand.refreshing.contains(&owner) || self.on_demand.stale.contains(&owner);
            if (live || unanswered) && !self.on_demand.probe.park_all {
                diag!("lazy: {id} holds something live in the playback worker at quit");
                (Vec::new(), true)
            } else {
                match self.standby_record(owner, &backend) {
                    Ok(calls) => (calls, false),
                    Err(reason) => {
                        diag!("lazy: {id} not recorded at quit: {reason}");
                        return;
                    }
                }
            }
        };
        match checkpoint::write(&directory, &key, calls, resident) {
            Ok(()) => diag!("lazy: {id} recorded at quit"),
            Err(error) => diag!("lazy: {id}: its recording: {error}"),
        }
    }
}
