//! Quit and a device-loss restart (doc/architecture/supervisor.md §7.2,
//! T31, A2; B4): the core's `shutdown` with one deadline for every plugin
//! process, 1 s at quit and 300 ms after a GPU device loss. Every
//! checkpoint is written before the first stop (coordinator ruling R6);
//! then every process gets its `runtime.stop` at once (the shared runner
//! one `runner.stop` for all its members), and what is still running at the
//! deadline is killed. A process left from a stop that landed stuck is
//! killed at once (§17.7 item 20). Before B4 each plugin was dropped with
//! the host, one after another, up to 300 ms each.
use super::*;

/// The quit's deadline for every plugin process (§7.2).
pub(crate) const QUIT_BUDGET: Duration = Duration::from_secs(1);
/// After a GPU device loss: the teardown has `EXIT_AFTER_LOOP` in all.
pub(crate) const DEVICE_LOSS_BUDGET: Duration = Duration::from_millis(300);

impl Host {
    /// Stops every plugin process within `budget` (T31) and waits for them
    /// (at most `budget` and the reaper's last poll).
    pub(crate) fn shutdown_plugins(&mut self, budget: Duration) {
        let began = Instant::now();
        let deadline = began + budget;
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
        // Checkpoints first (R6), from the registrations as they are now.
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
    /// A quit's stop of `owner` (the `Terminate` executor while quitting):
    /// its `runtime.stop` (\`quit\`), killed after `grace`; nothing else is
    /// cleaned up, the player goes. A shared-runner member stops with its
    /// runner (`runner.stop`).
    pub(super) fn quit_stop(&mut self, owner: u64, grace: u64) {
        let Some(mut plugin) = self.plugins.remove(&owner) else {
            return;
        };
        self.unit_stopped(&plugin.manifest.id.clone(), owner);
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
            match self.standby_record(owner, &backend) {
                Ok(calls) => (calls, false),
                Err(reason) => {
                    diag!("lazy: {id} not recorded at quit: {reason}");
                    return;
                }
            }
        };
        match checkpoint::write(&directory, &key, calls, resident) {
            Ok(()) => diag!("lazy: {id} recorded at quit"),
            Err(error) => diag!("lazy: {id}: its recording: {error}"),
        }
    }
}
